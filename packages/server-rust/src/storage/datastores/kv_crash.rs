//! Crash suite harness for [`KvEngine`]s and the map registry above them
//! (`TG-ENG-001`, `TG-ENG-005`, `TG-ENG-006`).
//!
//! A crash keeps everything synced and nothing, or a random part, of what was
//! written after the last sync. [`CrashableBackend`] is a `redb` storage
//! backend with exactly that model; [`CrashTarget`] is the seam every engine's
//! crash model implements; the drivers run a seeded or fixed workload, kill
//! the engine at a chosen backend op, crash it, reopen it and check that the
//! recovered state is a prefix of the called batches holding every durable
//! one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::io;
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::engine_store::{
    decode_map_by_id_row, decode_map_by_name_row, EngineMapDataStore, RecordKey, RecordOp,
    RecordSpace, KS_BACKUP, KS_MAP_BY_ID, KS_MAP_BY_NAME, KS_META, KS_PRIMARY, META_MIGRATION,
    META_NEXT_MAP_ID,
};
use super::kv_engine::{BatchOp, KeyRange, KvEngine, MemKvEngine, WriteBatch, WriteOptions};
use super::redb_engine::RedbKvEngine;
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

/// What a crash keeps of the writes made after the last sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrashMode {
    /// Nothing (Pebble's `CrashableMem`).
    DropUnsynced,
    /// Each unsynced op with probability ½, in order.
    KeepRandomWrites,
    /// As `KeepRandomWrites`, but each kept write is torn into 512-byte
    /// sectors, each kept with probability ½.
    KeepRandomSectors,
}

/// Every crash mode, in a fixed order.
pub(crate) const CRASH_MODES: [CrashMode; 3] = [
    CrashMode::DropUnsynced,
    CrashMode::KeepRandomWrites,
    CrashMode::KeepRandomSectors,
];

/// The unit a torn write is cut into, aligned to the file offset.
const SECTOR: u64 = 512;

/// A backend op since the last `sync_data(false)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingOp {
    /// `write(offset, bytes)`.
    Write {
        /// Where the bytes go.
        offset: u64,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// `set_len(len)`.
    SetLen {
        /// The new length.
        len: u64,
    },
    /// `sync_data(true)`: an ordering barrier that makes nothing durable.
    Barrier,
}

#[derive(Default)]
struct BackendState {
    /// What reads see.
    live: Vec<u8>,
    /// What survives any crash.
    durable: Vec<u8>,
    pending: Vec<PendingOp>,
    /// Counted ops entered so far.
    ops: u64,
    die_at: Option<u64>,
    dead: bool,
    eventual_syncs: u64,
}

impl BackendState {
    /// Entry of a counted op: its index is the counter at entry; the death
    /// latch is checked against that index, before anything is applied.
    fn enter_counted(&mut self) -> io::Result<()> {
        let index = self.ops;
        self.ops += 1;
        if self.die_at.is_some_and(|n| index >= n) {
            self.dead = true;
        }
        self.enter()
    }

    fn enter(&self) -> io::Result<()> {
        if self.dead {
            Err(io::Error::from_raw_os_error(5))
        } else {
            Ok(())
        }
    }
}

/// A `redb` storage backend that keeps synced bytes and loses unsynced ones
/// at a crash. Clones share one state.
#[derive(Clone)]
pub(crate) struct CrashableBackend {
    state: Arc<Mutex<BackendState>>,
}

impl fmt::Debug for CrashableBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the bytes: a file image is megabytes.
        let state = self.state.lock();
        f.debug_struct("CrashableBackend")
            .field("live_len", &state.live.len())
            .field("durable_len", &state.durable.len())
            .field("pending", &state.pending.len())
            .field("ops", &state.ops)
            .field("die_at", &state.die_at)
            .field("dead", &state.dead)
            .finish()
    }
}

fn offset_to_index(offset: u64) -> usize {
    usize::try_from(offset).expect("a test file offset fits in usize")
}

fn apply_write(image: &mut Vec<u8>, offset: u64, bytes: &[u8]) {
    let start = offset_to_index(offset);
    let end = start + bytes.len();
    if image.len() < end {
        image.resize(end, 0);
    }
    image[start..end].copy_from_slice(bytes);
}

/// A write a crash kept: only a kept `set_len` changes the image's length, so
/// the bytes past the end are lost. redb never writes past the end of its
/// file, and a journaling file system persists a length only as the metadata
/// change that made it, never as the end of a write or sector that landed.
fn persist_write(image: &mut [u8], offset: u64, bytes: &[u8]) {
    let start = offset_to_index(offset).min(image.len());
    let end = (start + bytes.len()).min(image.len());
    image[start..end].copy_from_slice(&bytes[..end - start]);
}

impl CrashableBackend {
    /// An empty, live backend that is never killed until `set_die_at`.
    pub(crate) fn new() -> Self {
        Self::with_image(Vec::new())
    }

    fn with_image(image: Vec<u8>) -> Self {
        Self {
            state: Arc::new(Mutex::new(BackendState {
                live: image.clone(),
                durable: image,
                ..BackendState::default()
            })),
        }
    }

    /// Kills the counted op with index `at` (or the next one, if the counter
    /// is already past it) and every later call; `None` disarms.
    pub(crate) fn set_die_at(&self, at: Option<u64>) {
        self.state.lock().die_at = at;
    }

    /// Counted ops (`write`, `set_len`, `sync_data`) entered so far.
    pub(crate) fn ops_so_far(&self) -> u64 {
        self.state.lock().ops
    }

    /// `sync_data(true)` calls seen so far.
    pub(crate) fn eventual_syncs(&self) -> u64 {
        self.state.lock().eventual_syncs
    }

