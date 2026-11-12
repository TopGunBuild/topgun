//! Durable before collect (`TG-WAL-003`, `TG-WB-001`): a collection tick and
//! `finish(true)` take the store's rows, sync the engine only when a commit
//! returned unsynced, and only then use what they took; the write-behind
//! fence publishes only a value a durable point covers.
//!
//! Most tests run over a [`SyncEngine`]: the store-mode tests' counting engine
//! with every `sync` logged and, when armed, refused, slowed or held. The
//! crash tests run over the `kv_crash` targets, whose crash keeps nothing
//! written since the last sync. Pass commits are written unsynced here
//! (`set_pass_sync(false)`) unless a test says otherwise, which is the regime
//! the collection sync exists for. The tests of the engine-aware choice let
//! the store choose (no override): over RocksDB, which has
//! `unsynced_write_survives_process_crash`, or over a [`Logged`] wrapper that
//! claims it for an engine whose crash model loses the unsynced tail.

// Each test is one scenario read top to bottom; splitting one across helpers
// would hide the order its claim depends on.
#![allow(clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use topgun_core::PARTITION_COUNT;

use super::store_mode_tests::{
    flush_idle, frames_on_disk, gc_skipped, keys_in, lww, open_engine, sidecar_files, sidecar_path,
    store_frame, wb_config, CountingEngine, Root, MAP,
};
use super::{Wal, WalFsyncPolicy, WalRecovery, WalWriter, WAL_COLLECTED_FILE};
use crate::storage::datastores::engine_store::EngineMapDataStore;
#[cfg(feature = "rocksdb")]
use crate::storage::datastores::kv_crash::RocksTarget;
use crate::storage::datastores::kv_crash::{CrashMode, CrashTarget, MemTarget, RedbTarget};
use crate::storage::datastores::kv_engine::{
    KeyRange, KvCapabilities, KvEngine, KvError, KvResult, KvSnapshot, WriteBatch, WriteOptions,
};
#[cfg(feature = "rocksdb")]
use crate::storage::datastores::rocks_engine::{OpenMode, RocksConfig, RocksKvEngine};
use crate::storage::datastores::{WalBootstrap, WriteBehindDataStore};
use crate::storage::map_data_store::{
    FlushCommit, MapDataStore, PutValue, StoreBrokenCause, StoreWrite, WatermarkKeeping,
};
use crate::storage::record::RecordValue;

/// The WAL partition every test writes to.
const P: u32 = 7;

/// Older than any segment's seal age, so the next tick seals the active one.
const SEAL_AGE: Duration = Duration::from_secs(61);

// ---------------------------------------------------------------------------
// The sync-logging engine
// ---------------------------------------------------------------------------

/// What a `sync` call saw when it began.
#[derive(Debug, Clone)]
struct SyncCall {
    /// Snapshots taken before it.
    snapshots_before: u64,
    /// The frames of [`P`] on disk.
    frames: BTreeSet<u64>,
    /// Whether any rollback hint already existed.
    hint_present: bool,
}

/// How an armed `sync` fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncFault {
    Other,
    Broken,
}

/// A [`KvEngine`] over a [`CountingEngine`] that logs every `sync` and can be
/// armed to refuse, slow or hold one.
struct SyncEngine {
    inner: Arc<CountingEngine>,
    wal_dir: PathBuf,
    calls: Mutex<Vec<SyncCall>>,
    snapshots: AtomicU64,
    writes_entered: AtomicU64,
    syncs_done: AtomicU64,
    fault: Mutex<Option<SyncFault>>,
    delay: Mutex<Duration>,
    /// One-shot: `(reached, release)`.
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl SyncEngine {
    fn new(inner: Arc<CountingEngine>, wal_dir: PathBuf) -> Self {
        Self {
            inner,
            wal_dir,
            calls: Mutex::new(Vec::new()),
            snapshots: AtomicU64::new(0),
            writes_entered: AtomicU64::new(0),
            syncs_done: AtomicU64::new(0),
            fault: Mutex::new(None),
            delay: Mutex::new(Duration::ZERO),
            gate: Mutex::new(None),
        }
    }

    fn syncs(&self) -> Vec<SyncCall> {
        self.calls.lock().clone()
    }

    fn clear_syncs(&self) {
        self.calls.lock().clear();
    }

    fn snapshots(&self) -> u64 {
        self.snapshots.load(Ordering::SeqCst)
    }

    /// Holds the next `sync` before it reaches the engine until the returned
    /// sender sends or is dropped.
    fn arm_gate(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached, reached_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *self.gate.lock() = Some((reached, release_rx));
        (reached_rx, release)
    }
}

impl KvEngine for SyncEngine {
    fn capabilities(&self) -> KvCapabilities {
        self.inner.capabilities()
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        self.writes_entered.fetch_add(1, Ordering::SeqCst);
        self.inner.write(batch, opts)
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        self.inner.snapshot()
    }

    fn ingest(
        &self,
        range: KeyRange,
        sorted: Vec<(Vec<u8>, Vec<u8>)>,
        opts: WriteOptions,
    ) -> KvResult<()> {
        self.inner.ingest(range, sorted, opts)
    }

    fn sync(&self) -> KvResult<()> {
        self.calls.lock().push(SyncCall {
            snapshots_before: self.snapshots(),
            frames: frames_on_disk(&self.wal_dir, P),
            hint_present: !sidecar_files(&self.wal_dir).is_empty(),
        });
        let gate = self.gate.lock().take();
        if let Some((reached, release)) = gate {
            let _ = reached.send(());
            let _ = release.recv();
        }
        let delay = *self.delay.lock();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let fault = *self.fault.lock();
        let result = match fault {
            Some(SyncFault::Other) => Err(KvError::Other(anyhow::anyhow!(
                "injected engine sync refusal"
            ))),
            Some(SyncFault::Broken) => Err(KvError::Broken {
                cause: StoreBrokenCause::PreviousIo,
                detail: "injected broken engine sync".to_string(),
            }),
            None => self.inner.sync(),
        };
        self.syncs_done.fetch_add(1, Ordering::SeqCst);
        result
    }

