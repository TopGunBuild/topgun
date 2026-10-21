//! Tests of the store mode of the WAL: the applied watermark kept by the store, committed with the data.
//!
//! Every test runs over a file-backed [`EngineMapDataStore`] whose engine is a
//! [`CountingEngine`]: a [`KvEngine`] over [`RedbKvEngine`] that records every
//! batch it is given, so a test can say how many engine writes a step made and
//! what each held, and that can refuse a write or stop the process inside one.
//! Crash points are reached in a child process that re-runs the test binary
//! with one test selected and stops by `std::process::abort`, the in-process
//! model of `kill -9`.

// Each test is one scenario read top to bottom — set up, crash or restart,
// check — and splitting one across helpers would hide the order its claim
// depends on.
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_core::{fnv1a_hash, PARTITION_COUNT};

use super::segment::parse_segment_filename;
use super::{
    decode_wal_collected, TestAbortPoint, Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery,
    WalStorePayload, WalWriter, TEST_ABORT_AT_ENV, WAL_COLLECTED_FILE, WAL_GC_SKIPPED_COUNTER,
};
use crate::storage::datastores::engine_store::{
    decode_map_by_name_row, map_by_name_key, meta_key, EngineMapDataStore, RecordKey, RecordSpace,
    KS_BACKUP, KS_PARTITION_META, KS_PRIMARY, META_WAL_IMPORT_PENDING, META_WAL_WATERMARK,
};
use crate::storage::datastores::kv_engine::{
    BatchOp, KeyRange, KvCapabilities, KvEngine, KvError, KvResult, KvSnapshot, WriteBatch,
    WriteOptions,
};
use crate::storage::datastores::{
    RedbDataStore, RedbKvEngine, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use crate::storage::map_data_store::{FlushCommit, MapDataStore, PutValue, StoreWrite};
use crate::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// The counting engine
// ---------------------------------------------------------------------------

/// Where, relative to delegating, an armed write stops the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AbortSide {
    /// Before the batch reaches the engine: nothing of it is applied.
    Before,
    /// After the engine returned `Ok`: the batch is applied (durable when
    /// written with `sync`), and the caller never sees the return.
    After,
}

/// One `write` call the counting engine was given, in call order.
#[derive(Debug, Clone)]
pub(super) struct WriteRecord {
    /// The batch's operations.
    pub(super) ops: Vec<BatchOp>,
    /// The `sync` flag it was written with.
    pub(super) sync: bool,
    /// Whether the write returned `Ok`.
    pub(super) ok: bool,
}

impl WriteRecord {
    fn keys(&self) -> impl Iterator<Item = &[u8]> {
        self.ops.iter().map(|op| match op {
            BatchOp::Put { key, .. } | BatchOp::Delete { key } => key.as_slice(),
            BatchOp::DeleteRange { start, .. } => start.as_slice(),
        })
    }

    /// Whether the batch puts or deletes `key`.
    pub(super) fn touches(&self, key: &[u8]) -> bool {
        self.keys().any(|k| k == key)
    }

    /// The record keys (`0x10` / `0x11`) the batch puts or deletes.
    pub(super) fn record_keys(&self) -> Vec<Vec<u8>> {
        self.keys()
            .filter(|k| {
                k.first()
                    .is_some_and(|b| *b == KS_PRIMARY || *b == KS_BACKUP)
            })
            .map(<[u8]>::to_vec)
            .collect()
    }

    /// The applied watermark rows the batch puts, decoded as
    /// `(wal_partition, sequence)`.
    pub(super) fn rows(&self) -> Vec<(u32, u64)> {
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

    /// Every key the batch names that is neither a record nor a `0x20` row.
    pub(super) fn other_keys(&self) -> Vec<Vec<u8>> {
        self.keys()
            .filter(|k| {
                !k.first()
                    .is_some_and(|b| *b == KS_PRIMARY || *b == KS_BACKUP || *b == KS_PARTITION_META)
            })
            .map(<[u8]>::to_vec)
            .collect()
    }
}

/// A [`KvEngine`] over [`RedbKvEngine`] that logs every `write`, and can be
/// armed to refuse a write, refuse a snapshot, sleep inside every write, or
/// stop the process inside a write.
pub(super) struct CountingEngine {
    inner: RedbKvEngine,
    log: Mutex<Vec<WriteRecord>>,
    /// Index in `log` of the one write that is refused.
    fail_at: Mutex<Option<usize>>,
    /// Every write whose batch names this key is refused while it is set.
    fail_holding: Mutex<Option<Vec<u8>>>,
    snapshots: AtomicU64,
    /// 1-based number of the one `snapshot` call that is refused.
    fail_snapshot_at: Mutex<Option<u64>>,
    /// Index in `log` of the write that stops the process, and on which side.
    abort_at: Mutex<Option<(usize, AbortSide)>>,
    /// The first write that puts a `0x20` row stops the process, on this side.
    abort_at_row: Mutex<Option<AbortSide>>,
    write_delay: Mutex<Duration>,
}

impl CountingEngine {
    /// Opens the engine file at `path`, creating it when absent.
    pub(super) fn open(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            inner: RedbKvEngine::open(path).expect("the engine file opens"),
            log: Mutex::new(Vec::new()),
            fail_at: Mutex::new(None),
            fail_holding: Mutex::new(None),
            snapshots: AtomicU64::new(0),
            fail_snapshot_at: Mutex::new(None),
            abort_at: Mutex::new(None),
            abort_at_row: Mutex::new(None),
            write_delay: Mutex::new(Duration::ZERO),
        })
    }

    /// The number of writes logged so far; pass it to
    /// [`CountingEngine::writes_since`].
    pub(super) fn mark(&self) -> usize {
        self.log.lock().len()
    }

    /// Every write logged from `mark` on, refused ones included.
    pub(super) fn writes_since(&self, mark: usize) -> Vec<WriteRecord> {
        self.log.lock()[mark..].to_vec()
    }

    /// The writes from `mark` on that returned `Ok`.
    pub(super) fn committed_since(&self, mark: usize) -> Vec<WriteRecord> {
        self.writes_since(mark)
            .into_iter()
            .filter(|w| w.ok)
            .collect()
    }

    /// The writes from `mark` on that were refused.
    pub(super) fn refused_since(&self, mark: usize) -> usize {
        self.log.lock()[mark..].iter().filter(|w| !w.ok).count()
    }

    /// The `n`-th write from now (`n >= 1`) is refused, once, with nothing
    /// applied.
    // Armed by the tests of refused ticks and slow commits.
    #[allow(dead_code)]
    pub(super) fn fail_write_in(&self, n: usize) {
        assert!(n >= 1, "fail_write_in takes n >= 1");
        *self.fail_at.lock() = Some(self.mark() + n - 1);
    }

    /// While `Some`, every write whose batch names the key is refused.
    pub(super) fn fail_writes_holding(&self, key: Option<Vec<u8>>) {
        *self.fail_holding.lock() = key;
    }

    /// The `n`-th `snapshot` call from now (`n >= 1`) is refused, once.
    pub(super) fn fail_snapshot_in(&self, n: u64) {
        assert!(n >= 1, "fail_snapshot_in takes n >= 1");
        *self.fail_snapshot_at.lock() = Some(self.snapshots.load(Ordering::SeqCst) + n);
    }

    /// The `n`-th write from now (`n >= 1`) stops the process on `side`.
    pub(super) fn abort_in(&self, n: usize, side: AbortSide) {
        assert!(n >= 1, "abort_in takes n >= 1");
        *self.abort_at.lock() = Some((self.mark() + n - 1, side));
    }

    /// The first write from now that puts an applied watermark row stops the
    /// process on `side`.
    pub(super) fn abort_at_first_row_write(&self, side: AbortSide) {
        *self.abort_at_row.lock() = Some(side);
    }

    /// Every later write sleeps this long before it reaches the engine.
    pub(super) fn set_write_delay(&self, delay: Duration) {
        *self.write_delay.lock() = delay;
    }

    /// The engine's content, read from one snapshot of the inner engine.
    pub(super) fn dump(&self) -> BTreeMap<Vec<u8>, Vec<u8>> {
        dump_engine(&self.inner)
    }

    /// The applied watermark row of WAL partition `partition`, if present.
    pub(super) fn row(&self, partition: u32) -> Option<u64> {
        self.inner
            .get(&row_key(partition))
            .expect("engine read")
            .map(|v| decode_u64(&v))
    }

    /// Whether the import marker is present.
    pub(super) fn marker(&self) -> Option<Vec<u8>> {
        self.inner
            .get(&meta_key(META_WAL_WATERMARK))
            .expect("engine read")
    }
}

impl KvEngine for CountingEngine {
    fn capabilities(&self) -> KvCapabilities {
        self.inner.capabilities()
    }

    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
        let delay = *self.write_delay.lock();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        // Held across the inner write, so the log's order is the order the
        // engine applied the batches in.
        let mut log = self.log.lock();
        let index = log.len();
        let puts_row = batch.ops().iter().any(
            |op| matches!(op, BatchOp::Put { key, .. } if key.first() == Some(&KS_PARTITION_META)),
        );
        let abort = (*self.abort_at.lock())
            .filter(|(at, _)| *at == index)
            .map(|(_, side)| side)
            .or_else(|| (*self.abort_at_row.lock()).filter(|_| puts_row));
        if abort == Some(AbortSide::Before) {
            std::process::abort();
        }
        let refused = *self.fail_at.lock() == Some(index)
            || self
                .fail_holding
                .lock()
                .as_ref()
                .is_some_and(|key| batch.ops().iter().any(|op| op_names(op, key)));
        let batch_ops = batch.ops().to_vec();
        let result = if refused {
            Err(KvError::Other(anyhow::anyhow!(
                "injected engine write refusal"
            )))
        } else {
            self.inner.write(batch, opts)
        };
        log.push(WriteRecord {
            ops: batch_ops,
            sync: opts.sync,
            ok: result.is_ok(),
        });
        if abort == Some(AbortSide::After) && result.is_ok() {
            std::process::abort();
        }
        result
    }

    fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
        let number = self.snapshots.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut fail_at = self.fail_snapshot_at.lock();
            if *fail_at == Some(number) {
                *fail_at = None;
                return Err(KvError::Other(anyhow::anyhow!(
                    "injected engine snapshot refusal"
                )));
            }
        }
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

fn op_names(op: &BatchOp, key: &[u8]) -> bool {
    match op {
        BatchOp::Put { key: k, .. } | BatchOp::Delete { key: k } => k == key,
        BatchOp::DeleteRange { start, end } => start.as_slice() <= key && key < end.as_slice(),
    }
}

fn dump_engine(engine: &impl KvEngine) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let snapshot = engine.snapshot().expect("engine snapshot");
    snapshot
        .iter(KeyRange::default())
        .expect("engine iter")
        .collect::<KvResult<BTreeMap<_, _>>>()
        .expect("engine entries")
}

// ---------------------------------------------------------------------------
// Keys, values, frames
// ---------------------------------------------------------------------------

/// The map every test writes.
pub(super) const MAP: &str = "m";

/// The engine key of the applied watermark row of WAL partition `partition`,
/// written out here rather than taken from the store, so the tests pin the
/// format: `0x20 ‖ partition u32 BE ‖ b"applied"`.
pub(super) fn row_key(partition: u32) -> Vec<u8> {
    let mut key = vec![KS_PARTITION_META];
    key.extend_from_slice(&partition.to_be_bytes());
    key.extend_from_slice(b"applied");
    key
}

fn decode_row_partition(key: &[u8]) -> u32 {
    assert_eq!(key.len(), 12, "a 0x20 row key is 12 bytes: {key:?}");
    u32::from_be_bytes(key[1..5].try_into().expect("4 bytes"))
}

fn decode_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().expect("an 8-byte value"))
}

/// Every applied watermark row of a dump.
pub(super) fn rows_of(dump: &BTreeMap<Vec<u8>, Vec<u8>>) -> BTreeMap<u32, u64> {
    dump.iter()
        .filter(|(k, _)| k.first() == Some(&KS_PARTITION_META))
        .map(|(k, v)| (decode_row_partition(k), decode_u64(v)))
        .collect()
}

/// The WAL partition write-behind gives `(map, key)`.
pub(super) fn wal_partition(map: &str, key: &str) -> u32 {
    fnv1a_hash(&format!("{map}:{key}")) % PARTITION_COUNT
}

/// The first `n` keys `k{i}` of `MAP` in WAL partition `partition`.
pub(super) fn keys_in(partition: u32, n: usize) -> Vec<String> {
    (0u64..)
        .map(|i| format!("k{i}"))
        .filter(|key| wal_partition(MAP, key) == partition)
        .take(n)
        .collect()
}

pub(super) fn lww(text: &str, millis: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::String(text.to_string()),
        timestamp: Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        },
    }
}

/// A `Store` frame of `MAP`, as write-behind appends one.
pub(super) fn store_frame(key: &str, value: &RecordValue, sequence: u64) -> WalEntry {
    let timestamp = match value {
        RecordValue::Lww { timestamp, .. } => Some(timestamp.clone()),
        _ => None,
    };
    WalEntry {
        map: MAP.to_string(),
        key: key.to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Record(value.clone()),
            expiration_time: None,
        },
        timestamp,
        sequence,
    }
}

