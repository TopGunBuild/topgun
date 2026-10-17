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
        unimplemented!()
    }

    fn enter(&self) -> io::Result<()> {
        unimplemented!()
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
        unimplemented!()
    }
}

fn offset_to_index(offset: u64) -> usize {
    unimplemented!()
}

fn apply_write(image: &mut Vec<u8>, offset: u64, bytes: &[u8]) {
    unimplemented!()
}

impl CrashableBackend {
    /// An empty, live backend that is never killed until `set_die_at`.
    pub(crate) fn new() -> Self {
        unimplemented!()
    }

    fn with_image(image: Vec<u8>) -> Self {
        unimplemented!()
    }

    /// Kills the counted op with index `at` (or the next one, if the counter
    /// is already past it) and every later call; `None` disarms.
    pub(crate) fn set_die_at(&self, at: Option<u64>) {
        unimplemented!()
    }

    /// Counted ops (`write`, `set_len`, `sync_data`) entered so far.
    pub(crate) fn ops_so_far(&self) -> u64 {
        unimplemented!()
    }

    /// `sync_data(true)` calls seen so far.
    pub(crate) fn eventual_syncs(&self) -> u64 {
        unimplemented!()
    }

    /// The backend a crash leaves, then releases this handle's state.
    ///
    /// The image is `durable` plus, by `mode`, a seeded part of the pending
    /// ops. This handle's state is then emptied and dead, so every clone
    /// answers EIO and no file image outlives the crash, whatever the dead
    /// engine still holds.
    pub(crate) fn crash_image(&self, mode: CrashMode, seed: u64) -> CrashableBackend {
        unimplemented!()
    }

    /// Bytes this handle's state keeps alive: both images and every pending write.
    pub(crate) fn retained_bytes(&self) -> usize {
        unimplemented!()
    }
}

impl redb::StorageBackend for CrashableBackend {
    fn len(&self) -> io::Result<u64> {
        unimplemented!()
    }

    fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        unimplemented!()
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        unimplemented!()
    }

    fn sync_data(&self, eventual: bool) -> io::Result<()> {
        unimplemented!()
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        unimplemented!()
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
        unimplemented!()
    }

    fn engine(&self) -> Arc<RedbKvEngine> {
        unimplemented!()
    }

    fn arm_death(&self, at_op: u64) {
        unimplemented!()
    }

    fn ops_so_far(&self) -> u64 {
        unimplemented!()
    }

    fn crash(self, mode: CrashMode, seed: u64) -> Self {
        unimplemented!()
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
        unimplemented!()
    }

    fn engine(&self) -> Arc<MemKvEngine> {
        unimplemented!()
    }

    fn arm_death(&self, at_op: u64) {
        unimplemented!()
    }

    fn ops_so_far(&self) -> u64 {
        unimplemented!()
    }

    fn crash(self, _mode: CrashMode, seed: u64) -> Self {
        unimplemented!()
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
        unimplemented!()
    }

    /// `d`: the units called up to the last durable call that returned
    /// before the first failure.
    pub(crate) fn durable_units(&self) -> usize {
        unimplemented!()
    }

    /// The longest prefix a crash may leave: every unit before the first
    /// failed call, plus that call when it is a unit (it was in flight).
    pub(crate) fn upper_units(&self) -> usize {
        unimplemented!()
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
        unimplemented!()
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    unimplemented!()
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
    unimplemented!()
}

/// `T`: the counted ops after a full run of `scenario` on a fresh target.
pub(crate) fn dry_run_ops<T: CrashTarget, S: Scenario>(scenario: &S) -> u64 {
    unimplemented!()
}

/// The crash variants of one point: every mode (a byte-tearing target) or
/// every number of unsynced batches to keep in `0..=B` (a whole-batch one).
fn variants<T: CrashTarget>(units: usize, point: u64) -> Vec<(CrashMode, u64)> {
    unimplemented!()
}

/// Every crash point `n ∈ 0..=T` of `scenario`, each in every variant.
pub(crate) fn every_crash_point<T: CrashTarget, S: Scenario>(scenario: &S) -> Report {
    unimplemented!()
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
    unimplemented!()
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
    unimplemented!()
}

/// An engine workload: batches and syncs against the bare engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KvWorkload {
    /// The steps, in call order.
    pub(crate) steps: Vec<Step>,
}

fn value_of(len: usize, tag: u8) -> Vec<u8> {
    unimplemented!()
}

fn put(key: &str, len: usize, tag: u8) -> BatchOp {
    unimplemented!()
}

fn delete(key: &str) -> BatchOp {
    unimplemented!()
}

fn delete_range(start: &str, end: &str) -> BatchOp {
    unimplemented!()
}

fn batch(ops: Vec<BatchOp>, sync: bool) -> Step {
    unimplemented!()
}

