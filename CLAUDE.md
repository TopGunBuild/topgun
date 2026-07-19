# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TopGun lets developers build real-time apps that work offline. Local writes are instant and survive disconnects; reconnecting clients sync seamlessly without manual conflict handling. The Rust server is single-node stable today, ships with an embedded backend for zero-config local development, and accepts Postgres for production; the TypeScript client is browser-and-Node-compatible with IndexedDB persistence. AI agents can read and mutate your live data natively through the bundled MCP server. The project is Apache-2.0 licensed and self-hostable end-to-end.

**Positioning note for agent-authored content:** When drafting landing, marketing, README, or external-facing copy, lead with the user outcome (what they build) before the mechanism (how it works). Internal architecture terminology (the merge primitives, the delta-sync tree, the logical-clock timestamping, the Rust transport layer) belongs in architecture sections or supporting bullets, not in H1 or hero copy. **Avoid:** opening with phrases like "hybrid offline-first in-memory data grid" or naming the conflict-resolution primitives in the lead sentence — those are mechanism, not outcome.

**Key Design Principles:**
- Local-first: Data lives in memory, reads/writes never wait for network
- CRDT conflict resolution using LWW-Map and OR-Map with Hybrid Logical Clocks (HLC)
- Merkle tree synchronization for efficient delta sync
- Server-authoritative cluster architecture with client offline capability

## Common Commands

```bash
# Install dependencies
pnpm install

# Build all packages
pnpm build

# Run all tests
pnpm test

# Run tests for a specific package
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/client test
# Test coverage
pnpm test:coverage
pnpm --filter @topgunbuild/core test:coverage

# Integration tests (TS client → Rust server)
pnpm test:integration-rust

# Simulation tests (deterministic, with fault injection)
pnpm test:sim

# k6 load tests (requires k6 installed)
pnpm test:k6:smoke
pnpm test:k6:throughput
pnpm test:k6:write
pnpm test:k6:connections

# CRDT micro-benchmarks
pnpm --filter @topgunbuild/core bench

# Rust load harness (in-process perf test)
cargo bench --bench load_harness

# Load harness: fire-and-forget mode
cargo bench --bench load_harness -- --fire-and-forget --interval 0

# Start Rust development server
pnpm start:server

# Start documentation site
pnpm start:docs
```

## Architecture

### Package Hierarchy

```
@topgunbuild/core (no internal deps)
    ↓
@topgunbuild/client (depends on core)
    ↓
@topgunbuild/adapters, @topgunbuild/react, @topgunbuild/adapter-better-auth,
@topgunbuild/mcp-server (depend on client)

core-rust (no internal deps)
    ↓
server-rust (depends on core-rust)
```

Note: The server is implemented in Rust (`packages/server-rust/`). The TS-side `@topgunbuild/server` package is gone — the only consumers of `core-rust` are Rust crates.

### Packages

| Package | Purpose |
|---------|---------|
| `core` | CRDTs (LWWMap, ORMap), Hybrid Logical Clock, MerkleTree, message schemas (Zod), serialization (msgpackr) |
| `core-rust` | Rust port of CRDT primitives (`MerkleTree`, `HLC`, `Timestamp`); depended on by `server-rust`. Internal — not published to crates.io |
| `client` | Browser/Node SDK: `TopGunClient`, `SyncEngine`, `QueryHandle`, storage adapter interface |
| `server-rust` | Rust server: axum WebSocket, clustering, redb (default) + Postgres (optional) backends, tokio runtime |
| `react` | React bindings: `TopGunProvider`, `useQuery`, `useMap`, `useORMap`, `useMutation`, `useTopic`, `useSyncState`, `useMergeRejections` |
| `adapters` | Storage implementations: `IDBAdapter` (IndexedDB for browsers) |
| `adapter-better-auth` | BetterAuth integration |
| `mcp-server` | `@topgunbuild/mcp-server` — MCP server (Claude Desktop, Cursor); eight tools over stdio |
| `schema` | `@topgunbuild/schema` — Zod schemas + `topgun codegen` source-of-truth |
| `create-topgun-app` | `npx create-topgun-app` scaffold CLI — publishes as bare `create-topgun-app` on npm |

### Key Abstractions

**Core:**
- `HLC` (Hybrid Logical Clock) - Global causality tracking: `{millis, counter, nodeId}`
- `LWWMap` - Last-Write-Wins Map, conflict resolution by highest timestamp
- `ORMap` - Observed-Remove Map, supports concurrent additions with unique tags
- `MerkleTree` - Efficient delta sync by comparing hashes

