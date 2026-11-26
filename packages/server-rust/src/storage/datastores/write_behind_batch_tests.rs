//! Main tests of the batched flush pass, the store-wide hold after a refused commit and
//! the batched shutdown drain.
//!
//! Every test runs write-behind over a file-backed [`EngineMapDataStore`] whose
//! engine is a [`BatchEngine`]: a [`KvEngine`] over [`RedbKvEngine`] that logs
//! every `write` with its operations and its `sync` flag, and can be armed to
//! refuse a write (the `n`-th, every one naming a key, or all of them) with a
//! chosen error class, or to sleep inside a write. Between write-behind and the
//! engine store sits a [`CommitSpy`], which forwards every call and logs every
//! commit with the form of each put, so a test can tell an encoded put from a
//! borrowed value. The WAL is the store-mode WAL over that spy, so every commit
//! carries its applied watermark rows and a restart can say what would replay.

#![cfg(feature = "redb")]
// Each test is one scenario read top to bottom — fill, pass or stop, check —
// and splitting one across helpers would hide the order its claim rests on.
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_core::PARTITION_COUNT;

use super::{
    WalBootstrap, WriteBehindConfig, WriteBehindDataStore, FLUSH_PASSES_COUNTER,
    FLUSH_PASS_ENTRIES_COUNTER, FLUSH_PASS_SECONDS_HISTOGRAM,
};
use crate::storage::datastores::engine_store::{
    EngineMapDataStore, RecordKey, KS_BACKUP, KS_PARTITION_META, KS_PRIMARY,
};
use crate::storage::datastores::kv_engine::{
    BatchOp, KeyRange, KvCapabilities, KvEngine, KvError, KvResult, KvSnapshot, WriteBatch,
    WriteOptions,
};
use crate::storage::datastores::RedbKvEngine;
use crate::storage::map_data_store::{
    AppliedWatermarks, FlushCommit, KeyVersion, LeafSink, Loaded, MapDataStore, PutValue,
    ScanBatch, ScanCursor, ScanOrder, StoreBrokenCause, StoreErrorClass, StoreWrite, WriteSource,
};
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::wal::{OrDelta, Wal, WalFsyncPolicy, WalWriter};

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

/// The error class an armed write is refused with.
#[derive(Debug, Clone, Copy)]
enum Refusal {
    /// An engine error that says nothing about the engine's health: the
    /// store classifies it `Unknown`.
    Unknown,
    /// The engine reports itself broken: the store classifies it
    /// `StoreBroken`.
    Broken(StoreBrokenCause),
}

impl Refusal {
    fn error(self) -> KvError {
        match self {
            Self::Unknown => KvError::Other(anyhow::anyhow!("injected engine write refusal")),
            Self::Broken(cause) => KvError::Broken {
                cause,
                detail: "injected broken engine".to_string(),
            },
        }
    }
}

/// One `write` the engine was given, in call order.
#[derive(Debug, Clone)]
struct EngineWrite {
    ops: Vec<BatchOp>,
    sync: bool,
    ok: bool,
    /// The runtime's clock when the write was made.
    at: tokio::time::Instant,
    /// The runtime's clock when the write was called, before any delay.
    began: tokio::time::Instant,
    /// The runtime's clock when the write returned.
    ended: tokio::time::Instant,
}

impl EngineWrite {
    /// The user keys of the primary records the batch puts or deletes.
    fn user_keys(&self) -> Vec<String> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                BatchOp::Put { key, .. } | BatchOp::Delete { key } => user_key_of(key),
                BatchOp::DeleteRange { .. } => None,
            })
            .collect()
    }

    /// The number of primary records the batch puts or deletes.
    fn records(&self) -> usize {
        self.user_keys().len()
    }

    /// The applied watermark rows the batch puts, by WAL partition.
    fn rows(&self) -> BTreeMap<u32, u64> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                BatchOp::Put { key, value } if key.first() == Some(&KS_PARTITION_META) => {
                    Some((decode_row_partition(key), decode_u64(value)))
                }
                _ => None,
            })
            .collect()
    }
}

/// The user key of a primary record key; `None` for every other key.
fn user_key_of(key: &[u8]) -> Option<String> {
    if key.first() != Some(&KS_PRIMARY) {
        return None;
    }
    RecordKey::decode(key).ok().map(|record| record.key)
}

/// Whether `key` is a record key (primary or backup) of user key `user`.
fn names_user_key(key: &[u8], user: &str) -> bool {
    key.first()
        .is_some_and(|b| *b == KS_PRIMARY || *b == KS_BACKUP)
        && RecordKey::decode(key).is_ok_and(|record| record.key == user)
}

/// The partition of an applied watermark row key, whose format the tests pin:
/// `0x20 ‖ partition u32 BE ‖ b"applied"`.
fn decode_row_partition(key: &[u8]) -> u32 {
    assert_eq!(key.len(), 12, "a 0x20 row key is 12 bytes: {key:?}");
    assert_eq!(&key[5..], b"applied", "a 0x20 row key ends in `applied`");
    u32::from_be_bytes(key[1..5].try_into().expect("4 bytes"))
}

fn decode_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().expect("an 8-byte value"))
}

/// A [`KvEngine`] over [`RedbKvEngine`] that logs every `write` and can be
/// armed to refuse writes or to sleep inside them.
struct BatchEngine {
    inner: RedbKvEngine,
    log: Mutex<Vec<EngineWrite>>,
    /// Index in `log` of the one write that is refused, and how.
    refuse_at: Mutex<Option<(usize, Refusal)>>,
    /// Every write naming one of these user keys is refused, as given.
    refuse_keys: Mutex<BTreeMap<String, Refusal>>,
    /// While set, every write is refused, as given.
    refuse_all: Mutex<Option<Refusal>>,
    /// Index in `log` of the one write that sleeps, and for how long.
    delay_at: Mutex<Option<(usize, Duration)>>,
    /// While set, every write sleeps this long before it reaches the engine.
    delay_all: Mutex<Option<Duration>>,
}

impl BatchEngine {
    fn open(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            inner: RedbKvEngine::open(path).expect("the engine file opens"),
            log: Mutex::new(Vec::new()),
            refuse_at: Mutex::new(None),
            refuse_keys: Mutex::new(BTreeMap::new()),
            refuse_all: Mutex::new(None),
            delay_at: Mutex::new(None),
            delay_all: Mutex::new(None),
        })
    }

    /// The number of writes logged so far.
    fn mark(&self) -> usize {
        self.log.lock().len()
    }

    /// Every write logged from `mark` on, refused ones included.
    fn writes_since(&self, mark: usize) -> Vec<EngineWrite> {
        self.log.lock()[mark..].to_vec()
    }

    /// The `n`-th write from now (`n >= 1`) is refused, once.
    fn refuse_write_in(&self, n: usize, refusal: Refusal) {
        assert!(n >= 1, "refuse_write_in takes n >= 1");
        *self.refuse_at.lock() = Some((self.mark() + n - 1, refusal));
    }

    /// Every write naming the record of user key `key` is refused from now on.
    fn refuse_writes_holding(&self, key: &str, refusal: Refusal) {
        self.refuse_keys.lock().insert(key.to_string(), refusal);
    }

    /// Every write is refused while `Some`.
    fn refuse_every_write(&self, refusal: Option<Refusal>) {
        *self.refuse_all.lock() = refusal;
    }

    /// The `n`-th write from now (`n >= 1`) sleeps `delay` before it reaches
    /// the engine, on the calling thread, as a slow engine write does.
    fn delay_write_in(&self, n: usize, delay: Duration) {
        assert!(n >= 1, "delay_write_in takes n >= 1");
        *self.delay_at.lock() = Some((self.mark() + n - 1, delay));
    }

    /// Writes naming the record of user key `key` are no longer refused for
    /// it.
    fn allow_writes_holding(&self, key: &str) {
        self.refuse_keys.lock().remove(key);
    }

    /// Every write sleeps `delay` before it reaches the engine while `Some`,
    /// on the calling thread: real time passes, a paused clock does not move.
    fn delay_every_write(&self, delay: Option<Duration>) {
        *self.delay_all.lock() = delay;
    }
}

