//! Removal of expired records through the ordinary operation pipeline
//! (TG-TTL-001), and the bounded background sweep that finds them.
//!
//! An expired record is removed by an [`Operation::ExpireRecord`], which the
//! CRDT service re-checks under the key's writer and applies as an ordinary
//! REMOVE or `OR_REMOVE`. This module lives under `service/` rather than beside
//! the expiry predicate in `storage/` because it names `service::operation`
//! types, and `storage/` does not depend on `service/`.
//!
//! # The sweep
//!
//! [`spawn_expiry_sweep`] runs [`sweep_tick`] every [`SWEEP_TICK`]. A *cycle*
//! walks every map — the maps the data store lists and the maps resident in
//! the factory, recomputed when the cycle starts — to its end; a cycle starts
//! no earlier than the configured interval after the previous one started, and
//! the first one interval after the sweep was created.
//!
//! On a durable store a map is read with a resumable cursor in batches of at
//! most [`SWEEP_BATCH_COST`] bytes. A tick examines at most
//! `ceil(rows_per_sec / 4)` rows; the rows of a batch it does not reach stay in
//! a remainder buffer that the next tick examines before it fetches again, so
//! no row is read twice or passed over. Residents are not visited there: every
//! resident write reaches the store within the flush interval.
//!
//! On a null store (`STORAGE_BACKEND=null`, simulation, load harness) there is
//! nothing to scan, so a map's resident stores are visited once per cycle, which
//! costs O(resident rows of the map). That visit queues at most
//! [`SWEEP_DISPATCHES_PER_TICK`] candidates and never more than the queue has
//! room for, and has no point to resume from, so a map with more expired
//! records than that needs one cycle per [`SWEEP_DISPATCHES_PER_TICK`] of them.
//!
//! A row is a candidate when it shows expiry as stored: an LWW value past its
//! TTL, or an OR slot with an expired entry. The candidate is only a hint. The
//! handler reads the key again under its writer, where a resident (newer)
//! value comes first: an LWW record rewritten since is skipped as `changed`,
//! and of an OR slot only the entries expired at the handler's own `now` are
//! removed, so a later write is never deleted by a stale candidate.
//!
//! Candidates wait in a queue of at most [`SWEEP_QUEUE_CAPACITY`]; while it is
//! full the scan does not move. A tick dispatches at most
//! [`SWEEP_DISPATCHES_PER_TICK`] of them, one at a time; `Overloaded` keeps the
//! candidate at the head, any other error drops it (the next cycle finds it
//! again), and either stops dispatch for a back-off of 250 ms doubling to 5 s,
//! reset by the next successful dispatch.
//!
//! A map whose last complete pass found no row with a TTL, and that has had
//! no write with a TTL since that pass began, is passed over. Every
//! [`SWEEP_SKIP_BACKSTOP_CYCLES`]th cycle passes over nothing, against an
//! ingest path that stores a TTL without noting it. This state is in memory
//! only: after a restart every map gets one full pass.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use metrics::{Counter, Gauge};
use tokio::sync::watch;
use tokio::time::Instant;
use topgun_core::{hash_to_partition, Timestamp};
use tower::Service;

use crate::service::config::ServerConfig;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::domain::crdt::CrdtService;
use crate::service::operation::{
    service_names, CallerOrigin, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::storage::expiry::{entry_is_live, lww_is_live, ExpiryExpectation};
use crate::storage::factory::RecordStoreFactory;
use crate::storage::map_data_store::{MapDataStore, ScanCursor};
use crate::storage::record::RecordValue;

/// Expiry removals applied, by `kind`: `lww_record` per LWW key, `or_tag` per
/// OR entry.
pub const METRIC_EXPIRY_REMOVED_TOTAL: &str = "topgun_expiry_removed_total";

/// Expiry ops that removed nothing, by `reason`: `absent`, `changed` or
/// `not_expired`.
pub const METRIC_EXPIRY_SKIPPED_TOTAL: &str = "topgun_expiry_skipped_total";

/// Rows examined: stored rows on a durable store, resident rows on a null store.
pub const METRIC_SWEEP_ROWS_SCANNED_TOTAL: &str = "topgun_expiry_sweep_rows_scanned_total";
/// Candidates queued, by `kind`: `lww` or `or`.
pub const METRIC_SWEEP_CANDIDATES_TOTAL: &str = "topgun_expiry_sweep_candidates_total";
/// Dispatches refused as `Overloaded`; the candidate was kept.
pub const METRIC_SWEEP_DISPATCH_OVERLOADED_TOTAL: &str = "topgun_expiry_dispatch_overloaded_total";
/// Dispatches that failed otherwise; the candidate was dropped.
pub const METRIC_SWEEP_DISPATCH_FAILED_TOTAL: &str = "topgun_expiry_dispatch_failed_total";
/// Cycles completed.
pub const METRIC_SWEEP_CYCLES_TOTAL: &str = "topgun_expiry_sweep_cycles_total";
/// How long the last completed cycle took, from its start to its end.
pub const METRIC_SWEEP_LAST_CYCLE_SECONDS: &str = "topgun_expiry_sweep_last_cycle_seconds";
/// 1 while the sweep task runs, 0 once it has ended by any route.
pub const METRIC_SWEEP_TASK_ALIVE: &str = "topgun_expiry_sweep_task_alive";
/// Ticks that panicked; the task went on.
pub const METRIC_SWEEP_PASS_PANICS_TOTAL: &str = "topgun_expiry_sweep_pass_panics_total";
/// Maps passed over by a cycle because they hold no row with a TTL.
pub const METRIC_SWEEP_MAPS_SKIPPED_TOTAL: &str = "topgun_expiry_sweep_maps_skipped_total";

/// How often the sweep task runs a tick.
pub const SWEEP_TICK: Duration = Duration::from_millis(250);
/// The most bytes one scan of a map fetches, beside one oversized row.
pub const SWEEP_BATCH_COST: u64 = 256 * 1024;
/// The most expire operations one tick dispatches.
pub const SWEEP_DISPATCHES_PER_TICK: usize = 256;
/// The most candidates waiting for dispatch.
pub const SWEEP_QUEUE_CAPACITY: usize = 1024;
/// Every this many cycles, one passes over no map.
pub const SWEEP_SKIP_BACKSTOP_CYCLES: u64 = 360;
/// Default of `TOPGUN_EXPIRY_SWEEP_INTERVAL_MS`.
pub const DEFAULT_SWEEP_INTERVAL_MS: u64 = 10_000;
/// The smallest `TOPGUN_EXPIRY_SWEEP_INTERVAL_MS` a node starts with.
pub const MIN_SWEEP_INTERVAL_MS: u64 = 1_000;
/// Default of `TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC`.
pub const DEFAULT_SWEEP_ROWS_PER_SEC: u64 = 4_096;

const BACKOFF_BASE: Duration = Duration::from_millis(250);
const BACKOFF_CAP: Duration = Duration::from_secs(5);

/// Where an expiry removal is sent: the partition dispatcher in production, so
/// it is routed, ordered and back-pressured like any other operation.
#[async_trait::async_trait]
pub trait ExpiryDispatch: Send + Sync {
    /// Sends `op` and waits for its outcome.
    async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError>;
}

#[async_trait::async_trait]
impl ExpiryDispatch for PartitionDispatcher {
    async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError> {
        PartitionDispatcher::dispatch(self, op).await
    }
}

/// Sends each removal straight to a CRDT service, past routing and the
/// middleware: for simulation nodes and tests, which have no dispatcher. It
/// never answers `Overloaded`.
pub struct CrdtServiceDispatch(pub Arc<CrdtService>);

#[async_trait::async_trait]
impl ExpiryDispatch for CrdtServiceDispatch {
    async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError> {
        Service::call(&mut Arc::clone(&self.0), op).await
    }
}

/// The removal of `key` of `map_name` that the expiry sweep saw expired as
/// `expected`, with the default operation timeout. Only tests build it this
/// way; the sweep uses [`expire_record_op_with_timeout`] with the server's.
#[must_use]
pub fn expire_record_op(map_name: &str, key: &str, expected: ExpiryExpectation) -> Operation {
    expire_record_op_with_timeout(
        map_name,
        key,
        expected,
        ServerConfig::default().default_operation_timeout_ms,
    )
}

/// The removal of `key` of `map_name` that the expiry sweep saw expired as
/// `expected`, timing out after `timeout_ms` like any operation of the server.
///
/// Routed to the key's partition and the CRDT service, with the `System`
/// origin, which skips authorization: the op is built by the server, never
/// classified from a client message. Its context carries no HLC stamp because
/// the handler reads none; the journal stamp of each removal is minted when it
/// is applied.
#[must_use]
pub fn expire_record_op_with_timeout(
    map_name: &str,
    key: &str,
    expected: ExpiryExpectation,
    timeout_ms: u64,
) -> Operation {
    let mut ctx = OperationContext::new(
        0,
        service_names::CRDT,
        Timestamp {
            millis: 0,
            counter: 0,
            node_id: String::new(),
        },
        timeout_ms,
    );
    ctx.partition_id = Some(hash_to_partition(key));
    ctx.caller_origin = CallerOrigin::System;
    Operation::ExpireRecord {
        ctx,
        map_name: map_name.to_string(),
        key: key.to_string(),
        expected,
    }
}

/// The sweep's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepConfig {
    /// Least time between the starts of two cycles.
    pub interval_ms: u64,
    /// Stored rows examined per second.
    pub rows_per_sec: u64,
    /// Timeout of each expire operation: the server's operation timeout.
    pub operation_timeout_ms: u64,
}

