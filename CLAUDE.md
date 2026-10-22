# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TopGun lets developers build real-time apps that work offline. Local writes are instant and survive disconnects; reconnecting clients sync seamlessly without manual conflict handling. The Rust server is single-node stable today, ships with an embedded backend for zero-config local development, and keeps its data in an embedded store, so there is no database to run; the TypeScript client is browser-and-Node-compatible with IndexedDB persistence. AI agents can read and mutate your live data natively through the bundled MCP server. The project is Apache-2.0 licensed and self-hostable end-to-end.

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
| `server-rust` | Rust server: axum WebSocket, clustering, embedded redb storage, tokio runtime |
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
- `packages/server-rust/` - Rust server built with axum, tokio, redb
- ServiceRegistry with Tower middleware pipeline for operation routing
- Domain services: CRDT, Sync, Query, Search, Messaging, Persistence, Coordination
- Cluster protocol with partition-based data distribution (271 partitions)

### Data Flow

1. Client writes locally to LWWMap + OpLog (IndexedDB)
2. UI updates immediately
3. SyncEngine batches and sends to server when online
4. Server merges using HLC timestamps, persists to embedded **redb** (default; `STORAGE_BACKEND=redb`) (the only supported store), broadcasts to subscribers
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
- That default run leaves out the files listed in `tests/integration-rust/opt-in-suites.js`, which need a binary a clean checkout lacks, and prints which and why (that one list is used by CI too); run them with `pnpm test:integration-rust:two-binary` (`map-name-upgrade` needs `OLD_SERVER_BINARY`, a build of `07d009f0`; `map-name-rollback` needs `FORMAT_OLD_SERVER_BINARY`, a build of `b346e277`; `watermark-rollback` needs `WATERMARK_OLD_SERVER_BINARY`, a build of `6a030bb6`; each with its `*_COMMIT` variable (`OLD_SERVER_COMMIT`, `FORMAT_OLD_SERVER_COMMIT`, `WATERMARK_OLD_SERVER_COMMIT`) and `NEW_SERVER_COMMIT`) , `pnpm test:integration-rust:liveness` (`store-refusal-liveness`, `flush-task-panic`, `store-broken`; needs `FAULT_SERVER_BINARY`) and `pnpm test:integration-rust:measure` (`write-behind-batch-measure`: the drain at `SIGTERM`, the start replay and the store file of a `6a030bb6` build against the binary under test, out of process, up to three hours; needs `MEASURE_BASE_SERVER_BINARY` and `MEASURE_BASE_SERVER_COMMIT` for the `6a030bb6` build, `RUST_SERVER_BINARY` and `MEASURE_NEW_SERVER_COMMIT` equal to `git rev-parse HEAD`; `MEASURE_OUT_FILE` names its JSON) — all three fail, never skip, without their variables (`watermark-rollback` included).
- The server refuses to start when `DATABASE_URL` is set (an empty value counts), because the Postgres backend has been removed; the test harnesses drop it from the environment they pass to the server, and a server started by hand needs it unset.
- Doc tests (G2 gate — every doc snippet run against the real server / type-checked against real types / explicitly skipped with a reason): `pnpm test:docs` (set `RUST_SERVER_BINARY` to a prebuilt binary to skip cargo). Authoring contract: `tests/doc-tests/README.md`. New snippets are picked up automatically — no allowlist; to exclude a block add an explicit `doctest skip reason="…"` directive.
- Run TS tests sequentially in CI to avoid port conflicts: `pnpm test --runInBand`

## Production Defaults (Memory + Persistence)

The server reads the following environment variables at startup. Defaults are tuned so a freshly-cloned demo server won't OOM under load — set the override before starting the process if your deployment needs different ceilings.