/// The primary record key of `(MAP, key)` in `engine`, by the map's id.
pub(super) fn record_key(engine: &CountingEngine, key: &str) -> Vec<u8> {
    let name_key = map_by_name_key(MAP);
    let row = engine
        .get(&name_key)
        .expect("engine read")
        .expect("the map is registered");
    let entry = decode_map_by_name_row(&name_key, &row).expect("registry row");
    RecordKey::encode(RecordSpace::Primary, entry.map_id, key)
}

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

/// A data directory and a WAL directory side by side.
pub(super) struct Root {
    pub(super) path: PathBuf,
    _dir: Option<tempfile::TempDir>,
}

impl Root {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        Self {
            path: dir.path().to_path_buf(),
            _dir: Some(dir),
        }
    }

    /// A root a child process was handed; the parent owns and removes it.
    pub(super) fn at(path: PathBuf) -> Self {
        Self { path, _dir: None }
    }

    pub(super) fn data_dir(&self) -> PathBuf {
        self.path.join("topgun-data")
    }

    pub(super) fn engine_path(&self) -> PathBuf {
        self.data_dir().join("engine.redb")
    }

    pub(super) fn wal_dir(&self) -> PathBuf {
        self.path.join("wal")
    }
}

/// Opens the counting engine of `root`'s data directory.
pub(super) fn open_engine(root: &Root) -> Arc<CountingEngine> {
    std::fs::create_dir_all(root.data_dir()).expect("data dir");
    CountingEngine::open(&root.engine_path())
}

/// The engine content of `root`, read with a plain engine handle that is
/// closed again before returning.
pub(super) fn dump_at(root: &Root) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let engine = RedbKvEngine::open(&root.engine_path()).expect("the engine file opens");
    dump_engine(&engine)
}

/// Writes, into `dir`, a data directory as a binary that keeps no applied
/// watermark leaves it: a formatted store with one record, no import marker
/// and no import-pending mark.
pub(super) async fn write_pre_marker_data_dir(dir: &Path) {
    std::fs::create_dir_all(dir).expect("data dir");
    let engine = Arc::new(RedbKvEngine::open(&dir.join("engine.redb")).expect("engine"));
    let store = EngineMapDataStore::open(Arc::clone(&engine)).expect("store");
    store
        .add(MAP, "old", &lww("old", 1), 0, 0)
        .await
        .expect("a record");
    drop(store);
    assert_eq!(
        Arc::strong_count(&engine),
        1,
        "the store released the engine"
    );
}

/// Replaces `root`'s data directory by a copy of `from`.
pub(super) fn put_data_dir(root: &Root, from: &Path) {
    let to = root.data_dir();
    if to.exists() {
        std::fs::remove_dir_all(&to).expect("remove the data dir");
    }
    copy_dir(from, &to);
}

/// Copies the regular files of `from` into a new directory `to`.
pub(super) fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        if entry.file_type().expect("file type").is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).expect("copy file");
        }
    }
}

/// The files of `dir` by name, each with the sha256 of its bytes: a change of
/// any byte, a new file or a removed one changes the map.
pub(super) fn fingerprint(dir: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let bytes = std::fs::read(entry.path()).expect("read file");
        files.insert(
            entry.file_name().to_string_lossy().into_owned(),
            hex::encode(Sha256::digest(&bytes)),
        );
    }
    files
}

/// The segment files of `partition` in `wal_dir`, by first sequence.
pub(super) fn segment_files(wal_dir: &Path, partition: u32) -> BTreeMap<u64, PathBuf> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(wal_dir).expect("read the WAL dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((p, first_seq)) = parse_segment_filename(&name) {
            if p == partition {
                files.insert(first_seq, entry.path());
            }
        }
    }
    files
}

/// The frame sequences each segment file of `partition` holds, by the
/// segment's first sequence, decoded from the files themselves.
pub(super) fn frames_in(wal_dir: &Path, partition: u32) -> BTreeMap<u64, Vec<u64>> {
    segment_files(wal_dir, partition)
        .into_iter()
        .map(|(first_seq, path)| {
            let data = std::fs::read(&path).expect("read segment");
            let entries =
                WalWriter::decode_segment_or_refuse(&path, &data, true).expect("decode segment");
            (first_seq, entries.iter().map(|e| e.sequence).collect())
        })
        .collect()
}

/// Every frame sequence of `partition` on disk.
pub(super) fn frames_on_disk(wal_dir: &Path, partition: u32) -> BTreeSet<u64> {
    frames_in(wal_dir, partition)
        .into_values()
        .flatten()
        .collect()
}

/// The `.applied` / `.applied.tmp` files of `wal_dir`, by name.
pub(super) fn sidecar_files(wal_dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(wal_dir)
        .expect("read the WAL dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            name.starts_with("partition-")
                && (name.ends_with(".applied") || name.ends_with(".applied.tmp"))
        })
        .collect()
}

/// The path of the `.applied` file of `partition`.
pub(super) fn sidecar_path(wal_dir: &Path, partition: u32) -> PathBuf {
    wal_dir.join(format!("partition-{partition:03}.applied"))
}

/// The collected-up-to record of `wal_dir`, one value per partition, or
/// `None` when the file is absent.
pub(super) fn read_collected(wal_dir: &Path) -> Option<Vec<u64>> {
    let path = wal_dir.join(WAL_COLLECTED_FILE);
    path.exists().then(|| {
        decode_wal_collected(&std::fs::read(&path).expect("read wal-collected"))
            .expect("a well-formed wal-collected")
    })
}

// ---------------------------------------------------------------------------
// A node: the store, its store-mode WAL and, when a test asks, write-behind
// ---------------------------------------------------------------------------

/// The store over a counting engine and a WAL opened over that store.
pub(super) struct Node {
    pub(super) engine: Arc<CountingEngine>,
    pub(super) store: Arc<EngineMapDataStore<CountingEngine>>,
    pub(super) wal: Arc<WalWriter>,
}

impl Node {
    /// Opens `root`'s store and its WAL directory in store mode.
    pub(super) fn open(root: &Root, policy: WalFsyncPolicy) -> anyhow::Result<Self> {
        Self::open_over(open_engine(root), root, policy)
    }

    /// As [`Node::open`], over an engine the caller has already armed.
    pub(super) fn open_over(
        engine: Arc<CountingEngine>,
        root: &Root,
        policy: WalFsyncPolicy,
    ) -> anyhow::Result<Self> {
        let store = Arc::new(EngineMapDataStore::open(Arc::clone(&engine))?);
        let wal = WalWriter::open_over_store(
            root.wal_dir(),
            policy,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )?;
        Ok(Self { engine, store, wal })
    }

    pub(super) fn dyn_store(&self) -> Arc<dyn MapDataStore> {
        Arc::clone(&self.store) as Arc<dyn MapDataStore>
    }

    /// Runs WAL recovery into the store, as the server does at start.
    pub(super) async fn recover(&self) -> anyhow::Result<()> {
        WalRecovery::new(Arc::clone(&self.wal), Vec::new())
            .run(self.dyn_store())
            .await
    }

    /// The first sequence a restarted server hands out.
    pub(super) async fn sequence_start(&self) -> u64 {
        self.wal
            .max_observed_sequence()
            .await
            .expect("max_observed_sequence")
            + 1
    }

    /// Write-behind over the store and this WAL, seeded as the server seeds it.
    pub(super) async fn write_behind(
        &self,
        config: WriteBehindConfig,
    ) -> Arc<WriteBehindDataStore> {
        self.write_behind_over(Arc::clone(&self.wal) as Arc<dyn Wal>, config)
            .await
    }

    /// Write-behind over the store and `wal`, a wrapper of this node's WAL.
    pub(super) async fn write_behind_over(
        &self,
        wal: Arc<dyn Wal>,
        config: WriteBehindConfig,
    ) -> Arc<WriteBehindDataStore> {
        let sequence_start = self.sequence_start().await;
        WriteBehindDataStore::new_with_wal(
            self.dyn_store(),
            config,
            Some(WalBootstrap {
                wal,
                sequence_start,
            }),
        )
    }

    /// Appends a `Store` frame of `(MAP, key)` to `partition`.
    pub(super) async fn append(&self, partition: u32, key: &str, value: &RecordValue, seq: u64) {
        self.wal
            .append(partition, &store_frame(key, value, seq))
            .await
            .expect("append");
    }

    /// Commits `records` with the rows `applied` in one store commit, then
    /// tells the WAL each row is durable: the store-mode protocol.
    pub(super) async fn commit(
        &self,
        records: &[(String, RecordValue)],
        applied: &[(u32, u64)],
    ) -> anyhow::Result<()> {
        let writes = records
            .iter()
            .map(|(key, value)| StoreWrite::Put {
                map: MAP,
                key,
                value: PutValue::Value(value),
                expiration_time: 0,
                store_time: 0,
            })
            .collect();
        self.store
            .commit(&FlushCommit {
                writes,
                applied: applied.to_vec(),
            })
            .await?;
        for (partition, sequence) in applied {
            self.wal.mark_applied(*partition, *sequence).await?;
        }
        Ok(())
    }

    /// Drops the WAL and the store and waits until nothing holds either, so
    /// the engine file is closed and the next open of `root` is a restart.
    pub(super) async fn close(self) {
        let Self { engine, store, wal } = self;
        let wal_weak = Arc::downgrade(&wal);
        drop(wal);
        drop(store);
        wait_for("the WAL writer and the store to be released", || {
            wal_weak.upgrade().is_none() && Arc::strong_count(&engine) == 1
        })
        .await;
        drop(engine);
    }
}

/// Stops write-behind as a clean server stop does, and drops it.
pub(super) async fn stop_write_behind(wb: Arc<WriteBehindDataStore>) {
    wb.hard_flush().await.expect("hard_flush");
    drop(wb);
}

/// Write-behind that flushes only when asked and never retries a refused
/// entry within a test.
pub(super) fn wb_config() -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 0,
        flush_interval_ms: 600_000,
        backoff_base_ms: 600_000,
        backoff_cap_ms: 600_000,
        shutdown_timeout_ms: 10_000,
        capacity: 0,
        ..WriteBehindConfig::default()
    }
}

/// Polls `done` until it holds, failing the test after 30 s.
pub(super) async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Requests a flush pass and waits until nothing is outstanding.
pub(super) async fn flush_idle(wb: &WriteBehindDataStore) {
    wb.soft_flush().await.expect("soft_flush");
    wait_for("the flush pass to leave nothing outstanding", || {
        wb.test_is_idle()
    })
    .await;
}

/// The pending WAL sequences of `partition`.
pub(super) fn pending(wb: &WriteBehindDataStore, partition: u32) -> Vec<u64> {
    wb.test_pending_wal_sequences(partition)
        .into_iter()
        .map(|(seq, _)| seq)
        .collect()
}

/// The value of `topgun_wal_gc_skipped_total{reason}` on a scrape.
pub(super) fn gc_skipped(reason: &str) -> f64 {
    let rendered = crate::service::middleware::observability::init_observability().render_metrics();
    rendered
        .lines()
        .find(|line| line.starts_with(WAL_GC_SKIPPED_COUNTER) && line.contains(reason))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// A [`Wal`] that forwards everything to a [`WalWriter`] except `unapplied`
/// of one partition, which fails: write-behind cannot seed that partition.
pub(super) struct UnseedableWal {
    pub(super) inner: Arc<WalWriter>,
    pub(super) partition: u32,
}

#[async_trait]
impl Wal for UnseedableWal {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        self.inner.append(partition, entry).await
    }

    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        self.inner.mark_applied(partition, sequence).await
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        if partition == self.partition {
            anyhow::bail!("injected: the frames of partition {partition} cannot be read");
        }
        self.inner.unapplied(partition).await
    }

    async fn sync(&self) -> anyhow::Result<()> {
        self.inner.sync().await
    }

    fn watermark_in_store(&self) -> bool {
        self.inner.watermark_in_store()
    }

    fn durable_applied(&self, partition: u32) -> Option<u64> {
        self.inner.durable_applied(partition)
    }

    async fn finish(&self, write_hints: bool) -> anyhow::Result<usize> {
        self.inner.finish(write_hints).await
    }
}

// ---------------------------------------------------------------------------
// Child processes
// ---------------------------------------------------------------------------

/// Set in a child: `"{scenario}\n{root}"`.
const CHILD_ENV: &str = "TOPGUN_TEST_STORE_MODE_CHILD";
const CHILD_DEADLINE: Duration = Duration::from_secs(120);
/// Printed by a child whose scenario returned instead of stopping.
pub(super) const CHILD_RETURNED: &str = "STORE-MODE-CHILD-RETURNED";

