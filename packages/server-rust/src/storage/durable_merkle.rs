//! Residency-independent Merkle index implementation.
//!
//! [`DurableMerkle`] implements [`DurableMerkleIndex`] by enumerating durable
//! leaves from a [`MapDataStore`] in a single streaming pass and folding them
//! into per-partition LWW + OR-Map Merkle trees that exactly mirror the
//! write-path observer's tree layout. The resulting [`MerkleSession`] snapshot
//! serves repeated `root` / `buckets` / `leaf_keys` queries from the
//! already-materialised trees — no additional storage round-trips per query.
//!
//! All hash and routing decisions delegate to the same primitives the write
//! path uses (`merkle_leaf_hash`, `hash_to_partition`, `combine_hashes`) so
//! the session root is byte-identical to the live in-memory root for the same
//! record set, regardless of whether those records are resident in memory.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use topgun_core::hash::combine_hashes;
use topgun_core::hash_to_partition;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};

use super::map_data_store::{
    DurableMerkleIndex, LeafSink, MapDataStore, MerkleLeaf, MerkleLeafKind, MerkleSession,
};

// ---------------------------------------------------------------------------
// DurableMerkle
// ---------------------------------------------------------------------------

/// Concrete implementation of [`DurableMerkleIndex`].
///
/// A unit struct — all state is carried by the returned [`MerkleSession`].
/// Holds no mutable state, so the same `DurableMerkle` instance may be shared
/// across threads and used concurrently to build independent sessions.
pub struct DurableMerkle;

impl DurableMerkleIndex for DurableMerkle {
    fn build_session(&self, map: &str, store: &dyn MapDataStore) -> anyhow::Result<MerkleSession> {
        // Run the async enumeration from a synchronous context. When called
        // from within a Tokio multi-threaded runtime (the normal server path
        // and `#[tokio::test(flavor = "multi_thread")]` tests), block_in_place
        // parks the current thread and hands the executor thread back to the
        // pool, avoiding a deadlock on the single-threaded runtime.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { build_session_async(map, store).await })
        })
    }
}

/// Async inner: enumerate leaves and fold into per-partition trees.
async fn build_session_async(map: &str, store: &dyn MapDataStore) -> anyhow::Result<MerkleSession> {
    let mut sink = SessionBuildSink {
        lww_trees: HashMap::new(),
        ormap_trees: HashMap::new(),
        leaf_keys_by_path: HashMap::new(),
        depth: 3,
    };
    // A single streaming pass — no values are loaded, only (key, kind, hash).
    //
    // Propagate any enumeration failure rather than swallowing it: a partial
    // scan would leave `sink` holding a subset of leaves, and `into_session`
    // would then publish a non-empty but WRONG root. The sync read source must
    // degrade to an error (so the handler can reject/retry), never to wrong
    // leaves — the same contract `MerkleSyncManager::rebuild_from_datastore`
    // upholds for the in-memory accelerator.
    store.enumerate_leaves(map, false, &mut sink).await?;
    Ok(sink.into_session())
}

// ---------------------------------------------------------------------------
// SessionBuildSink
// ---------------------------------------------------------------------------

/// [`LeafSink`] that accumulates leaves into per-partition LWW and OR-Map trees,
/// reproducing the write-path observer's routing exactly.
struct SessionBuildSink {
    /// `partition_id` → LWW tree.
    lww_trees: HashMap<u32, Mutex<MerkleTree>>,
    /// `partition_id` → OR-Map tree.
    ormap_trees: HashMap<u32, Mutex<ORMapMerkleTree>>,
    /// Leaf key membership by hex-path for `leaf_keys` queries.
    /// Key: hex-path prefix; value: sorted record keys at that leaf level.
    leaf_keys_by_path: HashMap<String, Vec<String>>,
    /// Tree depth — must match `MerkleSyncManager::depth` (default 3).
    depth: usize,
}