impl KvEngine for BatchEngine {
    fn capabilities(&self) -> KvCapabilities {
        self.inner.capabilities()
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        let began = tokio::time::Instant::now();
        // Before the log lock: a reader of the log is not kept waiting on a
        // sleeping write.
        let delay_all = *self.delay_all.lock();
        if let Some(delay) = delay_all {
            std::thread::sleep(delay);
        }
        // Held across the inner write, so the log's order is the order the
        // engine applied the batches in.
        let mut log = self.log.lock();
        let index = log.len();
        if let Some((_, delay)) = (*self.delay_at.lock()).filter(|(at, _)| *at == index) {
            std::thread::sleep(delay);
        }
        let at = tokio::time::Instant::now();
        let refusal = (*self.refuse_at.lock())
            .filter(|(at, _)| *at == index)
            .map(|(_, refusal)| refusal)
            .or(*self.refuse_all.lock())
            .or_else(|| {
                let keys = self.refuse_keys.lock();
                batch.ops().iter().find_map(|op| match op {
                    BatchOp::Put { key, .. } | BatchOp::Delete { key } => keys
                        .iter()
                        .find(|(user, _)| names_user_key(key, user))
                        .map(|(_, refusal)| *refusal),
                    BatchOp::DeleteRange { .. } => None,
                })
            });
        let logged_ops = batch.ops().to_vec();
        let result = match refusal {
            Some(refusal) => Err(refusal.error()),
            None => self.inner.write(batch, opts),
        };
        log.push(EngineWrite {
            ops: logged_ops,
            sync: opts.sync,
            ok: result.is_ok(),
            at,
            began,
            ended: tokio::time::Instant::now(),
        });
        result
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
        self.inner.sync()
    }

    fn shutdown(&self) -> KvResult<()> {
        self.inner.shutdown()
    }
}

// ---------------------------------------------------------------------------
// The commit spy
// ---------------------------------------------------------------------------

/// How a write of a commit carried its record.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PutForm {
    /// `PutValue::Encoded`, with the length of the bytes.
    Encoded(usize),
    /// `PutValue::Value`.
    Value,
    /// A remove.
    Remove,
}

/// One `commit` write-behind made, in call order.
#[derive(Debug, Clone)]
struct CommitCall {
    /// `(map, key, form)` of each write, in commit order.
    writes: Vec<(String, String, PutForm)>,
}

/// A [`MapDataStore`] that forwards every call to the engine store and logs
/// every commit.
///
/// The OR-witness methods keep their trait defaults, which are what the
/// engine store answers too: a package-wide count allows exactly one test
/// override of them, and it is not this one.
struct CommitSpy {
    inner: Arc<EngineMapDataStore<BatchEngine>>,
    commits: Mutex<Vec<CommitCall>>,
    /// `(key, seq, ok)` of every write of every commit, in commit order.
    versions: Mutex<Vec<(String, u64, bool)>>,
    /// One-shot: the next commit naming the key parks before it reaches the
    /// store, notifying `reached` and waiting for `release`.
    park: Mutex<Option<CommitPark>>,
}

/// `(key, reached, release)` of a parked commit.
type CommitPark = (String, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);

impl CommitSpy {
    fn commits_since(&self, mark: usize) -> Vec<CommitCall> {
        self.commits.lock()[mark..].to_vec()
    }
}

#[async_trait]
impl MapDataStore for CommitSpy {
    async fn add(&self, map: &str, key: &str, value: &RecordValue, now: i64) -> anyhow::Result<()> {
        self.inner.add(map, key, value, now).await
    }

    fn accepts_encoded(&self) -> bool {
        self.inner.accepts_encoded()
    }

    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        now: i64,
    ) -> anyhow::Result<()> {
        self.inner.add_encoded(map, key, bytes, now).await
    }

    fn scan_order(&self) -> Option<ScanOrder> {
        self.inner.scan_order()
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        now: i64,
    ) -> anyhow::Result<()> {
        self.inner.add_backup(map, key, value, now).await
    }

    async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        self.inner.remove(map, key, now).await
    }

    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        self.inner.remove_backup(map, key, now).await
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        self.inner.load(map, key).await
    }

    async fn load_slot(&self, map: &str, key: &str) -> anyhow::Result<Option<Loaded>> {
        self.inner.load_slot(map, key).await
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        self.inner.load_all(map, keys).await
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
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

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        self.inner.list_maps().await
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

    fn assigned_write_sequence(&self) -> u64 {
        self.inner.assigned_write_sequence()
    }

    fn flushed_watermark(&self) -> u64 {
        self.inner.flushed_watermark()
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        self.inner.hard_flush().await
    }

    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    fn classify_error(&self, err: &anyhow::Error) -> StoreErrorClass {
        self.inner.classify_error(err)
    }

    async fn commit(&self, commit: &FlushCommit<'_>) -> anyhow::Result<()> {
        let seqs: Vec<(String, u64)> = commit
            .writes
            .iter()
            .map(|write| match write {
                StoreWrite::Put { key, seq, .. } | StoreWrite::Remove { key, seq, .. } => {
                    ((*key).to_string(), *seq)
                }
            })
            .collect();
        let park = {
            let mut park = self.park.lock();
            if park
                .as_ref()
                .is_some_and(|(key, ..)| seqs.iter().any(|(k, _)| k == key))
            {
                park.take()
            } else {
                None
            }
        };
        if let Some((_, reached, release)) = park {
            reached.notify_one();
            release.notified().await;
        }
        let writes = commit
            .writes
            .iter()
            .map(|write| match write {
                StoreWrite::Put {
                    map, key, value, ..
                } => (
                    (*map).to_string(),
                    (*key).to_string(),
                    match value {
                        PutValue::Encoded(bytes) => PutForm::Encoded(bytes.len()),
                        PutValue::Value(_) => PutForm::Value,
                    },
                ),
                StoreWrite::Remove { map, key, .. } => {
                    ((*map).to_string(), (*key).to_string(), PutForm::Remove)
                }
            })
            .collect();
        let result = self.inner.commit(commit).await;
        self.versions.lock().extend(
            seqs.into_iter()
                .map(|(key, seq)| (key, seq, result.is_ok())),
        );
        self.commits.lock().push(CommitCall { writes });
        result
    }

    fn keeps_applied_watermarks(&self) -> bool {
        self.inner.keeps_applied_watermarks()
    }

    fn applied_watermarks(&self) -> anyhow::Result<Option<AppliedWatermarks>> {
        self.inner.applied_watermarks()
    }

    fn import_applied_watermarks(&self, rows: &[(u32, u64)]) -> anyhow::Result<()> {
        self.inner.import_applied_watermarks(rows)
    }

    async fn load_version(&self, map: &str, key: &str) -> anyhow::Result<Option<KeyVersion>> {
        self.inner.load_version(map, key).await
    }

    fn reclaim_record_versions(&self, watermarks: &[u64]) -> anyhow::Result<u64> {
        self.inner.reclaim_record_versions(watermarks)
    }
}