**Client:**
- `TopGunClient` - Main entry point, manages maps/queries/topics
- `SyncEngine` - Orchestrates synchronization, handles WebSocket connection and state machine
- `IStorageAdapter` - Interface for local persistence (IndexedDB, etc.)

**Server (Rust):**
- `packages/server-rust/` - Rust server built with axum, tokio, sqlx
- ServiceRegistry with Tower middleware pipeline for operation routing
- Domain services: CRDT, Sync, Query, Search, Messaging, Persistence, Coordination
- Cluster protocol with partition-based data distribution (271 partitions)

### Data Flow

1. Client writes locally to LWWMap + OpLog (IndexedDB)
2. UI updates immediately
3. SyncEngine batches and sends to server when online
4. Server merges using HLC timestamps, persists to embedded **redb** (default; `STORAGE_BACKEND=redb`) or Postgres (`STORAGE_BACKEND=postgres` + `DATABASE_URL`), broadcasts to subscribers
5. Clients reconnecting use MerkleTree for efficient delta sync

## Build System

- Package manager: pnpm 10.13.1 (monorepo with workspaces)
- Build tool: tsup (outputs CJS, ESM, and type declarations)
- Test runner: Jest with ts-jest
- Benchmarks: Vitest (bench mode)

## Commit Message Format

```
type(scope): description

Types: feat, fix, docs, test, chore, refactor, perf
Examples:
  feat(core): add new CRDT merge strategy
  fix(client): resolve sync race condition
  test(server): add cluster integration tests
```

## Code Comments Convention

- **Do NOT** add **provenance** references in code comments — markers whose only job is to answer
  "where did this come from" (e.g., `// Phase 8.02`, `// BUG-06`, `// SPEC-011`)
- Such references belong in **commit messages**, not in code
- Instead, write WHY-comments explaining the reason for the code:
  ```typescript
  // BAD: // Merge defaults (Phase 3 BUG-06)
  // GOOD: // Merge defaults to prevent race condition when topics subscribe before connection
  ```
- **Allowed exceptions** (these are not provenance — they are load-bearing, machine-checkable
  citations that a reader needs at the code):
  - `TG-<DOMAIN>-<NNN>` **invariant IDs** from `INVARIANTS.md`. They are stable identifiers under
    a CI-gated catalog (`scripts/check-invariants.sh`), so citing one in a doc-comment or a
    violation variant tells the reader *which contract this code upholds*, not which ticket
    produced it.
  - A **tracker pointer inside a doc-contract for an invariant that is explicitly known-false or
    deferred** — because a doc-contract asserting a property the code does not have is a
    false-invariant hazard, and naming the tracker is what makes the gap honest rather than
    silent.
- **Spec identifiers (`SPEC-NNN`) have no exception.** They are provenance by definition and are
  forbidden in code comments without qualification. When code needs to point at a spec, cite the
  `TG-<DOMAIN>-<NNN>` invariant instead and let the `INVARIANTS.md` row carry the spec reference —
  the catalog is the sanctioned home for spec routing.

## Pre-Existing Errors Are Owned, Not Deferred

When the user asks "подготовь / проверь проект" (or any audit / release / cleanup task), every failing check in the repository is in scope — regardless of whether the cause predates the session.

**Prohibited phrasings:** "это было до меня", "не моё изменение, не трогаю", "pre-existing, обхожу через --filter", "stale, but not introduced by my changes". These reframe ownership and cause CI breakage to surface at merge time instead of fix time.

**Required behaviour:** any failure surfaced by `pnpm -r build`, `pnpm test`, `pnpm lint`, `pnpm format:check`, `cargo build --release`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `pnpm --filter apps-docs-astro build`, or any equivalent must be either:

1. Fixed in the same session (commit + verify), OR
2. Explicitly recorded as a deferred blocker with: (a) WHY deferral is acceptable, (b) where the follow-up is tracked (TODO marker, issue, roadmap row, audit finding), (c) who owns the fix.

Working around a failure by narrowing scope (`pnpm --filter "./packages/*"` instead of `pnpm -r`) is **never** an acceptable mitigation — it's a bandaid that pushes the problem to CI or the next contributor.

**Pre-merge sanity gate:** before opening a PR / merging / publishing, run the full `MERGE-TO-MAIN-CHECKLIST.md §A` matrix end-to-end. If that file does not exist for the task, run at minimum: `pnpm install --frozen-lockfile && pnpm -r build && pnpm lint && pnpm format:check && cargo build --release && cargo test --release -p topgun-server --lib`.

## Test Notes

