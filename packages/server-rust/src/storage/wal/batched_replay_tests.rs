//! Tests of WAL replay at start in a store-mode WAL: frames are replayed in
//! batches, each one commit holding its frames' writes and the partition's
//! applied watermark row, and a batch reads the values it has prepared but
//! not yet committed (its own-batch overlay). Also the collection tick that
//! fails to unlink a segment, and the crash loop that kills a real child
//! process with `SIGKILL` under both fsync policies.
//!
//! The oracle of the replay tests is per-frame replay: `replay_entry` called
//! frame by frame over a store seeded the same way, which shares no batch
//! code with the path under test.

// Each test is one scenario read top to bottom — set up, replay or crash,
// check — and splitting one across helpers would hide the order its claim
// depends on.
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestRunner};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::store_mode_tests::{
    dump_at, lww, open_engine, rows_of, store_frame, wait_for, CountingEngine, Root, MAP,
};
use super::{OrDelta, Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery, WalWriter};
use crate::storage::datastores::engine_store::{EngineMapDataStore, KS_PARTITION_META, KS_PRIMARY};
use crate::storage::map_data_store::{
    FlushCommit, LeafSink, MapDataStore, PutValue, RecordRefused, ScanBatch, ScanCursor,
    StoreBrokenCause, StoreErrorClass, StoreWrite,
};
use crate::storage::record::{OrMapEntry, RecordValue};

// ---------------------------------------------------------------------------
// A store that refuses on purpose
// ---------------------------------------------------------------------------

/// How a [`FaultStore`] refuses a commit that holds a poisoned put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// `RecordRefused` naming the poisoned writes: the store takes the rest
    /// once they are left out.
    Record,
    /// The whole commit is refused, as an engine error with no class.
    Commit,
}

/// One `commit` call a [`FaultStore`] was given.
#[derive(Debug, Clone)]
struct CommitCall {
    /// The keys of the commit's writes, in order.
    keys: Vec<String>,
    /// The applied watermark rows it carried.
    applied: Vec<(u32, u64)>,
    /// Whether it returned `Ok`.
    ok: bool,
}

/// The error a [`FaultStore`] returns for a commit it treats as a broken
/// store; its `classify_error` answers `StoreBroken` for it.
#[derive(Debug)]
struct InjectedBroken;

impl std::fmt::Display for InjectedBroken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "injected: the store is broken")
    }
}

impl std::error::Error for InjectedBroken {}

/// A [`MapDataStore`] over a store-mode [`EngineMapDataStore`] that forwards
/// every call and can be made to refuse: a put whose value is poisoned (see
/// [`poisoned`]) is refused by `add` and by `commit`, `load` of a chosen key
/// fails, and the next `n` commits fail as a broken store. Every `commit`,
/// `add` and `remove` call is logged.
struct FaultStore {
    inner: Arc<EngineMapDataStore<CountingEngine>>,
    refusal: Mutex<Option<Refusal>>,
    failing_loads: Mutex<BTreeSet<String>>,
    broken_commits: Mutex<usize>,
    commits: Mutex<Vec<CommitCall>>,
    single_writes: AtomicU64,
}

impl FaultStore {
    fn new(inner: Arc<EngineMapDataStore<CountingEngine>>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            refusal: Mutex::new(None),
            failing_loads: Mutex::new(BTreeSet::new()),
            broken_commits: Mutex::new(0),
            commits: Mutex::new(Vec::new()),
            single_writes: AtomicU64::new(0),
        })
    }

    fn refuse_poisoned(&self, refusal: Option<Refusal>) {
        *self.refusal.lock() = refusal;
    }

    fn fail_load_of(&self, key: &str, fail: bool) {
        if fail {
            self.failing_loads.lock().insert(key.to_string());
        } else {
            self.failing_loads.lock().remove(key);
        }
    }

    fn break_next_commits(&self, n: usize) {
        *self.broken_commits.lock() = n;
    }

    fn commits(&self) -> Vec<CommitCall> {
        self.commits.lock().clone()
    }

    /// The `add` / `remove` calls made so far.
    fn single_writes(&self) -> u64 {
        self.single_writes.load(Ordering::SeqCst)
    }

    fn refuses(&self, value: &RecordValue) -> bool {
        self.refusal.lock().is_some() && poisoned(value)
    }
}

/// Whether `value` carries the poison marker: an LWW string or an OR-Map
/// entry whose string starts with `poison`.
fn poisoned(value: &RecordValue) -> bool {
    let is_poison = |v: &Value| matches!(v, Value::String(s) if s.starts_with("poison"));
    match value {
        RecordValue::Lww { value, .. } => is_poison(value),
        RecordValue::OrMap { records, .. } => records.iter().any(|e| is_poison(&e.value)),
        RecordValue::OrTombstones { .. } => false,
    }
}

fn write_key<'a>(write: &'a StoreWrite<'_>) -> &'a str {
    match write {
        StoreWrite::Put { key, .. } | StoreWrite::Remove { key, .. } => key,
    }
}

fn write_value(write: &StoreWrite<'_>) -> Option<RecordValue> {
    match write {
        StoreWrite::Put {
            value: PutValue::Value(value),
            ..
        } => Some((*value).clone()),
        StoreWrite::Put {
            value: PutValue::Encoded(bytes),
            ..
        } => Some(rmp_serde::from_slice(bytes).expect("an encoded record")),
        StoreWrite::Remove { .. } => None,
    }
}

#[async_trait]
impl MapDataStore for FaultStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        self.single_writes.fetch_add(1, Ordering::SeqCst);
        if self.refuses(value) {
            anyhow::bail!("injected: the store refuses the poisoned record {map}/{key}");
        }
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
        self.single_writes.fetch_add(1, Ordering::SeqCst);
        let value: RecordValue = rmp_serde::from_slice(bytes)?;
        if self.refuses(&value) {
            anyhow::bail!("injected: the store refuses the poisoned record {map}/{key}");
        }
        self.inner
            .add_encoded(map, key, bytes, expiration_time, now)
            .await
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
        self.single_writes.fetch_add(1, Ordering::SeqCst);
        self.inner.remove(map, key, now).await
    }

    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        self.inner.remove_backup(map, key, now).await
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        if self.failing_loads.lock().contains(key) {
            anyhow::bail!("injected: the read of {map}/{key} fails");
        }
        self.inner.load(map, key).await
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

    fn classify_error(&self, err: &anyhow::Error) -> StoreErrorClass {
        if err
            .chain()
            .any(|e| e.downcast_ref::<InjectedBroken>().is_some())
        {
            return StoreErrorClass::StoreBroken(StoreBrokenCause::Corrupted);
        }
        self.inner.classify_error(err)
    }

    async fn commit(&self, commit: &FlushCommit<'_>) -> anyhow::Result<()> {
        let keys = commit
            .writes
            .iter()
            .map(|w| write_key(w).to_string())
            .collect();
        let result = {
            let broken = {
                let mut left = self.broken_commits.lock();
                let broken = *left > 0;
                *left = left.saturating_sub(1);
                broken
            };
            let poisoned: Vec<usize> = commit
                .writes
                .iter()
                .enumerate()
                .filter(|(_, w)| write_value(w).is_some_and(|v| self.refuses(&v)))
                .map(|(i, _)| i)
                .collect();
            let refusal = *self.refusal.lock();
            if broken {
                Err(anyhow::Error::new(InjectedBroken))
            } else if poisoned.is_empty() {
                self.inner.commit(commit).await
            } else if refusal == Some(Refusal::Record) {
                Err(anyhow::Error::new(RecordRefused { indices: poisoned })
                    .context("injected: poisoned records"))
            } else {
                Err(anyhow::anyhow!(
                    "injected: the store refuses a commit holding a poisoned record"
                ))
            }
        };
        self.commits.lock().push(CommitCall {
            keys,
            applied: commit.applied.clone(),
            ok: result.is_ok(),
        });
        result
    }

    fn keeps_applied_watermarks(&self) -> bool {
        self.inner.keeps_applied_watermarks()
    }

    fn applied_watermarks(
        &self,
    ) -> anyhow::Result<Option<crate::storage::map_data_store::AppliedWatermarks>> {
        self.inner.applied_watermarks()
    }

    fn import_applied_watermarks(&self, rows: &[(u32, u64)]) -> anyhow::Result<()> {
        self.inner.import_applied_watermarks(rows)
    }
}

