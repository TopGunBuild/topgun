//! Open-loop write load at a fixed aggregate rate.
//!
//! Every batch has a scheduled send instant fixed before the run starts. A
//! batch sent late because the server answered its predecessor late has its
//! ack latency measured from that instant, not from the moment it was actually
//! sent: a stalled server then shows up as latency instead of as a silently
//! lower offered load (coordinated omission). A batch the generator reached in
//! time is measured from its send, so the generator's own timer lag is not
//! charged to the server.
//!
//! Each connection keeps exactly one batch in flight. The server's `ERROR`
//! frame names no operation and answers to several in-flight batches on one
//! connection may arrive in any order, so only one outstanding batch makes
//! every frame attributable to its batch and the refusal count exact.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use topgun_core::hlc::{LWWRecord, Timestamp};
use topgun_core::messages::{ClientOp, Message, OpBatchMessage, OpBatchPayload, PingData};

use crate::connection_pool::ConnectionPool;
use crate::store_arm::{dir_bytes, EffectiveStoreConfig, RunDirs, ServerHandle, Teardown};

/// The highest latency the histograms hold, in microseconds (one hour), so a
/// coordinated-omission-corrected tail is never clipped.
const HIST_MAX_US: u64 = 3_600_000_000;

/// How many distinct `ERROR` messages are kept per code.
const SAMPLES_PER_CODE: usize = 5;

/// What the open-loop generator offers.
#[derive(Debug, Clone)]
pub struct OpenLoopConfig {
    /// Aggregate target writes per second.
    pub rate: u64,
    pub batch_size: usize,
    pub connections: usize,
    pub warmup_secs: u64,
    pub duration_secs: u64,
    pub key_mix: KeyMix,
    pub sample_ms: u64,
    pub ack_timeout_ms: u64,
    pub max_wake_lag_ms: u64,
    /// The TTL every sent record asks for, as a client sets it (`--ttl-ms`).
    pub ttl_ms: Option<u64>,
}

impl Default for OpenLoopConfig {
    fn default() -> Self {
        Self {
            rate: 10_000,
            batch_size: 10,
            connections: 200,
            warmup_secs: 10,
            duration_secs: 30,
            key_mix: KeyMix::default(),
            sample_ms: 1000,
            ack_timeout_ms: 10_000,
            max_wake_lag_ms: 5,
            ttl_ms: None,
        }
    }
}

impl OpenLoopConfig {
    /// Checks the values the schedule cannot run with, naming the flag.
    pub fn validate(&self) -> Result<(), String> {
        let positive = [
            ("--rate", self.rate),
            ("--batch-size", self.batch_size as u64),
            ("--connections", self.connections as u64),
            ("--duration", self.duration_secs),
            ("--sample-ms", self.sample_ms),
            ("--ack-timeout-ms", self.ack_timeout_ms),
        ];
        if let Some((flag, _)) = positive.iter().find(|(_, v)| *v == 0) {
            return Err(format!("{flag} must be greater than 0"));
        }
        let fraction = self.key_mix.new_key_fraction;
        if !(0.0..=1.0).contains(&fraction) {
            return Err("--new-key-fraction must be within [0, 1]".to_string());
        }
        if fraction < 1.0 && self.key_mix.hot_keys == 0 {
            return Err(
                "--hot-keys must be greater than 0 when --new-key-fraction is below 1".into(),
            );
        }
        Ok(())
    }
}

/// Which keys the writes go to, and how large their values are.
#[derive(Debug, Clone, Serialize)]
pub struct KeyMix {
    /// Share of writes to a never-written key; the rest rewrite a hot key.
    pub new_key_fraction: f64,
    /// Size of the shared set of rewritten keys.
    pub hot_keys: u64,
    /// Extra binary payload per value, in bytes.
    pub value_bytes: usize,
    pub seed: u64,
}