impl SweepConfig {
    /// The settings from the raw values of `TOPGUN_EXPIRY_SWEEP_INTERVAL_MS`
    /// and `TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC` (`None` when unset). A value that
    /// does not parse warns and takes the default.
    ///
    /// # Errors
    ///
    /// An interval below [`MIN_SWEEP_INTERVAL_MS`] or a rate of 0, which the
    /// node refuses to start with.
    pub fn from_env_values(
        interval_ms: Option<&str>,
        rows_per_sec: Option<&str>,
        operation_timeout_ms: u64,
    ) -> Result<Self, String> {
        Ok(Self {
            interval_ms: parse_knob(
                "TOPGUN_EXPIRY_SWEEP_INTERVAL_MS",
                interval_ms,
                DEFAULT_SWEEP_INTERVAL_MS,
                |v| v >= MIN_SWEEP_INTERVAL_MS,
                "it must be at least 1000",
            )?,
            rows_per_sec: parse_knob(
                "TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC",
                rows_per_sec,
                DEFAULT_SWEEP_ROWS_PER_SEC,
                |v| v > 0,
                "it must be above 0",
            )?,
            operation_timeout_ms,
        })
    }

    fn rows_per_tick(&self) -> usize {
        usize::try_from(self.rows_per_sec.div_ceil(4)).unwrap_or(usize::MAX)
    }
}

fn parse_knob(
    name: &str,
    raw: Option<&str>,
    default: u64,
    valid: impl Fn(u64) -> bool,
    rule: &str,
) -> Result<u64, String> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    match raw.trim().parse::<u64>() {
        Ok(value) if valid(value) => Ok(value),
        Ok(value) => Err(format!("expiry sweep: {name}={value} is refused: {rule}")),
        Err(_) => {
            tracing::warn!(
                name,
                value = raw,
                default,
                "expiry sweep: the value does not parse as a whole number; using the default"
            );
            Ok(default)
        }
    }
}

/// What one [`sweep_tick`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepTickReport {
    /// A cycle started in this tick.
    pub cycle_started: bool,
    /// The running cycle completed in this tick.
    pub cycle_completed: bool,
    /// Rows examined.
    pub rows_scanned: u64,
    /// Candidates queued.
    pub candidates: u64,
    /// Maps passed over by the skip.
    pub maps_skipped: u64,
    /// Expire operations sent, whatever their outcome.
    pub dispatched: u64,
    /// Dispatch was held back by a back-off.
    pub backing_off: bool,
}

#[derive(Clone)]
struct SweepMetrics {
    rows_scanned: Counter,
    candidates_lww: Counter,
    candidates_or: Counter,
    overloaded: Counter,
    failed: Counter,
    cycles: Counter,
    last_cycle_seconds: Gauge,
    task_alive: Gauge,
    pass_panics: Counter,
    maps_skipped: Counter,
}

impl SweepMetrics {
    /// Resolves every handle and touches every series of the eleven families
    /// at zero, so a scrape shows them before the first tick.
    fn register() -> Self {
        metrics::describe_counter!(
            METRIC_EXPIRY_REMOVED_TOTAL,
            "Expiry removals applied, by kind"
        );
        metrics::describe_counter!(
            METRIC_EXPIRY_SKIPPED_TOTAL,
            "Expiry operations that removed nothing, by reason"
        );
        for kind in ["lww_record", "or_tag"] {
            metrics::counter!(METRIC_EXPIRY_REMOVED_TOTAL, "kind" => kind).increment(0);
        }
        for reason in ["absent", "changed", "not_expired"] {
            metrics::counter!(METRIC_EXPIRY_SKIPPED_TOTAL, "reason" => reason).increment(0);
        }
        metrics::describe_counter!(
            METRIC_SWEEP_ROWS_SCANNED_TOTAL,
            "Rows the expiry sweep examined: stored rows on a durable store, resident rows on a null store"
        );
        metrics::describe_counter!(
            METRIC_SWEEP_CANDIDATES_TOTAL,
            "Expired records the expiry sweep queued for removal, by kind"
        );
        metrics::describe_counter!(
            METRIC_SWEEP_DISPATCH_OVERLOADED_TOTAL,
            "Expiry removals refused as overloaded and kept for a retry"
        );
        metrics::describe_counter!(
            METRIC_SWEEP_DISPATCH_FAILED_TOTAL,
            "Expiry removals that failed and were left to the next cycle"
        );
        metrics::describe_counter!(METRIC_SWEEP_CYCLES_TOTAL, "Expiry sweep cycles completed");
        metrics::describe_gauge!(
            METRIC_SWEEP_LAST_CYCLE_SECONDS,
            "Duration of the last completed expiry sweep cycle"
        );
        metrics::describe_gauge!(
            METRIC_SWEEP_TASK_ALIVE,
            "1 while the expiry sweep task runs"
        );
        metrics::describe_counter!(
            METRIC_SWEEP_PASS_PANICS_TOTAL,
            "Expiry sweep ticks that panicked"
        );
        metrics::describe_counter!(
            METRIC_SWEEP_MAPS_SKIPPED_TOTAL,
            "Maps an expiry sweep cycle passed over because they hold no record with a TTL"
        );
        let touched = |counter: Counter| {
            counter.increment(0);
            counter
        };
        let metrics = Self {
            rows_scanned: touched(metrics::counter!(METRIC_SWEEP_ROWS_SCANNED_TOTAL)),
            candidates_lww: touched(
                metrics::counter!(METRIC_SWEEP_CANDIDATES_TOTAL, "kind" => "lww"),
            ),
            candidates_or: touched(
                metrics::counter!(METRIC_SWEEP_CANDIDATES_TOTAL, "kind" => "or"),
            ),
            overloaded: touched(metrics::counter!(METRIC_SWEEP_DISPATCH_OVERLOADED_TOTAL)),
            failed: touched(metrics::counter!(METRIC_SWEEP_DISPATCH_FAILED_TOTAL)),
            cycles: touched(metrics::counter!(METRIC_SWEEP_CYCLES_TOTAL)),
            last_cycle_seconds: metrics::gauge!(METRIC_SWEEP_LAST_CYCLE_SECONDS),
            task_alive: metrics::gauge!(METRIC_SWEEP_TASK_ALIVE),
            pass_panics: touched(metrics::counter!(METRIC_SWEEP_PASS_PANICS_TOTAL)),
            maps_skipped: touched(metrics::counter!(METRIC_SWEEP_MAPS_SKIPPED_TOTAL)),
        };
        metrics.last_cycle_seconds.set(0.0);
        metrics.task_alive.set(0.0);
        metrics
    }
}

/// What the last complete pass of a map found, for the skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastPass {
    found_ttl_row: bool,
    generation_at_start: u64,
}

#[derive(Debug)]
struct Candidate {
    map: String,
    key: String,
    expected: ExpiryExpectation,
}

enum Fetch {
    First,
    After(ScanCursor),
    Done,
}

struct MapPass {
    map: String,
    generation_at_start: u64,
    found_ttl_row: bool,
    next: Fetch,
    remainder: VecDeque<(String, RecordValue)>,
}

struct Cycle {
    started: Instant,
    maps: VecDeque<String>,
    ignore_skip: bool,
    pass: Option<MapPass>,
}