- `TOPGUN_DATA_DIR` (default: `{directory of TOPGUN_REDB_PATH}/topgun-data` when `TOPGUN_REDB_PATH` is set, else `./topgun-data`) — the store directory; it holds `engine.redb`, which also holds each WAL partition's applied watermark, committed together with the records it covers. The data directory and the WAL directory (`TOPGUN_WAL_DIR`) are therefore one unit: back them up, restore them and move them together. `TOPGUN_REDB_PATH` (default `./topgun.redb`) now names only the legacy store file of earlier versions, which the server migrates into the data directory once, and the marker file left at that path. A set-but-empty value of either is fatal at startup. The effective directory and where it came from are on the `store location: data directory …` startup line. See "Rolling a node BACK across the data-directory layout" below before deleting anything at either path.
- `TOPGUN_MAX_RAM_MB` (default: `1024`) — RAM ceiling for the in-memory record cache; eviction engages above the high water mark.
- `TOPGUN_EVICTION_HIGH_PCT` (default: `85`) — high water mark percent. Eviction starts when in-memory cost exceeds this fraction of `TOPGUN_MAX_RAM_MB`.
- `TOPGUN_EVICTION_LOW_PCT` (default: `70`) — low water mark percent. Eviction stops once in-memory cost drops to this fraction.
- `TOPGUN_EVICTION_INTERVAL_MS` (default: `1000`) — orchestrator tick interval.
- `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` (default: `1000`) — how often the write-behind buffer flushes to the durable backend.
- `TOPGUN_WRITEBEHIND_BATCH_SIZE` (default: `100`) — accepted and still without effect. Nothing reads the value: the size of a flush pass is fixed at 1000 entries or 8 MiB of encoded values, whichever comes first, and the pass is stored in one durable commit. The setting is deliberately not honoured, because the documented example value `100` would silently cut every pass tenfold on deployments that copied it. A deployment that sets it still starts.
- `TOPGUN_WRITEBEHIND_CAPACITY` (default: `10000`) — a soft bound on buffered keys, so the write buffer cannot grow without limit: a write of a key that is not already buffered is refused once this many keys are buffered; `0` means unlimited. It is not a hard limit: the count can exceed it by the writes of new keys that passed the check at the same moment (`W`), plus one re-write of each key the running flush pass holds (`H`) — `buffered <= C - 1 + W + H`, with `H <= min(L, C - 1 + W)`, because a pass holds at most `L` = 1000 of the entries that were due when it began, and `H <= 1` while a store-wide hold is in force (see "When the store refuses writes" below) — so in the worst case the count reaches up to about twice the capacity. A key stays counted until its entry is persisted or retired into a newer write of the same key; an entry the store refuses is neither, and stays counted until the store takes it. So while the store refuses writes the buffer stays at its bound, and every write of a key that is not already buffered is refused until the store returns. A refused write is not acknowledged: the server answers with the transient `internal` error (code 500), and the client keeps the operation and sends it again. `TOPGUN_WRITEBEHIND_CAPACITY=0` together with a store that refuses writes is not supported: nothing then limits the buffer, and memory grows for as long as the store refuses.
- `TOPGUN_WAL_FSYNC_POLICY` (default: `batched`) — how aggressively the WAL fsyncs. `batched` (default) acks a write after appending its frame but fsyncs only on a ~10 ms group-commit timer (or a 100-frame flush) — the throughput-favouring choice. `per_op` fsyncs every frame **before** the write acks, so acked-implies-durable under `kill -9`, at a large per-write latency cost (see durability note below). `none` never fsyncs (tests/benchmarks only). Accepts `per_op`/`perop`/`per-op`, `batched`, `none` (case-insensitive); an unparseable value is fatal at startup.
- `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS` (default: `30000`) — how long a graceful shutdown waits for the write-behind buffer to drain before logging the still-pending ops and exiting rather than blocking termination.
- `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` (default: `60000`) — how long a partition's smallest un-resolved WAL sequence may stay put, under ongoing writes, before the stalled-watermark alarm fires. **Raise it alongside `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` / `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`:** the 60 s default is derived from *their* defaults, so raising either pushes the longest legitimate non-advance past this bound and the alarm starts firing on correct behaviour. A store outage is not a legitimate non-advance: a write the store refuses is retried until it is stored and its sequence stays un-resolved for as long, so once an outage has outlasted this bound the alarm also fires, under ongoing writes, for the rest of the outage — which is correct (see "When the store refuses writes" below). An unparseable value warns and falls back to the default; a value that parses but is below `6000` is fatal at startup, because it would silently shrink the alarm's own false-positive guard while the boot line still reports the value as effective.
- `TOPGUN_OR_DELTA_WAL` (default: `true`) — how an OR-Map write is framed in the WAL. When armed, an OR mutation that arrives with a witness (the single add/remove the mutation point actually applied) is written as a delta frame carrying only that mutation, instead of a full snapshot of the key's whole slot — which is what stops WAL bytes growing with slot size on every OR write. Set it to `false` and every OR write frames a full snapshot exactly as it did before delta framing existed; the durable store write and the in-memory value are identical either way, only the WAL encoding changes. It is a kill-switch, so only an explicit falsey word (`false`/`0`/`no`/`off`, case-insensitive) disarms it — an unrecognised value leaves it ARMED rather than quietly rolling a node back to an encoding nobody asked for. Read ONCE at store construction, never per write, so the arming gate and the framing branch cannot observe different answers for the same op; the effective value is reported on the startup config line. See the rollback caveat below before flipping it on a node whose WAL already holds delta frames.
- `TOPGUN_JOURNAL_ENABLED` (default: `true`) — Event Journal capture. When `true`, every applied mutation is appended to an in-memory ring buffer and pushed to matching `JournalSubscribe` connections (powers `getEventJournal`/`useEventJournal`). Set `false` to shed the per-write journal cost; reads then observe an empty buffer — an explicit, startup-logged opt-out, not a silent dark feature.
- `TOPGUN_JOURNAL_CAPACITY` (default: `10000`) — Event Journal ring-buffer size. Oldest events are evicted once full. The journal is in-memory only and not durable across restart.
- `TOPGUN_RECLAMATION_MARGIN_EPOCHS` (default: `0`) — safety slack, in **epochs**, subtracted from the minimum live reclamation claim to form the tombstone-prune ceiling. `0` is a valid value and is the default: at margin `0` the ceiling is arithmetically the same fleet minimum the previous low-water-mark fold computed, so the reclamation registry moves **authority** over the boundary without moving the boundary itself. Read **ONCE** at registry construction and never per query, so the parse and the ceiling arithmetic cannot observe different answers within one prune pass — changing it therefore requires a restart. An unparseable value **WARNs and falls back to the default**; it never panics, because the constructor runs inside every `TombstoneFrontier`. Unlike the knobs above, its effective value is **not** on the startup config line — confirm it on `/metrics`, as the `topgun_reclamation_margin_epochs` gauge, alongside the twelve other `topgun_reclamation_*` series the registry registers eagerly (so all thirteen render from the first scrape, at zero, rather than appearing only once something moves).

