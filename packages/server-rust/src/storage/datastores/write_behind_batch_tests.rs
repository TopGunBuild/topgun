//! Main tests of the batched flush pass, the halving of a refused commit and the batched
//! shutdown drain.
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
    AppliedWatermarks, FlushCommit, LeafSink, Loaded, MapDataStore, PutValue, ScanBatch,
    ScanCursor, ScanOrder, StoreBrokenCause, StoreErrorClass, StoreWrite, WriteSource,
};
use crate::storage::record::{Record, RecordMetadata, RecordValue};
use crate::storage::wal::{Wal, WalFsyncPolicy, WalWriter};

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
    /// Index in `log` of the one write that is applied whatever else is
    /// armed.
    accept_at: Mutex<Option<usize>>,
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
            accept_at: Mutex::new(None),
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

    /// The `n`-th write from now (`n >= 1`) is applied, once, even while
    /// every write or a key it names is refused.
    fn accept_write_in(&self, n: usize) {
        assert!(n >= 1, "accept_write_in takes n >= 1");
        *self.accept_at.lock() = Some(self.mark() + n - 1);
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
        let accepted = *self.accept_at.lock() == Some(index);
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
            })
            .filter(|_| !accepted);
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
    ok: bool,
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
}

impl CommitSpy {
    fn mark(&self) -> usize {
        self.commits.lock().len()
    }

    fn commits_since(&self, mark: usize) -> Vec<CommitCall> {
        self.commits.lock()[mark..].to_vec()
    }
}

#[async_trait]
impl MapDataStore for CommitSpy {
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

    fn accepts_encoded(&self) -> bool {
        self.inner.accepts_encoded()
    }

    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        self.inner
            .add_encoded(map, key, bytes, expiration_time, now)
            .await
    }

    fn scan_order(&self) -> Option<ScanOrder> {
        self.inner.scan_order()
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
        self.commits.lock().push(CommitCall {
            writes,
            ok: result.is_ok(),
        });
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
        });
        let wal = WalWriter::open_over_store(
            dir.join("wal"),
            WalFsyncPolicy::None,
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
        )
        .expect("WAL open");
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
/// a refused entry is not attempted again within a test.
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
        wb.add(MAP, key, &value(i), 0, store_time)
            .await
            .expect("add");
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
            .any(|key| wb.test_retry_state(MAP, key).is_some())
    {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the entries were not stored"
        );
        wb.soft_flush().await.expect("soft_flush");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// `(retry_count, next_attempt is set, first_refused is set)` of the queued
/// entry of each key; `None` for a key with no queued entry.
fn retry_states(wb: &WriteBehindDataStore, keys: &[String]) -> Vec<Option<(u32, bool, bool)>> {
    keys.iter()
        .map(|key| {
            wb.test_retry_state(MAP, key).map(|(count, next)| {
                (
                    count,
                    next.is_some(),
                    wb.test_first_refused(MAP, key).is_some(),
                )
            })
        })
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

#[tokio::test(start_paused = true)]
async fn b04_one_refused_record_is_isolated_with_at_most_2_log2_n_extra_commits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;
    let keys = keys("k", 1_000);
    let refused = keys[600].clone();
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    node.engine
        .refuse_writes_holding(&refused, Refusal::Unknown);

    let mark = node.engine.mark();
    one_pass(&wb).await;
    let writes = node.engine.writes_since(mark);
    eprintln!(
        "b04: {} engine writes for one refused record among 1000",
        writes.len()
    );
    let committed: usize = writes
        .iter()
        .filter(|w| w.ok)
        .map(EngineWrite::records)
        .sum();
    let healthy: Vec<String> = keys.iter().filter(|k| **k != refused).cloned().collect();
    let stored = node.stored(&healthy).await.len();
    let refused_state = retry_states(&wb, std::slice::from_ref(&refused));
    assert_eq!(
        (
            writes.len() <= 21,
            committed,
            stored,
            refused_state,
            wb.test_store_hold_active()
        ),
        (true, 999, 999, vec![Some((1, true, true))], false),
        "{} engine writes, record counts {:?}",
        writes.len(),
        record_counts(&writes)
    );
}