// ---------------------------------------------------------------------------
// A node: engine, spy, store-mode WAL and write-behind
// ---------------------------------------------------------------------------

/// The map every test writes, unless it names another.
const MAP: &str = "m";

const MIB: usize = 1024 * 1024;

/// The store over a batch engine, the spy over it and a store-mode WAL over
/// the spy, in `dir`.
struct Node {
    engine: Arc<BatchEngine>,
    spy: Arc<CommitSpy>,
    wal: Arc<WalWriter>,
}

impl Node {
    fn open(dir: &Path) -> Self {
        let data_dir = dir.join("topgun-data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        let engine = BatchEngine::open(&data_dir.join("engine.redb"));
        let store = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).expect("store open"));
        let spy = Arc::new(CommitSpy {
            inner: store,
            commits: Mutex::new(Vec::new()),
            versions: Mutex::new(Vec::new()),
            park: Mutex::new(None),
        });
        let wal = WalWriter::open_over_store(
            dir.join("wal"),
            WalFsyncPolicy::None,
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
        )
        .expect("WAL open");
        // These tests count every engine write; a collection tick, every 5 s
        // of the runtime's clock, reclaims record versions with an engine
        // write of its own, which would land in a count whenever a slow run
        // reaches the tick. No test here relies on the tick.
        wal.test_hold_collection_task(true);
        Self { engine, spy, wal }
    }

    /// Write-behind over the spy and the WAL, seeded as the server seeds it.
    async fn write_behind(&self, config: WriteBehindConfig) -> Arc<WriteBehindDataStore> {
        let sequence_start = self
            .wal
            .max_observed_sequence()
            .await
            .expect("max_observed_sequence")
            + 1;
        WriteBehindDataStore::new_with_wal(
            Arc::clone(&self.spy) as Arc<dyn MapDataStore>,
            config,
            Some(WalBootstrap {
                wal: Arc::clone(&self.wal) as Arc<dyn Wal>,
                sequence_start,
            }),
        )
    }

    /// Of `keys` of `MAP`, those the store holds.
    async fn stored(&self, keys: &[String]) -> BTreeSet<String> {
        self.spy
            .load_all(MAP, keys)
            .await
            .expect("load_all")
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// The keys whose frames a start on this WAL would replay, by partition.
    async fn unapplied_keys(&self) -> BTreeMap<u32, BTreeSet<String>> {
        let mut unapplied = BTreeMap::new();
        for partition in 0..PARTITION_COUNT {
            let frames = self.wal.unapplied(partition).await.expect("unapplied");
            if !frames.is_empty() {
                unapplied.insert(
                    partition,
                    frames.into_iter().map(|frame| frame.key).collect(),
                );
            }
        }
        unapplied
    }

    /// Drops the WAL and the store and waits until nothing holds either, so
    /// the engine file is closed and the next open of the directory is a
    /// restart.
    async fn close(self) {
        let Self { engine, spy, wal } = self;
        let wal_weak = Arc::downgrade(&wal);
        drop(wal);
        drop(spy);
        let limit = std::time::Instant::now() + Duration::from_secs(30);
        while wal_weak.upgrade().is_some() || Arc::strong_count(&engine) > 1 {
            assert!(
                std::time::Instant::now() < limit,
                "the WAL writer and the store were not released"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        drop(engine);
    }
}

/// Stops write-behind as a clean server stop does, and drops it.
async fn stop(wb: Arc<WriteBehindDataStore>) {
    wb.hard_flush().await.expect("hard_flush");
    drop(wb);
}

/// Every write is due at once; a pass begins only when a test asks for one;
/// the hold after a refused commit outlasts every test.
fn config() -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 0,
        flush_interval_ms: 600_000,
        backoff_base_ms: 600_000,
        backoff_cap_ms: 600_000,
        shutdown_timeout_ms: 60_000,
        capacity: 0,
        ..WriteBehindConfig::default()
    }
}

fn lww(text: &str, millis: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::String(text.to_string()),
        timestamp: Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        },
        ttl_ms: None,
    }
}

/// `n` keys `{prefix}{i:05}`, in the order they are written.
fn keys(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i:05}")).collect()
}

/// Writes `keys` to `MAP` in order, the `i`-th with `store_time = first + i`,
/// so the pass order is the order of `keys`.
async fn fill(
    wb: &WriteBehindDataStore,
    keys: &[String],
    first: i64,
    value: impl Fn(usize) -> RecordValue,
) {
    for (i, key) in keys.iter().enumerate() {
        let store_time = first + i64::try_from(i).expect("small index");
        wb.add(MAP, key, &value(i), store_time).await.expect("add");
    }
}

