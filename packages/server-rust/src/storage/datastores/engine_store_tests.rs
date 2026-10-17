//! Main tests of [`EngineMapDataStore`]: one suite, run unchanged over the
//! in-memory reference engine and over a file-backed redb engine.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use topgun_core::hlc::Timestamp;
use topgun_core::partition::{hash_to_partition, PARTITION_COUNT};
use topgun_core::types::Value;

use super::engine_store::{
    decode_map_by_id_row, decode_map_by_name_row, map_by_id_key, map_by_name_key, map_id_in,
    meta_key, EngineMapDataStore, RecordKey, RecordOp, RecordSpace, FLAG_BACKUP, FLAG_PRIMARY,
    FORMAT_VERSION, KS_BACKUP, KS_MAP_BY_ID, KS_MAP_BY_NAME, KS_META, KS_PRIMARY, META_FORMAT,
    PARTITION_THEN_KEY,
};
use super::kv_engine::{
    KeyRange, KvCapabilities, KvCapability, KvEngine, KvError, KvResult, KvSnapshot, MemKvEngine,
    WriteBatch, WriteOptions,
};
use super::{WriteBehindConfig, WriteBehindDataStore};
use crate::storage::map_data_store::{
    merkle_leaf_hash, LeafSink, MapDataStore, MerkleLeaf, ScanBatch, ScanCursor, StoreBrokenCause,
    StoreErrorClass, WriteSource,
};
use crate::storage::record::{OrMapEntry, RecordValue};

const SYNC: WriteOptions = WriteOptions { sync: true };

/// How a case gets a fresh engine.
pub(super) trait EngineKind {
    type E: KvEngine;
    fn fresh() -> Fresh<Self::E>;
}

/// A fresh engine; the directory of a file-backed one lives as long as it.
pub(super) struct Fresh<E> {
    pub(super) engine: Arc<E>,
    _dir: Option<tempfile::TempDir>,
}

pub(super) struct Mem;

impl EngineKind for Mem {
    type E = MemKvEngine;
    fn fresh() -> Fresh<MemKvEngine> {
        Fresh {
            engine: Arc::new(MemKvEngine::new()),
            _dir: None,
        }
    }
}

#[cfg(feature = "redb")]
pub(super) struct Redb;

#[cfg(feature = "redb")]
impl EngineKind for Redb {
    type E = super::RedbKvEngine;
    fn fresh() -> Fresh<super::RedbKvEngine> {
        let dir = tempfile::tempdir().expect("tempdir");
        let engine = super::RedbKvEngine::open(&dir.path().join("engine.redb")).expect("open");
        Fresh {
            engine: Arc::new(engine),
            _dir: Some(dir),
        }
    }
}

/// Forwards every call to `E`, counts `write` calls, and can fail the n-th
/// `write` from the moment it is armed, once, without forwarding it.
pub(super) struct FailingWrites<E: KvEngine> {
    inner: Arc<E>,
    writes: AtomicU64,
    fail_at: Mutex<Option<u64>>,
    other_mutations: AtomicU64,
}

impl<E: KvEngine> FailingWrites<E> {
    pub(super) fn new(inner: Arc<E>) -> Self {
        Self {
            inner,
            writes: AtomicU64::new(0),
            fail_at: Mutex::new(None),
            other_mutations: AtomicU64::new(0),
        }
    }

    /// The `n`-th `write` call from now fails (`n >= 1`).
    pub(super) fn arm(&self, n: u64) {
        assert!(n >= 1, "arm(n) takes n >= 1");
        *self.fail_at.lock() = Some(self.writes.load(Ordering::SeqCst) + n);
    }

    pub(super) fn disarm(&self) {
        *self.fail_at.lock() = None;
    }

    /// `delete`, `delete_range` and `ingest` calls; the store makes none.
    pub(super) fn other_mutations(&self) -> u64 {
        self.other_mutations.load(Ordering::SeqCst)
    }
}

impl<E: KvEngine> KvEngine for FailingWrites<E> {
    fn capabilities(&self) -> KvCapabilities {
        self.inner.capabilities()
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        let index = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut fail_at = self.fail_at.lock();
            if *fail_at == Some(index) {
                *fail_at = None;
                return Err(KvError::Other(anyhow::anyhow!("injected write failure")));
            }
        }
        self.inner.write(batch, opts)
    }

    fn delete(&self, key: &[u8], opts: WriteOptions) -> KvResult<()> {
        self.other_mutations.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(key, opts)
    }

    fn delete_range(&self, start: &[u8], end: &[u8], opts: WriteOptions) -> KvResult<()> {
        self.other_mutations.fetch_add(1, Ordering::SeqCst);
        self.inner.delete_range(start, end, opts)
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        self.inner.snapshot()
    }

    fn ingest(
        &self,
        range: KeyRange,
        sorted: Vec<(Vec<u8>, Vec<u8>)>,
        opts: WriteOptions,
    ) -> KvResult<()> {
        self.other_mutations.fetch_add(1, Ordering::SeqCst);
        self.inner.ingest(range, sorted, opts)
    }

    fn sync(&self) -> KvResult<()> {
        self.inner.sync()
    }

    fn shutdown(&self) -> KvResult<()> {
        self.inner.shutdown()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn lww(value: &str, millis: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::String(value.to_string()),
        timestamp: Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        },
    }
}

fn or_value(tags: &[&str], tombstones: &[&str]) -> RecordValue {
    RecordValue::OrMap {
        records: tags
            .iter()
            .map(|tag| OrMapEntry {
                value: Value::String(format!("v-{tag}")),
                tag: (*tag).to_string(),
                timestamp: Timestamp {
                    millis: 5,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            })
            .collect(),
        tombstones: tombstones.iter().map(|t| (*t).to_string()).collect(),
    }
}

fn enc(value: &RecordValue) -> Vec<u8> {
    rmp_serde::to_vec_named(value).expect("encode")
}

fn dec(bytes: &[u8]) -> RecordValue {
    rmp_serde::from_slice(bytes).expect("decode")
}

/// Every key and value of the engine, ascending.
fn dump<E: KvEngine + ?Sized>(engine: &E) -> Vec<(Vec<u8>, Vec<u8>)> {
    let snapshot = engine.snapshot().expect("snapshot");
    let rows = snapshot
        .iter(KeyRange::default())
        .expect("iter")
        .collect::<KvResult<Vec<_>>>()
        .expect("rows");
    rows
}

fn keys_in_space<E: KvEngine + ?Sized>(engine: &E, first_byte: u8) -> Vec<Vec<u8>> {
    dump(engine)
        .into_iter()
        .filter(|(k, _)| k.first() == Some(&first_byte))
        .map(|(k, _)| k)
        .collect()
}

fn raw_put<E: KvEngine + ?Sized>(engine: &E, rows: &[(Vec<u8>, Vec<u8>)]) {
    let mut batch = WriteBatch::new();
    for (k, v) in rows {
        batch.put(k.clone(), v.clone());
    }
    engine.write(batch, SYNC).expect("raw write");
}

fn meta(name: &[u8]) -> Vec<u8> {
    let mut key = vec![0x00];
    key.extend_from_slice(name);
    key
}

/// The three metadata rows of a formatted store, with `next` as `next_map_id`.
fn meta_rows(format: u32, ack_mode: u8, next: u32) -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![
        (meta(b"format"), format.to_be_bytes().to_vec()),
        (meta(b"ack_mode"), vec![ack_mode]),
        (meta(b"next_map_id"), next.to_be_bytes().to_vec()),
    ]
}

fn id_of<E: KvEngine + ?Sized>(engine: &E, name: &str) -> Option<u32> {
    let snapshot = engine.snapshot().expect("snapshot");
    map_id_in(&*snapshot, name).expect("map_id_in")
}

fn next_map_id<E: KvEngine + ?Sized>(engine: &E) -> u32 {
    let bytes = engine
        .get(&meta(b"next_map_id"))
        .expect("get")
        .expect("next_map_id row");
    u32::from_be_bytes(bytes.try_into().expect("4 bytes"))
}

fn flags_of<E: KvEngine + ?Sized>(engine: &E, name: &str) -> Option<u8> {
    let mut key = vec![0x01];
    key.extend_from_slice(name.as_bytes());
    engine
        .get(&key)
        .expect("get")
        .map(|v| *v.last().expect("5 bytes"))
}

fn open<E: KvEngine>(engine: &Arc<E>) -> EngineMapDataStore<E> {
    EngineMapDataStore::open(Arc::clone(engine)).expect("open")
}

fn put<'a>(map: &'a str, key: &'a str, bytes: &'a [u8]) -> RecordOp<'a> {
    RecordOp::Put {
        map,
        backup: false,
        key,
        bytes,
    }
}

fn position(key: &str) -> Vec<u8> {
    (PARTITION_THEN_KEY.key_position)(key)
}

