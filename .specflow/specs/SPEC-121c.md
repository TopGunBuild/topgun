---
id: SPEC-121c
type: feature
status: draft
priority: P2
complexity: medium
created: 2026-03-17
source: TODO-114
parent: SPEC-121
depends_on: [SPEC-121a, SPEC-121b]
---

# Throughput Scenario and Harness Main

## Context

This is the final sub-specification for the Rust-native load testing harness (SPEC-121). It depends on traits and metrics from SPEC-121a and the connection pool from SPEC-121b.

This spec creates the throughput benchmark scenario (equivalent to k6 `bench-throughput.js`) and the `main.rs` entry point that starts an in-process test server, runs scenarios, and prints HDR histogram reports.

**Key Links:**
- `build_services()` from `test_server.rs` wires all 7 domain services with `NullDataStore`
- The harness reuses this setup to start an in-process server (no external binary)
- Throughput scenario sends PUT operation batches via MsgPack over WebSocket
- HDR histogram report is printed after scenario completion using `HdrMetricsCollector::print_report()`

## Task

Implement the throughput benchmark scenario and wire the harness entry point:

1. Create `ThroughputScenario` implementing `LoadScenario` -- sends PUT batches at configurable intervals
2. Create `ThroughputAssertion` implementing `Assertion` -- validates acked ops > 80% of sent
3. Create `main.rs` entry point that starts in-process server, parses CLI args, runs scenarios, prints reports

## Requirements

### New Files

**1. `packages/server-rust/benches/load_harness/scenarios/throughput.rs`**
- `ThroughputScenario` implementing `LoadScenario`:
  - `ThroughputConfig` struct: `num_connections: usize` (default 1000), `duration_secs: u64` (default 30), `batch_size: usize` (default 10), `send_interval_ms: u64` (default 50)
  - `setup()`: creates `ConnectionPool` via SPEC-121b with `num_connections` connections
  - `run()`:
    - Spawns one tokio task per connection
    - Each task loops for `duration_secs`: build a PUT batch of `batch_size` operations, send via MsgPack, await OP_ACK, record write latency (send-to-ack) in metrics collector
    - PUT operations target map `"bench"` with keys `"conn-{idx}-{seq}"` and value `{ "v": seq_number }`
    - Sleep `send_interval_ms` between batches
    - Track total sent ops and acked ops in `ScenarioResult`
  - `assertions()`: returns `vec![Box::new(ThroughputAssertion)]`
- `ThroughputAssertion` implementing `Assertion`:
  - `check()`: acked ops > 80% of sent ops AND p99 < 500ms (500_000 microseconds)
  - Returns `AssertionResult::Fail` with descriptive message if either condition fails

**2. `packages/server-rust/benches/load_harness/main.rs`**
- Custom bench harness entry point (`fn main()`)
- Uses `#[tokio::main]` for async runtime
- CLI argument parsing (basic `std::env::args`):
  - `--scenario <name>` -- which scenario to run (default: "throughput")
  - `--connections <n>` -- override connection count (default: 1000)
  - `--duration <secs>` -- override duration (default: 30)
- Server startup:
  - Calls `build_services()` (extracted from or duplicating `test_server.rs` logic)
  - Builds `AppState` and `axum::Router`
  - Binds to `127.0.0.1:0`, captures assigned port
  - Spawns server as background tokio task
- Scenario execution:
  - Creates `HdrMetricsCollector` (from SPEC-121a)
  - Creates `HarnessContext` with server addr, JWT secret, metrics collector
  - Looks up scenario by name, calls `setup()` then `run()`
  - Runs all assertions from `scenario.assertions()`
  - Calls `metrics.print_report()` to output HDR histogram summary
  - Prints `ops/sec:` line with total throughput
  - Prints assertion results (PASS/FAIL per assertion)
  - Exits with code 1 if any assertion fails
- Module declarations: `mod traits; mod metrics; mod connection_pool; mod scenarios;`
- `scenarios/mod.rs`: re-exports `ThroughputScenario`

### No Changes To

- Existing k6 tests
- Server production code (src/)
- `test_server.rs` binary (harness duplicates `build_services()` or extracts to shared module)
- Files created in SPEC-121a (traits.rs, metrics.rs)
- Files created in SPEC-121b (connection_pool.rs)

## Acceptance Criteria

1. `cargo bench --bench load_harness -- --scenario throughput` completes without error on a single machine
2. HDR histogram output shows p50, p95, p99, p99.9, and max latencies for write operations
3. 1000 concurrent WebSocket connections all authenticate successfully (connection pool setup completes)
4. Throughput scenario reports ops/sec metric consistent with k6 bench-throughput results (same order of magnitude)
5. `ThroughputAssertion` passes: acked ops > 80% of sent ops
6. All existing `cargo test` tests continue to pass (no production code changes)
7. Exit code is 0 when all assertions pass, 1 when any fail

## Validation Checklist

- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo bench --bench load_harness -- --scenario throughput` -- completes, prints HDR histogram report
- Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all existing tests pass
- Verify output contains lines matching `p50:`, `p95:`, `p99:`, `p99.9:`, `ops/sec:`
- Verify connection pool opens 1000 connections without timeout (< 30s setup time)
- Verify assertion output shows `PASS` or `FAIL` with description

## Constraints

- Do NOT modify any server production code (src/)
- Do NOT remove or modify existing k6 tests
- Do NOT use Criterion (`harness = false` custom bench)
- MsgPack encoding MUST use `rmp_serde::to_vec_named()` to match server wire format
- JWT tokens MUST use HS256 with secret `test-e2e-secret` and include `sub` claim
- In-process server MUST use `NullDataStore` (no PostgreSQL dependency)

## Assumptions

- `build_services()` logic can be duplicated from `test_server.rs` into `main.rs` (or extracted to a shared `test_support` module if the auditor recommends it)
- The throughput scenario targets map `"bench"` which does not need pre-creation (server creates maps on first write)
- OP_ACK messages from the server contain enough information to match against sent operations for latency measurement
- 30 seconds is sufficient duration to produce statistically meaningful histogram data at 1000 connections

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Implement `ThroughputScenario` + `ThroughputConfig` + `ThroughputAssertion` in `scenarios/throughput.rs` and `scenarios/mod.rs` | -- | ~40% |
| G2 | 1 | Wire `main.rs`: module declarations, CLI arg parsing, in-process server startup (duplicate `build_services()`) | -- | ~35% |
| G3 | 2 | Integrate scenario runner in `main.rs`: create context, run scenario, execute assertions, print report, set exit code | G1, G2 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)