    fn shutdown(&self) -> KvResult<()> {
        self.inner.shutdown()
    }
}

// ---------------------------------------------------------------------------
// The write-logging, capability-claiming engine
// ---------------------------------------------------------------------------

/// A [`KvEngine`] over `E` that logs the `sync` flag of every write and
/// counts `sync` calls; when `claim` is set it reports
/// `unsynced_write_survives_process_crash`, so the store chooses unsynced
/// commits over an engine whose crash model loses them.
struct Logged<E> {
    inner: Arc<E>,
    claim: bool,
    writes: Mutex<Vec<bool>>,
    syncs: AtomicU64,
}

impl<E: KvEngine> Logged<E> {
    fn over(inner: Arc<E>, claim: bool) -> Self {
        Self {
            inner,
            claim,
            writes: Mutex::new(Vec::new()),
            syncs: AtomicU64::new(0),
        }
    }

    fn mark(&self) -> usize {
        self.writes.lock().len()
    }

    /// The `sync` flag of every write from `mark` on.
    fn writes_since(&self, mark: usize) -> Vec<bool> {
        self.writes.lock()[mark..].to_vec()
    }

    fn syncs(&self) -> u64 {
        self.syncs.load(Ordering::SeqCst)
    }
}

impl<E: KvEngine> KvEngine for Logged<E> {
    fn capabilities(&self) -> KvCapabilities {
        if self.claim {
            KvCapabilities {
                unsynced_write_survives_process_crash: true,
                ..self.inner.capabilities()
            }
        } else {
            self.inner.capabilities()
        }
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        self.writes.lock().push(opts.sync);
        self.inner.write(batch, opts)
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
        self.inner.ingest(range, sorted, opts)
    }

    fn sync(&self) -> KvResult<()> {
        self.syncs.fetch_add(1, Ordering::SeqCst);
        self.inner.sync()
    }

    fn shutdown(&self) -> KvResult<()> {
        self.inner.shutdown()
    }
}

/// A RocksDB engine at `path` with the production options; memtables large
/// enough that a test's writes stay in them.
#[cfg(feature = "rocksdb")]
fn rocks_at(path: &Path, mode: OpenMode) -> Arc<RocksKvEngine> {
    let engine = RocksKvEngine::open(path, mode, &RocksConfig::production(64 * 1024 * 1024, -1))
        .unwrap_or_else(|e| panic!("rocksdb at {}: {e}", path.display()));
    Arc::new(engine)
}

/// The `0x20` row of [`P`] a store over `engine` reads.
fn row_of<E: KvEngine>(engine: Arc<E>) -> u64 {
    EngineMapDataStore::open(engine)
        .expect("store opens")
        .applied_watermarks()
        .expect("rows")
        .expect("kept")
        .by_partition[P as usize]
}

// ---------------------------------------------------------------------------
// A node over the sync-logging engine
// ---------------------------------------------------------------------------

struct Node {
    engine: Arc<SyncEngine>,
    store: Arc<EngineMapDataStore<SyncEngine>>,
    wal: Arc<WalWriter>,
}

impl Node {
    /// Opens `root`'s store, its pass commits synced as `pass_sync` says, and
    /// its WAL with the timer's collection held off: only a test runs a tick.
    fn open(root: &Root, pass_sync: bool) -> Self {
        let engine = Arc::new(SyncEngine::new(open_engine(root), root.wal_dir()));
        let mut store = EngineMapDataStore::open(Arc::clone(&engine)).expect("store opens");
        store.set_pass_sync(pass_sync);
        let store = Arc::new(store);
        let wal = WalWriter::open_over_store(
            root.wal_dir(),
            WalFsyncPolicy::PerOp,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )
        .expect("store-mode open");
        wal.test_hold_collection_task(true);
        Self { engine, store, wal }
    }

    async fn append(&self, key: &str, value: &RecordValue, seq: u64) {
        self.wal
            .append(P, &store_frame(key, value, seq))
            .await
            .expect("append");
    }

    /// One pass commit of `records` with the row `(P, row)`, then the WAL is
    /// told the row returned.
    async fn commit(&self, records: &[(String, RecordValue)], row: u64) {
        commit_and_mark(self.store.as_ref(), &self.wal, records, row).await;
    }

    async fn write_behind(&self) -> Arc<WriteBehindDataStore> {
        write_behind_over(Arc::clone(&self.store) as Arc<dyn MapDataStore>, &self.wal).await
    }
}

async fn commit_and_mark(
    store: &dyn MapDataStore,
    wal: &WalWriter,
    records: &[(String, RecordValue)],
    row: u64,
) {
    let writes = records
        .iter()
        .map(|(key, value)| StoreWrite::Put {
            map: MAP,
            key,
            value: PutValue::Value(value),
            store_time: 0,
        })
        .collect();
    store
        .commit(&FlushCommit {
            writes,
            applied: vec![(P, row)],
        })
        .await
        .expect("commit");
    wal.mark_applied(P, row).await.expect("mark_applied");
}

async fn write_behind_over(
    store: Arc<dyn MapDataStore>,
    wal: &Arc<WalWriter>,
) -> Arc<WriteBehindDataStore> {
    let sequence_start = wal
        .max_observed_sequence()
        .await
        .expect("max_observed_sequence")
        + 1;
    WriteBehindDataStore::new_with_wal(
        store,
        wb_config(),
        Some(WalBootstrap {
            wal: Arc::clone(wal) as Arc<dyn Wal>,
            sequence_start,
        }),
    )
}

/// `n` keys of [`P`] with a value each.
fn records(n: usize) -> Vec<(String, RecordValue)> {
    keys_in(P, n)
        .into_iter()
        .zip(1u64..)
        .map(|(key, i)| (key, lww("v", i)))
        .collect()
}

/// The hint `finish` wrote for [`P`].
fn hint_of(wal_dir: &Path) -> u64 {
    let bytes = std::fs::read(sidecar_path(wal_dir, P)).expect("the hint of P");
    u64::from_be_bytes(bytes.try_into().expect("an 8-byte hint"))
}

// ---------------------------------------------------------------------------
// AC1: one sync per tick when needed, none otherwise, in order
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac1_a_tick_syncs_once_after_its_take_and_before_any_unlink_and_only_when_needed() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(4);
    for (seq, (key, value)) in (1u64..).zip(&recs) {
        node.append(key, value, seq).await;
    }