    /// The backend a crash leaves, then releases this handle's state.
    ///
    /// The image is `durable` plus, by `mode`, a seeded part of the pending
    /// ops; only a kept `set_len` changes its length. This handle's state is
    /// then emptied and dead, so every clone answers EIO and no file image
    /// outlives the crash, whatever the dead engine still holds.
    pub(crate) fn crash_image(&self, mode: CrashMode, seed: u64) -> CrashableBackend {
        let mut state = self.state.lock();
        let mut image = state.durable.clone();
        if mode != CrashMode::DropUnsynced {
            let mut rng = StdRng::seed_from_u64(seed);
            for op in &state.pending {
                if matches!(op, PendingOp::Barrier) || !rng.random_bool(0.5) {
                    continue;
                }
                match op {
                    PendingOp::Write { offset, bytes } if mode == CrashMode::KeepRandomSectors => {
                        let mut at = *offset;
                        let mut rest = bytes.as_slice();
                        while !rest.is_empty() {
                            let room = offset_to_index(SECTOR - at % SECTOR);
                            let (sector, tail) = rest.split_at(room.min(rest.len()));
                            if rng.random_bool(0.5) {
                                persist_write(&mut image, at, sector);
                            }
                            at += sector.len() as u64;
                            rest = tail;
                        }
                    }
                    PendingOp::Write { offset, bytes } => persist_write(&mut image, *offset, bytes),
                    PendingOp::SetLen { len } => image.resize(offset_to_index(*len), 0),
                    PendingOp::Barrier => {}
                }
            }
        }
        state.live = Vec::new();
        state.durable = Vec::new();
        state.pending = Vec::new();
        state.dead = true;
        drop(state);
        Self::with_image(image)
    }

    /// Bytes this handle's state keeps alive: both images and every pending write.
    pub(crate) fn retained_bytes(&self) -> usize {
        let state = self.state.lock();
        let pending: usize = state
            .pending
            .iter()
            .map(|op| match op {
                PendingOp::Write { bytes, .. } => bytes.len(),
                PendingOp::SetLen { .. } | PendingOp::Barrier => 0,
            })
            .sum();
        state.live.capacity() + state.durable.capacity() + pending
    }
}

impl redb::StorageBackend for CrashableBackend {
    fn len(&self) -> io::Result<u64> {
        let state = self.state.lock();
        state.enter()?;
        Ok(state.live.len() as u64)
    }

    fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let state = self.state.lock();
        state.enter()?;
        let start = offset_to_index(offset);
        state
            .live
            .get(start..start + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let mut state = self.state.lock();
        state.enter_counted()?;
        state.live.resize(offset_to_index(len), 0);
        state.pending.push(PendingOp::SetLen { len });
        Ok(())
    }

    fn sync_data(&self, eventual: bool) -> io::Result<()> {
        let mut state = self.state.lock();
        state.enter_counted()?;
        if eventual {
            state.eventual_syncs += 1;
            state.pending.push(PendingOp::Barrier);
        } else {
            let BackendState {
                live,
                durable,
                pending,
                ..
            } = &mut *state;
            durable.clone_from(live);
            pending.clear();
        }
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        let mut state = self.state.lock();
        state.enter_counted()?;
        apply_write(&mut state.live, offset, data);
        state.pending.push(PendingOp::Write {
            offset,
            bytes: data.to_vec(),
        });
        Ok(())
    }
}

/// An engine with a crash model the suite can drive.
pub(crate) trait CrashTarget: Sized {
    /// The engine under test.
    type Engine: KvEngine;
    /// Names the target in failure reports.
    const NAME: &'static str;
    /// `true` for a target whose crash keeps whole unsynced batches and reads
    /// `crash`'s `seed` as how many of them to keep (`mode` is ignored);
    /// `false` for one that tears bytes by `mode` and `seed`.
    const KEEPS_WHOLE_BATCHES: bool;
    /// A fresh, open, empty engine; its death is not armed.
    fn fresh() -> Self;
    /// The engine, shared with the caller.
    fn engine(&self) -> Arc<Self::Engine>;
    /// Kills the counted op with index `at_op` and everything after it.
    fn arm_death(&self, at_op: u64);
    /// Counted ops entered so far.
    fn ops_so_far(&self) -> u64;
    /// What a crash leaves, reopened. Every other holder of the engine must
    /// have dropped it.
    fn crash(self, mode: CrashMode, seed: u64) -> Self;
}

/// `RedbKvEngine` over a [`CrashableBackend`].
pub(crate) struct RedbTarget {
    backend: CrashableBackend,
    engine: Arc<RedbKvEngine>,
}

impl CrashTarget for RedbTarget {
    type Engine = RedbKvEngine;
    const NAME: &'static str = "redb";
    const KEEPS_WHOLE_BATCHES: bool = false;

    fn fresh() -> Self {
        let backend = CrashableBackend::new();
        let engine = RedbKvEngine::with_backend(backend.clone())
            .unwrap_or_else(|e| panic!("redb over a fresh crashable backend: {e}"));
        Self {
            backend,
            engine: Arc::new(engine),
        }
    }

    fn engine(&self) -> Arc<RedbKvEngine> {
        Arc::clone(&self.engine)
    }

    fn arm_death(&self, at_op: u64) {
        self.backend.set_die_at(Some(at_op));
    }

    fn ops_so_far(&self) -> u64 {
        self.backend.ops_so_far()
    }

    fn crash(self, mode: CrashMode, seed: u64) -> Self {
        let Self { backend, engine } = self;
        assert_eq!(
            backend.eventual_syncs(),
            0,
            "STOP 3: the engine issued an Eventual sync barrier"
        );
        // The image first: the dead engine's destructor must not reach it.
        let image = backend.crash_image(mode, seed);
        assert_eq!(
            backend.retained_bytes(),
            0,
            "the crashed backend still holds file bytes"
        );
        assert_eq!(
            Arc::strong_count(&engine),
            1,
            "the dead engine is still shared, so dropping it here would not run its destructor"
        );
        drop(engine);
        let reopened = RedbKvEngine::with_backend(image.clone())
            .unwrap_or_else(|e| panic!("reopen after the crash failed: {e}"));
        Self {
            backend: image,
            engine: Arc::new(reopened),
        }
    }
}

/// The reference engine and its whole-batch crash image.
pub(crate) struct MemTarget {
    engine: Arc<MemKvEngine>,
}

impl CrashTarget for MemTarget {
    type Engine = MemKvEngine;
    const NAME: &'static str = "mem";
    const KEEPS_WHOLE_BATCHES: bool = true;

    fn fresh() -> Self {
        Self {
            engine: Arc::new(MemKvEngine::new()),
        }
    }