/// Everything the sweep carries from one tick to the next.
pub struct SweepState {
    factory: Arc<RecordStoreFactory>,
    dispatch: Arc<dyn ExpiryDispatch>,
    config: SweepConfig,
    metrics: SweepMetrics,
    shutdown: Option<watch::Receiver<bool>>,
    next_cycle_at: Instant,
    cycles_started: u64,
    cycle: Option<Cycle>,
    last_pass: HashMap<String, LastPass>,
    queue: VecDeque<Candidate>,
    backoff: Option<Duration>,
    backoff_until: Option<Instant>,
}

impl SweepState {
    /// A sweep over `factory`'s maps that sends removals to `dispatch`. Its
    /// first cycle starts one interval from now. Registers the metrics.
    #[must_use]
    pub fn new(
        factory: Arc<RecordStoreFactory>,
        dispatch: Arc<dyn ExpiryDispatch>,
        config: SweepConfig,
    ) -> Self {
        Self {
            factory,
            dispatch,
            config,
            metrics: SweepMetrics::register(),
            shutdown: None,
            next_cycle_at: Instant::now() + Duration::from_millis(config.interval_ms),
            cycles_started: 0,
            cycle: None,
            last_pass: HashMap::new(),
            queue: VecDeque::new(),
            backoff: None,
            backoff_until: None,
        }
    }

    fn stopping(&self) -> bool {
        self.shutdown.as_ref().is_some_and(|rx| *rx.borrow())
    }

    async fn begin_cycle(&mut self, now: Instant) {
        let mut maps: BTreeSet<String> = self.factory.map_names().into_iter().collect();
        match self.factory.data_store().list_maps().await {
            Ok(stored) => maps.extend(stored),
            Err(err) => tracing::warn!(
                error = %err,
                "expiry sweep: the stored maps could not be listed; this cycle visits the resident maps only"
            ),
        }
        // A map that has left (dropped, e.g. by a tenant delete) loses its
        // skip entry; its queued candidates are still dispatched and skip as
        // `absent`.
        self.last_pass.retain(|map, _| maps.contains(map));
        self.cycles_started += 1;
        self.next_cycle_at = now + Duration::from_millis(self.config.interval_ms);
        self.cycle = Some(Cycle {
            started: now,
            maps: maps.into_iter().collect(),
            ignore_skip: self
                .cycles_started
                .is_multiple_of(SWEEP_SKIP_BACKSTOP_CYCLES),
            pass: None,
        });
    }

    /// Examines rows of the running cycle and queues the expired ones, within
    /// the row budget and the room in the queue.
    async fn scan(&mut self, report: &mut SweepTickReport) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let now_ms = self.factory.now_ms();
        let data_store = self.factory.data_store();
        let mut budget = self.config.rows_per_tick();
        loop {
            if self.queue.len() >= SWEEP_QUEUE_CAPACITY {
                return;
            }
            let pass = if let Some(pass) = cycle.pass.as_mut() {
                pass
            } else {
                let Some(map) = cycle.maps.pop_front() else {
                    self.metrics.cycles.increment(1);
                    self.metrics
                        .last_cycle_seconds
                        .set(cycle.started.elapsed().as_secs_f64());
                    report.cycle_completed = true;
                    self.cycle = None;
                    return;
                };
                let generation_at_start = self.factory.ttl_write_generation(&map);
                let unchanged_without_ttl = LastPass {
                    found_ttl_row: false,
                    generation_at_start,
                };
                if !cycle.ignore_skip && self.last_pass.get(&map) == Some(&unchanged_without_ttl) {
                    self.metrics.maps_skipped.increment(1);
                    report.maps_skipped += 1;
                    continue;
                }
                cycle.pass.insert(MapPass {
                    map,
                    generation_at_start,
                    found_ttl_row: false,
                    next: Fetch::First,
                    remainder: VecDeque::new(),
                })
            };

            if data_store.is_null() {
                let room = (SWEEP_QUEUE_CAPACITY - self.queue.len()).min(SWEEP_DISPATCHES_PER_TICK);
                let (rows, found) = visit_residents(&self.factory, pass, room, now_ms);
                self.metrics.rows_scanned.increment(rows);
                report.rows_scanned += rows;
                for (key, expected) in found {
                    queue_candidate(
                        &mut self.queue,
                        &self.metrics,
                        report,
                        &pass.map,
                        key,
                        expected,
                    );
                }
                pass.next = Fetch::Done;
            }

            if pass.remainder.is_empty() {
                if budget == 0 && !matches!(pass.next, Fetch::Done) {
                    return;
                }
                if !refill(pass, &*data_store, &mut self.last_pass).await {
                    cycle.pass = None;
                }
                continue;
            }
            if budget == 0 {
                return;
            }
            let Some((key, value)) = pass.remainder.pop_front() else {
                continue;
            };
            budget -= 1;
            self.metrics.rows_scanned.increment(1);
            report.rows_scanned += 1;
            if let Some(expected) = candidate(&value, now_ms, &mut pass.found_ttl_row) {
                queue_candidate(
                    &mut self.queue,
                    &self.metrics,
                    report,
                    &pass.map,
                    key,
                    expected,
                );
            }
        }
    }

    /// Sends queued candidates, at most [`SWEEP_DISPATCHES_PER_TICK`], one at
    /// a time, unless a back-off holds.
    async fn dispatch_queue(&mut self, report: &mut SweepTickReport) {
        if self
            .backoff_until
            .is_some_and(|until| Instant::now() < until)
        {
            report.backing_off = true;
            return;
        }
        for _ in 0..SWEEP_DISPATCHES_PER_TICK {
            if self.stopping() {
                return;
            }
            let Some(next) = self.queue.front() else {
                return;
            };
            let op = expire_record_op_with_timeout(
                &next.map,
                &next.key,
                next.expected.clone(),
                self.config.operation_timeout_ms,
            );
            report.dispatched += 1;
            match self.dispatch.dispatch(op).await {
                Ok(_) => {
                    self.queue.pop_front();
                    self.backoff = None;
                    self.backoff_until = None;
                }
                Err(OperationError::Overloaded) => {
                    self.metrics.overloaded.increment(1);
                    self.back_off();
                    return;
                }
                Err(err) => {
                    self.metrics.failed.increment(1);
                    if let Some(dropped) = self.queue.pop_front() {
                        tracing::debug!(
                            map = %dropped.map,
                            key = %dropped.key,
                            error = %err,
                            "expiry sweep: a removal failed; the next cycle finds the record again"
                        );
                    }
                    self.back_off();
                    return;
                }
            }
        }
    }

    fn back_off(&mut self) {
        let delay = self
            .backoff
            .map_or(BACKOFF_BASE, |d| (d * 2).min(BACKOFF_CAP));
        self.backoff = Some(delay);
        self.backoff_until = Some(Instant::now() + delay);
    }
}

/// Fetches the next batch of `pass` into its remainder, and returns `true`;
/// or ends the pass, recording it for the skip when it read the map to its
/// end, and returns `false`.
async fn refill(
    pass: &mut MapPass,
    data_store: &dyn MapDataStore,
    last_pass: &mut HashMap<String, LastPass>,
) -> bool {
    let batch = match std::mem::replace(&mut pass.next, Fetch::Done) {
        Fetch::Done => {
            last_pass.insert(
                pass.map.clone(),
                LastPass {
                    found_ttl_row: pass.found_ttl_row,
                    generation_at_start: pass.generation_at_start,
                },
            );
            return false;
        }
        Fetch::First => data_store.scan_values(&pass.map, false, SWEEP_BATCH_COST),
        Fetch::After(cursor) => {
            data_store.scan_values_batched(&pass.map, false, cursor, SWEEP_BATCH_COST)
        }
    }
    .await;
    match batch {
        Ok(batch) => {
            pass.remainder = batch.records.into();
            pass.next = batch.next_cursor.map_or(Fetch::Done, Fetch::After);
            true
        }
        Err(err) => {
            // The pass is left unfinished, so no skip is decided from it; the
            // next cycle reads the map again.
            tracing::warn!(
                map = %pass.map,
                error = %err,
                "expiry sweep: a scan failed; the map is read again next cycle"
            );
            false
        }
    }
}

/// Visits every resident record of the map of `pass` once, for a null store,
/// which has nothing to scan. Returns the rows visited and at most `room`
/// candidates; the others are found by a later cycle.
fn visit_residents(
    factory: &RecordStoreFactory,
    pass: &mut MapPass,
    room: usize,
    now_ms: u64,
) -> (u64, Vec<(String, ExpiryExpectation)>) {
    let mut found = Vec::new();
    let mut rows = 0_u64;
    for store in factory.get_all_for_map(&pass.map) {
        store.for_each_boxed(
            &mut |key, record| {
                rows += 1;
                if let Some(expected) = candidate(&record.value, now_ms, &mut pass.found_ttl_row) {
                    if found.len() < room {
                        found.push((key.to_string(), expected));
                    }
                }
            },
            false,
        );
    }
    (rows, found)
}