/// Polls `cond` every 2 ms of the runtime's clock for at most `limit_ms` and
/// says after how many milliseconds it first held.
async fn poll_until(limit_ms: u64, mut cond: impl FnMut() -> bool) -> Option<u64> {
    let start = tokio::time::Instant::now();
    loop {
        let waited = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        if cond() {
            return Some(waited);
        }
        if waited >= limit_ms {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Wakes the flush task once and waits until a pass has ended and none is
/// open.
async fn one_pass(wb: &WriteBehindDataStore) {
    let before = wb.test_passes_closed();
    wb.soft_flush().await.expect("soft_flush");
    poll_until(120_000, || {
        wb.test_passes_closed() > before && !wb.test_pass_open()
    })
    .await
    .expect("a flush pass ended");
}

/// Wakes the flush task until nothing is outstanding.
async fn flush_until_idle(wb: &WriteBehindDataStore) {
    let start = tokio::time::Instant::now();
    while !wb.test_is_idle() {
        assert!(
            start.elapsed() < Duration::from_secs(600),
            "write-behind did not become idle"
        );
        wb.soft_flush().await.expect("soft_flush");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Wakes the flush task until no entry of `keys` of `MAP` is queued and no
/// pass is open, for a test whose refused entries stay queued.
async fn flush_until_gone(wb: &WriteBehindDataStore, keys: &[String]) {
    let start = tokio::time::Instant::now();
    while wb.test_pass_open()
        || keys
            .iter()
            .any(|key| wb.test_buffered_at(MAP, key).is_some())
    {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the entries were not stored"
        );
        wb.soft_flush().await.expect("soft_flush");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// For each of `keys` of `MAP`, whether its entry is still queued.
fn queued(wb: &WriteBehindDataStore, keys: &[String]) -> Vec<bool> {
    keys.iter()
        .map(|key| wb.test_buffered_at(MAP, key).is_some())
        .collect()
}

/// The record counts of `writes`, in order.
fn record_counts(writes: &[EngineWrite]) -> Vec<usize> {
    writes.iter().map(EngineWrite::records).collect()
}

/// A current-thread runtime on a paused clock, for a test that binds a
/// metrics recorder or a `tracing` subscriber to its thread: on such a
/// runtime that is also the thread the flush task runs on.
fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .expect("current-thread runtime")
}

/// The value of the series `series` (name, with its labels if any) on a
/// rendered scrape.
fn metric(rendered: &str, series: &str) -> Option<f64> {
    rendered
        .lines()
        .find(|line| line.starts_with(&format!("{series} ")))
        .and_then(|line| line[series.len() + 1..].trim().parse().ok())
}

// ---------------------------------------------------------------------------
// Log capture
// ---------------------------------------------------------------------------

/// One `tracing` event: its level and its fields by name, the text under
/// `message`.
#[derive(Debug, Clone)]
struct Event {
    level: tracing::Level,
    fields: BTreeMap<String, String>,
}

impl Event {
    fn says(&self, text: &str) -> bool {
        self.fields.get("message").map(String::as_str) == Some(text)
    }
}

/// Collects every event of the thread it is the default subscriber on.
struct EventCapture(Arc<Mutex<Vec<Event>>>);

struct EventFields<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for EventFields<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::new();
        event.record(&mut EventFields(&mut fields));
        self.0.lock().push(Event {
            level: *event.metadata().level(),
            fields,
        });
    }
}

const RETRY_WARNING: &str = "Write-behind entries are being retried: the store refuses them";

// ---------------------------------------------------------------------------
// b01–b09
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn b01_a_pass_is_one_durable_engine_write_holding_its_entries_and_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;
    let keys = keys("k", 500);
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    let partitions: BTreeSet<u32> = keys
        .iter()
        .map(|key| super::partition_for(MAP, key))
        .collect();
    let expected_rows: BTreeMap<u32, u64> = partitions
        .iter()
        .map(|&p| (p, wb.test_max_assigned_wal_sequence(p)))
        .collect();

    let mark = node.engine.mark();
    one_pass(&wb).await;
    let writes = node.engine.writes_since(mark);

    let first = writes
        .first()
        .map(|w| (w.ok, w.sync, w.records(), w.rows()));
    let stored = node.stored(&keys).await.len();
    let still_pending: usize = partitions
        .iter()
        .map(|&p| wb.test_pending_wal_sequences(p).len())
        .sum();
    assert_eq!(
        (writes.len(), first, stored, still_pending),
        (1, Some((true, true, 500, expected_rows)), 500, 0),
        "record counts of the pass's engine writes: {:?}",
        record_counts(&writes)
    );
}

#[tokio::test(start_paused = true)]
async fn b02_a_pass_takes_at_most_1000_entries_or_8_mib_and_the_first_entry_whole() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;

    // 2 500 small entries: three passes, of 1000, 1000 and 500.
    let small = keys("s", 2_500);
    fill(&wb, &small, 1, |i| lww(&format!("v{i}"), 1)).await;
    let mark = node.engine.mark();
    flush_until_idle(&wb).await;
    let by_count = record_counts(&node.engine.writes_since(mark));

    // Three values of 3 MiB: two fit 8 MiB, the third would carry the pass
    // over it.
    let three_mib = keys("t", 3);
    fill(&wb, &three_mib, 10_000, |_| lww(&"x".repeat(3 * MIB), 1)).await;
    let mark = node.engine.mark();
    flush_until_idle(&wb).await;
    let by_bytes = record_counts(&node.engine.writes_since(mark));

    // One value of 9 MiB, above the limit on its own: admitted whole, alone.
    let nine_mib = keys("n", 1);
    fill(&wb, &nine_mib, 20_000, |_| lww(&"x".repeat(9 * MIB), 1)).await;
    let mark = node.engine.mark();
    flush_until_idle(&wb).await;
    let oversized = record_counts(&node.engine.writes_since(mark));

    let mut all = small.clone();
    all.extend(three_mib);
    all.extend(nine_mib);
    let stored = node.stored(&all).await.len();
    assert_eq!(
        (by_count, by_bytes, oversized, stored),
        (vec![1000, 1000, 500], vec![2, 1], vec![1], 2_504)
    );
}

#[test]
fn b03_passes_run_back_to_back_while_entries_are_due() {
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let (during_fill, counts, idle_after, span, rendered) =
        metrics::with_local_recorder(&recorder, || {
            paused_runtime().block_on(async {
                let dir = tempfile::tempdir().expect("tempdir");
                let node = Node::open(dir.path());
                let wb = node
                    .write_behind(WriteBehindConfig {
                        flush_interval_ms: 60_000,
                        ..config()
                    })
                    .await;
                let keys = keys("k", 2_500);
                let mark = node.engine.mark();
                fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
                let during_fill = node.engine.mark() - mark;

                // One wake; no other until the 60 s interval.
                let woken = tokio::time::Instant::now();
                wb.soft_flush().await.expect("soft_flush");
                let idle_after = poll_until(2_000, || wb.test_is_idle()).await;
                let writes = node.engine.writes_since(mark);
                let span = writes.last().map(|w| w.at.saturating_duration_since(woken));
                (
                    during_fill,
                    record_counts(&writes),
                    idle_after,
                    span,
                    handle.render(),
                )
            })
        });
    let passes = metric(&rendered, FLUSH_PASSES_COUNTER);
    let entries = metric(&rendered, FLUSH_PASS_ENTRIES_COUNTER);
    let observed = metric(&rendered, &format!("{FLUSH_PASS_SECONDS_HISTOGRAM}_count"));
    assert_eq!(
        (
            during_fill,
            counts,
            idle_after.is_some(),
            span.is_some_and(|s| s <= Duration::from_secs(2)),
            passes,
            entries,
            observed,
        ),
        (
            0,
            vec![1000, 1000, 500],
            true,
            true,
            Some(3.0),
            Some(2_500.0),
            Some(3.0)
        ),
        "idle after {idle_after:?} ms, last write {span:?} after the wake"
    );
}

#[test]
fn b05_a_store_that_refuses_every_commit_starts_a_hold_after_one_call() {
    use tracing_subscriber::layer::SubscriberExt;

    const HOLD: &str = "topgun_write_behind_store_hold";
    const REFUSALS: &str = "topgun_write_behind_flush_refusals_total";

    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    let (at_hold, over_a_minute) = metrics::with_local_recorder(&recorder, || {
        crate::storage::wal::with_thread_subscriber(subscriber, || {
            paused_runtime().block_on(async {
                let dir = tempfile::tempdir().expect("tempdir");
                let node = Node::open(dir.path());
                let wb = node
                    .write_behind(WriteBehindConfig {
                        flush_interval_ms: 100,
                        backoff_base_ms: 500,
                        backoff_cap_ms: 5_000,
                        ..config()
                    })
                    .await;
                let keys = keys("k", 4);
                fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
                node.engine.refuse_every_write(Some(Refusal::Unknown));

                // An interest computation another thread began, from its own
                // default, before the capture was installed may have finished
                // after the capture's own rebuild; reading every interest
                // again now, with both dispatchers alive, undoes that.
                tracing::callsite::rebuild_interest_cache();
                // The hold's start, before its first retry pass (500 ms later).
                let mark = node.engine.mark();
                wb.soft_flush().await.expect("soft_flush");
                poll_until(400, || wb.test_store_hold_active() && !wb.test_pass_open()).await;
                let writes = node.engine.writes_since(mark);
                let rendered = handle.render();
                let warnings: Vec<Option<String>> = events
                    .lock()
                    .iter()
                    .filter(|e| e.level == tracing::Level::WARN && e.says(RETRY_WARNING))
                    .map(|e| e.fields.get("hold").cloned())
                    .collect();
                let at_hold = (
                    writes
                        .iter()
                        .map(EngineWrite::user_keys)
                        .collect::<Vec<_>>(),
                    wb.test_store_hold_active(),
                    queued(&wb, &keys),
                    warnings,
                    metric(&rendered, HOLD),
                    metric(&rendered, REFUSALS),
                );

                // A minute from the first refusal: calls at 0, 0.5, 1.5, 3.5,
                // 7.5 s and then every 5 s, 15 at most.
                let first = writes.first().map(|w| w.at);
                tokio::time::sleep(Duration::from_secs(61)).await;
                let in_a_minute = node
                    .engine
                    .writes_since(mark)
                    .iter()
                    .filter(|w| {
                        first.is_some_and(|f| {
                            w.at.saturating_duration_since(f) <= Duration::from_secs(60)
                        })
                    })
                    .count();
                (at_hold, in_a_minute)
            })
        })
    });
    let all: Vec<String> = keys("k", 4);
    assert_eq!(
        (at_hold, over_a_minute <= 15),
        (
            (
                vec![all],
                true,
                vec![true; 4],
                vec![Some("true".to_string())],
                Some(1.0),
                Some(1.0),
            ),
            true
        ),
        "{over_a_minute} store calls in the minute from the first refusal"
    );
}

#[tokio::test(start_paused = true)]
async fn b07_the_pass_byte_limit_counts_encoded_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;

    // The 8 MiB limit measures the encoded bytes: two values whose strings
    // together fit 8 MiB but whose encodings do not take two passes.
    let half = 4 * MIB - 8;
    let encoded_len = rmp_serde::to_vec_named(&lww(&"x".repeat(half), 1))
        .expect("encode")
        .len();
    assert!(
        2 * half <= 8 * MIB && 2 * encoded_len > 8 * MIB,
        "the two values straddle the limit only by their encoding: {encoded_len}"
    );
    let large = keys("l", 2);
    fill(&wb, &large, 300, |_| lww(&"x".repeat(half), 1)).await;
    let engine_mark = node.engine.mark();
    flush_until_gone(&wb, &large).await;
    let by_encoded_bytes = record_counts(&node.engine.writes_since(engine_mark));

    let forms: BTreeSet<PutForm> = node
        .spy
        .commits_since(0)
        .iter()
        .flat_map(|c| c.writes.iter().map(|(_, _, form)| form.clone()))
        .map(|form| match form {
            PutForm::Encoded(_) => PutForm::Encoded(0),
            other => other,
        })
        .collect();
    assert_eq!(
        (by_encoded_bytes, forms),
        (vec![1, 1], BTreeSet::from([PutForm::Encoded(0)]))
    );
}