    fn engine(&self) -> Arc<MemKvEngine> {
        Arc::clone(&self.engine)
    }

    fn arm_death(&self, at_op: u64) {
        self.engine.arm_death(at_op);
    }

    fn ops_so_far(&self) -> u64 {
        self.engine.calls_so_far()
    }

    fn crash(self, _mode: CrashMode, seed: u64) -> Self {
        let keep = usize::try_from(seed).unwrap_or(usize::MAX);
        let image = self.engine.crash_image(keep);
        assert_eq!(
            Arc::strong_count(&self.engine),
            1,
            "the dead engine is still shared"
        );
        drop(self.engine);
        Self {
            engine: Arc::new(image),
        }
    }
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

/// What a run did, per call in call order.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunRecord {
    /// The call returned `Ok`.
    pub(crate) ok: Vec<bool>,
    /// The call is one unit of the prefix (a batch, or a store operation).
    pub(crate) counted: Vec<bool>,
    /// An `Ok` of the call makes every earlier unit durable.
    pub(crate) durable: Vec<bool>,
}

impl RunRecord {
    fn push(&mut self, ok: bool, counted: bool, durable: bool) {
        self.ok.push(ok);
        self.counted.push(counted);
        self.durable.push(durable);
    }

    /// `d`: the units called up to the last durable call that returned
    /// before the first failure.
    pub(crate) fn durable_units(&self) -> usize {
        let mut units = 0;
        let mut durable = 0;
        for ((&ok, &counted), &makes_durable) in
            self.ok.iter().zip(&self.counted).zip(&self.durable)
        {
            if counted {
                units += 1;
            }
            if !ok {
                break;
            }
            if makes_durable {
                durable = units;
            }
        }
        durable
    }

    /// The longest prefix a crash may leave: every unit before the first
    /// failed call, plus that call when it is a unit (it was in flight).
    pub(crate) fn upper_units(&self) -> usize {
        let mut units = 0;
        for (&ok, &counted) in self.ok.iter().zip(&self.counted) {
            if counted {
                units += 1;
            }
            if !ok {
                return units;
            }
        }
        units
    }
}

/// A workload with a checker: what one crash case runs and checks.
pub(crate) trait Scenario {
    /// Runs the workload against `engine`, recording every call.
    fn run<E: KvEngine>(&self, engine: &Arc<E>) -> RunRecord;
    /// The prefix lengths in `d..=upper` the recovered engine matches, or why none does.
    fn check<E: KvEngine>(&self, engine: &Arc<E>, run: &RunRecord) -> Result<Vec<usize>, String>;
    /// `B`: the units of the whole workload.
    fn units(&self) -> usize;
}

/// One crash case's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaseOutcome {
    /// `d`.
    pub(crate) durable: usize,
    /// The longest admissible prefix.
    pub(crate) upper: usize,
    /// Every admissible prefix length the recovered state equals.
    pub(crate) matches: Vec<usize>,
    /// Units of the whole workload.
    pub(crate) units: usize,
}

/// Totals of a sweep or a random series.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Report {
    /// Crash cases run.
    pub(crate) cases: usize,
    /// Cases whose recovered state is shorter than the whole workload.
    pub(crate) lossy: usize,
    /// Cases whose recovered state is longer than `d`.
    pub(crate) beyond_durable: usize,
    /// `T` of the dry run (the sweep) or the largest `T_s` (a random series).
    pub(crate) ops: u64,
}

impl Report {
    fn add(&mut self, outcome: &CaseOutcome) {
        self.cases += 1;
        let longest = outcome.matches.iter().copied().max().unwrap_or(0);
        if longest < outcome.units {
            self.lossy += 1;
        }
        if longest > outcome.durable {
            self.beyond_durable += 1;
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "non-text panic".to_string())
}

/// One case: fresh target, death armed at `die_at` (never, for `None`), run,
/// crash, check. Any failure panics with the target, crash point, mode and
/// seed.
pub(crate) fn crash_case<T: CrashTarget, S: Scenario>(
    scenario: &S,
    die_at: Option<u64>,
    mode: CrashMode,
    seed: u64,
) -> CaseOutcome {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let target = T::fresh();
        if let Some(at) = die_at {
            target.arm_death(at);
        }
        let run = scenario.run(&target.engine());
        let target = target.crash(mode, seed);
        let engine = target.engine();
        let matches = scenario.check(&engine, &run)?;
        Ok::<_, String>(CaseOutcome {
            durable: run.durable_units(),
            upper: run.upper_units(),
            matches,
            units: scenario.units(),
        })
    }));
    let context = format!(
        "{} crash case: die_at {die_at:?}, mode {mode:?}, seed {seed}",
        T::NAME
    );
    match result {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(why)) => panic!("{context}: {why}"),
        Err(payload) => panic!("{context}: panicked: {}", panic_text(&*payload)),
    }
}

/// `T`: the counted ops after a full run of `scenario` on a fresh target.
pub(crate) fn dry_run_ops<T: CrashTarget, S: Scenario>(scenario: &S) -> u64 {
    let target = T::fresh();
    let run = scenario.run(&target.engine());
    assert!(
        run.ok.iter().all(|&ok| ok),
        "{}: a call failed in a run without a death",
        T::NAME
    );
    target.ops_so_far()
}

/// The crash variants of one point: every mode (a byte-tearing target) or
/// every number of unsynced batches to keep in `0..=B` (a whole-batch one).
fn variants<T: CrashTarget>(units: usize, point: u64) -> Vec<(CrashMode, u64)> {
    if T::KEEPS_WHOLE_BATCHES {
        (0..=units as u64)
            .map(|keep| (CrashMode::DropUnsynced, keep))
            .collect()
    } else {
        (0u64..)
            .zip(CRASH_MODES)
            .map(|(index, mode)| (mode, point.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ index))
            .collect()
    }
}

/// Every crash point `n ∈ 0..=T` of `scenario`, each in every variant.
pub(crate) fn every_crash_point<T: CrashTarget, S: Scenario>(scenario: &S) -> Report {
    let ops = dry_run_ops::<T, S>(scenario);
    let mut report = Report {
        ops,
        ..Report::default()
    };
    for point in 0..=ops {
        for (mode, seed) in variants::<T>(scenario.units(), point) {
            report.add(&crash_case::<T, S>(scenario, Some(point), mode, seed));
        }
    }
    report
}

