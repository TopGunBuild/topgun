//! The subscriber arm of the open loop: reader connections holding live
//! queries on the written map, what they receive, and a check of what they
//! hold against a fresh query once the writes have stopped.
//!
//! Each reader connection has a receive task, which only reads frames off the
//! socket and stamps their arrival, and a processing task, which decodes them
//! and applies them to the reader's view. Delivery latency is taken at the
//! arrival stamp, so the harness's own decoding is not charged to the server;
//! a processing task that falls behind shows up as unread frames
//! (`reader_behind`), not as a slower server. A slow reader is throttled in its
//! receive task instead, so the server meets a socket that drains slowly.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use hdrhistogram::Histogram;
use serde::Serialize;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use topgun_core::messages::base::SortField;
use topgun_core::messages::{
    ChangeEventType, Message, PredicateNode, PredicateOp, Query, QueryRespPayload, QuerySubMessage,
    QuerySubPayload, SortDirection,
};

use crate::connection_pool::ConnectionPool;
use crate::open_loop::Heartbeat;

/// The map every writer writes and every reader subscribes to.
pub const MAP: &str = "bench";

/// A reader with more unread frames than this measures its own lag, not the
/// server's (`reader_behind`).
pub const READER_BEHIND_FRAMES: i64 = 100_000;

/// The longest wait for the next snapshot or fresh result, and for quiesce,
/// before the run is given up or marked. Snapshots are waited for as long as
/// they keep coming: 10^4 live subscribes take minutes on one node.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(60);
const QUIESCE_CAP: Duration = Duration::from_secs(300);

/// Live subscribes a reader keeps unanswered at once, and how long it waits
/// for one's snapshot before sending it again. The server answers a
/// subscribe it sheds (worker inbox full) with an `ERROR 429` that names no
/// query, and one that times out in the pipeline with nothing, so an
/// unbounded burst loses subscribes for good.
const SUBSCRIBE_WINDOW: usize = 8;
const RESUBSCRIBE_AFTER: Duration = Duration::from_secs(10);

/// Connections and in-flight one-shot queries of the fresh-query check.
const CHECK_CONNECTIONS: usize = 4;
const CHECK_IN_FLIGHT: usize = 16;

const HIST_MAX_US: u64 = 3_600_000_000;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Which rows a subscription asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SubPredicate {
    None,
    Eq,
    Range,
}

impl std::str::FromStr for SubPredicate {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "none" => Ok(Self::None),
            "eq" => Ok(Self::Eq),
            "range" => Ok(Self::Range),
            other => Err(format!(
                "--sub-predicate {other}: expected none, eq or range"
            )),
        }
    }
}

/// `--sub-sort field:asc|desc`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubSort {
    pub field: String,
    pub descending: bool,
}

impl std::str::FromStr for SubSort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (field, dir) = s
            .split_once(':')
            .ok_or_else(|| format!("--sub-sort {s}: expected field:asc or field:desc"))?;
        let descending = match dir {
            "asc" => false,
            "desc" => true,
            _ => return Err(format!("--sub-sort {s}: expected field:asc or field:desc")),
        };
        if field.is_empty() {
            return Err(format!("--sub-sort {s}: the field is empty"));
        }
        Ok(Self {
            field: field.to_string(),
            descending,
        })
    }
}

/// The subscriber arm's flags. `readers` is `None` when the arm is off.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriberConfig {
    pub readers: Option<usize>,
    pub buckets: u64,
    pub subs_per_reader: usize,
    pub predicate: SubPredicate,
    pub sort: Option<SubSort>,
    pub limit: Option<u32>,
    pub slow_readers: usize,
    /// Subscriptions per slow reader; `subs_per_reader` when not given.
    pub slow_reader_subs: Option<usize>,
    pub slow_reader_frame_us: u64,
    pub quiesce_ms: u64,
}

impl Default for SubscriberConfig {
    fn default() -> Self {
        Self {
            readers: None,
            buckets: 100,
            subs_per_reader: 1,
            predicate: SubPredicate::None,
            sort: None,
            limit: None,
            slow_readers: 0,
            slow_reader_subs: None,
            slow_reader_frame_us: 2000,
            quiesce_ms: 2000,
        }
    }
}

