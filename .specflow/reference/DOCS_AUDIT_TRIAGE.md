# Docs Audit Triage — SPEC-140 Pass 1

**Date:** 2026-03-24
**Method:** Quick sweep — read doc headers + key code exports, grep server-rust/client/core-rust for feature keywords, cross-reference TODO.md
**Pages audited:** 22 (excludes authentication.mdx, security.mdx, rbac.mdx — covered in SPEC-139)

---

## Triage Table

| Page | Feature Status | Existing TODO | TS Server Reference | Pass 2 Needed? | Notes |
|------|---------------|---------------|---------------------|----------------|-------|
| adaptive-indexing.mdx | DOES_NOT_EXIST | none | `packages/server/src/` — unknown, needs git check | NO | No grep hits in server-rust. HashIndex/NavigableIndex/InvertedIndex types absent. |
| adoption-path.mdx | EXISTS | none | N/A — migration guide | YES | Uses `cargo run --bin test-server`. API examples match TS client. Verify binary name and Tier 3 code examples. |
| cluster-client.mdx | EXISTS | none | N/A — client-only feature | YES | `ClusterClient.ts` + `PartitionRouter.ts` + `ConnectionPool.ts` exist in client. Check circuitBreaker config key name. |
| cluster-replication.mdx | PARTIAL | none | `packages/server/src/` — check replication config | YES | Partition count (271) is correct. ReplicationFactor/backup_count struct exists. Verify env var claims. |
| conflict-resolvers.mdx | PARTIAL (stubs) | none | `packages/server/src/` — had sandbox executor | NO (P2 deep-audit with caveat) | `ConflictResolver` struct + `RegisterResolver` message exist, but persistence.rs doc says "stub — WASM sandbox required". Server returns NotImplemented. This is a DOES_NOT_EXIST for functional purposes. |
| deployment.mdx | PARTIAL | TODO-141 | N/A — Rust server | YES | `deploy/Dockerfile.server` exists, binary is `test-server` (renamed to `topgun-server` in container). `TOPGUN_PORT`, `TOPGUN_TLS_*` env vars are NOT parsed by server — docs show planned env vars as current. Only `PORT`, `JWT_SECRET`, `DATABASE_URL`, `RUST_LOG`, `TOPGUN_LOG_FORMAT`, `TOPGUN_ADMIN_*` are actually read. |
| distributed-locks.mdx | DOES_NOT_EXIST (stubs) | TODO-171 (partial — RBAC, not locks) | `packages/server/src/coordinator/` | NO | `LockRequest`/`LockRelease` ops exist in coordination.rs but return NotImplemented (test: AC6). Feature is stub-only. |
| entry-processor.mdx | DOES_NOT_EXIST (stubs) | none | `packages/server/src/` | NO | `EntryProcess`/`EntryProcessBatch` ops classified and routed, but persistence.rs doc says "stub — WASM sandbox required". Returns NotImplemented for all calls. |
| event-journal.mdx | EXISTS | none | `packages/server/src/` | YES | `JournalStore` + `JournalSubscription` in `service/domain/journal.rs`. In-memory ring buffer with subscribe/read/append. Check if all API calls in docs match actual implementation. |
| full-text-search.mdx | EXISTS | none | N/A — Rust tantivy | YES | `SearchService` backed by tantivy. `search()` API exists on `TopGunClient`. `queryValues` call in docs is for local predicate queries, not full-text — may be wrong. Verify API method names. |
| index.mdx | INFORMATIONAL | none | N/A | NO | Guide index page. Descriptions accuracy check only. No code verification needed. |
| indexing.mdx | DOES_NOT_EXIST | none | `packages/server/src/` | NO | No grep hits for `HashIndex`, `NavigableIndex`, `InvertedIndex`, `IndexType`, `IndexRegistry` in server-rust or core-rust. Feature does not exist. |
| interceptors.mdx | DOES_NOT_EXIST | none | `packages/server/src/` | NO | No grep hits for `Interceptor` or `interceptor` in server-rust. Tower middleware pipeline exists but is internal — not user-extensible via an `Interceptor` API. |
| live-queries.mdx | EXISTS | none | N/A — TS client + Rust QueryService | YES | `client.query()` returns `QueryHandle`, `.subscribe()` and `.onChanges()` exist. `QueryService` in server-rust. Verify predicate API, cluster query claims. |
| mcp-server.mdx | EXISTS | none | N/A — TS package | YES | `@topgunbuild/mcp-server` package exists with `TopGunMCPServer` class. `topgun-mcp` CLI binary exists. Check `httpTransportCode` uses `topgun-mcp --http` — CLI binary exists. Programmatic API matches. |
| observability.mdx | PARTIAL | TODO-137 | N/A — Rust tracing | YES | `MetricsLayer` + `/metrics` endpoint exists. `topgun_active_connections` gauge and `topgun_operations_total` counter exist. BUT: metric names in docs (`topgun_connected_clients`, `topgun_ops_total`) may not match actual code (`topgun_active_connections`, `topgun_operations_total`). Log format is JSON not human-readable (Rust tracing vs Node.js pino). |
| performance.mdx | PARTIAL | none | N/A — benchmarks | YES | Load harness results documented. Tuning knobs (`gc_interval_ms`, `max_concurrent_operations`, `outbound_channel_capacity`) exist in `ServerConfig`/`ConnectionConfig`. But no env var support for these yet — docs may show non-existent config keys. |
| pn-counter.mdx | PARTIAL | none | `packages/server/src/` | YES | `PNCounterState`, `CounterRequest`, `CounterSync`, `CounterResponse` messages exist. `PersistenceService` handles `CounterRequest`. `TopGunClient.getCounter()` exists. Verify TS client API (increment/decrement calls) match docs. |
| postgresql.mdx | EXISTS | none | N/A — Rust PostgresDataStore | YES | `PostgresDataStore` in `server-rust`. Table schema `topgun_maps` is accurate. `DATABASE_URL` env var works. `cargo run --bin test-server` accurate. Verify all schema columns match docs. |
| pub-sub.mdx | EXISTS | none | N/A — Rust MessagingService + TS TopicHandle | YES | `MessagingService` + `TopicRegistry` in server-rust. `client.topic(name)` returns `TopicHandle` with `.publish()` and `.subscribe()`. API in docs matches code. |
| ttl.mdx | PARTIAL | none | `packages/server/src/` | YES | `ExpiryPolicy` with `ttl_millis` exists in `record_store.rs`. `DefaultRecordStore` implements `has_expired()` and `evict_expired()`. Background GC via `gc_interval_ms`. But: no client API for setting TTL per-key yet — TS client `set()` has no `ttl` option. Check if `ttl_ms` field in CRDT ops is wired up client-side. |
| write-concern.mdx | PARTIAL | none | N/A — Rust + TS | YES | `WriteConcern` enum (FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED) exists in core-rust. Server accepts `write_concern` in `OpBatch`. BUT: `achieved_level` is always `None` in server responses (crdt.rs lines 197, 267) — server never reports what level was actually achieved. `setWithAck()` and `batchSet()` not found in TS client. |

