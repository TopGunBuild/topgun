# TopGun Roadmap

**Last updated:** 2026-02-15
**Strategy:** Complete TypeScript Wave 1 → Bridge to Rust → Rust server rewrite
**Product positioning:** "The reactive data grid that extends the cluster into the browser" ([PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md))

### Dual Reference Protocol

Each Rust spec should reference TWO sources:

1. **TopGun TS Server** (`packages/server/`) — behavioral specification (what the system does, test vectors, wire protocol)
2. **Hazelcast Java** (`/Users/koristuvac/Projects/hazelcast/`) — architectural reference (how a mature IMDG handles the same domain)

**Fix-on-port rule:** Before porting a domain, audit the TS source. Fix bugs/dead code in TS first, then port the corrected version. See PROJECT.md "Rust Migration Principles".

| Rust TODO | TopGun TS Source | Hazelcast Java Reference |
|---|---|---|
| TODO-063 Partitions | `server/src/cluster/PartitionService.ts` | `hazelcast/partition/` |
| TODO-064 Network | `server/src/modules/network-module.ts` | `hazelcast/internal/networking/` |
| TODO-065 Handlers | `server/src/coordinator/`, `server/src/modules/handlers-module.ts` | `hazelcast/map/impl/operation/` |
| TODO-066 Cluster | `server/src/cluster/` | `hazelcast/cluster/` |
| TODO-067 Storage | `server/src/storage/` | `hazelcast/map/impl/mapstore/` |
| TODO-025 DAG | [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) | `hazelcast/jet/core/`, `jet/impl/execution/` |
| TODO-033 AsyncStorage | — | `hazelcast/map/impl/mapstore/` (Write-Behind) |
| TODO-040 Tiered | — | `hazelcast/map/impl/eviction/`, `map/impl/record/` |
| TODO-041 Multi-tenancy | — | `hazelcast/security/`, `access/` |
| TODO-036 Extensions | — | `hazelcast/spi/` (Service Provider Interface) |
| TODO-071 Search | `server/src/search/` | `hazelcast/query/`, `map/impl/query/` |

**Not relevant from Hazelcast:** `sql/` (Calcite), `cp/` (Raft), `transaction/`, `wan/`, `cache/` (JCache), Spring modules.

---

## Phase 0: TypeScript Completion — COMPLETE

*Goal: Finish client cluster integration. After this, no new TypeScript server work.*

### SPEC-048b: Routing Logic and Error Recovery — DONE
- **Status:** Complete (archived to .specflow/archive/SPEC-048b.md)
- **Summary:** Per-key batch routing in SyncEngine, NOT_OWNER error handling, partition map re-request on reconnect

### SPEC-048c: End-to-End Cluster Integration Test — DONE
- **Status:** Complete (archived to .specflow/archive/SPEC-048c.md)
- **Summary:** Integration test: 3-node cluster startup, auth, write routing to partition owner, failover, cluster stats

### TODO-073: Fix ConnectionPool.remapNodeId Closure Bug — DONE
- **Status:** Complete (d6b490b, 2026-02-13, via `/sf:quick`)
- **Summary:** Changed socket event closures to use `connection.nodeId` (mutable) instead of captured `nodeId` parameter

### TODO-045: DST Documentation
- **Priority:** Low (optional, do between heavy tasks)
- **Summary:** Document VirtualClock, SeededRNG, ScenarioRunner in official docs
- **Location:** `apps/docs-astro/src/content/docs/reference/testing.mdx`
- **Effort:** 0.5-1 day

---

## Phase 1: Bridge TS to Rust (~1-2 days)

*Goal: Set up Rust infrastructure so the first Rust spec can be executed immediately.*

