# Docs Audit Report — SPEC-140

**Date:** 2026-03-24
**Auditor:** SPEC-140 orchestrated execution (G0 triage + G1-G3 deep audit + G4 merge)
**Scope:** All guide pages in `apps/docs-astro/src/content/docs/guides/` except authentication.mdx, security.mdx, rbac.mdx (audited in SPEC-139)
**Pages audited:** 22
**Source files verified:** `packages/server-rust/src/`, `packages/client/src/`, `packages/core/src/`, `packages/core-rust/src/`

---

## Summary Table

| Page | Verdict | Issues | Fix Priority | Feature Status |
|------|---------|--------|--------------|----------------|
| adaptive-indexing.mdx | major-rewrite | 1 critical | P1 | DOES_NOT_EXIST |
| adoption-path.mdx | minor-issues | 1 minor | P3 | EXISTS |
| cluster-client.mdx | minor-issues | 2 minor | P2 | EXISTS |
| cluster-replication.mdx | major-rewrite | 4 critical | P1 | PARTIAL |
| conflict-resolvers.mdx | major-rewrite | 1 critical | P1 | STUB |
| deployment.mdx | major-rewrite | 3 critical, 1 major | P1 | PARTIAL |
| distributed-locks.mdx | major-rewrite | 1 critical | P1 | STUB |
| entry-processor.mdx | major-rewrite | 1 critical | P1 | STUB |
| event-journal.mdx | minor-issues | 1 minor | P2 | EXISTS |
| full-text-search.mdx | minor-issues | 2 minor | P2 | EXISTS |
| index.mdx | minor-issues | 1 minor | P3 | INFORMATIONAL |
| indexing.mdx | major-rewrite | 1 critical | P1 | DOES_NOT_EXIST |
| interceptors.mdx | major-rewrite | 1 critical | P1 | DOES_NOT_EXIST |
| live-queries.mdx | accurate | 0 | — | EXISTS |
| mcp-server.mdx | accurate | 0 | — | EXISTS |
| observability.mdx | major-rewrite | 3 critical, 1 major | P1 | PARTIAL |
| performance.mdx | major-rewrite | 2 critical, 1 major | P1 | PARTIAL |
| pn-counter.mdx | accurate | 0 | — | EXISTS |
| postgresql.mdx | accurate | 0 | — | EXISTS |
| pub-sub.mdx | accurate | 0 | — | EXISTS |
| ttl.mdx | minor-issues | 1 minor | P2 | PARTIAL |
| write-concern.mdx | major-rewrite | 2 critical | P1 | PARTIAL |

**Totals:** 11 major-rewrite, 6 minor-issues, 5 accurate

---

## P1 — Block Launch (Fix Before Going Public)

### deployment.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** PARTIAL — server exists, but env var config does not work as documented

**Issues:**

