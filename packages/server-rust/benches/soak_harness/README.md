# Soak harness (G4b / TODO-484)

A long-duration **endurance** test for the TopGun server. Where `load_harness`
measures latency/throughput by booting the server **in-process** on a
`NullDataStore`, the soak harness drives the **real out-of-process
`topgun-server` binary** against an **on-disk redb + WAL**, so it can `kill -9`
the process and watch it recover real data — repeatedly, for hours.

It continuously asserts four endurance properties and fails, with captured
context, the moment any breaks:

| Property | What it checks | How |
|----------|----------------|-----|
| **Convergence** | Under client churn, every acked write is faithfully stored/served | Quiesce churn → read every key back via `QUERY_SUB` → compare against the harness's authoritative model (exact, single-writer-per-key) + cross-client Merkle-root agreement |
| **Crash recovery** | A `kill -9` + restart restores the *exact* pre-crash state | Quiesce → snapshot Merkle root + all values → `kill -9` → restart (WAL recovery) → snapshot again → assert byte-for-byte identical |
| **Bounded memory** | A fixed keyspace overwritten in place must plateau in RSS | Sample server RSS over time (`ps`); fail on slope > threshold (with a min-growth guard) or peak > ceiling. OR-Map add/remove churn drives tombstone growth (TODO-479/480) |
| **Zero panics** | No panic anywhere in the run | Live-scan server stdout/stderr for panic markers + flag any un-requested exit, with surrounding log context |

## Why a separate binary, not a `load_harness` flag

Crash recovery requires a real OS process and a real disk. The in-process load
harness cannot `kill -9` itself, and its `NullDataStore` has nothing to recover.
The soak harness spawns the actual server binary (resolved via
`CARGO_BIN_EXE_topgun-server`, overridable with `SOAK_SERVER_BINARY`), pins it to
a stable loopback port, and points it at a persistent data directory so restarts
recover from the same redb + WAL.

## Negative controls (prove the harness can fail)

A soak that cannot fail proves nothing. Two built-in negative controls exercise
the real detection code and **must exit non-zero (assertion RED)**:

```bash
# Convergence detection: record an op in the model that is NOT applied to the
# server ("skip applying one op on a replica"), then assert the read-back compare
# detects the divergence.
soak_harness --inject-divergence      # -> exit 1, "divergence correctly detected"

# Panic detection: feed a synthetic Rust panic line + exit-101 through the same
# PanicWatch the supervisor uses, and assert it trips.
soak_harness --inject-panic           # -> exit 1, "panic correctly captured"
```

If either *fails to detect* (a blind check), it exits 3 instead — that is the
real failure the controls guard against.

## Running

```bash
# Build the server binary + the bench (debug is fine for functional runs).
cargo build --bin topgun-server --bench soak_harness

# Convenience preset: short but full-feature (churn + crash loop + checks).
cargo bench --bench soak_harness -- --smoke

# 1-2h smoke soak to validate the harness on a stabilized build, with reports.
# caffeinate is REQUIRED on macOS so sleep does not break continuity.
caffeinate -dimsu cargo bench --bench soak_harness -- \
  --duration 5400 --crash-interval 180 --steady-interval 60 \
  --churn-clients 16 --keyspace 200 \
  --json-output soak.json --progress-output soak-progress.jsonl
```

A bare invocation (no mode flag) prints usage and exits 0 — this is deliberate
so `cargo test --all-targets`, which runs `harness=false` benches, never launches
a multi-hour default soak.

### Key flags

| Flag | Default | Meaning |
|------|---------|---------|
| `--duration <secs>` | 3600 | Total run time (smoke 1–2h, full 72h = 259200) |
| `--crash-interval <secs>` | 120 | Seconds between `kill -9` + restart recovery checkpoints; `0` disables the crash loop |
| `--steady-interval <secs>` | 30 | Seconds between steady-state convergence checkpoints |
| `--churn-clients <n>` | 16 | Concurrent churn clients (each owns a disjoint key slice) |
| `--keyspace <n>` | 200 | Distinct keys (bounded → legitimate memory is bounded) |
| `--quiesce <secs>` | 3 | Pause+settle before a checkpoint reads or kills (≥ write-behind flush window) |
| `--wal-fsync <policy>` | perop | `perop` \| `batched` \| `none`; `perop` makes recovery assertions crisp |
| `--or-churn <bool>` | true | Drive OR-Map add/remove to grow tombstones (memory watch) |
| `--mem-threshold-mb-per-hour <f>` | 50 | Fail if RSS slope exceeds this (with `--mem-min-growth-mb` guard, default 150) |
| `--mem-ceiling-mb <f>` | 1800 | Fail if peak RSS exceeds this |
| `--data-dir <path>` | tempdir | Persistent data dir (kept for forensics; default tempdir is cleaned on exit) |
| `--json-output <path>` | — | Final structured report |
| `--progress-output <path>` | — | Append one JSON line per checkpoint (tail-able during a 72h run) |

Exit code: `0` pass, `1` an assertion failed (convergence/recovery/memory/panic),
`2` setup error, `3` a negative control failed to detect its injected fault.

## Known finding surfaced by this harness

On its first crash-recovery checkpoint, the soak found **TODO-530** (HIGH):
after `kill -9` + restart the redb-backed server serves an **empty** map
(Merkle root `0`) although the data is durably on disk — the query full-scan and
Merkle-sync paths read only the in-memory `StorageEngine` and persisted records
are not rehydrated on restart (also latent under eviction). Until TODO-530 is
fixed the **crash-loop path is RED by design**. CI therefore runs the no-crash
convergence/churn/memory mode + both negative controls (all green); the crash
loop is run locally and on the Hetzner 72h runner as the TODO-530 reproducer.

## 72h run on Hetzner

See [`hetzner-soak-runner.sh`](./hetzner-soak-runner.sh) and the
"Hetzner 72h runner" section in [`SOAK_RUNNER.md`](./SOAK_RUNNER.md).