/// A store-mode node whose WAL replays into a [`FaultStore`].
struct FaultNode {
    engine: Arc<CountingEngine>,
    store: Arc<FaultStore>,
    wal: Arc<WalWriter>,
}

impl FaultNode {
    fn open(root: &Root) -> Self {
        let engine = open_engine(root);
        let inner = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).expect("store"));
        let store = FaultStore::new(inner);
        let wal = WalWriter::open_over_store(
            root.wal_dir(),
            WalFsyncPolicy::PerOp,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )
        .expect("store-mode open");
        Self { engine, store, wal }
    }

    fn dyn_store(&self) -> Arc<dyn MapDataStore> {
        Arc::clone(&self.store) as Arc<dyn MapDataStore>
    }

    fn recovery(&self) -> WalRecovery {
        WalRecovery::new(Arc::clone(&self.wal), Vec::new())
    }

    async fn recover(&self) -> anyhow::Result<()> {
        self.recovery().run(self.dyn_store()).await
    }

    async fn append(&self, partition: u32, entry: &WalEntry) {
        self.wal
            .append(partition, entry)
            .await
            .expect("append a frame");
    }

    /// Stores `records` with no row, as a seed the replay starts from.
    async fn seed(&self, records: &[(String, RecordValue)]) {
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
            .inner
            .commit(&FlushCommit {
                writes,
                applied: Vec::new(),
            })
            .await
            .expect("seed the store");
    }

    async fn load(&self, key: &str) -> Option<RecordValue> {
        self.store.inner.load(MAP, key).await.expect("load")
    }

    /// The sequences of `partition` a restart would replay.
    async fn replayable(&self, partition: u32) -> Vec<u64> {
        self.wal
            .unapplied(partition)
            .await
            .expect("unapplied")
            .iter()
            .map(|e| e.sequence)
            .collect()
    }

    async fn close(self) {
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

// ---------------------------------------------------------------------------
// Frames and values
// ---------------------------------------------------------------------------

const P: u32 = 7;
const Q: u32 = 11;
const R: u32 = 13;

fn ts(millis: u64) -> Timestamp {
    Timestamp {
        millis,
        counter: 0,
        node_id: "n".to_string(),
    }
}

fn entry(tag: &str, text: &str) -> OrMapEntry {
    OrMapEntry {
        value: Value::String(text.to_string()),
        tag: tag.to_string(),
        timestamp: ts(1),
    }
}

fn or_value(live: &[(&str, &str)], tombstones: &[&str]) -> RecordValue {
    RecordValue::OrMap {
        records: live.iter().map(|(t, v)| entry(t, v)).collect(),
        tombstones: tombstones.iter().map(|t| (*t).to_string()).collect(),
    }
}

fn remove_frame(key: &str, sequence: u64) -> WalEntry {
    WalEntry {
        map: MAP.to_string(),
        key: key.to_string(),
        op: WalOp::Remove,
        timestamp: None,
        sequence,
    }
}

fn delta_frame(key: &str, delta: OrDelta, sequence: u64) -> WalEntry {
    WalEntry {
        map: MAP.to_string(),
        key: key.to_string(),
        op: WalOp::OrDelta { delta },
        timestamp: None,
        sequence,
    }
}

/// The record keys of a write the counting engine logged.
fn record_count(write: &super::store_mode_tests::WriteRecord) -> usize {
    write.record_keys().len()
}

/// The primary records and applied watermark rows of an engine dump: what a
/// replay is judged by.
fn records_and_rows(dump: &BTreeMap<Vec<u8>, Vec<u8>>) -> BTreeMap<Vec<u8>, Vec<u8>> {
    dump.iter()
        .filter(|(k, _)| matches!(k.first(), Some(&KS_PRIMARY | &KS_PARTITION_META)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Main tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn c01_a_replay_batch_is_one_durable_engine_write_with_its_row() {
    let root = Root::new();
    let node = FaultNode::open(&root);
    // The map's registry rows are written by its first record, so the seed
    // stores one: the replay commit then holds records and the row only.
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    let mut seq = 0;
    for i in 0..6 {
        seq += 1;
        node.append(P, &store_frame(&format!("k{i}"), &lww("v", 10 + i), seq))
            .await;
    }
    seq += 1;
    node.append(P, &remove_frame("warm", seq)).await;
    seq += 1;
    node.append(P, &store_frame("o", &or_value(&[("t1", "a")], &[]), seq))
        .await;
    seq += 1;
    node.append(
        P,
        &delta_frame(
            "o",
            OrDelta::Add {
                entry: entry("t2", "b"),
            },
            seq,
        ),
    )
    .await;
    seq += 1;
    node.append(
        P,
        &delta_frame("o", OrDelta::Remove { tag: "t1".into() }, seq),
    )
    .await;
    node.close().await;

    let node = FaultNode::open(&root);
    let mark = node.engine.mark();
    node.recover().await.expect("recovery");
    let writes = node.engine.writes_since(mark);
    let shape: Vec<_> = writes
        .iter()
        .map(|w| (record_count(w), w.sync, w.ok, w.rows()))
        .collect();
    assert_eq!(
        (
            shape,
            node.store.single_writes(),
            node.wal.durable_applied(P)
        ),
        (vec![(9, true, true, vec![(P, seq)])], 0, Some(seq)),
        "the replay of one batch is one durable engine write holding every record and the row"
    );
    assert_eq!(
        (
            node.load("warm").await,
            node.load("o").await,
            node.replayable(P).await
        ),
        (None, Some(or_value(&[("t2", "b")], &["t1"])), vec![]),
    );
    node.close().await;
}

// --- c02 / c08: batched replay against per-frame replay ---------------------

/// One generated frame of `MAP`.
#[derive(Debug, Clone)]
enum GenFrame {
    Lww {
        key: usize,
        millis: u64,
        text: String,
    },
    Remove {
        key: usize,
    },
    OrSnapshot {
        key: usize,
        live: Vec<(usize, String)>,
        tombs: Vec<usize>,
    },
    OrAdd {
        key: usize,
        tag: usize,
        text: String,
    },
    OrRemove {
        key: usize,
        tag: usize,
    },
    OrPrune {
        key: usize,
        tags: Vec<usize>,
    },
}

/// A generated window: the seed of the store, the frames, the batch limit
/// of the batched run, and how poisoned puts are refused (none for c02).
#[derive(Debug, Clone)]
struct Window {
    seed: Vec<(usize, RecordValue)>,
    frames: Vec<GenFrame>,
    batch_frames: usize,
    refusal: Option<Refusal>,
}

const GEN_KEYS: usize = 4;

fn gen_key(i: usize) -> String {
    format!("g{i}")
}

fn gen_tag(i: usize) -> String {
    format!("t{i}")
}

fn text_strategy(poison: bool) -> BoxedStrategy<String> {
    if poison {
        prop_oneof![
            4 => (0u32..4).prop_map(|i| format!("v{i}")),
            1 => (0u32..4).prop_map(|i| format!("poison{i}")),
        ]
        .boxed()
    } else {
        (0u32..4).prop_map(|i| format!("v{i}")).boxed()
    }
}

fn frame_strategy(poison: bool) -> impl Strategy<Value = GenFrame> {
    let key = 0..GEN_KEYS;
    let tag = 0usize..5;
    prop_oneof![
        (key.clone(), 1u64..20, text_strategy(poison))
            .prop_map(|(key, millis, text)| GenFrame::Lww { key, millis, text }),
        key.clone().prop_map(|key| GenFrame::Remove { key }),
        (
            key.clone(),
            proptest::collection::vec((tag.clone(), text_strategy(poison)), 0..3),
            proptest::collection::vec(tag.clone(), 0..2)
        )
            .prop_map(|(key, live, tombs)| GenFrame::OrSnapshot { key, live, tombs }),
        (key.clone(), tag.clone(), text_strategy(poison))
            .prop_map(|(key, tag, text)| GenFrame::OrAdd { key, tag, text }),
        (key.clone(), tag.clone()).prop_map(|(key, tag)| GenFrame::OrRemove { key, tag }),
        (key, proptest::collection::vec(tag, 1..3))
            .prop_map(|(key, tags)| GenFrame::OrPrune { key, tags }),
    ]
}

fn seed_strategy() -> impl Strategy<Value = Vec<(usize, RecordValue)>> {
    proptest::collection::vec(
        prop_oneof![
            (1u64..20).prop_map(|m| lww("seed", m)),
            Just(or_value(&[("t0", "s"), ("t1", "s")], &["t2"])),
        ],
        GEN_KEYS,
    )
    .prop_map(|values| {
        values
            .into_iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 2)
            .collect()
    })
}

fn window_strategy(poison: bool) -> impl Strategy<Value = Window> {
    let refusal = if poison {
        prop_oneof![Just(Some(Refusal::Record)), Just(Some(Refusal::Commit))].boxed()
    } else {
        Just(None).boxed()
    };
    (
        seed_strategy(),
        proptest::collection::vec(frame_strategy(poison), 1..30),
        2usize..6,
        refusal,
    )
        .prop_map(|(seed, frames, batch_frames, refusal)| Window {
            seed,
            frames,
            batch_frames,
            refusal,
        })
}

fn to_entry(frame: &GenFrame, sequence: u64) -> WalEntry {
    match frame {
        GenFrame::Lww { key, millis, text } => {
            store_frame(&gen_key(*key), &lww(text, *millis), sequence)
        }
        GenFrame::Remove { key } => remove_frame(&gen_key(*key), sequence),
        GenFrame::OrSnapshot { key, live, tombs } => {
            let value = RecordValue::OrMap {
                records: live.iter().map(|(t, v)| entry(&gen_tag(*t), v)).collect(),
                tombstones: tombs.iter().map(|t| gen_tag(*t)).collect(),
            };
            store_frame(&gen_key(*key), &value, sequence)
        }
        GenFrame::OrAdd { key, tag, text } => delta_frame(
            &gen_key(*key),
            OrDelta::Add {
                entry: entry(&gen_tag(*tag), text),
            },
            sequence,
        ),
        GenFrame::OrRemove { key, tag } => delta_frame(
            &gen_key(*key),
            OrDelta::Remove { tag: gen_tag(*tag) },
            sequence,
        ),
        GenFrame::OrPrune { key, tags } => delta_frame(
            &gen_key(*key),
            OrDelta::Prune {
                tags: tags.iter().map(|t| gen_tag(*t)).collect(),
            },
            sequence,
        ),
    }
}

/// What a replay left: every generated key's value, the partition's row,
/// and the sequences a restart would replay.
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    values: Vec<Option<RecordValue>>,
    frontier: u64,
    replayable: Vec<u64>,
}

/// Seeds a fresh root, appends the window to `P`, and replays it in store
/// mode with batches of at most `batch_frames` frames.
async fn batched_outcome(window: &Window, entries: &[WalEntry], batch_frames: usize) -> Outcome {
    let root = Root::new();
    let node = FaultNode::open(&root);
    let seed: Vec<(String, RecordValue)> = window
        .seed
        .iter()
        .map(|(k, v)| (gen_key(*k), v.clone()))
        .collect();
    node.seed(&seed).await;
    for e in entries {
        node.append(P, e).await;
    }
    node.store.refuse_poisoned(window.refusal);
    let mut recovery = node.recovery();
    recovery.test_set_replay_batch_limits(batch_frames, 8 * 1024 * 1024);
    recovery.run(node.dyn_store()).await.expect("recovery");
    let mut values = Vec::new();
    for k in 0..GEN_KEYS {
        values.push(node.load(&gen_key(k)).await);
    }
    let outcome = Outcome {
        values,
        frontier: node.engine.row(P).unwrap_or(0),
        replayable: node.replayable(P).await,
    };
    node.close().await;
    outcome
}

/// Seeds a fresh store and replays the window frame by frame through
/// `replay_entry`, keeping the contiguous-success frontier by hand.
async fn per_frame_outcome(window: &Window, entries: &[WalEntry]) -> Outcome {
    let root = Root::new();
    let engine = open_engine(&root);
    let inner = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).expect("store"));
    let store = FaultStore::new(Arc::clone(&inner));
    let seed: Vec<(String, RecordValue)> = window
        .seed
        .iter()
        .map(|(k, v)| (gen_key(*k), v.clone()))
        .collect();
    let writes = seed
        .iter()
        .map(|(key, value)| StoreWrite::Put {
            map: MAP,
            key,
            value: PutValue::Value(value),
            expiration_time: 0,
            store_time: 0,
        })
        .collect();
    inner
        .commit(&FlushCommit {
            writes,
            applied: Vec::new(),
        })
        .await
        .expect("seed");
    store.refuse_poisoned(window.refusal);
    let dyn_store = Arc::clone(&store) as Arc<dyn MapDataStore>;
    let mut frontier = 0;
    let mut failed = false;
    for e in entries {
        match WalRecovery::replay_entry(&dyn_store, e, 0, true).await {
            Ok(()) if !failed => frontier = e.sequence,
            Ok(()) => {}
            Err(_) => failed = true,
        }
    }
    let mut values = Vec::new();
    for k in 0..GEN_KEYS {
        values.push(inner.load(MAP, &gen_key(k)).await.expect("load"));
    }
    Outcome {
        values,
        frontier,
        replayable: entries
            .iter()
            .map(|e| e.sequence)
            .filter(|s| *s > frontier)
            .collect(),
    }
}

