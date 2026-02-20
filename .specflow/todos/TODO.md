# TopGun Roadmap

**Last updated:** 2026-02-18
**Strategy:** Rust-first IMDG design informed by Hazelcast architecture, not a TypeScript port
**Product positioning:** "The reactive data grid that extends the cluster into the browser" ([PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md))

### Design Philosophy (updated 2026-02-18)

1. **Rust-first:** Maximize Rust's type system, ownership model, and async runtime. Do not replicate TypeScript limitations.
2. **Hazelcast-informed:** Use Hazelcast (`/Users/koristuvac/Projects/hazelcast/`) as primary architectural reference for server-side components. TS server is behavioral reference only (wire protocol, test vectors).
3. **No tech debt forward:** Every trait boundary must account for Phase 4-5 requirements (tiered storage, S3, multi-tenancy). Do not design for "just PostgreSQL" when the roadmap includes S3 and hot/cold tiers.
4. **Research before design:** Complex server subsystems (storage, cluster, operations) require research sprints before implementation specs.

### Triple Reference Protocol (updated 2026-02-19)

Each Rust spec should reference up to THREE sources:

1. **TopGun TS Server** (`packages/server/`) — behavioral specification (wire protocol, test vectors, message formats)
2. **Hazelcast Java** (`/Users/koristuvac/Projects/hazelcast/`) — **conceptual architecture** (WHAT to build: layer responsibilities, protocols, data flow)
3. **Rust OSS Projects** — **implementation patterns** (HOW to build in Rust: ownership, concurrency, async composition)
   - **TiKV** (`/Users/koristuvac/Projects/rust/tikv/`) — storage traits, per-partition FSM, DashMap, batch system
   - **Quickwit** (`/Users/koristuvac/Projects/rust/quickwit/`) — chitchat gossip, actor framework, Tower layers, service composition
   - **Databend** (`/Users/koristuvac/Projects/rust/databend/`) — OpenDAL object storage, GlobalServices singleton, pipeline DAG

**Fix-on-port rule:** Before porting a domain, audit the TS source. Fix bugs/dead code in TS first, then port the corrected version. See PROJECT.md "Rust Migration Principles".

| Rust TODO | TopGun TS Source | Hazelcast (WHAT) | Rust OSS (HOW) |
|---|---|---|---|
| TODO-063 Partitions | `server/src/cluster/PartitionService.ts` | `internal/partition/` | TiKV: `DashMap`, `RegionState` enum |
| TODO-064 Network | `server/src/modules/network-module.ts` | `internal/networking/` | Quickwit: Tower layers, axum |
| TODO-065 Operations | `server/src/coordinator/` | `spi/`, `internal/partition/operation/` | Quickwit: Actor+Tower; TiKV: Worker/Scheduler |
| TODO-066 Cluster | `server/src/cluster/` | `internal/cluster/impl/` | Quickwit: chitchat; TiKV: FSM batch system |
| TODO-067 Storage | `server/src/storage/` | `map/impl/recordstore/`, `mapstore/` | TiKV: `engine_traits`; Databend: OpenDAL |
| TODO-025 DAG | [DAG spec](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) | `jet/core/`, `jet/impl/execution/` | Databend: pipeline `StableGraph` |
| TODO-033 AsyncStorage | — | `map/impl/mapstore/` (Write-Behind) | TiKV: `Scheduler<T>` async flush |
| TODO-040 Tiered | — | `map/impl/eviction/`, `map/impl/record/` | TiKV: `in_memory_engine` hot/cold |
| TODO-041 Multi-tenancy | — | `security/`, `access/` | — |
| TODO-036 Extensions | — | `spi/` (SPI) | Databend: `GlobalInstance` registry |
| TODO-071 Search | `server/src/search/` | `query/`, `map/impl/query/` | Quickwit: tantivy integration, SearchService trait |
| TODO-043 S3 Storage | — | — | Databend: OpenDAL `Operator` + layer stack |

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

---

## Phase 1: Bridge TS to Rust — COMPLETE

*Goal: Set up Rust infrastructure so the first Rust spec can be executed immediately.*