impl SubscriberConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.buckets == 0 {
            return Err("--sub-buckets must be greater than 0".into());
        }
        if self.subs_per_reader == 0 {
            return Err("--subs-per-reader must be greater than 0".into());
        }
        if self.slow_reader_subs == Some(0) {
            return Err("--slow-reader-subs must be greater than 0".into());
        }
        if self.limit == Some(0) {
            return Err("--sub-limit must be greater than 0".into());
        }
        if self.readers.is_none() && self.slow_readers > 0 {
            return Err("--slow-readers needs --subscribers".into());
        }
        Ok(())
    }

    fn slow_subs(&self) -> usize {
        self.slow_reader_subs.unwrap_or(self.subs_per_reader)
    }

    /// Subscription `i`'s query.
    pub fn query(&self, i: u64) -> Query {
        let b = self.buckets;
        let attr = |op, v: u64| PredicateNode {
            op,
            attribute: Some("bucket".into()),
            value: Some(rmpv::Value::from(v)),
            ..Default::default()
        };
        let predicate = match self.predicate {
            SubPredicate::None => None,
            SubPredicate::Eq => Some(attr(PredicateOp::Eq, i % b)),
            SubPredicate::Range => {
                let width = b.div_ceil(10);
                let x = (i * width) % b;
                Some(PredicateNode {
                    op: PredicateOp::And,
                    children: Some(vec![
                        attr(PredicateOp::Gte, x),
                        attr(PredicateOp::Lt, x + width),
                    ]),
                    ..Default::default()
                })
            }
        };
        Query {
            predicate,
            sort: self.sort.as_ref().map(|s| {
                vec![SortField {
                    field: s.field.clone(),
                    direction: if s.descending {
                        SortDirection::Desc
                    } else {
                        SortDirection::Asc
                    },
                }]
            }),
            limit: self.limit,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// The `MsgPack` prefix of a `CHUNK` (`TG-WS-002`): a three-entry map whose
/// first entry is `type: "CHUNK"`.
const CHUNK_PREFIX: &[u8] = b"\x83\xa4type\xa5CHUNK";

/// Reassembles the `CHUNK`s of a chunk-capable connection into messages.
#[derive(Default)]
pub struct Reassembly {
    buf: Vec<u8>,
}

impl Reassembly {
    /// A whole message, or `None` while a chunked one is incomplete.
    pub fn push(&mut self, bytes: Vec<u8>) -> Result<Option<Vec<u8>>> {
        if !bytes.starts_with(CHUNK_PREFIX) {
            return Ok(Some(bytes));
        }
        let value = rmpv::decode::read_value(&mut bytes.as_slice())
            .map_err(|e| anyhow!("undecodable CHUNK: {e}"))?;
        let field = |name: &str| {
            value
                .as_map()
                .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some(name)))
                .map(|(_, v)| v)
        };
        let last = field("last").and_then(rmpv::Value::as_bool);
        let data = field("data").and_then(rmpv::Value::as_slice);
        let (Some(last), Some(data)) = (last, data) else {
            bail!("CHUNK without last or data");
        };
        self.buf.extend_from_slice(data);
        Ok(last.then(|| std::mem::take(&mut self.buf)))
    }
}

/// One server message, reduced to what a reader reads.
#[derive(Debug)]
pub enum ReaderFrame {
    Update {
        query_id: String,
        key: String,
        value: rmpv::Value,
        change: ChangeEventType,
    },
    Resp(QueryRespPayload),
    ServerEvent,
    Pong,
    Other,
}

impl ReaderFrame {
    pub fn decode(bytes: &[u8]) -> Self {
        match rmp_serde::from_slice::<Message>(bytes) {
            Ok(Message::QueryUpdate { payload }) => Self::Update {
                query_id: payload.query_id,
                key: payload.key,
                value: payload.value,
                change: payload.change_type,
            },
            Ok(Message::QueryResp(m)) => Self::Resp(m.payload),
            Ok(Message::ServerEvent { .. }) => Self::ServerEvent,
            Ok(Message::Pong(_)) => Self::Pong,
            _ => Self::Other,
        }
    }
}

/// The `t` field of a written value: its scheduled instant in µs since t0.
pub fn value_t(value: &rmpv::Value) -> Option<u64> {
    value
        .as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some("t"))?
        .1
        .as_u64()
}

// ---------------------------------------------------------------------------
// One reader's state
// ---------------------------------------------------------------------------

/// The instants a reader measures against, fixed once every snapshot is in.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub t0: Instant,
    pub window_start: Instant,
    pub window_end: Instant,
}

/// What one reader holds and counted.
pub struct ReaderState {
    pub slow: bool,
    /// Query id → (key → value).
    pub views: HashMap<String, HashMap<String, rmpv::Value>>,
    /// Query ids whose first `QUERY_RESP` arrived.
    snapshotted: HashSet<String>,
    pub deliveries: u64,
    pub map_events: u64,
    pub frames: u64,
    pub resnapshots: u64,
    pub query_errors: u64,
    pub anomalies: u64,
    pub latency: Histogram<u64>,
    pub disconnected: bool,
}

impl ReaderState {
    pub fn new(slow: bool, query_ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            slow,
            views: query_ids.into_iter().map(|q| (q, HashMap::new())).collect(),
            snapshotted: HashSet::new(),
            deliveries: 0,
            map_events: 0,
            frames: 0,
            resnapshots: 0,
            query_errors: 0,
            anomalies: 0,
            latency: Histogram::new_with_bounds(1, HIST_MAX_US, 3).expect("valid bounds"),
            disconnected: false,
        }
    }

    /// Applies one message received at `at`. Returns whether it was the first
    /// snapshot of one of this reader's subscriptions. Counts and latency are
    /// taken only for frames that arrived inside the measurement window.
    pub fn apply(&mut self, frame: ReaderFrame, at: Instant, timing: Option<&Timing>) -> bool {
        let in_window = timing.is_some_and(|t| at >= t.window_start && at < t.window_end);
        match frame {
            ReaderFrame::Update {
                query_id,
                key,
                value,
                change,
            } => {
                let Some(view) = self.views.get_mut(&query_id) else {
                    self.anomalies += 1;
                    return false;
                };
                if in_window {
                    self.deliveries += 1;
                    if let (Some(timing), Some(t)) = (timing, value_t(&value)) {
                        self.latency
                            .saturating_record(delivery_latency_us(timing.t0, at, t).max(1));
                    }
                }
                match change {
                    ChangeEventType::LEAVE => {
                        view.remove(&key);
                    }
                    ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                        view.insert(key, value);
                    }
                }
                false
            }
            ReaderFrame::Resp(resp) => {
                let Some(view) = self.views.get_mut(&resp.query_id) else {
                    self.anomalies += 1;
                    return false;
                };
                if resp.error.is_some() || resp.code.is_some() {
                    self.query_errors += 1;
                    return false;
                }
                *view = resp.results.into_iter().map(|e| (e.key, e.value)).collect();
                if self.snapshotted.insert(resp.query_id) {
                    true
                } else {
                    // Before the writes start a second answer is the reply to
                    // a subscribe sent again, not a re-snapshot.
                    if timing.is_some() {
                        self.resnapshots += 1;
                    }
                    false
                }
            }
            ReaderFrame::ServerEvent => {
                if in_window {
                    self.map_events += 1;
                }
                false
            }
            ReaderFrame::Pong => false,
            ReaderFrame::Other => {
                self.anomalies += 1;
                false
            }
        }
    }
}