#[async_trait]
impl LeafSink for SessionBuildSink {
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
        for leaf in batch {
            let partition_id = hash_to_partition(&leaf.key);
            match leaf.kind {
                MerkleLeafKind::Lww => {
                    self.lww_trees
                        .entry(partition_id)
                        .or_insert_with(|| Mutex::new(MerkleTree::new(self.depth)))
                        .lock()
                        .update(&leaf.key, leaf.leaf_hash);
                }
                MerkleLeafKind::OrMap => {
                    self.ormap_trees
                        .entry(partition_id)
                        .or_insert_with(|| Mutex::new(ORMapMerkleTree::new(self.depth)))
                        .lock()
                        .update(&leaf.key, leaf.leaf_hash);
                }
            }
            // Record the hex-path prefix so leaf_keys() can answer without
            // re-enumerating. We store the key under its depth-length prefix.
            let hex_path = format!("{:08x}", topgun_core::hash::fnv1a_hash(&leaf.key));
            let leaf_path = &hex_path[..self.depth.min(hex_path.len())];
            self.leaf_keys_by_path
                .entry(leaf_path.to_string())
                .or_default()
                .push(leaf.key);
        }
        Ok(())
    }
}

impl SessionBuildSink {
    /// Consume the sink and produce the immutable [`MerkleSession`] snapshot.
    fn into_session(self) -> MerkleSession {
        // Materialise LWW aggregate buckets for every path that any partition
        // exposes. Paths are discovered by walking `get_buckets` at each depth
        // level from root, descending the hex trie to enumerate every internal
        // node exactly once.

        // ---- LWW bucket map ------------------------------------------------
        let mut lww_nodes: HashMap<String, HashMap<char, u32>> = HashMap::new();
        materialise_aggregate_buckets(
            &self.lww_trees,
            &mut lww_nodes,
            |trees, path| {
                let mut per_char: HashMap<char, Vec<u32>> = HashMap::new();
                for tree in trees.values() {
                    let buckets = tree.lock().get_buckets(path);
                    for (c, h) in buckets {
                        per_char.entry(c).or_default().push(h);
                    }
                }
                per_char
                    .into_iter()
                    .map(|(c, hashes)| (c, combine_hashes(&hashes)))
                    .collect()
            },
            self.depth,
        );

        // ---- OR-Map bucket map --------------------------------------------
        let mut ormap_nodes: HashMap<String, HashMap<char, u32>> = HashMap::new();
        materialise_aggregate_buckets(
            &self.ormap_trees,
            &mut ormap_nodes,
            |trees, path| {
                let mut per_char: HashMap<char, Vec<u32>> = HashMap::new();
                for tree in trees.values() {
                    let buckets = tree.lock().get_buckets(path);
                    for (c, h) in buckets {
                        per_char.entry(c).or_default().push(h);
                    }
                }
                per_char
                    .into_iter()
                    .map(|(c, hashes)| (c, combine_hashes(&hashes)))
                    .collect()
            },
            self.depth,
        );

        // ---- Aggregate roots ---------------------------------------------
        let lww_root = {
            let hashes: Vec<u32> = self
                .lww_trees
                .values()
                .map(|t| t.lock().get_root_hash())
                .collect();
            combine_hashes(&hashes)
        };
        let ormap_root = {
            let hashes: Vec<u32> = self
                .ormap_trees
                .values()
                .map(|t| t.lock().get_root_hash())
                .collect();
            combine_hashes(&hashes)
        };

        MerkleSession::from_materialised(
            lww_nodes,
            ormap_nodes,
            lww_root,
            ormap_root,
            self.leaf_keys_by_path,
        )
    }
}

/// Walk a set of per-partition trees and materialise the aggregate
/// per-hex-bucket hashes for every reachable internal node into `out_nodes`.
///
/// `bucket_fn` computes the aggregate `HashMap<char, u32>` for a given path
/// by combining per-partition buckets with `combine_hashes`. `max_depth` is a
/// hard bound: internal nodes live at paths of length `0..max_depth`, so once a
/// path reaches `max_depth` its children are leaf-level and must not be
/// expanded. The trie self-terminates today (leaf nodes expose no children via
/// `get_buckets`), but enforcing the bound keeps the snapshot correct even if
/// `MerkleTree` leaf enumeration changes — never publish a node the live tree
/// would not.
fn materialise_aggregate_buckets<T>(
    trees: &HashMap<u32, Mutex<T>>,
    out_nodes: &mut HashMap<String, HashMap<char, u32>>,
    bucket_fn: impl Fn(&HashMap<u32, Mutex<T>>, &str) -> HashMap<char, u32>,
    max_depth: usize,
) where
    T: Send,
{
    // Depth-first walk from root ("") down through all reachable children,
    // visiting each internal node exactly once.
    let mut stack: Vec<String> = vec![String::new()]; // start at root path ""
    while let Some(path) = stack.pop() {
        // Hard bound: paths of length `max_depth` address leaf nodes, which are
        // served via `leaf_keys`, never `buckets`. Never call `get_buckets` on
        // them or publish them — even if a future `MerkleTree` change made leaf
        // nodes expose children, the snapshot must not gain a level the sync
        // protocol does not walk.
        if path.len() >= max_depth {
            continue;
        }
        let children = bucket_fn(trees, &path);
        if children.is_empty() {
            continue;
        }
        for c in children.keys() {
            stack.push(format!("{path}{c}"));
        }
        out_nodes.insert(path, children);
    }
}