At startup the server emits a single `tracing::info!` line containing the effective `max_ram_mb`, water marks, eviction interval, `write_behind_enabled`, `wal_fsync_policy`, and `wal_watermark_stall_bound_ms` so operators can confirm the active configuration without reading source. When the effective policy is the `batched` default (and a durable backend is active), the server also emits a `tracing::warn!` stating that acked writes in the group-commit window are not durable under unclean shutdown — so the caveat is visible at boot. A second `event journal initialized` line reports the effective `journal_enabled` and `journal_capacity`.

### WAL fsync durability (what the default guarantees under `kill -9`)

The default `TOPGUN_WAL_FSYNC_POLICY=batched` acks a write once its WAL frame is appended, then fsyncs the frame lazily on a ~10 ms group-commit timer (or once 100 un-fsynced frames accumulate). An unclean `kill -9` inside that window drops the acked-but-unfsynced writes: **the `batched` default does NOT guarantee acked-implies-durable.** This is a deliberate throughput choice for the single-node demo tier — the originating client still holds the write in local IndexedDB and re-converges via CRDT delta-sync on reconnect, so server-side loss is a re-sync event, not permanent data loss. Set `TOPGUN_WAL_FSYNC_POLICY=per_op` to fsync every frame before acking (acked == durable), at a large throughput/latency cost — measured at roughly 40× lower single-partition append throughput on macOS (`sync_data` there is a full `F_FULLFSYNC` device barrier; the penalty is materially smaller on Linux `fdatasync`/NVMe but has not yet been measured there). The crash-recovery durability proofs (SPEC-331/332/333) were all run under `per_op`. A crash loop in the server's tests (`c90_crash_loop`) kills a child process running the write path with `SIGKILL` at random moments and cuts the WAL back to its last fsync before each restart: under `per_op` every acknowledged write is recovered; under `batched` it shows no sequence number reused and no WAL collected above the store's watermark, but it does not bound how much the window may lose.

### When the store refuses writes

A write the server has acknowledged is not dropped because the durable store refuses it. It stays in the write-behind buffer, stays readable — also after its record was evicted from memory — and is stored once the store accepts writes again (`TG-WB-009`, `TG-EVI-001`). Nothing gives up after a number of attempts: a refused write is retried for as long as the process runs — with one exception: when the embedded redb store reports an I/O error (a full disk included), a previous I/O error, corruption or a poisoned lock, the node does not retry but stops with exit code 72 (see the next section, `TG-WB-010`). The price is back-pressure. Nothing leaves the buffer while the store refuses, so once the buffer is at its bound (`TOPGUN_WRITEBEHIND_CAPACITY`) the node refuses writes of new keys — as a temporary error, so they are not acknowledged and the client keeps them — until the store returns. Writes of keys that are already buffered are still accepted.

**How the retries are paced.** There is no setting for it. A flush pass takes up to 1000 due writes or 8 MiB of encoded values, whichever comes first, and stores them in one durable commit; while writes are still due, the next pass starts at once instead of waiting for the flush interval. One place in every pass is kept for the oldest due refused write, so a refused write is never starved by new ones. When the store refuses a commit of several writes, the server splits it in two halves and commits each: a refused half is split again, and an accepted half is stored. So one refused record ends up refused alone while every other write of the pass is stored, at a cost of at most `2⌈log₂ n⌉` extra commits for a pass of `n` writes (20 for 1000). A write refused alone waits before its next attempt: 1, 2, 4 and then 5 s between attempts. When **both halves** of a refused commit are refused, the server takes the store for down and starts a store-wide hold, after exactly three calls (the commit and its two halves): it stops calling the store altogether for 0.5 s, then tries one write as a probe, and after each refused probe leaves the store alone for longer — 1, 2, 4, then 5 s. When a probe is accepted the hold ends and the passes go on over everything that is due. The first refused commit after an accepted probe starts no hold: it is split down to single entries, so records refused in both halves are each refused alone and the rest is stored; only after that may a refused commit start a hold again.

**What an operator sees in the log.**

- `Write-behind entries are being retried: the store refuses them`, at `WARN`: first when a write has been refused three times or a hold begins, then at most once per 30 s while anything is still being retried. Its fields are `retrying`, `oldest_refusal_age_s`, `hold`, and one refused write as an example: `map`, `key`, `attempts`, `error`.
- `WAL watermark stalled on a write the store has not accepted …`, at `ERROR`: once a partition's oldest unstored write has stayed the same for `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` (60 s by default), on every watchdog tick (6 s with defaults) for every partition that has a refused write and ongoing writes, for the rest of the outage. This line is not rate-limited and during an outage it is expected (TODO-757).
- `Write-behind retry backlog cleared: the store accepted every refused entry`, at `INFO`, once: when nothing is being retried any more after a warning was logged.