- Rust server tests: `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server`
- Integration tests (TS client to Rust server): `pnpm test:integration-rust`
- Doc tests (G2 gate — every doc snippet run against the real server / type-checked against real types / explicitly skipped with a reason): `pnpm test:docs` (set `RUST_SERVER_BINARY` to a prebuilt binary to skip cargo). Authoring contract: `tests/doc-tests/README.md`. New snippets are picked up automatically — no allowlist; to exclude a block add an explicit `doctest skip reason="…"` directive.
- Run TS tests sequentially in CI to avoid port conflicts: `pnpm test -- --runInBand`

## Production Defaults (Memory + Persistence)

The server reads the following environment variables at startup. Defaults are tuned so a freshly-cloned demo server won't OOM under load — set the override before starting the process if your deployment needs different ceilings.

- `TOPGUN_MAX_RAM_MB` (default: `1024`) — RAM ceiling for the in-memory record cache; eviction engages above the high water mark.
- `TOPGUN_EVICTION_HIGH_PCT` (default: `85`) — high water mark percent. Eviction starts when in-memory cost exceeds this fraction of `TOPGUN_MAX_RAM_MB`.
- `TOPGUN_EVICTION_LOW_PCT` (default: `70`) — low water mark percent. Eviction stops once in-memory cost drops to this fraction.
- `TOPGUN_EVICTION_INTERVAL_MS` (default: `1000`) — orchestrator tick interval.
- `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` (default: `1000`) — how often the write-behind buffer flushes to the durable backend.
- `TOPGUN_WRITEBEHIND_BATCH_SIZE` (default: `100`) — maximum records flushed per write-behind tick.
- `TOPGUN_WRITEBEHIND_CAPACITY` (default: `10000`) — bounded buffer size; once full, writes apply pressure to the producer rather than allocating without limit.
- `TOPGUN_WAL_FSYNC_POLICY` (default: `batched`) — how aggressively the WAL fsyncs. `batched` (default) acks a write after appending its frame but fsyncs only on a ~10 ms group-commit timer (or a 100-frame flush) — the throughput-favouring choice. `per_op` fsyncs every frame **before** the write acks, so acked-implies-durable under `kill -9`, at a large per-write latency cost (see durability note below). `none` never fsyncs (tests/benchmarks only). Accepts `per_op`/`perop`/`per-op`, `batched`, `none` (case-insensitive); an unparseable value is fatal at startup.
- `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS` (default: `30000`) — how long a graceful shutdown waits for the write-behind buffer to drain before logging the still-pending ops and exiting rather than blocking termination.
- `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` (default: `60000`) — how long a partition's smallest un-resolved WAL sequence may stay put, under ongoing writes, before the stalled-watermark alarm fires. **Raise it alongside `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` / `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`:** the 60 s default is derived from *their* defaults, so raising either pushes the longest legitimate non-advance past this bound and the alarm starts firing on correct behaviour. An unparseable value warns and falls back to the default; a value that parses but is below `6000` is fatal at startup, because it would silently shrink the alarm's own false-positive guard while the boot line still reports the value as effective.
- `TOPGUN_JOURNAL_ENABLED` (default: `true`) — Event Journal capture. When `true`, every applied mutation is appended to an in-memory ring buffer and pushed to matching `JournalSubscribe` connections (powers `getEventJournal`/`useEventJournal`). Set `false` to shed the per-write journal cost; reads then observe an empty buffer — an explicit, startup-logged opt-out, not a silent dark feature.
- `TOPGUN_JOURNAL_CAPACITY` (default: `10000`) — Event Journal ring-buffer size. Oldest events are evicted once full. The journal is in-memory only and not durable across restart.

At startup the server emits a single `tracing::info!` line containing the effective `max_ram_mb`, water marks, eviction interval, `write_behind_enabled`, `wal_fsync_policy`, and `wal_watermark_stall_bound_ms` so operators can confirm the active configuration without reading source. When the effective policy is the `batched` default (and a durable backend is active), the server also emits a `tracing::warn!` stating that acked writes in the group-commit window are not durable under unclean shutdown — so the caveat is visible at boot. A second `event journal initialized` line reports the effective `journal_enabled` and `journal_capacity`.

### WAL fsync durability (what the default guarantees under `kill -9`)