/// Runs `cases` generated windows, each replayed three ways.
fn run_equivalence(cases: u32, poison: bool) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mut runner = TestRunner::new(ProptestConfig {
        cases,
        failure_persistence: None,
        max_shrink_iters: 64,
        ..ProptestConfig::default()
    });
    runner
        .run(&window_strategy(poison), |window| {
            let entries: Vec<WalEntry> = window
                .frames
                .iter()
                .zip(1u64..)
                .map(|(f, seq)| to_entry(f, seq))
                .collect();
            let (batched, singles, per_frame) = runtime.block_on(async {
                (
                    batched_outcome(&window, &entries, window.batch_frames).await,
                    batched_outcome(&window, &entries, 1).await,
                    per_frame_outcome(&window, &entries).await,
                )
            });
            prop_assert_eq!(&batched, &singles, "batched against batches of one frame");
            prop_assert_eq!(&batched, &per_frame, "batched against per-frame replay");
            Ok(())
        })
        .expect("batched replay equals per-frame replay");
}

#[test]
fn c02_batched_replay_equals_per_frame_replay() {
    run_equivalence(64, false);
}

#[tokio::test(flavor = "multi_thread")]
async fn c03_a_frame_that_fails_to_prepare_stops_the_frontier_and_later_frames_apply() {
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    node.append(P, &store_frame("a", &lww("a", 5), 1)).await;
    node.append(P, &store_frame("x", &lww("x", 5), 2)).await;
    node.append(P, &store_frame("b", &lww("b", 5), 3)).await;
    node.append(
        P,
        &delta_frame(
            "c",
            OrDelta::Add {
                entry: entry("t1", "c"),
            },
            4,
        ),
    )
    .await;
    node.close().await;

    let node = FaultNode::open(&root);
    // The LWW gate of frame 2 reads `x`, and that read fails.
    node.store.fail_load_of("x", true);
    let mark = node.engine.mark();
    node.recover().await.expect("recovery");
    let writes: Vec<(usize, Vec<(u32, u64)>)> = node
        .engine
        .writes_since(mark)
        .iter()
        .map(|w| (record_count(w), w.rows()))
        .collect();
    assert_eq!(
        (
            writes,
            node.load("a").await.is_some(),
            node.load("x").await.is_some(),
            node.load("b").await.is_some(),
            node.load("c").await.is_some(),
            node.wal.durable_applied(P),
            node.replayable(P).await,
        ),
        (vec![(3, vec![(P, 1)])], true, false, true, true, Some(1), vec![2, 3, 4]),
        "one batch write holding the frames that prepared and the row of the frontier below frame 2"
    );
    node.store.fail_load_of("x", false);
    node.close().await;

    let node = FaultNode::open(&root);
    node.recover().await.expect("recovery after the read heals");
    assert_eq!(
        (
            node.load("x").await.is_some(),
            node.engine.row(P),
            node.replayable(P).await
        ),
        (true, Some(4), vec![]),
    );
    node.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c04_replay_batches_respect_1000_frames_or_8_mib() {
    async fn replay_shape(frames: Vec<WalEntry>) -> Vec<(usize, Vec<(u32, u64)>)> {
        let root = Root::new();
        let node = FaultNode::open(&root);
        node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
        for e in &frames {
            node.append(P, e).await;
        }
        node.close().await;
        let node = FaultNode::open(&root);
        let mark = node.engine.mark();
        node.recover().await.expect("recovery");
        let shape = node
            .engine
            .writes_since(mark)
            .iter()
            .map(|w| (record_count(w), w.rows()))
            .collect();
        node.close().await;
        shape
    }

    let small: Vec<WalEntry> = (1..=2500)
        .map(|i| store_frame(&format!("k{i}"), &lww("v", 5), i))
        .collect();
    let three_mib = "x".repeat(3 * 1024 * 1024);
    let large: Vec<WalEntry> = (1..=3)
        .map(|i| store_frame(&format!("l{i}"), &lww(&three_mib, 5), i))
        .collect();
    let nine_mib = "y".repeat(9 * 1024 * 1024);
    let oversize = vec![
        store_frame("big", &lww(&nine_mib, 5), 1),
        store_frame("after", &lww("v", 5), 2),
    ];
    assert_eq!(
        (
            replay_shape(small).await,
            replay_shape(large).await,
            replay_shape(oversize).await,
        ),
        (
            vec![
                (1000, vec![(P, 1000)]),
                (1000, vec![(P, 2000)]),
                (500, vec![(P, 2500)])
            ],
            vec![(2, vec![(P, 2)]), (1, vec![(P, 3)])],
            vec![(1, vec![(P, 1)]), (1, vec![(P, 2)])],
        ),
        "batches of 1000 frames, of at most 8 MiB of values, and a first frame admitted whole"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn c05_a_refused_replay_commit_is_halved_to_single_frames_and_healthy_frames_apply() {
    // [k@1, h1, k@3, h2] with every write of `k` refused.
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    node.append(P, &store_frame("k", &lww("poison-1", 5), 1))
        .await;
    node.append(P, &store_frame("h1", &lww("h1", 5), 2)).await;
    node.append(P, &store_frame("k", &lww("poison-3", 6), 3))
        .await;
    node.append(P, &store_frame("h2", &lww("h2", 5), 4)).await;
    node.close().await;

    let node = FaultNode::open(&root);
    node.store.refuse_poisoned(Some(Refusal::Commit));
    node.recover().await.expect("recovery");
    let commits = node.store.commits();
    let first = commits.first().map(|c| c.keys.clone());
    assert_eq!(
        (
            first,
            commits.len() <= 7,
            node.store.single_writes(),
            node.load("h1").await,
            node.load("h2").await,
            node.load("k").await,
            node.engine.row(P),
            node.replayable(P).await,
        ),
        (
            Some(vec!["k".to_string(), "h1".into(), "k".into(), "h2".into()]),
            true,
            0,
            Some(lww("h1", 5)),
            Some(lww("h2", 5)),
            None,
            None,
            vec![1, 2, 3, 4],
        ),
        "the batch is one commit, halved down to single frames: h1 and h2 stored, the frontier \
         below k@1; commits {commits:?}"
    );
    node.close().await;

    // A broken store stops its partition at the first commit; the next
    // partition is still replayed.
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    node.append(Q, &store_frame("q1", &lww("q1", 5), 1)).await;
    node.append(Q, &store_frame("q2", &lww("q2", 5), 2)).await;
    node.append(R, &store_frame("r1", &lww("r1", 5), 3)).await;
    node.append(R, &store_frame("r2", &lww("r2", 5), 4)).await;
    node.close().await;

    let node = FaultNode::open(&root);
    node.store.break_next_commits(1);
    node.recover().await.expect("recovery");
    let commits: Vec<_> = node
        .store
        .commits()
        .into_iter()
        .map(|c| (c.keys, c.applied, c.ok))
        .collect();
    assert_eq!(
        (
            commits,
            node.load("q1").await.is_some(),
            node.load("r1").await.is_some(),
            node.load("r2").await.is_some(),
            node.engine.row(Q),
            node.engine.row(R),
            node.replayable(Q).await,
        ),
        (
            vec![
                (vec!["q1".to_string(), "q2".into()], vec![(Q, 2)], false),
                (vec!["r1".to_string(), "r2".into()], vec![(R, 4)], true),
            ],
            false,
            true,
            true,
            None,
            Some(4),
            vec![1, 2],
        ),
        "the broken commit is not halved and stops its partition; the next partition is replayed"
    );
    node.close().await;
}

/// Set in a child of the replay tests: `"{scenario}\n{root}"`.
const CHILD_ENV: &str = "TOPGUN_TEST_BATCHED_REPLAY_CHILD";
const CHILD_DEADLINE: Duration = Duration::from_secs(120);

/// The scenario and root this process was started with as a child, if any.
fn child_spec() -> Option<(String, PathBuf)> {
    let spec = std::env::var(CHILD_ENV).ok()?;
    let (scenario, root) = spec.split_once('\n').expect("child spec");
    Some((scenario.to_string(), PathBuf::from(root)))
}

/// Re-runs this test binary with only `test` selected, as a child that runs
/// `scenario` with `root`; returns the child's process for the caller to wait
/// on or kill. Its output goes to `{root}/child-logs`.
fn spawn_child(test: &str, scenario: &str, root: &Path) -> std::process::Child {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let logs = root.join("child-logs");
    std::fs::create_dir_all(&logs).expect("logs dir");
    let label = format!(
        "{}-{}",
        RUNS.fetch_add(1, Ordering::SeqCst),
        scenario.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, rest)| rest);
    std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            &format!("{module}::{test}"),
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
            "--include-ignored",
        ])
        .env_remove("DATABASE_URL")
        .env_remove(super::TEST_ABORT_AT_ENV)
        .env(CHILD_ENV, format!("{scenario}\n{}", root.display()))
        .env("TOPGUN_DATA_DIR", root.join("topgun-data"))
        .env("TOPGUN_REDB_PATH", root.join("topgun.redb"))
        .stdout(std::fs::File::create(logs.join(format!("{label}.out"))).expect("stdout file"))
        .stderr(std::fs::File::create(logs.join(format!("{label}.err"))).expect("stderr file"))
        .spawn()
        .expect("the child starts")
}