impl PartialOrd for PutForm {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PutForm {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |form: &Self| match form {
            Self::Encoded(len) => (0, *len),
            Self::Value => (1, 0),
            Self::Remove => (2, 0),
        };
        rank(self).cmp(&rank(other))
    }
}

#[test]
fn b08_a_broken_store_in_a_pass_resolves_nothing_and_calls_no_more() {
    const REFUSALS: &str = "topgun_write_behind_flush_refusals_total";
    const BROKEN: &str = "topgun_write_behind_store_broken_total{cause=\"corrupted\"}";

    // On its own thread with a wall bound: a flush task that went on running
    // passes after the store broke would hold the current-thread runtime and
    // the test with it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let observed = metrics::with_local_recorder(&recorder, || {
            paused_runtime().block_on(async {
                let dir = tempfile::tempdir().expect("tempdir");
                let node = Node::open(dir.path());
                let wb = node
                    .write_behind(WriteBehindConfig {
                        flush_interval_ms: 100,
                        backoff_base_ms: 500,
                        backoff_cap_ms: 5_000,
                        ..config()
                    })
                    .await;
                let keys = keys("k", 4);
                fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
                node.engine
                    .refuse_every_write(Some(Refusal::Broken(StoreBrokenCause::Corrupted)));

                let mark = node.engine.mark();
                wb.soft_flush().await.expect("soft_flush");
                poll_until(5_000, || node.engine.mark() > mark && !wb.test_pass_open()).await;
                let at_broken = (
                    record_counts(&node.engine.writes_since(mark)),
                    queued(&wb, &keys),
                );

                // Ten seconds with every entry due and the flush task woken
                // every 100 ms.
                let start = tokio::time::Instant::now();
                while start.elapsed() < Duration::from_secs(10) {
                    wb.soft_flush().await.expect("soft_flush");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let rendered = handle.render();
                let failed = wb
                    .background_failure()
                    .borrow()
                    .as_ref()
                    .map(super::BackgroundFailure::store_broken);
                (
                    at_broken,
                    node.engine.writes_since(mark).len(),
                    queued(&wb, &keys),
                    metric(&rendered, REFUSALS),
                    metric(&rendered, BROKEN),
                    failed,
                )
            })
        });
        let _ = tx.send(observed);
    });
    let observed = rx
        .recv_timeout(Duration::from_secs(120))
        .expect("the scenario ended within its wall bound");
    let untouched = vec![true; 4];
    assert_eq!(
        observed,
        (
            (vec![4], untouched.clone()),
            1,
            untouched,
            Some(1.0),
            Some(1.0),
            Some(true),
        )
    );
}

