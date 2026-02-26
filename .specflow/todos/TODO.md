# TopGun Roadmap

**Last updated:** 2026-02-25
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
   - **Arroyo** (`/Users/koristuvac/Projects/rust/arroyo/`) — distributed stream DAG (petgraph), DataFusion SQL planning (target_partitions=1 + custom distribution), barrier checkpointing, partial→final aggregation, connector traits, operator chaining
   - **ArkFlow** (`/Users/koristuvac/Projects/rust/arkflow/`) — DataFusion SessionContext integration, MsgPack→Arrow codec patterns, Input/Output/Codec connector traits

**Fix-on-port rule:** Before porting a domain, audit the TS source. Fix bugs/dead code in TS first, then port the corrected version. See PROJECT.md "Rust Migration Principles".

| Rust TODO | TopGun TS Source | Hazelcast (WHAT) | Rust OSS (HOW) |
|---|---|---|---|
| TODO-063 Partitions | `server/src/cluster/PartitionService.ts` | `internal/partition/` | TiKV: `DashMap`, `RegionState` enum |
| TODO-064 Network | `server/src/modules/network-module.ts` | `internal/networking/` | SurrealDB: WS lifecycle; Grafbase: middleware; Quickwit: Tower |
| TODO-065 Operations | `server/src/coordinator/` | `spi/`, `internal/partition/operation/` | Quickwit: Actor+Tower; TiKV: Worker/Scheduler |
| TODO-066 Cluster | `server/src/cluster/` | `internal/cluster/impl/` | Quickwit: chitchat; TiKV: FSM batch system |
| TODO-067 Storage | `server/src/storage/` | `map/impl/recordstore/`, `mapstore/` | TiKV: `engine_traits`; Databend: OpenDAL |
| TODO-025 DAG | [DAG spec](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) | `jet/core/`, `jet/impl/execution/` | **Arroyo**: petgraph DAG, operator chaining, barrier checkpoints, shuffle edges; Databend: pipeline `StableGraph` |
| TODO-088 Query | `server/src/coordinator/handlers/` | `map/impl/query/` | **Arroyo**: DataFusion `target_partitions=1` + custom distribution, partial→final agg; **ArkFlow**: SessionContext, MsgPack→Arrow codec |
| TODO-033 AsyncStorage | — | `map/impl/mapstore/` (Write-Behind) | TiKV: `Scheduler<T>` async flush |
| TODO-040 Tiered | — | `map/impl/eviction/`, `map/impl/record/` | TiKV: `in_memory_engine` hot/cold |
| TODO-041 Multi-tenancy | — | `security/`, `access/` | SurrealDB: `Namespace→Database→Table` hierarchy |
| TODO-036 Extensions | — | `spi/` (SPI) | Databend: `GlobalInstance` registry |
| TODO-071 Search | `server/src/search/` | `query/`, `map/impl/query/` | Quickwit: tantivy integration, SearchService trait |
| TODO-043 S3 Storage | — | — | Databend: OpenDAL `Operator` + layer stack |
| TODO-090 PostgreSQL | `server/src/storage/PostgreSQLAdapter.ts` | `map/impl/mapstore/` (write-through) | sqlx PgPool |
| TODO-091 DataFusion SQL | — | `sql/` (Calcite — conceptual) | **Arroyo**: `target_partitions=1` + shuffle; **ArkFlow**: SessionContext |
| TODO-092 Connectors | — | — | **Arroyo**: connector traits, SourceOperator/ArrowOperator; **ArkFlow**: Input/Output/Codec |

**Not relevant from Hazelcast:** `cp/` (Raft), `transaction/`, `wan/`, `cache/` (JCache), Spring modules. *(Note: `sql/` (Calcite) — conceptually relevant for distributed SQL planning, but TopGun uses DataFusion instead of Calcite.)*

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

### TODO-063: Basic Partition Hash and Lookup → SPEC-056 — DONE
- **Status:** Complete (partition hash + PartitionTable implemented as part of cluster protocol work)
- **Summary:** `hash_to_partition(key) -> u32` (FNV-1a % 271), `PartitionTable` struct, partition pruning. Subsumed by TODO-066 (SPEC-060a-e).

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