/// A delivery's latency: from the write's scheduled instant (`t`, µs since
/// `t0`) to the frame's arrival at the reader.
pub fn delivery_latency_us(t0: Instant, received_at: Instant, t: u64) -> u64 {
    let since_t0 =
        u64::try_from(received_at.saturating_duration_since(t0).as_micros()).unwrap_or(u64::MAX);
    since_t0.saturating_sub(t)
}

// ---------------------------------------------------------------------------
// The reader set
// ---------------------------------------------------------------------------

struct Shared {
    timing: OnceLock<Timing>,
    snapshots: AtomicU64,
    snapshot_notify: Notify,
    /// Nanoseconds since `epoch` of the last non-`PONG` frame any reader got.
    last_frame_ns: AtomicU64,
    epoch: Instant,
}

struct Reader {
    recv: JoinHandle<()>,
    process: JoinHandle<ReaderState>,
    backlog: Arc<AtomicI64>,
    max_backlog: Arc<AtomicI64>,
    _heartbeat: Heartbeat,
}

/// The running readers of one run.
pub struct ReaderSet {
    readers: Vec<Reader>,
    shared: Arc<Shared>,
    subscriptions: u64,
    /// Snapshots received when the subscribe phase returned.
    pub snapshots_at_warmup_start: u64,
}

/// Every subscription with its index and owning reader.
fn plan(config: &SubscriberConfig) -> Vec<(usize, bool, Vec<u64>)> {
    let n = config.readers.unwrap_or(0);
    let m = config.subs_per_reader as u64;
    let mut out: Vec<(usize, bool, Vec<u64>)> = (0..n)
        .map(|r| (r, false, (r as u64 * m..(r as u64 + 1) * m).collect()))
        .collect();
    let base = n as u64 * m;
    let ms = config.slow_subs() as u64;
    for s in 0..config.slow_readers {
        let first = base + s as u64 * ms;
        out.push((n + s, true, (first..first + ms).collect()));
    }
    out
}

fn sub_id(i: u64) -> String {
    format!("sub-{i}")
}

fn query_sub(query_id: String, query: Query, snapshot_only: bool) -> Vec<u8> {
    rmp_serde::to_vec_named(&Message::QuerySub(QuerySubMessage {
        payload: QuerySubPayload {
            query_id,
            map_name: MAP.to_string(),
            query,
            fields: None,
            snapshot_only,
        },
    }))
    .expect("serialize QUERY_SUB")
}

impl ReaderSet {
    /// Opens the readers, subscribes every subscription and returns once every
    /// snapshot is in (or fails once none has arrived for [`SNAPSHOT_TIMEOUT`]).
    pub async fn start(
        addr: SocketAddr,
        jwt_secret: &str,
        config: &SubscriberConfig,
    ) -> Result<Self> {
        let plan = plan(config);
        let pool = Arc::new(ConnectionPool::new(addr, plan.len(), jwt_secret).await?);
        let shared = Arc::new(Shared {
            timing: OnceLock::new(),
            snapshots: AtomicU64::new(0),
            snapshot_notify: Notify::new(),
            last_frame_ns: AtomicU64::new(0),
            epoch: Instant::now(),
        });
        let mut readers = Vec::with_capacity(plan.len());
        let mut subscribing = tokio::task::JoinSet::new();
        let mut subscriptions = 0;
        for (conn, slow, subs) in plan {
            let snapshotted = Arc::new(std::sync::Mutex::new(HashSet::new()));
            readers.push(spawn_reader(
                Arc::clone(&pool),
                conn,
                slow,
                subs.iter().map(|i| sub_id(*i)).collect(),
                config.slow_reader_frame_us,
                Arc::clone(&shared),
                Arc::clone(&snapshotted),
            ));
            subscriptions += subs.len() as u64;
            subscribing.spawn(subscribe_reader(
                Arc::clone(&pool),
                conn,
                subs,
                config.clone(),
                snapshotted,
                Arc::clone(&shared),
            ));
        }
        let mut set = Self {
            readers,
            shared,
            subscriptions,
            snapshots_at_warmup_start: 0,
        };
        let waited = set.wait_for_snapshots().await;
        subscribing.abort_all();
        waited?;
        set.snapshots_at_warmup_start = set.shared.snapshots.load(Ordering::Acquire);
        Ok(set)
    }

    async fn wait_for_snapshots(&self) -> Result<()> {
        loop {
            let notified = self.shared.snapshot_notify.notified();
            if self.shared.snapshots.load(Ordering::Acquire) >= self.subscriptions {
                return Ok(());
            }
            if tokio::time::timeout(SNAPSHOT_TIMEOUT, notified)
                .await
                .is_err()
            {
                bail!(
                    "{} of {} subscription snapshots arrived, then none for {:?}",
                    self.shared.snapshots.load(Ordering::Acquire),
                    self.subscriptions,
                    SNAPSHOT_TIMEOUT
                );
            }
        }
    }

    pub fn subscriptions(&self) -> u64 {
        self.subscriptions
    }

    /// Fixes the instants deliveries are measured against.
    pub fn set_timing(&self, timing: Timing) {
        let _ = self.shared.timing.set(timing);
    }