impl KvWorkload {
    /// The fixed 12-batch workload over keys `a`…`h`: multi-op batches, a
    /// `sync()` after unsynced batches followed by more unsynced ones before
    /// the next durable write, and a trailing unsynced batch (`d = 11`).
    pub(crate) fn fixed() -> Self {
        unimplemented!()
    }

    /// A seeded workload: 10–60 steps; a step is `sync()` (10 %) or a batch
    /// of 1–8 puts, deletes and range deletes over 64 keys of 1–6 bytes with
    /// values of 0–2 KiB, written with `sync = true` (30 %) or `false`.
    pub(crate) fn random(seed: u64) -> Self {
        unimplemented!()
    }

    /// The model after each prefix of batches: entry `i` is the state after
    /// the first `i`.
    pub(crate) fn models(&self) -> Vec<BTreeMap<Vec<u8>, Vec<u8>>> {
        unimplemented!()
    }
}

/// Every key of `engine`, read from one snapshot.
pub(crate) fn read_all<E: KvEngine>(engine: &E) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
    unimplemented!()
}

fn describe_diff(state: &Model, model: &Model) -> String {
    unimplemented!()
}

impl Scenario for KvWorkload {
    fn run<E: KvEngine>(&self, engine: &Arc<E>) -> RunRecord {
        unimplemented!()
    }

    fn check<E: KvEngine>(&self, engine: &Arc<E>, run: &RunRecord) -> Result<Vec<usize>, String> {
        unimplemented!()
    }

    fn units(&self) -> usize {
        unimplemented!()
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
    unimplemented!()
}

/// The bytes `add` stores for `stamp`.
fn lww_bytes(stamp: u64) -> Vec<u8> {
    unimplemented!()
}

/// Drives a store future that never suspends: every store call is
/// synchronous underneath.
fn ready<F: Future>(future: F) -> F::Output {
    unimplemented!()
}

fn s(text: &str) -> String {
    unimplemented!()
}

fn rec_put(map: &str, backup: bool, key: &str, tag: u8) -> RecordChange {
    unimplemented!()
}

impl RegistryModel {
    fn put(&mut self, map: &str, backup: bool, key: &str, bytes: Vec<u8>) {
        unimplemented!()
    }

    fn remove(&mut self, map: &str, backup: bool, key: &str) {
        unimplemented!()
    }

    fn apply(&mut self, op: &StoreOp) {
        unimplemented!()
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
    unimplemented!()
}

impl RegistryWorkload {
    /// First fixed workload: `add` to new maps `m1`, `m-2`, `m/3`, then
    /// `add_backup` to `m1`, a 3-op `write_records` (two puts to `m1`, one to
    /// new `m4`), `remove` on `m-2`, `remove_all` on `m/3`, `add` to `m4`.
    pub(crate) fn fixed() -> Self {
        unimplemented!()
    }

    /// Second fixed workload: `open_for_migration(m0)`, then 8
    /// `write_records` batches over `m1`…`m4`; batches 1, 2, 4, 6, 7 set a
    /// distinct mark, batch 8 clears it.
    pub(crate) fn fixed_migration() -> Self {
        unimplemented!()
    }

    /// A seeded workload over 20 map names (one with `/`, one with `-`) of
    /// `add`, `add_backup`, `remove`, `remove_all` and multi-op
    /// `write_records`. With `migration`, the open is
    /// `open_for_migration(m0)`, 60 % of the `write_records` calls set a
    /// distinct mark and the last operation is a `write_records` that clears
    /// it.
    pub(crate) fn random(seed: u64, migration: bool) -> Self {
        unimplemented!()
    }

    /// The model after each prefix of operations; entry 0 is the empty engine.
    fn models(&self) -> Vec<RegistryModel> {
        unimplemented!()
    }
}

fn call_store<E: KvEngine>(store: &EngineMapDataStore<E>, op: &StoreOp) -> bool {
    unimplemented!()
}

/// Whether an `Ok` of `op` makes every earlier operation durable: it must
/// write a batch, and with `sync = true` (every `MapDataStore` method does).
/// A call whose every change is a delete on a map with no registry row and
/// that sets no mark writes no batch, so it makes nothing durable.
fn op_is_durable(op: &StoreOp, registered: &BTreeSet<String>) -> bool {
    unimplemented!()
}

/// Adds the maps `op` registers (any put, primary or backup).
fn register(op: &StoreOp, registered: &mut BTreeSet<String>) {
    unimplemented!()
}

impl Scenario for RegistryWorkload {
    fn run<E: KvEngine>(&self, engine: &Arc<E>) -> RunRecord {
        unimplemented!()
    }

    fn check<E: KvEngine>(&self, engine: &Arc<E>, run: &RunRecord) -> Result<Vec<usize>, String> {
        unimplemented!()
    }

    fn units(&self) -> usize {
        unimplemented!()
    }
}
