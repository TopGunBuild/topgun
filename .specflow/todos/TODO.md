# TopGun Roadmap

**Last updated:** 2026-02-28
**Strategy:** Rust-first IMDG design informed by Hazelcast architecture, not a TypeScript port
**Product vision:** "The unified real-time data platform — from browser to cluster to cloud storage"
**v1.0 positioning:** "The reactive data grid that extends the cluster into the browser" ([PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md))

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
| TODO-093 Admin Dashboard | `apps/admin-dashboard/` | Management Center (commercial) | **Arroyo**: OpenAPI codegen, ReactFlow+Dagre DAG viz, SWR data fetching, live metric coloring |

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

### TODO-074: HLC Node ID Colon Validation (TS + Rust) — DONE
- **Status:** Complete (discovered already implemented during /sf:plan)
- **Summary:** HLC constructor rejects node IDs containing `:` in both TS (`HLC.ts` throw) and Rust (`hlc.rs` assert). 2 TS + 4 Rust test cases.

### TODO-075: Fix Rust ORMap Merkle Hash Determinism → SPEC-055 — DONE
- **Status:** Complete (SPEC-055 completed 2026-02-19)
- **Summary:** Replaced `serde_json::to_string()` with `canonical_json()` (recursive key sorting) in `hash_entry()`. 3 unit tests + 1 integration test. Zero deviations.

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

### TODO-088: QueryService (Live Queries) → SPEC-065 — DONE
- **Status:** Complete (SPEC-065 completed 2026-02-26)
- **Summary:** Live query subscriptions. Client subscribes to a query, server evaluates it against current data, then pushes incremental updates as data changes. Fifth domain service replacing stub.
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

### TODO-089: PersistenceService (Counters + Entry Processing) → SPEC-066 — DONE
- **Status:** Complete (SPEC-066 completed 2026-02-27)
- **Summary:** Server-side counters, entry processing, journal subscriptions, resolver requests. Sixth domain service replacing stub.

### TODO-068: Integration Test Suite — DONE
- **Status:** Complete (2026-03-03, SPEC-073a-e + SPEC-074 + SPEC-075)
- **Summary:** 50 TS→Rust integration tests across 6 suites (connection-auth, crdt-lww, crdt-ormap, pubsub, queries, search) — all passing. 502 Rust unit tests — all passing. Single-node behavioral equivalence proven.

---

## Milestone 1: Working IMDG (v1.0)

*Goal: TopGun Rust server replaces TS server. Clients connect, write/read data, sync offline, search, query. PostgreSQL persistence for durability. Production-usable.*

### TODO-090: PostgreSQL MapDataStore Adapter → SPEC-067 — DONE
- **Status:** Complete (SPEC-067 completed 2026-02-27)
- **Summary:** Write-through MapDataStore adapter via sqlx PgPool, BYTEA+MsgPack, feature-gated `postgres`.

### TODO-071: SearchService (Tantivy) → SPEC-068 — DONE
- **Status:** Complete (SPEC-068 completed 2026-02-28)
- **Summary:** Full-text search via tantivy: tokenization, fuzzy matching, phrase queries. Seventh and final domain service — `domain_stub!` macro fully removed.

### TODO-074: HLC Node ID Colon Validation (TS + Rust) — DONE
- **Status:** Complete (discovered already implemented during /sf:plan)
- **Summary:** HLC constructor rejects node IDs containing `:` in both TS and Rust. 6 test cases total.

### TODO-075: Fix Rust ORMap Merkle Hash Determinism → SPEC-055 — DONE
- **Status:** Complete (SPEC-055 completed 2026-02-19)
- **Summary:** Replaced `serde_json::to_string()` with `canonical_json()` (recursive key sorting) in `hash_entry()`. 3 unit tests + 1 integration test.

### TODO-094: Change LICENSE to Apache 2.0 → SPEC-069 — DONE
- **Status:** Complete (SPEC-069 completed 2026-02-28)
- **Summary:** Switched project license from BSL-1.1 to Apache-2.0. 13 files modified (LICENSE, NOTICE, Cargo.toml, root + 7 package.json files, CONTRIBUTING.md, mcp-server README).

### TODO-097: Server-Side Write Validation + HLC Sanitization → SPEC-071 — DONE
- **Status:** Complete (SPEC-071 completed 2026-02-28)
- **Summary:** WriteValidator intercepts client writes before CRDT merge; auth check, map ACL check, value size check, HLC sanitization. 1 file created, 6 modified.

