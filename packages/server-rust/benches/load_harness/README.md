# Load Harness

In-process load testing harness for TopGun's Rust server. It boots a full server instance (all 7 domain services, partition dispatcher, WebSocket handler) inside the same process, opens N WebSocket connections against it, and runs configurable scenarios while recording latency with HDR histograms. Results are printed as an ASCII table and optionally written as machine-readable JSON for CI consumption.

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--connections` | usize | 200 | Number of concurrent WebSocket connections |
| `--duration` | u64 | 30 | Test duration in seconds |
| `--interval` | u64 | 50 | Milliseconds between batch sends per connection |
| `--fire-and-forget` | bool (flag) | false | Send without waiting for OP_ACK |
| `--scenario` | string | `throughput` | Scenario to run |
| `--json-output` | path | none | Write machine-readable JSON report to this path |

## Scenarios

### `throughput`

Sends `OpBatch` PUT operations over N WebSocket connections for D seconds.

- **Batch size:** 10 operations per batch (hardcoded default in `ThroughputConfig`)
- **Fire-and-wait mode** (default): Sends a batch, waits for the server's `OP_ACK` response (5s timeout), records the round-trip latency in the HDR histogram under `write_latency`, then sleeps for `--interval` ms before sending the next batch.
- **Fire-and-forget mode** (`--fire-and-forget`): Sends batches as fast as possible without waiting for ACKs. Measures raw push throughput into the server pipeline. No latency histogram is recorded.

## Interpreting Results

After the scenario completes, the harness prints an ASCII histogram table:

```
operation                           count     p50 µs     p95 µs     p99 µs   p99.9 µs     max µs
--------------------------------------------------------------------------------------------
write_latency                        5000       1200       3500       8000      15000      25000
```

All latency values are in **microseconds (µs)**.

**ops/sec** is calculated as `total_ops / duration_secs` and printed below the table.

**Assertions:** After metrics are printed, the `ThroughputAssertion` runs two checks:

1. **Acked ratio >= 80%** — At least 80% of sent operations must receive an `OP_ACK` within the test duration. If fewer than 80% are acknowledged, the assertion reports `FAIL` with the actual ratio.
2. **p99 latency < 500ms** — The 99th percentile write latency must be under 500,000 µs (500ms). If p99 equals or exceeds this threshold, the assertion reports `FAIL` with the actual value.

Both conditions must pass for the harness to exit with code 0. If either fails, the harness exits with code 1.

```
PASS [throughput_assertion]
```

or:

```
FAIL [throughput_assertion]: acked ratio 0.72 < 0.80
```

## Baseline Numbers

Observed performance on reference hardware (in-process, no network hop):

| Mode | Connections | Approximate ops/sec |
|------|-------------|---------------------|
| Fire-and-forget | 200 | ~200k |
| Fire-and-wait | 200 | ~2.8k |

These numbers are from in-process testing where client and server share the same machine. Production deployments with a real network hop will see different results.

## CI Integration

The perf-gate job in `.github/workflows/rust.yml` runs the harness automatically on every push.

### `--json-output`

When provided, the harness writes a JSON report to the given path:

```json
{
  "scenario": "throughput",
  "mode": "fire-and-wait",
  "connections": 200,
  "duration_secs": 15,
  "total_ops": 42000,
  "ops_per_sec": 2800,
  "latency": {
    "p50_us": 1200,
    "p95_us": 3500,
    "p99_us": 8000,
    "p999_us": 15000
  },
  "assertions": [
    { "name": "throughput_assertion", "passed": true, "message": null }
  ],
  "timestamp": "2026-03-18T12:00:00Z"
}
```

### `baseline.json`

Located at `packages/server-rust/benches/load_harness/baseline.json`, it defines CI thresholds:

```json
{
  "fire_and_wait": {
    "min_ops_per_sec": 1000,
    "max_p50_us": 1000000
  },
  "fire_and_forget": {
    "min_ops_per_sec": 50000,
    "max_p50_us": 1000000
  },
  "regression_tolerance_pct": 20
}
```

The CI job runs both fire-and-wait and fire-and-forget scenarios, then uses `jq` to compare each result's `ops_per_sec` and `p50_us` against these thresholds. A 20% regression tolerance is applied. The perf-gate is currently informational (`continue-on-error: true`) and does not block merges.

## Adding New Scenarios

1. **Create a new file** in `scenarios/` (e.g., `scenarios/latency.rs`)

2. **Implement the `LoadScenario` trait** (`traits.rs`):
   ```rust
   #[async_trait]
   impl LoadScenario for LatencyScenario {
       fn name(&self) -> &str { "latency" }
       async fn setup(&self, ctx: &HarnessContext) -> Result<()> { /* ... */ }
       async fn run(&self, ctx: &HarnessContext) -> ScenarioResult { /* ... */ }
       fn assertions(&self) -> Vec<Box<dyn Assertion>> { /* ... */ }
   }
   ```

3. **Implement the `Assertion` trait** for post-run validation:
   ```rust
   #[async_trait]
   impl Assertion for LatencyAssertion {
       fn name(&self) -> &str { "latency_assertion" }
       async fn check(&self, ctx: &HarnessContext, result: &ScenarioResult) -> AssertionResult { /* ... */ }
   }
   ```

4. **Register in `scenarios/mod.rs`**:
   ```rust
   pub mod latency;
   pub use latency::LatencyScenario;
   ```

5. **Add a match arm** in `main.rs` scenario dispatch:
   ```rust
   "latency" => Box::new(LatencyScenario::new(config)),
   ```

6. **Add baseline thresholds** to `baseline.json` if the scenario should be CI-gated.

## Example Commands

```bash
# Default: 200 connections, 30s, fire-and-wait
cargo bench --bench load_harness

# Quick smoke test: fewer connections, shorter duration
cargo bench --bench load_harness -- --connections 50 --duration 10

# Fire-and-forget throughput test
cargo bench --bench load_harness -- --fire-and-forget --interval 0

# Full CI-style run with JSON output
cargo bench --bench load_harness -- --duration 15 --json-output results.json

# High connection count stress test
cargo bench --bench load_harness -- --connections 1000 --duration 60 --interval 10
```