    /// Waits until no reader has received a frame (other than `PONG`) for
    /// `quiesce`, with nothing left unread. Returns whether that happened
    /// before [`QUIESCE_CAP`], and how long it took.
    pub async fn quiesce(&self, quiesce: Duration) -> (bool, Duration) {
        let start = Instant::now();
        loop {
            let last = self.shared.epoch
                + Duration::from_nanos(self.shared.last_frame_ns.load(Ordering::Acquire));
            let idle = Instant::now().saturating_duration_since(last.max(start));
            let unread = self
                .readers
                .iter()
                .any(|r| r.backlog.load(Ordering::Acquire) > 0);
            if idle >= quiesce && !unread {
                return (true, start.elapsed());
            }
            if start.elapsed() >= QUIESCE_CAP {
                return (false, start.elapsed());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Stops every reader and returns their states and the most unread frames
    /// any reader had.
    pub async fn stop(self) -> Result<(Vec<ReaderState>, i64)> {
        let mut states = Vec::with_capacity(self.readers.len());
        let mut max_backlog = 0;
        for reader in self.readers {
            reader.recv.abort();
            let _ = reader.recv.await;
            states.push(reader.process.await?);
            max_backlog = max_backlog.max(reader.max_backlog.load(Ordering::Acquire));
        }
        Ok((states, max_backlog))
    }
}

/// Sends one reader's live subscribes, at most [`SUBSCRIBE_WINDOW`]
/// unanswered at once, sending again one unanswered for
/// [`RESUBSCRIBE_AFTER`]; ends once every one has its snapshot.
async fn subscribe_reader(
    pool: Arc<ConnectionPool>,
    conn: usize,
    subs: Vec<u64>,
    config: SubscriberConfig,
    snapshotted: Arc<std::sync::Mutex<HashSet<String>>>,
    shared: Arc<Shared>,
) -> Result<()> {
    let mut pending: VecDeque<u64> = subs.into();
    let mut unanswered: Vec<(u64, Instant)> = Vec::new();
    loop {
        let notified = shared.snapshot_notify.notified();
        {
            let done = snapshotted.lock().expect("snapshot set");
            unanswered.retain(|(i, _)| !done.contains(&sub_id(*i)));
        }
        while unanswered.len() < SUBSCRIBE_WINDOW {
            let Some(i) = pending.pop_front() else {
                break;
            };
            pool.send_to(conn, &query_sub(sub_id(i), config.query(i), false))
                .await?;
            unanswered.push((i, Instant::now()));
        }
        if unanswered.is_empty() {
            return Ok(());
        }
        for (i, sent) in &mut unanswered {
            if sent.elapsed() >= RESUBSCRIBE_AFTER {
                pool.send_to(conn, &query_sub(sub_id(*i), config.query(*i), false))
                    .await?;
                *sent = Instant::now();
            }
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), notified).await;
    }
}

enum Inbound {
    Frame(Instant, Vec<u8>),
    Disconnected,
}

fn spawn_reader(
    pool: Arc<ConnectionPool>,
    conn: usize,
    slow: bool,
    query_ids: Vec<String>,
    slow_frame_us: u64,
    shared: Arc<Shared>,
    snapshotted: Arc<std::sync::Mutex<HashSet<String>>>,
) -> Reader {
    let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();
    let backlog = Arc::new(AtomicI64::new(0));
    let max_backlog = Arc::new(AtomicI64::new(0));
    let heartbeat = Heartbeat::start(Arc::clone(&pool), conn);

    let recv = {
        let (backlog, max_backlog) = (Arc::clone(&backlog), Arc::clone(&max_backlog));
        tokio::spawn(async move {
            let frame_gap = Duration::from_micros(slow_frame_us);
            let mut next_read = Instant::now();
            loop {
                if slow {
                    tokio::time::sleep_until(next_read).await;
                }
                let Ok(bytes) = pool.recv_from(conn).await else {
                    let _ = tx.send(Inbound::Disconnected);
                    return;
                };
                let at = Instant::now();
                if slow {
                    next_read = (next_read + frame_gap).max(at);
                }
                let n = backlog.fetch_add(1, Ordering::AcqRel) + 1;
                max_backlog.fetch_max(n, Ordering::AcqRel);
                if tx.send(Inbound::Frame(at, bytes)).is_err() {
                    return;
                }
            }
        })
    };

    let process = {
        let backlog = Arc::clone(&backlog);
        tokio::spawn(async move {
            let mut state = ReaderState::new(slow, query_ids);
            let mut reassembly = Reassembly::default();
            while let Some(inbound) = rx.recv().await {
                let (at, bytes) = match inbound {
                    Inbound::Frame(at, bytes) => (at, bytes),
                    Inbound::Disconnected => {
                        state.disconnected = true;
                        break;
                    }
                };
                let message = reassembly.push(bytes);
                let frame = match message {
                    Ok(Some(message)) => Some(ReaderFrame::decode(&message)),
                    Ok(None) => None,
                    Err(_) => Some(ReaderFrame::Other),
                };
                if let Some(frame) = frame {
                    state.frames += 1;
                    if !matches!(frame, ReaderFrame::Pong) {
                        let ns =
                            u64::try_from(at.saturating_duration_since(shared.epoch).as_nanos())
                                .unwrap_or(u64::MAX);
                        shared.last_frame_ns.fetch_max(ns, Ordering::AcqRel);
                    }
                    let resp_id = match &frame {
                        ReaderFrame::Resp(resp) => Some(resp.query_id.clone()),
                        _ => None,
                    };
                    if state.apply(frame, at, shared.timing.get()) {
                        if let Some(id) = resp_id {
                            snapshotted.lock().expect("snapshot set").insert(id);
                        }
                        shared.snapshots.fetch_add(1, Ordering::AcqRel);
                        shared.snapshot_notify.notify_waiters();
                    }
                }
                backlog.fetch_sub(1, Ordering::AcqRel);
            }
            state
        })
    };

    Reader {
        recv,
        process,
        backlog,
        max_backlog,
        _heartbeat: heartbeat,
    }
}

// ---------------------------------------------------------------------------
// The fresh-query check
// ---------------------------------------------------------------------------

/// How one view compares with a fresh query of the same query.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ViewDiff {
    /// Keys missing from either side, plus keys present on both with
    /// different values.
    pub mismatches: u64,
}

/// A value with every map's entries in key order: a client reads a map as an
/// object, and the server does not keep one field order on every path (a
/// fresh query's rows come back with their fields sorted).
fn canonical(value: &rmpv::Value) -> rmpv::Value {
    match value {
        rmpv::Value::Map(entries) => {
            let mut entries: Vec<(rmpv::Value, rmpv::Value)> = entries
                .iter()
                .map(|(k, v)| (canonical(k), canonical(v)))
                .collect();
            entries.sort_by_cached_key(|(k, _)| k.to_string());
            rmpv::Value::Map(entries)
        }
        rmpv::Value::Array(items) => rmpv::Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Compares a reader's view with a fresh result, by key and by value.
pub fn compare_view(
    view: &HashMap<String, rmpv::Value>,
    fresh: &HashMap<String, rmpv::Value>,
) -> ViewDiff {
    let mut mismatches = 0;
    for (key, value) in view {
        if fresh.get(key).map(canonical) != Some(canonical(value)) {
            mismatches += 1;
        }
    }
    mismatches += fresh.keys().filter(|k| !view.contains_key(*k)).count() as u64;
    ViewDiff { mismatches }
}

/// A fresh result the check cannot compare: the server clamped an unlimited
/// query (`max_query_records`).
fn truncated(query: &Query, resp: &QueryRespPayload) -> bool {
    query.limit.is_none() && resp.has_more == Some(true)
}

/// Runs one one-shot query per subscription index, `CHECK_IN_FLIGHT` at a
/// time on each of `CHECK_CONNECTIONS` connections.
async fn fresh_results(
    addr: SocketAddr,
    jwt_secret: &str,
    config: &SubscriberConfig,
    indices: Vec<u64>,
) -> Result<HashMap<u64, QueryRespPayload>> {
    if indices.is_empty() {
        return Ok(HashMap::new());
    }
    let conns = CHECK_CONNECTIONS.min(indices.len());
    let pool = Arc::new(ConnectionPool::new(addr, conns, jwt_secret).await?);
    let queue = Arc::new(Mutex::new(indices.into_iter().collect::<VecDeque<u64>>()));
    let mut tasks = tokio::task::JoinSet::new();
    for conn in 0..conns {
        let (pool, queue, config) = (Arc::clone(&pool), Arc::clone(&queue), config.clone());
        tasks.spawn(async move {
            let _heartbeat = Heartbeat::start(Arc::clone(&pool), conn);
            let mut out = HashMap::new();
            let mut in_flight = 0usize;
            let mut reassembly = Reassembly::default();
            loop {
                while in_flight < CHECK_IN_FLIGHT {
                    let Some(i) = queue.lock().await.pop_front() else {
                        break;
                    };
                    pool.send_to(
                        conn,
                        &query_sub(format!("fresh-{i}"), config.query(i), true),
                    )
                    .await?;
                    in_flight += 1;
                }
                if in_flight == 0 {
                    return Ok::<_, anyhow::Error>(out);
                }
                let bytes = tokio::time::timeout(SNAPSHOT_TIMEOUT, pool.recv_from(conn))
                    .await
                    .map_err(|_| anyhow!("a fresh query was not answered in time"))??;
                let Some(message) = reassembly.push(bytes)? else {
                    continue;
                };
                if let ReaderFrame::Resp(resp) = ReaderFrame::decode(&message) {
                    let i: u64 = resp
                        .query_id
                        .strip_prefix("fresh-")
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| anyhow!("unexpected query id {}", resp.query_id))?;
                    if resp.error.is_some() || resp.code.is_some() {
                        bail!("fresh query {i} refused: {:?} {:?}", resp.code, resp.error);
                    }
                    out.insert(i, resp);
                    in_flight -= 1;
                }
            }
        });
    }
    let mut all = HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        all.extend(joined??);
    }
    Ok(all)
}

/// The outcome of comparing every view with a fresh query.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CheckOutcome {
    pub view_mismatches: u64,
    pub views_checked: u64,
    /// Views of connected readers whose fresh result was truncated.
    pub views_unchecked: u64,
    /// Views of readers the server disconnected: not compared, since a
    /// disconnected reader is behind by design; the disconnect itself is
    /// reported (and invalidates the run unless the reader is slow).
    pub views_disconnected: u64,
}