### TODO-059: Rust Project Bootstrap — DONE
- **Status:** Complete
- **Summary:** Cargo workspace, CI pipeline, Rust toolchain, pnpm + Cargo coexistence

### TODO-060: Upfront Trait Definitions → SPEC-050 — DONE
- **Status:** Complete (SPEC-050 completed 2026-02-13)
- **Summary:** 6 foundational traits: ServerStorage, MapProvider, QueryNotifier, Processor, RequestContext, SchemaProvider

---

## Phase 2: Rust Core (~3-4 weeks)

*Goal: Port foundational types and prove client-server binary compatibility.*

### TODO-061: Core CRDTs in Rust → SPEC-051 — DONE
- **Status:** Complete (LWWMap, ORMap, HLC, MerkleTree — all with proptest)
- **Summary:** Custom CRDT implementations with `serde` + `rmp-serde` for MsgPack compatibility
- **Test coverage:** 280+ tests including proptest commutativity/convergence

### TODO-062: Message Schema Compatibility → SPEC-052 — DONE
- **Priority:** P0 (client-server contract)
- **Status:** Complete (SPEC-052a through 052e all done, 2026-02-19). 77-variant Message enum, 12 HTTP sync types, 100+ domain types, 393 Rust tests + 62 TS tests, 61 golden fixtures.
- **Summary:** Rust server can serialize/deserialize all message types compatible with TS client
- **Depends on:** TODO-059 ✅

### TODO-079: Rust Message Schema Architecture (Fix-on-Port) → SPEC-054 — DONE
- **Status:** Complete (SPEC-054 completed 2026-02-17)
- **Summary:** Removed `r#type: String` from inner structs, replaced `f64` with proper integer types, added `Default` derives

### TODO-063: Basic Partition Hash and Lookup
- **Priority:** P1
- **Complexity:** Low
- **Summary:** Implement basic partition hash function (`fnv1a(key) % 271`) and partition table lookup in Rust. This is the **client-compatible subset** — same modulo hash used by `PartitionRouter` in TS client. Full partition state machine, migration lifecycle, and fault-domain awareness are Phase 3 concerns (after TODO-081 research).
- **Scope:**
  - `hash_to_partition(key) -> u32` (matches TS `hashString(key) % PARTITION_COUNT`)
  - `PartitionTable` struct: partition ID → owner node mapping
  - Partition pruning for query optimization
- **NOT in scope:** rebalancing, migration, state machine, backup assignment (→ Phase 3)
- **TS Source:** `packages/server/src/cluster/PartitionService.ts` (basic hash + table)
- **HC Reference:** `hazelcast/internal/partition/IPartitionService.java` (interface only)
- **Depends on:** TODO-059 ✅
- **Effort:** 1-2 days

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

### TODO-075: Fix Rust ORMap Merkle Hash Determinism → SPEC-055
- **Priority:** P1 (bug — cross-language sync broken)
- **Status:** Spec created (SPEC-055), pending audit
- **Summary:** `hash_entry()` uses unsorted `serde_json::to_string()` — breaks cross-language Merkle sync
- **Depends on:** TODO-061 ✅
- **Effort:** 0.5 day

### TODO-078: ~~Fix TS Hash Function Inconsistency (xxHash64 vs FNV-1a)~~ DONE
- **Status:** Completed 2026-02-19
- **Resolution:** Removed native xxHash64 path from `hash.ts`, forced FNV-1a unconditionally. Removed `@topgunbuild/native` from core optionalDependencies. Updated server nativeStats and integration tests.

### TODO-077: ~~Protocol Drift CI Check~~ DONE
- **Status:** Completed 2026-02-20
- **Resolution:** Added `cross-lang` job to `.github/workflows/rust.yml`. Triggers on TS schema changes (`packages/core/src/schemas/**`). Runs TS fixture generator then Rust `cross_lang_compat` test to catch protocol drift.

---

## Phase 2.5: Architecture Research Sprint (~1-2 weeks) — NEW

*Goal: Design Hazelcast-informed server architecture BEFORE writing implementation specs. Each research task produces a design document with trait hierarchies, data flow diagrams, and validation against Phase 4-5 requirements.*