fn queue_candidate(
    queue: &mut VecDeque<Candidate>,
    metrics: &SweepMetrics,
    report: &mut SweepTickReport,
    map: &str,
    key: String,
    expected: ExpiryExpectation,
) {
    match expected {
        ExpiryExpectation::Lww { .. } => metrics.candidates_lww.increment(1),
        ExpiryExpectation::Or => metrics.candidates_or.increment(1),
    }
    report.candidates += 1;
    queue.push_back(Candidate {
        map: map.to_string(),
        key,
        expected,
    });
}

/// What the sweep expects of a stored value that shows expiry at `now`, or
/// `None` when it shows none; notes in `found_ttl_row` whether the value
/// carries a TTL at all, expired or not.
fn candidate(value: &RecordValue, now: u64, found_ttl_row: &mut bool) -> Option<ExpiryExpectation> {
    match value {
        RecordValue::Lww {
            timestamp, ttl_ms, ..
        } => {
            *found_ttl_row |= ttl_ms.is_some();
            (!lww_is_live(timestamp, *ttl_ms, now)).then(|| ExpiryExpectation::Lww {
                timestamp: timestamp.clone(),
            })
        }
        RecordValue::OrMap { records, .. } => {
            *found_ttl_row |= records.iter().any(|e| e.ttl_ms.is_some());
            records
                .iter()
                .any(|e| !entry_is_live(e, now))
                .then_some(ExpiryExpectation::Or)
        }
        RecordValue::OrTombstones { .. } => None,
    }
}

/// One step of the sweep: starts a cycle when one is due, examines rows of the
/// running cycle within the tick's budget, and dispatches queued candidates.
/// Public so tests drive it without the task or real time.
pub async fn sweep_tick(state: &mut SweepState) -> SweepTickReport {
    let mut report = SweepTickReport::default();
    if state.stopping() {
        return report;
    }
    let now = Instant::now();
    if state.cycle.is_none() && now >= state.next_cycle_at {
        state.begin_cycle(now).await;
        report.cycle_started = true;
    }
    state.scan(&mut report).await;
    state.dispatch_queue(&mut report).await;
    report
}

/// Sets the liveness gauge to 0 when the task's future is dropped, on every
/// route out: shutdown, abort, runtime teardown or an escaped panic.
struct AliveLease(Gauge);

impl Drop for AliveLease {
    fn drop(&mut self) {
        self.0.set(0.0);
    }
}

/// Spawns the expiry sweep (TG-TTL-001): a tick every [`SWEEP_TICK`] until
/// `shutdown` turns `true`.
///
/// The metrics are registered, and the liveness gauge set, before this
/// returns. A tick that panics is counted and logged, its cycle is dropped and
/// the next one starts after the interval, and the task goes on. Shutdown is
/// observed at every tick and between two dispatches: a dispatch already sent
/// is awaited, the queued candidates are dropped (the next start finds them
/// again), and the task ends.
///
/// The catch is inert under `panic = "abort"`, which no profile of this
/// workspace sets.
#[must_use]
pub fn spawn_expiry_sweep(
    factory: Arc<RecordStoreFactory>,
    dispatch: Arc<dyn ExpiryDispatch>,
    config: SweepConfig,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let mut state = SweepState::new(factory, dispatch, config);
    state.shutdown = Some(shutdown.clone());
    state.metrics.task_alive.set(1.0);
    let lease = AliveLease(state.metrics.task_alive.clone());
    tokio::spawn(async move {
        // Named, so the lease lives as long as this future.
        let _lease = lease;
        let mut ticker = tokio::time::interval(SWEEP_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = shutdown_requested(&mut shutdown) => return,
                _ = ticker.tick() => {}
            }
            let outcome = {
                // Polled through a catch, as the prune task does, rather than
                // spawned: the state stays owned by this loop.
                let mut tick = std::pin::pin!(sweep_tick(&mut state));
                std::future::poll_fn(|cx| {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        std::future::Future::poll(tick.as_mut(), cx)
                    })) {
                        Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
                        Ok(std::task::Poll::Ready(_)) => std::task::Poll::Ready(Ok(())),
                        Err(payload) => std::task::Poll::Ready(Err(payload)),
                    }
                })
                .await
            };
            if let Err(payload) = outcome {
                state.metrics.pass_panics.increment(1);
                state.cycle = None;
                let message = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                tracing::warn!(
                    panic_message = %message,
                    "expiry sweep tick panicked; its cycle is dropped and the sweep goes on"
                );
            }
        }
    })
}

/// Resolves once `shutdown` reads `true`; never, when its sender is gone.
async fn shutdown_requested(shutdown: &mut watch::Receiver<bool>) {
    if shutdown.wait_for(|stop| *stop).await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use topgun_core::messages::Message;
    use topgun_core::{ClockSource, Principal, SystemClock, HLC};

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::classify::OperationService;
    use crate::service::dispatch::DispatchConfig;
    use crate::service::domain::crdt::CrdtService;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::schema::SchemaService;
    use crate::service::middleware::pipeline::build_operation_pipeline;
    use crate::service::policy::{
        InMemoryPolicyStore, PermissionAction, PermissionPolicy, PolicyEffect, PolicyEvaluator,
        PolicyStore,
    };
    use crate::service::router::OperationRouter;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::expiry::ManualClock;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::RecordValue;

    const DAY: u64 = 86_400_000;
    const MAP: &str = "acme.docs";
    const KEY: &str = "k";

    fn hlc() -> Arc<Mutex<HLC>> {
        Arc::new(Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )))
    }

    /// Every policy the store holds denies: everything, and the deny policy a
    /// tenant delete leaves for `acme`.
    async fn deny_everything() -> Arc<PolicyEvaluator> {
        let store = Arc::new(InMemoryPolicyStore::new());
        for (id, pattern) in [("deny-all", "**"), ("tenant-deleted:acme", "acme.**")] {
            store
                .upsert_policy(PermissionPolicy {
                    id: id.to_string(),
                    map_pattern: pattern.to_string(),
                    action: PermissionAction::All,
                    effect: PolicyEffect::Deny,
                    condition: None,
                })
                .await
                .expect("upsert policy");
        }
        Arc::new(PolicyEvaluator::new(store))
    }

    fn put(origin: CallerOrigin, stamp: &Timestamp, ttl_ms: Option<u64>) -> Operation {
        let mut ctx = OperationContext::new(1, service_names::CRDT, stamp.clone(), 5_000);
        ctx.partition_id = Some(hash_to_partition(KEY));
        ctx.caller_origin = origin;
        if origin == CallerOrigin::Client {
            ctx.principal = Some(Principal {
                id: "user-a".to_string(),
                roles: vec![],
                tenant: Some("acme".to_string()),
            });
        }
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("put".to_string()),
                    map_name: MAP.to_string(),
                    key: KEY.to_string(),
                    op_type: None,
                    record: Some(Some(topgun_core::LWWRecord {
                        value: Some(rmpv::Value::from("v")),
                        timestamp: stamp.clone(),
                        ttl_ms,
                    })),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    #[test]
    fn expire_record_op_targets_the_key_partition_of_the_crdt_service_as_system() {
        let op = expire_record_op(MAP, KEY, ExpiryExpectation::Or);
        let ctx = op.ctx();
        assert_eq!(ctx.partition_id, Some(hash_to_partition(KEY)));
        assert_eq!(ctx.service_name, service_names::CRDT);
        assert_eq!(ctx.caller_origin, CallerOrigin::System);
        assert!(ctx.principal.is_none());
        assert!(ctx.connection_id.is_none());
    }

    /// An `ExpireRecord` sent through `ExpiryDispatch` on a real partition
    /// dispatcher, whose pipeline carries the authorization layer over a store
    /// that denies every map and holds a tenant's deny policy, reaches the CRDT
    /// service and removes the expired record; a client write of the same map
    /// through the same dispatcher is refused, so the layer is in force.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expire_record_reaches_the_handler_through_the_dispatcher_and_authorization() {
        let wall = SystemClock.now();
        let clock = Arc::new(ManualClock::new(wall + 30 * DAY));
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_clock(Arc::clone(&clock) as Arc<dyn ClockSource>),
        );
        let crdt = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(WriteAdmission::new(
                Arc::new(SecurityConfig::default()),
                hlc(),
            )),
            Arc::new(QueryRegistry::new()),
            Arc::new(SchemaService::new()),
        ));
        let evaluator = deny_everything().await;
        let dispatcher = PartitionDispatcher::new(
            &DispatchConfig {
                worker_count: 2,
                channel_buffer_size: 16,
            },
            || {
                let mut router = OperationRouter::new();
                router.register(service_names::CRDT, Arc::clone(&crdt));
                build_operation_pipeline(
                    router,
                    &ServerConfig::default(),
                    Some(Arc::clone(&evaluator)),
                )
            },
        );
        let stamp = Timestamp {
            millis: wall,
            counter: 1,
            node_id: "writer".to_string(),
        };

        let refused = PartitionDispatcher::dispatch(
            &dispatcher,
            put(CallerOrigin::Client, &stamp, Some(DAY)),
        )
        .await;
        assert!(
            matches!(refused, Err(OperationError::Forbidden { .. })),
            "the authorization layer refuses a client write: {refused:?}"
        );
        PartitionDispatcher::dispatch(&dispatcher, put(CallerOrigin::System, &stamp, Some(DAY)))
            .await
            .expect("a trusted write is applied");
        let store = factory.get_or_create(MAP, hash_to_partition(KEY));
        assert!(matches!(
            store.get(KEY, false).await.unwrap().map(|r| r.value),
            Some(RecordValue::Lww { .. })
        ));

        let via: &dyn ExpiryDispatch = &dispatcher;
        let resp = via
            .dispatch(expire_record_op(
                MAP,
                KEY,
                ExpiryExpectation::Lww {
                    timestamp: stamp.clone(),
                },
            ))
            .await
            .expect("the expire passes authorization");

        assert!(matches!(resp, OperationResponse::Empty), "{resp:?}");
        assert!(
            store.get(KEY, false).await.unwrap().is_none(),
            "the expired record is removed"
        );
    }

    /// `classify` builds no `ExpireRecord` from any message: checked against
    /// every message of the cross-language fixture corpus, the encodings the
    /// TypeScript client produces, under a client and a trusted origin. The
    /// corpus also holds HTTP sync bodies, which are not messages and are
    /// passed over.
    #[test]
    fn classify_never_yields_expire_record() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core-rust/tests/fixtures");
        let service = OperationService::new(hlc(), Arc::new(ServerConfig::default()));
        let mut messages = 0;
        let mut classified = 0;
        for entry in std::fs::read_dir(&dir).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("msgpack") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("fixture bytes");
            let Ok(msg) = rmp_serde::from_slice::<Message>(&bytes) else {
                continue;
            };
            messages += 1;
            for origin in [CallerOrigin::Client, CallerOrigin::System] {
                if let Ok(op) = service.classify(msg.clone(), None, origin) {
                    classified += 1;
                    assert!(
                        !matches!(op, Operation::ExpireRecord { .. }),
                        "{} classified to ExpireRecord",
                        path.display()
                    );
                }
            }
        }
        assert!(messages >= 60, "the corpus is there: {messages} messages");
        assert!(classified > 0, "some fixtures classify");
    }
}