### ~~TODO-083: Networking Layer Research (Light)~~ ✅
- **Priority:** P1 (blocks TODO-064)
- **Complexity:** Low (targeted research, 1-2 days)
- **Status:** COMPLETE (2026-02-20, RES-006)
- **Summary:** Targeted research on Rust networking patterns for IMDG server. Not a full architecture sprint — axum + tower are already chosen. Focus on questions that could cause costly retrofits: connection abstraction (WebSocket-only → gRPC/QUIC later?), per-connection state management at 10K+ scale, backpressure strategy, middleware ordering.
- **Research scope:**
  - **SurrealDB** — WebSocket connection lifecycle, multi-protocol support (HTTP REST + WS + potentially gRPC), connection authentication flow, per-connection state management
  - **Grafbase** — edge deployment patterns, middleware composition, protocol negotiation, gateway architecture
  - **Quickwit** — Tower layer stacking (already partially studied in TODO-082), service composition startup, gRPC + REST coexistence
  - Connection abstraction: should TopGun have a protocol-agnostic Channel trait (like Hazelcast) or is axum's extractors + tower sufficient?
  - Backpressure: per-connection flow control, slow client handling, message queue bounds
  - Graceful shutdown: draining connections, health check transitions
- **Deliverable:** Short design document (~2-3 pages) with:
  - Connection trait design (or decision to not abstract)
  - Middleware stack ordering recommendation
  - Backpressure strategy
  - Patterns to adopt from each reference project
- **Rust Reference:**
  - SurrealDB: `/Users/koristuvac/Projects/rust/surrealdb/` — WebSocket server, connection management, RPC layer
  - Grafbase: `/Users/koristuvac/Projects/rust/grafbase/` — gateway architecture, middleware, protocol handling
  - Quickwit serve: `/Users/koristuvac/Projects/rust/quickwit/quickwit/quickwit-serve/src/` — Tower layers, gRPC + REST startup
- **Effort:** 1-2 days
- **Output:** `.specflow/reference/RUST_NETWORKING_PATTERNS.md`

---

## Phase 3: Rust Server Core (~8-10 weeks)

*Goal: Working Rust server that passes existing TS integration tests. Architecture informed by Phase 2.5 research, not by TS server structure.*

### TODO-064: Networking Layer (axum + WebSocket) → SPEC-057 — DONE
- **Status:** Complete (SPEC-057 split into 057a/b/c, all archived)
- **Summary:** axum HTTP + WebSocket server with ConnectionHandle, bounded mpsc backpressure, Tower middleware (LoadShed → Timeout → Metrics), graceful shutdown, health/sync/WS handlers, NetworkModule assembly
- **Rust code:** `packages/server-rust/src/network/` (config, connection, shutdown, handlers/, middleware, module)

### TODO-067: Multi-Layer Storage System → SPEC-058 — DONE
- **Status:** Complete (SPEC-058 split into 058a/b/c, all archived)
- **Summary:** Hazelcast-informed 3-layer storage: StorageEngine trait (L1), RecordStore trait with TTL/expiry/eviction/metadata (L2), MapDataStore trait for persistence (L3). HashMapStorage + NullDataStore + DefaultRecordStore + RecordStoreFactory. MutationObserver for index/Merkle hooks.
- **Rust code:** `packages/server-rust/src/storage/` (engine, record, record_store, map_data_store, mutation_observer, engines/, datastores/, impls/, factory)

### TODO-065: Operation Routing and Execution → SPEC-059 — DONE
- **Status:** Complete (SPEC-059, archived)
- **Summary:** Hazelcast-informed operation routing: ServiceRegistry with ManagedService lifecycle, Operation enum with CallerOrigin provenance, OperationRouter dispatching to 7 domain services, Tower middleware pipeline (LoadShed → Timeout → Metrics), BackgroundWorker for periodic tasks, domain_stub! macro for stub services
- **Rust code:** `packages/server-rust/src/service/` (config, registry, operation, classify, router, middleware/, worker, domain/)

### TODO-066: Cluster Protocol → SPEC-060 — DONE
- **Status:** Complete (SPEC-060 split into 060a-e, all archived). 288 server-rust tests, 0 failures.
- **Summary:** Hazelcast-informed cluster protocol: 5 service traits, 18-variant ClusterMessage, versioned MembersView (ArcSwap), DashMap partition table, phi-accrual + deadline failure detectors, partition assignment + rebalancing algorithms, MigrationCoordinator with 3-phase lifecycle, 4 resilience processors (SplitBrain, HeartbeatComplaint, MastershipClaim, GracefulLeave), `decide_merge()` deterministic split-brain resolution
- **Rust code:** `packages/server-rust/src/cluster/` (traits, types, messages, state, assignment, migration, resilience)

### Phase 3b: Domain Service Implementations

*Goal: Replace stubs with working domain logic. After this, TS client can connect to Rust server and perform real operations. Part of Milestone 1 (v1.0).*