### TODO-059: Rust Project Bootstrap
- **Priority:** P0 (blocks all Rust work)
- **Complexity:** Medium
- **Summary:** Create Cargo workspace, CI pipeline, and project structure for Rust server
- **Deliverables:**
  - `Cargo.toml` workspace root with `packages/core-rust/` and `packages/server-rust/`
  - CI pipeline: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt`
  - Rust toolchain config: `rust-toolchain.toml` (stable channel)
  - pnpm + Cargo coexistence verified (both build systems work)
  - `.specflow/PROJECT.md` updated with Rust Language Profile (enables SpecFlow Rust checks)
- **Context:** [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md) Section 8 (Monorepo Structure)
- **Effort:** 0.5-1 day

### TODO-060: Upfront Trait Definitions → SPEC-050 ✅
- **Priority:** P0 (blocks all Rust feature work)
- **Complexity:** Low (design only, ~150 lines of Rust)
- **Status:** DONE (SPEC-050 completed 2026-02-13)
- **Summary:** Define the 6 foundational traits that gate all Rust architecture decisions
- **Deliverables:**
  - `ServerStorage` trait (pluggable persistence)
  - `MapProvider` trait (async map access, tiered storage ready)
  - `QueryNotifier` trait (write-path notifications)
  - `Processor` trait (DAG executor vertices)
  - `RequestContext` struct (multi-tenancy, auth, tracing)
  - `SchemaProvider` trait (schema validation + partial replication shapes)
- **Context:**
  - Traits 1-5: [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md) Section 7
  - Trait 6: [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.5
- **Effort:** 0.5 day

---

## Phase 2: Rust Core (~3-4 weeks)

*Goal: Port foundational types and prove client-server binary compatibility.*

### TODO-061: Core CRDTs in Rust → SPEC-051
- **Priority:** P0 (foundation for everything)
- **Complexity:** Medium (spec: large — needs /sf:split)
- **Summary:** Port LWWMap, ORMap, HLC, and MerkleTree to Rust
- **Key decisions:**
  - Custom CRDT implementation (not yrs/crdts crate) for full control
  - `serde` + `rmp-serde` for MsgPack compatibility with existing TS client
  - Property-based testing with `proptest` for CRDT correctness
- **Source:** `packages/core/src/crdt/`, `packages/core/src/hlc/`, `packages/core/src/merkle/`
- **Verification:** Run same test vectors as TS to confirm behavioral equivalence
- **Effort:** 1-2 weeks

### TODO-062: Message Schema Compatibility → SPEC-052
- **Priority:** P0 (client-server contract)
- **Complexity:** Large (needs /sf:split — 53+ message types across 8 domains)
- **Status:** Spec created (SPEC-052, 2026-02-14)
- **Summary:** Ensure Rust server can serialize/deserialize all message types compatible with TS client
- **Key decisions:**
  - MsgPack wire format stays (cross-language compatibility)
  - Zod schemas in `packages/core/src/schemas/` are the source of truth
  - Rust serde structs must produce identical bytes as TS msgpackr
- **Approach:**
  - Generate Rust structs from Zod schema definitions (build step or manual)
  - Integration test: TS client sends message → Rust deserializes → Rust serializes → TS verifies
- **Depends on:** TODO-059 (project bootstrap)
- **Effort:** 1-2 weeks

### TODO-063: Partition Service in Rust
- **Priority:** P1
- **Complexity:** Low
- **Summary:** Port PartitionService (271 partitions, consistent hashing, partition pruning) to Rust
- **TS Source:** `packages/server/src/cluster/PartitionService.ts`
- **HC Reference:** `hazelcast/partition/` — partition table protocol, rebalancing algorithm
- **Note:** Pure logic, no I/O — straightforward port
- **Depends on:** TODO-059
- **Effort:** 2-3 days

### TODO-074: HLC Node ID Colon Validation (TS + Rust)
- **Priority:** P2 (hardening, theoretical risk)
- **Complexity:** Trivial
- **Summary:** Add validation in HLC constructor to reject node IDs containing `:`. The string format `millis:counter:nodeId` uses `:` as delimiter — an unvalidated colon in nodeId breaks `HLC.parse()` in TS (`split(':')` expects exactly 3 parts). Rust `splitn(3, ':')` survives but returns a corrupted nodeId. Currently safe by accident (`crypto.randomUUID()` produces dashes), but no guard against custom IDs.
- **Changes:**
  - `packages/core/src/HLC.ts` — add `if (nodeId.includes(':')) throw` in constructor
  - `packages/core-rust/src/hlc.rs` — add `assert!(!node_id.contains(':'))` in `HLC::new()`
  - Add test cases for rejection in both
- **Depends on:** —
- **Effort:** 1-2 hours
- **Source:** External audit finding (Audit 1, Section 2)

### TODO-075: Fix Rust ORMap Merkle Hash Determinism
- **Priority:** P1 (bug — cross-language sync broken)
- **Complexity:** Low
- **Summary:** `hash_entry()` in `packages/core-rust/src/or_map.rs:472` uses `serde_json::to_string(&record.value)` which does NOT sort object keys. TS version sorts keys via `JSON.stringify(value, Object.keys(value).sort())`. Result: different hashes for identical data, Merkle sync diverges. Merkle hashes are sent over the wire (`SYNC_RESP_ROOT`, `SYNC_RESP_BUCKETS`), so cross-language compatibility is mandatory.
- **Fix:** Replace `serde_json::to_string()` with a recursive key-sorting serializer (convert to `serde_json::Value`, sort `Map` keys recursively, then serialize).
- **Changes:**
  - `packages/core-rust/src/or_map.rs` — implement `canonical_json()` helper, use in `hash_entry()`
  - Add test: known object `{z:1, a:2}` hashes identically to sorted `{a:2, z:1}`
  - Add cross-language test vector: TS `stringifyValue({z:1, a:2})` === Rust `canonical_json({z:1, a:2})`
- **Depends on:** TODO-061 (CRDTs)
- **Effort:** 0.5 day
- **Source:** External audit finding (Audit 1, Section 1) + deep analysis confirmed bug

### TODO-078: Fix TS Hash Function Inconsistency (xxHash64 vs FNV-1a)
- **Priority:** P2 (pre-existing TS bug)
- **Complexity:** Trivial
- **Summary:** `packages/core/src/utils/hash.ts` has a runtime fallback: if `@topgunbuild/native` loads → xxHash64 (truncated to 32-bit), otherwise → FNV-1a. These produce DIFFERENT hashes for the same input. In a cluster where some nodes load the native module and others don't, Merkle hashes diverge → false sync cycles. Since TS server is being deprecated, simplest fix is to force FNV-1a unconditionally (matches Rust `fnv1a_hash()`).
- **Changes:**
  - `packages/core/src/utils/hash.ts` — remove native xxHash64 path, use FNV-1a only
  - OR: force one algorithm globally at startup
- **Depends on:** —
- **Effort:** 1-2 hours
- **Source:** Discovered during audit analysis (not in either external audit)

### TODO-079: Rust Message Schema Architecture (Fix-on-Port) → SPEC-054
- **Priority:** P0 (blocks all remaining message schema work)
- **Complexity:** Medium
- **Summary:** The current Rust message structs (SPEC-052a/b) copy JS limitations instead of leveraging Rust's type system. Three architectural issues must be resolved before implementing SPEC-052c/d/e:
  1. **`r#type: String` conflicts with Message enum.** SPEC-052e plans `#[serde(tag = "type")]` Message enum, but inner structs already have `r#type: String`. On serialization, serde produces duplicate `type` keys -- undefined behavior. Inner structs must NOT have a `type` field; the enum owns the tag.
  2. **`f64` for integer fields copies JS limitation.** JS has no integer type; Rust does. Fields like `root_hash`, `count`, `code`, `timeout` should use `u64`/`u32`/`i64`. This also produces **better** wire compatibility: TS `msgpackr` encodes integers as MsgPack integers, and Rust `u64` decodes them directly (no coercion), while `f64` causes rmp_serde to emit MsgPack float64 on re-serialization -- different binary format.
  3. **No `Default` derives** for payload structs with many optional fields.