/// Seeded random crashes: per seed, a dry run counts `T_s` and the death
/// point is uniform in `0..=T_s`. A byte-tearing target runs every mode
/// (`every_mode`) or one drawn from the seed; a whole-batch target keeps a
/// number of batches uniform in `0..=B`.
pub(crate) fn random_crashes<T: CrashTarget, S: Scenario>(
    seeds: Range<u64>,
    every_mode: bool,
    make: impl Fn(u64) -> S,
) -> Report {
    let mut report = Report::default();
    for seed in seeds {
        let scenario = make(seed);
        let ops = dry_run_ops::<T, S>(&scenario);
        report.ops = report.ops.max(ops);
        let mut rng = StdRng::seed_from_u64(seed ^ 0xC0A5_7E57_0000_0000);
        let choices: Vec<(CrashMode, u64)> = if T::KEEPS_WHOLE_BATCHES {
            vec![(
                CrashMode::DropUnsynced,
                rng.random_range(0..=scenario.units() as u64),
            )]
        } else if every_mode {
            CRASH_MODES
                .iter()
                .map(|&mode| (mode, rng.random()))
                .collect()
        } else {
            vec![(
                CRASH_MODES[rng.random_range(0..CRASH_MODES.len())],
                rng.random(),
            )]
        };
        for (mode, crash_seed) in choices {
            let point = rng.random_range(0..=ops);
            report.add(&crash_case::<T, S>(
                &scenario,
                Some(point),
                mode,
                crash_seed,
            ));
        }
    }
    report
}

// ---------------------------------------------------------------------------
// Engine workload (R2)
// ---------------------------------------------------------------------------

/// One step of an engine workload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// One `write`.
    Batch {
        /// Its ops, in order.
        ops: Vec<BatchOp>,
        /// `WriteOptions::sync`.
        sync: bool,
    },
    /// One `sync()`.
    Sync,
}

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

fn apply_model(model: &mut Model, ops: &[BatchOp]) {
    for op in ops {
        match op {
            BatchOp::Put { key, value } => {
                model.insert(key.clone(), value.clone());
            }
            BatchOp::Delete { key } => {
                model.remove(key);
            }
            BatchOp::DeleteRange { start, end } => {
                model.retain(|key, _| key < start || key >= end);
            }
        }
    }
}

/// An engine workload: batches and syncs against the bare engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KvWorkload {
    /// The steps, in call order.
    pub(crate) steps: Vec<Step>,
}

fn value_of(len: usize, tag: u8) -> Vec<u8> {
    (0..len)
        .map(|i| tag.wrapping_add(u8::try_from(i % 251).unwrap_or(0)))
        .collect()
}

fn put(key: &str, len: usize, tag: u8) -> BatchOp {
    BatchOp::Put {
        key: key.as_bytes().to_vec(),
        value: value_of(len, tag),
    }
}

fn delete(key: &str) -> BatchOp {
    BatchOp::Delete {
        key: key.as_bytes().to_vec(),
    }
}

fn delete_range(start: &str, end: &str) -> BatchOp {
    BatchOp::DeleteRange {
        start: start.as_bytes().to_vec(),
        end: end.as_bytes().to_vec(),
    }
}

fn batch(ops: Vec<BatchOp>, sync: bool) -> Step {
    Step::Batch { ops, sync }
}

impl KvWorkload {
    /// The fixed 12-batch workload over keys `a`…`h`: multi-op batches, a
    /// `sync()` after unsynced batches followed by more unsynced ones before
    /// the next durable write, and a trailing unsynced batch (`d = 11`).
    pub(crate) fn fixed() -> Self {
        let steps = vec![
            batch(
                vec![put("a", 16, 1), put("b", 300, 2), put("c", 2048, 3)],
                true,
            ),
            batch(vec![put("d", 700, 4), put("e", 64, 5), delete("a")], false),
            batch(vec![delete_range("b", "d"), put("f", 1500, 6)], false),
            Step::Sync,
            batch(
                vec![
                    put("a", 900, 7),
                    put("b", 33, 8),
                    put("g", 2048, 9),
                    put("h", 128, 10),
                ],
                false,
            ),
            batch(vec![put("c", 512, 11)], false),
            batch(
                vec![
                    put("d", 1024, 12),
                    put("e", 17, 13),
                    delete("f"),
                    delete("g"),
                ],
                true,
            ),
            batch(vec![delete_range("a", "c")], false),
            batch(
                vec![put("f", 256, 14), put("g", 2000, 15), put("a", 48, 16)],
                false,
            ),
            Step::Sync,
            batch(vec![put("b", 1800, 17)], false),
            batch(
                vec![
                    put("c", 40, 18),
                    put("d", 2048, 19),
                    put("e", 600, 20),
                    put("h", 1200, 21),
                    put("b", 77, 22),
                    delete("a"),
                ],
                false,
            ),
            batch(vec![put("g", 96, 23), put("a", 1000, 24)], true),
            batch(vec![put("h", 444, 25)], false),
        ];
        Self { steps }
    }

    /// A seeded workload: 10–60 steps; a step is `sync()` (10 %) or a batch
    /// of 1–8 puts, deletes and range deletes over 64 keys of 1–6 bytes with
    /// values of 0–2 KiB, written with `sync = true` (30 %) or `false`.
    pub(crate) fn random(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut keys = BTreeSet::new();
        while keys.len() < 64 {
            let len = rng.random_range(1..=6);
            keys.insert((0..len).map(|_| rng.random::<u8>()).collect::<Vec<u8>>());
        }
        let keys: Vec<Vec<u8>> = keys.into_iter().collect();
        let steps = (0..rng.random_range(10..=60))
            .map(|_| {
                if rng.random_bool(0.1) {
                    return Step::Sync;
                }
                let ops = (0..rng.random_range(1..=8))
                    .map(|_| {
                        let key = keys[rng.random_range(0..keys.len())].clone();
                        match rng.random_range(0..20) {
                            0..=11 => {
                                let len = rng.random_range(0..=2048);
                                let mut value = vec![0u8; len];
                                rng.fill(value.as_mut_slice());
                                BatchOp::Put { key, value }
                            }
                            12..=16 => BatchOp::Delete { key },
                            _ => {
                                let other = keys[rng.random_range(0..keys.len())].clone();
                                let (start, end) = if key <= other {
                                    (key, other)
                                } else {
                                    (other, key)
                                };
                                BatchOp::DeleteRange { start, end }
                            }
                        }
                    })
                    .collect();
                batch(ops, rng.random_bool(0.3))
            })
            .collect();
        Self { steps }
    }