### TODO-084: CoordinationService (Ping + PartitionMap + Heartbeat) → SPEC-061
- **Priority:** P0 (validates full pipeline end-to-end)
- **Complexity:** Small
- **Summary:** First real domain service. Handles Ping → Pong, PartitionMapRequest → PartitionMap, HeartbeatAck. Validates the complete path: WS message → deserialize → classify → route → domain service → serialize → WS response.
- **Scope:**
  - `Ping` → respond with `Pong` (timestamp echo)
  - `PartitionMapRequest` → respond with `PartitionMap` from `ClusterPartitionTable`
  - `HeartbeatAck` → update failure detector
  - Wire `CoordinationService` to `ClusterState` + `ConnectionRegistry`
- **TS Source:** `packages/server/src/coordinator/handlers/` (PingHandler, PartitionMapHandler)
- **Depends on:** TODO-065 ✅ (OperationRouter), TODO-066 ✅ (ClusterState)
- **Effort:** 2-3 days

### TODO-085: CrdtService (LWWMap + ORMap Operations) → SPEC-062
- **Priority:** P0 (core data path)
- **Complexity:** Medium
- **Summary:** Process client write operations through CRDT merge into RecordStore. This is the core data path: client sends `ClientOp` / `OpBatch`, server merges via LWW/OR rules, persists to RecordStore, broadcasts to subscribers.
- **Scope:**
  - `ClientOp` (single put/remove) → CRDT merge → RecordStore.put() with CallerProvenance::CrdtMerge
  - `OpBatch` → batch processing with per-key partition routing
  - `ServerEvent` broadcast to subscribed connections via ConnectionRegistry
  - Wire `CrdtService` to `RecordStoreFactory` + `ConnectionRegistry`
  - ORMap operations: add/remove with tag tracking
- **TS Source:** `packages/server/src/coordinator/handlers/` (ClientOpHandler, OpBatchHandler, CrdtMergeHandler)
- **Depends on:** TODO-084 ✅ (pipeline validated), TODO-067 ✅ (RecordStore)
- **Effort:** 1-2 weeks

### TODO-086: SyncService (Merkle Delta Sync) → SPEC-063 — DONE
- **Status:** Complete (SPEC-063 completed 2026-02-25)
- **Summary:** Merkle delta sync protocol: MerkleSyncManager, MerkleMutationObserver, LWW/OR-Map handlers, SyncService dispatch. Efficient delta sync for offline-first clients.

### TODO-087: MessagingService (Topic Pub/Sub) → SPEC-064 — DONE
- **Status:** Complete (SPEC-064 completed 2026-02-25, 360 tests, clippy clean)
- **Summary:** Topic pub/sub: TopicSubscribe/Unsubscribe, TopicPublish fan-out via ConnectionRegistry, topic lifecycle auto-cleanup. Fourth domain service replacing stub.

### TODO-088: QueryService (Live Queries)
- **Priority:** P1 (reactive UI)
- **Complexity:** Medium-Large
- **Summary:** Live query subscriptions. Client subscribes to a query, server evaluates it against current data, then pushes incremental updates as data changes. Critical for reactive UI patterns.
- **Scope:**
  - `QuerySubscribe` → evaluate query, return initial results, register standing query
  - `QueryUnsubscribe` → remove standing query
  - Standing query re-evaluation on RecordStore mutations (via MutationObserver)
  - `QueryUpdate` push to subscribed connections
  - Query filter evaluation (key prefix, field match, range)