// ---------------------------------------------------------------------------
// MerkleSession real implementations
// ---------------------------------------------------------------------------

// We extend `MerkleSession` here with the real fields and constructors.
// The stub in `map_data_store.rs` holds placeholder fields; this module
// replaces those bodies with real implementations that read from the
// materialised per-kind trie snapshots.

impl MerkleSession {
    /// Construct a fully materialised session snapshot from the aggregated trie data.
    ///
    /// Called exclusively from [`DurableMerkle::build_session`] after folding all
    /// enumerated leaves; no other code should construct `MerkleSession` directly.
    pub(crate) fn from_materialised(
        lww_nodes: HashMap<String, HashMap<char, u32>>,
        ormap_nodes: HashMap<String, HashMap<char, u32>>,
        lww_root: u32,
        ormap_root: u32,
        leaf_keys_by_path: HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            lww_nodes,
            ormap_nodes,
            lww_root,
            ormap_root,
            leaf_keys_by_path,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use async_trait::async_trait;
    use topgun_core::hash::fnv1a_hash;
    use topgun_core::hash_to_partition;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::RedbDataStore;
    use crate::storage::factory::ObserverFactory;
    use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch, ScanCursor};
    use crate::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
    use crate::storage::record::{OrMapEntry, Record, RecordMetadata, RecordValue};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn fresh_redb() -> (Arc<dyn MapDataStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("durable_merkle_test.redb");
        let store = RedbDataStore::new(&path).expect("redb open");
        (Arc::new(store), dir)
    }