/// Waits for `child` with a hard deadline; a child still running then fails
/// the test as a hang.
fn wait_child(mut child: std::process::Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + CHILD_DEADLINE;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("a child still running after {CHILD_DEADLINE:?}: a hang");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn c06_a_kill_between_replay_batches_resumes_from_the_last_committed_row() {
    const TEST: &str = "c06_a_kill_between_replay_batches_resumes_from_the_last_committed_row";
    if let Some((scenario, path)) = child_spec() {
        assert_eq!(scenario, "replay-and-stop-after-the-second-batch");
        let root = Root::at(path);
        let engine = open_engine(&root);
        let node = super::store_mode_tests::Node::open_over(
            Arc::clone(&engine),
            &root,
            WalFsyncPolicy::PerOp,
        )
        .expect("store-mode open");
        engine.abort_in(2, super::store_mode_tests::AbortSide::After);
        let _ = node.recover().await;
        return;
    }

    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    for i in 1..=2500u64 {
        node.append(
            P,
            &store_frame(&format!("k{i}"), &lww(&format!("v{i}"), 5), i),
        )
        .await;
    }
    node.close().await;
    let uninterrupted = Root::new();
    super::store_mode_tests::copy_dir(&root.data_dir(), &uninterrupted.data_dir());
    super::store_mode_tests::copy_dir(&root.wal_dir(), &uninterrupted.wal_dir());

    let status = wait_child(spawn_child(
        TEST,
        "replay-and-stop-after-the-second-batch",
        &root.path,
    ));
    let after_kill = rows_of(&dump_at(&root));

    let node = FaultNode::open(&root);
    node.recover().await.expect("recovery after the kill");
    node.close().await;
    let node = FaultNode::open(&uninterrupted);
    node.recover().await.expect("uninterrupted recovery");
    node.close().await;
    assert_eq!(
        (
            std::os::unix::process::ExitStatusExt::signal(&status),
            after_kill.get(&P).copied(),
            records_and_rows(&dump_at(&root)) == records_and_rows(&dump_at(&uninterrupted)),
        ),
        (Some(6), Some(2000), true),
        "the child stops after the second batch with the row of that batch durable, and the \
         restart ends where an uninterrupted replay ends"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn c07_a_frame_at_or_below_the_watermark_in_a_batch_refuses_the_start() {
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    for i in 1..=8u64 {
        node.append(P, &store_frame(&format!("k{i}"), &lww("v", 5), i))
            .await;
    }
    node.store
        .inner
        .commit(&FlushCommit {
            writes: Vec::new(),
            applied: vec![(P, 5)],
        })
        .await
        .expect("row 5");
    node.close().await;

    // Through `run`: the WAL is made to hand recovery frames from 4 up.
    let node = FaultNode::open(&root);
    node.wal.test_force_unapplied_floor(P, Some(3));
    let mark = node.engine.mark();
    let error = format!(
        "{:#}",
        node.recover().await.expect_err("the start is refused")
    );
    let through_run = (
        error.contains(&format!("partition {P}")) && error.contains("frame 4"),
        node.engine.writes_since(mark).len(),
    );

    // Straight into the batch builder, with the store's watermark 5.
    let entries: Vec<WalEntry> = (5..=8u64)
        .map(|i| store_frame(&format!("k{i}"), &lww("v", 5), i))
        .collect();
    let mark = node.engine.mark();
    let refused = node
        .recovery()
        .replay_partition(&node.dyn_store(), P, &entries, 5, 0)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:#}"));
    let through_builder = (
        refused
            .as_ref()
            .is_err_and(|e| e.contains(&format!("partition {P}")) && e.contains("frame 5")),
        node.engine.writes_since(mark).len(),
    );
    assert_eq!(
        (through_run, through_builder, node.engine.row(P)),
        ((true, 0), (true, 0), Some(5)),
        "a frame at or below the watermark refuses the start, naming partition and frame, and \
         nothing is written: {error} / {refused:?}"
    );
    node.close().await;
}

#[test]
fn c08_batched_replay_equals_per_frame_replay_under_refusals() {
    // The hand case first: several frames of a refused key in one batch, a
    // healthy frame of that key after its refused one, and refused frames
    // in both halves of the batch.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    for refusal in [Refusal::Record, Refusal::Commit] {
        let window = Window {
            seed: vec![(0, or_value(&[("t0", "s")], &[]))],
            frames: vec![
                GenFrame::OrAdd {
                    key: 0,
                    tag: 1,
                    text: "poison-a".into(),
                },
                GenFrame::Lww {
                    key: 1,
                    millis: 5,
                    text: "h1".into(),
                },
                GenFrame::OrAdd {
                    key: 0,
                    tag: 2,
                    text: "v".into(),
                },
                GenFrame::Lww {
                    key: 2,
                    millis: 5,
                    text: "poison-b".into(),
                },
                GenFrame::Lww {
                    key: 2,
                    millis: 4,
                    text: "older".into(),
                },
                GenFrame::Lww {
                    key: 3,
                    millis: 5,
                    text: "h2".into(),
                },
            ],
            batch_frames: 6,
            refusal: Some(refusal),
        };
        let entries: Vec<WalEntry> = window
            .frames
            .iter()
            .zip(1u64..)
            .map(|(f, seq)| to_entry(f, seq))
            .collect();
        let (batched, per_frame) = runtime.block_on(async {
            (
                batched_outcome(&window, &entries, window.batch_frames).await,
                per_frame_outcome(&window, &entries).await,
            )
        });
        assert_eq!(
            batched, per_frame,
            "the hand case under {refusal:?}: batched against per-frame replay"
        );
    }
    drop(runtime);
    run_equivalence(64, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn c09_a_segment_that_fails_to_unlink_holds_back_only_its_own_partition() {
    const A: u32 = 241;
    const B: u32 = 257;
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.seed(&[("warm".to_string(), lww("warm", 1))]).await;
    // Two sealed segments in A, one in B, every frame at or below the rows:
    // a tick seals an active segment whose first frame is old enough, and
    // collects nothing while no row covers a segment.
    let seal_age = Duration::from_secs(31);
    node.append(A, &store_frame("a1", &lww("v", 5), 1)).await;
    node.append(B, &store_frame("b1", &lww("v", 5), 2)).await;
    node.wal.test_advance_clock(seal_age);
    node.wal.test_collect_now().await;
    node.append(A, &store_frame("a2", &lww("v", 5), 3)).await;
    node.wal.test_advance_clock(seal_age);
    node.wal.test_collect_now().await;
    node.store
        .inner
        .commit(&FlushCommit {
            writes: Vec::new(),
            applied: vec![(A, 3), (B, 2)],
        })
        .await
        .expect("rows");
    node.wal.mark_applied(A, 3).await.expect("mark A");
    node.wal.mark_applied(B, 2).await.expect("mark B");

    let sealed_a = node.wal.test_sealed_paths(A).await;
    let sealed_b = node.wal.test_sealed_paths(B).await;
    assert_eq!(
        (sealed_a.len(), sealed_b.len()),
        (2, 1),
        "the sealed segments set up"
    );
    node.wal.test_refuse_unlink(&sealed_a[0], true);
    node.wal.test_collect_now().await;
    let first_tick = (
        sealed_a.iter().map(|p| p.exists()).collect::<Vec<_>>(),
        sealed_b[0].exists(),
        node.wal.test_sealed_paths(A).await == sealed_a,
        node.wal.test_sealed_paths(B).await.is_empty(),
        retained_matches_disk(&node.wal, &root.wal_dir(), &[A, B]).await,
    );

    node.wal.test_refuse_unlink(&sealed_a[0], false);
    node.wal.test_collect_now().await;
    let second_tick = (
        sealed_a.iter().map(|p| p.exists()).collect::<Vec<_>>(),
        node.wal.test_sealed_paths(A).await.is_empty(),
        retained_matches_disk(&node.wal, &root.wal_dir(), &[A, B]).await,
    );
    assert_eq!(
        (first_tick, second_tick),
        (
            (vec![true, true], false, true, true, true),
            (vec![false, false], true, true),
        ),
        "a refused unlink in A leaves A's segments on disk and in the sealed set, B is collected \
         in the same tick, and the next tick collects A"
    );
    node.close().await;
}

/// The bytes of the segment files of `partition` in `wal_dir`.
fn segment_bytes(wal_dir: &Path, partition: u32) -> u64 {
    super::store_mode_tests::segment_files(wal_dir, partition)
        .values()
        .map(|p| std::fs::metadata(p).expect("segment metadata").len())
        .sum()
}

/// Whether `topgun_wal_retained_bytes{partition}` reads the bytes on disk for
/// every partition of `partitions`, right after a tick of `wal` set it. The
/// recorder is process-global and another test's tick may set the same
/// series in between, so a mismatch is retried with a fresh tick.
async fn retained_matches_disk(wal: &WalWriter, wal_dir: &Path, partitions: &[u32]) -> bool {
    for _ in 0..20 {
        wal.test_collect_now().await;
        let rendered =
            crate::service::middleware::observability::init_observability().render_metrics();
        let all = partitions.iter().all(|p| {
            let series = format!("{}{{partition=\"{p}\"}}", super::WAL_RETAINED_BYTES_GAUGE);
            let gauge = rendered
                .lines()
                .find(|line| line.starts_with(&series))
                .and_then(|line| line.rsplit(' ').next())
                .and_then(|v| v.parse::<f64>().ok());
            #[allow(clippy::cast_precision_loss)]
            let disk = segment_bytes(wal_dir, *p) as f64;
            gauge == Some(disk)
        });
        if all {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// The crash loop
//
// One root holds a data directory and a WAL directory for the whole loop. Each
// incarnation is a child process that opens them as the server does (store
// open, store-mode WAL, recovery, seed, write-behind) and writes seeded LWW
// puts and removes and OR adds and removes over 200 keys in 4 maps, logging
// every op before it starts and again once it is acknowledged. The parent
// kills it with SIGKILL, cuts every WAL segment back to the last length the
// child's fsync observer logged — the power-loss model, since SIGKILL alone
// keeps the page cache — and opens the directories twice more in fresh
// children, each dumping what it finds.
// ---------------------------------------------------------------------------

/// The maps of the crash loop; the first two take LWW ops, the others OR ops.
const LOOP_MAPS: [&str; 4] = ["lww0", "lww1", "or0", "or1"];
/// Keys per map.
const LOOP_KEYS: u64 = 50;
/// Ops the child of the held incarnation writes before it lets the parent
/// kill it.
const HELD_OPS: u64 = 40;

/// A small seeded generator, so a failing incarnation can be replayed from
/// its printed seed.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn policy_name(policy: WalFsyncPolicy) -> &'static str {
    match policy {
        WalFsyncPolicy::PerOp => "per_op",
        WalFsyncPolicy::Batched => "batched",
        WalFsyncPolicy::None => "none",
    }
}

/// Appends `line` to `path` and makes it durable before returning.
fn append_durable(path: &Path, line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open a log file");
    std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes()).expect("write a log line");
    file.sync_data().expect("sync a log line");
}

fn now_millis() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis(),
    )
    .expect("millis")
}

/// Opens `root` as the server does: the store, the store-mode WAL over it,
/// and the WAL's fsync observer when `fsync_log` is given.
fn open_loop_store(
    root: &Path,
    policy: WalFsyncPolicy,
) -> (
    Arc<crate::storage::datastores::RedbKvEngine>,
    Arc<dyn MapDataStore>,
    Arc<WalWriter>,
) {
    let data_dir = root.join("topgun-data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let engine = Arc::new(
        crate::storage::datastores::RedbKvEngine::open(&data_dir.join("engine.redb"))
            .expect("the engine opens"),
    );
    let store: Arc<dyn MapDataStore> = Arc::new(
        crate::storage::datastores::RedbDataStore::open(Arc::clone(&engine))
            .expect("the store opens"),
    );
    let wal = WalWriter::open_over_store(root.join("wal"), policy, Arc::clone(&store))
        .expect("the store-mode WAL opens");
    (engine, store, wal)
}

/// The writer child: opens the root, recovers, seeds, and writes until it
/// is killed. With `hold`, every fsync of the batched policy is held, and
/// after [`HELD_OPS`] acknowledged ops whose rows are committed the child
/// tells the parent to kill it.
async fn crash_loop_writer(
    root: &Path,
    policy: WalFsyncPolicy,
    incarnation: u64,
    seed: u64,
    hold: bool,
) {
    let (_engine, store, wal) = open_loop_store(root, policy);
    let fsync_log = root.join("fsync-log");
    wal.test_set_fsync_observer(Some(Arc::new(move |path: &Path, length: u64| {
        append_durable(&fsync_log, &format!("{}\t{length}", path.display()));
    })));
    if hold {
        wal.test_hold_batched_fsync(true);
    }
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&store))
        .await
        .expect("recovery");
    let sequence_start = wal.max_observed_sequence().await.expect("seed") + 1;
    let wb = crate::storage::datastores::WriteBehindDataStore::new_with_wal(
        Arc::clone(&store),
        crate::storage::datastores::WriteBehindConfig {
            write_delay_ms: 2,
            flush_interval_ms: 5,
            shutdown_timeout_ms: 1_000,
            capacity: 0,
            ..crate::storage::datastores::WriteBehindConfig::default()
        },
        Some(crate::storage::datastores::WalBootstrap {
            wal: Arc::clone(&wal) as Arc<dyn Wal>,
            sequence_start,
        }),
    );
    let assigned = Arc::new(AtomicU64::new(0));
    let observed = Arc::clone(&assigned);
    wb.test_set_append_observer(Arc::new(move |_partition, sequence| {
        observed.store(sequence, Ordering::SeqCst);
    }));
    let ops_log = root.join("ops-log");
    std::fs::write(root.join(format!("ready-{incarnation}")), b"").expect("ready marker");

    let mut rng = SplitMix(seed);
    for n in 0u64.. {
        let map = LOOP_MAPS[usize::try_from(rng.below(4)).expect("index")];
        let key = format!("c{}", rng.below(LOOP_KEYS));
        let now = i64::try_from(now_millis()).expect("now");
        let (op, arg) = if map.starts_with("lww") {
            if rng.below(10) < 7 {
                ("put", format!("v{incarnation}-{n}"))
            } else {
                ("del", String::new())
            }
        } else {
            let current = wb.load(map, &key).await.expect("load");
            let live: Vec<String> = match &current {
                Some(RecordValue::OrMap { records, .. }) => {
                    records.iter().map(|e| e.tag.clone()).collect()
                }
                _ => Vec::new(),
            };
            if live.is_empty() || rng.below(10) < 6 {
                ("oradd", format!("t{incarnation}-{n}"))
            } else {
                let at = usize::try_from(rng.below(live.len() as u64)).expect("index");
                ("orrem", live[at].clone())
            }
        };
        append_durable(
            &ops_log,
            &format!("intent\t{incarnation}\t{map}\t{key}\t{op}\t{arg}"),
        );
        match op {
            "put" => {
                let value = RecordValue::Lww {
                    value: Value::String(arg.clone()),
                    timestamp: Timestamp {
                        millis: now_millis(),
                        counter: u32::try_from(n).expect("counter"),
                        node_id: "c90".to_string(),
                    },
                };
                wb.add(map, &key, &value, 0, now).await.expect("put");
            }
            "del" => wb.remove(map, &key, now).await.expect("remove"),
            _ => {
                let delta = if op == "oradd" {
                    OrDelta::Add {
                        entry: OrMapEntry {
                            value: Value::String("x".to_string()),
                            tag: arg.clone(),
                            timestamp: ts(now_millis()),
                        },
                    }
                } else {
                    OrDelta::Remove { tag: arg.clone() }
                };
                let mut value =
                    wb.load(map, &key)
                        .await
                        .expect("load")
                        .unwrap_or(RecordValue::OrMap {
                            records: Vec::new(),
                            tombstones: Vec::new(),
                        });
                crate::service::domain::crdt::normalize_to_or_map(&mut value);
                crate::service::domain::crdt::apply_or_delta(delta.clone(), &mut value);
                wb.add_with_witness(
                    map,
                    &key,
                    crate::storage::map_data_store::WriteSource::Value(&value),
                    0,
                    now,
                    Some(&delta),
                )
                .await
                .expect("OR write");
            }
        }
        let sequence = assigned.load(Ordering::SeqCst);
        append_durable(
            &ops_log,
            &format!("ack\t{incarnation}\t{map}\t{key}\t{op}\t{arg}\t{sequence}"),
        );
        if hold && n + 1 == HELD_OPS {
            // Every row the write-behind flush commits now covers frames no
            // fsync has reached.
            wait_for("the held writes to be flushed", || wb.test_is_idle()).await;
            std::fs::write(root.join(format!("held-{incarnation}")), b"").expect("held marker");
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
}

/// What a dump child found in the root.
#[derive(Debug, Clone, PartialEq)]
struct LoopDump {
    /// Frames above the store's watermarks before recovery ran.
    unapplied: u64,
    /// The first sequence the open would hand out.
    seed: u64,
    rows: BTreeMap<u32, u64>,
    collected: BTreeMap<u32, u64>,
    /// `(map, key)` → the value's description.
    values: BTreeMap<(String, String), String>,
    /// sha256 of every key and value of the engine.
    engine: String,
}

fn describe(value: Option<&RecordValue>) -> String {
    match value {
        None => "none".to_string(),
        Some(RecordValue::Lww {
            value: Value::String(s),
            ..
        }) => format!("lww:{s}"),
        Some(RecordValue::OrMap {
            records,
            tombstones,
        }) => {
            let live: BTreeSet<&str> = records.iter().map(|e| e.tag.as_str()).collect();
            let dead: BTreeSet<&str> = tombstones.iter().map(String::as_str).collect();
            format!(
                "or:{}|{}",
                live.into_iter().collect::<Vec<_>>().join(","),
                dead.into_iter().collect::<Vec<_>>().join(",")
            )
        }
        Some(other) => format!("other:{other:?}"),
    }
}

/// The dump child: opens the root as the server does, counts the frames
/// recovery will replay, recovers, seeds, and writes what it finds to
/// `{root}/{name}`.
async fn crash_loop_dump(root: &Path, policy: WalFsyncPolicy, name: &str) {
    use sha2::Digest as _;
    let (engine, store, wal) = open_loop_store(root, policy);
    let mut unapplied = 0;
    for partition in WalRecovery::discover_partitions(&root.join("wal")).expect("partitions") {
        unapplied += wal.unapplied(partition).await.expect("unapplied").len();
    }
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&store))
        .await
        .expect("recovery");
    let seed = wal.max_observed_sequence().await.expect("seed") + 1;
    let mut lines = vec![format!("unapplied\t{unapplied}"), format!("seed\t{seed}")];
    let watermarks = store
        .applied_watermarks()
        .expect("watermarks")
        .expect("a store that keeps watermarks");
    for (partition, w) in watermarks.by_partition.iter().enumerate() {
        if *w > 0 {
            lines.push(format!("row\t{partition}\t{w}"));
        }
    }
    if let Some(collected) = super::store_mode_tests::read_collected(&root.join("wal")) {
        for (partition, g) in collected.iter().enumerate() {
            if *g > 0 {
                lines.push(format!("collected\t{partition}\t{g}"));
            }
        }
    }
    for map in LOOP_MAPS {
        for i in 0..LOOP_KEYS {
            let key = format!("c{i}");
            let value = store.load(map, &key).await.expect("load");
            lines.push(format!("value\t{map}\t{key}\t{}", describe(value.as_ref())));
        }
    }
    let mut hasher = sha2::Sha256::new();
    let snapshot = crate::storage::datastores::kv_engine::KvEngine::snapshot(engine.as_ref())
        .expect("engine snapshot");
    for item in snapshot
        .iter(crate::storage::datastores::kv_engine::KeyRange::default())
        .expect("engine iter")
    {
        let (k, v) = item.expect("engine entry");
        hasher.update((k.len() as u64).to_be_bytes());
        hasher.update(&k);
        hasher.update((v.len() as u64).to_be_bytes());
        hasher.update(&v);
    }
    lines.push(format!("engine\t{}", hex::encode(hasher.finalize())));
    std::fs::write(root.join(name), lines.join("\n")).expect("write the dump");
}

fn parse_dump(text: &str) -> LoopDump {
    let mut dump = LoopDump {
        unapplied: 0,
        seed: 0,
        rows: BTreeMap::new(),
        collected: BTreeMap::new(),
        values: BTreeMap::new(),
        engine: String::new(),
    };
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields[0] {
            "unapplied" => dump.unapplied = fields[1].parse().expect("count"),
            "seed" => dump.seed = fields[1].parse().expect("seed"),
            "row" => {
                dump.rows
                    .insert(fields[1].parse().expect("p"), fields[2].parse().expect("w"));
            }
            "collected" => {
                dump.collected
                    .insert(fields[1].parse().expect("p"), fields[2].parse().expect("g"));
            }
            "value" => {
                dump.values.insert(
                    (fields[1].to_string(), fields[2].to_string()),
                    fields[3].to_string(),
                );
            }
            "engine" => dump.engine = fields[1].to_string(),
            other => panic!("unknown dump line {other:?}"),
        }
    }
    dump
}