- **Deliverables:**
  1. Prototype `#[serde(tag = "type")]` Message enum with 3 representative variants (payload-wrapped, flat, flat+binary) to verify serde behavior with rmp_serde
  2. Define integer type policy with MsgPack compatibility verification
  3. Establish struct pattern: inner structs WITHOUT `type` field, Message enum owns the tag
  4. Retroactively fix SPEC-052a structs (base.rs): remove `r#type: String` from AuthMessage, AuthRequiredMessage
  5. Retroactively fix SPEC-052b structs (sync.rs, query.rs): remove `r#type: String` from all message structs, replace `f64` with proper integer types where appropriate
  6. Add `Default` derives to payload structs
  7. Update all tests, verify `cargo test` + `cargo clippy` pass
- **Key type decisions to prototype:**
  - `root_hash`: `f64` → likely `u64` (FNV-1a returns 32-bit, but stored as `z.number()`)
  - `count` (BatchMessage): `f64` → likely `u32`
  - `code` (OpRejectedPayload): `f64` → likely `u32`
  - `timeout`: `f64` → likely `u64` (milliseconds)
  - `last_sync_timestamp`: `f64` → evaluate (JS timestamp, integer ms since epoch)
  - `Timestamp.millis`: already `i64` in hlc.rs -- correct, no change needed