    // N = 3 unsynced passes: they write and never sync.
    let writes = node.engine.inner.mark();
    node.commit(&recs[..2], 2).await;
    node.commit(&recs[2..3], 3).await;
    node.commit(&recs[3..], 4).await;
    let passes = node.engine.inner.writes_since(writes);
    assert_eq!(passes.len(), 3, "one engine write per pass");
    assert!(passes.iter().all(|w| !w.sync), "the passes are unsynced");
    assert!(node.engine.syncs().is_empty(), "the passes make no sync");

    // The tick: one sync, after its take, before any unlink.
    let snapshots = node.engine.snapshots();
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    let syncs = node.engine.syncs();
    assert_eq!(syncs.len(), 1, "one sync in the tick: {syncs:?}");
    assert!(
        syncs[0].snapshots_before > snapshots,
        "the sync follows the tick's watermark take"
    );
    assert_eq!(
        syncs[0].frames,
        BTreeSet::from([1, 2, 3, 4]),
        "nothing was unlinked before the sync"
    );
    assert!(
        frames_on_disk(&root.wal_dir(), P).is_empty(),
        "the tick collected what the synced rows cover"
    );

    // Nothing committed since: the next tick makes no sync.
    node.engine.clear_syncs();
    node.wal.test_collect_now().await;
    assert!(
        node.engine.syncs().is_empty(),
        "no sync when every returned commit is durable"
    );

    // Passes written `sync: true` (production): N passes and M ticks make no
    // sync and no write besides the passes.
    let root = Root::new();
    let node = Node::open(&root, true);
    for (seq, (key, value)) in (1u64..).zip(&recs) {
        node.append(key, value, seq).await;
    }
    let before_passes = node.engine.inner.mark();
    node.commit(&recs[..2], 2).await;
    node.commit(&recs[2..3], 3).await;
    node.commit(&recs[3..], 4).await;
    assert_eq!(node.engine.inner.writes_since(before_passes).len(), 3);
    let before_ticks = node.engine.inner.mark();
    for _ in 0..3 {
        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
    }
    assert!(
        node.engine.syncs().is_empty(),
        "a tick over synced passes makes no sync"
    );
    assert!(
        node.engine.inner.writes_since(before_ticks).is_empty(),
        "a tick makes no engine write"
    );
    assert!(
        frames_on_disk(&root.wal_dir(), P).is_empty(),
        "the ticks still collected"
    );
}

/// N passes over `engine` with a WAL attached and the store choosing: they
/// are written `sync: !unsynced` and make no sync; the tick after them makes
/// one sync exactly when they were unsynced.
async fn ac1_passes_choose<E: KvEngine>(engine: Arc<E>, wal_dir: PathBuf, unsynced: bool) {
    let engine = Arc::new(Logged::over(engine, false));
    let store = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).expect("store opens"));
    let wal = WalWriter::open_over_store(
        wal_dir,
        WalFsyncPolicy::PerOp,
        Arc::clone(&store) as Arc<dyn MapDataStore>,
    )
    .expect("store-mode open");
    wal.test_hold_collection_task(true);
    let recs = records(4);
    for (seq, (key, value)) in (1u64..).zip(&recs) {
        wal.append(P, &store_frame(key, value, seq))
            .await
            .expect("append");
    }
    let mark = engine.mark();
    commit_and_mark(store.as_ref(), &wal, &recs[..2], 2).await;
    commit_and_mark(store.as_ref(), &wal, &recs[2..3], 3).await;
    commit_and_mark(store.as_ref(), &wal, &recs[3..], 4).await;
    assert_eq!(
        engine.writes_since(mark),
        vec![!unsynced; 3],
        "one engine write per pass, sync: {}",
        !unsynced
    );
    assert_eq!(engine.syncs(), 0, "the passes make no sync");
    wal.test_advance_clock(SEAL_AGE);
    wal.test_collect_now().await;
    assert_eq!(
        engine.syncs(),
        u64::from(unsynced),
        "the tick syncs once exactly when a pass returned unsynced"
    );
}

#[cfg(feature = "rocksdb")]
#[tokio::test(flavor = "multi_thread")]
async fn ac1_on_rocksdb_under_a_wal_passes_are_unsynced_and_a_tick_syncs_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = rocks_at(&dir.path().join("db"), OpenMode::CreateNew);
    assert!(engine.capabilities().unsynced_write_survives_process_crash);
    ac1_passes_choose(engine, dir.path().join("wal"), true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ac1_on_redb_under_a_wal_passes_stay_synced() {
    let root = Root::new();
    let engine = open_engine(&root);
    assert!(!engine.capabilities().unsynced_write_survives_process_crash);
    ac1_passes_choose(engine, root.wal_dir(), false).await;
}

#[cfg(feature = "rocksdb")]
#[tokio::test(flavor = "multi_thread")]
async fn ac1_on_rocksdb_without_a_wal_commits_stay_synced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let engine = Arc::new(Logged::over(
        rocks_at(&dir.path().join("db"), OpenMode::CreateNew),
        false,
    ));
    let store = EngineMapDataStore::open(Arc::clone(&engine)).expect("store opens");
    let recs = records(1);
    let mark = engine.mark();
    store
        .commit(&FlushCommit {
            writes: vec![StoreWrite::Put {
                map: MAP,
                key: &recs[0].0,
                value: PutValue::Value(&recs[0].1),
                store_time: 0,
            }],
            applied: vec![(P, 1)],
        })
        .await
        .expect("commit");
    assert_eq!(
        engine.writes_since(mark),
        vec![true],
        "no WAL frame covers the commit, so it is synced"
    );
}