### TODO-099: Structured Tracing + Metrics Endpoint → SPEC-072 — DONE
- **Status:** Complete (SPEC-072 completed 2026-03-01)
- **Summary:** `tracing-subscriber` with `RUST_LOG` filtering, `metrics-exporter-prometheus` recorder, `/metrics` endpoint with Prometheus text format v0.0.4, operation counters/histograms/error counters, connection gauge, `info_span!` + `.instrument()` on all 7 domain services.

### TODO-096: Adoption Path Documentation + Security Model → SPEC-077 — DONE
- **Status:** Complete (SPEC-077 completed 2026-03-05)
- **Summary:** PRODUCT_CAPABILITIES.md already had Adoption Path (3 tiers) and Security Model sections. SPEC-077 delivered the missing pieces: Tier 1 example app (`examples/collaborative-tasks/`), PostgresDataStore integration guide, JWT auth integration guide. Competitive comparison table already had "Works alongside existing DB: Yes".

### TODO-104: Fix Demo App + Blog Issues → SPEC-070 — DONE
- **Status:** Complete (SPEC-070 completed 2026-02-28)
- **Summary:** Fixed Vite aliases, removed hardcoded JWT, translated Russian strings to English, removed incorrect Mongo reference, added RBAC GuideCard, deleted misleading serverless blog post. 1 file created, 5 modified, 1 deleted.

### TODO-105: Sync Showcase Demo App — NEW
- **Priority:** P1 (marketing — "Show, Don't Tell")
- **Complexity:** Medium
- **Summary:** Neither existing demo shows TopGun's core differentiator (offline → reconnect → automatic conflict resolution). Create a "Collaborative Board" demo that makes the value proposition instantly visible.
- **Scope:**
  - New example: collaborative board (Kanban or shared list)
  - **Zero external deps** — no Clerk, no R2, no push notifications. `pnpm install && pnpm dev` starts in 30 seconds
  - Visual sync status indicator (online / offline / pending ops count)
  - "Simulate Offline" button — disconnects WebSocket, shows pending queue
  - Multi-tab awareness: banner prompting to open another tab for live sync
  - Split-screen conflict demo: two "devices" editing same data while "offline", click "Reconnect" to see automatic LWW merge with visual highlight of which version won
  - Designed to be embeddable as iframe on docs homepage (replacing mock TacticalDemo)
  - `?demo` URL param: shows performance badge (load time, read latency) for marketing videos/screenshots. Inspired by RociCorp/zbugs approach — see [ZERO_ANALYSIS.md](../reference/ZERO_ANALYSIS.md)
- **Depends on:** Working Rust server (all domain services done) or can use TS server for initial version
- **Effort:** 1 week