#[test]
fn b05_a_store_that_refuses_every_commit_starts_a_hold_after_three_calls() {
    use tracing_subscriber::layer::SubscriberExt;

    const HOLD: &str = "topgun_write_behind_store_hold";
    const RETRYING: &str = "topgun_write_behind_retrying_entries";
    const REFUSALS: &str = "topgun_write_behind_flush_refusals_total";

    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    let (at_hold, over_a_minute) = metrics::with_local_recorder(&recorder, || {
        tracing::subscriber::with_default(subscriber, || {
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

                // The hold's start, before its first probe (500 ms later).
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
                    retry_states(&wb, &keys),
                    warnings,
                    metric(&rendered, HOLD),
                    metric(&rendered, RETRYING),
                    metric(&rendered, REFUSALS),
                );

                // A minute from the first refusal: `8 + D / 5 s` calls at most.
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
        (at_hold, over_a_minute <= 20),
        (
            (
                vec![all.clone(), all[..2].to_vec(), all[2..].to_vec()],
                true,
                vec![Some((0, false, false)); 4],
                vec![Some("true".to_string())],
                Some(1.0),
                Some(0.0),
                Some(3.0),
            ),
            true
        ),
        "{over_a_minute} store calls in the minute from the first refusal"
    );
}

#[tokio::test(start_paused = true)]
async fn b06_refused_records_in_both_halves_start_a_hold_the_documented_false_positive() {
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
    node.engine
        .refuse_writes_holding(&keys[1], Refusal::Unknown);
    node.engine
        .refuse_writes_holding(&keys[5], Refusal::Unknown);

    let mark = node.engine.mark();
    one_pass(&wb).await;
    let writes = node.engine.writes_since(mark);
    let stored = node.stored(&keys).await.len();
    assert_eq!(
        (
            writes
                .iter()
                .map(EngineWrite::user_keys)
                .collect::<Vec<_>>(),
            writes.iter().any(|w| w.ok),
            wb.test_store_hold_active(),
            stored,
            retry_states(&wb, &keys),
        ),
        (
            vec![keys.clone(), keys[..4].to_vec(), keys[4..].to_vec()],
            false,
            true,
            0,
            vec![Some((0, false, false)); 8],
        )
    );
}

#[tokio::test(start_paused = true)]
async fn b07_record_level_refusals_are_handled_per_entry_without_halving() {
    const BAD_MAP: &str = "bad__backup";

    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;

    // A map name the store refuses, in the middle of the pass: the commit is
    // refused for that write alone, nothing written, and the rest is
    // committed again in one engine write.
    let named = keys("h", 5);
    fill(&wb, &named[..2], 1, |i| lww(&format!("v{i}"), 1)).await;
    wb.add(BAD_MAP, "r", &lww("bad", 1), 0, 3)
        .await
        .expect("add");
    fill(&wb, &named[2..], 4, |i| lww(&format!("v{i}"), 1)).await;
    let (engine_mark, spy_mark) = (node.engine.mark(), node.spy.mark());
    one_pass(&wb).await;
    let refused_name = (
        record_counts(&node.engine.writes_since(engine_mark)),
        node.spy
            .commits_since(spy_mark)
            .iter()
            .map(|c| (c.writes.len(), c.ok))
            .collect::<Vec<_>>(),
        wb.test_retry_state(BAD_MAP, "r").map(|(count, _)| count),
        node.stored(&named).await.len(),
        wb.test_store_hold_active(),
    );

    // A `Store` entry whose encoding fails: refused alone, in no commit.
    let plain = keys("e", 5);
    wb.test_refuse_encoding(MAP, &plain[2]);
    fill(&wb, &plain, 100, |i| lww(&format!("v{i}"), 1)).await;
    let (engine_mark, spy_mark) = (node.engine.mark(), node.spy.mark());
    one_pass(&wb).await;
    let commits = node.spy.commits_since(spy_mark);
    let unencodable_store = (
        record_counts(&node.engine.writes_since(engine_mark)),
        commits
            .iter()
            .map(|c| (c.writes.len(), c.ok))
            .collect::<Vec<_>>(),
        commits
            .iter()
            .any(|c| c.writes.iter().any(|(_, key, _)| *key == plain[2])),
        retry_states(&wb, std::slice::from_ref(&plain[2])),
        node.stored(&plain).await.len(),
    );

    // A `StoreCell` entry whose encoding fails, beside a cell entry that
    // encodes: the first refused alone, the second carried encoded.
    let cells = keys("c", 5);
    wb.test_refuse_encoding(MAP, &cells[2]);
    let cell_of = |text: &str| {
        crate::storage::engine::new_slot_cell(Record {
            value: lww(text, 1),
            metadata: RecordMetadata::new(0, 0),
        })
    };
    let (refused_cell, encoded_cell) = (cell_of("refused"), cell_of("encoded"));
    for (i, key) in cells.iter().enumerate() {
        let store_time = 200 + i64::try_from(i).expect("small index");
        match i {
            2 => wb
                .add_with_witness(
                    MAP,
                    key,
                    WriteSource::Cell(&refused_cell),
                    0,
                    store_time,
                    None,
                )
                .await
                .expect("add"),
            3 => wb
                .add_with_witness(
                    MAP,
                    key,
                    WriteSource::Cell(&encoded_cell),
                    0,
                    store_time,
                    None,
                )
                .await
                .expect("add"),
            _ => wb
                .add(MAP, key, &lww("v", 1), 0, store_time)
                .await
                .expect("add"),
        }
    }
    let (engine_mark, spy_mark) = (node.engine.mark(), node.spy.mark());
    one_pass(&wb).await;
    let commits = node.spy.commits_since(spy_mark);
    let unencodable_cell = (
        record_counts(&node.engine.writes_since(engine_mark)),
        commits
            .iter()
            .map(|c| (c.writes.len(), c.ok))
            .collect::<Vec<_>>(),
        commits
            .iter()
            .any(|c| c.writes.iter().any(|(_, key, _)| *key == cells[2])),
        retry_states(&wb, std::slice::from_ref(&cells[2])),
        node.stored(&cells).await.len(),
    );

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
        (
            refused_name,
            unencodable_store,
            unencodable_cell,
            by_encoded_bytes,
            forms
        ),
        (
            (vec![5], vec![(6, false), (5, true)], Some(1), 5, false),
            (
                vec![4],
                vec![(4, true)],
                false,
                vec![Some((1, true, true))],
                4
            ),
            (
                vec![4],
                vec![(4, true)],
                false,
                vec![Some((1, true, true))],
                4
            ),
            vec![1, 1],
            BTreeSet::from([PutForm::Encoded(0)]),
        )
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
                    retry_states(&wb, &keys),
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
                    retry_states(&wb, &keys),
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
    let untouched = vec![Some((0, false, false)); 4];
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

    // Two engine-refused keys in different halves of the first drain pass:
    // every healthy entry of both passes is stored, the two are abandoned and
    // replay at the next start, and each pass makes at most `2n - 1` commits.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config()).await;
        fill(&wb, &all, 1, |i| lww(&format!("v{i}"), 1)).await;
        let refused = [all[100].clone(), all[700].clone()];
        for key in &refused {
            node.engine.refuse_writes_holding(key, Refusal::Unknown);
        }
        let refused_partitions: BTreeSet<u32> = refused
            .iter()
            .map(|key| super::partition_for(MAP, key))
            .collect();
        let mark = node.engine.mark();
        stop(wb).await;
        let writes = node.engine.writes_since(mark);
        let first_commits = writes.iter().filter(|w| in_pass(w, first_pass)).count();
        let second_commits = writes.iter().filter(|w| in_pass(w, second_pass)).count();
        let mixed = writes.len() - first_commits - second_commits;
        node.close().await;
        let node = Node::open(dir.path());
        let replayed = node.unapplied_keys().await;
        let replays_refused = refused
            .iter()
            .all(|key| replayed.values().any(|keys| keys.contains(key)));
        let replays_elsewhere: Vec<u32> = replayed
            .keys()
            .copied()
            .filter(|p| !refused_partitions.contains(p))
            .collect();
        let stored = node.stored(&all).await;
        let healthy_stored = all
            .iter()
            .filter(|k| !refused.contains(k))
            .all(|k| stored.contains(k));
        assert_eq!(
            (
                first_commits <= 1_999,
                second_commits <= 399,
                mixed,
                healthy_stored,
                stored.len(),
                replays_refused,
                replays_elsewhere,
            ),
            (true, true, 0, true, 1_198, true, Vec::<u32>::new()),
            "refused keys: {first_commits} commits in the first pass, {second_commits} in the \
             second, record counts {:?}",
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
    wb.add(MAP, key, &lww(key, 1), 0, store_time)
        .await
        .expect("add");
}

/// A `store_time` no pass of a test reaches.
const NOT_DUE: i64 = i64::MAX / 4;

/// What a refused commit must leave as it was: the buffered count, the flushed
/// write sequence, per WAL partition the in-memory watermark, the pending
/// sequences and the durable row, the value each key reads, and each key's
/// retry state.
type TrackingState = (
    u64,
    u64,
    Vec<(u32, Option<u64>, Vec<u64>, u64)>,
    Vec<String>,
    Vec<Option<(u32, bool, bool)>>,
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
        retry_states(wb, keys),
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
        ((vec![6, 3, 3], false, true, true), before)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b11_after_the_store_returns_every_buffered_entry_is_attempted_within_the_formula() {
    // Real time on a multi-thread runtime: the formula is about time, and
    // every commit sleeps a fixed 20 ms on the thread that makes it.
    const K_MS: u64 = 100;
    const I_MS: u64 = 100;
    const N_T: usize = 2_500;
    const M: usize = 999;
    const LOG2_L: u32 = 10;

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
    // A few refused probes, so the hold is past its first length.
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
    // `C_pass <= (1 + 2 log2 L) C_max`, and every commit of a pass that
    // begins by the bound begins at most one `C_pass` after it.
    let c_pass = c_max * (1 + 2 * LOG2_L);
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
    // A hold and its accepted probe.
    let probe_writes = {
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
                && keys.iter().all(|k| wb.test_retry_state(MAP, k).is_none())
        })
        .await;
        let writes = node.engine.writes_since(mark);
        stop(wb).await;
        writes
    };

    // A pass halving around one refused record, then a drain doing the same.
    let halving_writes = {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config()).await;
        let keys = keys("k", 8);
        node.engine
            .refuse_writes_holding(&keys[5], Refusal::Unknown);
        fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
        let mark = node.engine.mark();
        one_pass(&wb).await;
        fill(&wb, &keys_in_drain(), 100, |i| lww(&format!("d{i}"), 1)).await;
        stop(wb).await;
        node.engine.writes_since(mark)
    };

    assert_eq!(
        (
            record_counts(&probe_writes),
            probe_writes.iter().all(|w| w.sync),
            record_counts(&halving_writes),
            halving_writes.iter().all(|w| w.sync),
        ),
        (
            vec![4, 2, 2, 1, 3],
            true,
            vec![8, 4, 4, 2, 2, 1, 1, 4, 2, 2, 1, 1],
            true
        )
    );
}

/// The keys b12 adds just before its stop, for the drain.
fn keys_in_drain() -> Vec<String> {
    keys("d", 3)
}

#[tokio::test(start_paused = true)]
async fn b13_after_an_accepted_probe_two_records_refused_in_different_halves_are_isolated_without_a_new_hold(
) {
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
    let keys = keys("k", 1_000);
    let refused = [keys[100].clone(), keys[700].clone()];
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    for key in &refused {
        node.engine.refuse_writes_holding(key, Refusal::Unknown);
    }

    // The two refused halves look like a store that is down: a hold.
    let mark = node.engine.mark();
    wb.soft_flush().await.expect("soft_flush");
    let tripped = poll_until(400, || wb.test_store_hold_active() && !wb.test_pass_open())
        .await
        .is_some();
    let trip = node.engine.writes_since(mark);

    // The probe is accepted; the pass after it is split to single entries.
    let mark = node.engine.mark();
    let others: Vec<String> = keys
        .iter()
        .filter(|k| !refused.contains(k))
        .cloned()
        .collect();
    poll_until(2_000, || {
        !wb.test_pass_open() && others.iter().all(|k| wb.test_retry_state(MAP, k).is_none())
    })
    .await;
    let after = node.engine.writes_since(mark);
    let probe_is_another_key = after.first().is_some_and(|w| {
        let keys = w.user_keys();
        w.ok && keys.len() == 1 && !refused.contains(&keys[0])
    });
    let split = after.get(1..).unwrap_or(&[]);
    let split_stored: usize = split
        .iter()
        .filter(|w| w.ok)
        .map(EngineWrite::records)
        .sum();
    let stored = node.stored(&keys).await.len();
    assert_eq!(
        (
            tripped,
            record_counts(&trip),
            trip.iter().any(|w| w.ok),
            probe_is_another_key,
            split.first().map(EngineWrite::records),
            split.len() <= 1_997,
            split_stored,
            stored,
            retry_states(&wb, &refused),
            wb.test_store_hold_active(),
        ),
        (
            true,
            vec![1000, 500, 500],
            false,
            true,
            Some(999),
            true,
            997,
            998,
            vec![Some((1, true, true)); 2],
            false,
        ),
        "{} commits in the split pass",
        split.len()
    );
}

#[tokio::test(start_paused = true)]
async fn b14_a_store_that_goes_down_right_after_an_accepted_probe_splits_once_then_holds() {
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
    // One more than a pass takes: the probe is the one the first pass left.
    let keys = keys("k", 1_001);
    fill(&wb, &keys, 1, |i| lww(&format!("v{i}"), 1)).await;
    node.engine.refuse_every_write(Some(Refusal::Unknown));

    let mark = node.engine.mark();
    wb.soft_flush().await.expect("soft_flush");
    let tripped = poll_until(400, || wb.test_store_hold_active() && !wb.test_pass_open())
        .await
        .is_some();
    let trip = node.engine.writes_since(mark);

    // The probe is accepted and the store refuses everything again at once.
    node.engine.accept_write_in(1);
    let mark = node.engine.mark();
    poll_until(5_000, || {
        wb.test_store_hold_active() && !wb.test_pass_open() && node.engine.mark() > mark + 1
    })
    .await;
    let after = node.engine.writes_since(mark);
    let probe = after.first().map(|w| (w.user_keys(), w.ok));
    // The split commit begins with the whole pass; the next commit of a whole
    // pass is the one that trips the next hold.
    let next_whole = after
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, w)| w.records() == 1000)
        .map(|(i, _)| i);
    let split = after
        .get(1..next_whole.unwrap_or(after.len()))
        .unwrap_or(&[]);
    let second_trip = next_whole.map(|i| record_counts(&after[i..]));
    let all_refused_alone = retry_states(&wb, &keys[..1000])
        .iter()
        .all(|state| *state == Some((1, true, true)));
    assert_eq!(
        (
            tripped,
            record_counts(&trip),
            probe,
            split.first().map(EngineWrite::records),
            split.len() <= 1_999,
            split.iter().any(|w| w.ok),
            second_trip,
            wb.test_store_hold_active(),
            all_refused_alone,
        ),
        (
            true,
            vec![1000, 500, 500],
            Some((vec![keys[1000].clone()], true)),
            Some(1000),
            true,
            false,
            Some(vec![1000, 500, 500]),
            true,
            true,
        ),
        "{} commits in the split, record counts after the probe {:?}",
        split.len(),
        record_counts(&after).get(..12)
    );
}