// ---------------------------------------------------------------------------
// AC2: durability under a lost engine tail
// ---------------------------------------------------------------------------

/// How a life's store chooses whether its commits are synced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Passes {
    /// Unsynced by the test override, whatever the engine says.
    Forced,
    /// Chosen by the store, over a [`Logged`] wrapper that claims the
    /// capability for an engine whose crash loses the unsynced tail.
    Claimed,
    /// Chosen by the store from the engine's own capability.
    Native,
}

/// A runtime of its own per process life: dropping it is the crash of the
/// tasks it runs, write-behind's included, with no drain and no `finish`.
fn life_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

/// One process life over `engine`: the store with its commits synced as
/// `passes` says, its WAL recovered as at start; the WAL's collection timer
/// is held off.
async fn open_life<E: KvEngine + 'static>(
    engine: Arc<E>,
    wal_dir: &Path,
    passes: Passes,
) -> (Arc<dyn MapDataStore>, Arc<WalWriter>) {
    let store: Arc<dyn MapDataStore> = match passes {
        Passes::Forced => {
            let mut store = EngineMapDataStore::open(engine).expect("store opens");
            store.set_pass_sync(false);
            Arc::new(store)
        }
        Passes::Claimed => Arc::new(
            EngineMapDataStore::open(Arc::new(Logged::over(engine, true))).expect("store opens"),
        ),
        Passes::Native => Arc::new(EngineMapDataStore::open(engine).expect("store opens")),
    };
    let wal = WalWriter::open_over_store(
        wal_dir.to_path_buf(),
        WalFsyncPolicy::PerOp,
        Arc::clone(&store) as Arc<dyn MapDataStore>,
    )
    .expect("store-mode open after the crash");
    wal.test_hold_collection_task(true);
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&store) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    (store, wal)
}