**Rationale (2026-02-18):** Phase 3 items were originally formulated as "port from TS". Audit revealed the TS server is architecturally simplistic compared to Hazelcast in storage layering, partition management, cluster protocol, and operation routing. Implementing Phase 3 without research would embed TS limitations into the Rust codebase — exactly the tech debt we want to avoid.

### TODO-080: Storage Architecture Research
- **Priority:** P0 (blocks TODO-067)
- **Complexity:** Medium (research + design)
- **Summary:** Design multi-layer storage architecture informed by Hazelcast's Storage → RecordStore → MapDataStore hierarchy. Must account for Phase 5 requirements (S3, tiered hot/cold, write-behind) from day 1.
- **Research scope:**
  - Hazelcast `Storage<K,R>` interface — cursor-based iteration, mutation-tolerant iterators
  - Hazelcast `RecordStore<R>` — TTL/expiry, eviction, record metadata (version, timestamps, hit count)
  - Hazelcast `MapDataStore<K,V>` — write-through vs write-behind, soft/hard flush
  - Hazelcast `DefaultRecordStore` — CompositeMutationObserver pattern (event publishers, index managers)
  - Caller provenance tracking (`CallerOrigin` enum)
  - **TiKV `engine_traits` crate** — storage abstraction WITHOUT concrete engine dependency; extension traits (`Peekable`, `Iterable`, `WriteBatchExt`); `Arc<DB>` wrapping for cheap cloning
  - **Databend OpenDAL** — pluggable object storage (S3, GCS, Azure) via single `Operator` trait; layer composition (Timeout → Retry → Metrics)
- **Deliverable:** Design document with:
  - Rust trait hierarchy (3 layers minimum)
  - Record metadata struct design
  - How PostgreSQL, S3, in-memory, and tiered backends fit the same traits
  - Migration path from Phase 3 (PostgreSQL only) to Phase 5 (S3 + tiered)
  - Concrete Rust patterns: `Arc<DB>` wrapping (TiKV), OpenDAL for object stores (Databend)
- **HC Reference:** `hazelcast/map/impl/recordstore/Storage.java`, `RecordStore.java`, `mapstore/MapDataStore.java`, `DefaultRecordStore.java`
- **Rust Reference:**
  - TiKV `engine_traits`: `/Users/koristuvac/Projects/rust/tikv/components/engine_traits/src/` — trait hierarchy, extension traits, `KvEngine` composition
  - TiKV `engine_rocks`: `/Users/koristuvac/Projects/rust/tikv/components/engine_rocks/src/` — concrete `Arc<RocksDB>` implementation
  - Databend OpenDAL: `/Users/koristuvac/Projects/rust/databend/src/common/storage/src/operator.rs` — `Operator` + layer composition
- **Effort:** 3-5 days
- **Output:** `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md`

### TODO-081: Cluster Protocol Research
- **Priority:** P0 (blocks TODO-063 advanced, TODO-066)
- **Complexity:** Medium (research + design)
- **Summary:** Design Hazelcast-informed cluster protocol for Rust. TS ClusterManager lacks membership versioning, explicit join ceremony, split-brain recovery, and 3-phase migration lifecycle.
- **Research scope:**
  - Hazelcast `MembershipManager` — versioned MembersView, membership change propagation
  - Hazelcast `ClusterHeartbeatManager` — pluggable failure detectors (deadline, phi-accrual, ICMP)
  - Hazelcast `SplitBrainHandler` — master-centric merge detection
  - Hazelcast `ClusterJoinManager` — discovery + handshake ceremony
  - Hazelcast `MigrationPlanner` + `MigrationInfo` — 3-phase migration (prepare, replicate, finalize)
  - Hazelcast `PartitionRuntimeState` — compact binary partition table, version tracking, replica deduplication
  - Partition state machine: REPLICA → BACKUP → MIGRATING → LOST
  - **Quickwit chitchat** — UDP gossip-based membership + failure detection; `ClusterChangeStream` for reactive node join/leave; lightweight alternative to TCP mesh
  - **TiKV per-partition FSM + Batch System** — `BasicMailbox<Fsm>` per partition, batch scheduler pools 271 partitions across few threads; `DashMap<u32, PartitionMetadata>` for lock-free ownership lookup; `RegionState` enum (Pending/Loading/Active/Evicting)