#[tokio::test]
async fn b09_the_shutdown_drain_commits_in_passes_with_rows_and_stops_at_its_deadline() {
    let all = keys("k", 1_200);
    let (first_pass, second_pass) = all.split_at(1_000);
    let in_pass = |write: &EngineWrite, pass: &[String]| {
        let keys = write.user_keys();
        !keys.is_empty() && keys.iter().all(|k| pass.contains(k))
    };

    // A clean stop with nothing refused: two drain passes, each one durable
    // write with its rows, and a restart replays nothing.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config()).await;
        fill(&wb, &all, 1, |i| lww(&format!("v{i}"), 1)).await;
        let mark = node.engine.mark();
        stop(wb).await;
        let writes = node.engine.writes_since(mark);
        let shape = (
            record_counts(&writes),
            writes
                .iter()
                .all(|w| w.ok && w.sync && !w.rows().is_empty()),
        );
        node.close().await;
        let node = Node::open(dir.path());
        let replayed = node.unapplied_keys().await;
        let stored = node.stored(&all).await.len();
        assert_eq!(
            (shape, replayed, stored),
            ((vec![1000, 200], true), BTreeMap::new(), 1_200),
            "clean stop"
        );
        node.close().await;
    }

    // A store that refuses every drain commit: the drain commits its first
    // pass again after each hold length (500 ms, then 1 s at the cap) until
    // its 3 s deadline, so at most `1 + ceil(log2(K / B)) + ceil(T / K)` = 5
    // commits, all of the first pass; nothing is stored and every entry
    // replays at the next start.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node
            .write_behind(WriteBehindConfig {
                backoff_base_ms: 500,
                backoff_cap_ms: 1_000,
                shutdown_timeout_ms: 3_000,
                ..config()
            })
            .await;
        fill(&wb, &all, 1, |i| lww(&format!("v{i}"), 1)).await;
        node.engine.refuse_every_write(Some(Refusal::Unknown));
        let mark = node.engine.mark();
        stop(wb).await;
        let writes = node.engine.writes_since(mark);
        let first_commits = writes.iter().filter(|w| in_pass(w, first_pass)).count();
        let second_commits = writes.iter().filter(|w| in_pass(w, second_pass)).count();
        let mixed = writes.len() - first_commits - second_commits;
        node.close().await;
        let node = Node::open(dir.path());
        let replayed: BTreeSet<String> = node
            .unapplied_keys()
            .await
            .into_values()
            .flatten()
            .collect();
        let stored = node.stored(&all).await.len();
        assert_eq!(
            (
                (2..=5).contains(&first_commits),
                second_commits,
                mixed,
                writes.iter().any(|w| w.ok),
                stored,
                replayed.len(),
            ),
            (true, 0, 0, false, 0, 1_200),
            "a refusing store: {first_commits} commits of the first pass, record counts {:?}",
            record_counts(&writes)
        );
        node.close().await;
    }

    // A deadline that passes during the first drain pass's commit: that
    // commit completes, no second pass starts, and its entries replay.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node
            .write_behind(WriteBehindConfig {
                shutdown_timeout_ms: 300,
                ..config()
            })
            .await;
        fill(&wb, &all, 1, |i| lww(&format!("v{i}"), 1)).await;
        let mark = node.engine.mark();
        node.engine.delay_write_in(1, Duration::from_millis(600));
        stop(wb).await;
        let counts = record_counts(&node.engine.writes_since(mark));
        node.close().await;
        let node = Node::open(dir.path());
        let replayed: BTreeSet<String> = node
            .unapplied_keys()
            .await
            .into_values()
            .flatten()
            .collect();
        let stored = node.stored(&all).await;
        let first_stored = first_pass.iter().all(|k| stored.contains(k));
        let second_stored = second_pass.iter().filter(|k| stored.contains(*k)).count();
        let second_replays = second_pass.iter().all(|k| replayed.contains(k));
        assert_eq!(
            (counts, first_stored, second_stored, second_replays),
            (vec![1000], true, 0, true),
            "deadline before the second pass"
        );
        node.close().await;
    }
}

// ---------------------------------------------------------------------------
// b10–b19
// ---------------------------------------------------------------------------

/// The first `n` keys `{prefix}{i}` of `map` whose WAL partition is
/// `partition`.
fn keys_in_partition(map: &str, prefix: &str, partition: u32, n: usize) -> Vec<String> {
    (0..)
        .map(|i| format!("{prefix}{i}"))
        .filter(|key| super::partition_for(map, key) == partition)
        .take(n)
        .collect()
}

/// The index of the first write of `writes` that names `key`'s record.
fn first_write_naming(writes: &[EngineWrite], key: &str) -> Option<usize> {
    writes
        .iter()
        .position(|w| w.user_keys().iter().any(|k| k == key))
}

/// The applied watermark the engine holds for WAL partition `partition`.
fn durable_row(node: &Node, partition: u32) -> u64 {
    node.spy
        .applied_watermarks()
        .expect("applied_watermarks")
        .map_or(0, |w| w.by_partition[partition as usize])
}

/// Writes `key` of `MAP` with `store_time`; a far-future time keeps the entry
/// out of every pass while the drain still takes it.
async fn add_at(wb: &WriteBehindDataStore, key: &str, store_time: i64) {
    wb.add(MAP, key, &lww(key, 1), store_time)
        .await
        .expect("add");
}

/// A `store_time` no pass of a test reaches.
const NOT_DUE: i64 = i64::MAX / 4;

/// What a refused commit must leave as it was: the buffered count, the flushed
/// write sequence, per WAL partition the in-memory watermark, the pending
/// sequences and the durable row, the value each key reads, and whether each
/// key's entry is still queued.
type TrackingState = (
    u64,
    u64,
    Vec<(u32, Option<u64>, Vec<u64>, u64)>,
    Vec<String>,
    Vec<bool>,
);

async fn tracking_state(
    node: &Node,
    wb: &WriteBehindDataStore,
    keys: &[String],
    partitions: &BTreeSet<u32>,
) -> TrackingState {
    let mut reads = Vec::new();
    for key in keys {
        reads.push(format!("{:?}", wb.load(MAP, key).await.expect("load")));
    }
    (
        wb.pending_operation_count(),
        wb.flushed_watermark(),
        partitions
            .iter()
            .map(|&p| {
                (
                    p,
                    wb.test_wal_watermark(p).and_then(Result::ok),
                    wb.test_pending_wal_sequences(p)
                        .into_iter()
                        .map(|(seq, _)| seq)
                        .collect(),
                    durable_row(node, p),
                )
            })
            .collect(),
        reads,
        queued(wb, keys),
    )
}