- **Depends on:** SPEC-052b (complete -- provides structs to rework)
- **Blocks:** SPEC-052c, SPEC-052d, SPEC-052e (must not continue with broken pattern)
- **Effort:** 0.5-1 day
- **Source:** Post-execution architectural review of SPEC-052b

### TODO-077: Protocol Drift CI Check
- **Priority:** P2
- **Complexity:** Low
- **Summary:** After SPEC-052e completes (golden-file cross-language tests), add a CI step to `.github/workflows/rust.yml` that runs the TS fixture generator and Rust integration tests together. Prevents silent protocol drift when TS Zod schemas change without updating Rust serde structs.
- **CI step:**
  ```yaml
  - name: Cross-language protocol validation
    run: |
      pnpm --filter @topgunbuild/core test -- cross-lang-fixtures
      cargo test --test cross_lang_compat
  ```
- **Depends on:** TODO-062 (all sub-specs, especially SPEC-052e)
- **Effort:** 0.5 day
- **Source:** External audit finding (Audit 2, Section 3.2)

---

## Phase 3: Rust Server Core (~6-8 weeks)

*Goal: Working Rust server that passes existing TS integration tests.*

### TODO-064: Networking Layer (axum + WebSocket)
- **Priority:** P0
- **Complexity:** Medium
- **Summary:** HTTP + WebSocket server using axum, with deferred startup pattern
- **Key features:**
  - `GET /health`, `POST /sync` (HTTP sync, existing protocol)
  - WebSocket upgrade for real-time sync
  - TLS support (rustls)
  - IConnection adapter pattern preserved
- **Crates:** axum, tokio-tungstenite, tower, rustls
- **Depends on:** TODO-059, TODO-062
- **Effort:** 1-2 weeks

### TODO-065: Message Handlers (26 handlers, 8 domains)
- **Priority:** P0
- **Complexity:** Large
- **Summary:** Port all 26 message handlers organized by domain
- **Known TS bug (covered by rewrite):** `BatchProcessingHandler.processBatchAsync` nests inter-node forwarded messages in an extra `{type: 'CLIENT_OP', payload: {...}}` layer, causing `handleOpForward` to fail. Single-op path is correct. (Discovered by SPEC-048c)
- **Domains:**
  - CRDT (merge, conflict resolution)
  - Sync (delta sync, MerkleTree reconciliation)
  - Query (live queries, standing query registry)
  - Messaging (pub/sub topics)
  - Coordination (cluster protocol)
  - Search (BM25, distributed search)
  - Persistence (PostgreSQL operations)
  - Client/Server (auth, connection management)
- **Source:** `packages/server/src/coordinator/` and `packages/server/src/modules/handlers-module.ts`
- **Depends on:** TODO-061, TODO-062, TODO-064
- **Effort:** 2-3 weeks