    /// The model after each prefix of batches: entry `i` is the state after
    /// the first `i`.
    pub(crate) fn models(&self) -> Vec<BTreeMap<Vec<u8>, Vec<u8>>> {
        let mut model = Model::new();
        let mut models = vec![model.clone()];
        for step in &self.steps {
            if let Step::Batch { ops, .. } = step {
                apply_model(&mut model, ops);
                models.push(model.clone());
            }
        }
        models
    }
}

/// Every key of `engine`, read from one snapshot.
pub(crate) fn read_all<E: KvEngine>(engine: &E) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
    let snapshot = engine
        .snapshot()
        .map_err(|e| format!("snapshot of the recovered engine: {e}"))?;
    let mut state = BTreeMap::new();
    for entry in snapshot
        .iter(KeyRange::default())
        .map_err(|e| format!("iter of the recovered engine: {e}"))?
    {
        let (key, value) = entry.map_err(|e| format!("read of the recovered engine: {e}"))?;
        if state.insert(key.clone(), value).is_some() {
            return Err(format!("the recovered engine returned {key:?} twice"));
        }
    }
    Ok(state)
}

fn describe_diff(state: &Model, model: &Model) -> String {
    let mut lines = Vec::new();
    for (key, value) in state {
        match model.get(key) {
            None => lines.push(format!("extra {key:?} ({} bytes)", value.len())),
            Some(expected) if expected != value => lines.push(format!(
                "{key:?}: {} bytes, the model has {} bytes",
                value.len(),
                expected.len()
            )),
            Some(_) => {}
        }
    }
    for key in model.keys().filter(|key| !state.contains_key(*key)) {
        lines.push(format!("missing {key:?}"));
    }
    let shown = lines.len().min(6);
    format!(
        "{} (of {} differences)",
        lines[..shown].join("; "),
        lines.len()
    )
}

impl Scenario for KvWorkload {
    fn run<E: KvEngine>(&self, engine: &Arc<E>) -> RunRecord {
        let mut run = RunRecord::default();
        for step in &self.steps {
            match step {
                Step::Batch { ops, sync } => {
                    let mut batch = WriteBatch::new();
                    for op in ops {
                        match op {
                            BatchOp::Put { key, value } => batch.put(key.clone(), value.clone()),
                            BatchOp::Delete { key } => batch.delete(key.clone()),
                            BatchOp::DeleteRange { start, end } => {
                                batch.delete_range(start.clone(), end.clone());
                            }
                        }
                    }
                    let ok = engine.write(batch, WriteOptions { sync: *sync }).is_ok();
                    run.push(ok, true, *sync);
                }
                Step::Sync => run.push(engine.sync().is_ok(), false, true),
            }
        }
        run
    }

    fn check<E: KvEngine>(&self, engine: &Arc<E>, run: &RunRecord) -> Result<Vec<usize>, String> {
        let state = read_all(&**engine)?;
        let models = self.models();
        let durable = run.durable_units();
        let upper = run.upper_units();
        let matches: Vec<usize> = (durable..=upper).filter(|&i| models[i] == state).collect();
        if matches.is_empty() {
            let diffs: Vec<String> = (durable..=upper)
                .map(|i| format!("prefix {i}: {}", describe_diff(&state, &models[i])))
                .collect();
            return Err(format!(
                "the recovered state is no prefix in {durable}..={upper} of {} batches: {}",
                self.units(),
                diffs.join(" | ")
            ));
        }
        Ok(matches)
    }

    fn units(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, Step::Batch { .. }))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Registry workload (R3)
// ---------------------------------------------------------------------------

/// The migration-mark part of one `write_records` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkChange {
    /// No `SetMigrationMark` op.
    Untouched,
    /// `SetMigrationMark(Some(bytes))`.
    Set(Vec<u8>),
    /// `SetMigrationMark(None)`.
    Clear,
}

/// One record op of a `write_records` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordChange {
    /// `RecordOp::Put`.
    Put {
        /// The map.
        map: String,
        /// The backup keyspace.
        backup: bool,
        /// The user key.
        key: String,
        /// The stored bytes.
        bytes: Vec<u8>,
    },
    /// `RecordOp::Delete`.
    Delete {
        /// The map.
        map: String,
        /// The backup keyspace.
        backup: bool,
        /// The user key.
        key: String,
    },
}

/// One store operation after the open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreOp {
    /// `MapDataStore::add` of an LWW value stamped `stamp`.
    Add {
        /// The map.
        map: String,
        /// The key.
        key: String,
        /// Distinguishes the value.
        stamp: u64,
    },
    /// `MapDataStore::add_backup`.
    AddBackup {
        /// The map.
        map: String,
        /// The key.
        key: String,
        /// Distinguishes the value.
        stamp: u64,
    },
    /// `MapDataStore::remove`.
    Remove {
        /// The map.
        map: String,
        /// The key.
        key: String,
    },
    /// `MapDataStore::remove_all`.
    RemoveAll {
        /// The map.
        map: String,
        /// The keys, at least one.
        keys: Vec<String>,
    },
    /// `EngineMapDataStore::write_records`.
    Records {
        /// The record ops, in order.
        changes: Vec<RecordChange>,
        /// The mark op, if any.
        mark: MarkChange,
        /// `WriteOptions::sync`.
        sync: bool,
    },
}