- **Deliverable:** Design document with:
  - Cluster state machine (node lifecycle, partition lifecycle)
  - Membership protocol design (versioned views, heartbeat, failure detection)
  - Migration lifecycle (3-phase commit between master/source/destination)
  - How basic PartitionTable (TODO-063) evolves into full partition system
  - Concrete Rust concurrency: per-partition FSM vs shared state (TiKV), gossip vs mesh (Quickwit)
- **HC Reference:** `hazelcast/internal/cluster/impl/`, `hazelcast/internal/partition/`
- **Rust Reference:**
  - Quickwit chitchat: `/Users/koristuvac/Projects/rust/quickwit/quickwit/quickwit-cluster/src/` — gossip membership, `ClusterChangeStream`, graceful shutdown
  - TiKV batch system: `/Users/koristuvac/Projects/rust/tikv/components/batch-system/src/` — FSM trait, `BasicMailbox`, batch scheduler
  - TiKV raftstore: `/Users/koristuvac/Projects/rust/tikv/components/raftstore/src/store/` — `StoreMeta` (Arc<Mutex>), `PeerFsm`, region state machine
  - TiKV in-memory engine: `/Users/koristuvac/Projects/rust/tikv/components/in_memory_engine/src/` — `RegionState` enum, `DashMap` usage
- **Effort:** 3-5 days
- **Output:** `.specflow/reference/RUST_CLUSTER_ARCHITECTURE.md`

### TODO-082: Service & Operation Architecture Research
- **Priority:** P1 (blocks TODO-065)
- **Complexity:** Medium (research + design)
- **Summary:** Design Rust service registry and operation execution model. TS uses stateless message handlers; Hazelcast uses partition-routable operations with execution barriers, provenance tracking, and service lifecycle hooks.
- **Research scope:**
  - Hazelcast `ServiceManager` — dynamic service registration, interface-based discovery
  - Hazelcast `ManagedService` — lifecycle (init, reset, shutdown)
  - Hazelcast `MigrationAwareService` — migration hooks (prepare, commit, rollback)
  - Hazelcast `AbstractPartitionOperation` — partition-routable operations
  - Hazelcast inbound/outbound handler pipeline — composable middleware
  - How tower middleware maps to Hazelcast's handler pipeline
  - **Quickwit Actor framework** — custom actor system with `Actor` + `Handler<M>` traits, typed mailboxes, `ActorContext`, observable state for testing; `Universe` for lifecycle management; backpressure-aware messaging
  - **Quickwit Tower layers** — `TimeoutLayer`, `RetryLayer`, `LoadShedLayer`, `MetricsLayer`, `EventListenerLayer`; composable via `ServiceBuilder`
  - **TiKV Worker/Scheduler pattern** — `Scheduler<T>` with bounded channel + backpressure, `LazyWorker` with `Runnable` trait, specialized workers per subsystem (GC, CDC, PD)
  - **Databend GlobalServices** — `OnceCell` + `TypeMap` singleton registry, explicit initialization order, `GlobalInstance::set/get` pattern
- **Deliverable:** Design document with:
  - Rust ServiceRegistry trait and lifecycle hooks
  - Operation trait (partition-routable, with provenance)
  - Handler pipeline design (tower-compatible)
  - How 26 TS handlers map to Rust operations
  - Concrete Rust patterns: Actor vs Worker vs direct async (decision matrix)
- **HC Reference:** `hazelcast/spi/impl/`, `hazelcast/internal/partition/operation/`
- **Rust Reference:**
  - Quickwit actors: `/Users/koristuvac/Projects/rust/quickwit/quickwit/quickwit-actors/src/` — Actor/Handler traits, Universe, mailbox, observable state
  - Quickwit tower: `/Users/koristuvac/Projects/rust/quickwit/quickwit/quickwit-common/src/tower/` — custom tower layers
  - Quickwit serve: `/Users/koristuvac/Projects/rust/quickwit/quickwit/quickwit-serve/src/lib.rs` — service composition startup (~lines 428-600)
  - TiKV workers: `/Users/koristuvac/Projects/rust/tikv/components/tikv_util/src/worker/` — Scheduler, LazyWorker, Runnable trait
  - TiKV server init: `/Users/koristuvac/Projects/rust/tikv/src/server/` — layered initialization, `TikvServer` struct assembly
  - Databend global services: `/Users/koristuvac/Projects/rust/databend/src/query/service/src/global_services.rs` — initialization order (~lines 66-218)