#[tokio::test(start_paused = true)]
async fn b10_a_refused_commit_leaves_staging_counts_and_both_watermarks_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;
    // Something durable first, so the watermarks a refused commit must not
    // move are not all zero.
    let earlier = keys("s", 3);
    fill(&wb, &earlier, 1, |i| lww(&format!("s{i}"), 1)).await;
    one_pass(&wb).await;

    let keys = keys("k", 6);
    fill(&wb, &keys, 10, |i| lww(&format!("v{i}"), 1)).await;
    let partitions: BTreeSet<u32> = keys
        .iter()
        .chain(&earlier)
        .map(|key| super::partition_for(MAP, key))
        .collect();
    let before = tracking_state(&node, &wb, &keys, &partitions).await;

    node.engine.refuse_every_write(Some(Refusal::Unknown));
    let mark = node.engine.mark();
    one_pass(&wb).await;
    let writes = node.engine.writes_since(mark);
    let after = tracking_state(&node, &wb, &keys, &partitions).await;
    assert_eq!(
        (
            (
                record_counts(&writes),
                writes.iter().any(|w| w.ok),
                writes.first().is_some_and(|w| !w.rows().is_empty()),
                wb.test_store_hold_active(),
            ),
            after,
        ),
        ((vec![6], false, true, true), before)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b11_after_the_store_returns_every_buffered_entry_is_attempted_within_the_formula() {
    // Real time on a multi-thread runtime: the formula is about time, and
    // every commit sleeps a fixed 20 ms on the thread that makes it.
    const K_MS: u64 = 100;
    const I_MS: u64 = 100;
    const N_T: usize = 2_500;
    const M: usize = 1_000;

    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node
        .write_behind(WriteBehindConfig {
            flush_interval_ms: I_MS,
            backoff_base_ms: 50,
            backoff_cap_ms: K_MS,
            ..config()
        })
        .await;
    // Refusing from before the first write: a pass the interval starts
    // during the fill stores nothing either.
    node.engine.refuse_every_write(Some(Refusal::Unknown));
    node.engine
        .delay_every_write(Some(Duration::from_millis(20)));
    let keys = keys("k", N_T);
    let mark = node.engine.mark();
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    wb.soft_flush().await.expect("soft_flush");
    let held = poll_until(5_000, || wb.test_store_hold_active())
        .await
        .is_some();
    // A few refused retry passes, so the hold is past its first length.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let returned = tokio::time::Instant::now();
    node.engine.refuse_every_write(None);
    let first_attempt = |writes: &[EngineWrite]| {
        let mut first = BTreeMap::new();
        for write in writes.iter().filter(|w| w.ended >= returned) {
            for key in write.user_keys() {
                first.entry(key).or_insert(write.began);
            }
        }
        first
    };
    while first_attempt(&node.engine.writes_since(mark)).len() < N_T
        && returned.elapsed() < Duration::from_secs(10)
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let writes = node.engine.writes_since(mark);
    let first = first_attempt(&writes);
    let c_max = writes
        .iter()
        .filter(|w| w.ended >= returned)
        .map(|w| w.ended.saturating_duration_since(w.began))
        .max()
        .unwrap_or_default();
    // A pass is one commit, so `C_pass` is the longest commit.
    let c_pass = c_max;
    let passes = u32::try_from(N_T.div_ceil(M)).expect("small");
    let bound = Duration::from_millis(K_MS + I_MS) + c_pass * passes;
    let latest = first
        .values()
        .map(|began| began.saturating_duration_since(returned))
        .max()
        .unwrap_or_default();

    node.engine.delay_every_write(None);
    let all_attempted = first.len() == N_T;
    if all_attempted {
        stop(wb).await;
    } else {
        // Leaves whatever pass is still running nothing slow to do.
        node.engine.refuse_every_write(Some(Refusal::Unknown));
    }
    assert_eq!(
        (held, first.len(), latest <= bound),
        (true, N_T, true),
        "latest first attempt {latest:?} after the store returned, bound {bound:?} \
         (C_max {c_max:?})"
    );
}

#[tokio::test(start_paused = true)]
async fn b12_no_pass_commit_is_non_durable() {
    // A hold and the accepted pass after it.
    let retry_writes = {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node
            .write_behind(WriteBehindConfig {
                flush_interval_ms: 100,
                backoff_base_ms: 500,
                backoff_cap_ms: 5_000,
                ..config()
            })
            .await;
        let keys = keys("p", 4);
        fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
        node.engine.refuse_every_write(Some(Refusal::Unknown));
        let mark = node.engine.mark();
        wb.soft_flush().await.expect("soft_flush");
        poll_until(400, || wb.test_store_hold_active() && !wb.test_pass_open()).await;
        node.engine.refuse_every_write(None);
        poll_until(2_000, || {
            !wb.test_store_hold_active()
                && !wb.test_pass_open()
                && keys.iter().all(|k| wb.test_buffered_at(MAP, k).is_none())
        })
        .await;
        let writes = node.engine.writes_since(mark);
        stop(wb).await;
        writes
    };

    // A refused pass, then a drain whose first commit is refused and whose
    // retry after the hold is accepted.
    let drain_writes = {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node
            .write_behind(WriteBehindConfig {
                backoff_base_ms: 500,
                backoff_cap_ms: 5_000,
                ..config()
            })
            .await;
        let keys = keys("k", 8);
        fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
        node.engine.refuse_write_in(1, Refusal::Unknown);
        let mark = node.engine.mark();
        one_pass(&wb).await;
        fill(&wb, &keys_in_drain(), 100, |i| lww(&format!("d{i}"), 1)).await;
        node.engine.refuse_write_in(1, Refusal::Unknown);
        stop(wb).await;
        node.engine.writes_since(mark)
    };

    assert_eq!(
        (
            record_counts(&retry_writes),
            retry_writes.iter().all(|w| w.sync),
            record_counts(&drain_writes),
            drain_writes.iter().all(|w| w.sync),
        ),
        (vec![4, 4], true, vec![8, 11, 11], true)
    );
}

/// The keys b12 adds just before its stop, for the drain.
fn keys_in_drain() -> Vec<String> {
    keys("d", 3)
}

#[tokio::test(start_paused = true)]
async fn b18_a_new_healthy_key_waits_at_most_its_queue_position_in_passes() {
    // One due entry `R` whose pass the store refused, and the queue ahead of
    // the healthy key `H`; once the hold is over the store accepts, and `R`,
    // with the earliest store time, is ahead of both.
    async fn scenario(
        config: WriteBehindConfig,
        ahead: usize,
        value: impl Fn(usize) -> RecordValue,
    ) -> (Vec<usize>, Option<usize>, bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config).await;
        let refused = keys("r", 1);
        node.engine
            .refuse_writes_holding(&refused[0], Refusal::Unknown);
        fill(&wb, &refused, 1, &value).await;
        one_pass(&wb).await;
        node.engine.allow_writes_holding(&refused[0]);
        tokio::time::sleep(Duration::from_secs(61)).await;

        let ahead_keys = keys("k", ahead);
        fill(&wb, &ahead_keys, 100, &value).await;
        let healthy = keys("h", 1);
        fill(
            &wb,
            &healthy,
            100 + i64::try_from(ahead).expect("small"),
            &value,
        )
        .await;
        let mark = node.engine.mark();
        flush_until_idle(&wb).await;
        let writes = node.engine.writes_since(mark);
        let stored = node.stored(&healthy).await.len() == 1;
        (
            record_counts(&writes),
            first_write_naming(&writes, &healthy[0]),
            stored,
        )
    }

    let config = || WriteBehindConfig {
        backoff_base_ms: 60_000,
        backoff_cap_ms: 60_000,
        ..config()
    };
    // `Q_H = 2 501` small values ahead (`R` and 2 500), `m = 1000`: by pass
    // `ceil((Q_H + 1) / m)`, the third.
    let small = scenario(config(), 2_500, |i| lww(&format!("v{i}"), 1)).await;
    // Values just under 4 MiB encoded, two to a pass (`m = 2`), `Q_H = 4`:
    // by the third pass.
    let big = scenario(config(), 3, |_| lww(&"x".repeat(4 * MIB - 1024), 1)).await;
    assert_eq!(
        (small, (big.0, big.1.is_some_and(|i| i <= 2), big.2)),
        (
            (vec![1000, 1000, 502], Some(2), true),
            (vec![2, 2, 1], true, true)
        )
    );
}