---

## Summary Statistics

- **EXISTS (fully implemented):** 7 pages (adoption-path, event-journal, live-queries, mcp-server, postgresql, pub-sub, cluster-client)
- **PARTIAL (some gaps):** 8 pages (cluster-replication, deployment, observability, performance, pn-counter, ttl, write-concern, conflict-resolvers)
- **DOES_NOT_EXIST:** 7 pages (adaptive-indexing, distributed-locks, entry-processor, indexing, interceptors — plus conflict-resolvers and index.mdx is informational)

---

## Pages Skipping Pass 2 (feature does not exist or is fully stub)

These pages have verdict `major-rewrite` based on Pass 1 alone.
Fix approach: Add planned-feature banner, simplify to feature overview, remove "how to use" code examples.

| Page | Reason | Fix Priority | Existing TODO |
|------|--------|--------------|---------------|
| adaptive-indexing.mdx | No implementation in server-rust (no HashIndex/NavigableIndex/InvertedIndex) | P1 | TODO-174 (new) |
| distributed-locks.mdx | LockRequest/LockRelease return NotImplemented — AC6 in coordination.rs tests confirms stub | P1 | TODO-175 (new) |
| entry-processor.mdx | EntryProcess returns NotImplemented — "stub — WASM sandbox required" per persistence.rs comment | P1 | TODO-176 (new) |
| indexing.mdx | No HashIndex, NavigableIndex, or InvertedIndex types in server-rust or core-rust | P1 | TODO-177 (new) |
| interceptors.mdx | No user-facing Interceptor API — Tower pipeline is internal only | P1 | TODO-178 (new) |
| conflict-resolvers.mdx | RegisterResolver returns NotImplemented — "stub — WASM sandbox required" per persistence.rs | P1 | TODO-179 (new) |
| index.mdx | Informational only — descriptions accuracy check | P3 | none |

---

## Pages Entering Pass 2 (feature exists or partial)

Assign to G1, G2, G3 workers for deep line-by-line audit.