- **Effort:** 2-3 days
- **Output:** `.specflow/reference/RUST_SERVICE_ARCHITECTURE.md`

---

## Phase 3: Rust Server Core (~8-10 weeks)

*Goal: Working Rust server that passes existing TS integration tests. Architecture informed by Phase 2.5 research, not by TS server structure.*

### TODO-064: Networking Layer (axum + WebSocket)
- **Priority:** P0
- **Complexity:** Medium
- **Summary:** HTTP + WebSocket server using axum, with deferred startup pattern
- **Key features:**
  - `GET /health`, `POST /sync` (HTTP sync, existing protocol)
  - WebSocket upgrade for real-time sync
  - TLS support (rustls)
  - Channel abstraction (Hazelcast-style, protocol-agnostic)
  - Tower middleware pipeline for handler composition
- **Crates:** axum, tokio-tungstenite, tower, rustls
- **Depends on:** TODO-059 ✅, TODO-062 ✅
- **Effort:** 1-2 weeks

### TODO-067: Multi-Layer Storage System
- **Priority:** P0
- **Complexity:** Medium-Large (redesigned — was "PostgreSQL adapter")
- **Summary:** Implement Hazelcast-informed multi-layer storage architecture per TODO-080 research.
- **Architecture (expected from research):**
  - **Layer 1: Storage trait** — low-level key-value with cursor-based iteration, mutation-tolerant iterators
  - **Layer 2: RecordStore trait** — adds TTL/expiry, eviction, record metadata (version, timestamps, hit count)
  - **Layer 3: MapDataStore trait** — persistence bridge (write-through / write-behind), soft/hard flush
  - **Concrete implementations:** InMemoryStorage, PostgresMapDataStore
  - **Record metadata as first-class:** enables LWW conflict resolution, eviction policies, replication lag tracking
- **Key constraint:** Trait boundaries must support future S3 (TODO-043), tiered storage (TODO-040), and write-behind (TODO-033) without trait redesign.
- **Crates:** sqlx (compile-time checked queries)
- **Depends on:** TODO-060 ✅ (ServerStorage trait — will be expanded), **TODO-080 (research)**
- **Effort:** 2-3 weeks

### TODO-065: Operation Routing and Execution
- **Priority:** P0
- **Complexity:** Large (redesigned — was "Port 26 handlers")
- **Summary:** Implement Hazelcast-informed operation execution model per TODO-082 research. Not a port of TS stateless handlers.
- **Architecture (expected from research):**
  - **Operation trait** — partition-routable with provenance tracking (local, backup, WAN, client)
  - **ServiceRegistry** — dynamic service registration with lifecycle hooks (init, reset, shutdown)
  - **MigrationAwareService** — services can hook into migration lifecycle (prepare, commit, rollback)
  - **Execution barriers** — partition migration blocks operations on that partition
  - **Handler pipeline** — tower-compatible middleware composition
- **Domains:** CRDT, Sync, Query, Messaging, Coordination, Search, Persistence, Client/Server
- **Known TS bugs (covered by redesign):**
  - `BatchProcessingHandler.processBatchAsync` nests inter-node forwarded messages incorrectly
  - `PartitionService.getPartitionMap()` returns wrong ports
- **Depends on:** TODO-061 ✅, TODO-062 ✅, TODO-064, **TODO-082 (research)**
- **Effort:** 3-4 weeks