The default `TOPGUN_WAL_FSYNC_POLICY=batched` acks a write once its WAL frame is appended, then fsyncs the frame lazily on a ~10 ms group-commit timer (or once 100 un-fsynced frames accumulate). An unclean `kill -9` inside that window drops the acked-but-unfsynced writes: **the `batched` default does NOT guarantee acked-implies-durable.** This is a deliberate throughput choice for the single-node demo tier — the originating client still holds the write in local IndexedDB and re-converges via CRDT delta-sync on reconnect, so server-side loss is a re-sync event, not permanent data loss. Set `TOPGUN_WAL_FSYNC_POLICY=per_op` to fsync every frame before acking (acked == durable), at a large throughput/latency cost — measured at roughly 40× lower single-partition append throughput on macOS (`sync_data` there is a full `F_FULLFSYNC` device barrier; the penalty is materially smaller on Linux `fdatasync`/NVMe but has not yet been measured there). The crash-recovery durability proofs (SPEC-331/332/333) were all run under `per_op`.

Note (write-behind flush, distinct from the fsync policy above): Write-Behind buffers acked writes for ~1s before persisting to the durable backend. Acceptable for the demo server tier; crash-safe shutdown drain + WAL recovery land separately (TODO-339, post-HN). Until that lands, an unclean shutdown can lose buffered writes that have not yet been flushed.

## Simulation Testing

The simulation testing framework provides deterministic testing of distributed behavior under network faults and node failures, without real networking or timers.

### Architecture

- **SimCluster** (`packages/server-rust/src/sim/cluster.rs`) — orchestrates N in-memory nodes with `write`/`read`/`advance_time` convenience methods
- **SimNetwork** (`packages/server-rust/src/sim/network.rs`) — structural fault injection: `partition`, `heal`, `delay`, `reorder` between node pairs
- **SimNode** — single node built via `SimNode::build()`, wiring all 7 domain services with NullDataStore + HashMapStorage

### Running Simulation Tests

```bash
pnpm test:sim
# Equivalent to: cargo test --profile ci-sim --features simulation -p topgun-server -- sim
```

### When to Write Simulation Tests

Changes to domain services under `packages/server-rust/src/service/domain/` should be accompanied by simulation tests that exercise the changed behavior under at least one fault scenario (network partition or node failure). This ensures distributed correctness is validated before merge.

### Proptest Async Bridge

Proptest closures are synchronous, but the simulation harness is async. The bridge pattern uses `block_in_place` + `Handle::block_on` inside `#[tokio::test(flavor = "multi_thread")]` to run async sim code from within sync proptest strategies. The `multi_thread` flavor is required because `block_in_place` panics on a single-threaded runtime.

## Performance Testing

The load harness (`packages/server-rust/benches/load_harness/`) boots a full server instance (all 7 domain services, partition dispatcher, WebSocket handler) in-process, opens N WebSocket connections, and runs configurable scenarios while recording latency with HDR histograms. Results are printed as ASCII tables and optionally written as JSON for CI.

### Modes

- **Fire-and-wait** (default): sends an OpBatch, waits for OP_ACK, and records round-trip latency. Use this to measure end-to-end request latency.
- **Fire-and-forget** (`--fire-and-forget`): sends batches without waiting for acknowledgement. Use this to measure raw push throughput.

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--connections` | 200 | Number of concurrent WebSocket connections |
| `--duration` | 30s | Total test duration |
| `--interval` | 50ms | Delay between sends per connection |
| `--fire-and-forget` | off | Enable fire-and-forget mode |
| `--json-output` | off | Write results as JSON (for CI consumption) |

### Running Locally

```bash
# Quick smoke test
cargo bench --bench load_harness -- --connections 50 --duration 10

# Full run (default: 200 connections, 30s)
cargo bench --bench load_harness

# Fire-and-forget throughput test
cargo bench --bench load_harness -- --fire-and-forget --interval 0
```

### Baseline Assertions

The harness enforces two pass/fail checks:
- **Acked ratio** >= 80%
- **p99 latency** < 500ms

Both must pass for exit code 0. Baseline thresholds for CI are defined in `packages/server-rust/benches/load_harness/baseline.json`.

### CI Perf-Gate

The `perf-gate` job in `.github/workflows/rust.yml` runs both fire-and-wait and fire-and-forget scenarios (200 connections, 15s each), compares results against baseline.json thresholds using `jq`, and is currently informational (`continue-on-error: true`).

### Flamegraph Profiling

See `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md` for flamegraph methodology, baseline numbers, and hot-path analysis. Flamegraphs use the `release-with-debug` Cargo profile and can be generated with `cargo flamegraph` or macOS Instruments.

### When to Run the Load Harness

Changes to hot-path code should be verified with the load harness before merge:
- Service routing: `service/dispatch/`, `service/middleware/`
- CRDT merge: `service/domain/crdt/`
- WebSocket handling: `network/handlers/`
- Serialization

Run at minimum a fire-and-wait scenario and compare ops/sec against the baseline. Regressions over 20% require investigation or justification in the PR.
