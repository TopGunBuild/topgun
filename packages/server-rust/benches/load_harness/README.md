# Load Harness

In-process load testing harness for TopGun's Rust server. It boots a full server instance (all 7 domain services, partition dispatcher, WebSocket handler) inside the same process, opens N WebSocket connections against it, and runs configurable scenarios while recording latency with HDR histograms. Results are printed as an ASCII table and optionally written as machine-readable JSON for CI consumption.

## CLI Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--connections` | usize | 200 | Number of concurrent WebSocket connections |
| `--duration` | u64 | 30 | Test duration in seconds |
| `--interval` | u64 | 50 | Milliseconds between batch sends per connection |
| `--fire-and-forget` | bool (flag) | false | Send without waiting for OP_ACK |
| `--scenario` | string | `throughput` | Scenario to run (`throughput`, `open_loop` or `vector_search`) |
| `--json-output` | path | none | Write machine-readable JSON report to this path |
| `--vector-mode` | string | `build` | Vector sub-mode: `build`, `query`, `optimize`, or `hybrid` |
| `--vector-count` | usize | 10000 | Number of vectors to insert/query in vector scenarios |
| `--vector-dim` | u16 | 384 | Embedding dimensionality (384 = sentence-transformer default) |
| `--vector-k` | usize | 10 | Top-k nearest neighbours for query mode |
| `--vector-query-tasks` | usize | num_cpus | Concurrent query tasks in query mode (printed at run start) |
| `--store` | string | `null` | Store arm the server runs on: `null` or `redb` (see [Store arms](#store-arms)) |
| `--data-dir` | path | OS temp dir | Root under which a durable arm creates its fresh run directory |
| `--rate` | u64 | 10000 | `open_loop`: aggregate target writes/s |
| `--batch-size` | usize | 10 | `open_loop`: ops per `OP_BATCH` |
| `--warmup` | u64 | 10 | `open_loop`: seconds scheduled before the measurement window |
| `--new-key-fraction` | f64 | 1.0 | `open_loop`: share of writes to never-written keys, within [0, 1] |
| `--hot-keys` | u64 | 10000 | `open_loop`: size of the shared rewrite key set (> 0 when the fraction is < 1) |
| `--value-bytes` | usize | 0 | `open_loop`: extra binary payload per value |
| `--seed` | u64 | 1 | `open_loop`: key-choice seed (each connection uses `seed ^ connection`) |
| `--sample-ms` | u64 | 1000 | `open_loop`: timeline interval |
| `--ack-timeout-ms` | u64 | 10000 | `open_loop`: wait for a batch's answer before abandoning its connection |
| `--max-wake-lag-ms` | u64 | 5 | `open_loop`: generator-validity bound on wake lag p99.9 |
| `--arms` | list | the `--store` value | `open_loop` series: comma-separated arms, e.g. `null,redb` |
| `--repeat` | u64 | 1 | `open_loop` series: repetitions of the arm list |

## Scenarios

| Scenario name | What it measures |
|--------------|-----------------|
| `throughput` | Full request path: WebSocket → msgpack → partition dispatcher → CRDT merge |
| `open_loop` | The same path at a fixed offered rate, coordinated-omission-corrected latency (see [Open loop](#open-loop)) |
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

## Store arms

`--store` selects the data store the in-process server runs on.

- **`null`** (default) — `NullDataStore`. Nothing is persisted: no WAL, no write-behind buffer, no durable engine, no durable Merkle index. The eviction orchestrator runs over the in-memory record stores, as in the binary. The figure it gives (the ~37k acked writes/s in [Baseline Numbers](#baseline-numbers)) answers "what do network, dispatcher, CRDT merge and observers cost", not "what does a node sustain". CI runs only this arm.
- **`redb`** — the path the shipped binary runs on every write: the WAL (`batched` fsync by default), the write-behind buffer, the embedded redb engine (`EngineMapDataStore<RedbKvEngine>`), the eviction orchestrator and the durable Merkle index. The WAL bootstrap (open, recovery, sequence seed) is the same library function the binary calls (`WalBootstrap::open_over_store`).

**Environment.** A durable arm reads the binary's variables with the binary's parse rules, fatal ones included: `TOPGUN_WAL_FSYNC_POLICY`, `TOPGUN_WRITEBEHIND_CAPACITY`, `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS`, `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`, `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS`, `TOPGUN_OR_DELTA_WAL`, `TOPGUN_MAX_RAM_MB`, `TOPGUN_EVICTION_HIGH_PCT`, `TOPGUN_EVICTION_LOW_PCT`, `TOPGUN_EVICTION_INTERVAL_MS`. Every arm reads `TOPGUN_EXPIRY_SWEEP_INTERVAL_MS`, `TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC` and `TOPGUN_JOURNAL_ENABLED`, as before. All of them are parsed before anything is created, so a bad value exits with no directory left behind. The harness places the store itself and ignores `TOPGUN_WAL_DIR`, `TOPGUN_DATA_DIR`, `TOPGUN_REDB_PATH` and `STORAGE_BACKEND`; when any of them is set it prints one stderr line naming them.

**Run directory.** A durable arm creates a fresh `topgun-harness-*` directory under `--data-dir` (default: the OS temp dir; the root must exist), with `data/` and `wal/` inside, and removes it at the end of the run whatever the exit code.

**Encryption is off.** The harness measures encryption at rest off. `TOPGUN_ENCRYPTION_AT_REST` set to anything but `off` exits 1 with `the harness measures encryption off; unset TOPGUN_ENCRYPTION_AT_REST`, on every arm.

**Closed loop on a durable arm.** `--scenario throughput --store redb` prints one extra line `store: redb` and adds `store` and `config` (the effective configuration, below) to the throughput JSON; its throughput assertion is unchanged.

## Open loop

`--scenario open_loop` offers writes at a fixed aggregate rate, independent of how fast the server answers.

**Schedule.** With rate `R`, batch size `B` and `C` connections, connection `i` sends its `k`-th batch at `t0 + i·(B/R) + k·(C·B/R)`, so the aggregate schedule is uniform. The schedule never skips: a connection behind schedule sends its next due batch at once until it has caught up. Batches scheduled in the first `--warmup` seconds are sent but excluded from every statistic; the measurement window is the next `--duration` seconds, by **scheduled** instant. After the window nothing new is scheduled; window batches not yet sent keep being sent until the window end plus `--ack-timeout-ms`.

**Coordinated-omission correction.** `ack_latency_us` starts at the instant from which a batch waited because of the server. A batch the connection reached **after** its scheduled instant (its previous answer came late) is measured from the scheduled instant, so a stalled server raises the reported latency instead of lowering the offered load. A batch reached **before** its instant is measured from its actual send: the generator sleeps until the instant and tokio rounds every sleep up to the next millisecond, and that timer lag is the generator's, not the server's — it goes to `generator.wake_lag_us` and to the `generator_saturated` rule instead. Lateness caused by a starved generator rather than by the server is charged to the server: the reported figure errs on the worse side, never the better. `service_latency_us` (from the actual send, every batch) is diagnostic.

**One batch in flight per connection.** The server's `ERROR` frame names no operation, and answers to several in-flight batches on one connection can arrive in any order; with one outstanding batch every frame answers that batch, so refusals are counted exactly. The consequence: a connection whose answer arrives after its next scheduled instant sends late; that lateness is inside `ack_latency_us` and is reported, for window batches, as `generator.behind_schedule_sends` / `max_behind_us`. Size `--connections` so that `C·B/R` is well above the expected p99.9 — e.g. `--connections 1000` at `--rate 20000`, `--batch-size 10` gives each connection 500 ms per batch. The server also limits each connection's data plane to 20 000 ops/s with a burst of 40 000 (`network/config.rs`); if `rate / connections` exceeds it, the best-effort `ERROR 429` may be dropped, the batch counts as unanswered and its connection is abandoned — a generator-setup error, not a server result. Keep `rate / connections` well under that limit.

**Refusal accounting.** Per batch, frames are read until a terminal one: each `OP_REJECTED` refuses its op permanently (counted under its `code`); `OP_ACK` without `results` acks every op not already refused; `OP_ACK` with `results` acks exactly the listed ops; `ERROR {code}` refuses transiently every op not already refused by an `OP_REJECTED`; a batch whose every op is rejected ends at the last `OP_REJECTED`. No answer within `--ack-timeout-ms` makes the batch's ops `unanswered` and abandons the connection (its later batches count as `unsent`). **The harness never retries:** a refused write is not re-sent and never counted as acked, so the offered load stays fixed. `refused_transient` means **"answered by `ERROR`, not acknowledged"**, whatever the reason — the `ERROR` frame carries a transient failure (the write-behind capacity refusal is code `500`, overload is `429`) and also a non-attributed permanent batch error, and the server sends it even when some ops of the batch were applied. It is therefore **not** "not applied": write-behind depth and `store_bytes` may include writes counted as `refused_transient`. `refusals.transient_by_code` tells the kinds apart.

The identities `scheduled = sent + unsent` and `sent = acked + refused_permanent + refused_transient + unanswered` (ops, window batches only) are checked before the report is written; a violation is a harness bug and exits 1.

**Key mix.** With probability `--new-key-fraction` an op writes a never-written key `ol-{connection}-{id}`, else a key `hot-{j}`, `j` uniform in `0..--hot-keys`, shared by all connections. It matters because the write-behind capacity (`TOPGUN_WRITEBEHIND_CAPACITY`) is a wall for **new** keys only: a write of a key already buffered is always accepted. `--value-bytes` adds a binary field of that size to every value.

**Teardown drain.** After the measurement window the server stops serving, stops the expiry sweep and eviction, and drains the write-behind buffer with `hard_flush` — exactly as the binary's signal stop, with no outer timeout (the drain is bounded by the store's own `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`). The outcome is the report's `teardown` object `{ drained, drain_ms, pending_after, error }`: `drained` is `hard_flush` returned `Ok` and `pending_after == 0`, the binary's own definition; all four are `null` for the `null` arm. A drain that did not finish prints a stderr warning but neither invalidates the run nor changes the exit code: the figures were taken before it.

**Validity.** `validity.valid` is true when `validity.reasons` is empty. The reasons, each a fixed string:

| Reason | When |
|---|---|
| `generator_saturated` | wake-lag p99.9 (µs) > `--max-wake-lag-ms` × 1000: the client could not keep its own schedule while it was free, so the load reached the server later and less evenly than scheduled (with `--max-wake-lag-ms 0` any positive lag trips it) |
| `load_not_offered` | `ops.unsent > 0`: part of the scheduled load never reached the server |
| `unanswered_batches` | `ops.unanswered > 0`: the latency tail is censored |
| `store_failed` | the durable store's failure watch holds a failure at the end of the run (text in `validity.store_failure`) |
| `debug_build` | the harness was built with debug assertions |

Refusals do not invalidate a run: they are a result. Nothing here is a threshold on server performance, and an invalid run still writes its report and exits 0.

**Report (`--json-output`), `schema_version: 1`:**

```
{
  schema_version: 1,
  scenario: "open_loop", store, run_index (null outside a series), timestamp,
  target_ops_per_sec, batch_size, connections, warmup_secs, duration_secs,
  key_mix: { new_key_fraction, hot_keys, value_bytes, seed },
  achieved: { acked_ops_per_sec, sent_ops_per_sec },
  ops: { scheduled, sent, unsent, acked, refused_permanent, refused_transient, unanswered },
  refusals: { transient_by_code: {"<code>": n}, permanent_by_code: {"<code>": n},
              sample_messages: {"<code>": [up to 5 distinct ERROR messages]} },
  ack_latency_us: { count, p50, p90, p99, p999, max, mean },      // CO-corrected: from the scheduled instant when sent late, else from the send
  service_latency_us: { count, p50, p90, p99, p999, max, mean },  // from the actual send
  generator: { wake_lag_us: { count, p50, p99, p999, max }, behind_schedule_sends, max_behind_us,
               connections_abandoned, protocol_anomalies, max_wake_lag_ms },
  write_behind: { depth_max, depth_final, sample_ms, warmup_end_ms },
  timeline: [ { t_ms, acked_ops, refused_ops, depth, wal_bytes, store_bytes } ],
  config: <effective configuration, null for the null arm>,
  journal_enabled, expiry_sweep_interval_ms, expiry_sweep_rows_per_sec,
  host: { os, arch, cpus, kernel, cpu_model },
  build: { git_commit, git_dirty, binary_sha256, debug_assertions, allocator },
  teardown: { drained, drain_ms, pending_after, error },
  validity: { valid, reasons: [..], store_failure }
}
```

- `timeline` has `floor((warmup + duration) · 1000 / sample_ms)` entries, one per tick from `t0`, warm-up included (`warmup_end_ms` marks its end); the sampler stops at the window end. `acked_ops` / `refused_ops` are the ops answered in that interval (wall clock); `depth` is the write-behind buffered-key count (`pending_operation_count`, the number the capacity check uses, 0 on `null`); `wal_bytes` / `store_bytes` are the total file sizes under the run's `wal/` and `data/` (`null` on `null`).
- `config` (non-null arms; also in the closed-loop JSON of a non-null arm): `{ store, data_dir, wal_fsync_policy ("per_op" | "batched" | "none"), write_delay_ms, flush_interval_ms, capacity, backoff_base_ms, backoff_cap_ms, shutdown_timeout_ms, wal_watermark_stall_bound_ms, or_delta_wal, eviction: { max_ram_mb, high_pct, low_pct, interval_ms }, encryption_at_rest: "off", durable_merkle_index: true }`.
- `build.binary_sha256` is the SHA-256 of the running harness binary. Release builds are not byte-reproducible, so this — not the commit — is the provenance of a figure. `git_commit` / `git_dirty` are `null` where `git` is unavailable.

Stdout prints a short summary: arm, target, achieved, ack p50/p99/p99.9/max, refusals, depth max, validity.

## A/A and ABAB series

```bash
# A/A: five runs of the same arm — the noise floor
cargo bench --bench load_harness -- --scenario open_loop --arms redb --repeat 5 \
  --rate 10000 --connections 500 --duration 600 --data-dir /mnt/nvme --json-output aa.json

# ABAB: null, redb, null, redb, null, redb — interleaved so host drift spreads over both arms
cargo bench --bench load_harness -- --scenario open_loop --arms null,redb --repeat 3 \
  --rate 10000 --connections 500 --duration 300 --json-output abab.json
```

Series mode is active when `--repeat > 1` or `--arms` names more than one arm, and requires `--scenario open_loop`. Every run is a separate child process of the same binary (fresh allocator, runtimes and process state), with the parent's flags plus `--store <arm> --run-index <i>`. A child that exits non-zero or writes no report is recorded as `{store, run_index, error}`, the series continues, and the parent exits 1 at the end (a harness error, not a performance verdict).

**Series report, `schema_version: 1`:**

```
{
  schema_version: 1,
  kind: "open_loop_series",
  order: ["redb", "redb", ...],
  runs: [ <open-loop report> | { store, run_index, error } ],
  spread: { "<arm>": { runs, acked_ops_per_sec, ack_p50_us, ack_p99_us, ack_p999_us, ack_max_us, depth_max } },
  valid
}
```

Each spread figure is `{ min, max, mean, relative_spread }` over the arm's runs that produced a report, with `relative_spread = (max − min) / mean` (`null` with fewer than two runs or a zero mean). `valid` is true only when every run produced a report and every report is valid. Stdout prints a per-arm spread table.

**Reading it.** The A/A `relative_spread` of a figure is the noise floor: a later before/after comparison on the same host must move that figure by more than it to mean anything. No threshold is derived here, and nothing in a series passes or fails on a measured figure; a blocking perf-gate built on an A/A series is future work (TODO-935).

## Known limits

- Client and server share the host's CPUs, and both runtimes default to `available_parallelism` workers. `generator_saturated` sees a starved generator only while it still reaches batches early enough to sleep; a generator so starved that it reaches every batch late leaves the wake-lag histogram empty, and that shows only as `load_not_offered` or as `generator.behind_schedule_sends` / `max_behind_us` — with one batch in flight it cannot be told apart from a slow server. Neither removes the shared-CPU effect.
- Loopback only, no TLS.
- The harness installs the binary's Prometheus recorder (`init_observability`), so every metric costs what it costs in production. At the end of each run it prints the recorder's `topgun_expiry_*` and `topgun_reclamation_*` lines to stdout; they are not in the JSON report.
- The server is the binary's single-node wiring, built by the same `topgun_server::bootstrap` functions. Differences that remain: the policy store is in memory for both arms (the binary keeps policies durable; with no policies configured both stores take the same per-operation path); a record's TTL is set by the generator on every record it sends (`--ttl-ms`), as a client asks for it; and a stop is reported as `teardown`, not as the binary's exit codes 70/71/72. A PUT-only LWW load creates no OR tombstones, so the prune task is wired but has no work.
- Every connection sends the client's `PING` every 5 s, as the shipped client does; without it the connection reaper closes connections after 60 s. The ping shares the connection's send lock with the batches, so a batch send can wait for one small `PING` write every 5 s.
- On macOS a WAL fsync is a full `F_FULLFSYNC` device barrier, so `redb` figures from a Mac are far slower on fsync-bound passes than on Linux and are not comparable to a Linux target host.
- Subscriber deliveries are not measured: every connection only writes.
- At 10 000 writes/s the server's eviction tick sets the `null` arm's tail. Every second the tick scans every resident record. In 3 runs of 60 s on an M1 Max, ack p99 / p99.9 averaged 31.2 / 71.9 ms as shipped and 5.5 / 12.1 ms with `TOPGUN_EVICTION_INTERVAL_MS=3600000`. This is a server cost, not a harness one, and it is open as TODO-941.

## Linux run on a ccx33-class host

Described here; nothing in CI or in this repository requires it. Target host: 8 dedicated vCPU, 32 GB RAM, local NVMe.

1. OS packages and toolchain: `apt-get install -y build-essential pkg-config git clang`, then rustup with the toolchain from `rust-toolchain.toml`.
2. Check out the commit under test: `git clone … && git checkout <sha>`.
3. Build: `cd packages/server-rust && cargo build --release --bench load_harness`.
4. Mount the NVMe and use it as `--data-dir` (e.g. `/mnt/nvme`); `ulimit -n 65536` (or higher) in the shell that runs the harness; set the CPU governor to `performance` where the host exposes one (`cpupower frequency-set -g performance`).
5. A/A series first, to measure the noise floor of this host:
   ```bash
   cargo bench --bench load_harness -- --scenario open_loop --arms redb --repeat 5 --duration 600 \
     --rate 10000 --connections 1000 --data-dir /mnt/nvme --json-output aa.json
   ```
6. Latency at the operating points:
   ```bash
   cargo bench --bench load_harness -- --scenario open_loop --store redb --rate 10000 --connections 1000 \
     --duration 600 --data-dir /mnt/nvme --json-output t2.json
   cargo bench --bench load_harness -- --scenario open_loop --store redb --rate 20000 --connections 1000 \
     --duration 600 --data-dir /mnt/nvme --json-output t3.json
   ```
7. Sustained rate (≥ 4 h), with the timeline used for the plateau check of depth, WAL bytes and store bytes:
   ```bash
   cargo bench --bench load_harness -- --scenario open_loop --store redb --rate 20000 --connections 1000 \
     --duration 14400 --data-dir /mnt/nvme --json-output t1.json
   ```
8. Locate the write-behind wall: repeat step 6 with `TOPGUN_WRITEBEHIND_CAPACITY` varied (e.g. `1000`, `10000`, `100000`) and with `--new-key-fraction` varied (`1.0`, `0.5`, `0.1`, with `--hot-keys` sized to the key set of interest).
9. Keep every JSON (it carries `binary_sha256`, the host and the effective configuration), and delete the rented server when the series is done.

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

### Unchanged by the store arms and the open loop

The perf-gate still runs only the `null` closed-loop modes against `baseline.json`; their JSON keys are unchanged (no `store` key is added for `null`). The open loop and the series are never run in CI.

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