**The metrics.** The seven counters and gauges below are present from the first scrape, at zero; the histogram `topgun_write_behind_flush_pass_seconds` appears with the first committed pass.

- `topgun_write_behind_flush_refusals_total` — commits of the flush task the store refused; each half of a split commit and each probe counts on its own.
- `topgun_write_behind_buffered_entries` — the number of buffered keys, the count the capacity check uses. **This is the number to watch against `TOPGUN_WRITEBEHIND_CAPACITY`:** once it reaches the capacity, writes of new keys are refused.
- `topgun_write_behind_retrying_entries` — buffered entries the store has refused on their own at least once (alone in a commit, by a record-level refusal, or as a probe), as of the last pass. Writes a hold put back are not counted. It is not the size of the buffer: it reads 0 when a hold begins, and while the store refuses everything a hold allows one call per period, so this number is then about the number of refused probes.
- `topgun_write_behind_oldest_refusal_age_seconds` — for how long the store has been refusing the buffered entry it has refused longest; 0 when none is refused. Read it as "at least": cleanup of deleted records (tombstone pruning) on every map of the node has stood still for at least this long, and may have stopped earlier, when that entry was written. A key that is rewritten while refused keeps its first refusal time. It does not say how many tombstones are waiting (TODO-828).
- `topgun_write_behind_store_hold` — 1 from the moment a store-wide hold begins until a probe is accepted, else 0.
- `topgun_write_behind_flush_passes_total` — flush passes that committed at least one write.
- `topgun_write_behind_flush_pass_entries_total` — writes the committed passes stored, summed over the passes; divided by the passes it gives the mean pass size, at most 1000.
- `topgun_write_behind_flush_pass_seconds` — how long one committed pass took, from its selection to its settle, one observation per pass.

**What is bounded** (default settings; a store that answers promptly).

Below, `B` = 0.5 s, `K` = 5 s and `I` = 1 s are the backoff base, the backoff cap and the flush interval, and `L` = 1000 is the most writes one pass takes. `m` is the fewest never-refused writes a pass takes when that many are due: 999 for values up to about 8 KiB, at least 1 while values stay at or below 4 MiB. `C_pass` is the longest pass; a pass that meets one refused record lasts at most `1 + 2⌈log₂ L⌉` = 21 times the longest commit. Hold time is left out of every pass count.

- One refused record in a pass of `n` writes costs at most `2⌈log₂ n⌉` extra commits — at most 20 for `n = 1000` — and the other `n − 1` writes are stored in that pass.
- `k` refused records cost at most `2k⌈log₂ n⌉` extra commits as long as no two of them fall into the two halves of one split; when they do, a hold starts. This general formula is **not proven** by a test (one record and the meeting case are).
- While the store refuses every commit, the flush task calls it at most `3 + ⌈log₂(K / B)⌉ + 1 + D / K` = `8 + D / 5 s` times in any stretch `D` that begins with the first refusal and contains no accepted probe — 20 calls in 60 s — however many writes are buffered.
- When the store goes down again right after an accepted probe, the first refused commit of `n` writes is split down to single entries and costs at most `2n − 1` commits without a hold; the next refused commit starts a hold after three calls. A stretch `D` that begins with that refusal costs at most `2L − 1 + 8 + D / 5 s` commits — 2 019 in 60 s with `L = 1000`. For a store that flaps, each accepted probe adds up to `2L − 1` more; a bound over several flaps is **not proven**.
- Once the store accepts again at `T`, with no write after `T`, every write buffered at `T` is in a pass that begins no later than `T + K + I + (⌈N_T / m⌉ − 1) · C_pass`, `N_T` the writes buffered at `T` — within 6 s while they fit in one pass. How long one commit takes is not bounded by the server.
- A new write the store accepts is attempted in one of the first `⌈(Q_H + 1) / m⌉` passes that begin after it is due, `Q_H` the never-refused due writes ahead of it — the first pass while fewer than 999 are ahead — however many other buffered writes the store keeps refusing; during a hold within 6 s of the moment it is due. This holds while the store time of writes does not step back (it is the wall clock); after a backward step it is **not proven**, and for values over 4 MiB it is **not proven** either.
- A refused write whose next attempt is due is attempted in one of the first `R_T + 1` passes outside holds, `R_T` the due refused writes ahead of it in the order of their next attempt — whatever the load of new writes, because each pass keeps a place for the oldest due refused one. In time: within `(R_T + 1) · (C_pass + I)` plus hold time. Across holds one probe per hold applies: among `P` writes the store refuses for ever, at most `(P + 1) · (K + I)` — 186 s for `P = 30`, linear in `P`.

**What is not bounded.**