/// One logged op of the writer children.
#[derive(Debug, Clone)]
struct LoggedOp {
    incarnation: u64,
    map: String,
    key: String,
    op: String,
    arg: String,
    /// The WAL sequence, once acknowledged.
    acked: Option<u64>,
}

/// Every op of the ops log in order, an intent matched with the ack after it.
fn read_ops(root: &Path) -> Vec<LoggedOp> {
    let text = std::fs::read_to_string(root.join("ops-log")).unwrap_or_default();
    let mut ops: Vec<LoggedOp> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields[0] {
            "intent" => ops.push(LoggedOp {
                incarnation: fields[1].parse().expect("incarnation"),
                map: fields[2].to_string(),
                key: fields[3].to_string(),
                op: fields[4].to_string(),
                arg: fields[5].to_string(),
                acked: None,
            }),
            "ack" => {
                let last = ops.last_mut().expect("an ack follows its intent");
                assert_eq!(
                    (last.map.as_str(), last.key.as_str(), last.op.as_str()),
                    (fields[2], fields[3], fields[4]),
                    "an ack names the op of the intent before it"
                );
                last.acked = Some(fields[6].parse().expect("sequence"));
            }
            other => panic!("unknown ops-log line {other:?}"),
        }
    }
    ops
}

/// The states `(map, key)` (for an OR op: its tag) may be found in once the
/// ops so far are recovered: the effect of the last acknowledged op, or of
/// any unacknowledged op after it, or — with no acknowledged op — absent.
fn allowed_states(ops: &[&LoggedOp]) -> BTreeSet<bool> {
    let last_acked = ops.iter().rposition(|o| o.acked.is_some());
    let effect = |o: &LoggedOp| o.op == "put" || o.op == "oradd";
    let mut allowed = BTreeSet::new();
    allowed.insert(last_acked.is_some_and(|i| effect(ops[i])));
    for o in &ops[last_acked.map_or(0, |i| i + 1)..] {
        allowed.insert(effect(o));
    }
    allowed
}