#[cfg(test)]
mod sweep_tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use parking_lot::Mutex;
    use tokio::sync::{watch, Notify};
    use tokio::time::Instant;
    use topgun_core::types::Value;
    use topgun_core::{SystemClock, Timestamp, HLC};

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::schema::SchemaService;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::expiry::ManualClock;
    use crate::storage::factory::test_store::TestStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch};
    use crate::storage::record::OrMapEntry;

    const NOW: u64 = 10_000;
    const DAY: u64 = 86_400_000;
    /// Expired at [`NOW`]: stamped 1 000 with a 10 ms TTL.
    const EXPIRED: Option<u64> = Some(10);
    /// Live at [`NOW`].
    const LIVE: Option<u64> = Some(DAY);

    /// A [`TestStore`] that also lists its maps, counts `list_maps` calls and
    /// can drop a map, as a tenant delete does.
    struct ListingStore {
        rows: TestStore,
        listed: Mutex<BTreeSet<String>>,
        list_calls: AtomicU64,
    }

    impl ListingStore {
        fn put(&self, map: &str, key: &str, value: RecordValue) {
            self.listed.lock().insert(map.to_string());
            self.rows.put_row(map, key, value);
        }

        fn drop_map(&self, map: &str, keys: &[String]) {
            self.listed.lock().remove(map);
            for key in keys {
                self.rows.delete_row(map, key);
            }
        }
    }

    #[async_trait]
    impl MapDataStore for ListingStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.rows.add(map, key, value, expiration_time, now).await
        }
        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.rows
                .add_backup(map, key, value, expiration_time, now)
                .await
        }
        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.rows.remove(map, key, now).await
        }
        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.rows.remove_backup(map, key, now).await
        }
        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.rows.load(map, key).await
        }
        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.rows.load_all(map, keys).await
        }
        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.rows.remove_all(map, keys).await
        }
        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.rows.enumerate_leaves(map, is_backup, sink).await
        }
        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            assert_eq!(max_batch_cost, SWEEP_BATCH_COST);
            self.rows.scan_values(map, is_backup, max_batch_cost).await
        }
        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            assert_eq!(max_batch_cost, SWEEP_BATCH_COST);
            self.rows
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }
        async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.listed.lock().iter().cloned().collect())
        }
        fn is_loadable(&self, key: &str) -> bool {
            self.rows.is_loadable(key)
        }
        fn pending_operation_count(&self) -> u64 {
            self.rows.pending_operation_count()
        }
        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.rows.soft_flush().await
        }
        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.rows.hard_flush().await
        }
        fn is_null(&self) -> bool {
            self.rows.is_null()
        }
    }

    /// Records every removal it is sent and answers from a script, then
    /// forwards to `inner` (or answers `Empty`).
    #[derive(Default)]
    struct Scripted {
        sent: Mutex<Vec<(String, String, ExpiryExpectation)>>,
        replies: Mutex<VecDeque<Result<(), OperationError>>>,
        refuse_all: AtomicBool,
        inner: Option<Arc<dyn ExpiryDispatch>>,
    }

    impl Scripted {
        fn forwarding(inner: Arc<dyn ExpiryDispatch>) -> Self {
            Self {
                inner: Some(inner),
                ..Self::default()
            }
        }

        fn refuse_next(&self, err: OperationError) {
            self.replies.lock().push_back(Err(err));
        }

        fn accept_next(&self) {
            self.replies.lock().push_back(Ok(()));
        }

        fn keys(&self) -> Vec<String> {
            self.sent
                .lock()
                .iter()
                .map(|(map, key, _)| format!("{map}/{key}"))
                .collect()
        }
    }

    #[async_trait]
    impl ExpiryDispatch for Scripted {
        async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError> {
            let Operation::ExpireRecord {
                ref map_name,
                ref key,
                ref expected,
                ref ctx,
            } = op
            else {
                panic!("the sweep sends only ExpireRecord: {op:?}");
            };
            assert_eq!(
                ctx.call_timeout_ms, 4321,
                "the configured operation timeout"
            );
            self.sent
                .lock()
                .push((map_name.clone(), key.clone(), expected.clone()));
            if self.refuse_all.load(Ordering::SeqCst) {
                return Err(OperationError::Overloaded);
            }
            if let Some(Err(err)) = self.replies.lock().pop_front() {
                return Err(err);
            }
            match &self.inner {
                Some(inner) => inner.dispatch(op).await,
                None => Ok(OperationResponse::Empty),
            }
        }
    }

    struct Fixture {
        store: Arc<ListingStore>,
        factory: Arc<RecordStoreFactory>,
    }

    fn fixture_over(rows: TestStore) -> Fixture {
        let store = Arc::new(ListingStore {
            rows,
            listed: Mutex::new(BTreeSet::new()),
            list_calls: AtomicU64::new(0),
        });
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::clone(&store) as Arc<dyn MapDataStore>,
                Vec::new(),
            )
            .with_clock(Arc::new(ManualClock::new(NOW))),
        );
        Fixture { store, factory }
    }

    fn fixture(batch_rows: usize) -> Fixture {
        fixture_over(TestStore::new(batch_rows))
    }

    fn config(rows_per_sec: u64) -> SweepConfig {
        SweepConfig {
            interval_ms: 1_000,
            rows_per_sec,
            operation_timeout_ms: 4321,
        }
    }

    /// A sweep whose first cycle may start at once.
    fn ready(fx: &Fixture, dispatch: Arc<dyn ExpiryDispatch>, rows_per_sec: u64) -> SweepState {
        let mut state = SweepState::new(Arc::clone(&fx.factory), dispatch, config(rows_per_sec));
        state.next_cycle_at = Instant::now();
        state
    }

    /// Starts a cycle now and ticks until it completes.
    async fn run_cycle(state: &mut SweepState) -> SweepTickReport {
        assert!(state.cycle.is_none(), "no cycle runs");
        state.next_cycle_at = Instant::now();
        let mut total = SweepTickReport::default();
        for _ in 0..10_000 {
            let r = sweep_tick(state).await;
            total.cycle_started |= r.cycle_started;
            total.rows_scanned += r.rows_scanned;
            total.candidates += r.candidates;
            total.maps_skipped += r.maps_skipped;
            total.dispatched += r.dispatched;
            if r.cycle_completed {
                total.cycle_completed = true;
                return total;
            }
        }
        panic!("the cycle did not complete");
    }

    fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        }
    }

    fn lww(ttl_ms: Option<u64>) -> RecordValue {
        RecordValue::Lww {
            value: Value::Int(1),
            timestamp: ts(1_000),
            ttl_ms,
        }
    }

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("k{i:04}")).collect()
    }

    fn fill(fx: &Fixture, map: &str, n: usize, ttl_ms: Option<u64>) -> Vec<String> {
        let keys = keys(n);
        for key in &keys {
            fx.store.put(map, key, lww(ttl_ms));
        }
        keys
    }

    fn hlc() -> Arc<parking_lot::Mutex<HLC>> {
        Arc::new(parking_lot::Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )))
    }

    fn crdt(fx: &Fixture) -> Arc<CrdtService> {
        Arc::new(CrdtService::new(
            Arc::clone(&fx.factory),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(WriteAdmission::new(
                Arc::new(SecurityConfig::default()),
                hlc(),
            )),
            Arc::new(QueryRegistry::new()),
            Arc::new(SchemaService::new()),
        ))
    }

    /// A trusted LWW PUT, stored with the stamp and TTL given.
    async fn put(crdt: &Arc<CrdtService>, map: &str, key: &str, stamp: u64, ttl_ms: Option<u64>) {
        let mut ctx = OperationContext::new(1, service_names::CRDT, ts(stamp), 5_000);
        ctx.partition_id = Some(hash_to_partition(key));
        ctx.caller_origin = CallerOrigin::System;
        let op = Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: None,
                    map_name: map.to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: Some(Some(topgun_core::LWWRecord {
                        value: Some(rmpv::Value::from(stamp)),
                        timestamp: ts(stamp),
                        ttl_ms,
                    })),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        Service::call(&mut Arc::clone(crdt), op).await.expect("put");
    }

    fn series(rendered: &str, series: &str) -> Option<String> {
        rendered.lines().find_map(|line| {
            line.strip_prefix(series)
                .and_then(|rest| rest.strip_prefix(' '))
                .map(str::to_string)
        })
    }

    // --- budget, remainder, cursor, queue ---------------------------------

    #[tokio::test]
    async fn sweep_budget_limits_rows_and_dispatches_per_tick() {
        let fx = fixture(1_000);
        fill(&fx, "live", 25, LIVE);
        let dispatch = Arc::new(Scripted::default());
        let mut state = ready(&fx, dispatch.clone(), 40);
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(sweep_tick(&mut state).await.rows_scanned);
        }
        assert_eq!(seen, vec![10, 10, 5], "ceil(40 / 4) rows per tick");

        let fx = fixture(1_000);
        fill(&fx, "gone", 300, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        let first = sweep_tick(&mut state).await;
        assert_eq!((first.candidates, first.dispatched), (300, 256));
        assert_eq!(dispatch.sent.lock().len(), 256);
        let second = sweep_tick(&mut state).await;
        assert_eq!(second.dispatched, 44);
        assert_eq!(
            SweepConfig {
                rows_per_sec: 4_097,
                ..config(0)
            }
            .rows_per_tick(),
            1_025
        );
    }

    #[tokio::test]
    async fn sweep_resumes_mid_batch_from_remainder() {
        let fx = fixture(10);
        let keys = fill(&fx, "m", 25, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        let mut state = ready(&fx, dispatch.clone(), 12);
        let mut ticks = Vec::new();
        loop {
            let r = sweep_tick(&mut state).await;
            ticks.push(r.rows_scanned);
            if r.cycle_completed {
                break;
            }
        }
        assert!(ticks.iter().all(|&rows| rows <= 3), "{ticks:?}");
        let expected: Vec<String> = keys.iter().map(|k| format!("m/{k}")).collect();
        assert_eq!(dispatch.keys(), expected, "every row once, in order");
        assert_eq!(
            fx.store.rows.calls(),
            3,
            "three fetches of 10, 10 and 5 rows: no row is read twice"
        );
    }

    #[tokio::test]
    async fn sweep_resumes_across_ticks_and_maps_in_order() {
        let fx = fixture(10);
        let a = fill(&fx, "a", 4, EXPIRED);
        let b = fill(&fx, "b", 4, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        let mut state = ready(&fx, dispatch.clone(), 12);
        let report = run_cycle(&mut state).await;
        assert_eq!(report.rows_scanned, 8);
        let expected: Vec<String> = a
            .iter()
            .map(|k| format!("a/{k}"))
            .chain(b.iter().map(|k| format!("b/{k}")))
            .collect();
        assert_eq!(dispatch.keys(), expected);
    }

    #[tokio::test]
    async fn sweep_cursor_holds_while_queue_full() {
        let fx = fixture(100);
        fill(&fx, "m", 1_100, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        dispatch.refuse_all.store(true, Ordering::SeqCst);
        let mut state = ready(&fx, dispatch.clone(), 8_192);
        let first = sweep_tick(&mut state).await;
        assert_eq!(first.rows_scanned, 1_024);
        assert_eq!(state.queue.len(), SWEEP_QUEUE_CAPACITY);
        let calls = fx.store.rows.calls();
        let remainder = state
            .cycle
            .as_ref()
            .unwrap()
            .pass
            .as_ref()
            .unwrap()
            .remainder
            .len();
        for _ in 0..3 {
            let r = sweep_tick(&mut state).await;
            assert_eq!(r.rows_scanned, 0);
        }
        assert_eq!(
            fx.store.rows.calls(),
            calls,
            "no fetch while the queue is full"
        );
        assert_eq!(
            state
                .cycle
                .as_ref()
                .unwrap()
                .pass
                .as_ref()
                .unwrap()
                .remainder
                .len(),
            remainder,
            "the remainder does not move"
        );
    }

    // --- dispatch and back-off --------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn sweep_overloaded_keeps_candidate_and_backs_off() {
        let fx = fixture(100);
        fill(&fx, "m", 1, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        dispatch.refuse_next(OperationError::Overloaded);
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        assert_eq!(sweep_tick(&mut state).await.dispatched, 1);
        assert_eq!(state.queue.len(), 1, "the candidate stays at the head");
        let held = sweep_tick(&mut state).await;
        assert!(held.backing_off && held.dispatched == 0, "{held:?}");
        tokio::time::advance(Duration::from_millis(250)).await;
        assert_eq!(sweep_tick(&mut state).await.dispatched, 1);
        assert_eq!(dispatch.keys(), vec!["m/k0000", "m/k0000"]);
        assert!(state.queue.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_failed_dispatch_drops_candidate_and_backs_off() {
        let fx = fixture(100);
        fill(&fx, "m", 2, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        dispatch.refuse_next(OperationError::Internal(anyhow::anyhow!("refused")));
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        assert_eq!(sweep_tick(&mut state).await.dispatched, 1);
        assert_eq!(state.queue.len(), 1, "the failed candidate is dropped");
        assert!(sweep_tick(&mut state).await.backing_off);
        tokio::time::advance(Duration::from_millis(250)).await;
        sweep_tick(&mut state).await;
        assert_eq!(dispatch.keys(), vec!["m/k0000", "m/k0001"]);
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_backoff_doubles_to_cap_and_resets_on_success() {
        let fx = fixture(100);
        fill(&fx, "m", 2, EXPIRED);
        let dispatch = Arc::new(Scripted::default());
        for _ in 0..7 {
            dispatch.refuse_next(OperationError::Overloaded);
        }
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        state.config.interval_ms = 1_000_000;
        assert_eq!(sweep_tick(&mut state).await.dispatched, 1);
        for delay in [250, 500, 1_000, 2_000, 4_000, 5_000] {
            tokio::time::advance(Duration::from_millis(delay - 1)).await;
            assert!(sweep_tick(&mut state).await.backing_off, "{delay} ms");
            tokio::time::advance(Duration::from_millis(1)).await;
            assert_eq!(sweep_tick(&mut state).await.dispatched, 1, "{delay} ms");
        }
        // The seventh refusal waits the cap again; the next attempt succeeds and
        // the one after it is refused: the back-off starts over at 250 ms.
        dispatch.accept_next();
        dispatch.refuse_next(OperationError::Overloaded);
        tokio::time::advance(Duration::from_millis(4_999)).await;
        assert!(sweep_tick(&mut state).await.backing_off, "capped at 5 s");
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(sweep_tick(&mut state).await.dispatched, 2);
        tokio::time::advance(Duration::from_millis(249)).await;
        assert!(sweep_tick(&mut state).await.backing_off);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(
            sweep_tick(&mut state).await.dispatched,
            1,
            "reset to 250 ms"
        );
    }

    // --- cycles -------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn sweep_first_cycle_waits_one_interval_and_cycles_are_spaced_from_start() {
        let fx = fixture(100);
        fill(&fx, "m", 3, LIVE);
        let dispatch = Arc::new(Scripted::default());
        let start = Instant::now();
        let mut state = SweepState::new(Arc::clone(&fx.factory), dispatch, config(4));
        let mut starts = Vec::new();
        let mut ends = Vec::new();
        for _ in 0..=12 {
            let r = sweep_tick(&mut state).await;
            let at = start.elapsed().as_millis();
            if r.cycle_started {
                starts.push(at);
            }
            if r.cycle_completed {
                ends.push(at);
            }
            tokio::time::advance(SWEEP_TICK).await;
        }
        assert_eq!(starts, vec![1_000, 2_000, 3_000]);
        assert_eq!(ends[0], 1_500, "the first cycle takes three ticks");
    }

    #[tokio::test]
    async fn sweep_forgets_a_map_that_left_the_union() {
        let fx = fixture(100);
        let a = fill(&fx, "a", 2, LIVE);
        fill(&fx, "b", 2, LIVE);
        let mut state = ready(&fx, Arc::new(Scripted::default()), 4_096);
        run_cycle(&mut state).await;
        assert!(state.last_pass.contains_key("a"));
        assert_eq!(fx.store.rows.scans("a"), 1);

        fx.store.drop_map("a", &a);
        run_cycle(&mut state).await;
        assert_eq!(fx.store.rows.scans("a"), 1, "a departed map is not read");
        assert_eq!(fx.store.rows.scans("b"), 2);
        assert!(!state.last_pass.contains_key("a"), "it lost its entry");
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_dispatches_queued_candidates_of_a_departed_map() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let fx = fixture(100);
        let keys = fill(&fx, "acme.docs", 5, EXPIRED);
        let crdt = crdt(&fx);
        let dispatch = Arc::new(Scripted::forwarding(Arc::new(CrdtServiceDispatch(crdt))));
        dispatch.refuse_next(OperationError::Overloaded);
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        assert_eq!(sweep_tick(&mut state).await.candidates, 5);
        assert_eq!(state.queue.len(), 5);

        fx.store.drop_map("acme.docs", &keys);
        tokio::time::advance(Duration::from_millis(250)).await;
        assert_eq!(sweep_tick(&mut state).await.dispatched, 5);
        assert!(state.queue.is_empty());
        assert_eq!(
            series(
                &handle.render(),
                &format!("{METRIC_EXPIRY_SKIPPED_TOTAL}{{reason=\"absent\"}}")
            )
            .as_deref(),
            Some("5"),
            "{}",
            handle.render()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_stale_scanned_row_is_skipped_changed() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let fx = fixture(100);
        fx.store.put("m", "k", lww(EXPIRED));
        let crdt = crdt(&fx);
        let dispatch = Arc::new(Scripted::forwarding(Arc::new(CrdtServiceDispatch(
            Arc::clone(&crdt),
        ))));
        dispatch.refuse_next(OperationError::Overloaded);
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        assert_eq!(sweep_tick(&mut state).await.candidates, 1);

        put(&crdt, "m", "k", 5_000, None).await;
        tokio::time::advance(Duration::from_millis(250)).await;
        assert_eq!(sweep_tick(&mut state).await.dispatched, 1);
        assert_eq!(
            series(
                &handle.render(),
                &format!("{METRIC_EXPIRY_SKIPPED_TOTAL}{{reason=\"changed\"}}")
            )
            .as_deref(),
            Some("1")
        );
        let stored = fx
            .factory
            .get_or_create("m", hash_to_partition("k"))
            .get("k", false)
            .await
            .unwrap()
            .map(|r| r.value);
        assert!(
            matches!(stored, Some(RecordValue::Lww { ref timestamp, .. }) if timestamp.millis == 5_000),
            "the newer value survives: {stored:?}"
        );
    }

    /// A map that is not resident costs the handler two `list_maps` calls for
    /// its first removal only: that removal creates the map's store, and every
    /// later removal of the map finds it resident. The sweep itself lists the
    /// maps once per cycle.
    #[tokio::test]
    async fn sweep_counts_list_maps_per_expire() {
        let fx = fixture(1_000);
        fill(&fx, "cold", 10, EXPIRED);
        let dispatch: Arc<dyn ExpiryDispatch> = Arc::new(CrdtServiceDispatch(crdt(&fx)));
        let mut state = SweepState::new(Arc::clone(&fx.factory), dispatch, config(4_096));
        state.config.operation_timeout_ms = 4321;
        let report = run_cycle(&mut state).await;
        assert_eq!(report.dispatched, 10);
        assert!(fx.store.rows.row("cold", "k0000").is_none(), "removed");
        assert_eq!(
            fx.store.list_calls.load(Ordering::SeqCst),
            1 + 2,
            "one listing for the cycle, two for the first removal, none for the other nine"
        );
    }

    // --- the skip -------------------------------------------------------------

    #[tokio::test]
    async fn sweep_skips_a_map_without_ttl_rows_on_the_next_cycle() {
        let fx = fixture(100);
        fill(&fx, "plain", 3, None);
        fill(&fx, "ttl", 1, LIVE);
        let mut state = ready(&fx, Arc::new(Scripted::default()), 4_096);
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 0);
        let second = run_cycle(&mut state).await;
        assert_eq!(second.maps_skipped, 1);
        assert_eq!(fx.store.rows.scans("plain"), 1);
        assert_eq!(
            fx.store.rows.scans("ttl"),
            2,
            "a map with a TTL row is read"
        );
    }

    #[tokio::test]
    async fn sweep_unskips_map_after_ttl_write() {
        let fx = fixture(100);
        fill(&fx, "plain", 3, None);
        let mut state = ready(&fx, Arc::new(Scripted::default()), 4_096);
        run_cycle(&mut state).await;
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 1);
        fx.factory.note_ttl_write("plain");
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 0);
        assert_eq!(fx.store.rows.scans("plain"), 2);
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 1, "skipped again");
    }

    #[tokio::test]
    async fn sweep_ttl_write_during_pass_prevents_skip() {
        let fx = fixture(100);
        fill(&fx, "plain", 3, None);
        let mut state = ready(&fx, Arc::new(Scripted::default()), 4);
        let first = sweep_tick(&mut state).await;
        assert!(first.cycle_started && !first.cycle_completed);
        fx.factory.note_ttl_write("plain");
        while !sweep_tick(&mut state).await.cycle_completed {}
        assert_eq!(
            run_cycle(&mut state).await.maps_skipped,
            0,
            "a TTL write during the pass"
        );
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 1);
    }

    #[tokio::test]
    async fn sweep_backstop_cycle_ignores_the_skip() {
        let fx = fixture(100);
        fill(&fx, "plain", 3, None);
        let mut state = ready(&fx, Arc::new(Scripted::default()), 4_096);
        run_cycle(&mut state).await;
        state.cycles_started = SWEEP_SKIP_BACKSTOP_CYCLES - 2;
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 1, "cycle 359");
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 0, "cycle 360");
        assert_eq!(run_cycle(&mut state).await.maps_skipped, 1, "cycle 361");
        assert_eq!(fx.store.rows.scans("plain"), 2);
    }

    #[tokio::test]
    async fn sweep_queues_or_slots_with_an_expired_entry() {
        let entry = |tag: &str, ttl_ms| OrMapEntry {
            value: Value::Int(1),
            tag: tag.to_string(),
            timestamp: ts(1_000),
            ttl_ms,
        };
        let fx = fixture(100);
        fx.store.put(
            "or",
            "mixed",
            RecordValue::OrMap {
                records: vec![entry("old", EXPIRED), entry("new", LIVE)],
                tombstones: vec![],
            },
        );
        fx.store.put(
            "or",
            "live",
            RecordValue::OrMap {
                records: vec![entry("a", LIVE)],
                tombstones: vec![],
            },
        );
        let dispatch = Arc::new(Scripted::default());
        let mut state = ready(&fx, dispatch.clone(), 4_096);
        run_cycle(&mut state).await;
        assert_eq!(
            *dispatch.sent.lock(),
            vec![("or".to_string(), "mixed".to_string(), ExpiryExpectation::Or)]
        );
        assert!(state.last_pass["or"].found_ttl_row);
    }

    #[tokio::test]
    async fn sweep_null_store_visits_residents_once_per_cycle_and_caps_candidates() {
        let fx = fixture_over(TestStore::null());
        let crdt = crdt(&fx);
        for i in 0..300 {
            put(&crdt, "n", &format!("k{i:03}"), 1_000, EXPIRED).await;
        }
        let dispatch = Arc::new(Scripted::forwarding(Arc::new(CrdtServiceDispatch(
            Arc::clone(&crdt),
        ))));
        let mut state = ready(&fx, dispatch.clone(), 4);
        let first = run_cycle(&mut state).await;
        assert_eq!(
            (first.rows_scanned, first.candidates, first.dispatched),
            (300, 256, 256),
            "every resident row is visited, and at most 256 are queued"
        );
        let second = run_cycle(&mut state).await;
        assert_eq!(
            (second.rows_scanned, second.candidates, second.dispatched),
            (44, 44, 44)
        );
        assert_eq!(run_cycle(&mut state).await.rows_scanned, 0);
    }

    // --- the task ----------------------------------------------------------------

    const FAMILIES: [&str; 11] = [
        METRIC_SWEEP_ROWS_SCANNED_TOTAL,
        METRIC_SWEEP_CANDIDATES_TOTAL,
        METRIC_EXPIRY_REMOVED_TOTAL,
        METRIC_EXPIRY_SKIPPED_TOTAL,
        METRIC_SWEEP_DISPATCH_OVERLOADED_TOTAL,
        METRIC_SWEEP_DISPATCH_FAILED_TOTAL,
        METRIC_SWEEP_CYCLES_TOTAL,
        METRIC_SWEEP_LAST_CYCLE_SECONDS,
        METRIC_SWEEP_TASK_ALIVE,
        METRIC_SWEEP_PASS_PANICS_TOTAL,
        METRIC_SWEEP_MAPS_SKIPPED_TOTAL,
    ];

    #[tokio::test(start_paused = true)]
    async fn sweep_metrics_present_at_zero_before_first_tick() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let fx = fixture(100);
        let (_tx, rx) = watch::channel(false);
        let task = metrics::with_local_recorder(&recorder, || {
            spawn_expiry_sweep(
                Arc::clone(&fx.factory),
                Arc::new(Scripted::default()),
                config(4_096),
                rx,
            )
        });
        // Read before the task has been polled once.
        let rendered = handle.render();
        let mut expected: Vec<String> = FAMILIES
            .iter()
            .filter(|f| {
                ![
                    METRIC_SWEEP_CANDIDATES_TOTAL,
                    METRIC_EXPIRY_REMOVED_TOTAL,
                    METRIC_EXPIRY_SKIPPED_TOTAL,
                ]
                .contains(f)
            })
            .map(|f| (*f).to_string())
            .collect();
        for kind in ["lww", "or"] {
            expected.push(format!(
                "{METRIC_SWEEP_CANDIDATES_TOTAL}{{kind=\"{kind}\"}}"
            ));
        }
        for kind in ["lww_record", "or_tag"] {
            expected.push(format!("{METRIC_EXPIRY_REMOVED_TOTAL}{{kind=\"{kind}\"}}"));
        }
        for reason in ["absent", "changed", "not_expired"] {
            expected.push(format!(
                "{METRIC_EXPIRY_SKIPPED_TOTAL}{{reason=\"{reason}\"}}"
            ));
        }
        assert_eq!(expected.len(), 15);
        for s in &expected {
            let want = if s == METRIC_SWEEP_TASK_ALIVE {
                "1"
            } else {
                "0"
            };
            assert_eq!(
                series(&rendered, s).as_deref(),
                Some(want),
                "{s}:\n{rendered}"
            );
        }
        task.abort();
        let _ = task.await;
        assert_eq!(
            series(&handle.render(), METRIC_SWEEP_TASK_ALIVE).as_deref(),
            Some("0"),
            "the lease clears liveness when the task is dropped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_pass_panic_is_counted_and_the_task_keeps_running() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let fx = fixture(100);
        fill(&fx, "m", 2, LIVE);
        fx.store.rows.panic_scans(true);
        let (tx, rx) = watch::channel(false);
        let task = metrics::with_local_recorder(&recorder, || {
            spawn_expiry_sweep(
                Arc::clone(&fx.factory),
                Arc::new(Scripted::default()),
                config(4_096),
                rx,
            )
        });
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(
            series(&handle.render(), METRIC_SWEEP_PASS_PANICS_TOTAL).as_deref(),
            Some("1")
        );
        fx.store.rows.panic_scans(false);
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        assert!(!task.is_finished());
        assert_eq!(
            series(&handle.render(), METRIC_SWEEP_CYCLES_TOTAL).as_deref(),
            Some("1"),
            "the next cycle runs after the interval"
        );
        tx.send(true).unwrap();
        task.await.expect("the task ends on shutdown");
        assert_eq!(
            series(&handle.render(), METRIC_SWEEP_TASK_ALIVE).as_deref(),
            Some("0")
        );
    }

    /// Holds the first removal until released.
    struct Held {
        sent: AtomicU64,
        entered: Notify,
        release: Notify,
    }

    #[async_trait]
    impl ExpiryDispatch for Held {
        async fn dispatch(&self, _op: Operation) -> Result<OperationResponse, OperationError> {
            if self.sent.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(OperationResponse::Empty)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_shutdown_ends_task_between_dispatches() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let fx = fixture(100);
        fill(&fx, "m", 3, EXPIRED);
        let held = Arc::new(Held {
            sent: AtomicU64::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let (tx, rx) = watch::channel(false);
        let task = metrics::with_local_recorder(&recorder, || {
            spawn_expiry_sweep(Arc::clone(&fx.factory), held.clone(), config(4_096), rx)
        });
        held.entered.notified().await;
        tx.send(true).unwrap();
        held.release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task ends")
            .expect("cleanly");
        assert_eq!(
            held.sent.load(Ordering::SeqCst),
            1,
            "the removal in flight finished, no other was sent"
        );
        assert_eq!(
            series(&handle.render(), METRIC_SWEEP_TASK_ALIVE).as_deref(),
            Some("0")
        );
    }

    // --- configuration ---------------------------------------------------------

    #[test]
    fn sweep_config_parses_the_knobs() {
        let parse = |i: Option<&str>, r: Option<&str>| SweepConfig::from_env_values(i, r, 5_000);
        assert_eq!(
            parse(None, None),
            Ok(SweepConfig {
                interval_ms: 10_000,
                rows_per_sec: 4_096,
                operation_timeout_ms: 5_000,
            })
        );
        assert_eq!(parse(Some("1000"), Some("1")).unwrap().interval_ms, 1_000);
        assert_eq!(parse(Some(" 2500 "), None).unwrap().interval_ms, 2_500);
        assert_eq!(parse(None, Some("1")).unwrap().rows_per_sec, 1);
        assert!(parse(Some("999"), None)
            .unwrap_err()
            .contains("TOPGUN_EXPIRY_SWEEP_INTERVAL_MS"));
        assert!(parse(Some("0"), None).is_err());
        assert!(parse(None, Some("0"))
            .unwrap_err()
            .contains("TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC"));
        for garbage in ["", "abc", "-5", "1.5", "\u{FFFD}"] {
            assert_eq!(
                parse(Some(garbage), Some(garbage)),
                parse(None, None),
                "{garbage:?}"
            );
        }
    }
}
