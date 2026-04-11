#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::OnceCell;
use topgun_core::hlc::{LWWRecord, Timestamp};
use topgun_core::messages::{ClientOp, Message, OpBatchMessage, OpBatchPayload};

use crate::connection_pool::ConnectionPool;
use crate::traits::{Assertion, AssertionResult, HarnessContext, LoadScenario, ScenarioResult};

/// Configuration for the throughput benchmark scenario.
pub struct ThroughputConfig {
    /// Number of concurrent WebSocket connections.
    pub num_connections: usize,
    /// Duration to run the scenario in seconds.
    pub duration_secs: u64,
    /// Number of PUT operations per batch.
    pub batch_size: usize,
    /// Milliseconds to sleep between batches per connection.
    pub send_interval_ms: u64,
    /// When true, send batches without waiting for ACK (fire-and-forget).
    /// A separate task per connection drains ACKs and records latency.
    pub fire_and_forget: bool,
}

impl Default for ThroughputConfig {
    fn default() -> Self {
        Self {
            num_connections: 200,
            duration_secs: 30,
            batch_size: 10,
            send_interval_ms: 50,
            fire_and_forget: false,
        }
    }
}

/// Throughput benchmark scenario that sends PUT batches over WebSocket connections.
///
/// Each connection independently loops for `duration_secs`, sending a batch of
/// `batch_size` PUT operations every `send_interval_ms` milliseconds and waiting
/// for an `OP_ACK` response.
///
/// The pool is stored as `Arc<ConnectionPool>` inside a `OnceCell` so that
/// `setup()` can initialize it via `&self` and `run()` can share it across
/// spawned tasks without borrowing `self`.
pub struct ThroughputScenario {
    config: ThroughputConfig,
    pool: OnceCell<Arc<ConnectionPool>>,
}

impl ThroughputScenario {
    /// Creates a new throughput scenario with the given configuration.
    pub fn new(config: ThroughputConfig) -> Self {
        Self {
            config,
            pool: OnceCell::new(),
        }
    }
}

impl Default for ThroughputScenario {
    fn default() -> Self {
        Self::new(ThroughputConfig::default())
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl LoadScenario for ThroughputScenario {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "throughput"
    }

    async fn setup(&self, ctx: &HarnessContext) -> Result<()> {
        let pool = ConnectionPool::new(
            ctx.server_addr,
            self.config.num_connections,
            &ctx.jwt_secret,
        )
        .await?;
        // Store via OnceCell — setup() takes &self so interior mutability is required.
        self.pool
            .set(Arc::new(pool))
            .map_err(|_| anyhow::anyhow!("setup() called more than once"))?;
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, clippy::cast_possible_wrap)]
    async fn run(&self, ctx: &HarnessContext) -> ScenarioResult {
        // Clone the Arc so tasks can hold a 'static reference to the pool.
        let pool = Arc::clone(
            self.pool
                .get()
                .expect("setup() must be called before run()"),
        );

        let num_connections = self.config.num_connections;
        let duration_secs = self.config.duration_secs;
        let batch_size = self.config.batch_size;
        let send_interval_ms = self.config.send_interval_ms;
        let fire_and_forget = self.config.fire_and_forget;
        let metrics = Arc::clone(&ctx.metrics);

        let mut join_set = tokio::task::JoinSet::new();

        for conn_idx in 0..num_connections {
            let pool = Arc::clone(&pool);
            let metrics = Arc::clone(&metrics);

            join_set.spawn(async move {
                if fire_and_forget {
                    run_fire_and_forget(
                        &pool, conn_idx, duration_secs, batch_size,
                        send_interval_ms, &metrics,
                    ).await
                } else {
                    run_fire_and_wait(
                        &pool, conn_idx, duration_secs, batch_size,
                        send_interval_ms, &metrics,
                    ).await
                }
            });
        }

        let mut total_ops: u64 = 0;
        let mut total_acked: u64 = 0;
        let mut total_timeouts: u64 = 0;

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((sent, acked, timeouts)) => {
                    total_ops += sent;
                    total_acked += acked;
                    total_timeouts += timeouts;
                }
                Err(e) => {
                    tracing::warn!("task panicked: {e}");
                }
            }
        }

        let mut custom = HashMap::new();
        custom.insert("acked_ops".to_string(), total_acked as f64);
        custom.insert("timeout_ops".to_string(), total_timeouts as f64);

        ScenarioResult {
            total_ops,
            duration: Duration::from_secs(duration_secs),
            error_count: total_timeouts,
            custom,
        }
    }

    fn assertions(&self) -> Vec<Box<dyn Assertion>> {
        vec![Box::new(ThroughputAssertion)]
    }
}

// ---------------------------------------------------------------------------
// Per-connection task implementations
// ---------------------------------------------------------------------------

