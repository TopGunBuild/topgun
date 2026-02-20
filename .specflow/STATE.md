# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Project Phase:** Phase 2 (Rust Core)
- **TODO Items:** 29 (1 client bug fix + 9 Rust bridge/core + 5 audit findings + 14 existing deferred)
- **Next Step:** /sf:new or /sf:next
- **Roadmap:** See [TODO.md](todos/TODO.md) for full phase-based roadmap

## Queue

| Position | Spec | Title | Status | Phase |
|----------|------|-------|--------|-------|

## Migration Roadmap (high-level)

| Phase | Description | Status |
|-------|-------------|--------|
| **0. TypeScript Completion** | SPEC-048a/b/c (client cluster integration) | Complete |
| **1. Bridge** | Cargo workspace, CI, 6 upfront traits | Complete |
| **2. Rust Core** | CRDTs, message schemas, partitions | In Progress |
| **3. Rust Server** | Network, handlers, cluster, storage, tests | Not Started |
| **4. Rust Features** | Schema system, shapes, SSE, DAG, tantivy | Not Started |
| **5. Post-Migration** | AsyncStorage, S3, tiered, vector, extensions | Not Started |

See [TODO.md](todos/TODO.md) for detailed task breakdown with dependencies.

## Key Strategic Documents

| Document | Purpose |
|----------|---------|
| [TODO.md](todos/TODO.md) | Phase-based roadmap with all tasks and dependencies |
| [RUST_SERVER_MIGRATION_RESEARCH.md](reference/RUST_SERVER_MIGRATION_RESEARCH.md) | Technical migration strategy, 6 upfront traits |
| [PRODUCT_POSITIONING_RESEARCH.md](reference/PRODUCT_POSITIONING_RESEARCH.md) | Competitive analysis, schema strategy, partial replication |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-20 | SPEC-056 | COMPLETED: Implement Partition Hash and Partition Table in Rust. 1 commit. 3 audit cycles, 1 review cycle. `hash_to_partition`, `PartitionLookup` trait, `PartitionTable`, partition pruning. 33 new tests, 431 total passing. Zero clippy warnings. |
| 2026-02-20 | SPEC-056 | REVIEWED v1: APPROVED. All 11 acceptance criteria verified. 431 tests pass (414 unit + 10 integration + 7 doc-tests). Zero clippy warnings in partition.rs. 2 minor findings (informational only). |
| 2026-02-20 | SPEC-056 | EXECUTED: Implement Partition Hash and Partition Table in Rust. 1 commit (3 waves, single atomic). hash_to_partition, PartitionLookup trait, PartitionTable, partition pruning. 33 new tests, 431 total passing. Zero clippy warnings. |
| 2026-02-19 | SPEC-055 | COMPLETED: Fix Rust ORMap Merkle Hash Determinism. 2 commits. 2 audit cycles, 1 review cycle. `canonical_json` + `sort_json_value` private helpers replace `serde_json::to_string` in `hash_entry()`. 385 tests + 10 integration tests pass. Zero clippy warnings. |
| 2026-02-19 | SPEC-055 | REVIEWED v1: APPROVED. All 6 acceptance criteria verified. 385 unit tests + 10 integration tests + 6 doc tests pass. Zero clippy warnings. `canonical_json` + `sort_json_value` private helpers fix hash determinism for generic V. |
| 2026-02-19 | SPEC-052e | COMPLETED: Message Schema -- HTTP Sync, Message Union, and Cross-Language Tests. 5 commits. 4 audit cycles, 1 review cycle. 12 HTTP sync types, 77-variant Message enum, 61 golden fixtures, 9 integration tests. float64 deserialization fix for JS interop. 393 Rust tests + 62 TS tests. Zero clippy warnings. |
| 2026-02-19 | SPEC-052e | REVIEWED v1: APPROVED. All 8 acceptance criteria verified. 393 Rust tests + 62 TS tests pass, zero clippy warnings. 77-variant Message enum, 12 HTTP sync types, 61 golden fixtures. 3 minor findings -- all well-documented and non-blocking. |
| 2026-02-19 | SPEC-052e | EXECUTED: Message Schema -- HTTP Sync, Message Union, and Cross-Language Tests. 5 commits (3 waves). 12 HTTP sync types, 77-variant Message enum, 61 golden fixtures, 9 integration tests. float64 deserialization fix for JS interop. 393 Rust tests + 62 TS tests. Zero clippy warnings. |
| 2026-02-18 | SPEC-052d | COMPLETED: Message Schema -- Messaging and Client Events Domain Structs. 2 commits. 4 audit cycles, 1 review cycle. 44 types (33 messaging + 11 client events). 353 tests. All 7 AC pass. |
| 2026-02-18 | SPEC-052d | REVIEWED v1: APPROVED. All 7 acceptance criteria verified. 353 tests pass (72 new + 281 existing), zero clippy warnings. 44 types field-accurate against TS source. No critical, major, or minor issues found. |