/// Checks acknowledged ⊆ recovered: every acknowledged op's effect is in
/// `dump` unless a later op of the same key (OR: tag) may have superseded it.
fn check_acked_recovered(ops: &[LoggedOp], dump: &LoopDump) -> Result<(), String> {
    let mut by_key: BTreeMap<(String, String), Vec<&LoggedOp>> = BTreeMap::new();
    let mut by_tag: BTreeMap<(String, String, String), Vec<&LoggedOp>> = BTreeMap::new();
    for o in ops {
        if o.map.starts_with("lww") {
            by_key
                .entry((o.map.clone(), o.key.clone()))
                .or_default()
                .push(o);
        } else {
            by_tag
                .entry((o.map.clone(), o.key.clone(), o.arg.clone()))
                .or_default()
                .push(o);
        }
    }
    for ((map, key), ops) in &by_key {
        let found = &dump.values[&(map.clone(), key.clone())];
        let last_acked = ops.iter().rposition(|o| o.acked.is_some());
        let mut allowed: BTreeSet<String> = BTreeSet::new();
        let state = |o: &LoggedOp| {
            if o.op == "put" {
                format!("lww:{}", o.arg)
            } else {
                "none".to_string()
            }
        };
        allowed.insert(last_acked.map_or_else(|| "none".to_string(), |i| state(ops[i])));
        for o in &ops[last_acked.map_or(0, |i| i + 1)..] {
            allowed.insert(state(o));
        }
        if !allowed.contains(found) {
            return Err(format!(
                "{map}/{key} recovered as {found}, allowed {allowed:?}, ops {ops:?}"
            ));
        }
    }
    for ((map, key, tag), ops) in &by_tag {
        let found = &dump.values[&(map.clone(), key.clone())];
        let live = found
            .strip_prefix("or:")
            .and_then(|rest| rest.split('|').next())
            .is_some_and(|live| live.split(',').any(|t| t == tag));
        let allowed = allowed_states(ops);
        if !allowed.contains(&live) {
            return Err(format!(
                "{map}/{key} tag {tag} recovered live={live}, allowed {allowed:?}, value {found}, \
                 ops {ops:?}"
            ));
        }
    }
    Ok(())
}