### TODO-093: Admin Dashboard — v1.0 DONE, v2.0/v3.0 deferred
- **Priority:** P1 (v1.0 DONE; v2.0/v3.0 deferred to Milestones 2/3)
- **Complexity:** Medium (foundation exists, needs Rust server adaptation)
- **Status (v1.0):** Complete (SPEC-076a/b/c — 2026-03-04)
- **Summary:** Adapt existing React admin dashboard (`apps/admin-dashboard/`) to work with Rust server. Phased: v1.0 foundation → v2.0 data platform features → v3.0 enterprise views.
- **Existing state:** ~85% functional React 19 + Vite app with: cluster topology (SVG partition ring), data explorer (CRUD + Monaco JSON editor), query playground, settings management, JWT auth, command palette (Cmd+K), dark mode.
- **Scope (v1.0 — Rust Server Adaptation) — DONE:**
  - ~~Rust admin API: expose OpenAPI spec via `utoipa` crate on axum endpoints~~ ✅ SPEC-076a/b
  - ~~Admin API endpoints on Rust server (status, login, cluster/status, maps, settings GET/PUT)~~ ✅ SPEC-076b
  - ~~SWR migration: replace manual fetch+useState with SWR for live metrics~~ ✅ SPEC-076c
  - ~~CRDT Debug panel in Data Explorer (placeholder UI, system maps not populated)~~ ✅ SPEC-076c
  - ~~Dashboard adaptation: token unification, Settings restructuring, Setup Wizard hidden~~ ✅ SPEC-076c
  - **Deferred from v1.0:** OpenAPI codegen (hand-typed interfaces instead), system maps population ($sys/*), bootstrap/setup wizard
- **Scope (v2.0 — Data Platform Features):**
  - **Pipeline visualization:** ReactFlow + Dagre for DAG stream processing graphs
  - **Live metric coloring:** node/partition coloring by backpressure/load thresholds (Arroyo pattern)
  - **SQL playground upgrade:** DataFusion query validation in real-time (POST `/validate_query`)
  - **Connector wizard:** multi-step creation flow (choose → configure → test → schema → confirm)
  - **Schema browser:** view registered schemas, validation status
- **Scope (v3.0 — Enterprise Features):**
  - **Multi-tenant admin:** tenant list, per-tenant quotas/usage, tenant isolation controls
  - **Tiered storage monitor:** hot/warm/cold distribution visualization, migration status
  - **Vector search admin:** index status, embedding model config
- **Existing source:** `apps/admin-dashboard/`
- **Rust Reference:** **Arroyo** WebUI (`/Users/koristuvac/Projects/rust/arroyo/webui/`):
  - OpenAPI codegen: `openapi-typescript` CLI → `/gen/api-types.ts` → type-safe `openapi-fetch`
  - SWR: keyed fetchers with `refreshInterval`, `useSWRInfinite` for pagination
  - ReactFlow + Dagre: auto-layout DAG, click-to-detail sidebar, backpressure coloring
  - SSE streaming: `text/event-stream` for live connection testing
  - Multi-step wizard: ConnectionProfile creation (5 steps)
- **HC Reference:** Hazelcast Management Center (commercial) — conceptual feature parity
- **Depends on:** TODO-064 ✅ (Network — axum endpoints), TODO-090 (admin needs working storage for system maps)
- **Effort:** v1.0: 1-2 weeks · v2.0: 2-3 weeks · v3.0: 1-2 weeks

### TODO-068: Integration Test Suite — DONE
- **Status:** Complete (2026-03-03, SPEC-073a-e + SPEC-074 + SPEC-075)
- **Summary:** 50 TS→Rust integration tests across 6 suites — all passing. 502 Rust unit tests — all passing. Single-node behavioral equivalence proven. TS e2e tests (tests/e2e/) removed — they tested TS server internals, replaced by integration-rust suite.

### TODO-106: Update Documentation Content for Rust Server — NEW
- **Priority:** P2 (post-migration docs sync)
- **Complexity:** Medium
- **Summary:** Update docs site content for Rust server. Finalize after API surface is stable.
- **Scope:**
  - Update `reference/server.mdx` for Rust server API
  - Update `reference/cli.mdx` for new CLI commands
  - Update `reference/protocol.mdx` if wire format changed
  - Add Security Model section (from TODO-096 content)
  - Add Adoption Path section (from TODO-096 content)
  - Update `comparison.mdx`: add Replicache/Zero, Cloudflare Durable Objects
  - Verify all code snippets work with current SDK
- **Depends on:** TODO-096 (Adoption Path + Security docs), TODO-068 (API surface finalized)
- **Effort:** 1 week

### TODO-103: Remove Legacy TS Server Code — NEW
- **Priority:** P2 (cleanup — final v1.0 step)
- **Complexity:** Low
- **Summary:** Remove replaced TypeScript server packages after integration tests prove behavioral equivalence.
- **Scope:**
  - Remove `packages/server/` entirely (replaced by `packages/server-rust/`)
  - Remove `packages/native/` entirely (xxHash64 native addon — Rust handles hashing)
  - Evaluate `packages/mcp-server/` — keep if it wraps TS client, not TS server
  - Update pnpm workspace config
  - Update CI workflows (remove TS server test jobs)
  - Update CLAUDE.md package hierarchy
  - Update `examples/` to use Rust server
- **Keep:** `packages/core/`, `packages/client/`, `packages/react/`, `packages/adapters/`, `packages/adapter-better-auth/`
- **Depends on:** TODO-068 (Integration Tests — proves behavioral equivalence)
- **Effort:** 1-2 days

### TODO-107: Standardize JWT to RFC 7519 `sub` Claim Only — DONE
- **Status:** Complete (2026-03-02)
- **Summary:** Removed legacy `userId` field from JWT protocol. Rust `JwtClaims` now uses only standard `sub` claim (RFC 7519). All token generators (Rust auth, TS e2e helpers, TS server bootstrap, TS test files) updated to send only `sub`. No backward compatibility needed — no active clients.

### TODO-108: Investigate and Fix Pre-Existing Rust Integration Test Failures — DONE
- **Status:** Complete (2026-03-03, SPEC-074)
- **Summary:** Fixed 3 bugs: ephemeral RecordStore (DashMap cache), partition mismatch (multi-partition scan), AUTH_FAIL race (Close frame removal). 13 tests fixed.

### TODO-109: Wire QueryObserverFactory for Live Query Updates — DONE
- **Status:** Complete (2026-03-03, SPEC-075)
- **Summary:** Created QueryObserverFactory in test_server.rs, wired alongside SearchObserverFactory. 6 live query tests fixed. All 50/50 integration tests now pass.

### TODO-027: Deterministic Simulation Testing (DST) — NEW
- **Priority:** P2 (testing infrastructure — after v0.12.0)
- **Complexity:** Medium
- **Summary:** Deterministic simulation testing for distributed protocol correctness. Seeded RNG, virtual time, simulated network — reproduce any distributed bug by seed.
- **Scope:**
  - Integrate `madsim` crate — patches tokio for deterministic scheduling, virtual clock, simulated network
  - Virtual network layer: inject partitions, latency, packet loss between cluster nodes
  - Property-based invariant checking: CRDT convergence, Merkle sync correctness, cluster rebalancing
  - Seed-based regression: failing seed → reproducible test case
  - Scenarios: network partition during sync, node crash mid-replication, clock drift between nodes
- **Ref:** RisingWave (`/Users/koristuvac/Projects/rust/risingwave`) — production use of madsim for streaming SQL, `ci-sim` build profile; [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 2 (DST concept)
- **Depends on:** v0.12.0 release (stable cluster protocol)
- **Effort:** ~1 week (madsim integration, not from scratch)

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
- **Ref:** Arroyo connector traits, ArkFlow Input/Output/Codec pattern, RisingWave (`/Users/koristuvac/Projects/rust/risingwave/src/connector/`) — 25+ sources, 30+ sinks, pluggable codec layer (Avro/JSON/Protobuf)
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

### TODO-101: Client DevTools — NEW
- **Priority:** P2 (DX differentiator)
- **Complexity:** Medium-Large
- **Summary:** Browser DevTools panel (Chrome extension) or in-app debug overlay showing local replica state, pending sync queue, HLC timeline, connection status, CRDT merge history. Competitive with Ditto/PowerSync/ElectricSQL developer tools.
- **Scope:**
  - Chrome DevTools panel or standalone debug overlay
  - Local replica state viewer (per-map entries, sizes, last update)
  - Pending sync queue inspector (pending ops, retry state)
  - HLC timeline visualization
  - Connection status (online/offline/reconnecting)
  - CRDT merge visualization (what merged, which timestamp won, conflict count)
- **Depends on:** Client SDK stabilization
- **Effort:** 4-6 weeks

### TODO-102: Rust CLI (clap) — NEW
- **Priority:** P3 (not blocking)
- **Complexity:** Medium
- **Summary:** Rewrite CLI from JavaScript (commander) to Rust (clap). Merge with server binary for single entry point: `topgun serve`, `topgun status`, `topgun debug crdt`, `topgun sql`.
- **Scope:**
  - Rewrite CLI in Rust using clap
  - Merge with server binary: single `topgun` binary
  - Drop Node.js dependency for server operation
  - Keep JS CLI tools for TS client development
- **Depends on:** v1.0 completion (CLI shape depends on final server API)
- **Effort:** 1-2 weeks

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

### TODO-095: Enterprise Directory Structure — NEW
- **Priority:** P3 (not needed until v3.0 crates exist)
- **Complexity:** Low
- **Summary:** Create `enterprise/` directory with BSL 1.1 LICENSE file. Move v3.0 crates (multi-tenancy, tiered-storage, vector-search, bi-temporal) there when implemented. Feature-gated: `[features] enterprise = [...]` in topgun-server Cargo.toml.
- **Depends on:** First enterprise crate implementation
- **Effort:** 1 day

---

## Release & Merge Strategy

**Current state:** Branch `rust-migration` is 66 commits ahead of `main` (v0.11.0). `main` has 0 commits not in `rust-migration`.

**Versioning policy:** Continue v0.x until first production deployment + 3 months API stability. v1.0.0 is a trust signal, not a feature milestone — premature v1.0 without real users creates SemVer debt (every breaking fix burns a major version).

### Git Branch Strategy

| Event | Trigger | Git Action |
|-------|---------|------------|
| **Merge to main** | After TODO-068 (Integration Tests) passes — proves TS client ↔ Rust server equivalence | `git merge rust-migration` into `main` |
| **TS server deprecation** | Same merge — add `@deprecated` to `packages/server/package.json`, console warning on import | Part of the merge PR |
| **TS server removal** | TODO-103 — after 1 release cycle of deprecation notice | Remove `packages/server/`, `packages/native/` |

### npm Release Plan

| Version | Trigger | Packages | Notes |
|---------|---------|----------|-------|
| **v0.12.0-rc.1** | After TODO-097 (Security) + TODO-099 (Tracing) + TODO-068 first pass | `@topgunbuild/core`, `@topgunbuild/client`, `@topgunbuild/react`, `@topgunbuild/adapters` | Client packages unchanged; Rust server binary separate |
| **v0.12.0** | After TODO-068 complete + merge to `main` | All client packages | First release with Rust server |
| **topgun-server** (cargo/binary) | Same as v0.12.0 | GitHub Release with prebuilt binaries (linux-x64, darwin-arm64, darwin-x64) | `cargo install topgun-server` or download from Releases |
| **v0.13.0** | After TODO-093 (Admin Dashboard) + TODO-105 (Sync Showcase) | Client packages + Admin Dashboard | Polish release |
| **v0.14.0+** | Iterations based on first-user feedback | As needed | API stabilization |
| **v1.0.0** | ≥1 production deployment + 3 months stable API | All packages | Commitment to backward compatibility |

### GitHub Release Plan

| Release | Contents | Tag |
|---------|----------|-----|
| **v0.12.0-rc.1** | npm packages + Rust server binary (pre-release) | `v0.12.0-rc.1` |
| **v0.12.0** | npm packages + Rust server binary + migration guide + changelog | `v0.12.0` |
| **v1.0.0** | Stability milestone — after production validation | `v1.0.0` |

### Deprecation Timeline

1. **v0.12.0-rc.1:** `packages/server/` marked `@deprecated` in package.json, console.warn on first import
2. **v0.12.0:** Deprecation notice in npm (`npm deprecate @topgunbuild/server "Use topgun-server binary instead"`)
3. **v0.13.0:** `packages/server/` and `packages/native/` removed (TODO-103)

---

## Execution Order (milestone-driven)

Items within the same wave can run in parallel. Each wave starts when its blockers from previous waves complete.

### Milestone 1 — v1.0 (Working IMDG)

| Wave | Items | Blocked by | Status |
|------|-------|------------|--------|
| **1-4** | Phases 0-2.5, Phase 3 framework | — | ✅ All complete |
| **5a** | ~~TODO-084~~ ✅ · ~~TODO-085~~ ✅ · ~~TODO-086~~ ✅ · ~~TODO-087~~ ✅ · ~~TODO-088~~ ✅ · ~~TODO-089~~ ✅ · ~~TODO-090~~ ✅ · ~~TODO-071~~ ✅ | — | ✅ All 7 services + PostgreSQL + Search done |
| **5b** | ~~TODO-074~~ ✅ · ~~TODO-075~~ ✅ · ~~TODO-094~~ ✅ (LICENSE) · ~~TODO-104~~ ✅ (Fix demos/blog) | — | ✅ All done |
| **5c** | ~~TODO-097~~ ✅ (Security: HLC sanitize + ACL) · ~~TODO-099~~ ✅ (Tracing + /metrics) | 085 | ✅ All done |
| **5d** | ~~TODO-068~~ ✅ (Integration Tests) · ~~TODO-093 v1.0~~ ✅ (Admin Dashboard) · ~~TODO-096~~ ✅ (Adoption Path + Security docs → SPEC-077) · TODO-105 (Sync Showcase Demo) | All services · 097 · 097 · — | 068 + 093 v1.0 + 096 done; 105 remaining |
| — | **v0.12.0-rc.1** — npm pre-release + Rust server binary | 068 first pass | 🏷️ Tag + GitHub Release |
| **5e** | TODO-106 (Update Docs) · TODO-103 (Remove Legacy TS) | 068 · 068 | Final cleanup |
| — | **Merge `rust-migration` → `main`** · Deprecate TS server | 068 complete | 🔀 Merge |
| — | **v0.12.0** — first Rust server release | merge + 106 + 103 | 🏷️ Tag + GitHub Release + npm publish |

### Milestone 2 — v2.0 (Data Platform)

| Wave | Items | Blocked by |
|------|-------|------------|
| **6a** | TODO-069 (Schema) · TODO-091 (DataFusion SQL) | — · 088 |
| **6b** | TODO-070 (Shapes) · TODO-025 (DAG Executor) | 069 · 091 |
| **6c** | TODO-092 (Connectors) · TODO-033 (Write-Behind) · TODO-036 (Extensions) | 025 · 090 · — |
| **6d** | TODO-072 (WASM) · TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) | 091+071 · — · — · 075 |
| **6e** | TODO-093 v2.0 (Pipeline viz, SQL playground, connector wizard) · TODO-101 (Client DevTools) · TODO-102 (Rust CLI) | 025+091+092 · — · — |

### Milestone 3 — v3.0+ (Enterprise)

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) | — · 033 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |
| **7d** | TODO-095 (Enterprise dir structure) · TODO-093 v3.0 (Tenant admin, tiered storage monitor) | — · 041+040 |

**Current position:** Wave 5d — 068 ✅ + 093 v1.0 ✅ + 096 ✅ DONE. Remaining: 105 (Sync Showcase Demo). Critical path: 105 → v0.12.0-rc.1 → 106 (Update Docs) + 103 (Legacy removal) → merge to main → v0.12.0.

---

## Dependency Graph

```
Phases 0-2.5 [ALL DONE]
  TypeScript completion → Bridge → Rust Core → Research Sprint
              ↓
Phase 3 Framework [ALL DONE]
  TODO-064 ✅ (Network) · TODO-067 ✅ (Storage) · TODO-065 ✅ (Routing) · TODO-066 ✅ (Cluster)
              ↓
Phase 3b Domain Services [ALL DONE]
  TODO-084 ✅ · 085 ✅ · 086 ✅ · 087 ✅ · 088 ✅ · 089 ✅ (all 7 services)
  TODO-090 ✅ (PostgreSQL) · TODO-071 ✅ (Search/Tantivy)

═══════════════════════════════════════════════════════════════
MILESTONE 1: Working IMDG (v1.0) — remaining work
═══════════════════════════════════════════════════════════════

  ~~TODO-074 ✅~~ · ~~TODO-075 ✅~~ (bug fixes) — DONE
  ~~TODO-094 ✅~~ (LICENSE → Apache 2.0) — DONE
  ~~TODO-104 ✅~~ (Fix demo apps) — DONE
  ~~TODO-097 ✅~~ (Security: HLC sanitize + Map ACL) — DONE
  ~~TODO-099 ✅~~ (Structured Tracing + /metrics) — DONE
  ~~TODO-068 ✅~~ (Integration Tests — 50/50 pass) — DONE
  ~~TODO-108 ✅~~ · ~~TODO-109 ✅~~ (test bug fixes) — DONE
       ↓
  ~~TODO-093 v1.0~~ ✅ (Admin Dashboard — SPEC-076a/b/c) — DONE
  ~~TODO-096~~ ✅ (Adoption Path docs → SPEC-077) — DONE
  TODO-105 (Sync Showcase Demo) ← NEXT
       ↓
  TODO-106 (Update docs for Rust server) ← after API finalized
  TODO-103 (Remove legacy TS server) ← TS e2e tests already removed

═══════════════════════════════════════════════════════════════
MILESTONE 2: Data Platform (v2.0)
═══════════════════════════════════════════════════════════════

  TODO-069 (Schema) ──→ TODO-070 (Shapes)

  TODO-091 (DataFusion SQL) ──→ TODO-025 (DAG Stream Processing)
       │                              │
       └──→ TODO-072 (WASM)          └──→ TODO-092 (Connectors)

  TODO-033 (Write-Behind) ← depends on 090 ✅
  TODO-036 (Extensions)
  TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt)
  TODO-101 (Client DevTools) · TODO-102 (Rust CLI)
  TODO-093 v2.0 (Pipeline viz, SQL playground, connector wizard) ← depends on 025+091+092

═══════════════════════════════════════════════════════════════
MILESTONE 3: Enterprise (v3.0+)
═══════════════════════════════════════════════════════════════

  TODO-095 (Enterprise dir structure — BSL LICENSE)
  TODO-041 (Multi-Tenancy)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
  TODO-039 (Vector Search)
  TODO-093 v3.0 (Tenant admin, tiered storage monitor) ← depends on 041+040
```

---

## Timeline Summary

| Milestone | Remaining Items | Effort | Status |
|-----------|----------------|--------|--------|
| **v1.0 Working IMDG** | ~~093 v1.0~~ ✅, ~~096~~ ✅, 105, 106, 103 | ~1-2 weeks | **In progress** (services + security + tracing + tests + admin + adoption docs done; demo remaining) |
| — v1.0.0-rc.1 tag | After 105 | — | Pre-release: npm + Rust binary |
| — Merge to main | After 068 ✅ complete | — | Deprecate TS server |
| — v1.0.0 stable | After merge + 106 + 103 | — | npm publish + GitHub Release |
| **v2.0 Data Platform** | 069, 070, 091, 025, 092, 033, 036, 072, 048, 049, 076, 101, 102, 093 v2.0 | ~14-18 weeks | After v1.0 |
| **v3.0 Enterprise** | 041, 043, 040, 039, 044, 095, 093 v3.0 | ~18-26 weeks | After v2.0 |

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
| TODO-088 → SPEC-065 | QueryService: Live queries, PredicateEngine, standing query re-evaluation | 2026-02-26 |
| TODO-089 → SPEC-066 | PersistenceService: Counters, entry processing, journal subscriptions, resolver | 2026-02-27 |
| TODO-090 → SPEC-067 | PostgresDataStore: write-through MapDataStore adapter, sqlx PgPool, BYTEA+MsgPack | 2026-02-27 |
| TODO-071 → SPEC-068 | SearchService: Tantivy full-text search, final domain service, `domain_stub!` removed | 2026-02-28 |
| TODO-075 → SPEC-055 | ORMap Merkle hash determinism: `canonical_json()` with recursive key sorting | 2026-02-19 |

### Milestone 1 Security + Polish Items

| TODO | Spec | Completed |
|------|------|-----------|
| TODO-094 → SPEC-069 | LICENSE: BSL-1.1 → Apache-2.0, NOTICE file, 13 files updated | 2026-02-28 |
| TODO-104 → SPEC-070 | Fix demo apps: Vite aliases, hardcoded JWT, Russian strings, Mongo refs, serverless blog | 2026-02-28 |
| TODO-097 → SPEC-071 | WriteValidator: auth check, map ACL, value size, HLC sanitization before CRDT merge | 2026-02-28 |
| TODO-099 → SPEC-072 | Structured tracing: tracing-subscriber, metrics-exporter-prometheus, /metrics endpoint | 2026-03-01 |
| TODO-096 → SPEC-077 | Adoption Path docs + Tier 1 example app (collaborative-tasks), PostgresDataStore guide, JWT auth guide | 2026-03-05 |

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
| TODO-027 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 2 (DST concept), RisingWave: madsim integration (`/Users/koristuvac/Projects/rust/risingwave`, `ci-sim` profile) |
| TODO-092 | Arroyo: `arroyo-connector/src/`; ArkFlow: `arkflow-core/src/input/`, `arkflow-core/src/codec/`; RisingWave: `/Users/koristuvac/Projects/rust/risingwave/src/connector/` (25+ sources, 30+ sinks, pluggable codecs) |
| TODO-093 | Existing: `apps/admin-dashboard/`; Arroyo WebUI: `/Users/koristuvac/Projects/rust/arroyo/webui/` (OpenAPI codegen, ReactFlow+Dagre, SWR, live metric coloring) |
| TODO-096 | [PRODUCT_CAPABILITIES.md](../reference/PRODUCT_CAPABILITIES.md), [STRATEGIC_RECOMMENDATIONS.md](../reference/STRATEGIC_RECOMMENDATIONS.md) Section 4+5 |
| TODO-097 | [STRATEGIC_RECOMMENDATIONS.md](../reference/STRATEGIC_RECOMMENDATIONS.md) Section 5 (Security Model) |
| TODO-105 | [STRATEGIC_RECOMMENDATIONS.md](../reference/STRATEGIC_RECOMMENDATIONS.md) Section 12.4 (Sync Showcase spec), [SYNC_DEMO_RECOMMENDATIONS.md](../reference/SYNC_DEMO_RECOMMENDATIONS.md) (concept + review) |
| TODO-107 | Discovered during SPEC-073e integration test debugging — JWT `serde(alias)` conflict |
| TODO-108 | Discovered during SPEC-073e integration test execution — 19 pre-existing test failures (13 fixable by SPEC-074, 6 deferred to TODO-109) |
| TODO-109 | Discovered during SPEC-074 audit v2 — QueryMutationObserver not wired in test_server.rs, 6 live query tests fail |

---

*Restructured 2026-02-12: Replaced wave-based organization with phase-based Rust migration roadmap. Added TODO-059 through TODO-072 for Rust-specific work. Product positioning decisions (schema, shapes, WASM) integrated as concrete TODOs.*
*Updated 2026-02-15: Added TODO-074 through TODO-078 from external audit analysis. HLC validation, ORMap hash determinism bug, TS hash inconsistency, protocol drift CI, MsgPack hash evaluation.*
*Updated 2026-02-18: Strategic audit. Added Phase 2.5 (Architecture Research Sprint: TODO-080, 081, 082). Redesigned Phase 3 items (TODO-063/065/066/067) from "TS port" to "Hazelcast-informed design". Upgraded TODO-078 to P1 (client-server hash compatibility confirmed). Deferred TODO-076 to Phase 4. Eliminated TODO-045. Marked completed Phase 2 items.*
*Updated 2026-02-19: Added Triple Reference Protocol. Rust OSS projects (TiKV, Quickwit, Databend) added as implementation pattern references alongside Hazelcast conceptual architecture. Research tasks TODO-080/081/082 updated with concrete Rust file paths. Rationale: Java→Rust translation has real friction (ownership, no inheritance, no GC) — Rust-native patterns needed for storage traits (TiKV engine_traits), cluster concurrency (TiKV FSM+DashMap), service composition (Quickwit actors+Tower), object storage (Databend OpenDAL).*
*Updated 2026-02-20: Added TODO-083 (Networking Layer Research — light, 1-2 days). Focus: SurrealDB, Grafbase, Quickwit for connection abstraction, backpressure, middleware ordering. TODO-064 now depends on 083. Existing SPEC-057 to be recreated after research.*
*Updated 2026-02-22: Marked TODO-064, TODO-067, TODO-065 as DONE. Waves 3-4 progress: 064 (SPEC-057a-c), 067 (SPEC-058a-c), 065 (SPEC-059) all archived.*
*Updated 2026-02-24: Marked TODO-066 as DONE (SPEC-060a-e all archived, 288 server tests). Phase 3 FRAMEWORK complete. Added Phase 3b (Domain Service Implementations): TODO-084 (Coordination), TODO-085 (CRDT), TODO-086 (Sync), TODO-087 (Messaging), TODO-088 (Query), TODO-089 (Persistence). Updated TODO-068 to incremental approach starting from TODO-084. Execution order: 084 → 085 → 086/087/088/089 (parallel) → 068 (incremental throughout).*
*Updated 2026-02-25: Major restructuring — milestone-driven roadmap. Replaced Phase 4/5 with Milestone 1 (v1.0 Working IMDG), Milestone 2 (v2.0 Data Platform), Milestone 3 (v3.0+ Enterprise). Added TODO-090 (PostgreSQL MapDataStore), TODO-091 (DataFusion SQL), TODO-092 (Connector Framework). Marked TODO-063, TODO-086 as DONE. Moved TODO-048/049/076 from Phase 4 to Milestone 2 (v2.0). Added Arroyo + ArkFlow to Triple Reference Protocol. Updated dependency graph, execution order, timeline. Product vision: TopGun = Hazelcast (IMDG) + DataFusion (SQL) + Arroyo-informed streaming + offline-first clients.*
*Updated 2026-02-27: Marked TODO-088 as DONE (SPEC-065 completed, 419 server tests). 5 of 7 domain services complete. Created PRODUCT_CAPABILITIES.md — end-product capabilities document covering v1.0/v2.0/v3.0 feature sets, competitive comparison, positioning.*
*Updated 2026-02-27: Added TODO-093 (Admin Dashboard) — phased across v1.0/v2.0/v3.0. v1.0: Rust admin API (utoipa OpenAPI), OpenAPI codegen, SWR migration. v2.0: ReactFlow+Dagre pipeline viz, DataFusion SQL playground, connector wizard. v3.0: multi-tenant admin, tiered storage monitor. Reference: Arroyo WebUI patterns. Updated execution order (waves 5f, 6e, 7d), dependency graph, timeline.*
*Updated 2026-02-28: Strategic audit applied (STRATEGIC_RECOMMENDATIONS.md). Marked TODO-089 (SPEC-066), TODO-071 (SPEC-068), TODO-090 (SPEC-067) as DONE — ALL 7 domain services complete, `domain_stub!` removed, PostgreSQL done. Added 12 new TODOs from strategic audit: TODO-094 (Apache 2.0 LICENSE), TODO-095 (Enterprise dir v3.0), TODO-096 (Adoption Path + Security docs), TODO-097 (P0: HLC sanitization + Map ACL — blocks production), TODO-099 (Structured tracing + /metrics), TODO-101 (Client DevTools v2.0), TODO-102 (Rust CLI v2.0), TODO-103 (Remove legacy TS), TODO-104 (Fix demo apps), TODO-105 (Sync Showcase Demo), TODO-106 (Update docs for Rust). Merged TODO-098 into TODO-096, TODO-100 into TODO-093. Updated positioning to dual-level (vision + v1.0). Rewrote execution order: security (097) on critical path, then admin/docs/demo, then integration tests gate release. v1.0 effort unchanged (~6-8 weeks) but focus shifts from domain services (done) to security + polish.*
*Updated 2026-03-02: Added TODO-107 (JWT Claims Dual-Field Deserialization — P1, protocol compat fix in auth.rs) and TODO-108 (Pre-existing integration test failures — P1, 20 tests in 3 suites). Both discovered during SPEC-073e integration test execution. Quick fix applied in test-client.ts for TODO-107; fundamental Rust-side fix still needed. TODO-108 blocks TODO-068 completion.*
*Updated 2026-03-02: Added TODO-109 (Wire QueryObserverFactory — P1, 6 live query tests). Split from TODO-108 after SPEC-074 audit v2 discovered QueryMutationObserver only exists in #[cfg(test)] blocks, not wired in test_server.rs. SPEC-074 scoped to Bugs 1-3 (13 tests), TODO-109 handles Bug 4 (6 live update tests).*
*Updated 2026-03-03: Marked TODO-068, TODO-108, TODO-109 as DONE. Integration tests: 50/50 pass (6 suites), Rust unit tests: 502 pass. Single-node behavioral equivalence proven. Removed obsolete TS e2e tests (tests/e2e/) — they depended on TS ServerCoordinator and are replaced by tests/integration-rust/. TODO-103 (TS server removal) updated accordingly.*
