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
use crate::storage::map_data_store::ScanCursor;
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
            ignore_skip: self.cycles_started % SWEEP_SKIP_BACKSTOP_CYCLES == 0,
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
                let mut found = Vec::new();
                let mut rows = 0_u64;
                for store in self.factory.get_all_for_map(&pass.map) {
                    store.for_each_boxed(
                        &mut |key, record| {
                            rows += 1;
                            if let Some(expected) =
                                candidate(&record.value, now_ms, &mut pass.found_ttl_row)
                            {
                                if found.len() < room {
                                    found.push((key.to_string(), expected));
                                }
                            }
                        },
                        false,
                    );
                }
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
                let batch = match std::mem::replace(&mut pass.next, Fetch::Done) {
                    Fetch::Done => {
                        self.last_pass.insert(
                            pass.map.clone(),
                            LastPass {
                                found_ttl_row: pass.found_ttl_row,
                                generation_at_start: pass.generation_at_start,
                            },
                        );
                        cycle.pass = None;
                        continue;
                    }
                    Fetch::First => {
                        data_store
                            .scan_values(&pass.map, false, SWEEP_BATCH_COST)
                            .await
                    }
                    Fetch::After(cursor) => {
                        data_store
                            .scan_values_batched(&pass.map, false, cursor, SWEEP_BATCH_COST)
                            .await
                    }
                };
                match batch {
                    Ok(batch) => {
                        pass.remainder = batch.records.into();
                        pass.next = batch.next_cursor.map_or(Fetch::Done, Fetch::After);
                    }
                    Err(err) => {
                        // The pass is left unfinished, so no skip is decided
                        // from it; the next cycle reads the map again.
                        tracing::warn!(
                            map = %pass.map,
                            error = %err,
                            "expiry sweep: a scan failed; the map is read again next cycle"
                        );
                        cycle.pass = None;
                    }
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