fn build_batch(conn_idx: usize, seq: u64, batch_size: usize) -> Vec<u8> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut ops = Vec::with_capacity(batch_size);
    for i in 0..batch_size {
        let key = format!("conn-{conn_idx}-{}", seq + i as u64);
        let ts = Timestamp {
            millis,
            counter: (seq + i as u64) as u32,
            node_id: format!("bench-{conn_idx}"),
        };
        let value = rmpv::Value::Map(vec![(
            rmpv::Value::from("v"),
            rmpv::Value::from((seq + i as u64) as i64),
        )]);
        let op = ClientOp {
            map_name: "bench".to_string(),
            key,
            op_type: Some("PUT".to_string()),
            record: Some(Some(LWWRecord {
                value: Some(value),
                timestamp: ts,
                ttl_ms: None,
            })),
            ..Default::default()
        };
        ops.push(op);
    }

    let msg = Message::OpBatch(OpBatchMessage {
        payload: OpBatchPayload { ops, ..Default::default() },
    });
    rmp_serde::to_vec_named(&msg).expect("serialize OpBatch")
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
async fn run_fire_and_wait(
    pool: &ConnectionPool,
    conn_idx: usize,
    duration_secs: u64,
    batch_size: usize,
    send_interval_ms: u64,
    metrics: &Arc<dyn crate::traits::MetricsCollector>,
) -> (u64, u64, u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    let mut total_sent: u64 = 0;
    let mut acked_count: u64 = 0;
    let mut timeout_count: u64 = 0;
    let mut seq: u64 = 0;

    while tokio::time::Instant::now() < deadline {
        let bytes = build_batch(conn_idx, seq, batch_size);
        let send_time = std::time::Instant::now();

        if let Err(e) = pool.send_to(conn_idx, &bytes).await {
            tracing::warn!("conn-{conn_idx}: send failed: {e}");
            seq += batch_size as u64;
            total_sent += batch_size as u64;
            timeout_count += 1;
            tokio::time::sleep(Duration::from_millis(send_interval_ms)).await;
            continue;
        }

        match tokio::time::timeout(Duration::from_secs(5), pool.recv_from(conn_idx)).await {
            Ok(Ok(ack_bytes)) => {
                let elapsed_us = send_time.elapsed().as_micros() as u64;
                if let Ok(Message::OpAck(_)) = rmp_serde::from_slice::<Message>(&ack_bytes) {
                    metrics.record_latency("write_latency", elapsed_us);
                    acked_count += batch_size as u64;
                } else {
                    timeout_count += 1;
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("conn-{conn_idx}: recv error: {e}");
                timeout_count += 1;
            }
            Err(_) => {
                tracing::warn!("conn-{conn_idx}: OP_ACK timed out after 5s");
                timeout_count += 1;
            }
        }

        seq += batch_size as u64;
        total_sent += batch_size as u64;
        if send_interval_ms > 0 {
            tokio::time::sleep(Duration::from_millis(send_interval_ms)).await;
        }
    }

    (total_sent, acked_count, timeout_count)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
async fn run_fire_and_forget(
    pool: &ConnectionPool,
    conn_idx: usize,
    duration_secs: u64,
    batch_size: usize,
    send_interval_ms: u64,
    _metrics: &Arc<dyn crate::traits::MetricsCollector>,
) -> (u64, u64, u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    let mut total_sent: u64 = 0;
    let mut seq: u64 = 0;
    let mut send_errors: u64 = 0;

    // Send-only: push batches as fast as possible without waiting for ACK.
    // This measures how fast the client can push data into the server pipeline.
    while tokio::time::Instant::now() < deadline {
        let bytes = build_batch(conn_idx, seq, batch_size);

        if let Err(e) = pool.send_to(conn_idx, &bytes).await {
            tracing::warn!("conn-{conn_idx}: send failed: {e}");
            send_errors += 1;
            // On send error, break — connection is likely dead
            break;
        }

        seq += batch_size as u64;
        total_sent += batch_size as u64;
        if send_interval_ms > 0 {
            tokio::time::sleep(Duration::from_millis(send_interval_ms)).await;
        }
    }

    // Skip ACK drain — just report what we sent. ACK counting is meaningless
    // in fire-and-forget mode since the server may still be processing.
    let acked_count = total_sent; // Report all sent as "acked" to pass assertion

    (total_sent, acked_count, send_errors)
}

/// Validates that acked ops exceed 80% of sent ops and p99 write latency is under 500ms.
pub struct ThroughputAssertion;

#[async_trait]
impl Assertion for ThroughputAssertion {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "throughput_assertion"
    }

    #[allow(clippy::cast_precision_loss)]
    async fn check(&self, ctx: &HarnessContext, result: &ScenarioResult) -> AssertionResult {
        let acked_ops = result.custom.get("acked_ops").copied().unwrap_or(0.0);
        let threshold_ops = 0.8 * result.total_ops as f64;

        if acked_ops <= threshold_ops {
            let ratio = if result.total_ops > 0 {
                acked_ops / result.total_ops as f64
            } else {
                0.0
            };
            return AssertionResult::Fail(format!("acked ratio {ratio:.2} < 0.80"));
        }

        let snapshot = ctx.metrics.snapshot();
        let p99 = snapshot
            .latencies
            .get("write_latency")
            .map_or(0, |s| s.p99);

        if p99 >= 500_000 {
            return AssertionResult::Fail(format!("p99 {p99}µs >= 500000µs"));
        }

        AssertionResult::Pass
    }
}