pub async fn check_views(
    addr: SocketAddr,
    jwt_secret: &str,
    config: &SubscriberConfig,
    states: &[ReaderState],
) -> Result<CheckOutcome> {
    let mut outcome = CheckOutcome::default();
    let mut wanted = Vec::new();
    for state in states {
        for id in state.views.keys() {
            let i: u64 = id
                .strip_prefix("sub-")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| anyhow!("unexpected subscription id {id}"))?;
            if state.disconnected {
                outcome.views_disconnected += 1;
            } else {
                wanted.push(i);
            }
        }
    }
    let fresh = fresh_results(addr, jwt_secret, config, wanted).await?;
    for state in states {
        if state.disconnected {
            continue;
        }
        for (id, view) in &state.views {
            let i: u64 = id["sub-".len()..].parse()?;
            let resp = fresh
                .get(&i)
                .ok_or_else(|| anyhow!("no fresh result for {id}"))?;
            if truncated(&config.query(i), resp) {
                outcome.views_unchecked += 1;
                continue;
            }
            let fresh_view: HashMap<String, rmpv::Value> = resp
                .results
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect();
            outcome.view_mismatches += compare_view(view, &fresh_view).mismatches;
            outcome.views_checked += 1;
        }
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Server metrics
// ---------------------------------------------------------------------------

/// The live-delivery series of one recorder render; a family the server does
/// not register is `null`.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ServerMetrics {
    pub live_event_loss_disconnects_total: Option<BTreeMap<String, f64>>,
    pub live_resnapshots: Option<BTreeMap<String, f64>>,
    pub fanout_lane: Option<BTreeMap<String, f64>>,
    pub live_stage: Option<BTreeMap<String, f64>>,
}