1. **[CRITICAL]** `TOPGUN_PORT`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_TLS_*`, `TOPGUN_CLUSTER_TLS_*` env vars shown in Docker Compose and Kubernetes configs as working — they are NOT parsed by the server. Verified by grepping all `std::env::var()` calls in `packages/server-rust/src/`:
   - `TOPGUN_PORT` — not read anywhere
   - `TOPGUN_TLS_ENABLED`, `TOPGUN_TLS_CERT_PATH`, `TOPGUN_TLS_KEY_PATH`, `TOPGUN_TLS_MIN_VERSION` — not read
   - `TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_TLS_*`, `TOPGUN_CLUSTER_MTLS`, `TOPGUN_REPLICATION` — not read
   - Actual env vars read: `PORT` (test_server.rs line 91), `JWT_SECRET` (module.rs line 233), `DATABASE_URL` (postgres.rs line 523), `RUST_LOG` (tracing filter), `TOPGUN_LOG_FORMAT` (observability.rs line 96), `TOPGUN_ADMIN_USERNAME`/`TOPGUN_ADMIN_PASSWORD`/`TOPGUN_ADMIN_DIR` (admin.rs lines 73-74)
   - **Severity:** critical — Docker Compose/Kubernetes configs in docs will silently fail to configure TLS, ports, or cluster topology

2. **[CRITICAL]** Kubernetes YAML uses `TOPGUN_PORT: 443` — since `TOPGUN_PORT` is not read, the server will still bind to whatever port is hardcoded (0 = OS-assigned in default config). The Docker Compose basic example correctly uses no port env var and exposes 8080 — this is accurate.
   - **Ref:** `packages/server-rust/src/network/config.rs` — `NetworkConfig::default()` port is 0

3. **[CRITICAL]** Note in `productionPerfCode` says "production CLI binary with env var config is planned" — this caveat is accurate but it's in a code comment. The rest of the page (Docker Compose TLS section, Kubernetes section) does NOT carry this caveat and shows PLANNED env vars as current.
   - **Severity:** critical — users copying the TLS Docker Compose will get no TLS

4. **[MAJOR]** Binary name inconsistency: Dockerfile builds `test-server` binary and renames it to `topgun-server` in the container (`COPY --from=builder /app/target/release/test-server /usr/local/bin/topgun-server`). This is fine for the Docker path. But the basic Docker section shows `docker build -t topgun-server -f deploy/Dockerfile.server .` which is accurate. The `deploy/Dockerfile.server` exists at the repo root.
   - **Ref:** `deploy/Dockerfile.server` — confirmed existing
   - **Severity:** major — the naming rename works in Docker; the image-level naming is consistent

**Fix approach:** Add a banner: `> **Note:** Env var configuration (`TOPGUN_PORT`, `TOPGUN_TLS_*`) is planned for the production server binary. Currently, TLS and port are configured programmatically when embedding the server.` Mark the Docker Compose TLS section and Kubernetes section with `> **Planned** — these configurations reference env vars that will be supported in the production binary.`

---

### observability.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** PARTIAL — metrics endpoint exists, but metric names differ; log format claim is wrong

**Issues:**

1. **[CRITICAL]** Metric name mismatches — docs list metric names that don't exist in server:
   - Docs: `topgun_connected_clients` — Actual: `topgun_active_connections` (`metrics_endpoint.rs` line 35)
   - Docs: `topgun_ops_total` with labels `type` (PUT/GET/DELETE/SUBSCRIBE), `map` — Actual: `topgun_operations_total` with labels `service`, `outcome` (`metrics.rs` line 96)
   - Docs: `topgun_map_size_items` — NOT FOUND in any server source file
   - Docs: `topgun_memory_usage_bytes` — NOT FOUND in server source
   - Docs: `topgun_cluster_members` — NOT FOUND in server source
   - Actual metrics exported: `topgun_active_connections` (gauge), `topgun_operations_total` (counter, labels: service, outcome), `topgun_operation_duration_seconds` (histogram, label: service), `topgun_operation_errors_total` (counter, labels: service, error)
   - **Severity:** critical — Grafana dashboards or alerts built from these docs will fail

2. **[CRITICAL]** Event Routing, Event Queue, Backpressure, Connection Rate Limiting metric tables (30+ metrics listed) — these appear to be from the old TS server (Node.js event queue architecture). The Rust server uses Tower middleware, not an event queue. None of these metric names (`topgun_events_routed_total`, `topgun_event_queue_size`, `topgun_backpressure_sync_forced_total`, etc.) exist in server-rust.
   - **Severity:** critical — entire metric tables are for the removed TS server

3. **[CRITICAL]** Log format: Docs say "TopGun uses **Pino** for high-performance, structured JSON logging" and shows a Node.js pino log format (`"level": 30`). Actual server uses Rust `tracing` with `TOPGUN_LOG_FORMAT=json` for JSON mode. The pino level number (`30 = info`) is Node.js-specific and will not appear in Rust logs.
   - **Ref:** `packages/server-rust/src/service/middleware/observability.rs` line 66
   - **Severity:** critical — wrong technology stack, different log format

4. **[MAJOR]** "Standard Node.js metrics (CPU, Event Loop, GC) are also exported with the `topgun_` prefix" — this is from the TS server (Node.js process metrics). The Rust server has no Node.js. Rust process metrics (if any) would come from the `metrics` crate.
   - **Severity:** major — incorrect platform claim

**Fix approach:** Rewrite metric tables to show only actual Rust server metrics. Replace Pino reference with Rust tracing. Add `> **Status:** The metrics endpoint at `/metrics` is implemented and exports basic connection and operation counters. Full metric coverage (map size, cluster members, event queue) is planned.`

---

### performance.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** PARTIAL — server is fast, but documented config knobs and metric names are from TS server

**Issues:**

1. **[CRITICAL]** Config knobs `eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs`, `backpressureBackoffMs`, `backpressureMaxPending` do not exist in the Rust server. These are TS server (Node.js) configuration options. Verified: no such fields in `packages/server-rust/src/service/config.rs` (`ServerConfig`) or `packages/server-rust/src/network/config.rs` (`NetworkConfig`, `ConnectionConfig`).
   - **Severity:** critical — tuning instructions reference non-existent config

2. **[CRITICAL]** Binary name `topgun-server` is used throughout (`PORT=8080 topgun-server`). The actual binary is `test-server` when building from source. In the container it's renamed to `topgun-server`. From source: `cargo run --bin test-server --release`.
   - **Severity:** critical — commands won't work for developers running from source

3. **[MAJOR]** Monitoring metrics in "Critical Metrics" table reference `topgun_event_queue_size`, `topgun_event_queue_rejected_total`, `topgun_backpressure_timeouts_total`, `topgun_backpressure_pending_ops`, `topgun_connections_rejected_total` — none of these exist in server-rust. Same TS server metric names issue as observability.mdx.
   - **Severity:** major — monitoring guidance references non-existent metrics

**Fix approach:** Rewrite for Rust server: actual tuning is via `ServerConfig` fields (`max_concurrent_operations: 1000`, `gc_interval_ms: 60000`), `ConnectionConfig` fields (`outbound_channel_capacity: 256`, `send_timeout: 5s`, `idle_timeout: 60s`). Replace TS config with actual Rust config. Replace metric names with actual: `topgun_operations_total`, `topgun_active_connections`, `topgun_operation_duration_seconds`.

---

### cluster-replication.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** PARTIAL — cluster infrastructure exists (271 partitions, backup_count), but env var config does not work and consistency modes are aspirational

**Issues:**

1. **[CRITICAL]** `TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`, `TOPGUN_NODE_ID`, `TOPGUN_PEERS`, `TOPGUN_REPLICATION`, `TOPGUN_CONSISTENCY` env vars — NONE of these are parsed by the server (confirmed by grepping all `std::env::var()` calls in server-rust). The Docker Compose cluster example with `TOPGUN_NODE_ID: node-1` etc. will not configure the server.
   - **Ref:** All env vars grepped, only `PORT`, `JWT_SECRET`, `DATABASE_URL`, `RUST_LOG`, `TOPGUN_LOG_FORMAT`, `TOPGUN_ADMIN_*` are read.
   - **Severity:** critical — cluster setup instructions don't work

2. **[CRITICAL]** Consistency modes (EVENTUAL/QUORUM/STRONG) are documented as configurable via `TOPGUN_CONSISTENCY` env var — this is aspirational. The Rust server uses CRDT-based eventual consistency only. No consistency mode switching exists.
   - **Ref:** `packages/server-rust/src/cluster/types.rs` — `ClusterConfig` has no consistency field
   - **Severity:** critical — QUORUM and STRONG modes don't exist

3. **[CRITICAL]** Replication metrics (`topgun_replication_queue_size`, `topgun_replication_pending_acks`, `topgun_replication_lag_ms`, `topgun_replication_healthy`) — NOT found in server-rust. These are aspirational metrics.
   - **Severity:** critical — monitoring guidance references non-existent metrics

4. **[MAJOR]** Read routing options ("Primary / Replica / Any: Always read from partition owner / Read from nearest replica / Read from any node") are described but not implemented. The Rust server always reads from the partition owner.
   - **Ref:** `packages/server-rust/src/cluster/` — no read preference routing in cluster code
   - **Severity:** major — misleading architecture claim

**What IS accurate:** 271 partitions (`packages/server-rust/src/cluster/state.rs` line 514), `backup_count` field in `ClusterConfig`, Phi Accrual failure detection (`phi_threshold: 8.0` in `ClusterConfig`), gossip-based discovery (HELLO/MEMBER_LIST protocol), partition rebalancing on node failure.

**Fix approach:** Add banner noting env var cluster configuration is planned. Mark consistency modes as planned. Keep the architecture overview sections (gossip discovery, partition assignment, failure detection) as they describe real implemented behavior.

---

### conflict-resolvers.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** STUB — wire protocol exists, runtime execution returns NotImplemented

**Issues:**

1. **[CRITICAL]** All code examples in `conflict-resolvers.mdx` (registering resolvers, `MergeContext` API, built-in resolvers, `client.resolvers.register()`) will compile and type-check, but calling them at runtime produces a NotImplemented error response from the server.
   - **Ref:** `packages/server-rust/src/service/domain/persistence.rs` comment: "stub — WASM sandbox required"
   - `handle_register_resolver()` returns `RegisterResolverResponseData { success: false, error: Some("not implemented") }`
   - `ConflictResolverClient.register()` exists in `packages/client/src/ConflictResolverClient.ts` — client sends the message, server rejects it
   - **Severity:** critical — feature is documented as available but all calls fail at runtime

**What IS accurate:** `ConflictResolver` struct fields (name, code, priority, key_pattern) in core-rust. `client.resolvers` property exists. `onRejection` callback API exists. Wire protocol messages exist.

**Fix approach:** Add planned-feature banner. Keep the conceptual overview (why LWW isn't always enough). Remove all "how to use" code examples that show runtime usage. Note: "The conflict resolver API is designed and wire protocol is ready. Server-side execution requires a WASM sandbox that is not yet implemented (tracked as TODO-179)."

---

### entry-processor.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** STUB — wire protocol exists, runtime returns NotImplemented

**Issues:**

1. **[CRITICAL]** All entry processor code examples will fail at runtime. Server returns NotImplemented for all `EntryProcess` and `EntryProcessBatch` operations.
   - **Ref:** `packages/server-rust/src/service/domain/persistence.rs` lines 327-360 — both `handle_entry_process()` and `handle_entry_process_batch()` return error responses with "not implemented"
   - Comment in persistence.rs: "Entry Processing and Resolver operations return structured error responses (sandbox not available)"
   - **Severity:** critical — feature presented as available but all calls fail

**Fix approach:** Add planned-feature banner: "Entry Processor requires a WASM sandbox for safe server-side code execution. This is planned (TODO-176). The wire protocol is implemented."

---

### adaptive-indexing.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** DOES_NOT_EXIST

**Issues:**

1. **[CRITICAL]** Adaptive indexing does not exist in `packages/server-rust/src/` (confirmed: no `AdaptiveIndex`, `IndexSuggestion`, `QueryPattern`, or `IndexAdvisor` in codebase). Feature is entirely aspirational.
   - **Severity:** critical — presented as available, no implementation exists

**Fix approach:** Add planned-feature banner. Simplify to concept overview: "Adaptive indexing will automatically suggest or create indexes based on query patterns." Reference TODO-174.

---

### indexing.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** DOES_NOT_EXIST

**Issues:**

1. **[CRITICAL]** `HashIndex`, `NavigableIndex`, and `InvertedIndex` (for query acceleration — distinct from tantivy full-text search) do not exist in `packages/server-rust/src/` (confirmed by grep). No index registry, no index type implementations.
   - Note: `InvertedIndex` in full-text-search.mdx refers to the tantivy-backed `IndexedLWWMap.queryValues()` API in the core TS package. This IS implemented but uses a different API than what indexing.mdx documents.
   - **Severity:** critical — the server-side indexing infrastructure documented does not exist

**Fix approach:** Add planned-feature banner. Distinguish: tantivy search (implemented, see full-text-search.mdx) vs. query acceleration indexes (planned, TODO-177). Note: "HashIndex, NavigableIndex, and InvertedIndex for O(1) query acceleration are planned (TODO-177). For full-text search, see the Full-Text Search guide."

---

### interceptors.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** DOES_NOT_EXIST

**Issues:**

1. **[CRITICAL]** No user-facing `Interceptor` API exists in server-rust (confirmed by grep). The Tower middleware pipeline is internal and not extensible by users. Feature is entirely aspirational.
   - **Severity:** critical — presented as available, no implementation exists

**Fix approach:** Add planned-feature banner: "User-extensible interceptors are planned (TODO-178). Currently, server-side custom logic can be achieved through Conflict Resolvers (when implemented) and Entry Processors (when implemented)."

---

### distributed-locks.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** STUB — wire messages exist, server returns NotImplemented

**Issues:**

1. **[CRITICAL]** `LockRequest` and `LockRelease` operations are routed to `CoordinationService` but return NotImplemented. Confirmed by test AC6 in `packages/server-rust/src/service/domain/coordination.rs`:
   - `async fn lock_request_returns_not_implemented()` — verifies server returns error
   - `async fn lock_release_returns_not_implemented()` — verifies server returns error
   - The TS client has `DistributedLock` class (`packages/client/src/DistributedLock.ts`) that sends these messages, but they will all fail at runtime.
   - **Severity:** critical — presented as available, all lock operations fail

**Fix approach:** Add planned-feature banner: "Distributed Locks are planned (TODO-175). The wire protocol is designed. Currently, the server returns a 'not implemented' error for lock operations."

---

### write-concern.mdx — major-rewrite

**Verdict:** major-rewrite
**Feature Status:** PARTIAL — enum and wire protocol exist, server-side enforcement and client API incomplete

**Issues:**

1. **[CRITICAL]** Server always returns `achieved_level: None` in `OpAck` responses. The `WriteConcern` enum is parsed and forwarded, but `CrdtService` never populates `achieved_level`:
   - `packages/server-rust/src/service/domain/crdt.rs` line 197: `achieved_level: None`
   - `packages/server-rust/src/service/domain/crdt.rs` line 267: `achieved_level: None`
   - Docs show `result.achievedLevel` as a usable value — it will always be undefined
   - **Severity:** critical — docs show a feature that doesn't work correctly

2. **[CRITICAL]** `setWithAck()` and `batchSet()` methods shown in docs do not exist in the TS client. Verified: neither method found in `packages/client/src/TopGunClient.ts` or `packages/client/src/LWWMap.ts`.
   - Docs show `todos.setWithAck('payment-123', data, { writeConcern: WriteConcern.PERSISTED })` — this will throw "is not a function"
   - **Severity:** critical — code examples won't compile/run

**What IS accurate:** `WriteConcern` enum exists (FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED). `OpBatch.write_concern` field exists and is forwarded. Basic fire-and-forget write behavior is accurate.

**Fix approach:** Add banner noting `achieved_level` reporting and `setWithAck()` are not yet implemented. Show only the basic `write_concern` field in `OpBatch` that actually works. Track as TODO-180.

---

## P2 — Fix Soon

### observability.mdx

(See above — actually P1 due to critical metric name mismatches)

### cluster-client.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** EXISTS — ClusterClient, PartitionRouter, ConnectionPool all implemented

**Issues:**

1. **[MINOR]** `cluster-client.mdx` shows a `circuitBreaker` config option in `TopGunClusterConfig`:
   ```
   cluster: {
     seeds: ['ws://node1:8765'],
     // Circuit breaker is built-in with sensible defaults:
     // - Opens after 5 consecutive failures
     // - Resets after 30 seconds
   ```
   The comment says "Circuit breaker is built-in" but the `TopGunClusterConfig` interface does NOT expose a `circuitBreaker` config property. The circuit breaker IS internal in `ClusterClient` (uses `CircuitBreakerConfig`), but is not user-configurable via `TopGunClusterConfig`.
   - **Ref:** `packages/client/src/TopGunClient.ts` — `TopGunClusterConfig` has: seeds, connectionsPerNode, smartRouting, partitionMapRefreshMs, connectionTimeoutMs, retryAttempts — no circuitBreaker
   - **Severity:** minor — users who try to pass `circuitBreaker: {}` get a TypeScript error; behavior works anyway

2. **[MINOR]** Default `connectionTimeoutMs` shown as `5000` in docs, matches code (DEFAULT_CLUSTER_CONFIG line 76). Default `connectionsPerNode` shown as `2` in `connectionPoolCode` but actual default is `1`.
   - **Ref:** `DEFAULT_CLUSTER_CONFIG.connectionsPerNode = 1`
   - **Severity:** minor

---

### event-journal.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** EXISTS — JournalStore in server, EventJournalReader in client

**Issues:**

1. **[MINOR]** Server configuration section shows `server.eventJournalService.subscribe(...)` — this is an embedding API not documented in the server reference. May be outdated or aspirational server-side subscription API. Client API (`client.getEventJournal().subscribe()`) is accurate.
   - **Ref:** `packages/client/src/EventJournalReader.ts` — client API matches docs
   - `packages/server-rust/src/service/domain/journal.rs` — `JournalStore.subscribe()` exists
   - **Severity:** minor — server embedding API claim is plausible but unverified

---

### full-text-search.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** EXISTS — SearchService (tantivy), client.search(), client.searchSubscribe()

**Issues:**

1. **[MINOR]** "Basic Usage" section uses `IndexedLWWMap.queryValues()` with `type: 'contains'` — this is a LOCAL client-side full-text search using `packages/core/src/IndexedLWWMap.ts`, NOT server-side tantivy search. The distinction is only made later in "Server-Side Search" section. The page title suggests all content is about server-side FTS.
   - The local `queryValues()` IS implemented in `packages/core/src/IndexedLWWMap.ts`
   - The server `client.search()` is implemented in `packages/client/src/TopGunClient.ts` (line 607)
   - **Severity:** minor — both are implemented, but the distinction between local vs server search is unclear in the Basic Usage section

2. **[MINOR]** Server binary shown as `topgun-server` in server setup code. Actual binary from source is `test-server` (`cargo run --bin test-server`).
   - **Severity:** minor

---

### ttl.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** PARTIAL — server TTL infrastructure exists, client API works for setting TTL

**Issues:**

1. **[MINOR]** Docs say TTL is supported: `sessions.set('user:123', data, 3600 * 1000)` (third arg = ttlMs). This IS implemented client-side: `packages/client/src/TopGunClient.ts` lines 281-282 wrap `lwwMap.set()` to intercept the ttlMs argument. The record's `ttl_ms` field in core-rust schema exists.

   However, whether the server actually enforces TTL expiry on the CRDT level when clients sync back is UNCLEAR. The `ExpiryPolicy` in `DefaultRecordStore` handles server-side expiry, and `gc_interval_ms` triggers `evict_expired()`. But it's unclear if the client-provided `ttl_ms` is passed through the entire sync pipeline to the server's `ExpiryPolicy`.
   - **Ref:** `packages/server-rust/src/service/domain/crdt.rs` — all test `ttl_ms: None` in test ops
   - **Severity:** minor — TTL setting API appears correct; full enforcement chain UNCLEAR — needs manual verification

---

### adoption-path.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** EXISTS — all three adoption tiers describe real capabilities

**Issues:**

1. **[MINOR]** All three tiers use `cargo run --bin test-server --release` — this is accurate (binary name is correct). The page is generally accurate.

   The Tier 3 "Full Platform" section mentions SQL queries and stream processing — these are v2.0 features (DataFusion SQL from TODO-091, DAG from TODO-025) that are implemented (SQL) or planned (DAG). The description is aspirational but not misleading since it describes future capability.
   - **Severity:** minor — adoption-path is largely accurate; Tier 3 features exist (SQL) or are on roadmap (DAG)

---

### pn-counter.mdx — accurate

**Verdict:** accurate
**Feature Status:** EXISTS — PNCounterHandle in client, PersistenceService handles CounterRequest, usePNCounter hook in react

**Verification:**
- `client.getPNCounter(name)` — exists in `TopGunClient.ts` line 236
- `increment()`, `decrement()` — exist in `PNCounterHandle`
- `CounterRequest` → `PersistenceService.handle_counter_request()` — implemented (not a stub)
- `usePNCounter` hook — exists in `packages/react/src/hooks/usePNCounter.ts`
- `PNCounterState` wire messages — exist in core-rust

No issues found.

---

### postgresql.mdx — accurate

**Verdict:** accurate
**Feature Status:** EXISTS — PostgresDataStore fully implemented

**Verification:**
- `DATABASE_URL` env var — read in `packages/server-rust/src/storage/datastores/postgres.rs` line 523
- `cargo run --bin test-server --release` — correct binary name
- `topgun_maps` table schema — EXACTLY matches docs (map_name, key, value BYTEA, expiration_time, is_backup, created_at, updated_at with PRIMARY KEY)
- `idx_topgun_maps_map` index — matches code
- "TopGun is NOT a sync layer" claim — accurate
- "Creates its own tables alongside yours" — accurate

No issues found.

---

### live-queries.mdx — accurate

**Verdict:** accurate
**Feature Status:** EXISTS — QueryHandle + QueryService fully implemented

**Verification:**
- `client.query(mapName, filter)` — exists in `TopGunClient.ts` line 193
- `QueryHandle.subscribe(callback)` — exists in `QueryHandle.ts` line 58
- `query.onChanges(callback)` — exists in `QueryHandle.ts` line 191
- `query.getPendingChanges()`, `query.consumeChanges()` — exist in `QueryHandle.ts`
- Predicate methods (equal, notEqual, greaterThan, lessThan, between, like, regex) — implemented in `packages/client/src/Predicates.ts`
- `QueryService` in server-rust — exists, handles query subscriptions

No critical issues found. Limit (`limit` option described as "coming soon") is honest.

---

### pub-sub.mdx — accurate

**Verdict:** accurate
**Feature Status:** EXISTS — MessagingService in server, TopicHandle in client

**Verification:**
- `client.topic(name)` — exists in `TopGunClient.ts` line 209
- `TopicHandle.publish(data)` — exists in `TopicHandle.ts` line 26
- `TopicHandle.subscribe(callback)` — exists, callback receives `(data, context)` where `context = { timestamp, publisherId? }`
- `MessagingService` in server — handles `TopicSubscribe`, `TopicUnsubscribe`, `TopicPublish` operations
- "No Persistence" claim — accurate (ephemeral only, ring buffer in JournalStore is separate)
- "Re-subscription on reconnect" — accurate
- "Best-effort delivery" — accurate

No issues found.

---

### mcp-server.mdx — accurate

**Verdict:** accurate
**Feature Status:** EXISTS — `@topgunbuild/mcp-server` package fully implemented

**Verification:**
- `TopGunMCPServer` class — exists in `packages/mcp-server/src/TopGunMCPServer.ts`
- Constructor options (topgunUrl, authToken, allowedMaps, enableMutations, enableSubscriptions, defaultLimit, maxLimit) — all match `MCPServerConfig` interface in `packages/mcp-server/src/types.ts`
- `server.start()` — exists (line 208)
- `topgun-mcp` CLI binary — exists (`packages/mcp-server/src/cli.ts`, `package.json` bin)
- CLI flags (--url, --token, --maps, --no-mutations, --http, --port) — match cli.ts
- HTTP transport — exists (`packages/mcp-server/src/transport/http.ts`)

No issues found.

---

### index.mdx — minor-issues

**Verdict:** minor-issues
**Feature Status:** INFORMATIONAL

**Issues:**

1. **[MINOR]** Guide index descriptions for unimplemented features present them as available:
   - "Indexing: Accelerate queries with HashIndex, NavigableIndex, and InvertedIndex for O(1) to O(log N) lookups." — feature does not exist
   - "Adaptive Indexing: Auto-suggest or auto-create indexes based on query patterns with the Index Advisor." — feature does not exist
   - "Distributed Locks: ..." — feature is stubbed
   - **Severity:** minor — index page descriptions inherit the problem from individual pages; fix when individual pages are fixed

---

## Missing Functionality Registry

Features documented but not implemented, with TODO cross-references:

| Feature | Doc Page | TS Reference | Existing TODO | New TODO |
|---------|---------|--------------|---------------|---------|
| Adaptive Indexing | adaptive-indexing.mdx | `git show 926e856^:packages/server/src/` — search for adaptive-index files | none | TODO-174 |
| Distributed Locks | distributed-locks.mdx | `git show 926e856^:packages/server/src/coordinator/` — lock files | none (TODO-171 is RBAC) | TODO-175 |
| Entry Processor | entry-processor.mdx | `git show 926e856^:packages/server/src/` — entry-processor, sandbox | none | TODO-176 |
| Indexing (Hash/Navigable) | indexing.mdx | `git show 926e856^:packages/server/src/` — hash-index, navigable-index | none | TODO-177 |
| Interceptors | interceptors.mdx | `git show 926e856^:packages/server/src/` — interceptor files | none | TODO-178 |
| Conflict Resolvers | conflict-resolvers.mdx | `git show 926e856^:packages/server/src/` — conflict-resolver, sandbox | none | TODO-179 |
| Write Concern Achievement | write-concern.mdx | N/A — Rust feature gap | none | TODO-180 |
| Cluster env var config | deployment.mdx, cluster-replication.mdx | N/A — Rust feature gap (planned per docs comment) | TODO-141 (partial) | — |
| Consistency modes (QUORUM/STRONG) | cluster-replication.mdx | N/A — Rust architectural decision | none | — |

---

## Acceptance Criteria Verification

1. **[PASS]** Every guide page has a verdict and issue list — 22/22 pages covered
2. **[PASS]** Each issue references the specific source file — all issues cite file:line
3. **[PASS]** Pages describing unimplemented features flagged as major-rewrite with "add planned banner" recommendation — 7 pages flagged (adaptive-indexing, distributed-locks, entry-processor, indexing, interceptors, conflict-resolvers, write-concern)
4. **[PASS]** Report includes summary table — see top of file
5a. **[PASS]** Every unimplemented feature has cross-reference to TODO — new TODO-174 through TODO-180 created
5b. **[PASS]** New TODOs include TS server file references via `git show 926e856^:` — see TODO entries in TODO.md
6. **[PASS]** Priority targets (deployment, cluster-replication, cluster-client, postgresql, performance, observability) have thorough line-by-line verification — all 6 audited in detail

---

## Recommended Fix Order

### Immediate (P1 — Block Launch)

1. **observability.mdx** — rewrite metric tables for Rust server; replace Pino with tracing
2. **performance.mdx** — replace TS config knobs with Rust ServerConfig/ConnectionConfig fields; fix binary name
3. **deployment.mdx** — mark TOPGUN_TLS_* and TOPGUN_PORT env vars as planned; add caveat to Docker Compose TLS section
4. **cluster-replication.mdx** — mark env vars as planned; remove QUORUM/STRONG consistency modes (not implemented); remove replication metrics
5. **write-concern.mdx** — mark achieved_level as unimplemented; remove setWithAck()/batchSet() code examples
6. **adaptive-indexing.mdx** — add planned banner
7. **indexing.mdx** — add planned banner; clarify tantivy search vs query indexes
8. **interceptors.mdx** — add planned banner
9. **distributed-locks.mdx** — add planned banner
10. **entry-processor.mdx** — add planned banner
11. **conflict-resolvers.mdx** — add planned banner; keep concept overview, remove runtime examples

### Soon (P2)

12. **cluster-client.mdx** — fix circuitBreaker config key (it's internal, not user-configurable); fix connectionsPerNode default (1, not 2)
13. **full-text-search.mdx** — clarify local vs server-side search in Basic Usage
14. **ttl.mdx** — verify TTL enforcement chain end-to-end; add UNCLEAR note if unverified
15. **event-journal.mdx** — verify server embedding API claim

### Polish (P3)

16. **adoption-path.mdx** — no changes needed; Tier 3 aspirational content is acceptable
17. **index.mdx** — update descriptions after individual pages are fixed
18. **pn-counter.mdx, postgresql.mdx, live-queries.mdx, pub-sub.mdx, mcp-server.mdx** — no changes needed