### TODO-066: Cluster Protocol
- **Priority:** P1
- **Complexity:** Large (redesigned — was "ClusterManager + WebSocket mesh")
- **Summary:** Implement Hazelcast-informed cluster protocol per TODO-081 research. Not a port of TS ClusterManager.
- **Architecture (expected from research):**
  - **Versioned MembersView** — clients detect stale membership without full state comparison
  - **Master-centric coordination** — master decides partition assignment, initiates migrations
  - **Explicit join ceremony** — discovery → handshake → state sync (not automatic mesh)
  - **3-phase migration lifecycle** — prepare (lock source) → replicate → finalize (release source)
  - **Full partition state machine** — extends TODO-063 basic table with REPLICA/BACKUP/MIGRATING/LOST states
  - **Pluggable failure detection** — phi-accrual (portable from TS — one of few well-designed TS components)
  - **Split-brain detection** — master-centric, not peer-to-peer consensus
- **TS Source:** `packages/server/src/cluster/` (behavioral reference only)
- **HC Reference:** `hazelcast/internal/cluster/impl/` (primary architectural source)
- **Depends on:** TODO-063, TODO-064, **TODO-081 (research)**
- **Effort:** 3-4 weeks

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
- **Depends on:** TODO-066 (Cluster Protocol), TODO-063
- **Effort:** 1-2 weeks

### TODO-025: DAG Executor for Distributed Queries
- **Priority:** P2
- **Complexity:** Large
- **Summary:** Hazelcast-style DAG executor for distributed query processing
- **Architecture:** 3-tier processor model (Source → Transform → Sink), partition-aware, backpressure via tokio channels
- **Context:** [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) (700+ lines)
- **HC Reference:** `hazelcast/jet/core/` (Processor, Inbox, Outbox), `jet/impl/execution/` (TaskletTracker, CooperativeWorker, StoreSnapshotTasklet)
- **Key insight:** Rust `Future::poll()` maps naturally to Cooperative Tasklet model
- **Depends on:** TODO-060 ✅ (Processor trait), TODO-063
- **Effort:** 2-3 weeks

### TODO-071: Search with Tantivy
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Replace custom BM25 search with tantivy full-text search engine
- **Benefits:** Orders of magnitude faster, built-in tokenization, fuzzy search, phrase queries
- **Crate:** tantivy
- **Source:** `packages/server/src/search/`
- **Effort:** 1-2 weeks

### TODO-076: Evaluate MsgPack-Based Merkle Hashing
- **Priority:** P3 (deferred from P2 — premature optimization)
- **Complexity:** Medium (design decision + implementation in both TS and Rust)
- **Summary:** Current Merkle hashing converts values to JSON string then FNV-1a hashes the string. Alternative: hash MsgPack bytes directly. Evaluate after Rust server is functional.
- **Depends on:** TODO-075 (fix current hashing first), working Rust server
- **Effort:** 1 day (evaluation) + 2-3 days (implementation if approved)

---

## Phase 5: Post-Migration Features (~8-12 weeks, after Rust server launch)

*Goal: Enterprise and advanced features built natively in Rust.*

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Hazelcast-style Write-Behind pattern: staging area, write coalescing, batch flush, retry queue
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **HC Reference:** `hazelcast/map/impl/mapstore/` — WriteBehindStore, StoreWorker, coalescing logic, retry with backoff
- **Note:** TODO-067 (Multi-Layer Storage) should provide MapDataStore trait that this plugs into seamlessly
- **Depends on:** TODO-067 (MapDataStore trait)
- **Effort:** 2-3 weeks

### TODO-043: S3 Bottomless Storage
- **Priority:** P3
- **Complexity:** Very Large
- **Summary:** Append-only log in S3/R2/GCS. Immutable log segments, replay on startup, Merkle checkpoints
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7
- **Crates:** aws-sdk-s3, opendal
- **Note:** TODO-067 (Multi-Layer Storage) should provide Storage trait that S3 implements without trait redesign
- **Depends on:** TODO-033 (AsyncStorageWrapper)
- **Effort:** 6-8 weeks

### TODO-040: Tiered Storage (Hot/Cold)
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Hot data in memory, cold data in S3/cheap storage with transparent migration
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Note:** TODO-067 (Multi-Layer Storage) RecordStore trait with record metadata (hit count, access time) enables eviction policies
- **Depends on:** TODO-033 (AsyncStorageWrapper), TODO-067 (RecordStore trait)
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
- **Note:** TODO-082 (Service Architecture Research) should design ServiceRegistry trait that this builds on
- **Effort:** 2-3 weeks