/// The last length the fsync log holds for each segment file, by file name.
fn logged_lengths(root: &Path) -> BTreeMap<String, u64> {
    let text = std::fs::read_to_string(root.join("fsync-log")).unwrap_or_default();
    let mut lengths = BTreeMap::new();
    for line in text.lines() {
        let (path, length) = line.split_once('\t').expect("an fsync-log line");
        let name = Path::new(path)
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        lengths.insert(name, length.parse::<u64>().expect("a length"));
    }
    lengths
}

/// Cuts every segment file of the WAL back to its last logged fsync length,
/// 0 for a file with none; returns the highest frame sequence left.
fn cut_to_last_fsync(root: &Path) -> u64 {
    let lengths = logged_lengths(root);
    let wal_dir = root.join("wal");
    let mut partitions = BTreeSet::new();
    for entry in std::fs::read_dir(&wal_dir).expect("read the WAL dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((partition, _)) = super::segment::parse_segment_filename(&name) {
            partitions.insert(partition);
            let keep = lengths.get(&name).copied().unwrap_or(0);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(entry.path())
                .expect("open a segment");
            if file.metadata().expect("segment metadata").len() > keep {
                file.set_len(keep).expect("cut a segment");
            }
        }
    }
    partitions
        .into_iter()
        .filter_map(|p| {
            super::store_mode_tests::frames_on_disk(&wal_dir, p)
                .last()
                .copied()
        })
        .max()
        .unwrap_or(0)
}

