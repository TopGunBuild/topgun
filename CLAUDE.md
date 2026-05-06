# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TopGun is a hybrid offline-first in-memory data grid. It provides zero-latency reads/writes via local CRDTs, real-time sync via WebSockets, and durable storage on PostgreSQL.

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
@topgunbuild/adapters, @topgunbuild/react (depend on client)
```

Note: The server is implemented in Rust (`packages/server-rust/`). See `packages/server-rust/` for the Rust server codebase.

### Packages

| Package | Purpose |
|---------|---------|
| `core` | CRDTs (LWWMap, ORMap), Hybrid Logical Clock, MerkleTree, message schemas (Zod), serialization (msgpackr) |
| `client` | Browser/Node SDK: TopGunClient, SyncEngine, QueryHandle, storage adapters interface |
| `server-rust` | Rust server: axum WebSocket server, clustering, PostgreSQL adapter (tokio runtime) |
| `react` | React bindings: TopGunProvider, useQuery, useMap, useORMap, useMutation, useTopic hooks |
| `adapters` | Storage implementations: IDBAdapter (IndexedDB for browsers) |
| `adapter-better-auth` | BetterAuth integration |

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
4. Server merges using HLC timestamps, broadcasts to subscribers
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

- **Do NOT** add phase/spec/bug references in code comments (e.g., `// Phase 8.02`, `// BUG-06`, `// SPEC-011`)
- Such references belong in **commit messages**, not in code
- Instead, write WHY-comments explaining the reason for the code:
  ```typescript
  // BAD: // Merge defaults (Phase 3 BUG-06)
  // GOOD: // Merge defaults to prevent race condition when topics subscribe before connection
  ```

## Test Notes

- Rust server tests: `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server`
- Integration tests (TS client to Rust server): `pnpm test:integration-rust`
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

At startup the server emits a single `tracing::info!` line containing the effective `max_ram_mb`, water marks, eviction interval, and `write_behind_enabled` so operators can confirm the active configuration without reading source.

Note: Write-Behind buffers acked writes for ~1s before persisting to disk. Acceptable for the demo server tier; crash-safe shutdown drain + WAL recovery land separately (TODO-339, post-HN). Until that lands, an unclean shutdown can lose buffered writes that have not yet been flushed.

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
