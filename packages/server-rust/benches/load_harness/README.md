# Load Harness

In-process load testing harness for TopGun's Rust server. It boots a full server instance (all 7 domain services, partition dispatcher, WebSocket handler) inside the same process, opens N WebSocket connections against it, and runs configurable scenarios while recording latency with HDR histograms. Results are printed as an ASCII table and optionally written as machine-readable JSON for CI consumption.

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--connections` | usize | 200 | Number of concurrent WebSocket connections |
| `--duration` | u64 | 30 | Test duration in seconds |
| `--interval` | u64 | 50 | Milliseconds between batch sends per connection |
| `--fire-and-forget` | bool (flag) | false | Send without waiting for OP_ACK |
| `--scenario` | string | `throughput` | Scenario to run (`throughput` or `vector_search`) |
| `--json-output` | path | none | Write machine-readable JSON report to this path |
| `--vector-mode` | string | `build` | Vector sub-mode: `build`, `query`, `optimize`, or `hybrid` |
| `--vector-count` | usize | 10000 | Number of vectors to insert/query in vector scenarios |
| `--vector-dim` | u16 | 384 | Embedding dimensionality (384 = sentence-transformer default) |
| `--vector-k` | usize | 10 | Top-k nearest neighbours for query mode |
| `--vector-query-tasks` | usize | num_cpus | Concurrent query tasks in query mode (printed at run start) |

## Scenarios

| Scenario name | What it measures |
|--------------|-----------------|
| `throughput` | Full request path: WebSocket → msgpack → partition dispatcher → CRDT merge |
| `vector_search` | HNSW/hybrid-search hot path: directly calls `VectorIndex` and `HybridSearchEngine`, bypassing the WebSocket layer |

### When to use which scenario

| Goal | Scenario |
|------|---------|
| Measuring full request path (ws → dispatch → CRDT) | `throughput` |
| Measuring HNSW/hybrid-search hot path | `vector_search` |

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

## Vector Search Scenario

The `vector_search` scenario benchmarks the HNSW/hybrid-search subsystem directly — bypassing the WebSocket path entirely. It constructs an `Arc<VectorIndex>` (and a minimal `HybridSearchEngine` for Hybrid mode) inline, measures the hot path you actually care about, and records HDR histogram latencies under mode-specific keys.

### When to use which scenario

| Goal | Scenario |
|------|---------|
| Measuring full request path (ws → dispatch → CRDT) | `throughput` |
| Measuring HNSW/hybrid-search hot path | `vector_search` |

### Sub-modes

| Mode | What it measures | Primary latency key | ops = |
|------|-----------------|---------------------|-------|
| `build` | Per-batch commit latency for inserting vectors into HNSW | `vector_build_latency` | batch count (batch_size=1000) |
| `query` | ANN `search_nearest` latency under concurrent readers | `vector_query_latency` | total queries executed |
| `optimize` | Single full graph-rebuild cycle | `vector_optimize_latency` | vector_count (vectors rebuilt) |
| `hybrid` | End-to-end RRF fusion (Exact+FullText+Semantic pipeline) | `hybrid_search_latency` | total queries executed |

**Note:** Build and Optimize modes run to completion (not time-bounded). Query and Hybrid modes run for `--duration` seconds.

**Note:** `vector_optimize_latency` will always have `count = 1` because a single optimize cycle = a single wall-clock sample by construction.

**Note:** `optimize_progress_interval_us` is also recorded during Optimize mode — it captures the actual time between polling ticks (~10ms sleeps), providing visibility into rebuild smoothness and scheduling jitter.

### Interpreting HDR histogram output for vector modes

```
operation                           count     p50 µs     p95 µs     p99 µs   p99.9 µs     max µs
--------------------------------------------------------------------------------------------
vector_query_latency                94700        873       2257       3743       8055      36511
hybrid_search_latency               27337        357        449        509        597        724
```

All values are in **microseconds (µs)**. For `vector_optimize_latency`, the single sample represents the total rebuild wall-clock time (for 10K dim-384 vectors on M1 Max: ~19s).

### Corpus

- **Vectors:** `f32` arrays of length `--vector-dim`, generated by a seeded `SmallRng` (seed `0xBEEF` for indexed vectors, `0xCAFE` for query vectors) and L2-normalised for meaningful cosine distances. Reproducible across runs.
- **Documents (Hybrid mode):** `"doc N about topic {word}"` where `{word}` is drawn from the fixed 8-word corpus `[alpha, beta, gamma, delta, epsilon, zeta, eta, theta]` using the same seeded RNG. Keeps Tantivy index build time stable.

### Example commands

```bash
# Build mode: insert 10K dim-384 vectors, print per-batch commit latency
cargo bench --bench load_harness -- --scenario vector_search --vector-mode build --vector-count 10000 --vector-dim 384

# Query mode: pre-populate 10K vectors, run k=10 ANN queries for 10s
cargo bench --bench load_harness -- --scenario vector_search --vector-mode query --vector-count 10000 --vector-dim 384 --vector-k 10 --duration 10

# Query mode: same but k=100 (more expensive; compare p50 with k=10)
cargo bench --bench load_harness -- --scenario vector_search --vector-mode query --vector-count 10000 --vector-dim 384 --vector-k 100 --duration 10

# Optimize mode: rebuild HNSW graph from 10K committed vectors, measure total cycle time
cargo bench --bench load_harness -- --scenario vector_search --vector-mode optimize --vector-count 10000 --vector-dim 384

# Hybrid mode: run Exact+FullText+Semantic+RRF fusion for 10s
cargo bench --bench load_harness -- --scenario vector_search --vector-mode hybrid --vector-count 10000 --vector-dim 384 --duration 10

# With JSON output (mode field = "vector-query")
cargo bench --bench load_harness -- --scenario vector_search --vector-mode query --vector-count 10000 --duration 10 --json-output /tmp/vec-query.json
```

### Baseline interpretation

`baseline.json` contains a `vector_search` block with per-mode `max_p50_us` (and `max_p99_us` for build/optimize) thresholds. Thresholds are set to `measured * 1.20` (20% headroom, consistent with `regression_tolerance_pct`). The `VectorSearchAssertion` reads the embedded baseline.json at runtime and emits `Pass` if measured p50 ≤ threshold, `Fail` otherwise.

On first ever run (placeholder zeros in baseline), the assertion emits `[no baseline for vector-{mode}; recording first measurement]` and returns `Pass` — allowing the initial commit of measured values.

## Baseline Numbers

Observed performance on reference hardware (Apple M1 Max, in-process, no network hop):

| Mode | Connections | Approximate ops/sec | Median latency |
|------|-------------|---------------------|----------------|
| Fire-and-forget | 200 | ~560k | N/A (no ACK) |
| Fire-and-wait | 200 | ~37k | ~1.5ms |

These numbers are from in-process testing where client and server share the same machine (2026-03-27 benchmark). Fire-and-forget throughput is OS-limited (macOS socket buffers); actual server capacity is higher. Production deployments with a real network hop will see different results.

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
    "min_ops_per_sec": 30000,
    "max_p50_us": 5000
  },
  "fire_and_forget": {
    "min_ops_per_sec": 400000,
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