/// What a child run ended with.
pub(super) struct ChildRun {
    pub(super) status: std::process::ExitStatus,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl ChildRun {
    /// Whether the child stopped by `abort`, the model of `kill -9`.
    pub(super) fn aborted(&self) -> bool {
        self.status.signal() == Some(6)
    }

    /// Fails the test unless the child stopped by `abort`.
    pub(super) fn assert_aborted(&self, what: &str) {
        assert!(
            self.aborted(),
            "{what}: the child was to stop by abort, it ended with {:?}\n--- stdout\n{}\n--- stderr\n{}",
            self.status,
            self.stdout,
            self.stderr
        );
    }
}

/// The scenario and root this process was started with as a child, if any.
pub(super) fn child_spec() -> Option<(String, Root)> {
    let spec = std::env::var(CHILD_ENV).ok()?;
    let (scenario, root) = spec.split_once('\n').expect("child spec");
    Some((scenario.to_string(), Root::at(PathBuf::from(root))))
}

/// Re-runs this test binary with only `test` selected, as a child that runs
/// `scenario` on `root`, armed at `abort_at` when given; waits for it with a
/// hard deadline, a child still running then failing the test as a hang.
pub(super) fn run_child(
    test: &str,
    scenario: &str,
    root: &Root,
    abort_at: Option<TestAbortPoint>,
) -> ChildRun {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let logs = root.path.join("child-logs");
    std::fs::create_dir_all(&logs).expect("logs dir");
    let label = format!(
        "{}-{}",
        RUNS.fetch_add(1, Ordering::SeqCst),
        scenario.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let out_path = logs.join(format!("{label}.out"));
    let err_path = logs.join(format!("{label}.err"));
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, rest)| rest);
    let mut command = std::process::Command::new(std::env::current_exe().expect("test binary"));
    command
        .args([
            &format!("{module}::{test}"),
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env_remove("DATABASE_URL")
        .env(CHILD_ENV, format!("{scenario}\n{}", root.path.display()))
        // The store location the binary reads from the environment, given to
        // every child so none inherits the parent's and none mutates its own.
        .env("TOPGUN_DATA_DIR", root.data_dir())
        .env("TOPGUN_REDB_PATH", root.path.join("topgun.redb"))
        .stdout(std::fs::File::create(&out_path).expect("stdout file"))
        .stderr(std::fs::File::create(&err_path).expect("stderr file"));
    match abort_at {
        Some(point) => command.env(TEST_ABORT_AT_ENV, point.name()),
        None => command.env_remove(TEST_ABORT_AT_ENV),
    };
    let mut child = command.spawn().expect("the child starts");
    let deadline = Instant::now() + CHILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child {label} still running after {CHILD_DEADLINE:?}: a hang");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    ChildRun {
        status,
        stdout: std::fs::read_to_string(&out_path).unwrap_or_default(),
        stderr: std::fs::read_to_string(&err_path).unwrap_or_default(),
    }
}

/// Opens `root` in store mode, expects the open to be refused with every
/// phrase of `expect`, and checks that the refusal wrote nothing: no engine
/// write, the engine content and every file of the WAL directory unchanged.
async fn assert_open_refused_untouched(root: &Root, expect: &[String]) {
    let wal_before = fingerprint(&root.wal_dir());
    let engine_before = dump_at(root);
    let engine = open_engine(root);
    let mark = engine.mark();
    let error = match Node::open_over(Arc::clone(&engine), root, WalFsyncPolicy::PerOp) {
        Ok(node) => {
            node.close().await;
            panic!("the open was to be refused, it succeeded");
        }
        Err(e) => format!("{e:#}"),
    };
    for phrase in expect {
        assert!(
            error.contains(phrase.as_str()),
            "the refusal does not say {phrase:?}: {error}"
        );
    }
    assert!(
        engine.writes_since(mark).is_empty(),
        "a refused open wrote to the engine: {:?}",
        engine.writes_since(mark)
    );
    wait_for("the refused open to release the engine", || {
        Arc::strong_count(&engine) == 1
    })
    .await;
    drop(engine);
    assert_eq!(dump_at(root), engine_before, "the engine content changed");
    assert_eq!(
        fingerprint(&root.wal_dir()),
        wal_before,
        "a file of the WAL directory changed"
    );
}

const SEAL_AGE: Duration = Duration::from_secs(31);

const P: u32 = 7;
const Q: u32 = 11;
const R: u32 = 13;
const U: u32 = 17;

// ---------------------------------------------------------------------------
// Main tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a01_a_commit_is_one_engine_write_holding_its_records_and_its_watermark_rows() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wb = node.write_behind(wb_config()).await;

    // The map's first record also writes its registry rows; it is stored
    // first, so every later commit holds a record and rows only.
    let warm = keys_in(P, 1).remove(0);
    wb.add(MAP, &warm, &lww("warm", 1), 0, 0).await.unwrap();
    flush_idle(&wb).await;

    let mark = node.engine.mark();
    let mut entries = Vec::new();
    // A pass commits every due entry at once, so each entry is flushed on
    // its own to make each pass the commit of exactly one entry.
    for partition in [Q, R, U] {
        let key = keys_in(partition, 1).remove(0);
        wb.add(MAP, &key, &lww(&key, 2), 0, 0).await.unwrap();
        let seq = wb.test_max_assigned_wal_sequence(partition);
        entries.push((partition, key, seq));
        flush_idle(&wb).await;
    }

    let writes = node.engine.writes_since(mark);
    assert_eq!(
        writes.len(),
        entries.len(),
        "each stored entry is exactly one engine write, nothing else is written: {writes:?}"
    );
    for (partition, key, seq) in &entries {
        let record = record_key(&node.engine, key);
        let holding: Vec<&WriteRecord> = writes.iter().filter(|w| w.touches(&record)).collect();
        assert_eq!(holding.len(), 1, "one write holds the record of {key}");
        let write = holding[0];
        assert!(write.ok && write.sync, "the commit is durable: {write:?}");
        assert_eq!(
            write.record_keys(),
            vec![record.clone()],
            "the commit holds its own record and no other"
        );
        assert!(
            write.other_keys().is_empty(),
            "the commit holds nothing but its record and its rows: {write:?}"
        );
        assert!(
            write.rows().contains(&(*partition, *seq)),
            "the commit of {key} holds the row ({partition}, {seq}) of its partition: {:?}",
            write.rows()
        );
        assert_eq!(node.engine.row(*partition), Some(*seq));
        assert_eq!(node.wal.durable_applied(*partition), Some(*seq));
    }

    stop_write_behind(wb).await;
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a02_a_refused_commit_changes_no_watermark_and_no_pending_sequence() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wb = node.write_behind(wb_config()).await;

    let keys = keys_in(Q, 2);
    wb.add(MAP, &keys[0], &lww("warm", 1), 0, 0).await.unwrap();
    flush_idle(&wb).await;
    let warm_seq = wb.test_max_assigned_wal_sequence(Q);
    assert_eq!(node.wal.durable_applied(Q), Some(warm_seq));

    let value = lww("refused", 2);
    wb.add(MAP, &keys[1], &value, 0, 0).await.unwrap();
    let seq = wb.test_max_assigned_wal_sequence(Q);
    let pending_before = pending(&wb, Q);
    let count_before = wb.pending_operation_count();
    let fence_before = wb.flushed_watermark();
    let dump_before = node.engine.dump();
    assert!(pending_before.contains(&seq));

    node.engine
        .fail_writes_holding(Some(record_key(&node.engine, &keys[1])));
    let mark = node.engine.mark();
    wb.soft_flush().await.unwrap();
    wait_for("the refused commit's pass to close", || {
        node.engine.refused_since(mark) >= 1 && !wb.test_pass_open()
    })
    .await;

    assert_eq!(
        node.engine.dump(),
        dump_before,
        "a refused commit writes neither the record nor a row"
    );
    assert_eq!(
        node.wal.durable_applied(Q),
        Some(warm_seq),
        "a refused commit moves no durable watermark"
    );
    assert_eq!(
        pending(&wb, Q),
        pending_before,
        "the sequence stays pending"
    );
    assert_eq!(wb.pending_operation_count(), count_before, "the count");
    assert_eq!(wb.flushed_watermark(), fence_before, "the durability fence");
    assert_eq!(
        wb.load(MAP, &keys[1]).await.unwrap(),
        Some(value),
        "the staged value still serves the key"
    );

    // An aged segment is sealed by a tick, and no tick collects a frame the
    // store has not taken.
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    node.wal.test_collect_now().await;
    assert!(
        frames_on_disk(&root.wal_dir(), Q).contains(&seq),
        "the refused entry's frame is still in the WAL"
    );
    assert!(
        node.wal
            .unapplied(Q)
            .await
            .unwrap()
            .iter()
            .any(|e| e.sequence == seq),
        "and still un-applied"
    );
    assert_eq!(node.wal.durable_applied(Q), Some(warm_seq));
    assert_eq!(node.engine.dump(), dump_before, "a tick writes nothing");

    node.engine.fail_writes_holding(None);
    stop_write_behind(wb).await;
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a03_a_partition_whose_watermark_moved_by_coalescing_gets_its_row() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wb = node.write_behind(wb_config()).await;

    let r_keys = keys_in(R, 2);
    let q_keys = keys_in(Q, 2);
    // Both partitions stored once, so both are seeded and hold a row.
    wb.add(MAP, &r_keys[0], &lww("r0", 1), 0, 0).await.unwrap();
    wb.add(MAP, &q_keys[0], &lww("q0", 1), 0, 0).await.unwrap();
    flush_idle(&wb).await;
    let r_row = node.engine.row(R).expect("r's row");

    // Two writes of one key in R: the second retires the first, whose
    // sequence resolves at once, so R's watermark moves without R storing.
    wb.add(MAP, &r_keys[1], &lww("r1", 2), 0, 0).await.unwrap();
    let retired = wb.test_max_assigned_wal_sequence(R);
    wb.add(MAP, &r_keys[1], &lww("r1-again", 3), 0, 0)
        .await
        .unwrap();
    let survivor = wb.test_max_assigned_wal_sequence(R);
    assert_eq!(
        pending(&wb, R),
        vec![survivor],
        "the retired sequence resolved"
    );
    assert!(survivor > retired && retired > r_row);

    wb.add(MAP, &q_keys[1], &lww("q1", 4), 0, 0).await.unwrap();
    let q_seq = wb.test_max_assigned_wal_sequence(Q);

    // R's survivor is refused, so the only commit that can carry R's row is
    // the commit of Q's entry.
    node.engine
        .fail_writes_holding(Some(record_key(&node.engine, &r_keys[1])));
    let q_record = record_key(&node.engine, &q_keys[1]);
    let mark = node.engine.mark();
    wb.soft_flush().await.unwrap();
    wait_for("Q's commit and R's refusal", || {
        node.engine.refused_since(mark) >= 1
            && node
                .engine
                .committed_since(mark)
                .iter()
                .any(|w| w.touches(&q_record))
            && !wb.test_pass_open()
    })
    .await;

    let q_commit = node
        .engine
        .committed_since(mark)
        .into_iter()
        .find(|w| w.touches(&q_record))
        .expect("Q's commit");
    assert!(
        q_commit.rows().contains(&(R, survivor - 1)),
        "the commit of Q's entry holds R's row, moved by the coalesce: {:?}",
        q_commit.rows()
    );
    assert!(q_commit.rows().contains(&(Q, q_seq)));
    assert_eq!(node.engine.row(R), Some(survivor - 1));
    assert_eq!(node.wal.durable_applied(R), Some(survivor - 1));

    node.engine.fail_writes_holding(None);
    stop_write_behind(wb).await;
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a04_an_unseeded_partition_gets_no_row() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wal = Arc::new(UnseedableWal {
        inner: Arc::clone(&node.wal),
        partition: U,
    });
    let wb = node
        .write_behind_over(Arc::clone(&wal) as Arc<dyn Wal>, wb_config())
        .await;

    let warm = keys_in(Q, 2);
    wb.add(MAP, &warm[0], &lww("warm", 1), 0, 0).await.unwrap();
    flush_idle(&wb).await;

    let u_key = keys_in(U, 1).remove(0);
    wb.add(MAP, &u_key, &lww("u", 2), 0, 0).await.unwrap();
    assert!(
        !wb.test_wal_partition_seeded(U),
        "U is tracked but cannot be seeded"
    );
    assert!(!pending(&wb, U).is_empty(), "U has a pending sequence");
    wb.add(MAP, &warm[1], &lww("q", 3), 0, 0).await.unwrap();
    let q_seq = wb.test_max_assigned_wal_sequence(Q);

    let mark = node.engine.mark();
    let u_record = record_key(&node.engine, &u_key);
    let q_record = record_key(&node.engine, &warm[1]);
    wb.soft_flush().await.unwrap();
    wait_for("both entries stored", || {
        let committed = node.engine.committed_since(mark);
        committed.iter().any(|w| w.touches(&u_record))
            && committed.iter().any(|w| w.touches(&q_record))
            && !wb.test_pass_open()
    })
    .await;

    for write in node.engine.committed_since(mark) {
        assert!(
            write.rows().iter().all(|(p, _)| *p != U),
            "a commit holds a row of the unseeded partition: {:?}",
            write.rows()
        );
    }
    assert_eq!(
        node.engine.row(U),
        None,
        "the unseeded partition has no row"
    );
    assert_eq!(node.wal.durable_applied(U), Some(0));
    assert_eq!(
        node.engine.row(Q),
        Some(q_seq),
        "the seeded partition has its row"
    );