### TODO-066: Cluster Protocol
- **Priority:** P1
- **Complexity:** Medium
- **Summary:** ClusterManager, inter-node WebSocket mesh, partition ownership, rebalancing
- **Known TS bug (covered by rewrite):** `PartitionService.getPartitionMap()` returns cluster inter-node port instead of client WS port, and `host:'unknown'` for non-self nodes. Clients cannot use the partition map to connect to correct endpoints. (Discovered by SPEC-048c)
- **TS Source:** `packages/server/src/cluster/`
- **HC Reference:** `hazelcast/cluster/` — membership protocol, split-brain detection, heartbeat, cluster state machine
- **Key:** 26 cluster message types, partition table, node discovery
- **Depends on:** TODO-063, TODO-064
- **Effort:** 2-3 weeks

### TODO-067: Storage Layer (PostgreSQL)
- **Priority:** P1
- **Complexity:** Low-Medium
- **Summary:** ServerStorage trait implementation for PostgreSQL using sqlx
- **Source:** `packages/server/src/storage/`
- **Crates:** sqlx (compile-time checked queries), tokio-postgres
- **Depends on:** TODO-060 (ServerStorage trait)
- **Effort:** 1 week

### TODO-076: Evaluate MsgPack-Based Merkle Hashing
- **Priority:** P2 (performance optimization)
- **Complexity:** Medium (design decision + implementation in both TS and Rust)
- **Summary:** Current Merkle hashing converts values to JSON string then FNV-1a hashes the string. Alternative: hash MsgPack bytes directly (`hash(rmp_serde::to_vec_named(&value))`). MsgPack with named keys is already the wire format — reusing it avoids JSON serialization overhead (~30-50% faster per hash). Requires deterministic key ordering in MsgPack (use `BTreeMap` or sorted struct fields). Breaking change to Merkle hashes — requires simultaneous rollout to both TS and Rust, acceptable since backward compat is not required.
- **Evaluation criteria:**
  - Benchmark: JSON stringify + FNV-1a vs MsgPack + FNV-1a on typical ORMap entries
  - Verify MsgPack determinism: `rmp_serde::to_vec_named()` with sorted keys produces identical bytes in TS (msgpackr) and Rust
  - If perf gain < 20%, keep JSON approach (simpler debugging)
- **Depends on:** TODO-075 (fix current hashing first, then evaluate optimization)
- **Effort:** 1 day (evaluation) + 2-3 days (implementation if approved)
- **Source:** Deep analysis during audit review

### TODO-068: Integration Test Suite
- **Priority:** P0
- **Complexity:** Large
- **Summary:** Port critical test scenarios, use TS server as behavioral oracle
- **Approach:**
  - Run identical test scenarios against TS server and Rust server
  - Compare behavior for: CRDT merge, sync protocol, cluster operations, query results
  - Client-server tests: TS client connects to Rust server
- **Source:** `packages/server/src/__tests__/`, `tests/e2e/`
- **Depends on:** TODO-064, TODO-065
- **Effort:** 3-4 weeks (concurrent with other Phase 3 work)

---

## Phase 4: Rust Feature Completion (~4-6 weeks)

*Goal: Features that differentiate TopGun, including new product-positioning features.*

### TODO-069: Schema System
- **Priority:** P1 (product differentiator)
- **Complexity:** Medium
- **Summary:** TypeScript-first schema definition with server-side validation
- **Architecture:**
  - Developer writes `topgun.schema.ts` using `@topgunbuild/schema` helpers
  - Build step generates Rust validation code + TS client types
  - Server validates writes against registered schemas (optional → strict rollout)
  - SchemaProvider trait implementation
- **Phased rollout:**
  - Phase 2a: Optional TypedMap — server validates if schema registered
  - Phase 2b: New maps require registered schema
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.4
- **Effort:** 2-3 weeks

### TODO-070: Partial Replication / Shapes
- **Priority:** P1 (table stakes for competitive parity)
- **Complexity:** Medium-Large
- **Summary:** Client subscribes to data subsets; server syncs only matching entries
- **Architecture:**
  - Client API: `client.shape('todos', { where: { userId: id }, fields: [...] })`
  - Server: SyncShape struct with filter + field projection + limit
  - Integration with SchemaProvider.get_shape()
  - MerkleTree per shape (not per map) for efficient delta sync
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.5
- **Depends on:** TODO-069 (Schema System)
- **Effort:** 2-3 weeks