/// Ends a life by dropping its runtime, then waits until its WAL writer is
/// gone, so the next life opens the directory alone.
fn crash_life(rt: tokio::runtime::Runtime, wal: &std::sync::Weak<WalWriter>) {
    drop(rt);
    let deadline = Instant::now() + Duration::from_secs(30);
    while wal.upgrade().is_some() {
        assert!(
            Instant::now() < deadline,
            "the dead WAL writer is still held"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

async fn assert_served(store: &dyn MapDataStore, expect: &[(String, RecordValue)], when: &str) {
    for (key, value) in expect {
        let got = store.load(MAP, key).await.expect("load");
        assert_eq!(got.as_ref(), Some(value), "{when}: {key} is served");
    }
}

async fn write_through(wb: &WriteBehindDataStore, recs: &[(String, RecordValue)]) {
    for (key, value) in recs {
        wb.add(MAP, key, value, 0)
            .await
            .expect("acknowledged write");
    }
    flush_idle(wb).await;
}

/// Acknowledged writes in two passes, crashes before any tick and right
/// after recovery replay, then a tick that unlinks and a third crash: every
/// acknowledged write is served after each, and the WAL keeps every frame
/// above the row the last crash left.
///
/// Every crash keeps `seed` unsynced batches on a whole-batch target. Where
/// a crash drops the whole unsynced tail, the row it leaves is checked to be
/// the pre-crash synced one, so the commits of that life (passes, then
/// replay) really were unsynced; where it keeps every returned batch, the
/// row is the latest one.
fn ac2_lost_engine_tail<T: CrashTarget>(passes: Passes, seed: u64)
where
    T::Engine: 'static,
{
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().join("wal");
    let recs = records(8);
    let (first, second) = recs.split_at(4);
    let keeps_tail = T::KEEPS_EVERY_RETURNED_BATCH || (T::KEEPS_WHOLE_BATCHES && seed == u64::MAX);
    let drops_tail = !keeps_tail && (!T::KEEPS_WHOLE_BATCHES || seed == 0);

    // Life A: acknowledged writes in two unsynced passes, crash before any
    // tick.
    let target = T::fresh();
    let rt = life_runtime();
    let (wal, rows_a) = rt.block_on(async {
        let (store, wal) = open_life(target.engine(), &wal_dir, passes).await;
        let wb = write_behind_over(Arc::clone(&store), &wal).await;
        let mut rows = Vec::new();
        for half in first.chunks(2) {
            write_through(&wb, half).await;
            rows.push(
                store
                    .applied_watermarks()
                    .expect("rows")
                    .expect("kept")
                    .by_partition[P as usize],
            );
        }
        (Arc::downgrade(&wal), rows)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, seed);
    let row = row_of(target.engine());
    assert!(
        0 < rows_a[0] && rows_a[0] < rows_a[1],
        "two passes, two rows: {rows_a:?}"
    );
    if drops_tail {
        assert_eq!(row, 0, "the passes were unsynced, so the crash lost them");
    } else if keeps_tail {
        assert_eq!(row, rows_a[1], "the crash kept every returned pass");
    } else {
        assert_eq!(
            row, rows_a[0],
            "the crash kept the first pass: a row above the synced one survived"
        );
    }

    // Life B: every acknowledged write is served (replayed from the row the
    // crash left, unsynced again), then a second crash before the first tick.
    let rt = life_runtime();
    let wal = rt.block_on(async {
        let (store, wal) = open_life(target.engine(), &wal_dir, passes).await;
        assert_served(store.as_ref(), first, "after the first crash").await;
        Arc::downgrade(&wal)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, seed);
    if drops_tail {
        assert_eq!(
            row_of(target.engine()),
            row,
            "the replay commits were unsynced, so the crash lost them"
        );
    }

    // Life C: the same frames replay again; more writes; a tick that unlinks;
    // then the crash.
    let rt = life_runtime();
    let (wal, kept) = rt.block_on(async {
        let (store, wal) = open_life(target.engine(), &wal_dir, passes).await;
        assert_served(store.as_ref(), first, "after the replay was lost").await;
        let wb = write_behind_over(Arc::clone(&store), &wal).await;
        write_through(&wb, second).await;
        let before = frames_on_disk(&wal_dir, P);
        wal.test_advance_clock(SEAL_AGE);
        wal.test_collect_now().await;
        let kept = frames_on_disk(&wal_dir, P);
        assert!(
            kept.len() < before.len(),
            "the tick unlinked a segment: before {before:?}, after {kept:?}"
        );
        (Arc::downgrade(&wal), kept)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, seed);

    // Life D: the WAL holds every frame above the row the crash left, and
    // every acknowledged write is served.
    let row = row_of(target.engine());
    let on_disk = frames_on_disk(&wal_dir, P);
    assert_eq!(on_disk, kept, "the crash changed no WAL file");
    let top = kept.iter().copied().max().unwrap_or(row);
    for seq in row + 1..=top {
        assert!(
            on_disk.contains(&seq),
            "frame {seq} above the row {row} is kept"
        );
    }
    let rt = life_runtime();
    rt.block_on(async {
        let (store, _wal) = open_life(target.engine(), &wal_dir, passes).await;
        assert_served(store.as_ref(), &recs, "after collection and the crash").await;
    });
}

#[test]
fn ac2_every_acknowledged_write_survives_a_lost_engine_tail_on_redb() {
    ac2_lost_engine_tail::<RedbTarget>(Passes::Forced, 0);
}

#[test]
fn ac2_every_acknowledged_write_survives_a_lost_engine_tail_on_whole_batches() {
    ac2_lost_engine_tail::<MemTarget>(Passes::Forced, 0);
}

/// AC2(a): RocksDB's own choice, crashed as a process dies. The copy keeps
/// every returned batch, so this proves the capability's premise and replay
/// idempotence, not durability against a lost tail.
#[cfg(feature = "rocksdb")]
#[test]
fn ac2a_rocksdb_unsynced_passes_survive_a_process_crash() {
    ac2_lost_engine_tail::<RocksTarget>(Passes::Native, 0);
}

/// AC2(b): an engine that claims the capability loses everything since its
/// last sync; the store chose unsynced passes and replays.
#[test]
fn ac2b_with_the_capability_claimed_a_lost_tail_since_the_last_sync_loses_no_acknowledged_write() {
    ac2_lost_engine_tail::<RedbTarget>(Passes::Claimed, 0);
}

/// AC2(b′): the same over whole batches, keeping none, the first, or every
/// unsynced batch, so a surviving row above the synced one is replayed from.
#[test]
fn ac2b_prime_with_the_capability_claimed_a_lost_batch_suffix_loses_no_acknowledged_write() {
    for keep in [0, 1, u64::MAX] {
        ac2_lost_engine_tail::<MemTarget>(Passes::Claimed, keep);
    }
}

/// AC2(d): plain redb, the first open imports the rollback hints, writes
/// `wal-collected` and deletes the hints; a power loss right after, with no
/// later engine write to make the import durable by accident, must not leave
/// a store the next start refuses (`TG-WAL-013`).
#[test]
fn ac2d_the_import_survives_a_power_loss_right_after_the_first_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().join("wal");
    std::fs::create_dir_all(&wal_dir).expect("wal dir");
    std::fs::write(sidecar_path(&wal_dir, P), 5u64.to_be_bytes()).expect("hint");

    let target = RedbTarget::fresh();
    let rt = life_runtime();
    let wal = rt.block_on(async {
        let engine = Arc::new(Logged::over(target.engine(), false));
        let store = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).expect("store opens"));
        assert!(
            !store
                .applied_watermarks()
                .expect("rows")
                .expect("kept")
                .imported,
            "a fresh store has not imported"
        );
        let mark = engine.mark();
        let wal = WalWriter::open_over_store(
            wal_dir.clone(),
            WalFsyncPolicy::PerOp,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )
        .expect("store-mode open");
        wal.test_hold_collection_task(true);
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&store) as Arc<dyn MapDataStore>)
            .await
            .expect("recovery");
        assert!(sidecar_files(&wal_dir).is_empty(), "the hints are deleted");
        assert!(wal_dir.join(WAL_COLLECTED_FILE).is_file());
        assert_eq!(
            engine.writes_since(mark).len(),
            1,
            "the import is the last engine write before the crash"
        );
        Arc::downgrade(&wal)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, 0);

    let store = Arc::new(EngineMapDataStore::open(target.engine()).expect("store opens"));
    let held = store.applied_watermarks().expect("rows").expect("kept");
    assert!(held.imported, "the import survived the power loss");
    assert_eq!(held.by_partition[P as usize], 5);
    let rt = life_runtime();
    rt.block_on(async {
        WalWriter::open_over_store(
            wal_dir.clone(),
            WalFsyncPolicy::PerOp,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )
        .expect("the next start is not refused");
    });
}

/// Copies every file of the RocksDB directory `from` into a new directory,
/// RocksDB's own WAL files (`*.log`) only when `logs`.
#[cfg(feature = "rocksdb")]
fn copy_db(from: &Path, logs: bool) -> tempfile::TempDir {
    let to = tempfile::tempdir().expect("tempdir");
    for entry in std::fs::read_dir(from).expect("the database directory") {
        let path = entry.expect("a directory entry").path();
        if !logs && path.extension().is_some_and(|e| e == "log") {
            continue;
        }
        std::fs::copy(
            &path,
            to.path().join(path.file_name().expect("a file name")),
        )
        .unwrap_or_else(|e| panic!("copy {}: {e}", path.display()));
    }
    to
}

/// AC2(e): `kill -9`, a new process opens the store read-write, and the
/// machine then loses everything the new process has not synced. The rows
/// the new process read at open must have left RocksDB's WAL files during
/// the open (the recovery flush, `avoid_flush_during_recovery = false`).
#[cfg(feature = "rocksdb")]
#[test]
fn ac2e_rows_a_dead_process_left_unsynced_leave_the_engine_wal_at_the_next_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db");
    let wal_dir = dir.path().join("wal");
    let recs = records(4);

    // 1. The dead process: unsynced passes, ticks held, no graceful stop.
    let engine = rocks_at(&db, OpenMode::CreateNew);
    let rt = life_runtime();
    let (wal, latest) = rt.block_on(async {
        let (store, wal) = open_life(Arc::clone(&engine), &wal_dir, Passes::Native).await;
        let wb = write_behind_over(Arc::clone(&store), &wal).await;
        write_through(&wb, &recs).await;
        let latest = store
            .applied_watermarks()
            .expect("rows")
            .expect("kept")
            .by_partition[P as usize];
        (Arc::downgrade(&wal), latest)
    });
    crash_life(rt, &wal);
    assert!(latest > 0, "the passes stored a row");
    assert_eq!(Arc::strong_count(&engine), 1, "the dead store is gone");
    engine.stop_background_work();
    let image = copy_db(&db, true);
    drop(engine);

    // 2. Not vacuous: the row lived only in RocksDB's WAL files.
    let without_logs = copy_db(image.path(), false);
    assert!(
        row_of(rocks_at(without_logs.path(), OpenMode::Existing)) < latest,
        "the dead process flushed the row itself, so this arm proves nothing"
    );

    // 3. The new process opens read-write and writes nothing; the copy
    //    without RocksDB's WAL files is what a loss of the unsynced tail
    //    leaves.
    let reopened = rocks_at(image.path(), OpenMode::Existing);
    reopened.stop_background_work();
    let lost_tail = copy_db(image.path(), false);
    drop(reopened);

    // 4. The row and every record it covers are there.
    let engine = rocks_at(lost_tail.path(), OpenMode::Existing);
    assert_eq!(
        row_of(Arc::clone(&engine)),
        latest,
        "the row left the engine WAL during the open"
    );
    let store = EngineMapDataStore::open(engine).expect("store opens");
    life_runtime().block_on(assert_served(&store, &recs, "after the lost tail"));
}

// ---------------------------------------------------------------------------
// AC3: a commit between the take and the sync is not used
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac3_a_commit_that_lands_during_the_sync_is_not_used_by_that_tick() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(4);
    // Two sealed segments, [1, 2] and [3, 4].
    node.append(&recs[0].0, &recs[0].1, 1).await;
    node.append(&recs[1].0, &recs[1].1, 2).await;
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    node.append(&recs[2].0, &recs[2].1, 3).await;
    node.append(&recs[3].0, &recs[3].1, 4).await;
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([1, 2, 3, 4])
    );

    node.commit(&recs[..2], 2).await;
    let (reached, release) = node.engine.arm_gate();
    let wal = Arc::clone(&node.wal);
    let tick = tokio::spawn(async move { wal.test_collect_now().await });
    tokio::task::spawn_blocking(move || reached.recv())
        .await
        .expect("join")
        .expect("the tick reached its sync");
    // A pass carrying the higher row returns while the sync is held.
    node.commit(&recs[2..], 4).await;
    release.send(()).expect("release");
    tick.await.expect("the tick");

    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([3, 4]),
        "the tick collected only up to the row it took before its sync"
    );
}