    stop_write_behind(wb).await;
    drop(wal);
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a05_the_engine_watermark_never_decreases_across_commits_and_restarts() {
    let root = Root::new();
    let p_keys = keys_in(P, 4);
    let q_key = keys_in(Q, 1).remove(0);

    // First run: several commits in P.
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wb = node.write_behind(wb_config()).await;
    let mark = node.engine.mark();
    for (i, key) in p_keys[..3].iter().enumerate() {
        wb.add(MAP, key, &lww(key, i as u64 + 1), 0, 0)
            .await
            .unwrap();
        flush_idle(&wb).await;
    }
    let first_run = node.engine.committed_since(mark);
    stop_write_behind(wb).await;
    let stored = rows_of(&node.engine.dump());
    let p_stored = *stored.get(&P).expect("P's row");
    node.close().await;

    let mut last: BTreeMap<u32, u64> = BTreeMap::new();
    for write in &first_run {
        for (p, v) in write.rows() {
            if let Some(previous) = last.get(&p) {
                assert!(v > *previous, "row of {p} went from {previous} to {v}");
            }
            last.insert(p, v);
        }
    }

    // Restart: P is written again, and its entry is refused, so P stays
    // pending just above its stored row while Q's entry is committed.
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode reopen");
    node.recover().await.expect("recovery");
    let wb = node.write_behind(wb_config()).await;
    wb.add(MAP, &p_keys[3], &lww("p-again", 10), 0, 0)
        .await
        .unwrap();
    assert!(wb.test_max_assigned_wal_sequence(P) > p_stored);
    // The registry row exists since the first run, so the record key is known.
    node.engine
        .fail_writes_holding(Some(record_key(&node.engine, &p_keys[3])));
    wb.add(MAP, &q_key, &lww("q", 11), 0, 0).await.unwrap();
    let q_seq = wb.test_max_assigned_wal_sequence(Q);
    let q_record = record_key(&node.engine, &q_key);
    let mark = node.engine.mark();
    wb.soft_flush().await.unwrap();
    wait_for("Q's commit after the restart", || {
        node.engine.refused_since(mark) >= 1
            && node
                .engine
                .committed_since(mark)
                .iter()
                .any(|w| w.touches(&q_record))
            && !wb.test_pass_open()
    })
    .await;

    for write in node.engine.committed_since(mark) {
        for (p, v) in write.rows() {
            if let Some(previous) = last.get(&p) {
                assert!(
                    v > *previous,
                    "after the restart the row of {p} went from {previous} to {v}"
                );
            }
            last.insert(p, v);
        }
    }
    assert_eq!(
        node.engine.row(P),
        Some(p_stored),
        "P's row is never lowered"
    );
    assert_eq!(node.engine.row(Q), Some(q_seq));

    node.engine.fail_writes_holding(None);
    stop_write_behind(wb).await;
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a06_the_wal_collects_only_up_to_the_durable_watermark_on_its_tick() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let keys = keys_in(P, 4);
    let values: Vec<RecordValue> = (1..=4).map(|i| lww("v", i)).collect();

    // Two sealed segments, [1, 2] and [3, 4], nothing applied yet.
    node.append(P, &keys[0], &values[0], 1).await;
    node.append(P, &keys[1], &values[1], 2).await;
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    node.append(P, &keys[2], &values[2], 3).await;
    node.append(P, &keys[3], &values[3], 4).await;
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    let sealed = frames_in(&root.wal_dir(), P);
    assert!(
        sealed.values().any(|f| f == &vec![1, 2]) && sealed.values().any(|f| f == &vec![3, 4]),
        "two sealed segments: {sealed:?}"
    );

    // The WAL's copy says 2, but no commit made it durable: the tick reads
    // the engine, finds 0, and retains.
    node.wal.mark_applied(P, 2).await.unwrap();
    let before = gc_skipped("watermark_below_segment");
    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([1, 2, 3, 4]),
        "nothing above the engine's watermark is collected"
    );
    assert!(
        gc_skipped("watermark_below_segment") > before,
        "the retention is counted"
    );
    assert_eq!(
        read_collected(&root.wal_dir()).expect("wal-collected")[P as usize],
        0
    );

    // Durable now; a lower watermark the tick reads still retains.
    node.commit(
        &[
            (keys[0].clone(), values[0].clone()),
            (keys[1].clone(), values[1].clone()),
        ],
        &[(P, 2)],
    )
    .await
    .expect("commit");
    node.wal.test_force_tick_watermark(P, Some(1));
    let before = gc_skipped("watermark_below_segment");
    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([1, 2, 3, 4]),
        "a tick reading 1 keeps the segment that ends at 2"
    );
    assert!(gc_skipped("watermark_below_segment") > before);

    // The tick's own read at 2: [1, 2] goes, [3, 4] stays.
    node.wal.test_force_tick_watermark(P, None);
    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([3, 4]),
        "collected up to the durable watermark and no further"
    );
    assert_eq!(
        read_collected(&root.wal_dir()).expect("wal-collected")[P as usize],
        2
    );

    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a07_segments_are_sealed_by_size_or_age_not_per_commit() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    // Collection is held off, so every segment a seal leaves stays countable.
    node.wal.test_force_tick_watermark(P, Some(0));
    let wb = node.write_behind(wb_config()).await;
    let keys = keys_in(P, 211);

    for (i, key) in keys[..200].iter().enumerate() {
        wb.add(MAP, key, &lww(key, i as u64 + 1), 0, 0)
            .await
            .unwrap();
        flush_idle(&wb).await;
    }
    assert_eq!(
        node.engine.row(P),
        Some(wb.test_max_assigned_wal_sequence(P))
    );
    node.wal.test_collect_now().await;
    assert_eq!(
        segment_files(&root.wal_dir(), P).len(),
        1,
        "200 commits under 8 MiB and 30 s leave one segment"
    );

    let big = "x".repeat(1 << 20);
    for (i, key) in keys[200..209].iter().enumerate() {
        wb.add(MAP, key, &lww(&big, 1_000 + i as u64), 0, 0)
            .await
            .unwrap();
        flush_idle(&wb).await;
    }
    node.wal.test_collect_now().await;
    assert_eq!(
        segment_files(&root.wal_dir(), P).len(),
        2,
        "8 MiB appended: sealed at the next tick, a new segment started"
    );

    wb.add(MAP, &keys[209], &lww("small", 2_000), 0, 0)
        .await
        .unwrap();
    flush_idle(&wb).await;
    node.wal.test_collect_now().await;
    assert_eq!(
        segment_files(&root.wal_dir(), P).len(),
        2,
        "a young small segment is not sealed"
    );
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    assert_eq!(
        segment_files(&root.wal_dir(), P).len(),
        3,
        "a segment whose first frame is 30 s old is sealed"
    );

    stop_write_behind(wb).await;
    node.close().await;
}