## Project Patterns

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description`
- CRDTs use Hybrid Logical Clocks for causality tracking
- Handler extraction pattern: separate message handlers into focused modules with config injection
- Test polling pattern: use centralized test-helpers.ts with PollOptions for bounded iterations
- Late binding pattern: handlers can receive callbacks after construction via setXxxCallbacks methods
- Test harness pattern: ServerTestHarness provides controlled access to internal handlers for tests
- Timer cleanup pattern: handlers with timers implement stop() method, called by LifecycleManager during shutdown
- Message routing pattern: MessageRouter provides declarative type-based routing for server messages
- Module factory pattern: each domain gets its own factory function with explicit dependency injection
- Deferred startup pattern: module factories create resources but do not bind ports; start() method called after assembly
- Domain grouping pattern: handlers grouped by domain (CRDT, Sync, Query, Messaging, etc.) for Actor Model portability
- Client message handler pattern: ClientMessageHandlers module registers all client-side message types via registerClientMessageHandlers()
- React hook testing pattern: use renderHook + act from @testing-library/react with mock client wrapped in TopGunProvider
- Schema domain splitting pattern: organize schemas by domain (base, sync, query, search, cluster, messaging) with barrel re-exports
- DST infrastructure pattern: VirtualClock/SeededRNG/VirtualNetwork for deterministic simulation testing; injectable ClockSource via HLC for reproducible time
- HTTP sync transport pattern: HttpSyncProvider implements IConnectionProvider via message type routing in send(); AutoConnectionProvider provides WS-to-HTTP fallback; server uses setHttpRequestHandler() deferred wiring for POST /sync
- VM sandbox pattern: ProcessorSandbox fallback uses vm.Script + runInNewContext({ timeout }) for synchronous code interruption; isolated-vm primary path unchanged
- IConnection adapter pattern: IConnectionProvider returns IConnection interface (send/close/readyState) instead of concrete WebSocket; WebSocketConnection wraps WS, HttpConnection wraps HTTP queue; ConnectionReadyState constants avoid WebSocket global dependency
- Partition pruning pattern: PartitionService.getRelevantPartitions extracts key predicates (_key, key, id, _id) from queries to prune distributed fan-out; targetedNodes on DistributedSubscription keeps checkAcksComplete consistent with pruned node sets
- Node ID reconciliation pattern: ConnectionPool.addNode() matches incoming server-assigned nodeId against existing seed connections by endpoint; remapNodeId() transfers NodeConnection entry preserving WebSocket/state/pending; node:remapped event notifies ClusterClient and PartitionRouter
- Batch delegation pattern: IConnectionProvider.sendBatch is optional; SyncEngine delegates to it when available (cluster mode) for per-key partition routing; falls back to single OP_BATCH in single-server mode
- Double-option deserialization pattern: For Rust fields matching TS `.nullable().optional()`, use `Option<Option<T>>` with custom `deserialize_double_option` to distinguish absent (None) from explicitly-null (Some(None))
- Float64 numeric interop pattern: JavaScript's msgpackr encodes numbers > 2^32 as float64; use `serde_number::deserialize_u64` (from `hlc::serde_number`) on Rust u64/i64 fields to accept both integer and float64 MsgPack wire values

## Warnings

(none)

---
*Last updated: 2026-02-20 (SPEC-056 completed and archived)*