/// The keys of an engine write, its rows for two WAL partitions, and whether
/// it was applied.
type TwoPartitionWrite = (BTreeSet<String>, Option<u64>, Option<u64>, bool);

#[tokio::test(start_paused = true)]
async fn b15_a_half_committed_after_its_sibling_plans_its_rows_after_the_sibling_settled() {
    // One WAL partition: pending {5 in L, 7 in R, 9 outside the pass},
    // durable 3.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config()).await;
        let p = super::partition_for(MAP, "p0");
        let pk = keys_in_partition(MAP, "p", p, 6);
        let qk: Vec<String> = (0..)
            .map(|i| format!("q{i}"))
            .filter(|key| super::partition_for(MAP, key) != p)
            .take(3)
            .collect();
        // Sequences 1..=3, stored by a pass of their own.
        fill(&wb, &pk[..3], 1, |i| lww(&format!("v{i}"), 1)).await;
        one_pass(&wb).await;
        let durable_before = durable_row(&node, p);
        // Sequences 4..=9: 4, 6 and 8 in other partitions, none of them due.
        add_at(&wb, &qk[0], NOT_DUE).await;
        add_at(&wb, &pk[3], 10).await;
        add_at(&wb, &qk[1], NOT_DUE).await;
        add_at(&wb, &pk[4], 11).await;
        add_at(&wb, &qk[2], NOT_DUE).await;
        add_at(&wb, &pk[5], NOT_DUE).await;
        let pending: Vec<u64> = wb
            .test_pending_wal_sequences(p)
            .into_iter()
            .map(|(seq, _)| seq)
            .collect();

        node.engine.refuse_write_in(1, Refusal::Unknown);
        let mark = node.engine.mark();
        one_pass(&wb).await;
        let shape: Vec<(BTreeSet<String>, Option<u64>, bool)> = node
            .engine
            .writes_since(mark)
            .iter()
            .map(|w| {
                (
                    w.user_keys().into_iter().collect(),
                    w.rows().get(&p).copied(),
                    w.ok,
                )
            })
            .collect();
        let set = |keys: &[&String]| keys.iter().map(|k| (*k).clone()).collect::<BTreeSet<_>>();
        assert_eq!(
            (durable_before, pending, shape, durable_row(&node, p)),
            (
                3,
                vec![5, 7, 9],
                vec![
                    (set(&[&pk[3], &pk[4]]), Some(8), false),
                    (set(&[&pk[3]]), Some(6), true),
                    (set(&[&pk[4]]), Some(8), true),
                ],
                8
            ),
            "one partition"
        );
    }

    // Two WAL partitions: P1 pending {3 in L, 5 in R, 7 outside}, durable 1;
    // P2 pending {4 in L, 6 in R, 8 outside}, durable 2.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let node = Node::open(dir.path());
        let wb = node.write_behind(config()).await;
        let p1 = super::partition_for(MAP, "a0");
        let p2 = (0..10_000)
            .map(|i| super::partition_for(MAP, &format!("b{i}")))
            .find(|p| *p != p1)
            .expect("another partition");
        let x1 = keys_in_partition(MAP, "a", p1, 4);
        let x2 = keys_in_partition(MAP, "b", p2, 4);
        add_at(&wb, &x1[0], 1).await;
        add_at(&wb, &x2[0], 2).await;
        one_pass(&wb).await;
        let durable_before = (durable_row(&node, p1), durable_row(&node, p2));
        add_at(&wb, &x1[1], 10).await;
        add_at(&wb, &x2[1], 11).await;
        add_at(&wb, &x1[2], 12).await;
        add_at(&wb, &x2[2], 13).await;
        add_at(&wb, &x1[3], NOT_DUE).await;
        add_at(&wb, &x2[3], NOT_DUE).await;

        node.engine.refuse_write_in(1, Refusal::Unknown);
        let mark = node.engine.mark();
        one_pass(&wb).await;
        let shape: Vec<TwoPartitionWrite> = node
            .engine
            .writes_since(mark)
            .iter()
            .map(|w| {
                let rows = w.rows();
                (
                    w.user_keys().into_iter().collect(),
                    rows.get(&p1).copied(),
                    rows.get(&p2).copied(),
                    w.ok,
                )
            })
            .collect();
        let set = |keys: &[&String]| keys.iter().map(|k| (*k).clone()).collect::<BTreeSet<_>>();
        assert_eq!(
            (
                durable_before,
                shape,
                (durable_row(&node, p1), durable_row(&node, p2))
            ),
            (
                (1, 2),
                vec![
                    (
                        set(&[&x1[1], &x2[1], &x1[2], &x2[2]]),
                        Some(6),
                        Some(7),
                        false
                    ),
                    (set(&[&x1[1], &x2[1]]), Some(4), Some(5), true),
                    (set(&[&x1[2], &x2[2]]), Some(6), Some(7), true),
                ],
                (6, 7)
            ),
            "two partitions"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn b16_a_recommit_never_marks_a_refused_entry_applied() {
    const BAD_MAP: &str = "bad__backup";
    const BAD_KEY: &str = "x";

    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node.write_behind(config()).await;
    // Every entry in one WAL partition: `x` is refused by the store on its
    // own (a map name it does not take), `y` by the engine.
    let partition = super::partition_for(BAD_MAP, BAD_KEY);
    let ours = keys_in_partition(MAP, "a", partition, 4);
    let (key_a, key_b, key_y, key_c) = (&ours[0], &ours[1], &ours[2], &ours[3]);
    add_at(&wb, key_a, 1).await;
    wb.add(BAD_MAP, BAD_KEY, &lww("bad", 1), 0, 2)
        .await
        .expect("add");
    add_at(&wb, key_b, 3).await;
    add_at(&wb, key_y, 4).await;
    add_at(&wb, key_c, 5).await;
    let pending: Vec<u64> = wb
        .test_pending_wal_sequences(partition)
        .into_iter()
        .map(|(seq, _)| seq)
        .collect();
    let (seq_x, seq_y) = (2, 4);
    node.engine.refuse_writes_holding(key_y, Refusal::Unknown);

    let (engine_mark, spy_mark) = (node.engine.mark(), node.spy.mark());
    one_pass(&wb).await;
    let commits: Vec<(BTreeSet<String>, bool)> = node
        .spy
        .commits_since(spy_mark)
        .iter()
        .map(|c| (c.writes.iter().map(|(_, k, _)| k.clone()).collect(), c.ok))
        .collect();
    stop(wb).await;
    let rows: Vec<u64> = node
        .engine
        .writes_since(engine_mark)
        .iter()
        .filter_map(|w| w.rows().get(&partition).copied())
        .collect();
    node.close().await;

    let node = Node::open(dir.path());
    let replayed = node
        .unapplied_keys()
        .await
        .remove(&partition)
        .unwrap_or_default();
    let stored = node.stored(&ours).await;
    let set = |keys: &[&str]| {
        keys.iter()
            .map(|k| (*k).to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        (
            pending,
            commits,
            rows.iter().all(|row| *row < seq_x.min(seq_y)),
            replayed.contains(BAD_KEY) && replayed.contains(key_y),
            stored,
        ),
        (
            vec![1, 2, 3, 4, 5],
            vec![
                (set(&[key_a, BAD_KEY, key_b, key_y, key_c]), false),
                (set(&[key_a, key_b, key_y, key_c]), false),
                (set(&[key_a, key_b]), true),
                (set(&[key_y, key_c]), false),
                (set(&[key_y]), false),
                (set(&[key_c]), true),
            ],
            true,
            true,
            set(&[key_a, key_b, key_c]),
        ),
        "rows of partition {partition}: {rows:?}"
    );
    node.close().await;
}

#[tokio::test(start_paused = true)]
async fn b17_a_refused_entry_is_attempted_under_sustained_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Node::open(dir.path());
    let wb = node
        .write_behind(WriteBehindConfig {
            backoff_base_ms: 60_000,
            backoff_cap_ms: 60_000,
            ..config()
        })
        .await;
    // Three entries refused once each, in order, so their retry times are in
    // that order: the first has `R_T = 0`, the third `R_T = 2`.
    let refused = keys("r", 3);
    for (i, key) in refused.iter().enumerate() {
        node.engine.refuse_writes_holding(key, Refusal::Unknown);
        add_at(&wb, key, 1 + i64::try_from(i).expect("small")).await;
        one_pass(&wb).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let setup = retry_states(&wb, &refused);
    for key in &refused {
        node.engine.allow_writes_holding(key);
    }
    tokio::time::sleep(Duration::from_secs(61)).await;

    // 30 000 never-refused due entries: more than a pass takes, in each of
    // 30 passes back to back.
    let load = keys("k", 30_000);
    fill(&wb, &load, 100, |i| lww(&format!("v{i}"), 1)).await;
    let mark = node.engine.mark();
    flush_until_idle(&wb).await;
    let writes = node.engine.writes_since(mark);
    let counts = record_counts(&writes);
    let sustained = counts.len() >= 30 && counts[..30].iter().all(|c| *c == 1000);
    let first = first_write_naming(&writes, &refused[0]);
    let third = first_write_naming(&writes, &refused[2]);
    let stored = node.stored(&refused).await.len();
    assert_eq!(
        (
            setup,
            sustained,
            first,
            third.is_some_and(|i| i <= 2),
            stored
        ),
        (vec![Some((1, true, true)); 3], true, Some(0), true, 3),
        "{} writes, the first record counts {:?}, the third refused entry in write {third:?}",
        counts.len(),
        counts.get(..5)
    );
}

#[tokio::test(start_paused = true)]
async fn b18_a_new_healthy_key_waits_at_most_its_queue_position_in_passes() {
    // One due refused entry `R` and the queue ahead of the healthy key `H`;
    // `R` takes the reserved slot and is accepted when attempted.
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
    // `Q_H = 2 500` small values ahead, `m = 999`: by the third pass.
    let small = scenario(config(), 2_500, |i| lww(&format!("v{i}"), 1)).await;
    // Values just under 4 MiB encoded, `m = 1`: by pass `Q_H + 1` for
    // `Q_H = 3`.
    let big = scenario(config(), 3, |_| lww(&"x".repeat(4 * MIB - 1024), 1)).await;
    assert_eq!(
        (small, (big.0, big.1.is_some_and(|i| i <= 3), big.2)),
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