/// Every sample whose series name starts with `prefix`, keyed by the series
/// (name and labels); `None` when there is none.
pub fn scrape_family(render: &str, prefix: &str) -> Option<BTreeMap<String, f64>> {
    let family: BTreeMap<String, f64> = render
        .lines()
        .filter(|l| l.starts_with(prefix))
        .filter_map(|l| {
            let (series, value) = l.rsplit_once(' ')?;
            Some((series.to_string(), value.trim().parse().ok()?))
        })
        .collect();
    (!family.is_empty()).then_some(family)
}

impl ServerMetrics {
    pub fn scrape(render: &str) -> Self {
        Self {
            live_event_loss_disconnects_total: scrape_family(
                render,
                "topgun_live_event_loss_disconnects_total",
            ),
            live_resnapshots: scrape_family(render, "topgun_live_resnapshots_"),
            fanout_lane: scrape_family(render, "topgun_fanout_lane_"),
            live_stage: scrape_family(render, "topgun_live_stage_"),
        }
    }
}

// ---------------------------------------------------------------------------
// The arm's measurement and report block
// ---------------------------------------------------------------------------

/// Everything the subscriber arm measured.
pub struct SubscriberMeasurement {
    pub subscriptions: u64,
    pub snapshots_at_warmup_start: u64,
    pub states: Vec<ReaderState>,
    pub max_unread_frames: i64,
    pub quiesced: bool,
    pub quiesce_wait: Duration,
    pub check: CheckOutcome,
    pub metrics: ServerMetrics,
    /// Whether the server under test is built from the sources of
    /// [`BEFORE_SERVER`]; `None` when `git` could not tell.
    pub before_server: Option<bool>,
}

/// The server sources the "before" figures are taken on. That server drops a
/// reader at its first lost live event (`TG-LIVE-003`), which at the matrix
/// rate is the measured outcome, not a broken run.
pub const BEFORE_SERVER: &str = "e040f7ee";

/// The outbound channel capacity of a connection on [`BEFORE_SERVER`]. A
/// reader whose own unread frames reached it may have stopped reading
/// itself, so its disconnect is the harness's, not the server's.
const BEFORE_CHANNEL_CAPACITY: i64 = 256;