### G1: Infrastructure Pages
1. **deployment.mdx** — verify env vars (TOPGUN_PORT, TOPGUN_TLS_* are PLANNED not current), Dockerfile, binary name
2. **postgresql.mdx** — verify table schema columns, DATABASE_URL, cargo run --bin
3. **performance.mdx** — verify tuning knobs, benchmark numbers, config keys
4. **observability.mdx** — verify metric names, log format (JSON vs structured), endpoint path

### G2: Cluster + Sync Pages
1. **cluster-replication.mdx** — verify partition count (271 ✓), backup_count/replication_factor field names, env vars
2. **cluster-client.mdx** — verify TopGunClusterConfig fields (circuitBreaker key), ClusterClient API
3. **live-queries.mdx** — verify client.query() API, predicate names, QueryHandle methods
4. **pub-sub.mdx** — verify client.topic() API, TopicHandle.publish()/subscribe(), ephemeral behavior claim

### G3: Remaining Implemented + Partial Pages
1. **full-text-search.mdx** — verify SearchService API, client.search() vs client.query() confusion
2. **adoption-path.mdx** — verify Tier 1/2/3 code examples, binary name references
3. **mcp-server.mdx** — verify programmatic API (TopGunMCPServer constructor options), CLI flags
4. **pn-counter.mdx** — verify TopGunClient.getCounter(), increment/decrement API
5. **ttl.mdx** — verify client-side TTL setting (ttl_ms field), ExpiryPolicy wiring
6. **write-concern.mdx** — verify achieved_level always None, setWithAck() existence
7. **event-journal.mdx** — verify journal subscribe/read API against JournalStore

---

## New TODOs Created

The following TODO entries are created for unimplemented features with no existing TODO:

| TODO | Feature | Page | Priority |
|------|---------|------|----------|
| TODO-174 | Adaptive Indexing | adaptive-indexing.mdx | P3 |
| TODO-175 | Distributed Locks | distributed-locks.mdx | P2 |
| TODO-176 | Entry Processor | entry-processor.mdx | P2 |
| TODO-177 | Indexing (Hash/Navigable/Inverted) | indexing.mdx | P2 |
| TODO-178 | Interceptors | interceptors.mdx | P3 |
| TODO-179 | Conflict Resolvers | conflict-resolvers.mdx | P2 |

Notes:
- `distributed-locks.mdx` — there is no existing TODO for locks specifically (TODO-171 is RBAC). New TODO-175 covers locks.
- `conflict-resolvers.mdx` — the wire messages exist (ConflictResolver struct, RegisterResolver op) but the sandbox executor that runs the code does not exist. The feature appears functional in docs but returns NotImplemented at runtime.
- `entry-processor.mdx` — same situation as conflict-resolvers: wire protocol exists, execution engine doesn't.
- `write-concern.mdx` — write concern is PARTIAL: wire protocol and enum exist, but server never reports `achieved_level` in OP_ACK responses (always `None`). `setWithAck()` not found in TS client. Need new TODO for client API.

---

## Additional Observations for Pass 2 Workers

### deployment.mdx critical issues
- `TOPGUN_PORT`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_TLS_*`, `TOPGUN_CLUSTER_TLS_*` env vars shown as current — actually PLANNED (not parsed by test-server binary)
- Only actual env vars: `PORT`, `JWT_SECRET`, `DATABASE_URL`, `RUST_LOG`, `TOPGUN_LOG_FORMAT`, `TOPGUN_ADMIN_USERNAME`, `TOPGUN_ADMIN_PASSWORD`
- `deploy/Dockerfile.server` EXISTS and builds `test-server` binary, renamed to `topgun-server` in container — partially accurate
- `deploy/k8s/chart` exists — Helm chart claim is accurate

### observability.mdx metric name mismatch
- Docs: `topgun_connected_clients` — Actual: `topgun_active_connections`
- Docs: `topgun_ops_total` — Actual: `topgun_operations_total` (with labels `service`, `outcome`)
- Docs: `topgun_map_size_items` — Actual: NOT FOUND in grep results
- Log format shows Node.js pino JSON (`"level": 30`) — actual is Rust tracing JSON (`TOPGUN_LOG_FORMAT=json`)

### write-concern.mdx critical issues
- Server always returns `achieved_level: None` in OpAck (crdt.rs lines 197, 267)
- `setWithAck()` method not found in TS client (`TopGunClient.ts`) — feature may not exist
- `batchSet()` not found in TS client
- Wire protocol and enum values exist — this is a partial implementation

### conflict-resolvers.mdx major issue
- `ConflictResolver` struct + `RegisterResolver` message exist in core-rust
- But `handle_register_resolver()` in persistence.rs returns `NotImplemented` response
- Comment in persistence.rs: "stub — WASM sandbox required"
- All code examples in docs would compile but produce NotImplemented runtime error