- **Architecture (DataFusion-ready):**
  - **Dual format:** MsgPack stays for wire protocol/CRDT/replication. Arrow only inside query engine (lazy cache).
  - Define `QueryBackend` trait abstracting query execution (`execute_query(&self, map, predicate, projection, limit) → Stream<QueryResult>`)
  - **PredicateEngine** (Phase 3b, default backend — no SQL overhead):
    - L1: Eq, Gt, Lt, In, Exists, Prefix — single field filters
    - L2: AND/OR/NOT combinators, ORDER BY, LIMIT/OFFSET — covers ~80% CRUD apps
    - L3: Nested field access, simple aggregations (count/min/max/sum), distinct, projection — covers ~90% single-Map queries
  - `RecordStore::scan()` must return `Stream<Item = (Key, Record)>` — compatible with DataFusion TableProvider scan
  - `Record::to_value_map() → BTreeMap<String, Value>` for predicate evaluation
  - Accept `Query::Predicate(..) | Query::Sql(String)` — dual API, SQL variant reserved for DataFusion
  - **Arrow cache layer:** lazy MsgPack → Arrow RecordBatch conversion on first SQL query per Map, invalidated on mutation via MutationObserver
  - **Future SQL (Phase 4-5, feature-gated):**
    - `#[cfg(feature = "sql")] DataFusionBackend` — TopGunTableProvider implements DataFusion's TableProvider trait
    - Server: DataFusion plans with `target_partitions=1`, TopGun distributes via partition owners + shuffle edges (Arroyo pattern — no Ballista needed)
    - Partial→Final aggregation: partial aggregate per partition, merge on coordinator (Arroyo's proven approach)
    - Client: DataFusion WASM for offline SQL (same dialect server & client)
    - Ref: Arroyo (`arroyo-planner/builder.rs`, `arroyo-datastream/logical.rs`), ArkFlow (SessionContext, MsgPack→Arrow codec)
- **TS Source:** `packages/server/src/coordinator/handlers/` (QuerySubscribeHandler, QueryResultHandler)
- **Depends on:** TODO-085 (data path must work), TODO-067 ✅ (MutationObserver)
- **Effort:** 1-2 weeks

### TODO-089: PersistenceService (Counters + Entry Processing)
- **Priority:** P2 (advanced features)
- **Complexity:** Medium
- **Summary:** Server-side counters, entry processing (user-defined transforms), and journal subscriptions. These are specialized server-side computation features.
- **Scope:**
  - `CounterRequest` (increment/decrement/get) → atomic counter operations
  - `EntryProcessRequest` → execute user-defined transform on a record (read-modify-write)
  - `JournalSubscribe` → stream of all mutations for audit/CDC
  - `ResolverRequest` → server-side conflict resolution
- **TS Source:** `packages/server/src/coordinator/handlers/` (CounterHandler, EntryProcessHandler)
- **Depends on:** TODO-085 (data path), TODO-067 ✅ (RecordStore)
- **Effort:** 1-2 weeks

### TODO-068: Integration Test Suite
- **Priority:** P0
- **Complexity:** Large
- **Summary:** Port critical test scenarios, use TS server as behavioral oracle
- **Approach:**
  - Run identical test scenarios against TS server and Rust server
  - Compare behavior for: CRDT merge, sync protocol, cluster operations, query results
  - Client-server tests: TS client connects to Rust server
  - **Incremental:** Start after TODO-084 (Ping e2e), expand as each domain service lands
- **Source:** `packages/server/src/__tests__/`, `tests/e2e/`
- **Depends on:** TODO-084+ (incremental — each domain service adds testable surface)
- **Effort:** 3-4 weeks (concurrent with domain service work)

---

## Milestone 1: Working IMDG (v1.0)

*Goal: TopGun Rust server replaces TS server. Clients connect, write/read data, sync offline, search, query. PostgreSQL persistence for durability. Production-usable.*

### TODO-090: PostgreSQL MapDataStore Adapter — NEW
- **Priority:** P0 (v1.0 requires durable persistence)
- **Complexity:** Medium
- **Summary:** First real persistence backend. Implements `MapDataStore` trait for PostgreSQL using sqlx. Write-through mode: every RecordStore mutation persists synchronously. Load-all-keys on startup for cache warm-up.
- **Scope:**
  - `PostgresDataStore` implements `MapDataStore` trait (load, store, delete, load_all_keys)
  - Connection pooling via sqlx `PgPool`
  - Schema migration (CREATE TABLE for map data)
  - Bulk load on server startup (warm cache from PostgreSQL)
  - Integration with `RecordStoreFactory` — configurable per-map data store
- **Architecture:** Pure adapter — no coupling to PostgreSQL internals. Same `MapDataStore` trait works for MySQL, SQLite, DynamoDB via different impls.
- **TS Source:** `packages/server/src/storage/PostgreSQLAdapter.ts`
- **HC Reference:** `hazelcast/map/impl/mapstore/` — MapStoreWrapper (write-through path)
- **Depends on:** TODO-067 ✅ (MapDataStore trait)
- **Effort:** 1 week

### TODO-071: Search with Tantivy
- **Priority:** P1 (table stakes for real-time apps)
- **Complexity:** Medium
- **Summary:** Replace custom BM25 search with tantivy full-text search engine
- **Benefits:** Orders of magnitude faster, built-in tokenization, fuzzy search, phrase queries
- **Crate:** tantivy
- **Source:** `packages/server/src/search/`
- **Effort:** 1-2 weeks

### TODO-074: HLC Node ID Colon Validation (TS + Rust)
- **Priority:** P2 (hardening)
- **Complexity:** Trivial
- **Summary:** Reject node IDs containing `:` in HLC constructor (breaks `HLC.parse()`)
- **Effort:** 1-2 hours

### TODO-075: Fix Rust ORMap Merkle Hash Determinism → SPEC-055
- **Priority:** P1 (bug — cross-language sync broken)
- **Status:** Spec created (SPEC-055), pending audit
- **Summary:** `hash_entry()` uses unsorted `serde_json::to_string()` — breaks cross-language Merkle sync
- **Depends on:** TODO-061 ✅
- **Effort:** 0.5 day

### TODO-068: Integration Test Suite
- **Priority:** P0 (gates v1.0 release)
- **Complexity:** Large
- **Summary:** Port critical test scenarios, use TS server as behavioral oracle
- **Approach:**
  - Run identical test scenarios against TS server and Rust server
  - Compare behavior for: CRDT merge, sync protocol, cluster operations, query results
  - Client-server tests: TS client connects to Rust server
  - **Incremental:** Start after TODO-084 (Ping e2e), expand as each domain service lands
- **Source:** `packages/server/src/__tests__/`, `tests/e2e/`
- **Depends on:** TODO-084+ (incremental — each domain service adds testable surface)
- **Effort:** 3-4 weeks (concurrent with domain service work)

---

## Milestone 2: Data Platform (v2.0)

*Goal: SQL queries, stream processing, schema validation, connectors — competitive with Hazelcast feature set.*

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

### TODO-091: DataFusion SQL Integration — NEW
- **Priority:** P1 (distributed SQL — Hazelcast-level queries)
- **Complexity:** Large
- **Summary:** Integrate Apache DataFusion as SQL query engine. Server-side distributed queries across partitions + client-side WASM for offline SQL (same dialect everywhere).
- **Scope:**
  - `DataFusionBackend` implements `QueryBackend` trait (feature-gated: `#[cfg(feature = "sql")]`)
  - `TopGunTableProvider` implements DataFusion's `TableProvider` trait — wraps RecordStore
  - Arrow cache layer: lazy MsgPack → Arrow RecordBatch conversion, invalidated on mutation via MutationObserver
  - Distributed execution: DataFusion plans with `target_partitions=1`, TopGun distributes via partition owners + shuffle edges (Arroyo pattern)
  - Partial→Final aggregation: partial aggregate per partition, merge on coordinator
  - `Query::Sql(String)` variant activates DataFusion path
- **Ref:** Arroyo (`arroyo-planner/builder.rs`, `arroyo-datastream/logical.rs`), ArkFlow (SessionContext, MsgPack→Arrow codec)
- **Depends on:** TODO-088 ✅ (QueryBackend trait), TODO-067 ✅ (RecordStore)
- **Effort:** 2-3 weeks

### TODO-025: DAG Executor for Stream Processing
- **Priority:** P1 (Hazelcast Jet equivalent)
- **Complexity:** Large
- **Summary:** Distributed stream processing DAG. SQL-defined pipelines compiled to operator graphs with windowed aggregation, stateful processing, and checkpointing.
- **Architecture:** petgraph DiGraph, operator chaining, barrier-based checkpointing, shuffle edges for hash-based repartitioning, partial→final aggregation split
- **Context:** [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) (700+ lines)
- **HC Reference:** `hazelcast/jet/core/` (Processor, Inbox, Outbox), `jet/impl/execution/` (TaskletTracker, CooperativeWorker)
- **Rust Reference:** Arroyo (petgraph DAG, operator chaining, barrier checkpoints, shuffle edges)
- **Key insight:** Rust `Future::poll()` maps naturally to Cooperative Tasklet model
- **Depends on:** TODO-091 (DataFusion for SQL pipeline compilation)
- **Effort:** 3-4 weeks

### TODO-092: Connector Framework — NEW
- **Priority:** P2 (extensible data ingestion/egress)
- **Complexity:** Medium
- **Summary:** Trait-based connector system for external data sources and sinks. Enables stream processing pipelines to read from/write to Kafka, S3, databases, webhooks.
- **Scope:**
  - `ConnectorSource` trait: connect, read → RecordBatch stream, close
  - `ConnectorSink` trait: connect, write RecordBatch, close
  - `Codec` trait: encode/decode between wire format and Arrow
  - Connector registry for dynamic discovery
  - Initial connectors: Kafka source/sink, S3 sink, PostgreSQL CDC source
- **Ref:** Arroyo connector traits, ArkFlow Input/Output/Codec pattern
- **Depends on:** TODO-025 (DAG executor for pipeline integration)
- **Effort:** 2 weeks

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Hazelcast-style Write-Behind pattern: staging area, write coalescing, batch flush, retry queue
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **HC Reference:** `hazelcast/map/impl/mapstore/` — WriteBehindStore, StoreWorker, coalescing logic, retry with backoff
- **Depends on:** TODO-067 ✅ (MapDataStore trait), TODO-090 (PostgreSQL adapter for testing)
- **Effort:** 2-3 weeks

### TODO-036: Pluggable Extension System
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Modular extension system for community contributions (crypto, compression, audit, geo)
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5
- **Depends on:** TODO-082 ✅ (ServiceRegistry trait)
- **Effort:** 2-3 weeks

### TODO-072: Selective WASM Modules for Client
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Compile DataFusion SQL engine, tantivy search, and Entry Processors to WASM for browser use. DataFusion WASM enables same SQL dialect offline (client) and online (server).
- **Key constraint:** NOT for basic CRDT ops (sync JS is faster due to WASM boundary cost)
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.6
- **Depends on:** TODO-091 (DataFusion), TODO-071 ✅ (tantivy)
- **Effort:** 2-3 weeks

### TODO-048: SSE Push for HTTP Sync
- **Priority:** P3
- **Complexity:** Low (trivial in Rust)
- **Summary:** Server-Sent Events transport for real-time push in serverless
- **Architecture:**
  - `GET /events` SSE endpoint via axum
  - `SsePushProvider` implements IConnectionProvider
  - `AutoConnectionProvider` gains third tier: WS → SSE → HTTP polling
- **Context:** Extends SPEC-036 (HTTP Sync Protocol)
- **Effort:** 2-3 days

### TODO-049: Cluster-Aware HTTP Routing
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** HttpSyncHandler routes to partition owners in cluster
- **Depends on:** TODO-066 ✅ (Cluster Protocol)
- **Effort:** 1-2 weeks

### TODO-076: Evaluate MsgPack-Based Merkle Hashing
- **Priority:** P3 (deferred optimization)
- **Complexity:** Medium
- **Summary:** Hash MsgPack bytes directly instead of JSON string → FNV-1a. Evaluate after Rust server is functional.
- **Depends on:** TODO-075 (fix current hashing first)
- **Effort:** 1 day (evaluation) + 2-3 days (implementation if approved)

---

## Milestone 3: Enterprise (v3.0+)

*Goal: Enterprise-grade features for large-scale deployments. Multi-tenancy, tiered storage, advanced analytics.*

### TODO-041: Multi-Tenancy
- **Priority:** P1 (enterprise requirement)
- **Complexity:** Large
- **Summary:** Per-tenant isolation, quotas, billing, tenant-aware partitioning
- **Context:** [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Depends on:** TODO-060 ✅ (RequestContext.tenant_id)
- **Effort:** 4-6 weeks

### TODO-043: S3 Bottomless Storage
- **Priority:** P2
- **Complexity:** Very Large
- **Summary:** Append-only log in S3/R2/GCS. Immutable log segments, replay on startup, Merkle checkpoints
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7
- **Crates:** aws-sdk-s3, opendal
- **Depends on:** TODO-033 (AsyncStorageWrapper)
- **Effort:** 6-8 weeks

### TODO-040: Tiered Storage (Hot/Cold)
- **Priority:** P2
- **Complexity:** Large
- **Summary:** Hot data in memory, cold data in S3/cheap storage with transparent migration. RecordStore metadata (hit_count, last_access_time) enables eviction policies.
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Depends on:** TODO-033 (AsyncStorageWrapper), TODO-043 (S3)
- **Effort:** 4-6 weeks

### TODO-039: Vector Search
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Semantic vector search with local embeddings, HNSW index, tri-hybrid search (Exact + BM25 + Semantic)
- **Context:** [PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md)
- **Crate:** usearch (Rust bindings)
- **Effort:** 4 weeks

### TODO-044: Bi-Temporal Queries (Time-Travel)
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Query historical state with valid time + transaction time
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 8
- **Depends on:** TODO-043 (S3 Bottomless Storage)
- **Effort:** 4-6 weeks

---

## Execution Order (milestone-driven)

Items within the same wave can run in parallel. Each wave starts when its blockers from previous waves complete.

### Milestone 1 — v1.0 (Working IMDG)

| Wave | Items | Blocked by | Status |
|------|-------|------------|--------|
| **1-4** | Phases 0-2.5, Phase 3 framework | — | ✅ All complete |
| **5a** | ~~TODO-084~~ ✅ (Coordination) | 065+066 | Done |
| **5b** | ~~TODO-085~~ ✅ (CRDT) · ~~TODO-087~~ ✅ (Messaging) | 084 · 084 | Done |
| **5c** | ~~TODO-086~~ ✅ (Sync) · TODO-088 (Query) · TODO-089 (Persistence) | 085 · 085 · 085 | 086 done |
| **5d** | TODO-090 (PostgreSQL) · TODO-071 (Search) | 067 · — | Parallel |
| **5e** | TODO-074 (HLC fix) · TODO-075 (ORMap hash fix) | — · — | Trivial, parallel |
| **5f** | TODO-068 (Integration Tests) | 084+ incremental | Gates v1.0 release |

### Milestone 2 — v2.0 (Data Platform)

| Wave | Items | Blocked by |
|------|-------|------------|
| **6a** | TODO-069 (Schema) · TODO-091 (DataFusion SQL) | — · 088 |
| **6b** | TODO-070 (Shapes) · TODO-025 (DAG Executor) | 069 · 091 |
| **6c** | TODO-092 (Connectors) · TODO-033 (Write-Behind) · TODO-036 (Extensions) | 025 · 090 · — |
| **6d** | TODO-072 (WASM) · TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) | 091+071 · — · — · 075 |

### Milestone 3 — v3.0+ (Enterprise)

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) | — · 033 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |

**Current position:** Wave 5c — TODO-088 (QueryService) + TODO-089 (PersistenceService) next. 4 of 7 domain services done. v1.0 critical path: 088/089 → 090/071 → 074/075 → 068.

---

## Dependency Graph

```
Phases 0-2.5 [ALL DONE]
  TypeScript completion → Bridge → Rust Core → Research Sprint
              ↓
Phase 3 Framework [ALL DONE]
  TODO-064 (Network) · TODO-067 (Storage) · TODO-065 (Routing) · TODO-066 (Cluster)
              ↓
═══════════════════════════════════════════════════════════════
MILESTONE 1: Working IMDG (v1.0)
═══════════════════════════════════════════════════════════════

Phase 3b (Domain Services):
  TODO-084 ✅ (Coordination)
       │
       ├──→ TODO-085 ✅ (CRDT) ──→ TODO-086 ✅ (Sync)
       │         │
       │         ├──→ TODO-088 (Query, PredicateEngine L1-L3)
       │         └──→ TODO-089 (Persistence, Counters)
       │
       └──→ TODO-087 ✅ (Messaging)

  TODO-090 (PostgreSQL adapter) ← parallel, depends on 067 only
  TODO-071 (Tantivy Search) ← parallel, no deps
  TODO-074 · TODO-075 (bug fixes) ← parallel, trivial

  TODO-068 (Integration Tests) ← gates v1.0, incremental from 084+

═══════════════════════════════════════════════════════════════
MILESTONE 2: Data Platform (v2.0)
═══════════════════════════════════════════════════════════════

  TODO-069 (Schema) ──→ TODO-070 (Shapes)

  TODO-091 (DataFusion SQL) ──→ TODO-025 (DAG Stream Processing)
       │                              │
       └──→ TODO-072 (WASM)          └──→ TODO-092 (Connectors)

  TODO-033 (Write-Behind) ← depends on 090
  TODO-036 (Extensions)
  TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt)

═══════════════════════════════════════════════════════════════
MILESTONE 3: Enterprise (v3.0+)
═══════════════════════════════════════════════════════════════

  TODO-041 (Multi-Tenancy)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
  TODO-039 (Vector Search)
```