// ---------------------------------------------------------------------------
// AC4: a failed sync collects nothing
// ---------------------------------------------------------------------------

/// The WARN events of the WAL's watermark target, rendered.
#[derive(Clone, Default)]
struct WarnCapture(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarnCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let meta = event.metadata();
        if *meta.level() == tracing::Level::WARN
            && meta.target() == "topgun_server::storage::wal_watermark"
        {
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0.lock().push(message.0);
        }
    }
}

/// Runs `body` on a current-thread runtime with the WARN events of the
/// WAL's watermark target captured on this thread, and returns them.
fn with_captured_warns(body: impl std::future::Future<Output = ()>) -> Vec<String> {
    use tracing_subscriber::layer::SubscriberExt as _;
    let capture = WarnCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(body);
    let warns = capture.0.lock().clone();
    warns
}

fn ac4_with(fault: SyncFault) {
    let root = Root::new();
    let warns = with_captured_warns(async {
        let node = Node::open(&root, false);
        let recs = records(2);
        node.append(&recs[0].0, &recs[0].1, 1).await;
        node.append(&recs[1].0, &recs[1].1, 2).await;
        node.commit(&recs, 2).await;

        *node.engine.fault.lock() = Some(fault);
        let before = gc_skipped("store_sync_failed");
        node.wal.test_advance_clock(SEAL_AGE);
        assert!(node.wal.collect().await, "the tick task goes on");
        assert_eq!(
            frames_on_disk(&root.wal_dir(), P),
            BTreeSet::from([1, 2]),
            "a failed sync collects nothing"
        );
        assert!(
            gc_skipped("store_sync_failed") > before,
            "the failed sync is counted"
        );
        assert_eq!(node.engine.syncs().len(), 1);

        // Healthy again: the next tick syncs and collects.
        *node.engine.fault.lock() = None;
        assert!(node.wal.collect().await);
        assert!(
            frames_on_disk(&root.wal_dir(), P).is_empty(),
            "the next tick collects"
        );
        // The node still takes writes.
        node.append(&recs[0].0, &recs[0].1, 3).await;
    });
    let sync_warns: Vec<_> = warns
        .iter()
        .filter(|w| w.contains("the store could not be synced"))
        .collect();
    assert_eq!(
        sync_warns.len(),
        1,
        "one WARN for the failed sync: {warns:?}"
    );
}

#[test]
fn ac4_a_failed_sync_unlinks_nothing_is_counted_and_the_next_tick_collects() {
    ac4_with(SyncFault::Other);
}

#[test]
fn ac4_a_broken_engine_sync_is_counted_like_any_other_and_does_not_stop_the_node() {
    ac4_with(SyncFault::Broken);
}