impl Default for KeyMix {
    fn default() -> Self {
        Self {
            new_key_fraction: 1.0,
            hot_keys: 10_000,
            value_bytes: 0,
            seed: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Answer classification
// ---------------------------------------------------------------------------

/// One server frame, reduced to what the classification reads.
#[derive(Debug, Clone)]
pub enum Frame {
    Rejected {
        op_id: String,
        code: Option<u32>,
    },
    Ack {
        results: Option<Vec<String>>,
    },
    Error {
        code: u32,
        message: String,
    },
    /// The server's answer to the heartbeat; it answers no batch.
    Pong,
    Other,
}

impl Frame {
    fn decode(bytes: &[u8]) -> Self {
        match rmp_serde::from_slice::<Message>(bytes) {
            Ok(Message::OpRejected(m)) => Self::Rejected {
                op_id: m.payload.op_id,
                code: m.payload.code,
            },
            Ok(Message::OpAck(m)) => Self::Ack {
                results: m
                    .payload
                    .results
                    .map(|r| r.into_iter().map(|res| res.op_id).collect()),
            },
            Ok(Message::Error { payload }) => Self::Error {
                code: payload.code,
                message: payload.message,
            },
            Ok(Message::Pong(_)) => Self::Pong,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OpState {
    Pending,
    Acked,
    RefusedPermanent(String),
    RefusedTransient(String),
}

/// The answer state of the one outstanding batch on a connection, whose ops
/// carry the consecutive ids `first_id ..first_id + len`.
#[derive(Debug)]
pub struct BatchAnswer {
    first_id: u64,
    ops: Vec<OpState>,
    /// Frames that named an op outside this batch, repeated a verdict, or
    /// were of a kind the batch exchange never sends.
    pub anomalies: u64,
    /// The `ERROR` frame's message, kept for the per-code samples.
    pub error_message: Option<(String, String)>,
}

/// What a frame did to the batch exchange.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Continue,
    Terminal,
}

/// Per-batch op counts once the exchange ended.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchCounts {
    pub acked: u64,
    pub refused_permanent: BTreeMap<String, u64>,
    pub refused_transient: BTreeMap<String, u64>,
    pub unanswered: u64,
}

impl BatchCounts {
    pub fn permanent(&self) -> u64 {
        self.refused_permanent.values().sum()
    }

    pub fn transient(&self) -> u64 {
        self.refused_transient.values().sum()
    }

    fn add(&mut self, other: Self) {
        self.acked += other.acked;
        self.unanswered += other.unanswered;
        for (code, n) in other.refused_permanent {
            *self.refused_permanent.entry(code).or_default() += n;
        }
        for (code, n) in other.refused_transient {
            *self.refused_transient.entry(code).or_default() += n;
        }
    }
}

fn code_key(code: Option<u32>) -> String {
    code.map_or_else(|| "none".to_string(), |c| c.to_string())
}

impl BatchAnswer {
    pub fn new(first_id: u64, len: usize) -> Self {
        Self {
            first_id,
            ops: vec![OpState::Pending; len],
            anomalies: 0,
            error_message: None,
        }
    }

    fn index_of(&self, op_id: &str) -> Option<usize> {
        let id: u64 = op_id.parse().ok()?;
        let idx = id.checked_sub(self.first_id)?;
        usize::try_from(idx).ok().filter(|i| *i < self.ops.len())
    }

    fn all_answered(&self) -> bool {
        self.ops.iter().all(|s| *s != OpState::Pending)
    }

    /// Applies one frame: an `OP_REJECTED` refuses its op permanently; an
    /// `OP_ACK` without `results` acks every op not already refused, one with
    /// `results` acks exactly the listed ops; an `ERROR` refuses transiently
    /// every op not already refused (the server sends all `OP_REJECTED`
    /// frames first, then one `ERROR` that acknowledges nothing).
    pub fn apply(&mut self, frame: Frame) -> Step {
        match frame {
            Frame::Rejected { op_id, code } => {
                match self.index_of(&op_id) {
                    Some(i) if self.ops[i] == OpState::Pending => {
                        self.ops[i] = OpState::RefusedPermanent(code_key(code));
                    }
                    _ => self.anomalies += 1,
                }
                if self.all_answered() {
                    Step::Terminal
                } else {
                    Step::Continue
                }
            }
            Frame::Ack { results: None } => {
                self.settle_pending(&OpState::Acked);
                Step::Terminal
            }
            Frame::Ack {
                results: Some(listed),
            } => {
                for op_id in listed {
                    match self.index_of(&op_id) {
                        Some(i) if self.ops[i] == OpState::Pending => {
                            self.ops[i] = OpState::Acked;
                        }
                        _ => self.anomalies += 1,
                    }
                }
                Step::Terminal
            }
            Frame::Error { code, message } => {
                let code = code.to_string();
                self.settle_pending(&OpState::RefusedTransient(code.clone()));
                self.error_message = Some((code, message));
                Step::Terminal
            }
            Frame::Pong => Step::Continue,
            Frame::Other => {
                self.anomalies += 1;
                Step::Continue
            }
        }
    }

    fn settle_pending(&mut self, to: &OpState) {
        for state in &mut self.ops {
            if *state == OpState::Pending {
                *state = to.clone();
            }
        }
    }

    /// Op counts; an op still pending (no terminal frame, or a partial ack
    /// that did not list it) is unanswered.
    pub fn counts(&self) -> BatchCounts {
        let mut counts = BatchCounts::default();
        for state in &self.ops {
            match state {
                OpState::Pending => counts.unanswered += 1,
                OpState::Acked => counts.acked += 1,
                OpState::RefusedPermanent(code) => {
                    *counts.refused_permanent.entry(code.clone()).or_default() += 1;
                }
                OpState::RefusedTransient(code) => {
                    *counts.refused_transient.entry(code.clone()).or_default() += 1;
                }
            }
        }
        counts
    }
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// Answers counted on wall-clock time for the timeline, warm-up included.
#[derive(Default)]
struct LiveCounters {
    acked: AtomicU64,
    refused: AtomicU64,
}

fn new_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, HIST_MAX_US, 3).expect("histogram bounds are valid")
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// One connection's share of the window's accounting.
struct ConnTally {
    scheduled: u64,
    sent: u64,
    counts: BatchCounts,
    ack: Histogram<u64>,
    service: Histogram<u64>,
    wake_lag: Histogram<u64>,
    behind_schedule_sends: u64,
    max_behind_us: u64,
    /// Connections given up on (0 or 1 per connection before the merge).
    abandoned: u64,
    anomalies: u64,
    samples: BTreeMap<String, Vec<String>>,
}

impl ConnTally {
    fn new() -> Self {
        Self {
            scheduled: 0,
            sent: 0,
            counts: BatchCounts::default(),
            ack: new_histogram(),
            service: new_histogram(),
            wake_lag: new_histogram(),
            behind_schedule_sends: 0,
            max_behind_us: 0,
            abandoned: 0,
            anomalies: 0,
            samples: BTreeMap::new(),
        }
    }

    fn absorb(&mut self, other: Self) {
        self.scheduled += other.scheduled;
        self.sent += other.sent;
        self.counts.add(other.counts);
        self.ack.add(&other.ack).expect("same histogram bounds");
        self.service
            .add(&other.service)
            .expect("same histogram bounds");
        self.wake_lag
            .add(&other.wake_lag)
            .expect("same histogram bounds");
        self.abandoned += other.abandoned;
        self.behind_schedule_sends += other.behind_schedule_sends;
        self.max_behind_us = self.max_behind_us.max(other.max_behind_us);
        self.anomalies += other.anomalies;
        for (code, messages) in other.samples {
            let kept = self.samples.entry(code).or_default();
            for m in messages {
                if kept.len() < SAMPLES_PER_CODE && !kept.contains(&m) {
                    kept.push(m);
                }
            }
        }
    }
}

/// The fixed instants every connection schedules against.
#[derive(Clone, Copy)]
struct Schedule {
    t0: Instant,
    window_start: Instant,
    window_end: Instant,
    stop: Instant,
    /// Offset of connection 0's first batch is 0; each next connection is one
    /// aggregate batch interval later.
    batch_interval_ns: u128,
    connections: u128,
}

impl Schedule {
    fn instant(&self, conn: usize, k: u64) -> Instant {
        let ns = self.batch_interval_ns * (conn as u128 + u128::from(k) * self.connections);
        self.t0 + Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    fn in_window(&self, at: Instant) -> bool {
        at >= self.window_start && at < self.window_end
    }
}

/// Builds one `OP_BATCH`; each op carries a connection-local numeric id so
/// the server can name it in `OP_REJECTED` and in a partial `OP_ACK`.
fn build_batch(
    conn_idx: usize,
    first_id: u64,
    batch_size: usize,
    key_mix: &KeyMix,
    ttl_ms: Option<u64>,
    rng: &mut StdRng,
) -> Vec<u8> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let ops = (0..batch_size as u64)
        .map(|i| {
            let id = first_id + i;
            let key = if rng.random::<f64>() < key_mix.new_key_fraction {
                format!("ol-{conn_idx}-{id}")
            } else {
                format!("hot-{}", rng.random_range(0..key_mix.hot_keys))
            };
            let mut fields = vec![(rmpv::Value::from("v"), rmpv::Value::from(id as i64))];
            if key_mix.value_bytes > 0 {
                fields.push((
                    rmpv::Value::from("bin"),
                    rmpv::Value::Binary(vec![0u8; key_mix.value_bytes]),
                ));
            }
            ClientOp {
                id: Some(id.to_string()),
                map_name: "bench".to_string(),
                key,
                op_type: Some("PUT".to_string()),
                record: Some(Some(LWWRecord {
                    value: Some(rmpv::Value::Map(fields)),
                    timestamp: Timestamp {
                        millis,
                        counter: id as u32,
                        node_id: format!("bench-{conn_idx}"),
                    },
                    ttl_ms,
                })),
                ..Default::default()
            }
        })
        .collect();
    let msg = Message::OpBatch(OpBatchMessage {
        payload: OpBatchPayload {
            ops,
            ..Default::default()
        },
    });
    rmp_serde::to_vec_named(&msg).expect("serialize OpBatch")
}

/// Counts the window batches of `conn` from schedule index `k` on, which will
/// never be sent.
fn remaining_window_batches(schedule: &Schedule, conn: usize, mut k: u64) -> u64 {
    let mut n = 0;
    loop {
        let at = schedule.instant(conn, k);
        if at >= schedule.window_end {
            return n;
        }
        if schedule.in_window(at) {
            n += 1;
        }
        k += 1;
    }
}

/// How often the shipped client pings (`SyncEngine` sends `PING` every 5 s).
/// The server's reaper closes a connection whose last `PING` is older than its
/// idle timeout, and nothing else refreshes it, so a generator that never pings
/// loses every connection once that timeout has passed.
pub const PING_INTERVAL: Duration = Duration::from_secs(5);

/// One connection's heartbeat: a task of its own that sends the client's
/// `PING` every [`PING_INTERVAL`], so a ping is never sent from the batch loop
/// and never shifts a batch's schedule. It shares the connection's sink lock
/// with the batch loop, so a batch send may wait for one in-flight `PING`
/// write. The task stops when the guard is dropped or a send fails.
pub struct Heartbeat(tokio::task::JoinHandle<()>);

impl Heartbeat {
    pub fn start(pool: Arc<ConnectionPool>, conn: usize) -> Self {
        Self(tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if pool.send_to(conn, &ping_frame()).await.is_err() {
                    return;
                }
            }
        }))
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The client's `PING`, stamped with the wall clock in milliseconds.
pub fn ping_frame() -> Vec<u8> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    rmp_serde::to_vec_named(&Message::Ping(PingData { timestamp })).expect("serialize PING")
}

/// Where a batch's ack latency starts: the instant from which the batch
/// waited because of the server. A batch reached before its scheduled instant
/// waited only for the generator's own timer (tokio rounds a sleep up to the
/// next millisecond), which belongs in the wake lag, not in the server's
/// figure, so it counts from its actual send. A batch reached after its
/// instant waited for the server's late previous answer, so it counts from
/// the scheduled instant (coordinated-omission correction); lateness caused by
/// a starved generator is then charged to the server, the conservative side.
fn latency_origin(reached_early: bool, scheduled: Instant, sent_at: Instant) -> Instant {
    if reached_early {
        sent_at
    } else {
        scheduled
    }
}

/// Drives one connection through the schedule.
#[allow(clippy::too_many_lines)]
async fn drive_connection(
    pool: Arc<ConnectionPool>,
    conn: usize,
    schedule: Schedule,
    config: Arc<OpenLoopConfig>,
    live: Arc<LiveCounters>,
) -> ConnTally {
    let batch = config.batch_size as u64;
    let ack_timeout = Duration::from_millis(config.ack_timeout_ms);
    let mut rng = StdRng::seed_from_u64(config.key_mix.seed ^ conn as u64);
    let mut tally = ConnTally::new();
    let mut next_id: u64 = 1;
    let mut k: u64 = 0;
    let _heartbeat = Heartbeat::start(Arc::clone(&pool), conn);

    loop {
        let scheduled = schedule.instant(conn, k);
        if scheduled >= schedule.window_end {
            break;
        }
        let in_window = schedule.in_window(scheduled);
        let now = Instant::now();
        if now >= schedule.stop {
            let unsent = remaining_window_batches(&schedule, conn, k);
            tally.scheduled += unsent * batch;
            break;
        }
        let reached_early = now < scheduled;
        if reached_early {
            tokio::time::sleep_until(scheduled).await;
            if in_window {
                tally
                    .wake_lag
                    .saturating_record(micros(Instant::now() - scheduled).max(1));
            }
        } else if in_window {
            let behind = micros(now - scheduled);
            tally.behind_schedule_sends += 1;
            tally.max_behind_us = tally.max_behind_us.max(behind);
        }

        let first_id = next_id;
        next_id += batch;
        let bytes = build_batch(
            conn,
            first_id,
            config.batch_size,
            &config.key_mix,
            config.ttl_ms,
            &mut rng,
        );
        let sent_at = Instant::now();
        if pool.send_to(conn, &bytes).await.is_err() {
            // Not delivered: this batch and every later one of the window
            // never reached the server.
            tally.abandoned = 1;
            tally.scheduled += remaining_window_batches(&schedule, conn, k) * batch;
            break;
        }

        let mut answer = BatchAnswer::new(first_id, config.batch_size);
        let deadline = sent_at + ack_timeout;
        let terminal = loop {
            match tokio::time::timeout_at(deadline, pool.recv_from(conn)).await {
                Ok(Ok(bytes)) => {
                    if answer.apply(Frame::decode(&bytes)) == Step::Terminal {
                        break true;
                    }
                }
                Ok(Err(_)) | Err(_) => break false,
            }
        };
        let answered_at = Instant::now();
        let counts = answer.counts();
        live.acked.fetch_add(counts.acked, Ordering::Relaxed);
        live.refused
            .fetch_add(counts.permanent() + counts.transient(), Ordering::Relaxed);

        if in_window {
            tally.scheduled += batch;
            tally.sent += batch;
            tally.anomalies += answer.anomalies;
            if terminal {
                tally.ack.saturating_record(
                    micros(answered_at - latency_origin(reached_early, scheduled, sent_at)).max(1),
                );
                tally
                    .service
                    .saturating_record(micros(answered_at - sent_at).max(1));
            }
            if let Some((code, message)) = answer.error_message.take() {
                let kept = tally.samples.entry(code).or_default();
                if kept.len() < SAMPLES_PER_CODE && !kept.contains(&message) {
                    kept.push(message);
                }
            }
            tally.counts.add(counts);
        }

        k += 1;
        if !terminal {
            // A late answer could be read as the answer to the next batch, so
            // this connection carries no further batch.
            tally.abandoned = 1;
            tally.scheduled += remaining_window_batches(&schedule, conn, k) * batch;
            break;
        }
    }
    tally
}

/// One timeline sample.
#[derive(Debug, Clone, Serialize)]
pub struct TimelineSample {
    pub t_ms: u64,
    pub acked_ops: u64,
    pub refused_ops: u64,
    pub depth: u64,
    pub wal_bytes: Option<u64>,
    pub store_bytes: Option<u64>,
}

async fn sample_timeline(
    t0: Instant,
    config: Arc<OpenLoopConfig>,
    live: Arc<LiveCounters>,
    datastore: Arc<dyn topgun_server::storage::map_data_store::MapDataStore>,
    dirs: Option<RunDirs>,
) -> Vec<TimelineSample> {
    let ticks = (config.warmup_secs + config.duration_secs) * 1000 / config.sample_ms;
    let mut samples = Vec::with_capacity(ticks as usize);
    let (mut last_acked, mut last_refused) = (0, 0);
    for k in 1..=ticks {
        let t_ms = k * config.sample_ms;
        tokio::time::sleep_until(t0 + Duration::from_millis(t_ms)).await;
        let acked = live.acked.load(Ordering::Relaxed);
        let refused = live.refused.load(Ordering::Relaxed);
        samples.push(TimelineSample {
            t_ms,
            acked_ops: acked - last_acked,
            refused_ops: refused - last_refused,
            depth: datastore.pending_operation_count(),
            wal_bytes: dirs.as_ref().map(|d| dir_bytes(&d.wal_dir)),
            store_bytes: dirs.as_ref().map(|d| dir_bytes(&d.data_dir)),
        });
        (last_acked, last_refused) = (acked, refused);
    }
    samples
}

/// Everything the generator measured, before the report is assembled.
pub struct Measurement {
    tally: ConnTally,
    timeline: Vec<TimelineSample>,
    store_failure: Option<String>,
}

/// Runs the open loop against an already started server.
pub async fn run(
    config: &OpenLoopConfig,
    server: &ServerHandle,
    dirs: Option<RunDirs>,
    jwt_secret: &str,
) -> anyhow::Result<Measurement> {
    let pool = Arc::new(ConnectionPool::new(server.addr, config.connections, jwt_secret).await?);
    let config = Arc::new(config.clone());
    let live = Arc::new(LiveCounters::default());

    // A short lead so every task is parked before the first instant.
    let t0 = Instant::now() + Duration::from_millis(100);
    let window_start = t0 + Duration::from_secs(config.warmup_secs);
    let window_end = window_start + Duration::from_secs(config.duration_secs);
    let schedule = Schedule {
        t0,
        window_start,
        window_end,
        stop: window_end + Duration::from_millis(config.ack_timeout_ms),
        batch_interval_ns: config.batch_size as u128 * 1_000_000_000 / u128::from(config.rate),
        connections: config.connections as u128,
    };

    let sampler = tokio::spawn(sample_timeline(
        t0,
        Arc::clone(&config),
        Arc::clone(&live),
        Arc::clone(&server.datastore),
        dirs,
    ));
    let mut tasks = tokio::task::JoinSet::new();
    for conn in 0..config.connections {
        tasks.spawn(drive_connection(
            Arc::clone(&pool),
            conn,
            schedule,
            Arc::clone(&config),
            Arc::clone(&live),
        ));
    }
    let mut tally = ConnTally::new();
    while let Some(joined) = tasks.join_next().await {
        tally.absorb(joined?);
    }
    let timeline = sampler.await?;
    drop(pool);

    let store_failure = server
        .failure
        .as_ref()
        .and_then(|rx| rx.borrow().as_ref().map(|f| format!("{f:?}")));
    Ok(Measurement {
        tally,
        timeline,
        store_failure,
    })
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Achieved {
    pub acked_ops_per_sec: f64,
    pub sent_ops_per_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ops {
    pub scheduled: u64,
    pub sent: u64,
    pub unsent: u64,
    pub acked: u64,
    pub refused_permanent: u64,
    pub refused_transient: u64,
    pub unanswered: u64,
}

impl Ops {
    /// The two identities every run must satisfy; a violation is a bug of
    /// the harness, not a result.
    pub fn check(&self) -> Result<(), String> {
        let answered =
            self.acked + self.refused_permanent + self.refused_transient + self.unanswered;
        if self.scheduled == self.sent + self.unsent && self.sent == answered {
            Ok(())
        } else {
            Err(format!("op accounting identity violated: {self:?}"))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Refusals {
    /// Ops answered by `ERROR` and not acknowledged, by the frame's code;
    /// not necessarily unapplied.
    pub transient_by_code: BTreeMap<String, u64>,
    pub permanent_by_code: BTreeMap<String, u64>,
    pub sample_messages: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub count: u64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
    pub mean: f64,
}

impl LatencySummary {
    fn of(h: &Histogram<u64>) -> Self {
        Self {
            count: h.len(),
            p50: h.value_at_quantile(0.50),
            p90: h.value_at_quantile(0.90),
            p99: h.value_at_quantile(0.99),
            p999: h.value_at_quantile(0.999),
            max: h.max(),
            mean: h.mean(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WakeLag {
    pub count: u64,
    pub p50: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Generator {
    pub wake_lag_us: WakeLag,
    pub behind_schedule_sends: u64,
    pub max_behind_us: u64,
    pub connections_abandoned: u64,
    pub protocol_anomalies: u64,
    pub max_wake_lag_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteBehindSummary {
    pub depth_max: u64,
    pub depth_final: u64,
    pub sample_ms: u64,
    pub warmup_end_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Host {
    pub os: &'static str,
    pub arch: &'static str,
    pub cpus: usize,
    pub kernel: Option<String>,
    pub cpu_model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Build {
    pub git_commit: Option<String>,
    pub git_dirty: Option<bool>,
    pub binary_sha256: Option<String>,
    pub debug_assertions: bool,
    pub allocator: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Validity {
    pub valid: bool,
    pub reasons: Vec<&'static str>,
    pub store_failure: Option<String>,
}

/// One open-loop run, `schema_version` 1.
#[derive(Debug, Clone, Serialize)]
pub struct OpenLoopReport {
    pub schema_version: u32,
    pub scenario: &'static str,
    pub store: String,
    pub run_index: Option<u64>,
    pub timestamp: String,
    pub target_ops_per_sec: u64,
    pub batch_size: usize,
    pub connections: usize,
    pub warmup_secs: u64,
    pub duration_secs: u64,
    pub key_mix: KeyMix,
    pub achieved: Achieved,
    pub ops: Ops,
    pub refusals: Refusals,
    pub ack_latency_us: LatencySummary,
    pub service_latency_us: LatencySummary,
    pub generator: Generator,
    pub write_behind: WriteBehindSummary,
    pub timeline: Vec<TimelineSample>,
    pub config: Option<EffectiveStoreConfig>,
    pub journal_enabled: bool,
    pub expiry_sweep_interval_ms: u64,
    pub expiry_sweep_rows_per_sec: u64,
    pub host: Host,
    pub build: Build,
    pub teardown: Teardown,
    pub validity: Validity,
}

/// What the report needs besides the measurement.
pub struct ReportContext {
    pub store: &'static str,
    pub run_index: Option<u64>,
    pub timestamp: String,
    pub config: Option<EffectiveStoreConfig>,
    pub journal_enabled: bool,
    pub expiry_sweep_interval_ms: u64,
    pub expiry_sweep_rows_per_sec: u64,
}

/// The fixed validity reasons. None of them judges the server: each says the
/// measurement itself cannot be read as "the server at rate R".
fn validity(m: &Measurement, ops: &Ops, max_wake_lag_ms: u64) -> Validity {
    let mut reasons = Vec::new();
    if m.tally.wake_lag.value_at_quantile(0.999) > max_wake_lag_ms.saturating_mul(1000) {
        reasons.push("generator_saturated");
    }
    if ops.unsent > 0 {
        reasons.push("load_not_offered");
    }
    if ops.unanswered > 0 {
        reasons.push("unanswered_batches");
    }
    if m.store_failure.is_some() {
        reasons.push("store_failed");
    }
    if cfg!(debug_assertions) {
        reasons.push("debug_build");
    }
    Validity {
        valid: reasons.is_empty(),
        reasons,
        store_failure: m.store_failure.clone(),
    }
}

/// Assembles the run's report; fails only on an accounting identity
/// violation. `teardown` is all-null here and filled in after the drain.
pub fn report(
    m: &Measurement,
    config: &OpenLoopConfig,
    ctx: ReportContext,
) -> Result<OpenLoopReport, String> {
    let t = &m.tally;
    let ops = Ops {
        scheduled: t.scheduled,
        sent: t.sent,
        unsent: t.scheduled.saturating_sub(t.sent),
        acked: t.counts.acked,
        refused_permanent: t.counts.permanent(),
        refused_transient: t.counts.transient(),
        unanswered: t.counts.unanswered,
    };
    ops.check()?;
    let duration = config.duration_secs as f64;
    Ok(OpenLoopReport {
        schema_version: 1,
        scenario: "open_loop",
        store: ctx.store.to_string(),
        run_index: ctx.run_index,
        timestamp: ctx.timestamp,
        target_ops_per_sec: config.rate,
        batch_size: config.batch_size,
        connections: config.connections,
        warmup_secs: config.warmup_secs,
        duration_secs: config.duration_secs,
        key_mix: config.key_mix.clone(),
        achieved: Achieved {
            acked_ops_per_sec: ops.acked as f64 / duration,
            sent_ops_per_sec: ops.sent as f64 / duration,
        },
        refusals: Refusals {
            transient_by_code: t.counts.refused_transient.clone(),
            permanent_by_code: t.counts.refused_permanent.clone(),
            sample_messages: t.samples.clone(),
        },
        ack_latency_us: LatencySummary::of(&t.ack),
        service_latency_us: LatencySummary::of(&t.service),
        generator: Generator {
            wake_lag_us: WakeLag {
                count: t.wake_lag.len(),
                p50: t.wake_lag.value_at_quantile(0.50),
                p99: t.wake_lag.value_at_quantile(0.99),
                p999: t.wake_lag.value_at_quantile(0.999),
                max: t.wake_lag.max(),
            },
            behind_schedule_sends: t.behind_schedule_sends,
            max_behind_us: t.max_behind_us,
            connections_abandoned: t.abandoned,
            protocol_anomalies: t.anomalies,
            max_wake_lag_ms: config.max_wake_lag_ms,
        },
        write_behind: WriteBehindSummary {
            depth_max: m.timeline.iter().map(|s| s.depth).max().unwrap_or(0),
            depth_final: m.timeline.last().map_or(0, |s| s.depth),
            sample_ms: config.sample_ms,
            warmup_end_ms: config.warmup_secs * 1000,
        },
        timeline: m.timeline.clone(),
        config: ctx.config,
        journal_enabled: ctx.journal_enabled,
        expiry_sweep_interval_ms: ctx.expiry_sweep_interval_ms,
        expiry_sweep_rows_per_sec: ctx.expiry_sweep_rows_per_sec,
        host: host_info(),
        build: build_info(),
        teardown: Teardown::default(),
        validity: validity(m, &ops, config.max_wake_lag_ms),
        ops,
    })
}

impl OpenLoopReport {
    /// The short human summary on stdout.
    pub fn print_summary(&self) {
        let a = &self.ack_latency_us;
        println!("store: {}", self.store);
        println!(
            "target: {} ops/s; achieved: {:.0} acked ops/s, {:.0} sent ops/s",
            self.target_ops_per_sec,
            self.achieved.acked_ops_per_sec,
            self.achieved.sent_ops_per_sec
        );
        println!(
            "ack latency (CO-corrected) us: p50 {} p99 {} p99.9 {} max {}",
            a.p50, a.p99, a.p999, a.max
        );
        println!(
            "refused: {} permanent, {} transient; unanswered {}; unsent {}",
            self.ops.refused_permanent,
            self.ops.refused_transient,
            self.ops.unanswered,
            self.ops.unsent
        );
        println!("write-behind depth max: {}", self.write_behind.depth_max);
        if self.validity.valid {
            println!("validity: valid");
        } else {
            println!("validity: INVALID ({})", self.validity.reasons.join(", "));
        }
    }
}

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn host_info() -> Host {
    let cpu_model = if cfg!(target_os = "macos") {
        command_line("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|info| {
                info.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split_once(':'))
                    .map(|(_, v)| v.trim().to_string())
            })
    };
    Host {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        cpus: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        kernel: command_line("uname", &["-r"]).filter(|k| !k.is_empty()),
        cpu_model: cpu_model.filter(|m| !m.is_empty()),
    }
}

/// The SHA-256 of the running binary: release builds are not byte
/// reproducible, so the launched binary's hash is the provenance that counts.
pub fn binary_sha256() -> Option<String> {
    let bytes = std::fs::read(std::env::current_exe().ok()?).ok()?;
    Some(hex::encode(Sha256::digest(bytes)))
}

/// The allocator the harness serves from, by the same precedence as its
/// `#[global_allocator]` lattice.
fn allocator() -> &'static str {
    if cfg!(feature = "count-alloc") {
        "count-alloc"
    } else if cfg!(feature = "dhat-heap") {
        "dhat-heap"
    } else if cfg!(feature = "alloc-jemalloc") {
        "jemalloc"
    } else if cfg!(feature = "alloc-mimalloc") {
        "mimalloc"
    } else {
        "system"
    }
}

fn build_info() -> Build {
    Build {
        git_commit: command_line("git", &["rev-parse", "HEAD"]),
        git_dirty: command_line("git", &["status", "--porcelain"]).map(|s| !s.is_empty()),
        binary_sha256: binary_sha256(),
        debug_assertions: cfg!(debug_assertions),
        allocator: allocator(),
    }
}

// ---------------------------------------------------------------------------
// Series (A/A, ABAB)
// ---------------------------------------------------------------------------

/// The spread of one figure over an arm's runs.
#[derive(Debug, Clone, Serialize)]
pub struct FigureSpread {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    /// `(max - min) / mean`; `null` with fewer than two runs or a zero mean.
    pub relative_spread: Option<f64>,
}

impl FigureSpread {
    fn of(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                min: None,
                max: None,
                mean: None,
                relative_spread: None,
            };
        }
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        Self {
            min: Some(min),
            max: Some(max),
            mean: Some(mean),
            relative_spread: (values.len() >= 2 && mean > 0.0).then(|| (max - min) / mean),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArmSpread {
    pub runs: usize,
    pub acked_ops_per_sec: FigureSpread,
    pub ack_p50_us: FigureSpread,
    pub ack_p99_us: FigureSpread,
    pub ack_p999_us: FigureSpread,
    pub ack_max_us: FigureSpread,
    pub depth_max: FigureSpread,
}

/// The figures of each arm's runs that produced a report.
pub fn spread(runs: &[serde_json::Value], arms: &[String]) -> BTreeMap<String, ArmSpread> {
    let figure = |reports: &[&serde_json::Value], path: &[&str]| -> FigureSpread {
        let values: Vec<f64> = reports
            .iter()
            .filter_map(|r| path.iter().try_fold(*r, |v, k| v.get(k))?.as_f64())
            .collect();
        FigureSpread::of(&values)
    };
    arms.iter()
        .map(|arm| {
            let reports: Vec<&serde_json::Value> = runs
                .iter()
                .filter(|r| r["store"] == arm.as_str() && r.get("error").is_none())
                .collect();
            let spread = ArmSpread {
                runs: reports.len(),
                acked_ops_per_sec: figure(&reports, &["achieved", "acked_ops_per_sec"]),
                ack_p50_us: figure(&reports, &["ack_latency_us", "p50"]),
                ack_p99_us: figure(&reports, &["ack_latency_us", "p99"]),
                ack_p999_us: figure(&reports, &["ack_latency_us", "p999"]),
                ack_max_us: figure(&reports, &["ack_latency_us", "max"]),
                depth_max: figure(&reports, &["write_behind", "depth_max"]),
            };
            (arm.clone(), spread)
        })
        .collect()
}

/// The series report, `schema_version` 1.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesReport {
    pub schema_version: u32,
    pub kind: &'static str,
    pub order: Vec<String>,
    /// A run's report, or `{store, run_index, error}` for a run that failed.
    pub runs: Vec<serde_json::Value>,
    pub spread: BTreeMap<String, ArmSpread>,
    pub valid: bool,
}

impl SeriesReport {
    pub fn new(order: Vec<String>, runs: Vec<serde_json::Value>, arms: &[String]) -> Self {
        let valid = runs
            .iter()
            .all(|r| r.get("error").is_none() && r["validity"]["valid"] == true);
        Self {
            schema_version: 1,
            kind: "open_loop_series",
            spread: spread(&runs, arms),
            order,
            runs,
            valid,
        }
    }

    pub fn print_table(&self) {
        let show = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |x| format!("{x:.0}"));
        let rel =
            |v: Option<f64>| v.map_or_else(|| "-".to_string(), |x| format!("{:.1}%", x * 100.0));
        println!(
            "{:<6} {:>4}  {:>28}  {:>28}  {:>28}",
            "arm",
            "runs",
            "acked ops/s min/max/rel",
            "ack p99 us min/max/rel",
            "ack p99.9 us min/max/rel"
        );
        for (arm, s) in &self.spread {
            let cell = |x: &FigureSpread| {
                format!("{}/{}/{}", show(x.min), show(x.max), rel(x.relative_spread))
            };
            println!(
                "{arm:<6} {:>4}  {:>28}  {:>28}  {:>28}",
                s.runs,
                cell(&s.acked_ops_per_sec),
                cell(&s.ack_p99_us),
                cell(&s.ack_p999_us)
            );
        }
        println!("series valid: {}", self.valid);
    }
}

// ---------------------------------------------------------------------------
// Self-test
// ---------------------------------------------------------------------------

/// A batch of five ops whose first two are refused by `OP_REJECTED` and which
/// is then answered by `ERROR 500`: each op is counted exactly once — the
/// rejected ones as permanent, the rest as transient — and none is acked.
pub fn classify_partial_reject_then_error_counts_each_op_once() {
    const B: usize = 5;
    let mut answer = BatchAnswer::new(41, B);
    for id in ["41", "42"] {
        let step = answer.apply(Frame::Rejected {
            op_id: id.to_string(),
            code: Some(403),
        });
        assert_eq!(step, Step::Continue, "a partial rejection is not terminal");
    }
    let step = answer.apply(Frame::Error {
        code: 500,
        message: "Write-behind capacity exceeded".to_string(),
    });
    assert_eq!(step, Step::Terminal);

    let counts = answer.counts();
    assert_eq!(counts.refused_permanent.get("403"), Some(&2));
    assert_eq!(counts.permanent(), 2);
    assert_eq!(counts.refused_transient.get("500"), Some(&3));
    assert_eq!(counts.transient(), 3);
    assert_eq!(counts.acked, 0);
    assert_eq!(counts.unanswered, 0);
    let sent = B as u64;
    assert_eq!(
        sent,
        counts.acked + counts.permanent() + counts.transient() + counts.unanswered
    );
    assert_eq!(answer.anomalies, 0);
}

/// A batch whose every op is refused ends at the last `OP_REJECTED`; a
/// partial `OP_ACK` acks exactly the ops it lists.
pub fn classify_full_rejection_and_partial_ack() {
    let mut all = BatchAnswer::new(1, 2);
    let reject = |id: &str| Frame::Rejected {
        op_id: id.to_string(),
        code: Some(400),
    };
    assert_eq!(all.apply(reject("1")), Step::Continue);
    assert_eq!(all.apply(reject("2")), Step::Terminal);
    assert_eq!(all.counts().permanent(), 2);

    let mut partial = BatchAnswer::new(10, 3);
    assert_eq!(partial.apply(reject("11")), Step::Continue);
    let ack = Frame::Ack {
        results: Some(vec!["10".to_string(), "12".to_string()]),
    };
    assert_eq!(partial.apply(ack), Step::Terminal);
    let counts = partial.counts();
    assert_eq!(
        (counts.acked, counts.permanent(), counts.unanswered),
        (2, 1, 0)
    );
    assert_eq!(partial.anomalies, 0);

    let mut stray = BatchAnswer::new(10, 1);
    assert_eq!(stray.apply(reject("99")), Step::Continue);
    assert_eq!(stray.anomalies, 1);
}

/// A heartbeat `PONG` between a batch's send and its `OP_ACK` answers nothing
/// and is no anomaly: the batch is settled by the `OP_ACK`, every op once.
pub fn classify_pong_is_neither_answer_nor_anomaly() {
    let pong = rmp_serde::to_vec_named(&Message::Pong(topgun_core::messages::PongData {
        timestamp: 1,
        server_time: 2,
    }))
    .expect("serialize PONG");
    let mut answer = BatchAnswer::new(1, 3);
    assert_eq!(answer.apply(Frame::decode(&pong)), Step::Continue);
    assert_eq!(answer.counts().unanswered, 3, "a PONG settles no op");
    assert_eq!(answer.apply(Frame::Ack { results: None }), Step::Terminal);
    let counts = answer.counts();
    assert_eq!(
        (
            counts.acked,
            counts.permanent(),
            counts.transient(),
            counts.unanswered
        ),
        (3, 0, 0, 0)
    );
    assert_eq!(answer.anomalies, 0);
}

/// A batch reached before its instant counts from its actual send, so the
/// generator's timer lag stays out of the ack latency; a batch reached after
/// its instant counts from the scheduled instant.
pub fn latency_origin_early_from_send_late_from_schedule() {
    let scheduled = Instant::now();
    let sent_at = scheduled + Duration::from_micros(900);
    assert_eq!(latency_origin(true, scheduled, sent_at), sent_at);
    assert_eq!(latency_origin(false, scheduled, sent_at), scheduled);
}

/// Runs every check, each isolated so that one failing check does not hide the
/// others, and prints a libtest-shaped summary. Returns the exit code.
pub fn run_self_tests() -> i32 {
    let checks: [(&str, fn()); 4] = [
        (
            "classify_partial_reject_then_error_counts_each_op_once",
            classify_partial_reject_then_error_counts_each_op_once,
        ),
        (
            "classify_full_rejection_and_partial_ack",
            classify_full_rejection_and_partial_ack,
        ),
        (
            "classify_pong_is_neither_answer_nor_anomaly",
            classify_pong_is_neither_answer_nor_anomaly,
        ),
        (
            "latency_origin_early_from_send_late_from_schedule",
            latency_origin_early_from_send_late_from_schedule,
        ),
    ];
    let mut failed = Vec::new();
    for (name, check) in checks {
        let ok = std::panic::catch_unwind(check).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            failed.push(name);
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        checks.len() - failed.len(),
        failed.len()
    );
    i32::from(!failed.is_empty())
}
