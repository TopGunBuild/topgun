> **SPLIT:** This specification was decomposed into:
> - SPEC-121a: Load Harness Traits and Metrics
> - SPEC-121b: Load Harness Connection Pool
> - SPEC-121c: Throughput Scenario and Harness Main
>
> See child specifications for implementation.

---
id: SPEC-121
type: feature
status: draft
priority: P2
complexity: large
created: 2026-03-17
source: TODO-114
---

# Rust-Native Load Testing Harness

## Context

The current load testing suite uses k6 (JavaScript/Go runtime) which caps at ~200 VUs due to k6's WebSocket implementation overhead and requires a custom xk6-msgpack extension for MsgPack encoding. The k6 harness cannot:

- Scale beyond a few hundred concurrent WebSocket connections efficiently
- Assert CRDT convergence semantics (e.g., verify all replicas see the same final LWW value)
- Measure Merkle sync delta timing with sub-millisecond precision
- Simulate split-brain scenarios (partition network between cluster nodes)
- Produce HDR histogram percentiles (p99.9, p99.99) needed for tail latency analysis

A Rust-native harness using tokio + tokio-tungstenite can spawn 10k+ concurrent connections in-process, share the same MsgPack codec as the server, and assert domain-specific invariants directly in Rust.

k6 remains for CI regression gating (smoke tests, basic throughput thresholds).

## Goal Analysis

**Goal Statement:** Provide a Rust-native load testing framework that measures server throughput and latency at 10k+ concurrent WebSocket connections with CRDT-aware correctness assertions.

**Observable Truths:**
1. Running `cargo bench --bench load_harness` produces an HDR histogram report with p50/p95/p99/p99.9 latencies
2. 10,000 concurrent WebSocket connections authenticate and exchange messages without server crash or harness OOM
3. After a write-heavy load test, a CRDT convergence check confirms all subscribed clients received the final LWW value
4. Merkle sync round-trip time is measured: a late-joining client's time-to-consistent-state is reported
5. Split-brain simulation: two server instances partition, heal, and CRDT data converges post-heal

**Required Artifacts:**
- `packages/server-rust/benches/load_harness/` -- benchmark binary and modules
- Trait definitions for scenario, metrics collection, and assertions
- Connection pool that manages 10k+ tokio-tungstenite sessions
- HDR histogram integration for latency recording
- MsgPack encode/decode using same `rmp-serde` as server
- JWT token generation for auth (using `jsonwebtoken` crate, same as server)

**Key Links:**
- Connection pool -> tokio-tungstenite -> server's axum WS handler
- MsgPack codec must match server's `rmp_serde::to_vec_named()` format exactly
- JWT tokens must use `test-e2e-secret` to match `test_server.rs` config
- HDR histogram crate records latencies; report generation reads from histogram

## Task

Build a Rust load testing harness as a cargo bench target in `packages/server-rust/benches/load_harness/`. The harness:

1. Defines traits for load scenarios, metric collectors, and assertion checks
2. Implements a connection pool that opens N concurrent WebSocket connections with auth
3. Records all latencies in HDR histograms (hdrhistogram crate)
4. Provides CRDT-aware assertion helpers (convergence check, Merkle sync timing)
5. Includes a throughput benchmark scenario equivalent to the existing `bench-throughput.js`

## Requirements

### New Files

**1. `packages/server-rust/benches/load_harness/main.rs`**
- Criterion or custom bench harness entry point
- Starts an in-process test server (reuse `build_services()` from `test_server.rs`)
- Runs registered scenarios sequentially
- Prints HDR histogram summary after each scenario

**2. `packages/server-rust/benches/load_harness/traits.rs`**
- `LoadScenario` trait:
  - `fn name(&self) -> &str`
  - `async fn setup(&self, ctx: &HarnessContext) -> Result<()>` -- pre-scenario setup
  - `async fn run(&self, ctx: &HarnessContext) -> ScenarioResult` -- execute the scenario
  - `fn assertions(&self) -> Vec<Box<dyn Assertion>>` -- post-run checks
- `Assertion` trait:
  - `fn name(&self) -> &str`
  - `async fn check(&self, ctx: &HarnessContext, result: &ScenarioResult) -> AssertionResult`
- `MetricsCollector` trait:
  - `fn record_latency(&self, operation: &str, duration_us: u64)`
  - `fn increment_counter(&self, name: &str, count: u64)`
  - `fn snapshot(&self) -> MetricsSnapshot`
- `HarnessContext` struct: holds server addr, JWT secret, metrics collector, connection pool reference
- `ScenarioResult` struct: holds total_ops, duration, error_count, custom data map
- `AssertionResult` enum: `Pass` | `Fail(String)`
- `MetricsSnapshot` struct: holds HashMap of operation -> `LatencyStats { p50, p95, p99, p999, min, max, mean, count }`