---

## Timeline Summary

| Milestone | Remaining Items | Effort | Status |
|-----------|----------------|--------|--------|
| **v1.0 Working IMDG** | 087, 088, 089, 090, 071, 074, 075, 068 | ~6-8 weeks | **In progress** (3/7 services done) |
| **v2.0 Data Platform** | 069, 070, 091, 025, 092, 033, 036, 072, 048, 049, 076 | ~12-16 weeks | After v1.0 |
| **v3.0 Enterprise** | 041, 043, 040, 039, 044 | ~16-24 weeks | After v2.0 |

## Eliminated Items

| TODO | Reason | Date |
|------|--------|------|
| TODO-042 (DBSP) | Not needed; StandingQueryRegistry + ReverseQueryIndex sufficient | 2026-02-10 |
| TODO-034 (Rust/WASM hot paths) | Superseded by full Rust migration | 2026-02-10 |
| TODO-045 (DST Documentation) | TS testing infrastructure documentation for deprecated system | 2026-02-18 |

## Completed Items (archived)

### Phase 3 Rust Server Items

| TODO | Spec | Completed |
|------|------|-----------|
| TODO-064 → SPEC-057a-c | Networking Layer: axum HTTP/WS, ConnectionHandle, Tower middleware, graceful shutdown | 2026-02-21 |
| TODO-067 → SPEC-058a-c | Multi-Layer Storage: 3-layer traits, HashMapStorage, DefaultRecordStore, MutationObserver | 2026-02-21 |
| TODO-065 → SPEC-059 | Operation Routing: ServiceRegistry, Operation enum, OperationRouter, Tower pipeline, domain stubs | 2026-02-22 |
| TODO-066 → SPEC-060a-e | Cluster Protocol: 5 traits, 18-msg wire protocol, phi-accrual FD, partition assignment, migration coordinator, 4 resilience processors | 2026-02-24 |
| TODO-063 → SPEC-056 | Basic Partition Hash: FNV-1a % 271, PartitionTable (subsumed by TODO-066) | 2026-02-24 |
| TODO-084 → SPEC-061 | CoordinationService: Ping/Pong, PartitionMap, HeartbeatAck | 2026-02-24 |
| TODO-085 → SPEC-062 | CrdtService: LWW/OR-Map write path, ClientOp/OpBatch, ServerEvent broadcast | 2026-02-25 |
| TODO-086 → SPEC-063 | SyncService: Merkle delta sync, MerkleSyncManager, MutationObserver integration | 2026-02-25 |
| TODO-087 → SPEC-064 | MessagingService: Topic pub/sub, fan-out broadcast, topic lifecycle | 2026-02-25 |

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
| TODO-083 | Output: `.specflow/reference/RUST_NETWORKING_PATTERNS.md` |
| TODO-090 | TS ref: `packages/server/src/storage/PostgreSQLAdapter.ts` |
| TODO-091 | Arroyo: `arroyo-planner/builder.rs`, `arroyo-datastream/logical.rs`; ArkFlow: `arkflow-plugin/src/processor/sql.rs` |
| TODO-092 | Arroyo: `arroyo-connector/src/`; ArkFlow: `arkflow-core/src/input/`, `arkflow-core/src/codec/` |