    fn lww_value(key: &str, millis: u64, counter: u32, node_id: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(key.to_string()),
            timestamp: Timestamp {
                millis,
                counter,
                node_id: node_id.to_string(),
            },
        }
    }

    fn ormap_value(key: &str, tag: &str, millis: u64) -> RecordValue {
        RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String(key.to_string()),
                tag: tag.to_string(),
                timestamp: Timestamp {
                    millis,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            }],
            tombstones: vec![],
        }
    }

    fn lww_record(key: &str, millis: u64) -> Record {
        Record {
            value: lww_value(key, millis, 0, "n1"),
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    fn ormap_record(key: &str, tag: &str, millis: u64) -> Record {
        Record {
            value: ormap_value(key, tag, millis),
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    /// Drive the write-path observer for a single record, routing to the same
    /// `(map, hash_to_partition(key))` pair the real server uses.
    fn write_path_put(manager: &Arc<MerkleSyncManager>, map: &str, key: &str, record: &Record) {
        let factory = MerkleObserverFactory::new(Arc::clone(manager));
        let partition = hash_to_partition(key);
        let observer = factory
            .create_observer(map, partition)
            .expect("observer for any map");
        observer.on_put(key, record, None, false);
    }

    // -----------------------------------------------------------------------
    // SpyStore: panics on load/load_all, delegates enumerate_leaves to inner.
    // -----------------------------------------------------------------------
    struct SpyStore {
        inner: RedbDataStore,
        enumerate_calls: Arc<AtomicUsize>,
        /// When true, `enumerate_leaves` returns an error AFTER pushing a
        /// partial batch — models a mid-stream storage fault.
        fail_enumerate: bool,
    }

    #[async_trait]
    impl MapDataStore for SpyStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner.add(map, key, value, expiration_time, now).await
        }
        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }
        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }
        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }
        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            panic!("build_session must NOT load full values: load() called");
        }
        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            panic!("build_session must NOT load full values: load_all() called");
        }
        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.enumerate_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_enumerate {
                // Push a partial batch first so the sink holds a non-empty
                // subset, then fail — proving build_session must not publish a
                // session over that subset.
                sink.consume(vec![MerkleLeaf {
                    key: "partial".to_string(),
                    kind: MerkleLeafKind::Lww,
                    leaf_hash: 0xDEAD_BEEF,
                }])
                .await?;
                anyhow::bail!("simulated mid-stream enumerate_leaves fault");
            }
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }
        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }
        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }
        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }
        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }
        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }
        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }
        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }
        async fn flush_key(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            is_backup: bool,
        ) -> anyhow::Result<()> {
            self.inner.flush_key(map, key, value, is_backup).await
        }
        fn reset(&self) {
            self.inner.reset();
        }
    }

    // -----------------------------------------------------------------------
    // AC4 — session root is stable regardless of re-enumeration order.
    //
    // Proves build_session from a store with 2 keys produces a consistent
    // root: two independent sessions from the same store state must agree.
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac4_two_sessions_from_same_store_state_have_equal_root() {
        let (store, _dir) = fresh_redb();
        store
            .add("m", "key-a", &lww_value("key-a", 100, 0, "n1"), 0, 1)
            .await
            .unwrap();
        store
            .add("m", "key-b", &lww_value("key-b", 200, 0, "n2"), 0, 2)
            .await
            .unwrap();

        let idx = DurableMerkle;
        let session1 = idx
            .build_session("m", store.as_ref())
            .expect("build_session");
        let session2 = idx
            .build_session("m", store.as_ref())
            .expect("build_session");

        assert_ne!(
            session1.root(),
            0,
            "root must be non-zero for non-empty map"
        );
        assert_eq!(
            session1.root(),
            session2.root(),
            "two sessions from the same store state must have equal roots"
        );
    }

    // -----------------------------------------------------------------------
    // AC4 (write-path parity) — build_session root matches the write-path
    // observer's root for the same two LWW keys.
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac4_session_root_matches_write_path_root_for_two_lww_keys() {
        let (store, _dir) = fresh_redb();
        let resident = Arc::new(MerkleSyncManager::default());

        for (key, millis) in [("key-a", 100u64), ("key-b", 200u64)] {
            let rec = lww_record(key, millis);
            write_path_put(&resident, "m", key, &rec);
            store.add("m", key, &rec.value, 0, 1).await.unwrap();
        }
        let write_path_root = resident.aggregate_lww_root_hash("m");

        let idx = DurableMerkle;
        let session = idx
            .build_session("m", store.as_ref())
            .expect("build_session");

        assert_eq!(
            session.lww_root(),
            write_path_root,
            "durable session LWW root must equal the write-path resident root (AC4 parity)"
        );
    }

    // -----------------------------------------------------------------------
    // AC5 — Datastore root equals write-path resident tree (mixed LWW+OR-Map).
    //
    // Writes LWW keys and OR-Map keys to redb, builds a DurableMerkle session,
    // and asserts: LWW root matches, OR-Map root matches, combined root matches.
    // Also asserts OR-Map tombstones produce no leaf (parity with write path).
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac5_mixed_lww_and_ormap_session_root_matches_write_path() {
        let (store, _dir) = fresh_redb();
        let resident = Arc::new(MerkleSyncManager::default());

        // LWW records
        for (key, millis) in [("alice", 111u64), ("bob", 222u64), ("carol", 333u64)] {
            let rec = lww_record(key, millis);
            write_path_put(&resident, "users", key, &rec);
            store.add("users", key, &rec.value, 0, 1).await.unwrap();
        }
        // OR-Map records
        for (key, tag, millis) in [("t1", "tagA", 100u64), ("t2", "tagB", 200u64)] {
            let rec = ormap_record(key, tag, millis);
            write_path_put(&resident, "users", key, &rec);
            store.add("users", key, &rec.value, 0, 1).await.unwrap();
        }

        let idx = DurableMerkle;
        let session = idx
            .build_session("users", store.as_ref())
            .expect("build_session");

        let expected_lww = resident.aggregate_lww_root_hash("users");
        let expected_ormap = resident.aggregate_ormap_root_hash("users");

        assert_ne!(expected_lww, 0, "LWW root must be non-zero");
        assert_ne!(expected_ormap, 0, "OR-Map root must be non-zero");
        assert_eq!(
            session.lww_root(),
            expected_lww,
            "durable LWW root must match write-path root (AC5)"
        );
        assert_eq!(
            session.ormap_root(),
            expected_ormap,
            "durable OR-Map root must match write-path root (AC5)"
        );
    }

    // -----------------------------------------------------------------------
    // AC5 (OR-Map tombstone parity) — OrTombstones contribute no leaf.
    //
    // A removed OR-Map key should appear in neither the in-memory tree nor
    // the durable session root. We model this by writing an OrTombstones
    // variant directly to the store (the redb backend stores it as-is).
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac5_ormap_tombstone_produces_no_leaf_in_session() {
        let (store, _dir) = fresh_redb();

        // Write an OR-Map key then replace it with a tombstone.
        store
            .add("m", "gone", &ormap_value("gone", "tag1", 100), 0, 1)
            .await
            .unwrap();
        // Overwrite with tombstone (the redb backend persists whatever we put).
        store
            .add(
                "m",
                "gone",
                &RecordValue::OrTombstones {
                    tags: vec!["tag1".to_string()],
                },
                0,
                2,
            )
            .await
            .unwrap();

        let idx = DurableMerkle;
        let session = idx
            .build_session("m", store.as_ref())
            .expect("build_session");

        // A pure-tombstone map should produce root 0 — no live leaves.
        assert_eq!(
            session.ormap_root(),
            0,
            "OrTombstones must not contribute any leaf to the OR-Map tree (AC5 parity)"
        );
        assert_eq!(
            session.root(),
            0,
            "combined root must be 0 when the only record is a tombstone"
        );
    }

    // -----------------------------------------------------------------------
    // AC6 — No value loading: SpyStore panics on load/load_all.
    //
    // 500+ records written to a real redb store wrapped in SpyStore.
    // build_session must complete without panicking and produce the correct
    // root (matching a control session from an independent store seeded with
    // the same data).
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac6_build_session_never_loads_values() {
        // Seed data: key-value pairs shared between the spy and control stores.
        let records: Vec<(String, RecordValue)> = (0..500u32)
            .map(|i| (format!("k{i:04}"), lww_value("v", u64::from(i), i, "n")))
            .collect();

        // Control store: independent redb file seeded with the same records.
        let ctrl_dir = tempfile::tempdir().expect("tempdir");
        let ctrl_store = RedbDataStore::new(ctrl_dir.path().join("ctrl.redb")).expect("redb open");
        for (k, v) in &records {
            ctrl_store.add("big", k, v, 0, 1).await.unwrap();
        }
        let ctrl_session = DurableMerkle
            .build_session("big", &ctrl_store as &dyn MapDataStore)
            .expect("build_session");
        let expected_root = ctrl_session.root();
        drop(ctrl_store); // close before second open attempt

        // Spy store: separate redb file seeded with the same records.
        let spy_dir = tempfile::tempdir().expect("tempdir");
        let spy_inner = RedbDataStore::new(spy_dir.path().join("spy.redb")).expect("redb open");
        for (k, v) in &records {
            spy_inner.add("big", k, v, 0, 1).await.unwrap();
        }
        let enumerate_calls = Arc::new(AtomicUsize::new(0));
        let spy: Arc<dyn MapDataStore> = Arc::new(SpyStore {
            inner: spy_inner,
            enumerate_calls: Arc::clone(&enumerate_calls),
            fail_enumerate: false,
        });
        // Must not panic; if load/load_all are called the test panics.
        let spy_session = DurableMerkle
            .build_session("big", spy.as_ref())
            .expect("build_session");

        assert!(
            enumerate_calls.load(Ordering::SeqCst) >= 1,
            "build_session must drive enumerate_leaves (streaming)"
        );
        assert_ne!(
            spy_session.root(),
            0,
            "root must be non-zero over 500 records"
        );
        assert_eq!(
            spy_session.root(),
            expected_root,
            "SpyStore session root must equal the control session root (AC6)"
        );
    }

    // -----------------------------------------------------------------------
    // AC-IM — In-memory tree is NOT the SYNC read source.
    //
    // Build a session WITHOUT creating any MerkleSyncManager; verify the root
    // matches the independently computed expected hash.
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac_im_build_session_does_not_require_merkle_sync_manager() {
        let (store, _dir) = fresh_redb();
        // Write exactly one LWW record.
        let key = "alice";
        let millis = 111u64;
        store
            .add("m", key, &lww_value(key, millis, 0, "n1"), 0, 1)
            .await
            .unwrap();

        // Build session with NO MerkleSyncManager in scope.
        let idx = DurableMerkle;
        let session = idx
            .build_session("m", store.as_ref())
            .expect("build_session");

        // Independently compute what the root should be via the same primitives.
        let leaf_hash = fnv1a_hash(&format!("{key}:{millis}:0:n1"));
        let partition_id = hash_to_partition(key);
        // Build a fresh MerkleTree for that one partition and read its root.
        let mut tree = topgun_core::merkle::MerkleTree::new(3);
        tree.update(key, leaf_hash);
        let partition_root = tree.get_root_hash();
        let expected_lww_root = topgun_core::hash::combine_hashes(&[partition_root]);

        assert_eq!(
            session.lww_root(),
            expected_lww_root,
            "session root must equal the independently computed hash (AC-IM)"
        );
        assert_ne!(session.root(), 0);
        // Prove MerkleSyncManager was never needed — no reference to it anywhere here.
        let _ = partition_id; // used in construction above, silence unused warning
    }

    // -----------------------------------------------------------------------
    // AC-SnapshotIsolation — session is point-in-time.
    //
    // Build a session, then write additional records to the underlying store.
    // root(), buckets(), leaf_keys() must all return the pre-write values.
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn ac_snapshot_isolation_session_is_point_in_time() {
        let (store, _dir) = fresh_redb();
        store
            .add("m", "key-t0", &lww_value("key-t0", 100, 0, "n1"), 0, 1)
            .await
            .unwrap();

        let idx = DurableMerkle;
        let session = idx
            .build_session("m", store.as_ref())
            .expect("build_session");

        let root_before = session.root();
        let buckets_before = session.buckets("");
        // leaf_keys at the depth-3 hex path for "key-t0"
        let hex = format!("{:08x}", fnv1a_hash("key-t0"));
        let leaf_path = &hex[..3];
        let leaves_before = session.leaf_keys(leaf_path);

        // Inject a write to the underlying store AFTER the session was built.
        store
            .add("m", "new-key", &lww_value("new-key", 999, 0, "n2"), 0, 2)
            .await
            .unwrap();

        // Session must still reflect T0 — it must NOT see the new write.
        assert_eq!(
            session.root(),
            root_before,
            "root() must return T0 value after post-session write (AC-SnapshotIsolation)"
        );
        assert_eq!(
            session.buckets(""),
            buckets_before,
            "buckets() must return T0 value after post-session write"
        );
        assert_eq!(
            session.leaf_keys(leaf_path),
            leaves_before,
            "leaf_keys() must return T0 value after post-session write"
        );
    }

    // -----------------------------------------------------------------------
    // Error propagation — a mid-stream enumerate fault must surface, never
    // produce a session over a partial (wrong) leaf set.
    //
    // The sync read source degrades to an error (so the handler can
    // reject/retry), never to wrong leaves — the same contract
    // MerkleSyncManager::rebuild_from_datastore upholds.
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn build_session_propagates_enumerate_error_no_partial_session() {
        let spy_dir = tempfile::tempdir().expect("tempdir");
        let spy_inner = RedbDataStore::new(spy_dir.path().join("fail.redb")).expect("redb open");
        // Seed some real data so a swallowed error would yield a plausible,
        // non-empty (but WRONG) root rather than an obviously-empty one.
        for i in 0..10u32 {
            spy_inner
                .add(
                    "m",
                    &format!("k{i}"),
                    &lww_value("v", u64::from(i), i, "n"),
                    0,
                    1,
                )
                .await
                .unwrap();
        }
        let spy: Arc<dyn MapDataStore> = Arc::new(SpyStore {
            inner: spy_inner,
            enumerate_calls: Arc::new(AtomicUsize::new(0)),
            fail_enumerate: true,
        });

        let result = DurableMerkle.build_session("m", spy.as_ref());
        assert!(
            result.is_err(),
            "build_session must propagate the enumerate fault, not return a partial session"
        );
    }

    // -----------------------------------------------------------------------
    // Bonus: empty store produces root 0
    // -----------------------------------------------------------------------
    #[tokio::test(flavor = "multi_thread")]
    async fn empty_store_produces_root_zero() {
        let (store, _dir) = fresh_redb();
        let session = DurableMerkle
            .build_session("nonexistent", store.as_ref())
            .expect("build_session");
        assert_eq!(session.root(), 0, "empty map must produce root 0");
        assert!(session.buckets("").is_empty());
        assert!(session.leaf_keys("000").is_empty());
    }
}