#[tokio::test(start_paused = true)]
async fn b19_a_run_of_passes_yields_and_stops_at_the_shutdown_signal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;
    let keys = keys("k", 4_000);
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    let mark = node.engine.mark();

    // One wake starts a run of four passes. Another task on this
    // current-thread runtime watches the engine and, once two passes have
    // committed, signals shutdown; a bound on its turns ends it if it is
    // never let in.
    wb.soft_flush().await.expect("soft_flush");
    let observer = {
        let wb = Arc::clone(&wb);
        let engine = Arc::clone(&node.engine);
        tokio::spawn(async move {
            let mut seen = BTreeSet::new();
            for _ in 0..10_000_000u64 {
                let committed = engine.mark() - mark;
                seen.insert(committed);
                if committed >= 2 {
                    let _ = wb.shutdown.send(true);
                    break;
                }
                tokio::task::yield_now().await;
            }
            seen
        })
    };
    let seen = observer.await.expect("observer");
    for _ in 0..1_000 {
        tokio::task::yield_now().await;
    }
    let counts = record_counts(&node.engine.writes_since(mark));
    let finished = wb
        .flush_loop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(tokio::task::JoinHandle::is_finished);
    stop(wb).await;
    let stored = node.stored(&keys).await.len();
    assert_eq!(
        (
            seen.contains(&1) && seen.contains(&2),
            counts,
            finished,
            stored
        ),
        (true, vec![1000, 1000], Some(true), 4_000),
        "the observer saw {seen:?} committed writes"
    );
}

/// The WAL sequences of `key`'s frames in `partition`, oldest first.
async fn wal_seqs(node: &Node, partition: u32, key: &str) -> Vec<u64> {
    node.wal
        .unapplied(partition)
        .await
        .expect("unapplied")
        .into_iter()
        .filter(|frame| frame.key == key)
        .map(|frame| frame.sequence)
        .collect()
}

/// `(seq, ok)` of every committed write of `key`, in commit order.
fn committed_versions(node: &Node, key: &str) -> Vec<(u64, bool)> {
    node.spy
        .versions
        .lock()
        .iter()
        .filter(|(k, ..)| k == key)
        .map(|(_, seq, ok)| (*seq, *ok))
        .collect()
}

/// An OR write of `key` adding `tag`, as the OR write path makes it: the
/// key's current value with the delta applied, the delta as its witness.
async fn or_add(wb: &WriteBehindDataStore, key: &str, tag: &str, store_time: i64) {
    let delta = OrDelta::Add {
        entry: OrMapEntry {
            value: Value::String(tag.to_string()),
            tag: tag.to_string(),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n".to_string(),
            },
            ttl_ms: None,
        },
    };
    let mut value = wb
        .load(MAP, key)
        .await
        .expect("load")
        .unwrap_or(RecordValue::OrMap {
            records: Vec::new(),
            tombstones: Vec::new(),
        });
    crate::service::domain::crdt::normalize_to_or_map(&mut value);
    crate::service::domain::crdt::apply_or_delta(delta.clone(), &mut value);
    wb.add_with_witness(
        MAP,
        key,
        WriteSource::Value(&value),
        store_time,
        Some(&delta),
    )
    .await
    .expect("OR write");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b20_a_key_commits_in_wal_order_across_passes_and_a_coalesced_survivor_stores_its_own_wal_sequence(
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    // The WAL counter starts far above the entry counter (from 0), so a
    // version taken from an entry's own sequence differs from its WAL
    // sequence. A write delay of 1 ms leaves every small store time due and
    // `NOT_DUE` out of every pass.
    let wb = WriteBehindDataStore::new_with_wal(
        Arc::clone(&node.spy) as Arc<dyn MapDataStore>,
        WriteBehindConfig {
            write_delay_ms: 1,
            ..config()
        },
        Some(WalBootstrap {
            wal: Arc::clone(&node.wal) as Arc<dyn Wal>,
            sequence_start: 1001,
        }),
    );
    let partition = 7;
    let keys = keys_in_partition(MAP, "k", partition, 3);
    let (pin, k, c) = (keys[0].as_str(), keys[1].as_str(), keys[2].as_str());

    // A write no pass takes holds the partition's watermark below every
    // later write, so each of them stores its version row (`TG-WAL-015`).
    add_at(&wb, pin, NOT_DUE).await;
    one_pass(&wb).await;
    assert!(
        wb.test_buffered_at(MAP, pin).is_some(),
        "the pin stays queued"
    );

    // A pass holds K's entry inside its commit while a newer write of K is
    // queued behind it.
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *node.spy.park.lock() = Some((k.to_string(), Arc::clone(&reached), Arc::clone(&release)));
    wb.add(MAP, k, &lww("v1", 1), 2).await.expect("add v1");
    wb.soft_flush().await.expect("soft_flush");
    reached.notified().await;
    wb.add(MAP, k, &lww("v2", 2), 3).await.expect("add v2");
    release.notify_one();
    flush_until_gone(&wb, &[k.to_string()]).await;

    let k_seqs = wal_seqs(&node, partition, k).await;
    assert_eq!(k_seqs.len(), 2, "two frames of K: {k_seqs:?}");
    assert!(k_seqs[0] > 1000, "WAL sequences start at 1001: {k_seqs:?}");
    assert_eq!(
        committed_versions(&node, k),
        vec![(k_seqs[0], true), (k_seqs[1], true)],
        "K's entries are committed in WAL-sequence order, each at its WAL sequence"
    );
    assert_eq!(
        node.spy
            .load_version(MAP, k)
            .await
            .expect("load_version")
            .map(|v| v.seq),
        Some(k_seqs[1])
    );
    assert_eq!(
        node.spy.load(MAP, k).await.expect("load"),
        Some(lww("v2", 2))
    );

    // Two OR writes of C coalesce into one entry; the second carries the
    // first's sequence.
    or_add(&wb, c, "a", 4).await;
    or_add(&wb, c, "b", 5).await;
    let frames: Vec<_> = node
        .wal
        .unapplied(partition)
        .await
        .expect("unapplied")
        .into_iter()
        .filter(|frame| frame.key == c)
        .collect();
    assert!(
        frames.iter().all(|frame| !frame.op.subsumes_on_coalesce()),
        "both writes are framed as deltas, so the survivor carries the first"
    );
    let c_seqs: Vec<u64> = frames.iter().map(|frame| frame.sequence).collect();
    assert_eq!(c_seqs.len(), 2);
    flush_until_gone(&wb, &[c.to_string()]).await;
    assert_eq!(
        committed_versions(&node, c),
        vec![(c_seqs[1], true)],
        "one commit of the survivor, at its own WAL sequence"
    );
    assert_eq!(
        node.spy
            .load_version(MAP, c)
            .await
            .expect("load_version")
            .map(|v| v.seq),
        Some(c_seqs[1])
    );
    let stored = node.spy.load(MAP, c).await.expect("load").expect("C");
    let RecordValue::OrMap { records, .. } = &stored else {
        panic!("C is an OR slot: {stored:?}");
    };
    let tags: BTreeSet<&str> = records.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(
        tags,
        BTreeSet::from(["a", "b"]),
        "the survivor's value holds the carried write's effect"
    );

    stop(wb).await;
}