fn error_text<T>(result: &anyhow::Result<T>) -> String {
    match result {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// Writes `keys` of `map` (primary) in one call, each holding `lww(key)`.
fn write_keys<E: KvEngine>(store: &EngineMapDataStore<E>, map: &str, keys: &[String]) {
    let values: Vec<Vec<u8>> = keys.iter().map(|k| enc(&lww(k, 1))).collect();
    for chunk in (0..keys.len()).collect::<Vec<_>>().chunks(5_000) {
        let ops: Vec<RecordOp<'_>> = chunk
            .iter()
            .map(|&i| put(map, &keys[i], &values[i]))
            .collect();
        store.write_records(&ops, SYNC).expect("write_records");
    }
}

/// `n` keys `{prefix}{i}`.
fn numbered(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i}")).collect()
}

/// Keys `k{i}` until every partition holds at least `per_partition`.
fn keys_covering_every_partition(per_partition: usize) -> Vec<String> {
    let mut counts = vec![0usize; PARTITION_COUNT as usize];
    let mut keys = Vec::new();
    let mut i = 0u64;
    while counts.iter().any(|&c| c < per_partition) {
        let key = format!("k{i}");
        counts[hash_to_partition(&key) as usize] += 1;
        keys.push(key);
        i += 1;
    }
    keys
}

/// A full scan: every batch with the cursor it returned.
async fn scan_batches(
    store: &dyn MapDataStore,
    map: &str,
    backup: bool,
    budget: u64,
) -> Vec<ScanBatch> {
    let mut out = Vec::new();
    let mut batch = store.scan_values(map, backup, budget).await.expect("scan");
    loop {
        let cursor = batch.next_cursor.clone();
        out.push(batch);
        match cursor {
            Some(c) => {
                // Positions must rise strictly from cursor to cursor, so a store
                // that re-emits rows fails here instead of looping for ever.
                let at = (PARTITION_THEN_KEY.cursor_position)(&c).expect("cursor position");
                batch = store
                    .scan_values_batched(map, backup, c, budget)
                    .await
                    .expect("resumed scan");
                assert!(
                    batch.records.iter().all(|(k, _)| position(k) > at),
                    "resumed batch does not start after its cursor"
                );
                if let Some(next) = &batch.next_cursor {
                    let next_at =
                        (PARTITION_THEN_KEY.cursor_position)(next).expect("cursor position");
                    assert!(next_at > at, "cursor did not advance");
                }
            }
            None => return out,
        }
    }
}

fn batch_keys(batches: &[ScanBatch]) -> Vec<String> {
    batches
        .iter()
        .flat_map(|b| b.records.iter().map(|(k, _)| k.clone()))
        .collect()
}

fn sorted_by_position(mut keys: Vec<String>) -> Vec<String> {
    keys.sort_by_key(|k| position(k));
    keys
}

fn assert_strictly_ascending(keys: &[String], what: &str) {
    for pair in keys.windows(2) {
        assert!(
            position(&pair[0]) < position(&pair[1]),
            "{what}: {:?} is not strictly before {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[derive(Default)]
struct CollectingSink {
    leaves: Vec<MerkleLeaf>,
    batch_sizes: Vec<usize>,
}

#[async_trait]
impl LeafSink for CollectingSink {
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
        self.batch_sizes.push(batch.len());
        self.leaves.extend(batch);
        Ok(())
    }
}

fn leaf_triples(leaves: &[MerkleLeaf]) -> Vec<(String, u32)> {
    leaves
        .iter()
        .map(|leaf| (leaf.key.clone(), leaf.leaf_hash))
        .collect()
}

/// A tiny deterministic generator, so a failing run can be replayed.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_key(rng: &mut XorShift) -> String {
    const ALPHABET: [&str; 10] = ["a", "z", "0", "-", "/", "é", "ж", "日", "🙂", "\u{7f}"];
    let len = rng.below(9) as usize;
    let ascii_only = rng.below(2) == 0;
    (0..len)
        .map(|_| {
            if ascii_only {
                char::from(b' ' + rng.below(95) as u8).to_string()
            } else {
                ALPHABET[rng.below(ALPHABET.len() as u64) as usize].to_string()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

pub(super) mod cases {
    use super::*;

    pub(in super::super) fn s01<K: EngineKind>() {
        assert_eq!(
            topgun_core::hash::fnv1a_hash("") % 271,
            199,
            "the hand-computed partition of the empty key is wrong"
        );
        let vectors: [(&str, u32, RecordSpace, &[u8]); 3] = [
            (
                "hello",
                1,
                RecordSpace::Primary,
                &[
                    0x10, 0x00, 0x00, 0x00, 0x5F, 0x00, 0x00, 0x00, 0x01, 0x68, 0x65, 0x6C, 0x6C,
                    0x6F,
                ],
            ),
            (
                "key1",
                2,
                RecordSpace::Backup,
                &[
                    0x11, 0x00, 0x00, 0x01, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x6B, 0x65, 0x79, 0x31,
                ],
            ),
            (
                "",
                7,
                RecordSpace::Primary,
                &[0x10, 0x00, 0x00, 0x00, 0xC7, 0x00, 0x00, 0x00, 0x07],
            ),
        ];
        for (key, map_id, space, bytes) in vectors {
            assert_eq!(RecordKey::encode(space, map_id, key), bytes, "{key:?}");
            let decoded = RecordKey::decode(bytes).expect("decode");
            assert_eq!(
                decoded,
                RecordKey {
                    space,
                    partition: u32::from_be_bytes(bytes[1..5].try_into().unwrap()),
                    map_id,
                    key: key.to_string(),
                }
            );
        }
        // The store writes the same bytes: the first map gets id 1.
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let value = enc(&lww("v", 1));
        store
            .write_records(&[put("m", "hello", &value)], SYNC)
            .expect("write");
        assert_eq!(
            keys_in_space(&*fresh.engine, KS_PRIMARY),
            vec![vectors[0].3.to_vec()]
        );
    }

    fn triple() -> impl Strategy<Value = (bool, u32, String)> {
        (any::<bool>(), any::<u32>(), "\\PC{0,8}")
    }

    fn space_of(backup: bool) -> RecordSpace {
        if backup {
            RecordSpace::Backup
        } else {
            RecordSpace::Primary
        }
    }

    pub(in super::super) fn s02<K: EngineKind>() {
        let mut runner = TestRunner::new(Config::with_cases(512));
        runner
            .run(&(triple(), triple()), |(a, b)| {
                let ea = RecordKey::encode(space_of(a.0), a.1, &a.2);
                let eb = RecordKey::encode(space_of(b.0), b.1, &b.2);
                prop_assert_eq!(a == b, ea == eb);
                for (t, e) in [(&a, &ea), (&b, &eb)] {
                    let decoded = RecordKey::decode(e).expect("decode");
                    prop_assert_eq!(
                        decoded,
                        RecordKey {
                            space: space_of(t.0),
                            partition: hash_to_partition(&t.2),
                            map_id: t.1,
                            key: t.2.clone(),
                        }
                    );
                }
                Ok(())
            })
            .unwrap();
    }

    fn prefix(space: RecordSpace, partition: u32, map_id: u32) -> Vec<u8> {
        let mut p = vec![space.byte()];
        p.extend_from_slice(&partition.to_be_bytes());
        p.extend_from_slice(&map_id.to_be_bytes());
        p
    }

    pub(in super::super) fn s03<K: EngineKind>() {
        let strategy = (
            any::<bool>(),
            any::<bool>(),
            any::<u32>(),
            0u8..3,
            "\\PC{0,6}",
            "\\PC{0,3}",
            0u8..3,
        );
        let mut runner = TestRunner::new(Config::with_cases(512));
        runner
            .run(
                &strategy,
                |(backup_a, backup_b, id_a, id_rel, key_a, tail, key_rel)| {
                    // Ids that differ only in the low byte, and keys that are
                    // prefixes of each other, are generated on purpose.
                    let id_b = match id_rel {
                        0 => id_a,
                        1 => id_a ^ 1,
                        _ => id_a.wrapping_add(256),
                    };
                    let key_b = match key_rel {
                        0 => key_a.clone(),
                        1 => format!("{key_a}{tail}"),
                        _ => tail.clone(),
                    };
                    let (sa, sb) = (space_of(backup_a), space_of(backup_b));
                    let (pa, pb) = (hash_to_partition(&key_a), hash_to_partition(&key_b));
                    let eb = RecordKey::encode(sb, id_b, &key_b);
                    let starts = eb.starts_with(&prefix(sa, pa, id_a));
                    prop_assert_eq!(starts, sa == sb && pa == pb && id_a == id_b);
                    Ok(())
                },
            )
            .unwrap();
    }

    pub(in super::super) fn s04<K: EngineKind>() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        let mut keys: Vec<String> = (0..10_000).map(|_| random_key(&mut rng)).collect();
        keys.push(String::new());
        for key in &keys {
            let bytes = RecordKey::encode(RecordSpace::Primary, 1, key);
            assert_eq!(
                bytes[1..5],
                hash_to_partition(key).to_be_bytes(),
                "partition bytes of {key:?}"
            );
            assert_eq!(
                RecordKey::decode(&bytes).expect("decode").partition,
                hash_to_partition(key)
            );
        }
        // And through the store's own write path.
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let some: Vec<String> = keys
            .iter()
            .take(300)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        write_keys(&store, "m", &some);
        let written = keys_in_space(&*fresh.engine, KS_PRIMARY);
        assert_eq!(written.len(), some.len());
        for key in written {
            let user = std::str::from_utf8(&key[9..]).expect("utf8");
            assert_eq!(key[1..5], hash_to_partition(user).to_be_bytes());
        }
    }

    pub(in super::super) async fn s05<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        for map in ["a", "b", "c"] {
            store.add(map, "k", &lww("v", 1), 0, 0).await.expect("add");
        }
        let ids: Vec<Option<u32>> = ["a", "b", "c"]
            .iter()
            .map(|m| id_of(&*fresh.engine, m))
            .collect();
        assert_eq!(ids, vec![Some(1), Some(2), Some(3)]);
        drop(store);
        let reopened = open(&fresh.engine);
        reopened
            .add("d", "k", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        assert_eq!(id_of(&*fresh.engine, "d"), Some(4));
        assert_eq!(next_map_id(&*fresh.engine), 5);
    }

    fn no_registry_or_records<E: KvEngine + ?Sized>(engine: &E) -> bool {
        keys_in_space(engine, KS_MAP_BY_NAME).is_empty()
            && keys_in_space(engine, KS_MAP_BY_ID).is_empty()
            && keys_in_space(engine, KS_PRIMARY).is_empty()
    }

    pub(in super::super) async fn s06<K: EngineKind>() {
        for n in [1u64, 2] {
            let fresh = K::fresh();
            let failing = Arc::new(FailingWrites::new(Arc::clone(&fresh.engine)));
            let store = open(&failing);
            failing.arm(n);
            let outcome = store.add("users", "k", &lww("v", 1), 0, 0).await;
            if n == 1 {
                assert!(outcome.is_err(), "the failure path must be exercised");
            }
            match outcome {
                Err(_) => {
                    assert!(store.list_maps().await.expect("list").is_empty(), "n={n}");
                    assert!(no_registry_or_records(&*fresh.engine), "n={n}");
                    assert_eq!(next_map_id(&*fresh.engine), 1, "n={n}");
                    failing.disarm();
                    store
                        .add("users", "k", &lww("v", 1), 0, 0)
                        .await
                        .expect("un-armed add");
                    assert_eq!(id_of(&*fresh.engine, "users"), Some(1), "n={n}");
                }
                Ok(()) => {
                    assert!(store.load("users", "k").await.expect("load").is_some());
                    assert_eq!(store.list_maps().await.expect("list"), vec!["users"]);
                    assert_eq!(id_of(&*fresh.engine, "users"), Some(1), "n={n}");
                }
            }
            assert_eq!(failing.other_mutations(), 0);
        }

        for n in [1u64, 2] {
            let fresh = K::fresh();
            let failing = Arc::new(FailingWrites::new(Arc::clone(&fresh.engine)));
            let store = open(&failing);
            let value = enc(&lww("v", 1));
            failing.arm(n);
            let outcome = store.write_records(
                &[
                    put("m1", "a", &value),
                    put("m2", "b", &value),
                    put("m3", "c", &value),
                ],
                SYNC,
            );
            if n == 1 {
                assert!(outcome.is_err(), "the failure path must be exercised");
            }
            match outcome {
                Err(_) => {
                    assert!(no_registry_or_records(&*fresh.engine), "n={n}");
                    assert_eq!(next_map_id(&*fresh.engine), 1, "n={n}");
                }
                Ok(()) => {
                    for (map, key) in [("m1", "a"), ("m2", "b"), ("m3", "c")] {
                        assert!(id_of(&*fresh.engine, map).is_some(), "{map}");
                        assert!(store.load(map, key).await.expect("load").is_some());
                    }
                }
            }
            assert_eq!(failing.other_mutations(), 0);
        }
    }

    pub(in super::super) fn s07<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let value = enc(&lww("v", 1));
        let (reached, release) = store.arm_registry_park();
        let (outcome_a, outcome_b) = std::thread::scope(|scope| {
            // Owned here, so a failing assertion drops it and A continues
            // before the scope joins.
            let release = release;
            let store = &store;
            let value = &value;
            let a = scope.spawn(move || store.write_records(&[put("ma", "a", value)], SYNC));
            reached
                .recv_timeout(Duration::from_secs(10))
                .expect("A never reached the park");
            let (b_done_tx, b_done) = channel();
            let b = scope.spawn(move || {
                let outcome = store.write_records(&[put("mb", "b", value)], SYNC);
                let _ = b_done_tx.send(());
                outcome
            });
            assert_eq!(
                b_done.recv_timeout(Duration::from_secs(1)),
                Err(RecvTimeoutError::Timeout),
                "B returned while A was parked holding the registry lock"
            );
            release.send(()).expect("release");
            (a.join().expect("A"), b.join().expect("B"))
        });
        outcome_a.expect("A");
        outcome_b.expect("B");
        assert_eq!(id_of(&*fresh.engine, "ma"), Some(1));
        assert_eq!(id_of(&*fresh.engine, "mb"), Some(2));
        let by_id: Vec<(u32, String)> = dump(&*fresh.engine)
            .into_iter()
            .filter(|(k, _)| k.first() == Some(&KS_MAP_BY_ID))
            .map(|(k, v)| decode_map_by_id_row(&k, &v).expect("row"))
            .collect();
        assert_eq!(by_id, vec![(1, "ma".to_string()), (2, "mb".to_string())]);
        assert_eq!(next_map_id(&*fresh.engine), 3);
        block_on(async {
            assert!(store.load("ma", "a").await.expect("load").is_some());
            assert!(store.load("mb", "b").await.expect("load").is_some());
        });
    }

    pub(in super::super) async fn s08<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        store
            .add_backup("bk", "k", &lww("v", 1), 0, 0)
            .await
            .expect("add_backup");
        assert!(store.list_maps().await.expect("list").is_empty());
        assert_eq!(flags_of(&*fresh.engine, "bk"), Some(FLAG_BACKUP));
        let before = dump(&*fresh.engine);
        store.remove("bk", "k", 0).await.expect("remove");
        assert!(store.list_maps().await.expect("list").is_empty());
        assert_eq!(flags_of(&*fresh.engine, "bk"), Some(FLAG_BACKUP));
        assert_eq!(
            dump(&*fresh.engine),
            before,
            "a remove of an absent primary"
        );

        for key in ["k1", "k2"] {
            store.add("em", key, &lww("v", 1), 0, 0).await.expect("add");
        }
        for key in ["k1", "k2"] {
            store.remove("em", key, 0).await.expect("remove");
        }
        assert!(store.load("em", "k1").await.expect("load").is_none());
        assert_eq!(store.list_maps().await.expect("list"), vec!["em"]);
    }

    pub(in super::super) async fn s09<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let before = dump(&*fresh.engine).len();
        for i in 0..5_000 {
            let name = format!("absent-{i}");
            assert!(store.load(&name, "k").await.expect("load").is_none());
        }
        assert_eq!(store.registry_cache_len(), 0);
        assert_eq!(dump(&*fresh.engine).len(), before);
    }

    fn refused<T>(result: anyhow::Result<T>, what: &str) {
        let text = error_text(&result);
        assert!(
            text.contains("Invalid map name"),
            "{what}: expected a name refusal, got {text:?}"
        );
    }

    pub(in super::super) async fn s10<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        store
            .add("foo", "k", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        store
            .add_backup("foo", "k", &lww("b", 1), 0, 0)
            .await
            .expect("add_backup");
        let before = dump(&*fresh.engine);
        let long_backup = format!("{}__backup", "x".repeat(592));
        assert_eq!(long_backup.len(), 600);
        let value = lww("v", 1);
        let bytes = enc(&value);
        let cursor = ScanCursor(RecordKey::encode(RecordSpace::Primary, 1, "k"));
        for name in [
            "",
            "foo__backup",
            "a-b__backup",
            "a\0b",
            long_backup.as_str(),
        ] {
            refused(store.add(name, "k", &value, 0, 0).await, "add");
            refused(
                store.add_encoded(name, "k", &bytes, 0, 0).await,
                "add_encoded",
            );
            refused(
                store.add_backup(name, "k", &value, 0, 0).await,
                "add_backup",
            );
            refused(
                store
                    .add_with_witness(name, "k", WriteSource::Value(&value), 0, 0, None)
                    .await,
                "add_with_witness",
            );
            refused(store.remove(name, "k", 0).await, "remove");
            refused(store.remove_backup(name, "k", 0).await, "remove_backup");
            refused(store.load(name, "k").await, "load");
            refused(store.load_slot(name, "k").await, "load_slot");
            refused(store.load_all(name, &["k".to_string()]).await, "load_all");
            refused(
                store.remove_all(name, &["k".to_string()]).await,
                "remove_all",
            );
            refused(store.remove_all(name, &[]).await, "remove_all empty");
            for backup in [false, true] {
                refused(store.scan_values(name, backup, 0).await, "scan_values");
                refused(
                    store
                        .scan_values_batched(name, backup, cursor.clone(), 0)
                        .await,
                    "scan_values_batched",
                );
                let mut sink = CollectingSink::default();
                refused(
                    store.enumerate_leaves(name, backup, &mut sink).await,
                    "enumerate_leaves",
                );
                assert!(sink.leaves.is_empty());
                refused(
                    store.visit_records(name, backup, &mut |_, _| Ok(())),
                    "visit_records",
                );
                for op in [
                    RecordOp::Put {
                        map: name,
                        backup,
                        key: "k",
                        bytes: &bytes,
                    },
                    RecordOp::Delete {
                        map: name,
                        backup,
                        key: "k",
                    },
                    RecordOp::Touch { map: name, backup },
                ] {
                    refused(store.write_records(&[op], SYNC), "write_records");
                    // A refused op refuses the whole call, a valid op beside it included.
                    refused(
                        store.write_records(&[put("foo", "other", &bytes), op], SYNC),
                        "write_records with a valid op",
                    );
                }
            }
        }
        assert_eq!(
            dump(&*fresh.engine),
            before,
            "a refused name reached the engine"
        );

        let long = "y".repeat(600);
        store
            .add(&long, "k", &value, 0, 0)
            .await
            .expect("long name");
        assert!(store.load(&long, "k").await.expect("load").is_some());
    }

    async fn walk_and_check(store: &dyn MapDataStore, map: &str, written: &BTreeSet<String>) {
        for budget in [1u64, 100, 0] {
            let batches = scan_batches(store, map, false, budget).await;
            let keys = batch_keys(&batches);
            assert_strictly_ascending(&keys, &format!("budget {budget}"));
            assert_eq!(keys.len(), written.len(), "budget {budget}: each key once");
            assert_eq!(
                keys.iter().cloned().collect::<BTreeSet<_>>(),
                *written,
                "budget {budget}"
            );
        }
    }

    pub(in super::super) async fn s11<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let keys = keys_covering_every_partition(2);
        let partitions: Vec<u32> = keys.iter().map(|k| hash_to_partition(k)).collect();
        assert!(partitions.iter().filter(|&&p| p == 0).count() >= 2);
        assert!(partitions.iter().filter(|&&p| p == 270).count() >= 2);
        write_keys(&store, "m", &keys);
        walk_and_check(&store, "m", &keys.iter().cloned().collect()).await;
    }

    pub(in super::super) async fn s12<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let keys = numbered("c", 600);
        write_keys(&store, "m", &keys);
        let map_id = id_of(&*fresh.engine, "m").expect("id");
        let all = sorted_by_position(keys.clone());

        for budget in [1u64, 100] {
            let batches = scan_batches(&store, "m", false, budget).await;
            let rows = batch_keys(&batches);
            // Clause 1: strictly ascending across batches.
            assert_strictly_ascending(&rows, "rows");
            let mut lower: Option<Vec<u8>> = None;
            for batch in &batches {
                let keys: Vec<String> = batch.records.iter().map(|(k, _)| k.clone()).collect();
                // Clause 3: a resumed batch holds only rows above its cursor.
                if let Some(lower) = &lower {
                    assert!(keys.iter().all(|k| position(k) > *lower));
                }
                let upper = match &batch.next_cursor {
                    Some(c) => {
                        let last = keys.last().expect("a batch with a cursor has a row");
                        // Clause 2, and the cursor is the engine key of the last row.
                        assert_eq!(c.0, RecordKey::encode(RecordSpace::Primary, map_id, last));
                        let at = (PARTITION_THEN_KEY.cursor_position)(c).expect("position");
                        assert_eq!(at, position(last));
                        Some(at)
                    }
                    None => None,
                };
                // Clause 4: every row of the range is in the batch.
                let expected: Vec<String> = all
                    .iter()
                    .filter(|k| lower.as_ref().is_none_or(|l| position(k) > *l))
                    .filter(|k| upper.as_ref().is_none_or(|u| position(k) <= *u))
                    .cloned()
                    .collect();
                assert_eq!(keys, expected);
                lower = upper;
            }
            assert_eq!(rows, all);
            if budget == 1 {
                let parts: Vec<u32> = rows.iter().map(|k| hash_to_partition(k)).collect();
                assert!(
                    parts.windows(2).any(|w| w[0] == w[1]),
                    "a boundary inside one partition"
                );
                assert!(
                    parts.windows(2).any(|w| w[0] != w[1]),
                    "a boundary between partitions"
                );
            }
        }

        // A resume from the first key of a partition holding two keys starts
        // at its second key.
        let pair = all
            .windows(2)
            .find(|w| hash_to_partition(&w[0]) == hash_to_partition(&w[1]))
            .expect("a partition with two keys");
        let cursor = ScanCursor(RecordKey::encode(RecordSpace::Primary, map_id, &pair[0]));
        let resumed = store
            .scan_values_batched("m", false, cursor, 1)
            .await
            .expect("resume");
        assert_eq!(batch_keys(&[resumed]), vec![pair[1].clone()]);
    }

    pub(in super::super) async fn s13<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let payload = lww("payload", 1);
        for i in 0..20 {
            store
                .add("m", &format!("k{i:03}"), &payload, 0, 0)
                .await
                .expect("add");
        }
        let one_cost = enc(&payload).len() as u64;
        let budget = one_cost * 3;
        let batches = scan_batches(&store, "m", false, budget).await;
        for batch in &batches {
            let cost: u64 = batch.records.iter().map(|(_, v)| enc(v).len() as u64).sum();
            assert!(cost <= budget, "batch cost {cost} over budget {budget}");
            assert!(!batch.records.is_empty(), "each batch makes progress");
        }
        assert_eq!(batch_keys(&batches).len(), 20);
        assert!(batches.len() >= 7);

        let big = lww(&"x".repeat(4096), 1);
        store.add("big", "only", &big, 0, 0).await.expect("add");
        let batch = store.scan_values("big", false, 1).await.expect("scan");
        assert_eq!(batch_keys(&[batch]), vec!["only".to_string()]);
        let batch = store.scan_values("big", false, 1).await.expect("scan");
        assert!(batch.next_cursor.is_none(), "one record drains the map");

        store.add("two", "a", &big, 0, 0).await.expect("add");
        store.add("two", "b", &big, 0, 0).await.expect("add");
        let first = store.scan_values("two", false, 1).await.expect("scan");
        assert_eq!(first.records.len(), 1, "an oversized first row comes whole");
        assert!(first.next_cursor.is_some());
    }

    pub(in super::super) async fn s14<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let mut expected: Vec<(String, RecordValue)> = Vec::new();
        for i in 0..1_500 {
            expected.push((format!("l{i}"), lww(&format!("v{i}"), i)));
        }
        expected.push(("or-live".to_string(), or_value(&["t1", "t2"], &["t0"])));
        expected.push(("or-tomb".to_string(), or_value(&[], &["t9"])));
        expected.push(("or-empty".to_string(), or_value(&[], &[])));
        let bytes: Vec<Vec<u8>> = expected.iter().map(|(_, v)| enc(v)).collect();
        let ops: Vec<RecordOp<'_>> = expected
            .iter()
            .zip(&bytes)
            .map(|((k, _), b)| put("m", k, b))
            .collect();
        store.write_records(&ops, SYNC).expect("write");

        let mut sink = CollectingSink::default();
        store
            .enumerate_leaves("m", false, &mut sink)
            .await
            .expect("leaves");
        let mut want: Vec<(String, u32)> = expected
            .iter()
            .filter_map(|(k, v)| merkle_leaf_hash(k, v).map(|(_, h)| (k.clone(), h)))
            .collect();
        want.sort_by_key(|(k, _)| position(k));
        assert_eq!(leaf_triples(&sink.leaves), want);
        assert!(!sink.leaves.iter().any(|l| l.key == "or-empty"));
        assert_eq!(sink.batch_sizes, vec![1024, want.len() - 1024]);
        for leaf in &sink.leaves {
            let value = &expected.iter().find(|(k, _)| *k == leaf.key).unwrap().1;
            assert_eq!(
                merkle_leaf_hash(&leaf.key, value).map(|(kind, _)| kind),
                Some(leaf.kind)
            );
        }
    }

    pub(in super::super) async fn s15<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        for key in ["a", "b", "c"] {
            store
                .add("m", key, &lww("primary", 1), 0, 0)
                .await
                .expect("add");
            store
                .add_backup("m", key, &lww("backup", 2), 0, 0)
                .await
                .expect("add_backup");
        }
        assert_eq!(
            store.load("m", "a").await.expect("load"),
            Some(lww("primary", 1))
        );
        for (backup, value) in [(false, lww("primary", 1)), (true, lww("backup", 2))] {
            let batches = scan_batches(&store, "m", backup, 0).await;
            let rows: Vec<(String, RecordValue)> =
                batches.into_iter().flat_map(|b| b.records).collect();
            assert_eq!(rows.len(), 3);
            assert!(rows.iter().all(|(_, v)| *v == value), "backup={backup}");
            let mut sink = CollectingSink::default();
            store
                .enumerate_leaves("m", backup, &mut sink)
                .await
                .expect("leaves");
            let hash = merkle_leaf_hash("a", &value).unwrap().1;
            assert_eq!(
                sink.leaves
                    .iter()
                    .find(|l| l.key == "a")
                    .map(|l| l.leaf_hash),
                Some(hash)
            );
            let mut seen = Vec::new();
            store
                .visit_records("m", backup, &mut |_, bytes| {
                    seen.push(dec(bytes));
                    Ok(())
                })
                .expect("visit");
            assert_eq!(seen, vec![value.clone(); 3]);
        }
        store
            .remove_backup("m", "a", 0)
            .await
            .expect("remove_backup");
        assert_eq!(
            store.load("m", "a").await.expect("load"),
            Some(lww("primary", 1))
        );
        store.remove("m", "b", 0).await.expect("remove");
        assert!(store.load("m", "b").await.expect("load").is_none());
        let backup_keys = batch_keys(&scan_batches(&store, "m", true, 0).await);
        assert_eq!(
            backup_keys.into_iter().collect::<BTreeSet<_>>(),
            ["b", "c"].iter().map(|s| (*s).to_string()).collect()
        );
    }

    pub(in super::super) async fn s16<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        for (round, value) in [
            lww("first", 1),
            lww("second", 2),
            or_value(&["t1"], &["t0", "t2"]),
            or_value(&["t3", "t4"], &[]),
        ]
        .into_iter()
        .enumerate()
        {
            store.add("a", "k", &value, 0, 0).await.expect("add");
            store
                .add_encoded("b", "k", &enc(&value), 0, 0)
                .await
                .expect("add_encoded");
            let snapshot = fresh.engine.snapshot().expect("snapshot");
            let id_a = map_id_in(&*snapshot, "a").unwrap().unwrap();
            let id_b = map_id_in(&*snapshot, "b").unwrap().unwrap();
            let stored_a = snapshot
                .get(&RecordKey::encode(RecordSpace::Primary, id_a, "k"))
                .unwrap()
                .expect("a");
            let stored_b = snapshot
                .get(&RecordKey::encode(RecordSpace::Primary, id_b, "k"))
                .unwrap()
                .expect("b");
            assert_eq!(stored_a, stored_b, "round {round}");
            assert_eq!(stored_a, enc(&value), "round {round}");
        }
    }

    pub(in super::super) async fn s17<K: EngineKind>() {
        let fresh = K::fresh();
        let failing = Arc::new(FailingWrites::new(Arc::clone(&fresh.engine)));
        let store = open(&failing);
        for key in ["k1", "k2", "k3"] {
            store.add("m", key, &lww(key, 1), 0, 0).await.expect("add");
        }
        let got = store
            .load_all("m", &["k3".to_string(), "x".to_string(), "k1".to_string()])
            .await
            .expect("load_all");
        assert_eq!(
            got,
            vec![
                ("k3".to_string(), lww("k3", 1)),
                ("k1".to_string(), lww("k1", 1))
            ]
        );
        assert!(store
            .load_all("never", &["k1".to_string()])
            .await
            .expect("load_all")
            .is_empty());

        let all = vec!["k1".to_string(), "k3".to_string(), "nope".to_string()];
        failing.arm(1);
        assert!(store.remove_all("m", &all).await.is_err());
        for key in ["k1", "k2", "k3"] {
            assert!(store.load("m", key).await.expect("load").is_some(), "{key}");
        }
        store.remove_all("m", &all).await.expect("remove_all");
        assert!(store.load("m", "k1").await.expect("load").is_none());
        assert!(store.load("m", "k2").await.expect("load").is_some());
        assert!(store.load("m", "k3").await.expect("load").is_none());

        let before = dump(&*fresh.engine);
        store
            .remove_all("never", &["a".to_string()])
            .await
            .expect("unknown map");
        store.remove_all("m", &[]).await.expect("no keys");
        assert_eq!(dump(&*fresh.engine), before);
        assert_eq!(failing.other_mutations(), 0);
    }

    pub(in super::super) fn s18<K: EngineKind>() {
        let fresh = K::fresh();
        let _store = open(&fresh.engine);
        assert_eq!(
            dump(&*fresh.engine),
            vec![
                (b"\x00ack_mode".to_vec(), vec![0x00]),
                (b"\x00format".to_vec(), vec![0x00, 0x00, 0x00, 0x01]),
                (b"\x00next_map_id".to_vec(), vec![0x00, 0x00, 0x00, 0x01]),
            ]
        );
    }

    fn assert_refused_unchanged<E: KvEngine>(engine: &Arc<E>, what: &str) -> String {
        let before = dump(&**engine);
        let outcome = EngineMapDataStore::open(Arc::clone(engine));
        let text = error_text(&outcome);
        assert!(
            text.starts_with("store format: refusing to open:"),
            "{what}: {text:?}"
        );
        assert_eq!(dump(&**engine), before, "{what}: a refusal wrote");
        text
    }

    pub(in super::super) fn s19<K: EngineKind>() {
        let cases: Vec<(&str, Vec<(Vec<u8>, Vec<u8>)>)> = vec![
            ("format 2", meta_rows(2, 0, 1)),
            ("ack mode 1", meta_rows(1, 1, 1)),
            ("unknown name", {
                let mut rows = meta_rows(1, 0, 1);
                rows.push((meta(b"x"), vec![1]));
                rows
            }),
            ("no next_map_id", meta_rows(1, 0, 1)[..2].to_vec()),
        ];
        for (what, rows) in cases {
            let fresh = K::fresh();
            raw_put(&*fresh.engine, &rows);
            assert_refused_unchanged(&fresh.engine, what);
        }
    }

    pub(in super::super) fn s20<K: EngineKind>() {
        let rows: Vec<Vec<(Vec<u8>, Vec<u8>)>> = vec![
            vec![(
                RecordKey::encode(RecordSpace::Primary, 1, "k"),
                enc(&lww("v", 1)),
            )],
            vec![(map_by_id_key(1), b"m".to_vec())],
            vec![
                (meta(b"ack_mode"), vec![0]),
                (meta(b"next_map_id"), 1u32.to_be_bytes().to_vec()),
            ],
        ];
        for rows in rows {
            let fresh = K::fresh();
            raw_put(&*fresh.engine, &rows);
            assert_refused_unchanged(&fresh.engine, "no format row");
        }
    }

    pub(in super::super) fn s21<K: EngineKind>() {
        for first in [0x03u8, 0x0F, 0x12, 0x20, 0xFF] {
            let fresh = K::fresh();
            let mut rows = meta_rows(1, 0, 1);
            rows.push((vec![first, 0x01, 0x02], vec![0x07]));
            raw_put(&*fresh.engine, &rows);
            assert_refused_unchanged(&fresh.engine, &format!("keyspace {first:#04x}"));
        }
    }

    pub(in super::super) async fn s22<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let broken = |cause| KvError::Broken {
            cause,
            detail: "d".to_string(),
        };
        for cause in [
            StoreBrokenCause::Io(std::io::ErrorKind::StorageFull),
            StoreBrokenCause::PreviousIo,
            StoreBrokenCause::Corrupted,
            StoreBrokenCause::LockPoisoned,
        ] {
            let bare = anyhow::Error::from(broken(cause));
            assert_eq!(
                store.classify_error(&bare),
                StoreErrorClass::StoreBroken(cause)
            );
            let wrapped = anyhow::Error::from(broken(cause)).context("while writing");
            assert_eq!(
                store.classify_error(&wrapped),
                StoreErrorClass::StoreBroken(cause)
            );
        }
        let refusal = store
            .add("", "k", &lww("v", 1), 0, 0)
            .await
            .expect_err("refused");
        let unknown = vec![
            anyhow::Error::from(KvError::Other(anyhow::anyhow!("other"))),
            anyhow::Error::from(KvError::Unsupported(KvCapability::Ingest)),
            anyhow::Error::from(KvError::InvalidArgument("bad")),
            anyhow::Error::from(KvError::ShutDown),
            anyhow::Error::from(std::io::Error::other("io")),
            refusal,
        ];
        for err in unknown {
            assert_eq!(
                store.classify_error(&err),
                StoreErrorClass::Unknown,
                "{err}"
            );
        }
    }

    pub(in super::super) async fn s23<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        assert!(store.is_loadable("k"));
        assert_eq!(store.pending_operation_count(), 0);
        assert_eq!(store.soft_flush().await.expect("soft_flush"), 0);
        store.hard_flush().await.expect("hard_flush");
        assert!(!store.is_null());
        assert!(!store.wants_or_witness());
        assert!(store.accepts_encoded());
        assert_eq!(store.assigned_write_sequence(), 0);
        assert_eq!(store.flushed_watermark(), 0);
        assert_eq!(
            store.scan_order().map(|o| o.name),
            Some("partition-then-key")
        );
        store.sync().expect("sync");
    }

    #[derive(Debug, Clone)]
    enum ModelOp {
        Add(usize, usize, usize),
        AddEncoded(usize, usize, usize),
        AddBackup(usize, usize, usize),
        Remove(usize, usize),
        RemoveBackup(usize, usize),
        RemoveAll(usize, Vec<usize>),
        Load(usize, usize),
        LoadAll(usize, Vec<usize>),
        Scan(usize, bool, u64),
        Leaves(usize, bool),
        ListMaps,
        Visit(usize, bool),
        Reopen,
    }

    const MODEL_MAPS: [&str; 4] = ["users", "user-profiles", "users/profiles", "x__backup"];
    const MODEL_KEYS: [&str; 7] = ["a", "b", "k1", "k2", "", "é", "zz"];

    fn model_op() -> impl Strategy<Value = ModelOp> {
        let m = 0..MODEL_MAPS.len();
        let k = 0..MODEL_KEYS.len();
        let v = 0..4usize;
        let keys = prop::collection::vec(0..MODEL_KEYS.len(), 0..4);
        prop_oneof![
            (m.clone(), k.clone(), v.clone()).prop_map(|(m, k, v)| ModelOp::Add(m, k, v)),
            (m.clone(), k.clone(), v.clone()).prop_map(|(m, k, v)| ModelOp::AddEncoded(m, k, v)),
            (m.clone(), k.clone(), v).prop_map(|(m, k, v)| ModelOp::AddBackup(m, k, v)),
            (m.clone(), k.clone()).prop_map(|(m, k)| ModelOp::Remove(m, k)),
            (m.clone(), k.clone()).prop_map(|(m, k)| ModelOp::RemoveBackup(m, k)),
            (m.clone(), keys.clone()).prop_map(|(m, ks)| ModelOp::RemoveAll(m, ks)),
            (m.clone(), k).prop_map(|(m, k)| ModelOp::Load(m, k)),
            (m.clone(), keys).prop_map(|(m, ks)| ModelOp::LoadAll(m, ks)),
            (m.clone(), any::<bool>(), prop_oneof![Just(0u64), 1u64..200])
                .prop_map(|(m, b, budget)| ModelOp::Scan(m, b, budget)),
            (m.clone(), any::<bool>()).prop_map(|(m, b)| ModelOp::Leaves(m, b)),
            Just(ModelOp::ListMaps),
            (m, any::<bool>()).prop_map(|(m, b)| ModelOp::Visit(m, b)),
            Just(ModelOp::Reopen),
        ]
    }

    fn model_value(v: usize) -> RecordValue {
        match v {
            0 => lww("v0", 1),
            1 => lww("v1-longer-value", 2),
            2 => or_value(&["t1"], &["t0"]),
            _ => or_value(&[], &[]),
        }
    }

    type Model = BTreeMap<(String, bool, String), Vec<u8>>;

    fn model_rows(model: &Model, map: &str, backup: bool) -> Vec<(String, Vec<u8>)> {
        let mut rows: Vec<(String, Vec<u8>)> = model
            .iter()
            .filter(|((m, b, _), _)| m == map && *b == backup)
            .map(|((_, _, k), v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by_key(|(k, _)| position(k));
        rows
    }

    async fn run_model<E: KvEngine>(engine: &Arc<E>, ops: &[ModelOp]) {
        let mut store = open(engine);
        let mut model: Model = BTreeMap::new();
        let mut listed: BTreeSet<String> = BTreeSet::new();
        for op in ops {
            let refused_map = |m: usize| MODEL_MAPS[m] == "x__backup";
            match op {
                ModelOp::Add(m, k, v)
                | ModelOp::AddEncoded(m, k, v)
                | ModelOp::AddBackup(m, k, v) => {
                    let (map, key, value) = (MODEL_MAPS[*m], MODEL_KEYS[*k], model_value(*v));
                    let backup = matches!(op, ModelOp::AddBackup(..));
                    let outcome = match op {
                        ModelOp::Add(..) => store.add(map, key, &value, 0, 0).await,
                        ModelOp::AddEncoded(..) => {
                            store.add_encoded(map, key, &enc(&value), 0, 0).await
                        }
                        _ => store.add_backup(map, key, &value, 0, 0).await,
                    };
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        outcome.expect("add");
                        model.insert((map.to_string(), backup, key.to_string()), enc(&value));
                        if !backup {
                            listed.insert(map.to_string());
                        }
                    }
                }
                ModelOp::Remove(m, k) | ModelOp::RemoveBackup(m, k) => {
                    let (map, key) = (MODEL_MAPS[*m], MODEL_KEYS[*k]);
                    let backup = matches!(op, ModelOp::RemoveBackup(..));
                    let outcome = if backup {
                        store.remove_backup(map, key, 0).await
                    } else {
                        store.remove(map, key, 0).await
                    };
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        outcome.expect("remove");
                        model.remove(&(map.to_string(), backup, key.to_string()));
                    }
                }
                ModelOp::RemoveAll(m, ks) => {
                    let map = MODEL_MAPS[*m];
                    let keys: Vec<String> = ks.iter().map(|k| MODEL_KEYS[*k].to_string()).collect();
                    let outcome = store.remove_all(map, &keys).await;
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        outcome.expect("remove_all");
                        for key in keys {
                            model.remove(&(map.to_string(), false, key));
                        }
                    }
                }
                ModelOp::Load(m, k) => {
                    let (map, key) = (MODEL_MAPS[*m], MODEL_KEYS[*k]);
                    let outcome = store.load(map, key).await;
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        let want = model
                            .get(&(map.to_string(), false, key.to_string()))
                            .map(|b| dec(b));
                        assert_eq!(outcome.expect("load"), want);
                    }
                }
                ModelOp::LoadAll(m, ks) => {
                    let map = MODEL_MAPS[*m];
                    let keys: Vec<String> = ks.iter().map(|k| MODEL_KEYS[*k].to_string()).collect();
                    let outcome = store.load_all(map, &keys).await;
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        let want: Vec<(String, RecordValue)> = keys
                            .iter()
                            .filter_map(|k| {
                                model
                                    .get(&(map.to_string(), false, k.clone()))
                                    .map(|b| (k.clone(), dec(b)))
                            })
                            .collect();
                        assert_eq!(outcome.expect("load_all"), want);
                    }
                }
                ModelOp::Scan(m, backup, budget) => {
                    let map = MODEL_MAPS[*m];
                    if refused_map(*m) {
                        assert!(store.scan_values(map, *backup, *budget).await.is_err());
                        continue;
                    }
                    let batches = scan_batches(&store, map, *backup, *budget).await;
                    let rows: Vec<(String, Vec<u8>)> = batches
                        .into_iter()
                        .flat_map(|b| b.records)
                        .map(|(k, v)| (k, enc(&v)))
                        .collect();
                    let keys: Vec<String> = rows.iter().map(|(k, _)| k.clone()).collect();
                    assert_strictly_ascending(&keys, "model scan");
                    let mut sorted = rows.clone();
                    sorted.sort_by_key(|(k, _)| position(k));
                    assert_eq!(sorted, model_rows(&model, map, *backup));
                }
                ModelOp::Leaves(m, backup) => {
                    let map = MODEL_MAPS[*m];
                    let mut sink = CollectingSink::default();
                    let outcome = store.enumerate_leaves(map, *backup, &mut sink).await;
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                        continue;
                    }
                    outcome.expect("leaves");
                    let want: Vec<(String, u32)> = model_rows(&model, map, *backup)
                        .into_iter()
                        .filter_map(|(k, b)| merkle_leaf_hash(&k, &dec(&b)).map(|(_, h)| (k, h)))
                        .collect();
                    assert_eq!(leaf_triples(&sink.leaves), want);
                }
                ModelOp::ListMaps => {
                    let want: Vec<String> = listed.iter().cloned().collect();
                    assert_eq!(store.list_maps().await.expect("list"), want);
                }
                ModelOp::Visit(m, backup) => {
                    let map = MODEL_MAPS[*m];
                    let mut seen = Vec::new();
                    let outcome = store.visit_records(map, *backup, &mut |k, b| {
                        seen.push((k.to_string(), b.to_vec()));
                        Ok(())
                    });
                    if refused_map(*m) {
                        assert!(outcome.is_err());
                    } else {
                        outcome.expect("visit");
                        assert_eq!(seen, model_rows(&model, map, *backup));
                    }
                }
                ModelOp::Reopen => {
                    store = open(engine);
                }
            }
        }
    }

    pub(in super::super) fn s24<K: EngineKind>() {
        let mut runner = TestRunner::new(Config::with_cases(256));
        runner
            .run(&prop::collection::vec(model_op(), 0..=64), |ops| {
                let fresh = K::fresh();
                block_on(run_model(&fresh.engine, &ops));
                Ok(())
            })
            .unwrap();
    }

    pub(in super::super) async fn s25<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        store.add("m", "a", &lww("v", 1), 0, 0).await.expect("add");
        let map_id = id_of(&*fresh.engine, "m").expect("id");
        let mut cursor = vec![KS_PRIMARY];
        cursor.extend_from_slice(&0u32.to_be_bytes());
        cursor.extend_from_slice(&map_id.to_be_bytes());
        cursor.extend_from_slice(&[0xFF, 0xFE]);
        let outcome = store
            .scan_values_batched("m", false, ScanCursor(cursor), 0)
            .await;
        let text = error_text(&outcome);
        assert!(
            text.starts_with("scan cursor is not valid UTF-8"),
            "{text:?}"
        );
    }

    pub(in super::super) fn s26<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        store
            .write_records(
                &[RecordOp::Touch {
                    map: "tp",
                    backup: false,
                }],
                SYNC,
            )
            .expect("touch");
        store
            .write_records(
                &[RecordOp::Touch {
                    map: "tb",
                    backup: true,
                }],
                SYNC,
            )
            .expect("touch");
        assert_eq!(block_on(store.list_maps()).expect("list"), vec!["tp"]);
        assert!(keys_in_space(&*fresh.engine, KS_PRIMARY).is_empty());
        assert!(keys_in_space(&*fresh.engine, KS_BACKUP).is_empty());
        assert_eq!(flags_of(&*fresh.engine, "tp"), Some(FLAG_PRIMARY));
        assert_eq!(flags_of(&*fresh.engine, "tb"), Some(FLAG_BACKUP));
        let before = dump(&*fresh.engine);
        let reopened = open(&fresh.engine);
        for store in [&store, &reopened] {
            store
                .write_records(
                    &[RecordOp::Touch {
                        map: "tp",
                        backup: false,
                    }],
                    SYNC,
                )
                .expect("touch again");
        }
        assert_eq!(dump(&*fresh.engine), before, "a second touch wrote");
    }

    pub(in super::super) fn s27<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        let keys = numbered("v", 500);
        write_keys(&store, "m", &keys);
        let mut seen: Vec<(String, Vec<u8>)> = Vec::new();
        store
            .visit_records("m", false, &mut |k, b| {
                seen.push((k.to_string(), b.to_vec()));
                Ok(())
            })
            .expect("visit");
        let want: Vec<(String, Vec<u8>)> = sorted_by_position(keys)
            .into_iter()
            .map(|k| {
                let b = enc(&lww(&k, 1));
                (k, b)
            })
            .collect();
        assert_eq!(seen, want);

        let mut visits = 0;
        let outcome = store.visit_records("m", false, &mut |_, _| {
            visits += 1;
            if visits == 5 {
                anyhow::bail!("stop here");
            }
            Ok(())
        });
        assert_eq!(error_text(&outcome), "stop here");
        assert_eq!(visits, 5);
    }

    pub(in super::super) async fn s28<K: EngineKind>() {
        let (p1, p2, p6) = (
            hash_to_partition("k1"),
            hash_to_partition("k2"),
            hash_to_partition("s6"),
        );
        assert!(p2 < p1, "k2 must sort before k1 by partition");
        assert!(p2 < p6 && p6 < p1, "s6 must lie between k2 and k1");
        let fresh = K::fresh();
        let inner = Arc::new(open(&fresh.engine));
        inner
            .add("m", "k1", &lww("d1", 1), 0, 0)
            .await
            .expect("add");
        inner
            .add("m", "k2", &lww("d2", 1), 0, 0)
            .await
            .expect("add");
        let config = WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 600_000,
            capacity: 0,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(Arc::clone(&inner) as Arc<dyn MapDataStore>, config);
        store
            .add("m", "k2", &lww("st", 2), 0, 0)
            .await
            .expect("stage");
        store
            .add("m", "s6", &lww("st", 2), 0, 0)
            .await
            .expect("stage");

        let batches = scan_batches(store.as_ref(), "m", false, 1).await;
        let keys: Vec<Vec<String>> = batches
            .iter()
            .map(|b| {
                let mut ks: Vec<String> = b.records.iter().map(|(k, _)| k.clone()).collect();
                ks.sort();
                ks
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                vec!["k2".to_string()],
                vec!["k1".to_string(), "s6".to_string()]
            ]
        );
        assert_eq!(
            batches[0].records[0].1,
            lww("st", 2),
            "k2 carries the staged value"
        );
    }

    pub(in super::super) fn s29<K: EngineKind>() {
        let fresh = K::fresh();
        let failing = Arc::new(FailingWrites::new(Arc::clone(&fresh.engine)));
        failing.arm(1);
        assert!(EngineMapDataStore::open_for_migration(Arc::clone(&failing), b"m0").is_err());
        assert!(dump(&*fresh.engine).is_empty(), "a failed format left rows");

        let (store, mark) =
            EngineMapDataStore::open_for_migration(Arc::clone(&failing), b"m0").expect("format");
        assert_eq!(mark, b"m0");
        drop(store);
        let mut want = meta_rows(1, 0, 1);
        want.push((meta(b"migration"), b"m0".to_vec()));
        want.sort();
        assert_eq!(
            dump(&*fresh.engine),
            want,
            "format rows and mark in one batch"
        );

        let text = assert_refused_unchanged(&failing, "a marked store");
        assert!(
            text.contains("a migration into this store is unfinished"),
            "{text}"
        );

        let before = dump(&*fresh.engine);
        let (_, mark) =
            EngineMapDataStore::open_for_migration(Arc::clone(&failing), b"other").expect("resume");
        assert_eq!(mark, b"m0");
        assert_eq!(
            EngineMapDataStore::read_migration_mark(&*failing).expect("read"),
            Some(b"m0".to_vec())
        );
        assert_eq!(dump(&*fresh.engine), before);

        let complete = K::fresh();
        let _ = open(&complete.engine);
        let before = dump(&*complete.engine);
        let outcome = EngineMapDataStore::open_for_migration(Arc::clone(&complete.engine), b"x");
        assert!(error_text(&outcome).starts_with(
            "store format: refusing to resume a migration: the store has no migration mark"
        ));
        assert_eq!(
            EngineMapDataStore::read_migration_mark(&*complete.engine).expect("read"),
            None
        );
        assert_eq!(dump(&*complete.engine), before);
        assert_eq!(failing.other_mutations(), 0);
    }

    pub(in super::super) fn s30<K: EngineKind>() {
        for n in [1u64, 2] {
            let fresh = K::fresh();
            let failing = Arc::new(FailingWrites::new(Arc::clone(&fresh.engine)));
            let (store, _) =
                EngineMapDataStore::open_for_migration(Arc::clone(&failing), b"m0").expect("open");
            let value = enc(&lww("v", 1));
            failing.arm(n);
            let outcome = store.write_records(
                &[
                    put("mm", "a", &value),
                    put("mm", "b", &value),
                    put("mm", "c", &value),
                    RecordOp::SetMigrationMark(Some(b"m1")),
                ],
                SYNC,
            );
            if n == 1 {
                assert!(outcome.is_err(), "the failure path must be exercised");
            }
            let mark = EngineMapDataStore::read_migration_mark(&*fresh.engine).expect("mark");
            match outcome {
                Err(_) => {
                    assert!(
                        keys_in_space(&*fresh.engine, KS_PRIMARY).is_empty(),
                        "n={n}"
                    );
                    assert_eq!(mark, Some(b"m0".to_vec()), "n={n}");
                    assert_eq!(id_of(&*fresh.engine, "mm"), None, "n={n}");
                }
                Ok(()) => {
                    assert_eq!(keys_in_space(&*fresh.engine, KS_PRIMARY).len(), 3, "n={n}");
                    assert_eq!(mark, Some(b"m1".to_vec()), "n={n}");
                }
            }
            failing.disarm();
            store
                .write_records(&[RecordOp::SetMigrationMark(None)], SYNC)
                .expect("clear the mark");
            assert_eq!(
                EngineMapDataStore::read_migration_mark(&*fresh.engine).expect("mark"),
                None
            );
            drop(store);
            EngineMapDataStore::open(Arc::clone(&fresh.engine)).expect("open once complete");
            assert_eq!(failing.other_mutations(), 0);
        }
    }

    pub(in super::super) async fn s31<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        for map in ["ma", "mb"] {
            for key in ["a", "b", "c"] {
                store.add(map, key, &lww("v", 1), 0, 0).await.expect("add");
                store
                    .add_backup(map, key, &lww("v", 1), 0, 0)
                    .await
                    .expect("add");
            }
        }
        let cursor_a = store
            .scan_values("ma", false, 1)
            .await
            .expect("scan")
            .next_cursor
            .expect("cursor");
        let backup_cursor = store
            .scan_values("ma", true, 1)
            .await
            .expect("scan")
            .next_cursor
            .expect("cursor");
        let cases = [
            ("mb", false, cursor_a.clone(), "another map's cursor"),
            (
                "ma",
                false,
                backup_cursor,
                "a backup cursor on a primary scan",
            ),
            (
                "ma",
                true,
                cursor_a.clone(),
                "a primary cursor on a backup scan",
            ),
            (
                "ma",
                false,
                ScanCursor(cursor_a.0[..8].to_vec()),
                "an 8-byte cursor",
            ),
        ];
        for (map, backup, cursor, what) in cases {
            let outcome = store.scan_values_batched(map, backup, cursor, 0).await;
            assert!(
                error_text(&outcome).starts_with("scan cursor is not a record key of this map"),
                "{what}: {:?}",
                error_text(&outcome)
            );
        }
        let unknown = store
            .scan_values_batched("never", false, cursor_a, 0)
            .await
            .expect("unknown map");
        assert!(unknown.records.is_empty() && unknown.next_cursor.is_none());
    }

    pub(in super::super) fn s32<K: EngineKind>() {
        let mut runner = TestRunner::new(Config::with_cases(256));
        runner
            .run(
                &(
                    prop::collection::btree_set("\\PC{0,6}", 1..30),
                    1u32..u32::MAX,
                ),
                |(keys, map_id)| {
                    let keys: Vec<String> = keys.into_iter().collect();
                    let mut by_position = keys.clone();
                    by_position.sort_by_key(|k| position(k));
                    let mut by_engine_key = keys.clone();
                    by_engine_key
                        .sort_by_key(|k| RecordKey::encode(RecordSpace::Primary, map_id, k));
                    prop_assert_eq!(&by_position, &by_engine_key);
                    for key in &keys {
                        for space in [RecordSpace::Primary, RecordSpace::Backup] {
                            let cursor = ScanCursor(RecordKey::encode(space, map_id, key));
                            let at = (PARTITION_THEN_KEY.cursor_position)(&cursor).expect("cursor");
                            prop_assert_eq!(at, position(key));
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
    }

    pub(in super::super) async fn s33<K: EngineKind>() {
        let fresh = K::fresh();
        let mut rows = meta_rows(1, 0, u32::MAX);
        let mut name_value = 7u32.to_be_bytes().to_vec();
        name_value.push(FLAG_PRIMARY);
        rows.push((map_by_name_key("old"), name_value));
        rows.push((map_by_id_key(7), b"old".to_vec()));
        raw_put(&*fresh.engine, &rows);
        let store = open(&fresh.engine);
        let before = dump(&*fresh.engine);
        let err = store
            .add("new", "k", &lww("v", 1), 0, 0)
            .await
            .expect_err("no id left");
        assert!(err.to_string().contains("map id space exhausted"), "{err}");
        assert_eq!(store.classify_error(&err), StoreErrorClass::Unknown);
        assert_eq!(dump(&*fresh.engine), before);
        store
            .add("old", "k", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        assert!(store.load("old", "k").await.expect("load").is_some());
    }

    pub(in super::super) async fn s34<K: EngineKind>() {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        store
            .add("users", "u1", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        store
            .add("users", "u2", &lww("v", 2), 0, 0)
            .await
            .expect("add");
        store
            .add("user-profiles", "p1", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        store
            .add_backup("bk", "b1", &lww("v", 1), 0, 0)
            .await
            .expect("add");
        store
            .write_records(
                &[RecordOp::Touch {
                    map: "tm",
                    backup: false,
                }],
                SYNC,
            )
            .expect("touch");

        let snapshot = fresh.engine.snapshot().expect("snapshot");
        let range = |first: u8| {
            snapshot
                .iter(KeyRange {
                    start: Some(vec![first]),
                    end: Some(vec![first + 1]),
                })
                .expect("iter")
                .collect::<KvResult<Vec<_>>>()
                .expect("rows")
        };
        let by_name: Vec<_> = range(KS_MAP_BY_NAME)
            .iter()
            .map(|(k, v)| decode_map_by_name_row(k, v).expect("0x01 row"))
            .collect();
        let by_id: BTreeSet<(u32, String)> = range(KS_MAP_BY_ID)
            .iter()
            .map(|(k, v)| decode_map_by_id_row(k, v).expect("0x02 row"))
            .collect();
        assert_eq!(
            by_name
                .iter()
                .map(|e| (e.map_id, e.name.clone()))
                .collect::<BTreeSet<_>>(),
            by_id
        );
        let flags: HashMap<String, u8> =
            by_name.iter().map(|e| (e.name.clone(), e.flags)).collect();
        assert_eq!(flags["users"], FLAG_PRIMARY);
        assert_eq!(flags["bk"], FLAG_BACKUP);
        assert_eq!(flags["tm"], FLAG_PRIMARY);
        for entry in &by_name {
            assert_eq!(
                map_id_in(&*snapshot, &entry.name).unwrap(),
                Some(entry.map_id)
            );
        }
        assert_eq!(map_id_in(&*snapshot, "absent").unwrap(), None);
        let id = |name: &str| map_id_in(&*snapshot, name).unwrap().unwrap();

        let mut records: BTreeSet<(u8, u32, String)> = BTreeSet::new();
        for first in [KS_PRIMARY, KS_BACKUP] {
            for (k, _) in range(first) {
                let decoded = RecordKey::decode(&k).expect("record key");
                records.insert((decoded.space.byte(), decoded.map_id, decoded.key));
            }
        }
        let want: BTreeSet<(u8, u32, String)> = [
            (RecordSpace::Primary, id("users"), "u1"),
            (RecordSpace::Primary, id("users"), "u2"),
            (RecordSpace::Primary, id("user-profiles"), "p1"),
            (RecordSpace::Backup, id("bk"), "b1"),
        ]
        .into_iter()
        .map(|(s, i, k)| (s.byte(), i, k.to_string()))
        .collect();
        assert_eq!(records, want);

        let mut visited = Vec::new();
        store
            .visit_records("users", false, &mut |k, b| {
                visited.push((k.to_string(), b.to_vec()));
                Ok(())
            })
            .expect("visit");
        assert_eq!(visited.len(), 2);
        for (key, bytes) in &visited {
            let stored = snapshot
                .get(&RecordKey::encode(RecordSpace::Primary, id("users"), key))
                .unwrap();
            assert_eq!(stored.as_ref(), Some(bytes));
        }
        assert_eq!(
            snapshot.get(&meta_key(META_FORMAT)).unwrap(),
            Some(FORMAT_VERSION.to_be_bytes().to_vec())
        );
        assert_eq!(meta_key(META_FORMAT)[0], KS_META);

        // Each decoder refuses a malformed input of its list.
        assert!(decode_map_by_name_row(&[KS_MAP_BY_NAME, 0xFF], &[0, 0, 0, 1, 1]).is_err());
        assert!(decode_map_by_name_row(&map_by_name_key("m"), &[0, 0, 0, 1]).is_err());
        assert!(decode_map_by_name_row(&map_by_name_key("m"), &[0, 0, 0, 0, 1]).is_err());
        assert!(decode_map_by_name_row(&[KS_MAP_BY_ID, b'm'], &[0, 0, 0, 1, 1]).is_err());
        assert!(decode_map_by_id_row(&[KS_MAP_BY_ID, 0, 0, 1], b"m").is_err());
        assert!(decode_map_by_id_row(&map_by_id_key(1), &[0xFF]).is_err());
        assert!(decode_map_by_id_row(&map_by_name_key("abcd"), b"m").is_err());
        assert!(RecordKey::decode(&[KS_PRIMARY, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(RecordKey::decode(&[0x12, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());
        let mut wrong_partition = RecordKey::encode(RecordSpace::Primary, 1, "hello");
        wrong_partition[4] ^= 1;
        assert!(RecordKey::decode(&wrong_partition).is_err());
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    pub(in super::super) async fn s90<K: EngineKind>(engine_name: &str) {
        let fresh = K::fresh();
        let store = open(&fresh.engine);
        for (map, n) in [("small", 10usize), ("big", 100_000)] {
            write_keys(&store, map, &numbered("key", n));
            let mut walks = Vec::new();
            let mut leaves = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                let batches = scan_batches(&store, map, false, 0).await;
                walks.push(start.elapsed());
                assert_eq!(batch_keys(&batches).len(), n);
                let start = Instant::now();
                let mut sink = CollectingSink::default();
                store
                    .enumerate_leaves(map, false, &mut sink)
                    .await
                    .expect("leaves");
                leaves.push(start.elapsed());
            }
            println!(
                "s90 engine={engine_name} map_keys={n} scan_values_full_walk_median_us={} enumerate_leaves_median_us={}",
                median(walks).as_micros(),
                median(leaves).as_micros()
            );
        }
    }
}

macro_rules! engine_store_suite {
    ($arm:ident, $kind:ty) => {
        mod $arm {
            use super::cases;

            type K = $kind;

            #[test]
            fn s01_record_key_golden_vectors() {
                cases::s01::<K>();
            }

            #[test]
            fn s02_record_key_encoding_is_injective_and_decodes_back() {
                cases::s02::<K>();
            }

            #[test]
            fn s03_a_record_key_prefix_holds_exactly_its_triple() {
                cases::s03::<K>();
            }

            #[test]
            fn s04_the_partition_is_hash_to_partition_of_the_user_key() {
                cases::s04::<K>();
            }

            #[tokio::test]
            async fn s05_map_ids_are_issued_from_one_and_survive_a_reopen() {
                cases::s05::<K>().await;
            }

            #[tokio::test]
            async fn s06_a_failed_first_write_leaves_no_registry_entry_and_no_record() {
                cases::s06::<K>().await;
            }

            #[test]
            fn s07_two_first_writes_of_two_new_maps_get_distinct_map_ids() {
                cases::s07::<K>();
            }

            #[tokio::test]
            async fn s08_a_map_with_only_backup_rows_is_not_listed_and_one_emptied_by_removes_stays_listed(
            ) {
                cases::s08::<K>().await;
            }

            #[tokio::test]
            async fn s09_reads_of_absent_maps_grow_no_registry_cache() {
                cases::s09::<K>().await;
            }

            #[tokio::test]
            async fn s10_store_refused_names_reach_no_engine_key_through_any_entry_point() {
                cases::s10::<K>().await;
            }

            #[tokio::test]
            async fn s11_a_scan_returns_keys_of_every_partition_in_partition_then_key_order() {
                cases::s11::<K>().await;
            }

            #[tokio::test]
            async fn s12_each_cursor_is_the_engine_key_of_the_last_row_and_a_resume_is_strictly_after_it(
            ) {
                cases::s12::<K>().await;
            }

            #[tokio::test]
            async fn s13_the_byte_budget_is_never_overshot_and_an_oversized_record_comes_whole() {
                cases::s13::<K>().await;
            }

            #[tokio::test]
            async fn s14_enumerate_leaves_matches_the_write_path_formula_in_scan_order() {
                cases::s14::<K>().await;
            }

            #[tokio::test]
            async fn s15_backup_rows_are_isolated_from_primary_rows() {
                cases::s15::<K>().await;
            }

            #[tokio::test]
            async fn s16_add_encoded_stores_the_bytes_add_stores() {
                cases::s16::<K>().await;
            }

            #[tokio::test]
            async fn s17_load_all_and_remove_all() {
                cases::s17::<K>().await;
            }

            #[test]
            fn s18_a_fresh_engine_gets_format_one_and_local_ack_mode() {
                cases::s18::<K>();
            }

            #[test]
            fn s19_a_store_of_an_unknown_format_or_with_unknown_metadata_is_refused() {
                cases::s19::<K>();
            }

            #[test]
            fn s20_a_non_empty_engine_without_format_metadata_is_refused() {
                cases::s20::<K>();
            }

            #[test]
            fn s21_a_key_in_a_reserved_keyspace_is_refused_at_open() {
                cases::s21::<K>();
            }

            #[tokio::test]
            async fn s22_engine_errors_classify_by_kind() {
                cases::s22::<K>().await;
            }

            #[tokio::test]
            async fn s23_the_trait_answers_hold() {
                cases::s23::<K>().await;
            }

            #[test]
            fn s24_the_store_matches_the_model() {
                cases::s24::<K>();
            }

            #[tokio::test]
            async fn s25_a_scan_cursor_whose_key_is_not_utf8_fails() {
                cases::s25::<K>().await;
            }

            #[test]
            fn s26_touch_registers_a_map_without_a_record() {
                cases::s26::<K>();
            }

            #[test]
            fn s27_visit_records_walks_raw_bytes_in_scan_order() {
                cases::s27::<K>();
            }

            #[tokio::test]
            async fn s28_a_write_behind_scan_over_the_engine_store_emits_a_key_at_a_partition_boundary_once(
            ) {
                cases::s28::<K>().await;
            }

            #[test]
            fn s29_a_store_with_a_migration_mark_is_refused_by_open_and_admitted_by_open_for_migration(
            ) {
                cases::s29::<K>();
            }

            #[test]
            fn s30_a_migration_mark_is_written_in_the_batch_of_its_records() {
                cases::s30::<K>();
            }

            #[tokio::test]
            async fn s31_a_cursor_of_another_map_or_keyspace_fails_the_scan() {
                cases::s31::<K>().await;
            }

            #[test]
            fn s32_partition_then_key_positions_agree_with_engine_key_order() {
                cases::s32::<K>();
            }

            #[tokio::test]
            async fn s33_an_exhausted_map_id_space_refuses_a_new_map_and_changes_nothing() {
                cases::s33::<K>().await;
            }

            #[tokio::test]
            async fn s34_the_public_codecs_read_what_the_store_wrote() {
                cases::s34::<K>().await;
            }

            #[tokio::test]
            #[ignore = "measurement; prints medians, asserts nothing"]
            async fn s90_scan_cost_measurement() {
                cases::s90::<K>(stringify!($arm)).await;
            }
        }
    };
}

engine_store_suite!(mem, super::Mem);
#[cfg(feature = "redb")]
engine_store_suite!(redb, super::Redb);

mod mem_only {
    use super::*;

    #[test]
    fn s35_map_id_in_returns_an_engine_error_not_absent() {
        let engine = Arc::new(MemKvEngine::new());
        let store = open(&engine);
        block_on(store.add("users", "k", &lww("v", 1), 0, 0)).expect("add");
        let snapshot = engine.snapshot().expect("snapshot");
        assert_eq!(map_id_in(&*snapshot, "users").expect("live read"), Some(1));
        engine.arm_death(engine.calls_so_far());
        assert!(
            engine.sync().is_err(),
            "the death fires on the counted sync"
        );
        let outcome = map_id_in(&*snapshot, "users");
        assert!(outcome.is_err(), "an engine error read as {outcome:?}");
    }
}