/// Whether `packages/server-rust/src` of the working tree equals
/// [`BEFORE_SERVER`]'s; `None` when `git` cannot answer.
pub fn server_src_is_before() -> Option<bool> {
    let status = std::process::Command::new("git")
        .args([
            "diff",
            "--quiet",
            BEFORE_SERVER,
            "--",
            ":/packages/server-rust/src",
        ])
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// The "before" server's recorded outcome of an arm whose readers it
/// dropped: every disconnect the server counted was `channel_full`, and no
/// reader's own backlog reached the channel capacity.
#[derive(Debug, Clone, Serialize)]
pub struct BeforeOutcome {
    pub kind: &'static str,
    pub disconnects: u64,
    pub slow_reader_disconnects: u64,
    pub channel_full: u64,
    pub max_unread_frames: i64,
}

/// The mechanical predicate of [`BeforeOutcome`]; `None` when it does not
/// hold or nothing was disconnected.
pub fn before_outcome(
    before_server: Option<bool>,
    disconnects: u64,
    slow_reader_disconnects: u64,
    metrics: &ServerMetrics,
    max_unread_frames: i64,
) -> Option<BeforeOutcome> {
    let total = disconnects + slow_reader_disconnects;
    let losses = metrics.live_event_loss_disconnects_total.as_ref()?;
    let (mut channel_full, mut other) = (0u64, 0u64);
    for (series, value) in losses {
        if series.contains("cause=\"channel_full\"") {
            channel_full += *value as u64;
        } else {
            other += *value as u64;
        }
    }
    let holds = before_server == Some(true)
        && total > 0
        && other == 0
        && channel_full >= total
        && max_unread_frames < BEFORE_CHANNEL_CAPACITY;
    holds.then_some(BeforeOutcome {
        kind: "readers_disconnected",
        disconnects,
        slow_reader_disconnects,
        channel_full,
        max_unread_frames,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryLatency {
    pub count: u64,
    pub p50: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlowReaders {
    pub readers: usize,
    pub frames: u64,
    pub resnapshots: u64,
    pub disconnects: u64,
}

/// The report's `subscribers` block.
#[derive(Debug, Clone, Serialize)]
pub struct SubscribersBlock {
    pub config: SubscriberConfig,
    pub subscriptions: u64,
    pub snapshots_at_warmup_start: u64,
    /// `QUERY_UPDATE` frames of the non-slow readers inside the window.
    pub deliveries: u64,
    pub deliveries_per_sec: f64,
    /// `SERVER_EVENT` frames of the non-slow readers inside the window.
    pub map_events: u64,
    pub map_events_per_sec: f64,
    pub delivery_latency_us: DeliveryLatency,
    pub resnapshots: u64,
    /// Non-slow readers the server disconnected.
    pub disconnects: u64,
    pub query_errors: u64,
    pub protocol_anomalies: u64,
    pub slow_readers: SlowReaders,
    pub max_unread_frames: i64,
    pub quiesced: bool,
    pub quiesce_wait_ms: u64,
    pub view_mismatches: u64,
    pub views_checked: u64,
    pub views_unchecked: u64,
    pub views_disconnected: u64,
    pub server_metrics: ServerMetrics,
    pub before_server: Option<bool>,
    /// Set when the "before" server dropped readers (see [`before_outcome`]);
    /// those disconnects then do not invalidate the run.
    pub before_outcome: Option<BeforeOutcome>,
}

impl SubscribersBlock {
    pub fn of(m: &SubscriberMeasurement, config: &SubscriberConfig, duration_secs: u64) -> Self {
        let mut latency = Histogram::<u64>::new_with_bounds(1, HIST_MAX_US, 3).expect("bounds");
        let (mut deliveries, mut map_events, mut resnapshots, mut disconnects) = (0, 0, 0, 0);
        let (mut query_errors, mut anomalies) = (0, 0);
        let mut slow = SlowReaders {
            readers: 0,
            frames: 0,
            resnapshots: 0,
            disconnects: 0,
        };
        for s in &m.states {
            query_errors += s.query_errors;
            anomalies += s.anomalies;
            if s.slow {
                slow.readers += 1;
                slow.frames += s.frames;
                slow.resnapshots += s.resnapshots;
                slow.disconnects += u64::from(s.disconnected);
                continue;
            }
            latency.add(&s.latency).expect("same bounds");
            deliveries += s.deliveries;
            map_events += s.map_events;
            resnapshots += s.resnapshots;
            disconnects += u64::from(s.disconnected);
        }
        let secs = duration_secs as f64;
        let before_outcome = before_outcome(
            m.before_server,
            disconnects,
            slow.disconnects,
            &m.metrics,
            m.max_unread_frames,
        );
        Self {
            config: config.clone(),
            subscriptions: m.subscriptions,
            snapshots_at_warmup_start: m.snapshots_at_warmup_start,
            deliveries,
            deliveries_per_sec: deliveries as f64 / secs,
            map_events,
            map_events_per_sec: map_events as f64 / secs,
            delivery_latency_us: DeliveryLatency {
                count: latency.len(),
                p50: latency.value_at_quantile(0.50),
                p99: latency.value_at_quantile(0.99),
                p999: latency.value_at_quantile(0.999),
                max: latency.max(),
            },
            resnapshots,
            disconnects,
            query_errors,
            protocol_anomalies: anomalies,
            slow_readers: slow,
            max_unread_frames: m.max_unread_frames,
            quiesced: m.quiesced,
            quiesce_wait_ms: m.quiesce_wait.as_millis() as u64,
            view_mismatches: m.check.view_mismatches,
            views_checked: m.check.views_checked,
            views_unchecked: m.check.views_unchecked,
            views_disconnected: m.check.views_disconnected,
            server_metrics: m.metrics.clone(),
            before_server: m.before_server,
            before_outcome,
        }
    }

    /// The validity reasons this block adds.
    pub fn invalid_reasons(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.max_unread_frames > READER_BEHIND_FRAMES {
            reasons.push("reader_behind");
        }
        if self.view_mismatches > 0 {
            reasons.push("view_mismatch");
        }
        if self.disconnects > 0 && self.before_outcome.is_none() {
            reasons.push("disconnect");
        }
        if !self.quiesced {
            reasons.push("not_quiesced");
        }
        if self.views_unchecked > 0 {
            reasons.push("view_unchecked");
        }
        reasons
    }
}

// ---------------------------------------------------------------------------
// Self-tests
// ---------------------------------------------------------------------------

fn update_frame(query_id: &str, key: &str, t: u64, change: ChangeEventType) -> Vec<u8> {
    rmp_serde::to_vec_named(&Message::QueryUpdate {
        payload: topgun_core::messages::QueryUpdatePayload {
            query_id: query_id.to_string(),
            key: key.to_string(),
            value: rmpv::Value::Map(vec![
                (rmpv::Value::from("v"), rmpv::Value::from(1)),
                (rmpv::Value::from("t"), rmpv::Value::from(t)),
            ]),
            change_type: change,
        },
    })
    .expect("serialize QUERY_UPDATE")
}

/// A delivery of a value written for `t` = 1 000 µs, received 5 000 µs after
/// t0, is recorded as 4 000 µs: from the write's schedule, not from anything
/// the server or the reader did.
pub fn delivery_latency_is_from_the_scheduled_instant() {
    let t0 = Instant::now();
    let timing = Timing {
        t0,
        window_start: t0,
        window_end: t0 + Duration::from_secs(1),
    };
    let mut state = ReaderState::new(false, ["q".to_string()]);
    let frame = ReaderFrame::decode(&update_frame("q", "k", 1_000, ChangeEventType::ENTER));
    state.apply(frame, t0 + Duration::from_micros(5_000), Some(&timing));
    assert_eq!(state.deliveries, 1);
    assert_eq!(state.latency.len(), 1);
    let recorded = state.latency.max();
    assert!(
        (3_990..=4_010).contains(&recorded),
        "recorded {recorded} us, expected 4000"
    );
    assert_eq!(state.views["q"].len(), 1);
    let leave = ReaderFrame::decode(&update_frame("q", "k", 2_000, ChangeEventType::LEAVE));
    state.apply(leave, t0 + Duration::from_micros(6_000), Some(&timing));
    assert!(state.views["q"].is_empty(), "LEAVE removes the key");
}

/// A view that holds a key with a stale value differs from the fresh result
/// by exactly one; a missing and an extra key count one each; the same fields
/// in another order are the same value.
pub fn compare_counts_a_stale_value() {
    let v = |n: i64| {
        rmpv::Value::Map(vec![
            (rmpv::Value::from("v"), rmpv::Value::from(n)),
            (rmpv::Value::from("bucket"), rmpv::Value::from(3)),
        ])
    };
    let reordered = |n: i64| {
        rmpv::Value::Map(vec![
            (rmpv::Value::from("bucket"), rmpv::Value::from(3)),
            (rmpv::Value::from("v"), rmpv::Value::from(n)),
        ])
    };
    let fresh: HashMap<String, rmpv::Value> =
        [("a".to_string(), v(1)), ("b".to_string(), v(2))].into();
    let stale: HashMap<String, rmpv::Value> =
        [("a".to_string(), v(1)), ("b".to_string(), v(1))].into();
    assert_eq!(compare_view(&stale, &fresh).mismatches, 1);
    assert_eq!(compare_view(&fresh, &fresh).mismatches, 0);
    let same: HashMap<String, rmpv::Value> = [
        ("a".to_string(), reordered(1)),
        ("b".to_string(), reordered(2)),
    ]
    .into();
    assert_eq!(compare_view(&same, &fresh).mismatches, 0);
    let other: HashMap<String, rmpv::Value> =
        [("a".to_string(), v(1)), ("c".to_string(), v(2))].into();
    assert_eq!(compare_view(&other, &fresh).mismatches, 2);
}

/// A second `QUERY_RESP` for an active id replaces the view and counts one
/// re-snapshot; a chunked message is reassembled before it is decoded.
pub fn resp_replaces_and_chunks_reassemble() {
    let resp = |keys: &[&str]| {
        rmp_serde::to_vec_named(&Message::QueryResp(
            topgun_core::messages::QueryRespMessage {
                payload: QueryRespPayload {
                    query_id: "q".into(),
                    results: keys
                        .iter()
                        .map(|k| topgun_core::messages::QueryResultEntry {
                            key: (*k).to_string(),
                            value: rmpv::Value::from(1),
                        })
                        .collect(),
                    ..Default::default()
                },
            },
        ))
        .expect("serialize QUERY_RESP")
    };
    let mut state = ReaderState::new(false, ["q".to_string()]);
    let now = Instant::now();
    let timing = Timing {
        t0: now,
        window_start: now,
        window_end: now,
    };
    assert!(state.apply(ReaderFrame::decode(&resp(&["a", "b"])), now, None));
    assert!(!state.apply(ReaderFrame::decode(&resp(&["a"])), now, None));
    assert_eq!(
        state.resnapshots, 0,
        "a second answer before t0 is no re-snapshot"
    );
    assert!(!state.apply(ReaderFrame::decode(&resp(&["c"])), now, Some(&timing)));
    assert_eq!(state.resnapshots, 1);
    assert_eq!(state.views["q"].keys().collect::<Vec<_>>(), vec!["c"]);

    let whole = resp(&["x", "y", "z"]);
    let chunk = |last: bool, data: &[u8]| {
        let mut out = Vec::new();
        rmpv::encode::write_value(
            &mut out,
            &rmpv::Value::Map(vec![
                (rmpv::Value::from("type"), rmpv::Value::from("CHUNK")),
                (rmpv::Value::from("last"), rmpv::Value::from(last)),
                (
                    rmpv::Value::from("data"),
                    rmpv::Value::Binary(data.to_vec()),
                ),
            ]),
        )
        .expect("encode CHUNK");
        out
    };
    let mut r = Reassembly::default();
    let (a, b) = whole.split_at(whole.len() / 2);
    assert!(r.push(chunk(false, a)).expect("chunk").is_none());
    assert_eq!(r.push(chunk(true, b)).expect("chunk"), Some(whole));
}

/// The "before" server's disconnects with `channel_full` and a reader backlog
/// below the channel capacity are its recorded outcome; on any other server,
/// with another cause, or with the backlog at the capacity, they invalidate.
pub fn before_outcome_only_on_the_before_server() {
    let metrics = |full: f64, other: f64| ServerMetrics {
        live_event_loss_disconnects_total: Some(BTreeMap::from([
            (
                "topgun_live_event_loss_disconnects_total{cause=\"channel_full\"}".to_string(),
                full,
            ),
            (
                "topgun_live_event_loss_disconnects_total{cause=\"encode_failed\"}".to_string(),
                other,
            ),
        ])),
        ..ServerMetrics::default()
    };
    let block = |before: Option<bool>, m: ServerMetrics, backlog: i64| {
        let mut states = vec![ReaderState::new(false, ["q".to_string()])];
        states[0].disconnected = true;
        let measurement = SubscriberMeasurement {
            subscriptions: 1,
            snapshots_at_warmup_start: 1,
            states,
            max_unread_frames: backlog,
            quiesced: true,
            quiesce_wait: Duration::ZERO,
            check: CheckOutcome::default(),
            metrics: m,
            before_server: before,
        };
        SubscribersBlock::of(&measurement, &SubscriberConfig::default(), 1)
    };
    let before = block(Some(true), metrics(1.0, 0.0), 127);
    assert_eq!(
        before.before_outcome.as_ref().map(|o| o.kind),
        Some("readers_disconnected")
    );
    assert!(!before.invalid_reasons().contains(&"disconnect"));
    for after in [
        block(Some(false), metrics(1.0, 0.0), 127),
        block(None, metrics(1.0, 0.0), 127),
        block(Some(true), metrics(1.0, 1.0), 127),
        block(Some(true), metrics(1.0, 0.0), 256),
    ] {
        assert!(after.before_outcome.is_none());
        assert!(after.invalid_reasons().contains(&"disconnect"));
    }
}

pub const SELF_TESTS: [(&str, fn()); 4] = [
    (
        "before_outcome_only_on_the_before_server",
        before_outcome_only_on_the_before_server,
    ),
    (
        "delivery_latency_is_from_the_scheduled_instant",
        delivery_latency_is_from_the_scheduled_instant,
    ),
    ("compare_counts_a_stale_value", compare_counts_a_stale_value),
    (
        "resp_replaces_and_chunks_reassemble",
        resp_replaces_and_chunks_reassemble,
    ),
];