/// Builds three sealed segments of P, `[1, 2]`, `[3, 4]`, `[5, 6]`, and a
/// commit of 1..=4 with the row `(P, 4)`; 5 and 6 are only in the WAL.
async fn a08_setup(root: &Root) -> Vec<(String, RecordValue)> {
    let node = Node::open(root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let records: Vec<(String, RecordValue)> = keys_in(P, 6)
        .into_iter()
        .enumerate()
        .map(|(i, key)| {
            let value = lww(&key, i as u64 + 1);
            (key, value)
        })
        .collect();
    node.wal.test_force_tick_watermark(P, Some(0));
    for (i, pair) in records.chunks(2).enumerate() {
        let seq = 2 * i as u64 + 1;
        node.append(P, &pair[0].0, &pair[0].1, seq).await;
        node.append(P, &pair[1].0, &pair[1].1, seq + 1).await;
        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
    }
    node.commit(&records[..4], &[(P, 4)]).await.expect("commit");
    node.wal.test_force_tick_watermark(P, None);
    node.close().await;
    records
}

#[tokio::test(flavor = "multi_thread")]
async fn a08_the_collected_mark_is_durable_before_the_first_unlink() {
    const TEST: &str = "a08_the_collected_mark_is_durable_before_the_first_unlink";
    if let Some((scenario, root)) = child_spec() {
        assert_eq!(scenario, "collect");
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        node.wal.test_collect_now().await;
        println!("{CHILD_RETURNED}");
        return;
    }

    for point in [
        TestAbortPoint::CollectAfterMark,
        TestAbortPoint::CollectInsideUnlink,
        TestAbortPoint::CollectAfterUnlink,
    ] {
        let root = Root::new();
        let records = a08_setup(&root).await;
        let segments = frames_in(&root.wal_dir(), P);
        let covered: Vec<(u64, PathBuf, u64)> = segment_files(&root.wal_dir(), P)
            .into_iter()
            .filter_map(|(first, path)| {
                let max = *segments[&first].iter().max()?;
                (max <= 4).then_some((first, path, max))
            })
            .collect();
        assert_eq!(
            covered.len(),
            2,
            "two sealed segments at or below W: {segments:?}"
        );
        let saved = root.path.join("saved");
        std::fs::create_dir_all(&saved).unwrap();
        for (first, path, _) in &covered {
            std::fs::copy(path, saved.join(first.to_string())).unwrap();
        }

        run_child(TEST, "collect", &root, Some(point)).assert_aborted(point.name());

        // What the crash left: every unlinked segment is covered by a
        // durable collected mark, and the mark never passes W.
        let collected = read_collected(&root.wal_dir()).expect("wal-collected")[P as usize];
        assert!(collected <= 4, "{point:?}: G(P) = {collected} above W = 4");
        let gone: Vec<u64> = covered
            .iter()
            .filter(|(_, path, _)| !path.exists())
            .map(|(_, _, max)| *max)
            .collect();
        for max in &gone {
            assert!(
                collected >= *max,
                "{point:?}: a segment up to {max} was unlinked while G(P) = {collected}"
            );
        }
        match point {
            TestAbortPoint::CollectAfterMark => {
                assert!(gone.is_empty(), "nothing unlinked yet");
                assert_eq!(collected, 4);
            }
            TestAbortPoint::CollectInsideUnlink => assert_eq!(gone.len(), 1),
            _ => assert_eq!(gone.len(), 2),
        }
        assert!(frames_on_disk(&root.wal_dir(), P).is_superset(&BTreeSet::from([5, 6])));

        // The restart passes the directory check, replays what is above W,
        // and collects what the crash left behind.
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("the restart is not refused");
        node.recover().await.expect("recovery");
        for (key, value) in &records {
            assert_eq!(
                node.store.load(MAP, key).await.unwrap().as_ref(),
                Some(value),
                "{point:?}: {key} after the restart"
            );
        }
        let row = node.engine.row(P).expect("P's row");
        assert!(row >= 4);
        assert!(read_collected(&root.wal_dir()).unwrap()[P as usize] <= row);
        node.wal.test_collect_now().await;
        for (_, path, _) in &covered {
            assert!(!path.exists(), "{point:?}: {path:?} is collected again");
        }
        node.close().await;

        if point == TestAbortPoint::CollectAfterUnlink {
            // An unlink the power loss undid: the file comes back. Its frames
            // are at or below W, so nothing replays and the tick collects it.
            let (first, path, _) = &covered[0];
            std::fs::copy(saved.join(first.to_string()), path).unwrap();
            let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
            let mark = node.engine.mark();
            node.recover().await.expect("recovery");
            assert!(
                node.engine
                    .writes_since(mark)
                    .iter()
                    .all(|w| w.record_keys().is_empty()),
                "a frame at or below W was replayed"
            );
            node.wal.test_collect_now().await;
            assert!(!path.exists(), "the re-created segment is collected again");
            node.close().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a09_a_data_directory_older_than_the_wal_directory_is_refused_and_nothing_is_written() {
    const TEST: &str =
        "a09_a_data_directory_older_than_the_wal_directory_is_refused_and_nothing_is_written";
    let keys = keys_in(P, 4);
    let values: Vec<RecordValue> = (1..=4).map(|i| lww("v", i)).collect();
    if let Some((scenario, root)) = child_spec() {
        // A store-mode run that is killed: no clean stop, so no hints.
        assert_eq!(scenario, "write-and-kill");
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        node.append(P, &keys[0], &values[0], 1).await;
        node.append(P, &keys[1], &values[1], 2).await;
        node.commit(
            &[
                (keys[0].clone(), values[0].clone()),
                (keys[1].clone(), values[1].clone()),
            ],
            &[(P, 2)],
        )
        .await
        .expect("commit");
        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
        std::process::abort();
    }

    let o4 = |v: u64, g: u64| {
        vec![
            "WAL directory check: refusing to start: the data directory is older than the WAL directory"
                .to_string(),
            format!("partition {P} has applied watermark {v} there, but the WAL was collected up to {g}"),
        ]
    };
    let o3 = vec![
        "WAL directory check: refusing to start: the data directory is older than the WAL directory"
            .to_string(),
        "the data directory holds no applied watermarks, but the WAL directory has served a store that keeps them"
            .to_string(),
    ];

    // The data directory copied at W = 2, then the WAL collected up to 4.
    {
        let root = Root::new();
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        node.append(P, &keys[0], &values[0], 1).await;
        node.append(P, &keys[1], &values[1], 2).await;
        node.commit(
            &[
                (keys[0].clone(), values[0].clone()),
                (keys[1].clone(), values[1].clone()),
            ],
            &[(P, 2)],
        )
        .await
        .expect("commit");
        node.close().await;
        let copy = root.path.join("data-copy");
        copy_dir(&root.data_dir(), &copy);

        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
        node.append(P, &keys[2], &values[2], 3).await;
        node.append(P, &keys[3], &values[3], 4).await;
        node.commit(
            &[
                (keys[2].clone(), values[2].clone()),
                (keys[3].clone(), values[3].clone()),
            ],
            &[(P, 4)],
        )
        .await
        .expect("commit");
        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
        assert_eq!(read_collected(&root.wal_dir()).unwrap()[P as usize], 4);
        node.close().await;

        put_data_dir(&root, &copy);
        assert_open_refused_untouched(&root, &o4(2, 4)).await;
    }

    // (i) A data directory with no applied watermarks beside the WAL
    // directory of a killed store-mode run.
    {
        let root = Root::new();
        run_child(TEST, "write-and-kill", &root, None).assert_aborted("write-and-kill");
        assert!(
            read_collected(&root.wal_dir()).is_some(),
            "wal-collected is present"
        );
        assert!(
            sidecar_files(&root.wal_dir()).is_empty(),
            "no hints after a kill"
        );
        let pre_marker = root.path.join("pre-marker");
        write_pre_marker_data_dir(&pre_marker).await;
        put_data_dir(&root, &pre_marker);
        assert_open_refused_untouched(&root, &o3).await;
    }

    // (ii) The same beside the WAL directory of a clean stop, whose rollback
    // hints would otherwise read as sidecars to import.
    {
        let root = Root::new();
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let wb = node.write_behind(wb_config()).await;
        for partition in [P, Q] {
            let key = keys_in(partition, 1).remove(0);
            wb.add(MAP, &key, &lww(&key, 1), 0, 0).await.unwrap();
        }
        flush_idle(&wb).await;
        stop_write_behind(wb).await;
        node.close().await;
        let collected = read_collected(&root.wal_dir()).expect("wal-collected is present");
        let hints = sidecar_files(&root.wal_dir());
        assert!(!hints.is_empty(), "a clean stop wrote rollback hints");
        for partition in [P, Q] {
            let hint = std::fs::read(sidecar_path(&root.wal_dir(), partition)).expect("a hint");
            assert!(decode_u64(&hint) >= collected[partition as usize]);
        }
        let pre_marker = root.path.join("pre-marker");
        write_pre_marker_data_dir(&pre_marker).await;
        put_data_dir(&root, &pre_marker);
        assert_open_refused_untouched(&root, &o3).await;
    }
}

const A10_P1: u32 = 3;
const A10_P2: u32 = 4;

/// A WAL directory written in sidecar mode — sidecars `A10_P1 = 3` and
/// `A10_P2 = 4` — beside a data directory with no applied watermarks.
async fn a10_setup(root: &Root) {
    write_pre_marker_data_dir(&root.data_dir()).await;
    let wal = WalWriter::new(root.wal_dir(), WalFsyncPolicy::PerOp).expect("sidecar-mode open");
    for seq in 1..=3 {
        wal.append(
            A10_P1,
            &store_frame(&format!("a{seq}"), &lww("a", seq), seq),
        )
        .await
        .unwrap();
    }
    for seq in 4..=5 {
        wal.append(
            A10_P2,
            &store_frame(&format!("b{seq}"), &lww("b", seq), seq),
        )
        .await
        .unwrap();
    }
    wal.mark_applied(A10_P1, 3).await.unwrap();
    wal.mark_applied(A10_P2, 4).await.unwrap();
    assert_eq!(wal.test_read_applied_sequence(A10_P1), 3);
    assert_eq!(wal.test_read_applied_sequence(A10_P2), 4);
    let weak = Arc::downgrade(&wal);
    drop(wal);
    wait_for("the sidecar-mode WAL to be released", || {
        weak.upgrade().is_none()
    })
    .await;
}

/// Checks the state a finished import leaves.
fn a10_assert_imported(root: &Root, node: &Node) {
    assert_eq!(node.engine.row(A10_P1), Some(3));
    assert_eq!(node.engine.row(A10_P2), Some(4));
    assert_eq!(node.engine.marker(), Some(vec![1]), "the import marker");
    assert_eq!(node.wal.durable_applied(A10_P1), Some(3));
    assert_eq!(node.wal.durable_applied(A10_P2), Some(4));
    assert!(
        sidecar_files(&root.wal_dir()).is_empty(),
        "every sidecar is deleted: {:?}",
        sidecar_files(&root.wal_dir())
    );
    assert!(
        read_collected(&root.wal_dir()).is_some(),
        "wal-collected exists"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a10_sidecars_are_imported_once_under_the_marker_and_never_read_again() {
    const TEST: &str = "a10_sidecars_are_imported_once_under_the_marker_and_never_read_again";
    if let Some((scenario, root)) = child_spec() {
        let engine = open_engine(&root);
        match scenario.as_str() {
            "import-before" => engine.abort_in(1, AbortSide::Before),
            "import-after" => engine.abort_in(1, AbortSide::After),
            "delete" => {}
            other => panic!("unknown scenario {other}"),
        }
        let node = Node::open_over(engine, &root, WalFsyncPolicy::PerOp).expect("open");
        drop(node);
        println!("{CHILD_RETURNED}");
        return;
    }

    // The import, then a sidecar put back lower and one higher than its row:
    // neither is read, and the next start deletes both.
    {
        let root = Root::new();
        a10_setup(&root).await;
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        a10_assert_imported(&root, &node);
        node.close().await;

        std::fs::write(sidecar_path(&root.wal_dir(), A10_P1), 1u64.to_be_bytes()).unwrap();
        std::fs::write(sidecar_path(&root.wal_dir(), A10_P2), 99u64.to_be_bytes()).unwrap();
        let engine = open_engine(&root);
        let mark = engine.mark();
        let node =
            Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp).expect("reopen");
        assert!(
            engine.writes_since(mark).is_empty(),
            "a start after the marker imports nothing"
        );
        drop(engine);
        a10_assert_imported(&root, &node);
        node.close().await;
    }

    // Crashes inside the open converge.
    for (scenario, point) in [
        ("import-before", None),
        ("import-after", None),
        ("delete", Some(TestAbortPoint::OpenInsideSidecarDelete)),
    ] {
        let root = Root::new();
        a10_setup(&root).await;
        run_child(TEST, scenario, &root, point).assert_aborted(scenario);
        let dump = dump_at(&root);
        let marker = dump.get(&meta_key(META_WAL_WATERMARK)).cloned();
        match scenario {
            "import-before" => {
                assert_eq!(marker, None, "the import batch is not applied");
                assert!(rows_of(&dump).is_empty());
                assert_eq!(sidecar_files(&root.wal_dir()).len(), 2);
            }
            "import-after" => {
                assert_eq!(marker, Some(vec![1]));
                assert_eq!(sidecar_files(&root.wal_dir()).len(), 2);
                // The sidecars are no longer read once the marker is present.
                std::fs::write(sidecar_path(&root.wal_dir(), A10_P1), 1u64.to_be_bytes()).unwrap();
                std::fs::write(sidecar_path(&root.wal_dir(), A10_P2), 99u64.to_be_bytes()).unwrap();
            }
            _ => {
                assert_eq!(marker, Some(vec![1]));
                assert_eq!(
                    sidecar_files(&root.wal_dir()).len(),
                    1,
                    "one sidecar deleted"
                );
            }
        }
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("the next start");
        a10_assert_imported(&root, &node);
        node.close().await;
    }

    // A sidecar of the wrong length refuses the import, naming the file.
    {
        let root = Root::new();
        a10_setup(&root).await;
        let bad = sidecar_path(&root.wal_dir(), 5);
        std::fs::write(&bad, [0u8; 7]).unwrap();
        assert_open_refused_untouched(&root, &["partition-005.applied".to_string()]).await;
        assert!(bad.exists());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a11_the_sequence_seed_is_above_the_engine_watermark_of_a_fully_collected_partition() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let records: Vec<(String, RecordValue)> = keys_in(P, 3)
        .into_iter()
        .enumerate()
        .map(|(i, key)| (key, lww("v", i as u64 + 1)))
        .collect();
    for (i, (key, value)) in records.iter().enumerate() {
        node.append(P, key, value, i as u64 + 1).await;
    }
    node.commit(&records, &[(P, 3)]).await.expect("commit");
    node.wal.test_advance_clock(SEAL_AGE);
    node.wal.test_collect_now().await;
    assert!(
        frames_on_disk(&root.wal_dir(), P).is_empty(),
        "every frame of P is collected"
    );
    node.close().await;

    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
    node.recover().await.expect("recovery");
    let start = node.sequence_start().await;
    assert!(start > 3, "the seed {start} is not above the engine row 3");
    let key = keys_in(P, 4).remove(3);
    node.append(P, &key, &lww("new", 9), start).await;
    assert!(
        node.wal
            .unapplied(P)
            .await
            .unwrap()
            .iter()
            .any(|e| e.sequence == start),
        "a new write's frame is not filtered"
    );
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a12_under_batched_a_wal_cut_to_its_last_fsync_never_reuses_a_sequence() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::Batched).expect("store-mode open");
    let records: Vec<(String, RecordValue)> = keys_in(P, 5)
        .into_iter()
        .enumerate()
        .map(|(i, key)| (key, lww("v", i as u64 + 1)))
        .collect();
    for (i, (key, value)) in records[..2].iter().enumerate() {
        node.append(P, key, value, i as u64 + 1).await;
    }
    node.wal.sync().await.unwrap();
    let segments = segment_files(&root.wal_dir(), P);
    assert_eq!(segments.len(), 1);
    let segment = segments.into_values().next().unwrap();
    let synced_len = std::fs::metadata(&segment).unwrap().len();
    for (i, (key, value)) in records.iter().enumerate().skip(2) {
        node.append(P, key, value, i as u64 + 1).await;
    }
    node.commit(&records, &[(P, 5)]).await.expect("commit");
    node.wal.sync().await.unwrap();
    node.close().await;

    // The power loss: the active segment ends at its last fsync, below the
    // stored row.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&segment)
        .unwrap()
        .set_len(synced_len)
        .unwrap();
    assert_eq!(frames_on_disk(&root.wal_dir(), P), BTreeSet::from([1, 2]));

    let engine = open_engine(&root);
    let mark = engine.mark();
    let node =
        Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::Batched).expect("reopen");
    drop(engine);
    node.recover().await.expect("recovery");
    assert!(
        node.engine
            .writes_since(mark)
            .iter()
            .all(|w| w.record_keys().is_empty()),
        "recovery replayed a frame at or below the stored row"
    );
    let start = node.sequence_start().await;
    assert!(
        start > 5,
        "the seed {start} reuses a sequence at or below the row 5"
    );
    node.append(P, &records[0].0, &lww("new", 9), start).await;
    assert!(
        node.wal
            .unapplied(P)
            .await
            .unwrap()
            .iter()
            .any(|e| e.sequence == start),
        "a new write's frame is not filtered"
    );
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a13_a_frame_at_or_below_the_engine_watermark_refuses_recovery() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let records: Vec<(String, RecordValue)> = keys_in(P, 3)
        .into_iter()
        .enumerate()
        .map(|(i, key)| (key, lww("v", i as u64 + 1)))
        .collect();
    for (i, (key, value)) in records.iter().enumerate() {
        node.append(P, key, value, i as u64 + 1).await;
    }
    node.commit(&records, &[(P, 3)]).await.expect("commit");
    node.close().await;

    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
    node.wal.test_force_unapplied_floor(P, Some(1));
    let mark = node.engine.mark();
    let error = format!(
        "{:#}",
        node.recover()
            .await
            .expect_err("a frame at or below the watermark refuses recovery")
    );
    assert!(
        error.contains(&format!(
            "partition {P} holds frame 2 at or below its applied watermark 3"
        )),
        "the refusal names the partition and the frame: {error}"
    );
    assert!(
        node.engine.writes_since(mark).is_empty(),
        "nothing is replayed: {:?}",
        node.engine.writes_since(mark)
    );
    node.wal.test_force_unapplied_floor(P, None);
    node.close().await;
}

// ---------------------------------------------------------------------------
// Helpers of the tests below
// ---------------------------------------------------------------------------

/// The partitions that have at least one segment file in `wal_dir`.
fn segment_partitions(wal_dir: &Path) -> BTreeSet<u32> {
    std::fs::read_dir(wal_dir)
        .expect("read the WAL dir")
        .filter_map(|e| {
            let name = e.expect("dir entry").file_name();
            parse_segment_filename(&name.to_string_lossy()).map(|(p, _)| p)
        })
        .collect()
}

/// The size of every segment file of `partition`, summed.
fn segment_bytes(wal_dir: &Path, partition: u32) -> u64 {
    segment_files(wal_dir, partition)
        .values()
        .map(|path| std::fs::metadata(path).expect("segment metadata").len())
        .sum()
}

/// The value of the series `series` (name and labels, as rendered) on a
/// scrape, if it is rendered.
fn series_value(rendered: &str, series: &str) -> Option<f64> {
    rendered.lines().find_map(|line| {
        let (name, value) = line.rsplit_once(' ')?;
        (name == series).then(|| value.parse::<f64>().expect("a numeric sample"))
    })
}

fn retained_series(partition: u32) -> String {
    format!("topgun_wal_retained_bytes{{partition=\"{partition}\"}}")
}

const RETAINED_TOTAL: &str = "topgun_wal_retained_bytes_total";

/// Runs `body` to completion on a current-thread runtime.
fn block_on<T>(body: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
        .block_on(body)
}

/// The line write-behind logs at a clean stop once its rollback hints are
/// written.
const HINTS_LINE: &str = "write-behind shutdown: rollback hints written";

/// One log event: its level and its fields as text.
#[derive(Debug, Clone)]
struct CapturedEvent {
    level: tracing::Level,
    fields: BTreeMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

struct FieldCollector<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldCollector<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

struct EventCapture(Arc<Mutex<Vec<CapturedEvent>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldCollector(&mut fields));
        self.0.lock().push(CapturedEvent {
            level: *event.metadata().level(),
            fields,
        });
    }
}

/// Runs `body` on a current-thread runtime and returns its output with every
/// event logged meanwhile.
///
/// The subscriber is scoped to this thread, never installed globally, as the
/// test binary is shared. One thread is what makes the capture complete: the
/// store's background tasks run on it too.
fn run_captured<T>(body: impl std::future::Future<Output = T>) -> (T, Vec<CapturedEvent>) {
    use tracing_subscriber::layer::SubscriberExt;

    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    let output = tracing::subscriber::with_default(subscriber, || block_on(body));
    let events = events.lock().clone();
    (output, events)
}

/// The rollback-hint lines among `events`.
fn hint_lines(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
    events
        .iter()
        .filter(|event| event.field("message") == Some(HINTS_LINE))
        .collect()
}

/// Writes, at `path`, a store file in the table-per-map format of earlier
/// binaries: one table `map__{MAP}` holding `records`.
fn write_table_per_map_file(path: &Path, records: &[(&str, RecordValue)]) {
    let db = redb::Database::create(path).expect("legacy create");
    let txn = db.begin_write().expect("begin_write");
    {
        let table_name = format!("map__{MAP}");
        let mut table = txn
            .open_table(redb::TableDefinition::<&str, &[u8]>::new(&table_name))
            .expect("open the legacy table");
        for (key, value) in records {
            let bytes = rmp_serde::to_vec_named(value).expect("encode");
            table.insert(*key, bytes.as_slice()).expect("insert");
        }
    }
    txn.commit().expect("commit");
}

// ---------------------------------------------------------------------------
// Main tests, continued
// ---------------------------------------------------------------------------

/// Appends a `Store` frame for each of `records` to P, at sequences 1, 2, ...,
/// and stores none of them.
async fn a14_setup(root: &Root, records: &[(String, RecordValue)]) {
    let node = Node::open(root, WalFsyncPolicy::PerOp).expect("store-mode open");
    for (i, (key, value)) in records.iter().enumerate() {
        node.append(P, key, value, i as u64 + 1).await;
    }
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a14_recovery_commits_its_frontier_row_and_a_second_start_replays_nothing() {
    const TEST: &str = "a14_recovery_commits_its_frontier_row_and_a_second_start_replays_nothing";
    let records: Vec<(String, RecordValue)> = keys_in(P, 3)
        .into_iter()
        .enumerate()
        .map(|(i, key)| {
            let value = lww(&key, i as u64 + 1);
            (key, value)
        })
        .collect();
    if let Some((scenario, root)) = child_spec() {
        // Recovery stopped at its frontier commit: the window is replayed,
        // the row that would close it is not written.
        assert_eq!(scenario, "replay-then-kill");
        let engine = open_engine(&root);
        let node = Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp)
            .expect("store-mode open");
        engine.abort_at_first_row_write(AbortSide::Before);
        node.recover().await.expect("recovery");
        println!("{CHILD_RETURNED}");
        return;
    }

    // Recovery replays the window in sequence order, then commits the
    // frontier as one row of its own.
    let root = Root::new();
    a14_setup(&root, &records).await;
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
    let mark = node.engine.mark();
    node.recover().await.expect("recovery");
    let writes = node.engine.writes_since(mark);
    let with_rows: Vec<usize> = (0..writes.len())
        .filter(|i| !writes[*i].rows().is_empty())
        .collect();
    assert_eq!(with_rows.len(), 1, "one frontier commit: {writes:?}");
    let frontier = &writes[with_rows[0]];
    assert!(
        frontier.ok && frontier.sync,
        "the frontier commit is durable"
    );
    assert_eq!(frontier.rows(), vec![(P, 3)]);
    assert!(
        frontier.record_keys().is_empty(),
        "the frontier commit holds no record"
    );
    let mut previous = None;
    for (key, value) in &records {
        let record = record_key(&node.engine, key);
        let at = writes
            .iter()
            .position(|w| w.touches(&record))
            .unwrap_or_else(|| panic!("{key} is replayed"));
        assert!(at < with_rows[0], "{key} is replayed before the frontier");
        assert!(previous < Some(at), "the window replays in sequence order");
        previous = Some(at);
        assert_eq!(
            node.store.load(MAP, key).await.unwrap().as_ref(),
            Some(value)
        );
    }
    assert_eq!(node.engine.row(P), Some(3));
    assert_eq!(node.wal.durable_applied(P), Some(3));
    node.close().await;

    // The second start: the row covers every frame, so nothing is replayed
    // and nothing committed.
    let engine = open_engine(&root);
    let mark = engine.mark();
    let node = Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp).expect("reopen");
    drop(engine);
    assert!(
        node.wal.unapplied(P).await.unwrap().is_empty(),
        "no frame above the row"
    );
    node.recover().await.expect("recovery");
    assert!(
        node.engine.writes_since(mark).is_empty(),
        "a second start replays and commits nothing: {:?}",
        node.engine.writes_since(mark)
    );
    for (key, value) in &records {
        assert_eq!(
            node.store.load(MAP, key).await.unwrap().as_ref(),
            Some(value)
        );
    }
    node.close().await;

    // Killed before the frontier commit: the row is absent, so the next
    // start replays the whole window again, in order, and then commits it.
    let root = Root::new();
    a14_setup(&root, &records).await;
    run_child(TEST, "replay-then-kill", &root, None).assert_aborted("replay-then-kill");
    assert_eq!(
        rows_of(&dump_at(&root)).get(&P),
        None,
        "the frontier row was not written"
    );
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
    let mark = node.engine.mark();
    node.recover().await.expect("recovery");
    let writes = node.engine.writes_since(mark);
    let mut previous = None;
    for (key, value) in &records {
        let record = record_key(&node.engine, key);
        let at = writes
            .iter()
            .position(|w| w.touches(&record))
            .unwrap_or_else(|| panic!("{key} is replayed again"));
        assert!(previous < Some(at), "the window replays in sequence order");
        previous = Some(at);
        assert_eq!(
            node.store.load(MAP, key).await.unwrap().as_ref(),
            Some(value)
        );
    }
    assert_eq!(node.engine.row(P), Some(3));
    node.close().await;
}

/// Engine rows, key and value, as written raw.
type RawRows = Vec<(Vec<u8>, Vec<u8>)>;

/// Writes a formatted store at `root` with `rows` put into it raw.
fn a15_store_with(root: &Root, rows: &[(Vec<u8>, Vec<u8>)]) {
    std::fs::create_dir_all(root.data_dir()).expect("data dir");
    let engine = Arc::new(RedbKvEngine::open(&root.engine_path()).expect("engine"));
    drop(EngineMapDataStore::open(Arc::clone(&engine)).expect("a fresh store is formatted"));
    let mut batch = WriteBatch::new();
    for (key, value) in rows {
        batch.put(key.clone(), value.clone());
    }
    engine
        .write(batch, WriteOptions { sync: true })
        .expect("raw write");
    assert_eq!(
        Arc::strong_count(&engine),
        1,
        "the store released the engine"
    );
}

/// The open of `root`'s store refuses with `phrase` and writes nothing.
fn a15_assert_refused(root: &Root, case: &str, phrase: &str) {
    let engine = open_engine(root);
    let before = engine.dump();
    let error = match EngineMapDataStore::open(Arc::clone(&engine)) {
        Ok(_) => panic!("{case}: the open was to be refused"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        error.contains("store format: refusing to open:") && error.contains(phrase),
        "{case}: the refusal does not say {phrase:?}: {error}"
    );
    assert!(
        engine.writes_since(0).is_empty(),
        "{case}: a refused open wrote"
    );
    assert_eq!(engine.dump(), before, "{case}: the engine content changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a15_the_engine_refuses_malformed_partition_metadata() {
    let marker = || (meta_key(META_WAL_WATERMARK), vec![1u8]);
    let pending = || (meta_key(META_WAL_IMPORT_PENDING), vec![1u8]);
    let row = |p: u32, v: u64| (row_key(p), v.to_be_bytes().to_vec());
    let with_partition_tail = |p: u32, tail: &[u8]| {
        let mut key = vec![KS_PARTITION_META];
        key.extend_from_slice(&p.to_be_bytes());
        key.extend_from_slice(tail);
        (key, 5u64.to_be_bytes().to_vec())
    };
    let reserved_0x20 = "a key in the reserved keyspace 0x20";

    let refused: Vec<(&str, RawRows, &str)> = vec![
        (
            "a marker of another value",
            vec![(meta_key(META_WAL_WATERMARK), vec![2])],
            "",
        ),
        (
            "an import-pending mark of another value",
            vec![(meta_key(META_WAL_IMPORT_PENDING), vec![2])],
            "",
        ),
        (
            "both marks",
            vec![marker(), pending()],
            "an import-pending mark beside the import marker",
        ),
        (
            "rows without the marker",
            vec![row(P, 5)],
            "applied watermark rows without the import marker",
        ),
        (
            "a row key one byte short",
            vec![marker(), with_partition_tail(P, b"applie")],
            reserved_0x20,
        ),
        (
            "a row key with another tail",
            vec![marker(), with_partition_tail(P, b"appliex")],
            reserved_0x20,
        ),
        (
            "a row of a partition out of range",
            vec![marker(), row(PARTITION_COUNT, 5)],
            reserved_0x20,
        ),
        (
            "a row value of 7 bytes",
            vec![marker(), (row_key(P), vec![0; 7])],
            "",
        ),
        ("a key in 0x1f", vec![(vec![0x1f, 1], vec![1])], ""),
        ("a key in 0x21", vec![(vec![0x21, 1], vec![1])], ""),
    ];
    for (case, rows, phrase) in refused {
        let root = Root::new();
        a15_store_with(&root, &rows);
        a15_assert_refused(&root, case, phrase);
    }

    // Rows with the marker, the import-pending mark alone, and neither open.
    for (rows, imported, import_pending, by_partition) in [
        (
            vec![marker(), row(P, 5), row(Q, 3)],
            true,
            false,
            vec![(P, 5), (Q, 3)],
        ),
        (vec![pending()], false, true, vec![]),
        (vec![], false, false, vec![]),
    ] {
        let root = Root::new();
        a15_store_with(&root, &rows);
        let store = EngineMapDataStore::open(open_engine(&root)).expect("the store opens");
        let held = store
            .applied_watermarks()
            .expect("read")
            .expect("the store keeps watermarks");
        assert_eq!(held.imported, imported);
        assert_eq!(held.import_pending, import_pending);
        assert_eq!(held.by_partition.len(), PARTITION_COUNT as usize);
        let mut expected = vec![0u64; PARTITION_COUNT as usize];
        for (p, v) in by_partition {
            expected[p as usize] = v;
        }
        assert_eq!(held.by_partition, expected);
    }

    // A store whose watermark came through a WAL open and a commit reopens:
    // the import wrote its marker with the rows.
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let key = keys_in(P, 1).remove(0);
    node.append(P, &key, &lww("v", 1), 1).await;
    node.commit(&[(key, lww("v", 1))], &[(P, 1)])
        .await
        .expect("commit");
    node.close().await;
    let engine = open_engine(&root);
    assert_eq!(engine.marker(), Some(vec![1]), "the import marker");
    assert!(
        EngineMapDataStore::open(engine).is_ok(),
        "a store with rows and the marker reopens"
    );
}

#[tokio::test(flavor = "multi_thread")]
// The gauges hold byte counts, integers far below 2^53: exact as f64.
#[allow(clippy::float_cmp)]
async fn a16_the_retained_wal_bytes_gauges_render_from_the_first_scrape_and_follow_the_tick() {
    const TEST: &str =
        "a16_the_retained_wal_bytes_gauges_render_from_the_first_scrape_and_follow_the_tick";
    if let Some((scenario, root)) = child_spec() {
        // A process of its own: the gauges are process-wide, and no other
        // WAL may have set them before this one's first scrape.
        assert_eq!(scenario, "scrape");
        let observability = crate::service::middleware::observability::init_observability();
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");

        let first = observability.render_metrics();
        for p in 0..PARTITION_COUNT {
            assert_eq!(
                series_value(&first, &retained_series(p)),
                Some(0.0),
                "partition {p} on the first scrape"
            );
        }
        assert_eq!(series_value(&first, RETAINED_TOTAL), Some(0.0));

        let records: Vec<(String, RecordValue)> = keys_in(P, 3)
            .into_iter()
            .enumerate()
            .map(|(i, key)| (key, lww("v", i as u64 + 1)))
            .collect();
        for (i, (key, value)) in records.iter().enumerate() {
            node.append(P, key, value, i as u64 + 1).await;
        }
        let assert_follows = |what: &str| {
            let scrape = observability.render_metrics();
            let mut sum = 0.0;
            for p in 0..PARTITION_COUNT {
                let value = series_value(&scrape, &retained_series(p))
                    .unwrap_or_else(|| panic!("{what}: partition {p} is rendered"));
                #[allow(clippy::cast_precision_loss)]
                let on_disk = segment_bytes(&root.wal_dir(), p) as f64;
                assert_eq!(value, on_disk, "{what}: partition {p}");
                sum += value;
            }
            assert_eq!(
                series_value(&scrape, RETAINED_TOTAL),
                Some(sum),
                "{what}: the total"
            );
            series_value(&scrape, &retained_series(P)).unwrap_or_default()
        };

        node.wal.test_collect_now().await;
        assert!(
            assert_follows("after a tick over appended frames") > 0.0,
            "the appended frames are retained"
        );

        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
        assert!(assert_follows("after the seal") > 0.0);

        node.commit(&records, &[(P, 3)]).await.expect("commit");
        node.wal.test_collect_now().await;
        assert_eq!(
            assert_follows("after the collection"),
            0.0,
            "nothing of P is retained once collected"
        );
        node.close().await;
        println!("{CHILD_RETURNED}");
        return;
    }

    let root = Root::new();
    let run = run_child(TEST, "scrape", &root, None);
    assert!(
        run.status.success() && run.stdout.contains(CHILD_RETURNED),
        "the scrape child failed: {:?}\n--- stdout\n{}\n--- stderr\n{}",
        run.status,
        run.stdout,
        run.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a17_a_sidecar_mode_wal_refuses_a_directory_kept_by_a_store() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    node.append(P, &keys_in(P, 1).remove(0), &lww("v", 1), 1)
        .await;
    node.close().await;
    assert!(read_collected(&root.wal_dir()).is_some());

    let before = fingerprint(&root.wal_dir());
    let error = match WalWriter::new(root.wal_dir(), WalFsyncPolicy::PerOp) {
        Ok(_) => panic!("a sidecar-mode open of a directory kept by a store succeeded"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        error.contains(&format!(
            "WAL directory {} is kept by a store that holds its applied watermark; open it over \
             that store",
            root.wal_dir().display()
        )),
        "the refusal: {error}"
    );
    assert_eq!(
        fingerprint(&root.wal_dir()),
        before,
        "the refusal changed the WAL directory"
    );

    // A directory no store keeps still opens in sidecar mode.
    let fresh = Root::new();
    let wal = WalWriter::new(fresh.wal_dir(), WalFsyncPolicy::PerOp).expect("sidecar-mode open");
    assert!(!wal.watermark_in_store());
}

#[tokio::test(flavor = "multi_thread")]
async fn a18_a_kill_inside_or_right_after_a_commit_loses_no_acked_write() {
    const TEST: &str = "a18_a_kill_inside_or_right_after_a_commit_loses_no_acked_write";
    let keys = keys_in(P, 4);
    // Two such values exceed a pass's 8 MiB value limit, so each pass takes
    // one entry and the three acknowledged entries are three commits.
    let filler = "x".repeat(5 << 20);
    let value = |i: usize| lww(&format!("{}-{filler}", keys[i]), i as u64 + 1);
    if let Some((scenario, root)) = child_spec() {
        let (n, side) = scenario.split_once('-').expect("scenario");
        let n: usize = n.parse().expect("write number");
        let side = match side {
            "before" => AbortSide::Before,
            "after" => AbortSide::After,
            other => panic!("unknown side {other}"),
        };
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let wb = node.write_behind(wb_config()).await;
        wb.add(MAP, &keys[0], &value(0), 0, 0).await.unwrap();
        flush_idle(&wb).await;
        // Acknowledged: under per_op each frame is fsynced before `add`
        // returns.
        for (i, key) in keys.iter().enumerate().skip(1) {
            wb.add(MAP, key, &value(i), 0, 0).await.unwrap();
        }
        node.engine.abort_in(n, side);
        flush_idle(&wb).await;
        println!("{CHILD_RETURNED}");
        return;
    }

    for scenario in ["1-before", "1-after", "2-before", "2-after"] {
        let root = Root::new();
        run_child(TEST, scenario, &root, None).assert_aborted(scenario);
        let crashed = rows_of(&dump_at(&root))
            .get(&P)
            .copied()
            .expect("the first commit's row is durable");

        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("the restart");
        let window = node.wal.unapplied(P).await.unwrap();
        assert!(
            window.iter().all(|e| e.sequence > crashed),
            "{scenario}: a frame at or below the durable row {crashed} is handed to replay"
        );
        let mark = node.engine.mark();
        node.recover().await.expect("recovery");
        let writes = node.engine.writes_since(mark);
        let replayed: BTreeSet<Vec<u8>> =
            writes.iter().flat_map(WriteRecord::record_keys).collect();
        let expected: BTreeSet<Vec<u8>> = window
            .iter()
            .map(|e| record_key(&node.engine, &e.key))
            .collect();
        assert_eq!(
            replayed, expected,
            "{scenario}: recovery replays exactly the frames above the durable row"
        );
        for write in &writes {
            for (p, v) in write.rows() {
                assert!(
                    p != P || v >= crashed,
                    "{scenario}: the row of P went from {crashed} to {v}"
                );
            }
        }
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                node.store.load(MAP, key).await.unwrap(),
                Some(value(i)),
                "{scenario}: the acknowledged write of {key} after the restart"
            );
        }
        let highest = frames_on_disk(&root.wal_dir(), P).into_iter().max();
        assert_eq!(node.engine.row(P), highest, "{scenario}: the frontier row");
        assert!(node.engine.row(P) >= Some(crashed));
        node.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a19_a_wal_directory_older_than_the_data_directory_replays_nothing_at_or_below_the_watermark_and_reuses_no_sequence(
) {
    let root = Root::new();
    let p_keys = keys_in(P, 3);
    let q_key = keys_in(Q, 1).remove(0);
    let mut acked: BTreeMap<String, RecordValue> = BTreeMap::new();

    // First run, stopped cleanly; its WAL directory is copied.
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let wb = node.write_behind(wb_config()).await;
    for (key, millis) in [(&p_keys[0], 1), (&q_key, 2)] {
        let value = lww(&format!("{key}-1"), millis);
        wb.add(MAP, key, &value, 0, 0).await.unwrap();
        acked.insert(key.clone(), value);
    }
    flush_idle(&wb).await;
    stop_write_behind(wb).await;
    node.close().await;
    let old_wal = root.path.join("wal-copy");
    copy_dir(&root.wal_dir(), &old_wal);

    // Second run: a key rewritten, new keys, stopped cleanly.
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("reopen");
    node.recover().await.expect("recovery");
    let wb = node.write_behind(wb_config()).await;
    for (key, millis) in [(&p_keys[0], 10), (&p_keys[1], 11), (&p_keys[2], 12)] {
        let value = lww(&format!("{key}-2"), millis);
        wb.add(MAP, key, &value, 0, 0).await.unwrap();
        acked.insert(key.clone(), value);
    }
    flush_idle(&wb).await;
    stop_write_behind(wb).await;
    let rows = rows_of(&node.engine.dump());
    node.close().await;
    let highest_row = *rows.values().max().expect("rows");

    // The older WAL copy put back beside the newer data directory.
    std::fs::remove_dir_all(root.wal_dir()).unwrap();
    copy_dir(&old_wal, &root.wal_dir());

    let engine = open_engine(&root);
    let mark = engine.mark();
    let node = Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp)
        .expect("a WAL directory older than the data directory is not refused");
    drop(engine);
    for partition in [P, Q] {
        assert!(
            node.wal.unapplied(partition).await.unwrap().is_empty(),
            "every frame of the older copy is at or below the row of {partition}"
        );
    }
    node.recover().await.expect("recovery");
    assert!(
        node.engine
            .writes_since(mark)
            .iter()
            .all(|w| w.record_keys().is_empty()),
        "recovery replayed a record from the older copy: {:?}",
        node.engine.writes_since(mark)
    );
    for (key, value) in &acked {
        assert_eq!(
            node.store.load(MAP, key).await.unwrap().as_ref(),
            Some(value),
            "{key} is served as acknowledged before the second stop"
        );
    }
    assert_eq!(rows_of(&node.engine.dump()), rows, "no row changed");

    let start = node.sequence_start().await;
    assert!(
        start > highest_row,
        "the seed {start} reuses a sequence at or below the row {highest_row}"
    );
    let key = keys_in(P, 4).remove(3);
    node.append(P, &key, &lww("new", 99), start).await;
    assert!(
        node.wal
            .unapplied(P)
            .await
            .unwrap()
            .iter()
            .any(|e| e.sequence == start),
        "a new write's frame is not filtered"
    );
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a20_a_commit_carrying_a_watermark_below_the_held_row_is_refused_and_writes_nothing() {
    let root = Root::new();
    let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let keys = keys_in(P, 4);
    let commit = |i: usize, sequence: u64| {
        let value = lww(&keys[i], i as u64 + 1);
        let store = Arc::clone(&node.store);
        let key = keys[i].clone();
        async move {
            store
                .commit(&FlushCommit {
                    writes: vec![StoreWrite::Put {
                        map: MAP,
                        key: &key,
                        value: PutValue::Value(&value),
                        expiration_time: 0,
                        store_time: 0,
                    }],
                    applied: vec![(P, sequence)],
                })
                .await
        }
    };

    commit(0, 10).await.expect("row 10");
    assert_eq!(node.engine.row(P), Some(10));

    let before = node.engine.dump();
    let mark = node.engine.mark();
    let error = format!(
        "{:#}",
        commit(1, 9)
            .await
            .expect_err("a commit carrying 9 below the held 10 is refused")
    );
    assert!(
        error.contains(&format!(
            "applied watermark regression refused: WAL partition {P} holds 10, the commit \
             carries 9"
        )),
        "the refusal names the partition, the held row and the proposed one: {error}"
    );
    assert!(
        node.engine.writes_since(mark).is_empty(),
        "a refused commit reaches no engine write"
    );
    assert_eq!(
        node.engine.dump(),
        before,
        "a refused commit writes nothing"
    );

    commit(2, 10).await.expect("a re-commit of the held row");
    assert_eq!(node.engine.row(P), Some(10));
    commit(3, 11).await.expect("a higher row");
    assert_eq!(node.engine.row(P), Some(11));
    node.close().await;
}

/// Two sealed segments of P, `[1, 2]` and `[3, 4]`, beside an empty active
/// one, with `[1, 2]` stored and its row `(P, 2)` committed.
async fn a21_setup(root: &Root) -> Node {
    let node = Node::open(root, WalFsyncPolicy::PerOp).expect("store-mode open");
    let records: Vec<(String, RecordValue)> = keys_in(P, 4)
        .into_iter()
        .enumerate()
        .map(|(i, key)| (key, lww("v", i as u64 + 1)))
        .collect();
    for (pair, first) in records.chunks(2).zip([1u64, 3]) {
        node.append(P, &pair[0].0, &pair[0].1, first).await;
        node.append(P, &pair[1].0, &pair[1].1, first + 1).await;
        node.wal.test_advance_clock(SEAL_AGE);
        node.wal.test_collect_now().await;
    }
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([1, 2, 3, 4])
    );
    node.commit(&records[..2], &[(P, 2)]).await.expect("commit");
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn a21_a_tick_whose_watermark_read_fails_unlinks_nothing_and_the_next_tick_collects() {
    let root = Root::new();
    let node = a21_setup(&root).await;
    let before = fingerprint(&root.wal_dir());
    let skipped = gc_skipped("watermark_read_failed");

    node.engine.fail_snapshot_in(1);
    node.wal.test_collect_now().await;
    assert_eq!(
        fingerprint(&root.wal_dir()),
        before,
        "a tick whose read failed unlinked a segment or wrote wal-collected"
    );
    assert!(
        gc_skipped("watermark_read_failed") > skipped,
        "the failed read is counted"
    );

    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([3, 4]),
        "the next tick collects"
    );
    assert_eq!(
        read_collected(&root.wal_dir()).expect("wal-collected")[P as usize],
        2
    );
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a22_after_finish_no_tick_unlinks_and_the_tick_task_ends() {
    let root = Root::new();
    let node = a21_setup(&root).await;
    assert!(node.wal.test_collection_tick_running());
    let before = fingerprint(&root.wal_dir());

    assert_eq!(
        node.wal.finish(false).await.expect("finish"),
        0,
        "no hint is written"
    );
    node.wal.test_wake_collection_tick();
    wait_for("the tick task to end after finish", || {
        !node.wal.test_collection_tick_running()
    })
    .await;
    node.wal.test_collect_now().await;
    assert_eq!(
        frames_on_disk(&root.wal_dir(), P),
        BTreeSet::from([1, 2, 3, 4]),
        "no tick collects after finish"
    );
    assert_eq!(
        fingerprint(&root.wal_dir()),
        before,
        "the WAL directory is unchanged"
    );
    node.close().await;

    // Without finish, the tick task ends once the store is gone.
    let root = Root::new();
    let Node { engine, store, wal } =
        Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
    assert!(wal.test_collection_tick_running());
    drop(store);
    wal.test_wake_collection_tick();
    wait_for("the tick task to end with its store", || {
        !wal.test_collection_tick_running()
    })
    .await;
    drop(wal);
    drop(engine);
}

/// One entry in each of P, Q and R, stored.
async fn a23_write(wb: &WriteBehindDataStore) {
    for partition in [P, Q, R] {
        let key = keys_in(partition, 1).remove(0);
        wb.add(MAP, &key, &lww(&key, 1), 0, 0).await.unwrap();
    }
    flush_idle(wb).await;
}

#[test]
fn a23_a_clean_stop_writes_one_rollback_hint_per_partition_equal_to_the_engine_row() {
    const TEST: &str =
        "a23_a_clean_stop_writes_one_rollback_hint_per_partition_equal_to_the_engine_row";
    if let Some((scenario, root)) = child_spec() {
        assert_eq!(scenario, "stop");
        block_on(async {
            let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
            let wb = node.write_behind(wb_config()).await;
            a23_write(&wb).await;
            wb.hard_flush().await.expect("hard_flush");
        });
        println!("{CHILD_RETURNED}");
        return;
    }

    // A clean stop: a hint for every partition with a segment or a row, each
    // the engine row, and one line saying so.
    let root = Root::new();
    let (rows, events) = run_captured(async {
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let wb = node.write_behind(wb_config()).await;
        a23_write(&wb).await;
        stop_write_behind(wb).await;
        let rows = rows_of(&node.engine.dump());
        node.close().await;
        rows
    });
    let mut hinted = segment_partitions(&root.wal_dir());
    hinted.extend(rows.keys());
    assert!(hinted.is_superset(&BTreeSet::from([P, Q, R])));
    assert_eq!(
        sidecar_files(&root.wal_dir()),
        hinted
            .iter()
            .map(|p| format!("partition-{p:03}.applied"))
            .collect::<BTreeSet<_>>(),
        "one hint per partition with a segment or a row, and no temporary file"
    );
    for p in &hinted {
        let hint = std::fs::read(sidecar_path(&root.wal_dir(), *p)).expect("the hint");
        assert_eq!(hint.len(), 8, "the hint of {p} is 8 bytes");
        assert_eq!(
            decode_u64(&hint),
            rows.get(p).copied().unwrap_or(0),
            "the hint of {p} is its engine row"
        );
    }
    let lines = hint_lines(&events);
    assert_eq!(lines.len(), 1, "one rollback-hints line: {events:?}");
    assert_eq!(lines[0].level, tracing::Level::INFO);
    assert_eq!(lines[0].field("drained"), Some("true"));
    assert_eq!(lines[0].field("pending"), Some("0"));
    assert_eq!(
        lines[0].field("partitions"),
        Some(hinted.len().to_string().as_str())
    );

    // A stop after a background task panicked writes no hint.
    let root = Root::new();
    let (shutdown, events) = run_captured(async {
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let config = WriteBehindConfig {
            wal_watermark_stall_bound_ms: 1_000,
            ..wb_config()
        };
        let wb = node.write_behind(config).await;
        wb.test_set_classifier_hook(Arc::new(|_partition| {
            panic!("injected stall watchdog panic");
        }));
        let key = keys_in(P, 1).remove(0);
        wb.add(MAP, &key, &lww(&key, 1), 0, 0).await.unwrap();
        let failure = wb.background_failure();
        wait_for("the stall watchdog to panic", || failure.borrow().is_some()).await;
        let shutdown = wb.hard_flush().await;
        drop(wb);
        node.close().await;
        shutdown
    });
    assert!(shutdown.is_err(), "the stop reports the failure");
    assert!(
        sidecar_files(&root.wal_dir()).is_empty(),
        "a stop after a recorded failure wrote a hint: {:?}",
        sidecar_files(&root.wal_dir())
    );
    assert!(hint_lines(&events).is_empty());

    // Killed inside the hint loop: what is left is deleted by the next start
    // and never read, not even a hint the import would refuse.
    let root = Root::new();
    let point = TestAbortPoint::FinishInsideHints;
    run_child(TEST, "stop", &root, Some(point)).assert_aborted(point.name());
    let left = sidecar_files(&root.wal_dir());
    let renamed = left.iter().filter(|n| n.ends_with(".applied")).count();
    assert!(
        !left.is_empty() && renamed < 3,
        "the kill left part of the hints: {left:?}"
    );
    std::fs::write(sidecar_path(&root.wal_dir(), U), [0u8; 7]).unwrap();
    block_on(async {
        let engine = open_engine(&root);
        let rows = rows_of(&engine.dump());
        let mark = engine.mark();
        let node = Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp)
            .expect("the next start reads no hint");
        drop(engine);
        assert!(
            node.engine.writes_since(mark).is_empty(),
            "the next start imported a hint"
        );
        assert!(
            sidecar_files(&root.wal_dir()).is_empty(),
            "the next start deleted every hint: {:?}",
            sidecar_files(&root.wal_dir())
        );
        assert_eq!(rows_of(&node.engine.dump()), rows);
        for (p, row) in &rows {
            assert_eq!(node.wal.durable_applied(*p), Some(*row));
        }
        node.close().await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a24_a_store_just_migrated_beside_a_used_wal_directory_imports_the_sidecars_once() {
    const TEST: &str =
        "a24_a_store_just_migrated_beside_a_used_wal_directory_imports_the_sidecars_once";
    if let Some((scenario, _root)) = child_spec() {
        let location = RedbDataStore::location_from_env().expect("location");
        let store = RedbDataStore::open_location(&location).expect("the legacy file migrates");
        match scenario.as_str() {
            "migrate" => drop(store),
            // Stopped between the migration and the WAL open.
            "migrate-then-kill" => std::process::abort(),
            other => panic!("unknown scenario {other}"),
        }
        println!("{CHILD_RETURNED}");
        return;
    }

    let legacy_value = lww("legacy", 1);
    for scenario in ["migrate", "migrate-then-kill"] {
        let root = Root::new();
        // A store-mode run: the WAL directory holds wal-collected.
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let key = keys_in(P, 1).remove(0);
        node.append(P, &key, &lww("v", 1), 1).await;
        node.commit(&[(key, lww("v", 1))], &[(P, 1)])
            .await
            .expect("commit");
        node.close().await;
        assert!(read_collected(&root.wal_dir()).is_some());

        // An older binary's store file, and the sidecars it wrote beside it.
        std::fs::remove_dir_all(root.data_dir()).unwrap();
        write_table_per_map_file(
            &root.path.join("topgun.redb"),
            &[("legacy", legacy_value.clone())],
        );
        std::fs::write(sidecar_path(&root.wal_dir(), P), 5u64.to_be_bytes()).unwrap();
        std::fs::write(sidecar_path(&root.wal_dir(), Q), 3u64.to_be_bytes()).unwrap();

        let run = run_child(TEST, scenario, &root, None);
        if scenario == "migrate" {
            assert!(
                run.status.success() && run.stdout.contains(CHILD_RETURNED),
                "the migration child failed: {:?}\n{}",
                run.status,
                run.stderr
            );
        } else {
            run.assert_aborted(scenario);
        }
        let dump = dump_at(&root);
        assert_eq!(
            dump.get(&meta_key(META_WAL_IMPORT_PENDING)),
            Some(&vec![1]),
            "{scenario}: the migration left the import-pending mark"
        );
        assert_eq!(dump.get(&meta_key(META_WAL_WATERMARK)), None);

        let engine = open_engine(&root);
        let mark = engine.mark();
        let node = Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp)
            .expect("a just-migrated store beside a used WAL directory opens");
        drop(engine);
        let writes = node.engine.writes_since(mark);
        assert_eq!(writes.len(), 1, "{scenario}: one import batch: {writes:?}");
        let import = &writes[0];
        assert!(import.ok && import.sync);
        let mut imported = import.rows();
        imported.sort_unstable();
        assert_eq!(
            imported,
            vec![(P, 5), (Q, 3)],
            "{scenario}: the sidecars' values"
        );
        assert!(import.ops.iter().any(|op| matches!(
            op,
            BatchOp::Put { key, value } if *key == meta_key(META_WAL_WATERMARK) && *value == vec![1]
        )));
        assert!(import.ops.iter().any(|op| matches!(
            op,
            BatchOp::Delete { key } if *key == meta_key(META_WAL_IMPORT_PENDING)
        )));
        let dump = node.engine.dump();
        assert_eq!(rows_of(&dump), BTreeMap::from([(P, 5), (Q, 3)]));
        assert_eq!(dump.get(&meta_key(META_WAL_IMPORT_PENDING)), None);
        assert!(sidecar_files(&root.wal_dir()).is_empty());
        assert_eq!(
            node.store.load(MAP, "legacy").await.unwrap(),
            Some(legacy_value.clone())
        );
        node.close().await;

        // A second start imports nothing.
        std::fs::write(sidecar_path(&root.wal_dir(), P), 9u64.to_be_bytes()).unwrap();
        let engine = open_engine(&root);
        let mark = engine.mark();
        let node =
            Node::open_over(Arc::clone(&engine), &root, WalFsyncPolicy::PerOp).expect("reopen");
        drop(engine);
        assert!(
            node.engine.writes_since(mark).is_empty(),
            "{scenario}: a second start imported again"
        );
        assert_eq!(node.engine.row(P), Some(5));
        assert!(sidecar_files(&root.wal_dir()).is_empty());
        node.close().await;
    }
}

#[test]
fn a25_a_stop_whose_drain_timed_out_writes_hints_at_the_durable_watermark_and_logs_drained_false() {
    let config = WriteBehindConfig {
        shutdown_timeout_ms: 50,
        ..wb_config()
    };

    // Every commit takes 500 ms against a 50 ms drain deadline, and two of
    // the entries' values exceed a pass's 8 MiB value limit, so the drain's
    // first pass takes one entry and the drain stores at most one of the
    // three entries.
    let root = Root::new();
    let ((left, rows, unstored), events) = run_captured(async {
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let wb = node.write_behind(config.clone()).await;
        let keys = keys_in(P, 4);
        wb.add(MAP, &keys[0], &lww("warm", 1), 0, 0).await.unwrap();
        flush_idle(&wb).await;
        let filler = "x".repeat(5 << 20);
        let mut sequences = Vec::new();
        for (i, key) in keys[1..].iter().enumerate() {
            wb.add(
                MAP,
                key,
                &lww(&format!("{key}-{filler}"), i as u64 + 2),
                0,
                0,
            )
            .await
            .unwrap();
            sequences.push((key.clone(), wb.test_max_assigned_wal_sequence(P)));
        }
        node.engine.set_write_delay(Duration::from_millis(500));
        wb.hard_flush()
            .await
            .expect("a stop whose drain timed out still returns Ok");
        node.engine.set_write_delay(Duration::ZERO);
        let left = wb.pending_operation_count();
        let dump = node.engine.dump();
        let unstored: Vec<u64> = sequences
            .iter()
            .filter(|(key, _)| !dump.contains_key(&record_key(&node.engine, key)))
            .map(|(_, seq)| *seq)
            .collect();
        drop(wb);
        node.close().await;
        (left, rows_of(&dump), unstored)
    });
    assert!(left >= 1, "the drain left entries pending");
    assert_eq!(
        unstored.len() as u64,
        left,
        "the pending entries are unstored"
    );
    let hint = std::fs::read(sidecar_path(&root.wal_dir(), P)).expect("P's hint");
    assert_eq!(hint.len(), 8);
    let hint = decode_u64(&hint);
    assert_eq!(Some(&hint), rows.get(&P), "the hint is the engine row");
    for seq in &unstored {
        assert!(
            hint < *seq,
            "the hint {hint} is not below the pending sequence {seq}"
        );
    }
    let lines = hint_lines(&events);
    assert_eq!(lines.len(), 1, "one rollback-hints line: {events:?}");
    assert_eq!(lines[0].level, tracing::Level::INFO);
    assert_eq!(lines[0].field("drained"), Some("false"));
    assert_eq!(lines[0].field("pending"), Some(left.to_string().as_str()));

    // A stop with nothing pending says drained=true.
    let root = Root::new();
    let ((), events) = run_captured(async {
        let node = Node::open(&root, WalFsyncPolicy::PerOp).expect("store-mode open");
        let wb = node.write_behind(config.clone()).await;
        let key = keys_in(P, 1).remove(0);
        wb.add(MAP, &key, &lww(&key, 1), 0, 0).await.unwrap();
        flush_idle(&wb).await;
        stop_write_behind(wb).await;
        node.close().await;
    });
    let lines = hint_lines(&events);
    assert_eq!(lines.len(), 1, "one rollback-hints line: {events:?}");
    assert_eq!(lines[0].field("drained"), Some("true"));
    assert_eq!(lines[0].field("pending"), Some("0"));
}

/// The store-level sanity measurement: the time one flush pass takes to store
/// 2 000 new keys through write-behind with a `per_op` WAL. Run by hand, in
/// alternation with a base arm whose body differs only in the line that
/// constructs the WAL; prints one `A90_ELAPSED_MS=` line.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, run by hand against a base arm"]
async fn a90_one_entry_commits_cost_no_more_than_base() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn MapDataStore> =
        Arc::new(RedbDataStore::new(dir.path().join("topgun-data")).expect("store"));
    let wal_dir = dir.path().join("wal");
    let policy = WalFsyncPolicy::PerOp;
    let wal = WalWriter::open_over_store(wal_dir, policy, Arc::clone(&store)).expect("WAL");
    let sequence_start = wal.max_observed_sequence().await.expect("seed") + 1;
    let config = WriteBehindConfig {
        write_delay_ms: 0,
        flush_interval_ms: 600_000,
        capacity: 0,
        ..WriteBehindConfig::default()
    };
    let wb = WriteBehindDataStore::new_with_wal(
        store,
        config,
        Some(WalBootstrap {
            wal,
            sequence_start,
        }),
    );
    for i in 0..2_000u64 {
        let key = format!("k{i}");
        let value = RecordValue::Lww {
            value: Value::String(key.clone()),
            timestamp: Timestamp {
                millis: i + 1,
                counter: 0,
                node_id: "n".to_string(),
            },
        };
        wb.add("m", &key, &value, 0, 0).await.expect("add");
    }
    let started = Instant::now();
    wb.soft_flush().await.expect("soft_flush");
    while wb.pending_operation_count() > 0 {
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "the flush did not finish"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let elapsed = started.elapsed();
    println!("A90_ELAPSED_MS={:.3}", elapsed.as_secs_f64() * 1_000.0);
    wb.hard_flush().await.expect("hard_flush");
}