- Without classifying store errors there is no time by which every key reaches the store while some records are refused for ever. A healthy key written before `N` refused keys that have not been tried yet, with no newer healthy write, waits up to one hold for each of them when they start holds — measured before passes were batched: 7.5 s for `N = 3`, 142.5 s for `N = 30`, 442.5 s for `N = 90`, in the order of 14 hours for a full default buffer (TODO-824).
- Both halves of a refused commit refused start the store-wide hold also when the store is healthy and two particular records are refused, one in each half: two refused records anywhere in a pass start a hold even though everything else in it is stored. The hold is a heuristic for "the store is down", and such records look the same to it. After the accepted probe the next refused commit is split to single entries, which isolates those records; that costs one hold cycle plus at most `2n − 1` commits. New refused records that keep arriving while isolated ones are still retried may each cost another hold cycle; that is **not proven** bounded. No such record-level refusal is known for redb.
- A pass whose kept place holds a record the store keeps refusing pays one split, at most `2⌈log₂ n⌉` extra commits; with `P` such records under sustained load that is up to `20 P` extra commits per 5 s, and a few tens of them can exceed the flush budget.
- Records the store never accepts stay in the buffer. Once there are as many of them as the capacity, the node refuses writes of new keys until an operator intervenes (TODO-824).
- `TOPGUN_WRITEBEHIND_CAPACITY=0` together with a store that refuses is not supported. With a steady arrival of records the store always refuses there is no bound at all on when a healthy key is stored, and memory grows without limit.
- Tombstone pruning stops on the whole node while any write is refused. For an outage that ends this is correct: the cleanup must not pass a delete the store did not get. A record the store never accepts stops it until the key is rewritten or the node restarts (TODO-828, TODO-824).
- A write-ahead-log frame the store refuses at startup, during recovery, is not retried this way: recovery logs it and goes on, and the node serves the previous value of that key until the next restart (TODO-823).

**WAL growth during an outage.** The write-ahead log is collected only up to each partition's applied watermark, and that watermark moves only past writes the store has accepted. While the store refuses writes nothing is collected, so the WAL directory keeps every frame written since the outage began and grows for as long as it lasts. Watch `topgun_wal_retained_bytes_total` and `topgun_wal_retained_bytes{partition}` — the bytes the WAL keeps (sealed segments plus the active one), present from the first scrape. Collection runs on a tick every 5 s: it seals a partition's active segment once it holds 8 MiB or its first frame is 30 s old, and unlinks sealed segments whose every frame is at or below the watermark the store holds. A tick that cannot read that watermark collects nothing, logs `WAL collection skipped: the applied watermarks could not be read` at `WARN` and counts `topgun_wal_gc_skipped_total{reason="watermark_read_failed"}`; the next tick tries again. Once the store has taken the buffered writes, the following ticks collect what they cover.

### When the write-behind flush task or the stall watchdog panics, or the redb store is broken (exit codes 70, 71 and 72)

A node whose write-behind flush task or stall watchdog has ended by a panic stops itself instead of running on with flushes it can no longer confirm (`TG-WB-006`): the store logs one error line (`write-behind background task panicked; the store refuses writes from now on`, with the task and the panic message) and counts it in `topgun_write_behind_background_task_panics_total{task}`; every write from then on is refused as a temporary error, so it is not acked and the client keeps it; `/health/ready` and `/healthz` answer 503 while `/health/live` stays 200, because the process is still running; the node then runs its ordinary shutdown — it drains what is still queued, in passes of the same 1000-entry / 8 MiB limits, each a durable commit, and syncs the WAL — and exits with code **70**. A drain commit the store refuses is halved down to single entries for as long as the shutdown deadline allows; a write the drain did not store is abandoned and stays in the WAL, replayable at the next start, and one refused write does not stop the drain of the others. A commit that has started is not cut, so a stop may overrun its deadline by one commit. If that shutdown itself panics or passes its deadline, the code is **71**. The code is non-zero whichever came first, the panic or a stop signal: a `SIGTERM` that arrives before the panic still ends in 70, never in 0; a stop with no such failure exits 0 as before. The batch the dead task was holding is not written by the dying node; on a WAL-backed store its frames stay un-applied and are replayed at the next start — on a store with no WAL those writes are gone. Two things follow for whoever runs the process. First, the failure path may take `30 s + TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS + 10 s` — **70 s with defaults** — before the process is gone, and the process supervisor has to tolerate that before it kills the node (a stop begun by a signal has no such deadline of its own). Second, the node needs a supervisor that restarts the process on a non-zero exit code (systemd `Restart=on-failure`, a Kubernetes restart policy, or the like): without one the node stays down and clients keep the refused writes on their side until it is started again. A panic that repeats at every start — one that comes from the inner store for a particular record and is met again during WAL replay, or a watchdog panic that recurs — gives a crash loop; nothing quarantines it today.