/// A registry workload: operation 0 is the open (the format batch, with the
/// initial mark when `initial_mark` is set); operations `1..` are `ops`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegistryWorkload {
    /// `Some(m0)`: open with `open_for_migration(m0)`; `None`: `open`.
    pub(crate) initial_mark: Option<Vec<u8>>,
    /// The operations after the open.
    pub(crate) ops: Vec<StoreOp>,
}

/// The record-level model of a prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RegistryModel {
    formatted: bool,
    records: BTreeMap<(String, bool, String), Vec<u8>>,
    primary_names: BTreeSet<String>,
    put_names: BTreeSet<String>,
    mark: Option<Vec<u8>>,
}

fn lww_value(stamp: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::String(format!("v{stamp}")),
        timestamp: Timestamp {
            millis: stamp,
            counter: 0,
            node_id: "crash".to_string(),
        },
    }
}

/// The bytes `add` stores for `stamp`.
fn lww_bytes(stamp: u64) -> Vec<u8> {
    rmp_serde::to_vec_named(&lww_value(stamp)).expect("an LWW value encodes")
}

/// Drives a store future that never suspends: every store call is
/// synchronous underneath.
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("a store call suspended; the crash suite polls it once"),
    }
}

fn s(text: &str) -> String {
    text.to_string()
}

fn rec_put(map: &str, backup: bool, key: &str, tag: u8) -> RecordChange {
    RecordChange::Put {
        map: s(map),
        backup,
        key: s(key),
        bytes: value_of(24, tag),
    }
}

impl RegistryModel {
    fn put(&mut self, map: &str, backup: bool, key: &str, bytes: Vec<u8>) {
        self.records.insert((s(map), backup, s(key)), bytes);
        self.put_names.insert(s(map));
        if !backup {
            self.primary_names.insert(s(map));
        }
    }

    fn remove(&mut self, map: &str, backup: bool, key: &str) {
        self.records.remove(&(s(map), backup, s(key)));
    }

    fn apply(&mut self, op: &StoreOp) {
        match op {
            StoreOp::Add { map, key, stamp } => self.put(map, false, key, lww_bytes(*stamp)),
            StoreOp::AddBackup { map, key, stamp } => self.put(map, true, key, lww_bytes(*stamp)),
            StoreOp::Remove { map, key } => self.remove(map, false, key),
            StoreOp::RemoveAll { map, keys } => {
                for key in keys {
                    self.remove(map, false, key);
                }
            }
            StoreOp::Records { changes, mark, .. } => {
                for change in changes {
                    match change {
                        RecordChange::Put {
                            map,
                            backup,
                            key,
                            bytes,
                        } => self.put(map, *backup, key, bytes.clone()),
                        RecordChange::Delete { map, backup, key } => self.remove(map, *backup, key),
                    }
                }
                match mark {
                    MarkChange::Untouched => {}
                    MarkChange::Set(bytes) => self.mark = Some(bytes.clone()),
                    MarkChange::Clear => self.mark = None,
                }
            }
        }
    }
}

/// What the recovered engine holds, decoded, with assertions 1 and 2 checked.
struct RecoveredRegistry {
    empty: bool,
    records: BTreeMap<(String, bool, String), Vec<u8>>,
    names: BTreeSet<String>,
    mark: Option<Vec<u8>>,
}

fn decode_registry(raw: &BTreeMap<Vec<u8>, Vec<u8>>) -> Result<RecoveredRegistry, String> {
    let mut by_name: BTreeMap<String, u32> = BTreeMap::new();
    let mut by_id: BTreeMap<u32, String> = BTreeMap::new();
    let mut next_map_id = None;
    let mut mark = None;
    let mut record_keys = Vec::new();
    for (key, value) in raw {
        match key.first().copied() {
            Some(KS_META) => {
                if key[1..] == *META_NEXT_MAP_ID {
                    let word: [u8; 4] = value
                        .as_slice()
                        .try_into()
                        .map_err(|_| "next_map_id is not 4 bytes".to_string())?;
                    next_map_id = Some(u32::from_be_bytes(word));
                } else if key[1..] == *META_MIGRATION {
                    mark = Some(value.clone());
                }
            }
            Some(KS_MAP_BY_NAME) => {
                let row =
                    decode_map_by_name_row(key, value).map_err(|e| format!("0x01 row: {e}"))?;
                by_name.insert(row.name, row.map_id);
            }
            Some(KS_MAP_BY_ID) => {
                let (id, name) =
                    decode_map_by_id_row(key, value).map_err(|e| format!("0x02 row: {e}"))?;
                by_id.insert(id, name);
            }
            Some(KS_PRIMARY | KS_BACKUP) => record_keys.push((key, value)),
            other => return Err(format!("a key in keyspace {other:?} after a crash")),
        }
    }
    // Assertion 2: the two registry keyspaces are inverse bijections, and
    // `next_map_id` is above every issued id.
    for (name, id) in &by_name {
        if by_id.get(id) != Some(name) {
            return Err(format!("0x01 {name:?} → {id} has no matching 0x02 row"));
        }
    }
    for (id, name) in &by_id {
        if by_name.get(name) != Some(id) {
            return Err(format!("0x02 {id} → {name:?} has no matching 0x01 row"));
        }
    }
    if let Some(max_id) = by_id.keys().max() {
        match next_map_id {
            Some(next) if next > *max_id => {}
            other => {
                return Err(format!(
                    "next_map_id {other:?} does not exceed the issued id {max_id}"
                ))
            }
        }
    }
    // Assertion 1: every record decodes, under an id the registry resolves.
    let mut records = BTreeMap::new();
    for (key, value) in record_keys {
        let record = RecordKey::decode(key).map_err(|e| format!("record key {key:?}: {e}"))?;
        let Some(name) = by_id.get(&record.map_id) else {
            return Err(format!(
                "orphan record {:?} under unregistered id {}",
                record.key, record.map_id
            ));
        };
        records.insert(
            (
                name.clone(),
                record.space == RecordSpace::Backup,
                record.key,
            ),
            value.clone(),
        );
    }
    Ok(RecoveredRegistry {
        empty: raw.is_empty(),
        records,
        names: by_name.into_keys().collect(),
        mark,
    })
}

