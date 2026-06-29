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

## CI coverage: a fast smoke gate + a loaded convergence gate

Two CI jobs run this harness (`.github/workflows/rust.yml`):

- **`soak-smoke` (blocking).** The two negative controls (must go RED) + a tiny
  no-crash soak (`--duration 25 --churn-clients 6 --keyspace 48 --crash-interval 0`).
  It guards that the harness compiles and its fault detection still works, on every PR.
- **`soak-loaded` (non-blocking, advisory).** A **loaded** no-crash convergence soak
  (`--duration 150 --churn-clients 16 --keyspace 200`, `--crash-interval 0`) that builds
  a real write-behind backlog. The smoke gate is too small to accumulate a backlog, so a
  backlog-dependent durability/convergence regression cannot surface there — exactly the
  gap that masked **SPEC-325b AC1**, which only failed at keyspace ~200 / churn ~16. This
  job shifts that coverage left to PR/CI time. It is `continue-on-error: true` for now
  because the GREEN result is established on local debug/macOS and not yet validated on
  ubuntu CI; **promote it to blocking after ≥10 consecutive green runs on `main`** (owner:
  the stabilization-program G4b gate).

  Memory on the loaded job: the slope threshold stays at the honest default (50 MB/h) but
  `--mem-min-growth-mb 500` makes the slope arm fire only as a **gross-leak tripwire** — a
  150s OR-churn burst's slope extrapolated to MB/hour is not a valid slow-leak signal (that
  is the 72h run's job); the `--mem-ceiling-mb 1200` ceiling is the real OOM guard. The job
  gates on **convergence-under-backlog**, not on the memory slope.

## Known finding surfaced by this harness

### Crash-recovery at load is RED — now due to TODO-546, not TODO-530

The first version of this harness found **TODO-530** (HIGH): after `kill -9` + restart the
redb-backed server served an **empty** map (Merkle root `0`) although the data was durably
on disk — the query full-scan and Merkle-sync paths read only the in-memory `StorageEngine`
and persisted records were not rehydrated on restart. **TODO-530 is now CLOSED** (SPEC-325a/b):
the `DurableMerkleIndex` and `FullScanPager` read from the datastore, the recovery checkpoint
gates were promoted to **HARD** (`main.rs` `recovery_checkpoint`, "must survive a kill -9 +
restart unchanged"), and the checkpoint quiesce (`main.rs`, `paused` + `sleep(quiesce)`) drains
the write-behind buffer before the kill. The empty-map failure no longer reproduces.

A **loaded with-crash** soak is, however, **still RED** — but for a different reason. At
churn 16 / keyspace 200, a `kill -9` recovery checkpoint shows keys a few increments **behind**
post-restart with a **non-zero** Merkle root (i.e. partial, not empty). That is **TODO-546**
(ack-before-durable): the write-behind buffer acks writes ~1s before they are persisted, and at
load the 3s quiesce does not fully drain the backlog, so acked-but-unflushed writes are lost on
the unclean kill. This is a **known-open critical** (MUST-FIX before public launch), not a fresh
regression. CI therefore runs the loaded variant **no-crash** (above); the **with-crash** loaded
run stays local and on the Hetzner 72h runner as the **TODO-546 reproducer/validator** (TODO-546
`depends_on` this loaded soak). Once TODO-546 is fixed, promote a with-crash loaded variant into
CI as a blocking crash-recovery-at-load gate — otherwise crash recovery at load is never
CI-gated, the same gap class as SPEC-325b. (Issue states drift: re-check the live TODO-530/546
status before trusting this note.)

### Crash-window `write_errors` are transient connection drops, not a write-path defect

The `write_errors` counter increments **only** when an active-burst `write_lww` fails at the
socket (`run_churn_client`: on `write_lww(...).is_err()` it bumps `write_errors`, sets
`session_alive = false`, breaks, and the outer loop reconnects via `connect_with_retry`, bumping
`reconnects`; the next session's replay phase resends every owned key under LWW, bumping
`resends`). A failed write therefore means *the connection died mid-write* — which during a
`kill -9` window is expected — and is immediately self-healing: reconnect + idempotent
higher-HLC resend restores any value lost to the kill window.

Evidence this is transient, not a flush/write-path defect:

- **No-crash runs are clean.** Every no-crash soak (smoke and loaded, 60k+ writes) reports
  `write_errors = 0` — the steady-state write path produces no errors.
- **In this harness the counter is 0 even with crashes**, because `kill -9` happens during the
  recovery-checkpoint quiesce while churn is **paused** (clients hold, no in-flight burst write),
  so no write straddles the kill window.
- The historically observed **~13 / 20k** crash-window `write_errors` (≈0.065%) came from a
  config where crashes overlapped **active** churn: those ~13 are exactly the in-flight burst
  writes whose socket died during the kill, each followed by a `reconnects` increment and a
  `resends` replay. They track reconnects 1:1 and leave no missing data once the client replays.

**Conclusion:** the crash-window `write_errors` are expected transient kill-window connection
drops, self-healed by reconnect + LWW resend — **not** a flush-induced write-path defect. (Data
loss at the crash boundary at load is a *separate* property, owned by the crash-recovery
assertion above = TODO-546, and is not measured by `write_errors`.) No follow-up TODO is filed.

## 72h run on Hetzner

See [`hetzner-soak-runner.sh`](./hetzner-soak-runner.sh) and the
"Hetzner 72h runner" section in [`SOAK_RUNNER.md`](./SOAK_RUNNER.md).