---

*Restructured 2026-02-12: Replaced wave-based organization with phase-based Rust migration roadmap. Added TODO-059 through TODO-072 for Rust-specific work. Product positioning decisions (schema, shapes, WASM) integrated as concrete TODOs.*
*Updated 2026-02-15: Added TODO-074 through TODO-078 from external audit analysis. HLC validation, ORMap hash determinism bug, TS hash inconsistency, protocol drift CI, MsgPack hash evaluation.*
*Updated 2026-02-18: Strategic audit. Added Phase 2.5 (Architecture Research Sprint: TODO-080, 081, 082). Redesigned Phase 3 items (TODO-063/065/066/067) from "TS port" to "Hazelcast-informed design". Upgraded TODO-078 to P1 (client-server hash compatibility confirmed). Deferred TODO-076 to Phase 4. Eliminated TODO-045. Marked completed Phase 2 items.*
*Updated 2026-02-19: Added Triple Reference Protocol. Rust OSS projects (TiKV, Quickwit, Databend) added as implementation pattern references alongside Hazelcast conceptual architecture. Research tasks TODO-080/081/082 updated with concrete Rust file paths. Rationale: Java→Rust translation has real friction (ownership, no inheritance, no GC) — Rust-native patterns needed for storage traits (TiKV engine_traits), cluster concurrency (TiKV FSM+DashMap), service composition (Quickwit actors+Tower), object storage (Databend OpenDAL).*
*Updated 2026-02-20: Added TODO-083 (Networking Layer Research — light, 1-2 days). Focus: SurrealDB, Grafbase, Quickwit for connection abstraction, backpressure, middleware ordering. TODO-064 now depends on 083. Existing SPEC-057 to be recreated after research.*
*Updated 2026-02-22: Marked TODO-064, TODO-067, TODO-065 as DONE. Waves 3-4 progress: 064 (SPEC-057a-c), 067 (SPEC-058a-c), 065 (SPEC-059) all archived.*
*Updated 2026-02-24: Marked TODO-066 as DONE (SPEC-060a-e all archived, 288 server tests). Phase 3 FRAMEWORK complete. Added Phase 3b (Domain Service Implementations): TODO-084 (Coordination), TODO-085 (CRDT), TODO-086 (Sync), TODO-087 (Messaging), TODO-088 (Query), TODO-089 (Persistence). Updated TODO-068 to incremental approach starting from TODO-084. Execution order: 084 → 085 → 086/087/088/089 (parallel) → 068 (incremental throughout).*
*Updated 2026-02-25: Major restructuring — milestone-driven roadmap. Replaced Phase 4/5 with Milestone 1 (v1.0 Working IMDG), Milestone 2 (v2.0 Data Platform), Milestone 3 (v3.0+ Enterprise). Added TODO-090 (PostgreSQL MapDataStore), TODO-091 (DataFusion SQL), TODO-092 (Connector Framework). Marked TODO-063, TODO-086 as DONE. Moved TODO-048/049/076 from Phase 4 to Milestone 2 (v2.0). Added Arroyo + ArkFlow to Triple Reference Protocol. Updated dependency graph, execution order, timeline. Product vision: TopGun = Hazelcast (IMDG) + DataFusion (SQL) + Arroyo-informed streaming + offline-first clients.*