// ---------------------------------------------------------------------------
// AC5: shutdown
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac5_finish_syncs_once_before_its_first_hint() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(2);
    node.append(&recs[0].0, &recs[0].1, 1).await;
    node.append(&recs[1].0, &recs[1].1, 2).await;
    // The ticks are held (`Node::open`); this pass lands after that, so a
    // returned commit is unsynced when `finish` runs.
    node.commit(&recs, 2).await;
    node.engine.clear_syncs();

    let written = node.wal.finish(true).await.expect("finish");
    assert!(written >= 1, "a hint was written");
    let syncs = node.engine.syncs();
    assert_eq!(syncs.len(), 1, "finish syncs exactly once: {syncs:?}");
    assert!(
        !syncs[0].hint_present,
        "the sync ran before the first hint was written"
    );
    assert_eq!(hint_of(&root.wal_dir()), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn ac5_a_failed_sync_at_finish_writes_no_hint() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(1);
    node.append(&recs[0].0, &recs[0].1, 1).await;
    node.commit(&recs, 1).await;
    *node.engine.fault.lock() = Some(SyncFault::Other);

    assert!(
        node.wal.finish(true).await.is_err(),
        "finish fails with the sync"
    );
    assert!(
        sidecar_files(&root.wal_dir()).is_empty(),
        "no hint was written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ac5_a_hint_is_the_lower_of_the_wal_copy_and_the_store_row() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(2);
    node.append(&recs[0].0, &recs[0].1, 1).await;
    node.append(&recs[1].0, &recs[1].1, 2).await;
    node.commit(&recs, 2).await;
    // A WAL copy with no commit behind it.
    node.wal.mark_applied(P, 5).await.expect("mark_applied");

    node.wal.finish(true).await.expect("finish");
    assert_eq!(
        hint_of(&root.wal_dir()),
        2,
        "the hint is the engine row, not the WAL's copy"
    );
}

fn ac5_hints_after_finish_survive_a_crash<T: CrashTarget>()
where
    T::Engine: 'static,
{
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().join("wal");
    let recs = records(4);
    let target = T::fresh();
    let rt = life_runtime();
    let wal = rt.block_on(async {
        let (store, wal) = open_life(target.engine(), &wal_dir, Passes::Forced).await;
        for (seq, (key, value)) in (1u64..).zip(&recs) {
            wal.append(P, &store_frame(key, value, seq))
                .await
                .expect("append");
        }
        commit_and_mark(store.as_ref(), &wal, &recs[..2], 2).await;
        commit_and_mark(store.as_ref(), &wal, &recs[2..], 4).await;
        wal.finish(true).await.expect("finish");
        Arc::downgrade(&wal)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, 0);

    let store = EngineMapDataStore::open(target.engine()).expect("store opens");
    let rows = store
        .applied_watermarks()
        .expect("rows")
        .expect("kept")
        .by_partition;
    let hints: Vec<(u32, u64)> = (0..PARTITION_COUNT)
        .filter(|&p| sidecar_path(&wal_dir, p).exists())
        .map(|p| {
            let bytes = std::fs::read(sidecar_path(&wal_dir, p)).expect("hint");
            (p, u64::from_be_bytes(bytes.try_into().expect("8 bytes")))
        })
        .collect();
    assert!(hints.contains(&(P, 4)), "the hint of P: {hints:?}");
    for (p, hint) in &hints {
        assert!(
            *hint <= rows[*p as usize],
            "hint {hint} of partition {p} is above the durable row {}",
            rows[*p as usize]
        );
    }
    let rt = life_runtime();
    rt.block_on(async {
        // Every row the hints claim: the records of frames 1..=4.
        assert_served(&store, &recs, "the rows the hints claim").await;
    });
}

#[test]
fn ac5_hints_after_finish_are_durable_rows_on_redb() {
    ac5_hints_after_finish_survive_a_crash::<RedbTarget>();
}

#[test]
fn ac5_hints_after_finish_are_durable_rows_on_whole_batches() {
    ac5_hints_after_finish_survive_a_crash::<MemTarget>();
}

// ---------------------------------------------------------------------------
// AC6: the sync is off the runtime worker
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn ac6_a_slow_sync_does_not_hold_the_runtime_worker() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let recs = records(1);
    node.append(&recs[0].0, &recs[0].1, 1).await;
    node.commit(&recs, 1).await;
    *node.engine.delay.lock() = Duration::from_secs(2);

    let wal = Arc::clone(&node.wal);
    let tick = tokio::spawn(async move { wal.test_collect_now().await });
    // 500 ms gives the blocking pool room to enter the sync on a loaded
    // runner; the sync stays parked for 2 s, so it has not returned yet.
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let elapsed = started.elapsed();
    assert_eq!(node.engine.syncs().len(), 1, "the tick is in its sync");
    assert_eq!(
        node.engine.syncs_done.load(Ordering::SeqCst),
        0,
        "the sync has not returned yet"
    );
    assert!(
        elapsed < Duration::from_millis(600),
        "a task on the one worker was delayed {elapsed:?} by the sync"
    );
    tick.await.expect("the tick");
}

// ---------------------------------------------------------------------------
// AC7: the fence
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac7_the_fence_moves_only_after_a_sync_and_never_back() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let wb = node.write_behind().await;
    let recs = records(3);
    let mut seen = Vec::new();

    let f0 = wb.flushed_watermark();
    seen.push(f0);
    write_through(&wb, &recs[..1]).await;
    let a1 = wb.assigned_write_sequence();
    assert!(a1 > f0);
    for _ in 0..2 {
        let f = wb.flushed_watermark();
        seen.push(f);
        assert_eq!(f, f0, "an unsynced pass does not move the fence");
    }

    node.wal.test_collect_now().await;
    let f = wb.flushed_watermark();
    seen.push(f);
    assert_eq!(f, a1, "the tick's sync publishes the captured value");

    // Captured, then a later pass, then a sync that covers only the capture:
    // the later value waits for the next sync.
    write_through(&wb, &recs[1..2]).await;
    let a2 = wb.assigned_write_sequence();
    seen.push(wb.flushed_watermark());
    node.wal.test_collect_now().await;
    write_through(&wb, &recs[2..3]).await;
    let a3 = wb.assigned_write_sequence();
    let f = wb.flushed_watermark();
    seen.push(f);
    assert_eq!(f, a2, "the capture is published, the later value is not");
    let f = wb.flushed_watermark();
    seen.push(f);
    assert_eq!(f, a2, "not before the next sync");
    node.wal.test_collect_now().await;
    let f = wb.flushed_watermark();
    seen.push(f);
    assert_eq!(f, a3, "published after the following sync");

    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "the fence never decreases: {seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ac7_with_synced_passes_the_fence_is_the_raw_value_at_once() {
    let root = Root::new();
    let node = Node::open(&root, true);
    let wb = node.write_behind().await;
    for rec in records(3).chunks(1) {
        write_through(&wb, rec).await;
        assert_eq!(
            wb.flushed_watermark(),
            wb.assigned_write_sequence(),
            "a synced pass publishes at once"
        );
    }
}