**3. `packages/server-rust/benches/load_harness/connection_pool.rs`**
- `ConnectionPool` struct:
  - `async fn new(addr: SocketAddr, pool_size: usize, jwt_secret: &str) -> Result<Self>`
  - Creates `pool_size` tokio-tungstenite WebSocket connections
  - Each connection authenticates with a generated JWT (using `jsonwebtoken` crate)
  - Connections stored as `Vec<SplitSink + SplitStream>` pairs wrapped in `Arc<Mutex<>>`
  - `async fn send_to(&self, conn_idx: usize, msg: &[u8]) -> Result<()>`
  - `async fn broadcast(&self, msg: &[u8]) -> Result<()>`
  - `async fn recv_from(&self, conn_idx: usize) -> Result<Vec<u8>>`
  - `async fn close_all(&self)`
- Connection lifecycle: connect -> receive AUTH_REQUIRED -> send AUTH -> receive AUTH_ACK
- Connections opened in batches of 500 with 10ms delay between batches to avoid SYN flood

**4. `packages/server-rust/benches/load_harness/metrics.rs`**
- `HdrMetricsCollector` implementing `MetricsCollector`:
  - Uses `hdrhistogram::Histogram<u64>` per operation name
  - Thread-safe via `DashMap<String, Mutex<Histogram<u64>>>`
  - `record_latency()` records value in microseconds
  - `snapshot()` reads percentiles from each histogram
  - `print_report()` formats ASCII table of all operations with p50/p95/p99/p99.9/max
- Counters stored in `DashMap<String, AtomicU64>`

**5. `packages/server-rust/benches/load_harness/scenarios/throughput.rs`**
- `ThroughputScenario` implementing `LoadScenario`:
  - Config: `num_connections: usize` (default 1000), `duration_secs: u64` (default 30), `batch_size: usize` (default 10), `send_interval_ms: u64` (default 50)
  - `run()`: each connection sends PUT batches at `send_interval_ms` intervals for `duration_secs`
  - Records write latency (send -> OP_ACK) per batch
  - Records total ops/sec
  - Equivalent to k6 `bench-throughput.js` but at 1000+ connections
- Assertion: `ThroughputAssertion` -- acked ops > 80% of sent ops, p99 < 500ms

### Cargo.toml Changes

Add to `packages/server-rust/Cargo.toml`:

```toml
[dev-dependencies]
hdrhistogram = "7"

[[bench]]
name = "load_harness"
path = "benches/load_harness/main.rs"
harness = false
```

### No Changes To

- Existing k6 tests (kept as-is for CI)
- Server production code
- `test_server.rs` binary (harness starts server in-process by calling `build_services()` directly)

## Acceptance Criteria

1. `cargo bench --bench load_harness -- --scenario throughput` completes without error on a single machine
2. HDR histogram output shows p50, p95, p99, p99.9, and max latencies for write operations
3. 1000 concurrent WebSocket connections all authenticate successfully (connection pool setup completes)
4. Throughput scenario reports ops/sec metric consistent with k6 bench-throughput results (same order of magnitude)
5. `ThroughputAssertion` passes: acked ops > 80% of sent ops
6. All existing `cargo test` tests continue to pass (no production code changes)

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo bench --bench load_harness -- --scenario throughput` -- completes, prints HDR histogram report
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all existing tests pass
- Verify output contains lines matching `p50:`, `p95:`, `p99:`, `p99.9:`, `ops/sec:`
- Verify connection pool opens 1000 connections without timeout (< 30s setup time)

## Constraints

- Do NOT modify any server production code (src/)
- Do NOT remove or modify existing k6 tests
- Do NOT use Criterion for this bench (custom harness with `harness = false`) -- Criterion's iteration model does not fit long-running load scenarios
- MsgPack encoding MUST use `rmp_serde::to_vec_named()` to match server wire format
- JWT tokens MUST use HS256 with secret `test-e2e-secret` and include `sub` claim (standard JWT, per project convention)
- Connection pool MUST handle server backpressure (429 responses) gracefully -- log and retry, do not panic

## Assumptions

- The harness runs on the same machine as the server (localhost connections, no network simulation needed for throughput scenario)
- 1000 connections is the default for the initial throughput scenario; 10k+ connections is a stretch goal for future scenarios (connection-storm scenario in a follow-up spec)
- The in-process server uses `NullDataStore` (no PostgreSQL) matching `test_server.rs` behavior
- `hdrhistogram` crate version 7.x is used (latest stable, well-maintained)
- Split-brain simulation and Merkle sync timing scenarios are deferred to follow-up specs after this foundation is established
- The `build_services()` function from `test_server.rs` will be extracted into a shared test utility (or the harness duplicates it) -- preference is to extract into a `test_support` module

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define traits: `LoadScenario`, `Assertion`, `MetricsCollector`, `HarnessContext`, `ScenarioResult`, `AssertionResult`, `MetricsSnapshot` in `traits.rs` | -- | ~15% |
| G2 | 2 | Implement `HdrMetricsCollector` in `metrics.rs` using hdrhistogram + DashMap | G1 | ~20% |
| G3 | 2 | Implement `ConnectionPool` in `connection_pool.rs` with batched connect + auth | G1 | ~25% |
| G4 | 3 | Implement `ThroughputScenario` + `ThroughputAssertion` in `scenarios/throughput.rs` | G1, G2, G3 | ~25% |
| G5 | 4 | Wire `main.rs` entry point: start in-process server, parse CLI args, run scenario, print report | G1, G2, G3, G4 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |
| 4 | G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)