impl RegistryWorkload {
    /// First fixed workload: `add` to new maps `m1`, `m-2`, `m/3`, then
    /// `add_backup` to `m1`, a 3-op `write_records` (two puts to `m1`, one to
    /// new `m4`), `remove` on `m-2`, `remove_all` on `m/3`, `add` to `m4`.
    pub(crate) fn fixed() -> Self {
        let add = |map: &str, key: &str, stamp| StoreOp::Add {
            map: s(map),
            key: s(key),
            stamp,
        };
        let ops = vec![
            add("m1", "k1", 1),
            add("m-2", "k1", 2),
            add("m/3", "k1", 3),
            StoreOp::AddBackup {
                map: s("m1"),
                key: s("k1"),
                stamp: 4,
            },
            StoreOp::Records {
                changes: vec![
                    rec_put("m1", false, "k2", 5),
                    rec_put("m1", false, "k3", 6),
                    rec_put("m4", false, "k1", 7),
                ],
                mark: MarkChange::Untouched,
                sync: false,
            },
            StoreOp::Remove {
                map: s("m-2"),
                key: s("k1"),
            },
            StoreOp::RemoveAll {
                map: s("m/3"),
                keys: vec![s("k1"), s("k2")],
            },
            add("m4", "k2", 8),
        ];
        Self {
            initial_mark: None,
            ops,
        }
    }

    /// Second fixed workload: `open_for_migration(m0)`, then 8
    /// `write_records` batches over `m1`…`m4`; batches 1, 2, 4, 6, 7 set a
    /// distinct mark, batch 8 clears it.
    pub(crate) fn fixed_migration() -> Self {
        let maps = ["m1", "m2", "m3", "m4"];
        let marked = [1, 2, 4, 6, 7];
        let synced = [2, 5, 8];
        let ops = (1u8..=8)
            .map(|n| {
                let map = maps[usize::from(n - 1) % maps.len()];
                let other = maps[usize::from(n) % maps.len()];
                let mut changes = vec![
                    rec_put(map, false, &format!("k{n}"), n),
                    rec_put(other, n % 3 == 0, "k0", n.wrapping_add(100)),
                ];
                if n > 4 {
                    changes.push(RecordChange::Delete {
                        map: s(maps[usize::from(n) % maps.len()]),
                        backup: false,
                        key: format!("k{}", n - 3),
                    });
                }
                let mark = if n == 8 {
                    MarkChange::Clear
                } else if marked.contains(&n) {
                    MarkChange::Set(format!("mark-{n}").into_bytes())
                } else {
                    MarkChange::Untouched
                };
                StoreOp::Records {
                    changes,
                    mark,
                    sync: synced.contains(&n),
                }
            })
            .collect();
        Self {
            initial_mark: Some(b"mark-0".to_vec()),
            ops,
        }
    }

    /// A seeded workload over 20 map names (one with `/`, one with `-`) of
    /// `add`, `add_backup`, `remove`, `remove_all` and multi-op
    /// `write_records`. With `migration`, the open is
    /// `open_for_migration(m0)`, 60 % of the `write_records` calls set a
    /// distinct mark and the last operation is a `write_records` that clears
    /// it.
    pub(crate) fn random(seed: u64, migration: bool) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let names: Vec<String> = (0..20)
            .map(|i| match i {
                7 => s("map/7"),
                13 => s("map-13"),
                _ => format!("map{i}"),
            })
            .collect();
        let pick_map = |rng: &mut StdRng| names[rng.random_range(0..names.len())].clone();
        let pick_key = |rng: &mut StdRng| format!("k{}", rng.random_range(0..8));
        let mut marks = 0u32;
        let mut ops: Vec<StoreOp> = Vec::new();
        let count: u64 = rng.random_range(10..=40);
        for index in 0..count {
            let stamp = index + 1;
            let last = index + 1 == count;
            let kind = if migration && last {
                4
            } else {
                rng.random_range(0..20)
            };
            let op = match kind {
                0..=5 => StoreOp::Add {
                    map: pick_map(&mut rng),
                    key: pick_key(&mut rng),
                    stamp,
                },
                6..=8 => StoreOp::AddBackup {
                    map: pick_map(&mut rng),
                    key: pick_key(&mut rng),
                    stamp,
                },
                9..=11 => StoreOp::Remove {
                    map: pick_map(&mut rng),
                    key: pick_key(&mut rng),
                },
                12..=13 => StoreOp::RemoveAll {
                    map: pick_map(&mut rng),
                    keys: (0..rng.random_range(1..=3))
                        .map(|_| pick_key(&mut rng))
                        .collect(),
                },
                _ => {
                    let changes = (0..rng.random_range(2..=5))
                        .map(|_| {
                            let map = pick_map(&mut rng);
                            let backup = rng.random_bool(0.3);
                            let key = pick_key(&mut rng);
                            if rng.random_bool(0.75) {
                                let mut bytes = vec![0u8; rng.random_range(1..=64)];
                                rng.fill(bytes.as_mut_slice());
                                RecordChange::Put {
                                    map,
                                    backup,
                                    key,
                                    bytes,
                                }
                            } else {
                                RecordChange::Delete { map, backup, key }
                            }
                        })
                        .collect();
                    let mark = if !migration {
                        MarkChange::Untouched
                    } else if last {
                        MarkChange::Clear
                    } else if rng.random_bool(0.6) {
                        marks += 1;
                        MarkChange::Set(format!("mark-{seed}-{marks}").into_bytes())
                    } else {
                        MarkChange::Untouched
                    };
                    StoreOp::Records {
                        changes,
                        mark,
                        sync: rng.random_bool(0.5),
                    }
                }
            };
            ops.push(op);
        }
        Self {
            initial_mark: migration.then(|| b"mark-0".to_vec()),
            ops,
        }
    }

    /// The model after each prefix of operations; entry 0 is the empty engine.
    fn models(&self) -> Vec<RegistryModel> {
        let mut model = RegistryModel::default();
        let mut models = vec![model.clone()];
        model.formatted = true;
        model.mark.clone_from(&self.initial_mark);
        models.push(model.clone());
        for op in &self.ops {
            model.apply(op);
            models.push(model.clone());
        }
        models
    }
}