#[test]
fn ac7_a_commit_between_the_fence_reads_is_not_published_before_a_sync() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().join("wal");
    let recs = records(3);
    let target = RedbTarget::fresh();
    let rt = life_runtime();
    let (wal, covered) = rt.block_on(async {
        let (store, wal) = open_life(target.engine(), &wal_dir, Passes::Forced).await;
        let wb = write_behind_over(Arc::clone(&store) as Arc<dyn MapDataStore>, &wal).await;
        let mut after = Vec::new();
        for rec in recs[..2].chunks(1) {
            write_through(&wb, rec).await;
            after.push(wb.assigned_write_sequence());
        }
        // Synced just before the call: durable == returned.
        wal.test_collect_now().await;
        assert_eq!(wb.flushed_watermark(), after[1], "published after the sync");

        let (reached, release) = wb.arm_fence_park();
        let fence_wb = Arc::clone(&wb);
        let fence = std::thread::spawn(move || fence_wb.flushed_watermark());
        tokio::task::spawn_blocking(move || reached.recv())
            .await
            .expect("join")
            .expect("the fence reached its park");
        // One more pass commit and its settle while the fence is parked.
        write_through(&wb, &recs[2..]).await;
        after.push(wb.assigned_write_sequence());
        release.send(()).expect("release");
        let value = fence.join().expect("the fence call");
        let covered: Vec<usize> = after
            .iter()
            .enumerate()
            .filter(|(_, a)| **a <= value)
            .map(|(i, _)| i)
            .collect();
        (Arc::downgrade(&wal), covered)
    });
    crash_life(rt, &wal);
    let target = target.crash(CrashMode::DropUnsynced, 0);

    let store = EngineMapDataStore::open(target.engine()).expect("store opens");
    let rt = life_runtime();
    rt.block_on(async {
        for i in covered {
            let (key, value) = &recs[i];
            assert_eq!(
                store.load(MAP, key).await.expect("load").as_ref(),
                Some(value),
                "write {i} is under the published fence, so it is in the crash image"
            );
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn ac7_over_a_store_without_counters_the_fence_is_the_raw_value() {
    let root = Root::new();
    let engine = Arc::new(SyncEngine::new(open_engine(&root), root.wal_dir()));
    let mut inner = EngineMapDataStore::open(engine).expect("store opens");
    inner.set_pass_sync(false);
    // Keeps the defaults: `commit_durability` is `None`.
    let store: Arc<dyn MapDataStore> = Arc::new(WatermarkKeeping::new(inner));
    assert!(store.commit_durability().is_none());
    let wb = WriteBehindDataStore::new(store, wb_config());
    for rec in records(2).chunks(1) {
        write_through(&wb, rec).await;
        assert_eq!(
            wb.flushed_watermark(),
            wb.assigned_write_sequence(),
            "the raw value, unsynced passes or not"
        );
        assert_eq!(wb.test_fence_state(), (0, None), "no published state");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ac7_the_fence_does_not_wait_on_a_pass_inside_its_engine_write() {
    let root = Root::new();
    let node = Node::open(&root, false);
    let wb = node.write_behind().await;
    let recs = records(2);
    write_through(&wb, &recs[..1]).await;

    node.engine
        .inner
        .set_write_delay(Duration::from_millis(500));
    let entered = node.engine.writes_entered.load(Ordering::SeqCst);
    wb.add(MAP, &recs[1].0, &recs[1].1, 0)
        .await
        .expect("acknowledged write");
    wb.soft_flush().await.expect("soft_flush");
    let deadline = Instant::now() + Duration::from_secs(30);
    while node.engine.writes_entered.load(Ordering::SeqCst) == entered {
        assert!(
            Instant::now() < deadline,
            "the pass never reached its write"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let fence_wb = Arc::clone(&wb);
    let took = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let _ = fence_wb.flushed_watermark();
        started.elapsed()
    })
    .await
    .expect("join");
    // The pass is parked for 500 ms; 100 ms leaves room for a loaded runner
    // while a fence that waits on the pass still fails.
    assert!(
        took < Duration::from_millis(100),
        "the fence waited {took:?} on a pass's engine write"
    );
    node.engine.inner.set_write_delay(Duration::ZERO);
    flush_idle(&wb).await;
}

// ---------------------------------------------------------------------------
// AC10: wal_attached
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac10_a_wal_opened_over_the_store_marks_it_attached() {
    let root = Root::new();
    let engine = Arc::new(SyncEngine::new(open_engine(&root), root.wal_dir()));
    let store = Arc::new(EngineMapDataStore::open(engine).expect("store opens"));
    assert!(!store.is_wal_attached(), "no WAL over it yet");
    let _wal = WalWriter::open_over_store(
        root.wal_dir(),
        WalFsyncPolicy::PerOp,
        Arc::clone(&store) as Arc<dyn MapDataStore>,
    )
    .expect("store-mode open");
    assert!(store.is_wal_attached(), "the WAL's open marked it");
}