### TODO-041: Multi-Tenancy
- **Priority:** P4
- **Complexity:** Large
- **Summary:** Per-tenant isolation, quotas, billing, tenant-aware partitioning
- **Context:** [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Depends on:** TODO-060 ✅ (RequestContext.tenant_id)
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
| **1** | SPEC-052e (Message union) · TODO-074 (HLC ✓) · TODO-078 (TS hash fix) · TODO-075 (ORMap hash) | — | 2 |
| **1a** | TODO-077 (CI drift check) · TODO-063 (Basic partitions) | 052e · — | 2 |
| **2** | **TODO-080 (Storage research)** · **TODO-081 (Cluster research)** · **TODO-082 (Service research)** | — (can start parallel to Wave 1) | 2.5 |
| **3** | TODO-064 (Network) · TODO-067 (Multi-Layer Storage) | 062 · **080** | 3 |
| **4** | TODO-065 (Operation Routing) · TODO-066 (Cluster) · TODO-068 (Tests, incremental) | 064+**082** · 063+064+**081** · 064 | 3 |
| **5** | TODO-069 (Schema) · TODO-048 (SSE) · TODO-025 (DAG) · TODO-071 (Tantivy) | — · — · 063 · — | 4 |
| **6** | TODO-070 (Shapes) · TODO-049 (Cluster HTTP) · TODO-076 (MsgPack hash eval) | 069 · 066+063 · 075 | 4 |
| **7** | TODO-033 (AsyncStorage) · TODO-039 (Vector) · TODO-036 (Extensions) · TODO-041 (Multi-tenancy) · TODO-072 (WASM) | 067 · — · — · — · — | 5 |
| **8** | TODO-043 (S3) · TODO-040 (Tiered) | 033 · 033+067 | 5 |
| **9** | TODO-044 (Time-Travel) | 043 | 5 |

**Current position:** Wave 1 — SPEC-052e is next (all dependencies met). TODO-080 (Storage research) can start in parallel.

---

## Dependency Graph

```
Phase 0 (TypeScript) [DONE]     Phase 1 (Bridge) [DONE]
SPEC-048b ──→ SPEC-048c          TODO-059 (Cargo) ──→ TODO-060 (Traits)
                                    │
                                    ↓
                         Phase 2 (Rust Core)
              TODO-061 (CRDTs) [DONE]     TODO-079 (Schema arch) [DONE]
                    │
                    ↓
              TODO-062 (Message Schema) [DONE] ─── 052a-e complete
                    │
                    ↓
    │         │
    │   TODO-074 (HLC ✓) · TODO-078 (TS hash P1) · TODO-075 (ORMap hash)
    │         │
    │   TODO-077 (CI drift) · TODO-063 (Basic partitions)  ←── WAVE 1a
    │         │
    │         ↓
    │  Phase 2.5 (Research Sprint)  ←── WAVE 2
    │  ┌──────────────────────────────────────────┐
    │  │ TODO-080        TODO-081        TODO-082  │
    │  │ Storage arch    Cluster proto   Service   │
    │  │ research        research        arch      │
    │  └──────┬──────────────┬──────────────┬──────┘
    │         │              │              │
    └─────────┼──────────────┼──────────────┘
              ↓              ↓
         Phase 3 (Rust Server)  ←── WAVES 3-4
    TODO-064 (Network)
        │
        ├──→ TODO-067* (Multi-Layer Storage) ←── TODO-080
        │
        ├──→ TODO-065* (Operation Routing) ←── TODO-082
        │
        ├──→ TODO-066* (Cluster Protocol) ←── TODO-081 + TODO-063
        │
        └──→ TODO-068 (Integration Tests)
              │
              ↓
         Phase 4 (Features)  ←── WAVES 5-6
  TODO-069 (Schema) ──→ TODO-070 (Shapes)
  TODO-048 (SSE)
  TODO-049 (Cluster HTTP)
  TODO-025 (DAG Executor)
  TODO-071 (Tantivy Search)
  TODO-076 (MsgPack hash eval — deferred)
              │
              ↓
         Phase 5 (Post-Migration)  ←── WAVES 7-9
  TODO-033 (AsyncStorage) ──→ TODO-043 (S3) ──→ TODO-044 (Time-Travel)
                          └──→ TODO-040 (Tiered Storage)
  TODO-039 (Vector Search)
  TODO-036 (Extensions)
  TODO-041 (Multi-Tenancy)
  TODO-072 (WASM Modules)

  * = redesigned after research (not TS port)
```

---

## Timeline Summary

| Phase | Effort | Prerequisites |
|-------|--------|---------------|
| **0. TypeScript Completion** | 3-4 days | ~~Current codebase~~ DONE |
| **1. Bridge** | 1-2 days | ~~Phase 0 complete~~ DONE |
| **2. Rust Core** | 3-4 weeks | ~~Phase 1 complete~~ IN PROGRESS (052e + bug fixes remaining) |
| **2.5 Research Sprint** | 1-2 weeks | Can start parallel to Phase 2 completion |
| **3. Rust Server** | 8-10 weeks | Phase 2 + 2.5 complete |
| **4. Rust Features** | 4-6 weeks | Phase 3 complete (some items parallelizable) |
| **5. Post-Migration** | 8-12 weeks | Phase 4 complete (independent items) |
| **Total to Rust server launch (Phases 0-3)** | **~14-18 weeks** | |
| **Total with features (Phases 0-4)** | **~18-24 weeks** | |

## Eliminated Items

| TODO | Reason | Date |
|------|--------|------|
| TODO-042 (DBSP) | Not needed; StandingQueryRegistry + ReverseQueryIndex sufficient | 2026-02-10 |
| TODO-034 (Rust/WASM hot paths) | Superseded by full Rust migration | 2026-02-10 |
| TODO-045 (DST Documentation) | TS testing infrastructure documentation for deprecated system | 2026-02-18 |

## Completed Items (archived)

### Phase 2 Rust Items

| TODO | Spec | Completed |
|------|------|-----------|
| TODO-059 → bootstrap | Cargo workspace + CI | 2026-02-13 |
| TODO-060 → SPEC-050 | 6 foundational traits | 2026-02-13 |
| TODO-061 → SPEC-051 | Core CRDTs (LWWMap, ORMap, HLC, MerkleTree) | 2026-02-14 |
| TODO-079 → SPEC-054 | Message schema architecture fix-on-port | 2026-02-17 |
| TODO-062 → SPEC-052a-e | Message schema all 8 domains + union + cross-lang tests (393 Rust + 62 TS tests) | 2026-02-19 |

### Phase 0 TypeScript Items

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
| TODO-080 | Output: `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` |
| TODO-081 | Output: `.specflow/reference/RUST_CLUSTER_ARCHITECTURE.md` |
| TODO-082 | Output: `.specflow/reference/RUST_SERVICE_ARCHITECTURE.md` |

---

*Restructured 2026-02-12: Replaced wave-based organization with phase-based Rust migration roadmap. Added TODO-059 through TODO-072 for Rust-specific work. Product positioning decisions (schema, shapes, WASM) integrated as concrete TODOs.*
*Updated 2026-02-15: Added TODO-074 through TODO-078 from external audit analysis. HLC validation, ORMap hash determinism bug, TS hash inconsistency, protocol drift CI, MsgPack hash evaluation.*
*Updated 2026-02-18: Strategic audit. Added Phase 2.5 (Architecture Research Sprint: TODO-080, 081, 082). Redesigned Phase 3 items (TODO-063/065/066/067) from "TS port" to "Hazelcast-informed design". Upgraded TODO-078 to P1 (client-server hash compatibility confirmed). Deferred TODO-076 to Phase 4. Eliminated TODO-045. Marked completed Phase 2 items.*
*Updated 2026-02-19: Added Triple Reference Protocol. Rust OSS projects (TiKV, Quickwit, Databend) added as implementation pattern references alongside Hazelcast conceptual architecture. Research tasks TODO-080/081/082 updated with concrete Rust file paths. Rationale: Java→Rust translation has real friction (ownership, no inheritance, no GC) — Rust-native patterns needed for storage traits (TiKV engine_traits), cluster concurrency (TiKV FSM+DashMap), service composition (Quickwit actors+Tower), object storage (Databend OpenDAL).*