fn call_store<E: KvEngine>(store: &EngineMapDataStore<E>, op: &StoreOp) -> bool {
    match op {
        StoreOp::Add { map, key, stamp } => {
            ready(store.add(map, key, &lww_value(*stamp), 0, 0)).is_ok()
        }
        StoreOp::AddBackup { map, key, stamp } => {
            ready(store.add_backup(map, key, &lww_value(*stamp), 0, 0)).is_ok()
        }
        StoreOp::Remove { map, key } => ready(store.remove(map, key, 0)).is_ok(),
        StoreOp::RemoveAll { map, keys } => ready(store.remove_all(map, keys)).is_ok(),
        StoreOp::Records {
            changes,
            mark,
            sync,
        } => {
            let mut ops: Vec<RecordOp<'_>> = changes
                .iter()
                .map(|change| match change {
                    RecordChange::Put {
                        map,
                        backup,
                        key,
                        bytes,
                    } => RecordOp::Put {
                        map,
                        backup: *backup,
                        key,
                        bytes,
                    },
                    RecordChange::Delete { map, backup, key } => RecordOp::Delete {
                        map,
                        backup: *backup,
                        key,
                    },
                })
                .collect();
            match mark {
                MarkChange::Untouched => {}
                MarkChange::Set(bytes) => {
                    ops.push(RecordOp::SetMigrationMark(Some(bytes.as_slice())));
                }
                MarkChange::Clear => ops.push(RecordOp::SetMigrationMark(None)),
            }
            store
                .write_records(&ops, WriteOptions { sync: *sync })
                .is_ok()
        }
    }
}

/// Whether an `Ok` of `op` makes every earlier operation durable: it must
/// write a batch, and with `sync = true` (every `MapDataStore` method does).
/// A call whose every change is a delete on a map with no registry row and
/// that sets no mark writes no batch, so it makes nothing durable.
fn op_is_durable(op: &StoreOp, registered: &BTreeSet<String>) -> bool {
    match op {
        StoreOp::Add { .. } | StoreOp::AddBackup { .. } => true,
        StoreOp::Remove { map, .. } | StoreOp::RemoveAll { map, .. } => registered.contains(map),
        StoreOp::Records {
            changes,
            mark,
            sync,
        } => {
            *sync
                && (*mark != MarkChange::Untouched
                    || changes.iter().any(|change| match change {
                        RecordChange::Put { .. } => true,
                        RecordChange::Delete { map, .. } => registered.contains(map),
                    }))
        }
    }
}

/// Adds the maps `op` registers (any put, primary or backup).
fn register(op: &StoreOp, registered: &mut BTreeSet<String>) {
    match op {
        StoreOp::Add { map, .. } | StoreOp::AddBackup { map, .. } => {
            registered.insert(map.clone());
        }
        StoreOp::Records { changes, .. } => {
            for change in changes {
                if let RecordChange::Put { map, .. } = change {
                    registered.insert(map.clone());
                }
            }
        }
        StoreOp::Remove { .. } | StoreOp::RemoveAll { .. } => {}
    }
}

impl Scenario for RegistryWorkload {
    fn run<E: KvEngine>(&self, engine: &Arc<E>) -> RunRecord {
        let mut run = RunRecord::default();
        let opened = match &self.initial_mark {
            Some(mark) => {
                EngineMapDataStore::open_for_migration(Arc::clone(engine), mark).map(|(s, _)| s)
            }
            None => EngineMapDataStore::open(Arc::clone(engine)),
        };
        let Ok(store) = opened else {
            // A death in the format batch ends the run.
            run.push(false, true, true);
            return run;
        };
        run.push(true, true, true);
        let mut registered = BTreeSet::new();
        for op in &self.ops {
            let durable = op_is_durable(op, &registered);
            let ok = call_store(&store, op);
            if ok {
                register(op, &mut registered);
            }
            run.push(ok, true, durable);
        }
        run
    }

    fn check<E: KvEngine>(&self, engine: &Arc<E>, run: &RunRecord) -> Result<Vec<usize>, String> {
        let raw = read_all(&**engine)?;
        let recovered = decode_registry(&raw)?;
        // Assertion 4 and the refusal half of assertion 5: `open` refuses
        // exactly when a mark is present, and the matching open succeeds.
        let plain = EngineMapDataStore::open(Arc::clone(engine));
        let store = match (&recovered.mark, plain) {
            (Some(_), Ok(_)) => return Err("open admitted a store with a migration mark".into()),
            (Some(mark), Err(_)) => {
                let (store, read) =
                    EngineMapDataStore::open_for_migration(Arc::clone(engine), b"unused")
                        .map_err(|e| format!("open_for_migration after the crash: {e}"))?;
                if read != *mark {
                    return Err("open_for_migration returned another mark".into());
                }
                store
            }
            (None, Ok(store)) => store,
            (None, Err(e)) => return Err(format!("open after the crash: {e}")),
        };
        let listed: BTreeSet<String> = ready(store.list_maps())
            .map_err(|e| format!("list_maps after the crash: {e}"))?
            .into_iter()
            .collect();
        let models = self.models();
        let durable = run.durable_units();
        let upper = run.upper_units();
        let mut why = Vec::new();
        let mut matches = Vec::new();
        for (i, model) in models.iter().enumerate().take(upper + 1).skip(durable) {
            let mismatch = if model.formatted == recovered.empty {
                Some("format rows")
            } else if model.records != recovered.records {
                Some("records")
            } else if model.primary_names != listed {
                Some("list_maps")
            } else if model.put_names != recovered.names {
                Some("registry names")
            } else if model.mark != recovered.mark {
                Some("migration mark")
            } else {
                None
            };
            match mismatch {
                None => matches.push(i),
                Some(what) => why.push(format!("prefix {i}: {what}")),
            }
        }
        if matches.is_empty() {
            return Err(format!(
                "the recovered registry is no prefix in {durable}..={upper} of {} operations \
                 (records {}, listed {listed:?}, names {:?}, mark {:?}): {}",
                self.units(),
                recovered.records.len(),
                recovered.names,
                recovered.mark.as_deref().map(String::from_utf8_lossy),
                why.join("; ")
            ));
        }
        Ok(matches)
    }

    fn units(&self) -> usize {
        1 + self.ops.len()
    }
}