A node whose embedded redb store reports itself broken stops the same way, with exit code **72** (`TG-WB-010`). redb is broken for this process when it reports one of four causes: an I/O error (`io`, with its kind — `io_kind="StorageFull"` is a full disk), a previous I/O error (`previous_io`: after its first I/O error redb refuses every later write on the same handle, also once the cause is gone), corruption (`corrupted`) or a poisoned lock (`lock_poisoned`). Retrying is pointless on such a handle, so instead of retrying for ever the store logs one error line (`write-behind store: the durable store is broken and stores nothing more in this process; writes are refused from now on and the node stops`, with `cause`, `io_kind`, `error`, `map`, `key`, `site`), counts it in `topgun_write_behind_store_broken_total{cause}`, stops calling the store, refuses every new write as a temporary error, answers 503 on `/health/ready`, and runs the shutdown above, whose drain calls the store at most once more (`write-behind shutdown drain stopped at a broken store: …`, with `abandoned_entries`). The binary logs `the durable store is broken; shutting the node down` with `cause` and `exit_code=72`. A `SIGTERM` on a broken store with writes still buffered also exits 72. If the shutdown itself panics or passes its deadline the code is 71, as above; when a panic was recorded first, the code stays 70 (the first recorded failure wins). Nothing acknowledged is lost: the writes the store did not take stay un-applied in the WAL, and a restart gives redb a fresh handle and replays them. When the cause is gone — the disk has room again, a one-off read error — one restart heals the node. While the disk is still full, every restart exits with code **1** when it opens the store (redb writes to the data directory's `engine.redb` at every open), until the disk has room; a corrupted `engine.redb` also loops, with exit 1 at the open (or 72 when only a later flush meets the bad page), until the data directory is restored from a backup. Nothing in the process counts or delays these restarts: the supervisor's restart delay paces the loop — give it several seconds (systemd `RestartSec=` together with `StartLimitIntervalSec=`/`StartLimitBurst=`; Kubernetes paces it by `CrashLoopBackOff`). Every other store error is retried as described under "When the store refuses writes". Not shown: the full disk in the tests is injected below redb, not real; a WAL append on a really full disk aborts the process instead (exit 134, `TG-WAL-007`), which no test has observed. A latched handle with nothing buffered at `SIGTERM` exits 0, and a read of an unreadable page returns its error to the caller without stopping the node (TODO-838).

### Rolling a node BACK across OR delta framing (`TOPGUN_OR_DELTA_WAL`)

A node running the default `TOPGUN_OR_DELTA_WAL=true` writes OR delta frames. The WAL frame version is deliberately **not** bumped, so which binaries can still read that WAL is a question with a precise answer.

**Which rollback targets are safe.** The contract is **one-step**: any binary at or after the OR delta *reader* — the commit that taught recovery to fold `WalOp::OrDelta`, merged as **`feacb7d4`**, one change before this emitter — is a **safe rollback target**: it decodes both frame kinds and loses nothing. Only a **pre-reader** binary (built before `feacb7d4`) cannot deserialize a delta frame, so "an older binary cannot read this WAL" is a statement about pre-reader binaries and nothing else. `git merge-base --is-ancestor feacb7d4 <the binary's commit>` answers the question for a given build. A two-step downgrade to a pre-reader binary is **out of contract** — it is not claimed to work, and the paragraph below is what it does instead. Rolling *forward* is unaffected: a newer binary reads both frame kinds.

**What a pre-reader binary actually does, by frame position.** There are three cases and only one of them is silent, so the guarantee is NOT the unqualified "always fails closed" it is easy to assume:

- **Mid-segment** (any frame written to that partition after the delta, before rotation) — the deserialize failure is classified as `Corruption` and recovery **refuses to start**, naming the file and offset. Under mixed traffic this is the *ordinary* position, not an exotic one.
- **Trailing frame on a SEALED (non-active) segment** — reported as a truncated tail on a segment that is not the last, which is also corruption: recovery **refuses to start**.
- **Trailing frame on the ACTIVE segment** — the one silent case, and the likely position for the newest write on each partition. Recovery tolerates it, warns, replays the intact prefix, and `WalWriter`'s open path then **durably truncates the segment to that prefix**, so the mutation is physically gone rather than merely skipped.

A rollback to a pre-reader binary that must not hit either the refusal or the silent drop has to drain the WAL under the new binary first, or run with `TOPGUN_OR_DELTA_WAL=false` long enough for every delta frame already on disk to be applied and garbage-collected.

### Rolling a node BACK across the data-directory layout (`TOPGUN_DATA_DIR`)

Upgrading to this layout needs no step by hand: at its first start the server moves the store into a data directory once, keeps the old store file untouched under a new name, and from then on an older server binary refuses to start on that node instead of serving an empty store. A rollback is possible only as a return to that kept file, and it loses, on the server, every write acknowledged since the migration.

**Where the store lives.**

- `TOPGUN_DATA_DIR` names the store **directory**. It holds `engine.redb` and nothing else the server creates.
- `TOPGUN_REDB_PATH` (default `./topgun.redb`) no longer names the store. It names only the **legacy file** — the store file of earlier versions, which kept one redb table per map (`map__{name}`, `mapr__{name}` and their backups `map__{name}__backup`, `maprb__{name}`) — which is the source of the one-time migration, and the place of the marker file below.
- Default of the data directory: `TOPGUN_DATA_DIR` when it is set; else `{directory of TOPGUN_REDB_PATH}/topgun-data` when `TOPGUN_REDB_PATH` is set (so a node whose store sits on a volume stays on it); else `./topgun-data`. The container image and the Helm chart set both (`/data/topgun-data` beside `/data/topgun.redb`). A variable that is set but empty is refused at startup: `store location: {VAR} is set but empty`.
- In the log lines quoted in this section, `{D}` is the data directory, `{L}` the path `TOPGUN_REDB_PATH` names, and `{K}` the kept file `{L}.pre-migration`.
- At every start the server logs, at `INFO` and before any other store line, `store location: data directory {D}` with `data_dir` (absolute), `data_dir_source` (`TOPGUN_DATA_DIR`, `derived from TOPGUN_REDB_PATH` or `default`) and `legacy_path`.

**The marker file at the old path.** Whenever the server holds an open store and nothing is at `TOPGUN_REDB_PATH`, it writes a small text file there — on a fresh install, at the end of a migration, and when it opens an existing store with that path empty. Its first line is `TOPGUN-STORE-MOVED 1`; the rest says that the store moved to the data directory and points at the `Changed (on-disk format)` entry of `CHANGELOG.md`. It is written whole or not at all (a temporary file `{path}.stub-tmp`, synced, then renamed into place), so the path never holds an empty or partial file. It is not a database and the server never migrates it. **Do not delete it while a server binary older than this layout may be started with that path:** such a binary reads only `TOPGUN_REDB_PATH`; on the marker file its redb refuses the open, while on an absent path it would create an empty store and serve empty maps.

**The one-time migration.** A node whose `TOPGUN_REDB_PATH` holds a table-per-map store file and whose data directory is absent or empty migrates at start-up, inside the store open — before the write-ahead log is opened and before the server listens, so no client write can interleave. Nothing is run by hand and there is no separate command.

- It copies every row with its value bytes unchanged, verifies the copy (row counts per map, every value byte for byte, the set of maps), renames the old file to `{TOPGUN_REDB_PATH}.pre-migration`, writes the marker file at the old path, and then serves the store. WAL frames an older binary left un-applied are replayed afterwards into the new store by the normal recovery.
- Log lines, all `INFO`: `store format: migrating {L} into {D} from the table-per-map format` (`tables`, `bytes`); `store migration progress` at most every 10 s (`tables_done`, `tables_total`, `current_table`, `rows_copied_this_run`, `bytes_copied_this_run`); after an interruption `store migration: resuming from table {T}` (`tables_done`, `tables_total`) or `store migration: resuming at the verification of the copy`; at the end `store format: migration complete` (`tables`, `rows`, `elapsed_ms`, `kept_at`). While the kept file exists, every start logs `store format: the pre-migration store file is kept at {K}; delete it once this store is confirmed`.
- **Free space.** Before it creates or writes anything the migration needs `1.5 × the size of the old file + 64 MiB` free on the file system of the data directory (on a resume, less what the data directory already holds). Otherwise it refuses: `store migration: refusing to start: {available} bytes are free on the file system of {D}, {required} are needed (1.5 × the size of {L}, plus 64 MiB); nothing was changed`. While the kept file exists the node uses about twice the disk of the store.
- **Duration.** Measured on a Mac, on a table-per-map file of 2.0 GiB on disk (100 maps × 10 000 rows of 1 KiB values, about 0.96 GiB of values): 32 s, about 16 s per GiB of old file (34 s per GiB of values, 31 000 rows/s); the data directory came out at 1.0 × the size of the old file (both files grow in the same redb allocation steps, so the ratio is coarse). Time grows with the number of rows.
- **Resumable.** The copied rows and the mark that says how far the copy got are written in one batch, and a batch (at most 10 000 rows or 8 MiB of values) is synced to disk at least every 2 s. A run that is killed at any point resumes from its mark at the next start and ends with the same content as an uninterrupted one. A supervisor that restarts the node, for example on a failed health probe, therefore makes the migration converge, **provided each run lives longer than start-up plus 2 s plus two batches** — give a node that migrates a large store a start-up probe that allows at least that long.
- **Verification failure.** `store migration: verification failed for {table}: {first difference}; {D} is left unfinished and is refused until it is removed`. Every later start verifies again and refuses again; move the data directory away and start again to migrate anew from the old file, which is untouched.

**Two stores refuse the start.** If the data directory holds a store and `TOPGUN_REDB_PATH` again holds a table-per-map file — typically an old file restored from a backup, or put back by a rollback that left the data directory in place — the server refuses rather than silently serving one and leaving the other's writes out: `store location: two stores found: {D} holds a store in the data-directory format and {L} is a table-per-map store file; the server serves only one. Move one of them aside: rename {L} to keep the data-directory store, or move {D} away to migrate {L} again`. Every refusal at the store open — this one, an empty file at the old path, a kept `.pre-migration` file or a marker file with no store, a data directory that is neither a store nor empty, the two paths naming the same place — logs one `ERROR` line, exits 1, and creates, moves or deletes nothing; the old file, the kept file and the marker file stay byte for byte as they were, and the store's records are unchanged (its bytes may change: a refusal that reads the migration mark opens the store, and an open writes). The full list, each with what to do, is in the troubleshooting guide (`apps/docs-astro/src/content/docs/guides/troubleshooting.mdx`).

**What an older binary does.** A binary built before this layout reads only `TOPGUN_REDB_PATH`. Started with the same environment it meets the marker file; started with `TOPGUN_REDB_PATH` pointing at the data directory it meets a directory. Either way its redb refuses the open inside the store open — before it opens the WAL and before it listens — so it exits 1 (`Error: I/O error: invalid data` on the marker file), serves nothing and changes no file. This is tested for the binary built from `b346e277` (`map-name-rollback`, after a clean stop and after `kill -9` of the newer binary) and read from the source for the binary built from `07d009f0`, whose store open is the same `Database::create(TOPGUN_REDB_PATH)` before the WAL. Nothing is claimed about earlier binaries.

**Rollback is a return to the pre-migration file.**

1. Stop the new binary — it must exit 0 and log `write-behind shutdown: rollback hints written` with `drained=true`. At that stop it writes the rollback hints the older binary reads: one `partition-{p:03}.applied` file in the WAL directory for each WAL partition that has a segment or a watermark, each the applied watermark the data directory holds. Leave the WAL directory in place.
2. Move the data directory away.
3. Delete the marker file at `TOPGUN_REDB_PATH`.
4. Rename `{TOPGUN_REDB_PATH}.pre-migration` to `TOPGUN_REDB_PATH`.
5. Start the old binary.

- The restored store is the state at migration time: **every write the new binary acknowledged is absent from it, on the server.** No recovery path for those writes is claimed: a client drops an operation once the server acknowledged it, and LWW sync does not push client-held records to the server (TODO-653).
- There is **no tool** that exports the new format back to tables, and none is planned.
- A rollback after a `kill -9` of the new binary is out of contract.
- **After a crash, a non-zero exit, or a stop that logged `drained=false`** (the drain did not finish, for example because the store refused writes until the shutdown timeout ran out — that stop still exits 0), do not roll back yet: start the new binary again and stop it, until a stop exits 0 and its line reads `drained=true`. Keep the WAL directory in place throughout: the hints, and the frames the older binary replays above them, are in it. Without hints the older binary would replay the whole WAL onto the pre-migration file; after `drained=false` it would replay writes that were never stored onto it.
- **Rolling forward again:** with the data directory moved away, the new binary finds the old file and migrates it anew, including what the old binary wrote after the rollback. Started with the old data directory still in place it refuses (two stores, above), so writes made after the rollback are never silently left out.
- **Not claimed:** an older binary pointed at the data directory's `engine.redb` itself (`TOPGUN_REDB_PATH={D}/engine.redb`) is not protected — it opens that file as a table-per-map store with no maps and serves empty maps. Never point an older binary there.

**Backup.** Stop the server; the stop must exit 0 and log `write-behind shutdown: rollback hints written` with `drained=true` (after `drained=false`, start it again and stop it until the line reads `drained=true`). Then copy both directories together — the whole `TOPGUN_DATA_DIR` directory recursively and the WAL directory (`TOPGUN_WAL_DIR`) — and restore them together. A copy of a running server's directory is not a consistent backup. A data directory restored from an older backup than the WAL directory beside it is refused at start, before the server listens and with nothing written: `WAL directory check: refusing to start: the data directory is older than the WAL directory {wal_dir}: partition {p} has applied watermark {V} there, but the WAL was collected up to {G}; …`, or, for a data directory written by a binary that kept no applied watermark, `… the data directory holds no applied watermarks, but the WAL directory has served a store that keeps them. …`. Both end with `Restore the data directory and the WAL directory from the same backup, taken with the server stopped`. A WAL directory older than the data directory is not refused: frames at or below the data directory's watermark are never replayed, and new writes never reuse their sequence numbers (`TG-WAL-013`). **Before upgrading to this layout,** copy the `TOPGUN_REDB_PATH` file elsewhere: the migration keeps it as `{TOPGUN_REDB_PATH}.pre-migration`, but only a copy on another disk survives the loss of this one.

### Rolling a node BACK across the applied-watermark move

Each WAL partition's applied watermark — how far the WAL has been stored — now lives in the data directory (`engine.redb`), committed in the same batch as the records it covers, instead of the `.applied` files in the WAL directory. At its first start this binary imports those files into the data directory once, marks the store as imported, deletes the files, and writes `wal-collected` into the WAL directory. Collection reads the watermark from the data directory.

- **Older data-directory binaries refuse.** Once this binary has opened a store, the binary built from `6a030bb6`, and every data-directory binary before this one, started with the same environment exits non-zero at its store open — before it opens the WAL and before it listens — on metadata it does not know (`store format: refusing to open: unknown metadata …`, naming `wal_watermark`). It serves nothing and changes no file of the WAL directory. This is tested for `6a030bb6` (`watermark-rollback`, after a clean stop and after `kill -9` of the newer binary).
- There is **no tool** that takes the watermark back out of the data directory, and none is planned. The only way back to an older binary is the return to the pre-migration file described in the previous section, which goes to a binary built before the data-directory layout, not to `6a030bb6`.
- The `.applied` files a stop writes (see step 1 above) are rollback hints for that older binary. This binary reads one only when it imports — when the WAL directory holds no `wal-collected`, or the data directory has just been migrated from the old file — and deletes every one at its next start.

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