/// What a crash loop of one policy observed.
#[derive(Debug)]
struct LoopReport {
    kills: u64,
    /// Incarnations after whose cut the highest row stood above the highest
    /// frame left in the WAL.
    below_w: u64,
    acked: usize,
}

/// Runs `kills` incarnations of one policy on one root; `held_at` names the
/// incarnation whose child holds every batched fsync.
fn crash_loop(
    test: &str,
    policy: WalFsyncPolicy,
    seed: u64,
    kills: u64,
    held_at: Option<u64>,
) -> LoopReport {
    let root = Root::new();
    let mut rng = SplitMix(seed);
    let mut rows_seen: BTreeMap<u32, u64> = BTreeMap::new();
    // No sequence at or below this may be handed out by the next incarnation.
    let mut floor = 0u64;
    let mut report = LoopReport {
        kills: 0,
        below_w: 0,
        acked: 0,
    };
    let fail = |incarnation: u64, what: String| -> ! {
        panic!(
            "crash loop {} seed {seed} incarnation {incarnation}: {what} (root {})",
            policy_name(policy),
            root.path.display()
        )
    };
    for incarnation in 0..kills {
        let hold = held_at == Some(incarnation);
        let child_seed = rng.next();
        let delay = Duration::from_millis(50 + rng.below(351));
        let scenario = format!(
            "write\t{}\t{incarnation}\t{child_seed}\t{hold}",
            policy_name(policy)
        );
        let mut child = spawn_child(test, &scenario, &root.path);
        let marker = root.path.join(format!(
            "{}-{incarnation}",
            if hold { "held" } else { "ready" }
        ));
        let deadline = Instant::now() + CHILD_DEADLINE;
        while !marker.exists() {
            if let Some(status) = child.try_wait().expect("try_wait") {
                fail(
                    incarnation,
                    format!("the writer ended by itself with {status:?}"),
                );
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                fail(incarnation, "the writer never got ready".to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !hold {
            std::thread::sleep(delay);
        }
        if let Some(status) = child.try_wait().expect("try_wait") {
            fail(
                incarnation,
                format!("the writer ended by itself with {status:?}"),
            );
        }
        child.kill().expect("SIGKILL the writer");
        child.wait().expect("reap the writer");
        report.kills += 1;

        let ops = read_ops(&root.path);
        let mine: Vec<u64> = ops
            .iter()
            .filter(|o| o.incarnation == incarnation)
            .filter_map(|o| o.acked)
            .collect();
        report.acked += mine.len();
        if let Some(reused) = mine.iter().find(|s| **s <= floor) {
            fail(
                incarnation,
                format!("sequence {reused} handed out again: none at or below {floor} may be"),
            );
        }
        if hold && mine.len() as u64 != HELD_OPS {
            fail(
                incarnation,
                format!("the held writer acknowledged {} ops", mine.len()),
            );
        }

        let highest_left = cut_to_last_fsync(&root.path);
        let mut observations = vec![(
            "after the cut",
            rows_of(&dump_at(&root)),
            super::store_mode_tests::read_collected(&root.wal_dir()).unwrap_or_default(),
        )];
        if observations[0].1.values().copied().max().unwrap_or(0) > highest_left {
            report.below_w += 1;
        }

        let mut dumps = Vec::new();
        for name in ["a", "b"] {
            let file = format!("dump-{incarnation}-{name}");
            let status = wait_child(spawn_child(
                test,
                &format!("dump\t{}\t{file}", policy_name(policy)),
                &root.path,
            ));
            if !status.success() {
                fail(incarnation, format!("dump {name} ended with {status:?}"));
            }
            let dump = parse_dump(&std::fs::read_to_string(root.path.join(&file)).expect("dump"));
            observations.push(("a dump", dump.rows.clone(), Vec::new()));
            dumps.push(dump);
        }
        let collected: BTreeMap<u32, u64> = dumps[1].collected.clone();
        if dumps[1].unapplied != 0 {
            fail(
                incarnation,
                format!("the second open replays {} frames", dumps[1].unapplied),
            );
        }
        let mut second = dumps[1].clone();
        second.unapplied = dumps[0].unapplied;
        if second != dumps[0] {
            fail(
                incarnation,
                format!(
                    "the second open changed the store: {:?} then {:?}",
                    dumps[0], dumps[1]
                ),
            );
        }
        for (what, rows, collected_now) in &observations {
            for (partition, w) in rows {
                let before = rows_seen.get(partition).copied().unwrap_or(0);
                if *w < before {
                    fail(
                        incarnation,
                        format!("W({partition}) went down from {before} to {w} ({what})"),
                    );
                }
                rows_seen.insert(*partition, *w);
            }
            for (partition, g) in collected_now.iter().enumerate() {
                let w = rows
                    .get(&u32::try_from(partition).expect("p"))
                    .copied()
                    .unwrap_or(0);
                if *g > w {
                    fail(
                        incarnation,
                        format!("G({partition}) {g} above W {w} ({what})"),
                    );
                }
            }
        }
        for (partition, g) in &collected {
            let w = dumps[1].rows.get(partition).copied().unwrap_or(0);
            if *g > w {
                fail(
                    incarnation,
                    format!("G({partition}) {g} above W {w} (the dump)"),
                );
            }
        }
        if policy == WalFsyncPolicy::PerOp {
            if let Err(what) = check_acked_recovered(&ops, &dumps[0]) {
                fail(
                    incarnation,
                    format!("an acknowledged op was not recovered: {what}"),
                );
            }
        }
        floor = floor
            .max(dumps[1].rows.values().copied().max().unwrap_or(0))
            .max(highest_left);
        if dumps[1].seed <= floor {
            fail(
                incarnation,
                format!("the open seeds {} at or below {floor}", dumps[1].seed),
            );
        }
    }
    report
}

/// The seed of a crash loop: `TOPGUN_TEST_C90_SEED` when set, else the clock.
fn loop_seed(offset: u64) -> u64 {
    std::env::var("TOPGUN_TEST_C90_SEED")
        .ok()
        .map_or_else(now_millis, |s| {
            s.parse().expect("TOPGUN_TEST_C90_SEED is a number")
        })
        .wrapping_add(offset)
}

/// Runs a child of the crash loop if this process is one; returns whether
/// it was.
async fn crash_loop_child() -> bool {
    let Some((scenario, root)) = child_spec() else {
        return false;
    };
    let fields: Vec<&str> = scenario.split('\t').collect();
    let policy = match fields[1] {
        "per_op" => WalFsyncPolicy::PerOp,
        "batched" => WalFsyncPolicy::Batched,
        other => panic!("unknown policy {other}"),
    };
    match fields[0] {
        "write" => {
            crash_loop_writer(
                &root,
                policy,
                fields[2].parse().expect("incarnation"),
                fields[3].parse().expect("seed"),
                fields[4] == "true",
            )
            .await;
        }
        "dump" => crash_loop_dump(&root, policy, fields[2]).await,
        other => panic!("unknown crash-loop scenario {other}"),
    }
    true
}

async fn run_crash_loops(test: &str, kills: u64) {
    if crash_loop_child().await {
        return;
    }
    let started = Instant::now();
    let per_op_seed = loop_seed(0);
    let batched_seed = loop_seed(1);
    let per_op = tokio::task::block_in_place(|| {
        crash_loop(test, WalFsyncPolicy::PerOp, per_op_seed, kills, None)
    });
    // The second incarnation holds every batched fsync, so the rows it commits
    // stand above every frame its cut leaves.
    let batched = tokio::task::block_in_place(|| {
        crash_loop(test, WalFsyncPolicy::Batched, batched_seed, kills, Some(1))
    });
    println!(
        "{test}: per_op seed {per_op_seed} kills {} acked {} below-W incarnations {}; batched seed \
         {batched_seed} kills {} acked {} below-W incarnations {}; {:?}",
        per_op.kills,
        per_op.acked,
        per_op.below_w,
        batched.kills,
        batched.acked,
        batched.below_w,
        started.elapsed()
    );
    assert!(
        batched.below_w > 0,
        "no batched incarnation ended with a row above the frames its cut left (seed {batched_seed})"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn c90_crash_loop() {
    run_crash_loops("c90_crash_loop", 3).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "50 kills per policy; run on its own with --ignored"]
async fn c90_crash_loop_long() {
    run_crash_loops("c90_crash_loop_long", 50).await;
}