### TODO-048: SSE Push for HTTP Sync
- **Priority:** P2
- **Complexity:** Low (trivial in Rust)
- **Summary:** Server-Sent Events transport for real-time push in serverless
- **Architecture:**
  - `GET /events` SSE endpoint via axum
  - `SsePushProvider` implements IConnectionProvider
  - `AutoConnectionProvider` gains third tier: WS → SSE → HTTP polling
- **Context:** Extends SPEC-036 (HTTP Sync Protocol)
- **Effort:** 2-3 days

### TODO-049: Cluster-Aware HTTP Routing
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** HttpSyncHandler routes to partition owners in cluster
- **Depends on:** TODO-066 (Cluster Protocol), TODO-063 (Partition Service)
- **Effort:** 1-2 weeks

### TODO-025: DAG Executor for Distributed Queries
- **Priority:** P2
- **Complexity:** Large
- **Summary:** Hazelcast-style DAG executor for distributed query processing
- **Architecture:** 3-tier processor model (Source → Transform → Sink), partition-aware, backpressure via tokio channels
- **Context:** [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) (700+ lines)
- **HC Reference:** `hazelcast/jet/core/` (Processor, Inbox, Outbox), `jet/impl/execution/` (TaskletTracker, CooperativeWorker, StoreSnapshotTasklet)
- **Key insight:** Rust `Future::poll()` maps naturally to Cooperative Tasklet model
- **Depends on:** TODO-060 (Processor trait), TODO-063 (Partition Service)
- **Effort:** 2-3 weeks

### TODO-071: Search with Tantivy
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Replace custom BM25 search with tantivy full-text search engine
- **Benefits:** Orders of magnitude faster, built-in tokenization, fuzzy search, phrase queries
- **Crate:** tantivy
- **Source:** `packages/server/src/search/`
- **Effort:** 1-2 weeks

---

## Phase 5: Post-Migration Features (~8-12 weeks, after Rust server launch)

*Goal: Enterprise and advanced features built natively in Rust.*

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Hazelcast-style Write-Behind pattern: staging area, write coalescing, batch flush, retry queue
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **HC Reference:** `hazelcast/map/impl/mapstore/` — WriteBehindStore, StoreWorker, coalescing logic, retry with backoff
- **Depends on:** TODO-060 (ServerStorage trait)
- **Effort:** 2-3 weeks

### TODO-043: S3 Bottomless Storage
- **Priority:** P3
- **Complexity:** Very Large
- **Summary:** Append-only log in S3/R2/GCS. Immutable log segments, replay on startup, Merkle checkpoints
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7
- **Crates:** aws-sdk-s3, opendal
- **Depends on:** TODO-033 (AsyncStorageWrapper)
- **Effort:** 6-8 weeks

### TODO-040: Tiered Storage (Hot/Cold)
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Hot data in memory, cold data in S3/cheap storage with transparent migration
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Depends on:** TODO-033 (AsyncStorageWrapper), TODO-060 (MapProvider trait)
- **Effort:** 4-6 weeks

### TODO-039: Vector Search
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Semantic vector search with local embeddings, HNSW index, tri-hybrid search (Exact + BM25 + Semantic)
- **Context:** [PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md)
- **Crate:** usearch (Rust bindings)
- **Effort:** 4 weeks

### TODO-036: Pluggable Extension System
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Modular extension system for community contributions (crypto, compression, audit, geo)
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5
- **Effort:** 2-3 weeks

### TODO-041: Multi-Tenancy
- **Priority:** P4
- **Complexity:** Large
- **Summary:** Per-tenant isolation, quotas, billing, tenant-aware partitioning
- **Context:** [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Depends on:** TODO-060 (RequestContext.tenant_id)
- **Effort:** 4-6 weeks

### TODO-044: Bi-Temporal Queries (Time-Travel)
- **Priority:** P4
- **Complexity:** Large
- **Summary:** Query historical state with valid time + transaction time
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 8
- **Depends on:** TODO-043 (S3 Bottomless Storage)
- **Effort:** 4-6 weeks

### TODO-072: Selective WASM Modules for Client
- **Priority:** P4
- **Complexity:** Medium
- **Summary:** Compile DAG Executor, tantivy search, and Entry Processors to WASM for browser use
- **Key constraint:** NOT for basic CRDT ops (sync JS is faster due to WASM boundary cost)
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.6
- **Effort:** 2-3 weeks

---

## Execution Order (parallel waves)

Items within the same wave can run in parallel. Each wave starts when its blockers from previous waves complete.

| Wave | Items | Blocked by | Phase |
|------|-------|------------|-------|
| **1** | TODO-061 (CRDTs) · TODO-062 starts (052a+052b) · TODO-063 (Partitions) · TODO-074 (HLC validation) · TODO-078 (TS hash fix) | — (all unblocked) | 2 |
| **1a** | **TODO-079 (Schema arch + fix 052a/b)** | 052b complete | 2 |
| **1b** | TODO-062 continues (052c+052d+052e) · TODO-075 (ORMap hash fix) | **079** · 061 | 2 |
| **1c** | TODO-077 (CI drift check) | 062 (all sub-specs incl. 052e) | 2 |
| **2** | TODO-064 (Network) · TODO-067 (PostgreSQL) · TODO-076 (MsgPack hash eval) | 062 · 060✓ · 075 | 3 |
| **3** | TODO-065 (Handlers) · TODO-066 (Cluster) · TODO-068 (Tests, incremental) | 061+062+064 · 063+064 · 064 | 3 |
| **4** | TODO-069 (Schema) · TODO-048 (SSE) · TODO-025 (DAG) · TODO-071 (Tantivy) | — · — · 063 · — | 4 |
| **5** | TODO-070 (Shapes) · TODO-049 (Cluster HTTP) | 069 · 066+063 | 4 |
| **6** | TODO-033 (AsyncStorage) · TODO-039 (Vector) · TODO-036 (Extensions) · TODO-041 (Multi-tenancy) · TODO-072 (WASM) | — | 5 |
| **7** | TODO-043 (S3) · TODO-040 (Tiered) | 033 · 033 | 5 |
| **8** | TODO-044 (Time-Travel) | 043 | 5 |

**Current position:** Wave 1a — SPEC-052b executed, TODO-079 → SPEC-054 (Schema architecture) is next. Must complete before continuing 052c/d/e.

---

## Dependency Graph

```
Phase 0 (TypeScript)            Phase 1 (Bridge)
SPEC-048b ──→ SPEC-048c         TODO-059 (Cargo) ──→ TODO-060 (Traits)
          [DONE]                       [DONE]              [DONE]
                                    │
                                    ↓
                            Phase 2 (Rust Core) — WAVE 1
                    TODO-061 (CRDTs)    TODO-062 starts      TODO-063 (Partitions)
                    TODO-074 (HLC ✓)    (052a+052b done)
                    TODO-078 (TS hash)       │
                        │                    ↓
                        │         ★ TODO-079 (Schema arch)  ←── WAVE 1a
                        │         fix 052a/b: remove type:String,
                        │         f64→integers, Message enum pattern
                        │                    │
                        ↓                    ↓
                    TODO-075          TODO-062 continues     ←── WAVE 1b
                    (ORMap hash)      (052c → 052d → 052e)
                        │                    │
                        │              TODO-077 (CI drift)   ←── WAVE 1c
                        │                    │
                        └────────┬───────────┘
                                 ↓
                            Phase 3 (Server) — WAVES 2-3
                    TODO-064 (Network) ──→ TODO-065 (Handlers)
                        │                                     │
                        └──→ TODO-066 (Cluster) ←─────────────┘←── TODO-063
                        │
                        └──→ TODO-067 (PostgreSQL)
                        │
                        └──→ TODO-068 (Integration Tests)
                        │
                    TODO-076 (MsgPack hash eval) ←── TODO-075
                                 │
                                 ↓
                            Phase 4 (Features) — WAVES 4-5
            TODO-069 (Schema) ──→ TODO-070 (Shapes)
            TODO-048 (SSE)
            TODO-049 (Cluster HTTP)
            TODO-025 (DAG Executor)
            TODO-071 (Tantivy Search)
                                 │
                                 ↓
                            Phase 5 (Post-Migration) — WAVES 6-8
            TODO-033 (AsyncStorage) ──→ TODO-043 (S3) ──→ TODO-044 (Time-Travel)
                                   └──→ TODO-040 (Tiered Storage)
            TODO-039 (Vector Search)
            TODO-036 (Extensions)
            TODO-041 (Multi-Tenancy)
            TODO-072 (WASM Modules)
```

## Timeline Summary

| Phase | Effort | Prerequisites |
|-------|--------|---------------|
| **0. TypeScript Completion** | 3-4 days | Current codebase |
| **1. Bridge** | 1-2 days | Phase 0 complete |
| **2. Rust Core** | 3-4 weeks | Phase 1 complete |
| **3. Rust Server** | 6-8 weeks | Phase 2 complete |
| **4. Rust Features** | 4-6 weeks | Phase 3 complete (some items parallelizable) |
| **5. Post-Migration** | 8-12 weeks | Phase 4 complete (independent items) |
| **Total to Rust server launch (Phases 0-3)** | **~11-15 weeks** | |
| **Total with features (Phases 0-4)** | **~15-21 weeks** | |

## Eliminated Items

| TODO | Reason | Date |
|------|--------|------|
| TODO-042 (DBSP) | Not needed; StandingQueryRegistry + ReverseQueryIndex sufficient | 2026-02-10 |
| TODO-034 (Rust/WASM hot paths) | Superseded by full Rust migration | 2026-02-10 |

## Completed TypeScript Items (archived)

All items below are completed and archived in `.specflow/archive/`:

| TODO | Spec | Completed |
|------|------|-----------|
| TODO-051 → SPEC-038 | WebSocket auth handshake fix | 2026-02-08 |
| TODO-052 → SPEC-040 | Interceptor + TLS verification | 2026-02-08 |
| TODO-053 → SPEC-041 | DistributedSearch + GC fix | 2026-02-08 |
| TODO-054 → SPEC-045 | ProcessorSandbox + docs update | 2026-02-10 |
| TODO-055 → SPEC-042 | setTimeout → polling hardening | 2026-02-09 |
| TODO-056 → SPEC-039 | network.start() reject path | 2026-02-08 |
| TODO-057 → SPEC-043 | SearchCoordinator LEAVE bug | 2026-02-09 |
| TODO-058 → SPEC-044 | Resilience test rewrite | 2026-02-09 |
| TODO-050 → SPEC-046 | IConnection abstraction | 2026-02-10 |
| TODO-029 → SPEC-047 | Partition pruning | 2026-02-11 |
| TODO-023 → SPEC-048/a | Client cluster (part 1) | 2026-02-11 |

## Context Files

| TODO | Context File |
|------|-------------|
| TODO-025 | [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) |
| TODO-033 | [topgun-rocksdb.md](../reference/topgun-rocksdb.md) |
| TODO-036 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5 |
| TODO-039 | [PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md) |
| TODO-040 | [topgun-rocksdb.md](../reference/topgun-rocksdb.md) |
| TODO-041 | [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md) |
| TODO-043 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7 |
| TODO-044 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 8 |
| TODO-059-072 | [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md), [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) |

---

*Restructured 2026-02-12: Replaced wave-based organization with phase-based Rust migration roadmap. Added TODO-059 through TODO-072 for Rust-specific work. Product positioning decisions (schema, shapes, WASM) integrated as concrete TODOs.*
*Updated 2026-02-15: Added TODO-074 through TODO-078 from external audit analysis. HLC validation, ORMap hash determinism bug, TS hash inconsistency, protocol drift CI, MsgPack hash evaluation.*
