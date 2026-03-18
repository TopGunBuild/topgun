---
id: SPEC-059
type: feature
status: done
priority: P0
complexity: large
created: 2026-02-21
depends_on: [SPEC-055, SPEC-058]
todo_ref: TODO-065
---

# Operation Routing and Execution Framework

## Context

The Rust server currently has a network layer (SPEC-057a-c) and a multi-layer storage system (SPEC-058a-c), but no mechanism to route incoming client messages to domain-specific business logic. The TypeScript server uses 26 stateless handler classes registered via a `MessageRouter`, but this design has known flaws: `BatchProcessingHandler.processBatchAsync` nests inter-node forwarded messages incorrectly, `PartitionService.getPartitionMap()` returns wrong ports, and there is no lifecycle management for handlers.

The Phase 2.5 Service Architecture Research (`.specflow/reference/RUST_SERVICE_ARCHITECTURE.md`) defines a Hazelcast-informed hybrid architecture that replaces the 26 TS handlers with:

- **ServiceRegistry** with lifecycle hooks (init/reset/shutdown)
- **Operation enum** with provenance tracking (CallerOrigin) and partition routing metadata
- **Tower middleware pipeline** for cross-cutting concerns (timeout, metrics, load-shedding)
- **OperationRouter** dispatching to 7 domain services
- **BackgroundWorker** pattern for periodic tasks (GC)

### Phase 3 Scope (single-node, minimum viable)

Per the architecture document (Section 10.1), Phase 3 includes:

- `ServiceRegistry` with `ManagedService` only (no `MigrationAwareService` yet)
- `Operation` enum with all message type variants
- `OperationRouter` with 7 domain services (stub implementations)
- Tower pipeline: `TimeoutLayer` + `MetricsLayer` + `LoadShedLayer`
- No `PartitionRoutingLayer` (single-node first)
- No `MigrationBarrierLayer` (no migrations in single-node)
- Background workers: GC only
- `OperationService` to classify `Message` into `Operation`
- Server assembly wiring

### Design Source

All trait definitions, enum structures, middleware patterns, and assembly sequences are drawn from `.specflow/reference/RUST_SERVICE_ARCHITECTURE.md`. That document is the authoritative design reference for this specification.

### What This Does NOT Include

- `MigrationAwareService` trait (Phase 3+ cluster)
- `PartitionRoutingLayer` and `MigrationBarrierLayer` (Phase 3+ cluster)
- `AuthLayer` (Phase 4 security)
- Domain service business logic (each domain service gets its own spec)
- Heartbeat and persistence flush workers (Phase 3+ cluster)

### Umbrella Specification Notice

This specification is an **umbrella design document**. It defines the full operation routing architecture across ~20 files, which exceeds the Language Profile limit (max 5 files per spec). It MUST be split into sub-specifications via `/sf:split` before implementation. The parent spec itself is NOT directly implementable. Each sub-spec (SPEC-059a through SPEC-059e) will comply with the 5-file limit individually.

## Goal Analysis

### Goal Statement

Provide the operation routing and execution framework so that incoming `Message` values are classified into typed `Operation` variants, passed through a composable Tower middleware pipeline, and dispatched to the correct domain service -- enabling each domain (CRDT, Sync, Query, etc.) to be implemented independently in subsequent specs.

### Observable Truths

1. A `ServiceRegistry` can register, retrieve by type, retrieve by name, initialize in order, and shut down in reverse order any number of `ManagedService` implementations
2. An `Operation` enum represents every client-facing message type with an `OperationContext` carrying call_id, partition_id, service_name, caller_origin, client_id, timestamp, and timeout
3. An `OperationService::classify()` returns `Result<Operation, ClassifyError>` for any `Message` variant -- classifiable client-to-server messages produce `Ok(Operation)`, while server-to-client responses and transport envelopes produce `Err(ClassifyError)` with a descriptive variant
4. An `OperationRouter` dispatches each `Operation` to the domain service matching its `service_name`, returning `OperationError::UnknownService` for unregistered names
5. A Tower pipeline wraps the `OperationRouter` with `TimeoutLayer`, `MetricsLayer`, and `LoadShedLayer` that apply to every operation uniformly
6. A `BackgroundWorker<T>` can start, receive tasks via channel, execute them through a `BackgroundRunnable`, and shut down gracefully
7. Seven domain service stubs (CrdtService, SyncService, QueryService, MessagingService, CoordinationService, SearchService, PersistenceService) each implement `ManagedService` and `tower::Service<Operation>`, returning `OperationResponse::NotImplemented` for their operations

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `service/mod.rs` | Module declarations and re-exports for service subsystem |
| `service/registry.rs` | `ManagedService` trait, `ServiceContext`, `ServiceRegistry` struct |
| `service/operation.rs` | `CallerOrigin`, `OperationContext`, `Operation` enum, `OperationResponse`, `OperationError`, `ClassifyError` |
| `service/classify.rs` | `OperationService` that converts `Message` to `Result<Operation, ClassifyError>` |
| `service/router.rs` | `OperationRouter` implementing `tower::Service<Operation>` |
| `service/middleware/mod.rs` | Module declarations for middleware |
| `service/middleware/timeout.rs` | `TimeoutLayer` for operation-level timeouts |
| `service/middleware/metrics.rs` | `MetricsLayer` for operation timing and counting |
| `service/middleware/load_shed.rs` | `LoadShedLayer` for semaphore-based backpressure |
| `service/middleware/pipeline.rs` | `build_operation_pipeline()` function composing all layers |
| `service/worker.rs` | `BackgroundWorker<T>`, `BackgroundRunnable` trait, `GcTask` enum |
| `service/domain/mod.rs` | All 7 domain service stubs inline (CrdtService, SyncService, QueryService, MessagingService, CoordinationService, SearchService, PersistenceService) |
| `service/config.rs` | `ServerConfig` (operation-level config: timeouts, concurrency limits) |
| `lib.rs` | Add `pub mod service;` |

### Required Wiring

- `ServiceRegistry` holds services in `DashMap<&'static str, ServiceEntry>` (by name) and `DashMap<TypeId, Arc<dyn Any>>` (by type), with `RwLock<Vec<&'static str>>` for init/shutdown ordering
- `OperationContext` references `topgun_core::Timestamp` and carries partition_id computed by `topgun_core::hash_to_partition()`
- `OperationService` holds `Arc<parking_lot::Mutex<HLC>>` for timestamps (HLC::now() requires `&mut self`) and uses `topgun_core::hash_to_partition()` for partition ID computation
- `OperationRouter` holds `HashMap<&'static str, Box<dyn Service<Operation>>>` mapping service names to domain services
- `build_operation_pipeline()` wraps `OperationRouter` in Tower layers and returns `impl Service<Operation>`
- Each domain service stub implements both `ManagedService` (lifecycle) and `tower::Service<Operation>` (request handling)
- `BackgroundWorker<T>` uses `tokio::sync::mpsc` channel for task submission and `tokio::spawn` for the worker loop

### Key Links (fragile/critical)

- `Operation` enum variants MUST align 1:1 with the `Message` enum variants that represent client-to-server requests (not server-to-client responses). Any new `Message` variant requires a corresponding `Operation` variant.
- `OperationContext.service_name` MUST match the `ManagedService::name()` return value of the target domain service. A mismatch causes `OperationError::UnknownService` at runtime.
- Tower `Service<Operation>` requires `poll_ready` + `call` -- domain services that hold shared state via `Arc` need `&self` not `&mut self`. This means using `impl Service<Operation> for Arc<DomainService>` or wrapping in a newtype.
- `parking_lot::RwLock` (not `std::sync::RwLock`) is needed for `init_order` in `ServiceRegistry` because `DashMap` already uses `parking_lot` internally, and mixing lock implementations can cause subtle issues.

## Task

Create a `service/` module in `packages/server-rust/src/` implementing the operation routing and execution framework with:

1. **ServiceRegistry:** `ManagedService` trait with init/reset/shutdown, `ServiceContext`, `ServiceRegistry` with type-based and name-based lookup, ordered init and reverse-ordered shutdown
2. **Operation Model:** `CallerOrigin` enum, `OperationContext` struct, `Operation` enum covering all client-to-server message types, `OperationResponse` enum, `OperationError` enum, `ClassifyError` enum
3. **Classification:** `OperationService` that converts `Message` + client metadata into `Result<Operation, ClassifyError>`
4. **Routing:** `OperationRouter` implementing `tower::Service<Operation>` with name-based dispatch to domain services
5. **Middleware:** `TimeoutLayer`, `MetricsLayer`, `LoadShedLayer`, and `build_operation_pipeline()` compositor
6. **Background Workers:** `BackgroundWorker<T>` generic worker, `BackgroundRunnable` trait, `GcTask` enum
7. **Domain Service Stubs:** Seven services implementing `ManagedService` + `tower::Service<Operation>`, each returning `OperationResponse::NotImplemented` for their operations

## Requirements

### Files to Create

**Module structure:**
```
packages/server-rust/src/service/
  mod.rs                              # Module declarations, re-exports
  registry.rs                         # ManagedService trait, ServiceContext, ServiceRegistry
  operation.rs                        # CallerOrigin, OperationContext, Operation, OperationResponse, OperationError, ClassifyError
  classify.rs                         # OperationService (Message -> Result<Operation, ClassifyError>)
  router.rs                           # OperationRouter
  config.rs                           # ServerConfig (operation-level)
  worker.rs                           # BackgroundWorker<T>, BackgroundRunnable, GcTask
  middleware/
    mod.rs                            # Module declarations
    timeout.rs                        # TimeoutLayer
    metrics.rs                        # MetricsLayer
    load_shed.rs                      # LoadShedLayer
    pipeline.rs                       # build_operation_pipeline()
  domain/
    mod.rs                            # All 7 domain service stubs inline
```

### Files to Modify

| File | Change |
|------|--------|
| `packages/server-rust/src/lib.rs` | Add `pub mod service;` and re-export key types |
| `packages/server-rust/Cargo.toml` | Add `parking_lot` and `thiserror` dependencies |

### Key Type Signatures

**ManagedService trait:**
```rust
#[async_trait]
pub trait ManagedService: Send + Sync + Any {
    fn name(&self) -> &'static str;
    async fn init(&self, ctx: &ServiceContext) -> anyhow::Result<()>;
    async fn reset(&self) -> anyhow::Result<()>;
    async fn shutdown(&self, terminate: bool) -> anyhow::Result<()>;
}
```

**ServiceContext struct:**
```rust
#[derive(Debug, Clone)]
pub struct ServiceContext {
    pub config: Arc<ServerConfig>,
}
```

**CallerOrigin enum:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerOrigin {
    Client,
    Forwarded,
    Backup,
    Wan,
    System,
}
```

**OperationContext struct:**
```rust
#[derive(Debug, Clone)]
pub struct OperationContext {
    pub call_id: u64,
    pub partition_id: Option<u32>,
    pub service_name: &'static str,
    pub caller_origin: CallerOrigin,
    pub client_id: Option<String>,
    pub caller_node_id: Option<String>,
    pub timestamp: topgun_core::Timestamp,
    pub call_timeout_ms: u64,
}
```

`OperationContext` does NOT derive `Default` because `call_id`, `service_name`, and `timestamp` are required fields with no sensible defaults. Use a constructor function `OperationContext::new(call_id, service_name, timestamp)` that sets optional fields to `None` and `call_timeout_ms` to a provided default.

**Operation enum (grouped by domain):**
- CRDT: `ClientOp`, `OpBatch`
- Sync: `SyncInit`, `MerkleReqBucket`, `OrMapSyncInit`, `OrMapMerkleReqBucket`, `OrMapDiffRequest`, `OrMapPushDiff`
- Query: `QuerySubscribe`, `QueryUnsubscribe`
- Messaging: `TopicSubscribe`, `TopicUnsubscribe`, `TopicPublish`
- Coordination: `LockRequest`, `LockRelease`, `PartitionMapRequest`, `Ping`
- Search: `Search`, `SearchSubscribe`, `SearchUnsubscribe`
- Persistence: `CounterRequest`, `CounterSync`, `EntryProcess`, `EntryProcessBatch`, `RegisterResolver`, `UnregisterResolver`, `ListResolvers`, `JournalSubscribe`, `JournalUnsubscribe`, `JournalRead`
- System (internal, not from classify): `GarbageCollect`

Each variant carries `ctx: OperationContext` plus domain-specific payload fields extracted from the corresponding `Message` inner struct.

**Routing decisions for specific operations:**
- `Ping` routes to `CoordinationService` (`service_name: "coordination"`). CoordinationService is the natural fit for cluster-level heartbeat concerns. The Pong response is generated by CoordinationService.
- `CounterSync` is a **bidirectional** message: the Rust Message doc says "Server syncs counter state" but the TS handler (`CounterHandler.handleCounterSync`) documents "client sends their state to merge." The TS `message-registry.ts` registers `COUNTER_SYNC` as a client-handled message. It remains a PersistenceService operation.
- `ORMapPushDiff` is a **bidirectional** message: the Rust Message doc says "Server pushes ORMap diff to client" but the TS `handlers-module.ts` registers `ORMAP_PUSH_DIFF` as a client-handled message routed to `orMapSyncHandler.handleORMapPushDiff`. It remains a SyncService operation.
- `GarbageCollect` is a system-internal operation triggered by `BackgroundWorker`, not from `classify()`. It is constructed directly by the GC worker with `CallerOrigin::System`.

**ClassifyError enum:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum ClassifyError {
    #[error("server-to-client response cannot be classified as operation: {variant}")]
    ServerToClient { variant: &'static str },
    #[error("transport envelope must be unpacked before classification: {variant}")]
    TransportEnvelope { variant: &'static str },
    #[error("authentication message handled at transport layer: {variant}")]
    AuthMessage { variant: &'static str },
}
```

**classify() signature:**
```rust
impl OperationService {
    pub fn classify(
        &self,
        msg: Message,
        client_id: Option<String>,
        caller_origin: CallerOrigin,
    ) -> Result<Operation, ClassifyError>;
}
```

`classify()` behavior for non-operation Message variants:
- **Server-to-client responses** (OpAck, OpRejected, SyncRespRoot, SyncRespBuckets, SyncRespLeaf, ORMapSyncRespRoot, ORMapSyncRespBuckets, ORMapSyncRespLeaf, ORMapDiffResponse, QueryResp, QueryUpdate, SearchResp, SearchUpdate, CounterResponse, CounterUpdate, EntryProcessResponse, EntryProcessBatchResponse, JournalEvent, JournalReadResponse, RegisterResolverResponse, UnregisterResolverResponse, ListResolversResponse, MergeRejected, ServerEvent, ServerBatchEvent, GcPrune, AuthAck, AuthFail, Error, LockGranted, LockReleased, SyncResetRequired, Pong, PartitionMap, TopicMessage): return `Err(ClassifyError::ServerToClient { variant })`
- **Transport envelopes** (Batch): `Message::Batch` is unpacked by the network layer before reaching `classify()`. If `classify()` receives a `Batch`, it returns `Err(ClassifyError::TransportEnvelope { variant: "Batch" })`
- **Auth messages** (Auth, AuthRequired): handled at the transport layer before routing. Return `Err(ClassifyError::AuthMessage { variant })`
- **Cluster-internal messages** (ClusterSubRegister, ClusterSubAck, ClusterSubUpdate, ClusterSubUnregister, ClusterSearchReq, ClusterSearchResp, ClusterSearchSubscribe, ClusterSearchUnsubscribe, ClusterSearchUpdate): return `Err(ClassifyError::ServerToClient { variant })` (these are node-to-node, not client-to-server operations; they will be handled by a separate cluster message path in Phase 3+)

**OperationResponse enum:**
```rust
#[derive(Debug)]
pub enum OperationResponse {
    Ack { call_id: u64 },
    Message(topgun_core::messages::Message),
    Messages(Vec<topgun_core::messages::Message>),
    NotImplemented { service_name: &'static str, call_id: u64 },
    Empty,
}
```

**OperationError enum:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("unknown service: {name}")]
    UnknownService { name: String },
    #[error("operation timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("server overloaded, try again later")]
    Overloaded,
    #[error("wrong service for operation")]
    WrongService,
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
```

**BackgroundRunnable trait:**
```rust
#[async_trait]
pub trait BackgroundRunnable: Send + 'static {
    type Task: Send + 'static;
    async fn run(&mut self, task: Self::Task);
    async fn on_tick(&mut self) {}
    async fn shutdown(&mut self) {}
}
```

**ServerConfig (operation-level):**
```rust
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub node_id: String,
    pub default_operation_timeout_ms: u64,
    pub max_concurrent_operations: u32,
    pub gc_interval_ms: u64,
    pub partition_count: u32,  // Configurable for testing; defaults to topgun_core::PARTITION_COUNT (271) in production
}
```

### Deletions

None. This is additive.

## Acceptance Criteria

1. `ServiceRegistry::register()` stores a service retrievable via both `get::<ConcreteType>()` and `get_by_name("name")`
2. `ServiceRegistry::init_all()` calls `ManagedService::init()` on every registered service in registration order
3. `ServiceRegistry::shutdown_all()` calls `ManagedService::shutdown()` on every registered service in reverse registration order
4. `Operation` enum has one variant per client-to-server message type (minimum 30 variants: 29 from the mapping table in RUST_SERVICE_ARCHITECTURE.md Section 7.1 plus Ping routed to CoordinationService), plus `GarbageCollect` as a system-internal variant
5. `OperationService::classify()` converts every client-to-server `Message` variant into the correct `Operation` variant with appropriate `service_name` and `partition_id`, and returns `Err(ClassifyError)` for server-to-client, transport envelope, auth, and cluster-internal variants
6. `OperationRouter::call()` dispatches to the registered domain service for the operation's `service_name` and returns `OperationError::UnknownService` for unregistered names
7. `TimeoutLayer` rejects operations that exceed `call_timeout_ms` with `OperationError::Timeout`
8. `LoadShedLayer` rejects operations when concurrent count exceeds `max_concurrent_operations` with `OperationError::Overloaded`
9. `MetricsLayer` records operation duration and increments a counter (using `tracing` spans, not a full metrics crate)
10. `BackgroundWorker::start()` spawns a tokio task that processes tasks from its mpsc channel, and `stop()` shuts down gracefully
11. All seven domain service stubs compile and return `OperationResponse::NotImplemented` for their respective operations
12. `cargo test` passes with all new tests; `cargo clippy` reports no warnings
13. All types follow PROJECT.md Rust type mapping rules (no `f64` for integer semantics, enums for known value sets, `Default` on structs with 2+ optional fields)

## Constraints

- Do NOT implement domain service business logic -- stubs only return `NotImplemented`
- Do NOT add `MigrationAwareService` trait -- that is Phase 3+ (cluster)
- Do NOT add `PartitionRoutingLayer` or `MigrationBarrierLayer` -- those require cluster infrastructure
- Do NOT add `AuthLayer` -- that is Phase 4 (security)
- Do NOT delete or modify the existing `ServerStorage`, `MapProvider`, or `SchemaProvider` traits -- those are consumed by storage, not operations
- The `service_name` values are compile-time `&'static str` constants, not runtime strings: `"crdt"`, `"sync"`, `"query"`, `"messaging"`, `"coordination"`, `"search"`, `"persistence"`
- `OperationContext.call_timeout_ms` defaults to `ServerConfig.default_operation_timeout_ms` but individual operations can override if their `Message` payload includes a timeout field
- Internal-only types (`OperationContext`, `ServerConfig`, `CallerOrigin`, `OperationError`, `ClassifyError`, `OperationResponse`, `ServiceContext`) do NOT require `#[serde(rename_all = "camelCase")]` or any serde derives. These types are never serialized to MsgPack; they exist only within the server process. The PROJECT.md Auditor Checklist serde requirement applies only to types that cross the wire boundary.

## Assumptions

- `parking_lot` crate will be added as a dependency for `ServiceRegistry` internal locking (it is already an indirect dependency via `dashmap`)
- `thiserror` crate will be added as a dependency for `OperationError` and `ClassifyError` derive macros (zero-cost at runtime; it is a proc-macro crate that generates code at compile time only)
- Partition ID computation uses `topgun_core::hash_to_partition()` which already exists in core-rust
- `Ping` is classified as an Operation routed to `CoordinationService` (`service_name: "coordination"`), which generates the Pong response
- `MetricsLayer` uses `tracing::info_span!` for timing rather than adding a full metrics crate (prometheus/metrics) -- that is a future enhancement
- Domain service stubs hold no real dependencies; they will be fleshed out in subsequent per-domain specs
- `BackgroundWorker` tick interval (for periodic tasks like GC scheduling) is configurable but defaults to 60 seconds
- `tower::Service` is implemented for `Arc<DomainService>` (not `&mut DomainService`) because services are shared across async tasks
- `Message::Batch` is always unpacked by the network layer before reaching `classify()`. If `classify()` receives a `Batch`, it returns `Err(ClassifyError::TransportEnvelope)`

## Suggested Split Plan

This specification is **large** (~20 files, 5+ subsystems). It MUST be split via `/sf:split` before implementation. The recommended decomposition follows trait-first ordering. Each sub-spec complies with the Language Profile limit of 5 files.

### SPEC-059a: Service Registry and Operation Types (Wave 1 -- traits/types only)
- `service/mod.rs`, `service/registry.rs`, `service/operation.rs`, `service/config.rs`
- `ManagedService` trait, `ServiceContext`, `ServiceRegistry`
- `CallerOrigin`, `OperationContext`, `Operation` enum, `OperationResponse`, `OperationError`, `ClassifyError`
- `ServerConfig`
- ~4 files, Wave 1 (no dependencies)

### SPEC-059b: Operation Classification and Routing (Wave 2)
- `service/classify.rs`, `service/router.rs`
- `OperationService` (Message -> Result<Operation, ClassifyError> classification)
- `OperationRouter` (name-based dispatch)
- ~2 files, depends on SPEC-059a

### SPEC-059c: Tower Middleware Pipeline (Wave 2, parallel with 059b)
- `service/middleware/mod.rs`, `service/middleware/timeout.rs`, `service/middleware/metrics.rs`, `service/middleware/load_shed.rs`, `service/middleware/pipeline.rs`
- `TimeoutLayer`, `MetricsLayer`, `LoadShedLayer`, `build_operation_pipeline()`
- ~5 files, depends on SPEC-059a

### SPEC-059d: Background Workers and Domain Service Stubs (Wave 3)
- `service/worker.rs`, `service/domain/mod.rs`
- `BackgroundWorker<T>`, `BackgroundRunnable`, `GcTask`
- Seven domain service stubs (all inline in `domain/mod.rs` -- each stub is 5-15 lines, combined into a single file)
- ~2 files, depends on SPEC-059a

### SPEC-059e: Integration Wiring (Wave 4)
- `lib.rs` modification, `Cargo.toml` update
- Full pipeline integration test: Message -> classify -> pipeline -> router -> stub -> response
- Depends on SPEC-059b, SPEC-059c, SPEC-059d

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 (059a) | 1 | ManagedService trait, ServiceContext, ServiceRegistry, Operation enum, OperationContext, CallerOrigin, OperationResponse, OperationError, ClassifyError, ServerConfig | -- | ~25% |
| G2 (059b) | 2 | OperationService (classify), OperationRouter | G1 | ~20% |
| G3 (059c) | 2 | TimeoutLayer, MetricsLayer, LoadShedLayer, build_operation_pipeline() | G1 | ~20% |
| G4 (059d) | 3 | BackgroundWorker, BackgroundRunnable, GcTask, 7 domain service stubs (inline in domain/mod.rs) | G1 | ~20% |
| G5 (059e) | 4 | lib.rs wiring, Cargo.toml, integration tests | G2, G3, G4 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |
| 4 | G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-21)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (20+ files across 5 subsystems -- far exceeds single-spec capacity)

**Critical:**
1. **Language Profile violation -- file count (20 files vs max 5):** PROJECT.md Language Profile sets `Max files per spec: 5`. This spec creates 20 files plus modifies 2. The spec correctly self-identifies as needing a split, but it must actually BE split via `/sf:split` before it can proceed. The parent spec cannot be implemented directly.
2. **`thiserror` contradiction:** The OperationError code example uses `#[derive(thiserror::Error)]` but the Constraints section says "Do NOT use thiserror unless it is already in Cargo.toml dependencies." Checked: `thiserror` is NOT in `packages/server-rust/Cargo.toml`. Either add `thiserror` to the Cargo.toml additions in "Files to Modify" (alongside `parking_lot`) and remove the constraint, or change the code example to use manual `Display` + `Error` impls. The current state is contradictory.
3. **`OperationService::classify()` does not specify behavior for non-operation Message variants:** The `Message` enum has ~77 variants, but only ~29-32 are client-to-server operations. What does `classify()` return for the ~45 server-to-client variants (OpAck, QueryResp, SyncRespRoot, AuthAck, ServerEvent, etc.) and transport envelopes (Batch, Auth, AuthRequired)? The spec must define the return type for unclassifiable messages -- either an `OperationError::NotClassifiable` variant, a `Result<Operation, ClassifyError>`, or explicit documentation that those variants are unreachable at the classify call site. Without this, the implementer must guess.
4. **SPEC-059c exceeds Language Profile file limit (5 files):** The middleware sub-spec creates `mod.rs`, `timeout.rs`, `metrics.rs`, `load_shed.rs`, `pipeline.rs` -- exactly 5 files. This is at the limit but technically compliant. However, SPEC-059d lists 9 files which significantly exceeds the 5-file limit. The split plan must be revised to keep each sub-spec at or under 5 files. Suggestion: split 059d into two sub-specs -- one for `BackgroundWorker` (worker.rs + domain/mod.rs = 2 files) and one for domain stubs (7 files, but consider combining stubs into fewer files, e.g., domain/mod.rs with all stubs inline, reducing to 1-2 files).

**Recommendations:**
5. **[Strategic] `Heartbeat` in System domain has no target service:** The spec lists `Operation::Heartbeat` under System domain but no service handles it. The `Ping` assumption says "handled inline or as system" but this is vague. Clarify: either add a "system" service, route Heartbeat to CoordinationService, or exclude Heartbeat from the Operation enum (handle at the network layer before classification).
6. **`CounterSync` direction ambiguity:** `Message::CounterSync` is documented in the codebase as "Server syncs counter state" (server-to-server or server-to-client). The mapping table classifies it as a PersistenceService operation. If CounterSync is server-to-client, it should not be an Operation variant. Verify the actual direction and adjust the mapping if needed.
7. **`ORMapPushDiff` direction ambiguity:** `Message::ORMapPushDiff` is documented as "Server pushes ORMap diff to client" but appears in the mapping table as a SyncService operation (client-to-server). Verify whether this is bidirectional or if the mapping is incorrect.
8. **`ServiceContext` is undefined:** The `ManagedService::init()` takes `&ServiceContext` but `ServiceContext` is never defined in the spec. What fields does it carry? At minimum it likely needs `Arc<ServerConfig>` and perhaps `Arc<ServiceRegistry>` for inter-service discovery. Define the struct or at least its key fields.
9. **`OperationContext` has 3 optional fields but does not derive `Default`:** Per PROJECT.md rules, structs with 2+ optional fields should derive `Default`. However, `OperationContext` also has required fields (`call_id`, `service_name`, `timestamp`) with no sensible defaults. Consider whether a builder pattern or constructor function is more appropriate, and document the decision explicitly.
10. **Serde annotations not needed on internal types:** The PROJECT.md Auditor Checklist requires `#[serde(rename_all = "camelCase")]` on every struct, but `OperationContext`, `ServerConfig`, and `CallerOrigin` are internal types never serialized to MsgPack. The sub-specs should explicitly note that serde derives are omitted for internal-only types.
11. **`Batch` message handling in classify:** `Message::Batch` is a transport envelope containing packed sub-messages. The classify function needs to either unpack it (recursive classification) or reject it. Document the intended behavior.
12. **Consider combining domain stubs to reduce file count:** The 7 domain service stubs are each 5-15 lines. Combining them into a single `domain/stubs.rs` file (or even putting them all in `domain/mod.rs`) would reduce file count from 9 to 2-3 in SPEC-059d, helping comply with the Language Profile limit.

### Response v1 (2026-02-21)
**Applied:** All 4 critical issues and all 8 recommendations (items 1-12).

**Changes:**
1. [v] Language Profile violation -- Added "Umbrella Specification Notice" section in Context clarifying this spec is a design document that MUST be split via `/sf:split` before implementation. Each sub-spec individually complies with the 5-file limit.
2. [v] `thiserror` contradiction -- Added `thiserror` to Files to Modify table (alongside `parking_lot`). Removed the contradictory constraint "Do NOT use thiserror unless..." from Constraints. Updated Assumptions to state thiserror WILL be added (zero-cost proc-macro). Both `OperationError` and `ClassifyError` now use `thiserror::Error` derive.
3. [v] `classify()` behavior for non-operation Messages -- Changed Observable Truth #3 and classify signature to return `Result<Operation, ClassifyError>`. Added full `ClassifyError` enum with 3 variants (ServerToClient, TransportEnvelope, AuthMessage). Documented exhaustive categorization of all non-classifiable Message variants (server-to-client responses, transport envelopes, auth messages, cluster-internal messages). Updated acceptance criterion #5 to require ClassifyError for non-classifiable variants.
4. [v] SPEC-059d file count -- Combined all 7 domain service stubs into `domain/mod.rs` (inline). SPEC-059d now has 2 files (`worker.rs`, `domain/mod.rs`), well within the 5-file limit. Updated Required Artifacts, Files to Create, and Split Plan sections.
5. [v] Heartbeat/Ping routing -- Removed `Heartbeat` from Operation enum (no such Message variant exists in Rust Message enum; only `Ping` and `Pong` exist). Moved `Ping` from System domain to Coordination domain, routed to CoordinationService. Added routing decision documentation explaining the rationale.
6. [v] CounterSync direction -- Verified bidirectional: TS CounterHandler.handleCounterSync documents "client sends their state to merge" and message-registry.ts registers COUNTER_SYNC as client-handled. Kept as PersistenceService operation. Added routing decision note documenting the verification.
7. [v] ORMapPushDiff direction -- Verified bidirectional: TS handlers-module.ts registers ORMAP_PUSH_DIFF with orMapSyncHandler.handleORMapPushDiff as client-handled. Kept as SyncService operation. Added routing decision note documenting the verification.
8. [v] ServiceContext definition -- Added `ServiceContext` struct definition with `config: Arc<ServerConfig>`. Added to Key Type Signatures and Required Artifacts.
9. [v] OperationContext Default -- Added explicit documentation that OperationContext does NOT derive Default due to required fields (call_id, service_name, timestamp). Documented constructor function `OperationContext::new()` as the intended construction pattern.
10. [v] Serde annotations note -- Added constraint in Constraints section explicitly listing all internal-only types that do NOT require serde derives, with rationale that the Auditor Checklist serde requirement applies only to wire-boundary types.
11. [v] Batch handling in classify -- Documented that `Message::Batch` is unpacked by the network layer before reaching classify. If classify receives a Batch, it returns `Err(ClassifyError::TransportEnvelope)`. Added to Assumptions and classify behavior documentation.
12. [v] Domain stubs combined -- Already addressed by item 4. All 7 stubs inline in `domain/mod.rs`.

### Audit v2 (2026-02-21)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (umbrella spec -- not directly implementable; sub-specs individually ~15-25%)

**Audit scope:** Post-revision re-audit with fresh codebase verification. All v1 critical issues confirmed resolved. One new critical issue discovered via source code inspection.

**Critical:**
1. **`OperationService` holds `Arc<HLC>` but `HLC::now()` requires `&mut self`:** The Required Wiring section states "OperationService holds `Arc<HLC>` for timestamps." However, `HLC::now()` in `packages/core-rust/src/hlc.rs` (line 330) has signature `pub fn now(&mut self) -> Timestamp` -- it requires mutable access because it updates internal counter/millis state. `Arc<HLC>` provides only shared (`&self`) access. The spec must change `Arc<HLC>` to `Arc<parking_lot::Mutex<HLC>>` (or `Arc<tokio::sync::Mutex<HLC>>`) in the Required Wiring section, the classify() implementation notes, and in SPEC-059b scope. Without this fix, `classify()` cannot generate timestamps and the code will not compile.

**Recommendations:**
2. **`TopicMessage` missing from classify() server-to-client list:** The classify() behavior section exhaustively lists server-to-client variants that return `Err(ClassifyError::ServerToClient)`, but `Message::TopicMessage` (line 307-310 of `packages/core-rust/src/messages/mod.rs`, doc: "Server delivers a topic message") is not in the list. It should be added alongside TopicSub/TopicUnsub/TopicPub. This is a completeness gap in the exhaustive enumeration -- an implementer might incorrectly classify TopicMessage as a client-to-server operation.
3. **Acceptance criterion #4 variant count says "minimum 29" but spec defines 30 client-facing + 1 system = 31:** The mapping table in Section 7.1 has 29 rows (without Ping). The spec correctly adds Ping (routed to CoordinationService) beyond what the mapping table shows. Update AC#4 to say "minimum 30 client-to-server variants" to accurately reflect the spec's own Operation enum definition and avoid confusion during review.
4. **`ServerConfig.partition_count` may duplicate `topgun_core::partition::PARTITION_COUNT`:** The core-rust crate defines `pub const PARTITION_COUNT: u32 = 271` in `packages/core-rust/src/partition.rs`. The spec adds `partition_count: u32` to `ServerConfig`. If this field always equals 271, it is redundant. If it is intended to be configurable (e.g., for testing), document that intention. Otherwise, remove it and use the core constant directly.

**Project compliance:** Honors PROJECT.md decisions (Rust type mapping, trait-first ordering, Language Profile file limits via umbrella/split pattern, no deferred features).

**Strategic fit:** Aligned with project goals -- Phase 3 operation routing is the logical next step after network layer and storage.

**Language profile:** Compliant as umbrella spec. All 5 sub-specs individually meet the 5-file limit (4, 2, 5, 2, 2 files respectively).

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (call_id: u64, partition_id: u32, timeout_ms: u64, etc.)
- [x] No `r#type: String` on message structs (N/A -- Operation is internal)
- [x] `Default` exemption for OperationContext documented with rationale
- [x] Enums used for known value sets (CallerOrigin, ClassifyError, OperationError, OperationResponse)
- [x] Wire compatibility: N/A (internal types only)
- [x] Serde exemption explicitly documented in Constraints
- [x] `Option<T>` serde annotation: N/A (internal types only)

### Response v2 (2026-02-21)
**Applied:** All 1 critical issue and all 3 recommendations (items 1-4).

**Changes:**
1. [v] `Arc<HLC>` mutability -- Changed `Arc<HLC>` to `Arc<parking_lot::Mutex<HLC>>` in Required Wiring section with note that `HLC::now()` requires `&mut self`.
2. [v] `TopicMessage` missing from classify() list -- Already applied externally; `TopicMessage` now present in server-to-client response list on line 245.
3. [v] AC#4 variant count -- Updated from "minimum 29 variants" to "minimum 30 variants: 29 from the mapping table plus Ping routed to CoordinationService".
4. [v] `ServerConfig.partition_count` duplication -- Added inline comment documenting that the field is configurable for testing and defaults to `topgun_core::PARTITION_COUNT` (271) in production.

### Audit v3 (2026-02-21)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (umbrella spec -- not directly implementable; sub-specs individually ~15-25%)

**Audit scope:** Post-revision v2 re-audit with fresh codebase verification against `packages/core-rust/src/messages/mod.rs` (77 Message variants), `packages/core-rust/src/hlc.rs`, `packages/core-rust/src/partition.rs`, and `packages/server-rust/Cargo.toml`.

**Codebase verification results:**
- Message enum: 77 variants confirmed. 30 client-to-server operations, 35 server-to-client responses, 1 transport envelope, 2 auth, 9 cluster-internal. Exhaustive mapping in classify() is complete and correct.
- `HLC::now(&mut self)` confirmed at line 330 of hlc.rs. `Arc<parking_lot::Mutex<HLC>>` fix is correctly in place.
- `hash_to_partition()` re-exported from `topgun_core` root (lib.rs line 62). Confirmed available.
- `Timestamp` re-exported from `topgun_core` root (lib.rs line 40). Confirmed available.
- `PARTITION_COUNT = 271` confirmed (partition.rs line 20). `ServerConfig.partition_count` documented as test-configurable.
- Dependencies already present: `tower 0.5`, `dashmap 6`, `async-trait 0.1`, `anyhow 1`, `tokio` (sync/time), `tracing 0.1`. Only `parking_lot` and `thiserror` need adding. Correct.
- `lib.rs` currently has `network`, `storage`, `traits` modules. Adding `pub mod service;` is straightforward.

**All prior critical issues confirmed resolved.** No new critical issues found.

**Per-Group Breakdown (sub-specs after split):**

| Group | Wave | Tasks | Est. Context | Status |
|-------|------|-------|--------------|--------|
| G1 (059a) | 1 | ManagedService trait, ServiceRegistry, Operation enum, types, config | ~25% | OK |
| G2 (059b) | 2 | OperationService (classify), OperationRouter | ~20% | OK |
| G3 (059c) | 2 | TimeoutLayer, MetricsLayer, LoadShedLayer, pipeline | ~20% | OK |
| G4 (059d) | 3 | BackgroundWorker, domain stubs | ~20% | OK |
| G5 (059e) | 4 | lib.rs wiring, Cargo.toml, integration tests | ~15% | OK |

**Quality Projection:** GOOD range (each sub-spec individually 15-25%)

**Recommendations:**
1. **G4 (059d) could run in Wave 2 instead of Wave 3:** G4 depends only on G1. Its placement in Wave 3 is unnecessarily sequential. Moving G4 to Wave 2 (parallel with G2 and G3) would reduce total waves from 4 to 3, at the cost of increasing max parallel workers from 2 to 3. Consider during `/sf:split` whether the parallelism gain is worth the extra worker.
2. **Cluster-internal messages use `ClassifyError::ServerToClient` which is semantically misleading:** Nine cluster-internal messages (ClusterSubRegister, etc.) are categorized under `ServerToClient` error variant, with a parenthetical note explaining they are node-to-node. Consider adding a `ClusterInternal` variant to `ClassifyError` for clearer error semantics. This is a minor clarity improvement -- not blocking.

**Project compliance:** Honors PROJECT.md decisions (Rust type mapping, trait-first ordering, Language Profile file limits via umbrella/split pattern, no deferred features).

**Strategic fit:** Aligned with project goals -- Phase 3 operation routing is the logical next step after network layer (SPEC-057a-c) and storage (SPEC-058a-c).

**Language profile:** Compliant as umbrella spec. All 5 sub-specs individually meet the 5-file limit (4, 2, 5, 2, 2 files respectively). Trait-first ordering respected: G1 (Wave 1) contains only traits and types.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (call_id: u64, partition_id: u32, timeout_ms: u64, etc.)
- [x] No `r#type: String` on message structs (N/A -- Operation is internal)
- [x] `Default` exemption for OperationContext documented with rationale
- [x] Enums used for known value sets (CallerOrigin, ClassifyError, OperationError, OperationResponse)
- [x] Wire compatibility: N/A (internal types only)
- [x] Serde exemption explicitly documented in Constraints
- [x] `Option<T>` serde annotation: N/A (internal types only)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (ServiceRegistry) has artifacts | OK | registry.rs |
| Truth 2 (Operation enum) has artifacts | OK | operation.rs |
| Truth 3 (classify) has artifacts | OK | classify.rs |
| Truth 4 (OperationRouter) has artifacts | OK | router.rs |
| Truth 5 (Tower pipeline) has artifacts | OK | middleware/*.rs |
| Truth 6 (BackgroundWorker) has artifacts | OK | worker.rs |
| Truth 7 (domain stubs) has artifacts | OK | domain/mod.rs |
| All artifacts map to truths | OK | No orphans |
| Wiring completeness | OK | All connections specified |
| Key links identified | OK | 4 fragile links documented |

**Assumptions verified against codebase:**

| # | Assumption | Verified | Impact if wrong |
|---|------------|----------|-----------------|
| A1 | `parking_lot` is indirect dep via `dashmap` | Yes (dashmap 6 uses parking_lot) | Build would still work; add as direct dep |
| A2 | `hash_to_partition()` exists in core-rust | Yes (partition.rs line 42, re-exported) | Would need to implement |
| A3 | `tower::Service` available in server-rust | Yes (tower 0.5 in Cargo.toml) | Would need to add dependency |
| A4 | `async-trait` available | Yes (async-trait 0.1 in Cargo.toml) | Would need to add dependency |
| A5 | `HLC::now()` requires `&mut self` | Yes (hlc.rs line 330) | Already fixed to Mutex wrapper |

**Recommendation:** Use `/sf:split` to decompose into SPEC-059a through SPEC-059e, then `/sf:run --parallel` for implementation.

## Execution Summary

**Executed:** 2026-02-22
**Mode:** orchestrated (sequential fallback -- subagent spawning unavailable)
**Commits:** 6

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 (059a) | complete |
| 2 | G2 (059b), G3 (059c) | complete |
| 3 | G4 (059d) | complete |
| 4 | G5 (059e) | complete |

### Files Created

- `packages/server-rust/src/service/mod.rs` -- Module declarations and re-exports
- `packages/server-rust/src/service/config.rs` -- ServerConfig struct
- `packages/server-rust/src/service/registry.rs` -- ManagedService trait, ServiceContext, ServiceRegistry
- `packages/server-rust/src/service/operation.rs` -- CallerOrigin, OperationContext, Operation enum (31 variants), OperationResponse, OperationError, ClassifyError, service_names
- `packages/server-rust/src/service/classify.rs` -- OperationService (Message -> Result<Operation, ClassifyError>)
- `packages/server-rust/src/service/router.rs` -- OperationRouter (Tower Service dispatch by service_name)
- `packages/server-rust/src/service/worker.rs` -- BackgroundWorker<R>, BackgroundRunnable trait, GcTask enum
- `packages/server-rust/src/service/middleware/mod.rs` -- Middleware module declarations and re-exports
- `packages/server-rust/src/service/middleware/timeout.rs` -- TimeoutLayer (per-operation timeout)
- `packages/server-rust/src/service/middleware/metrics.rs` -- MetricsLayer (tracing-based timing/counting)
- `packages/server-rust/src/service/middleware/load_shed.rs` -- LoadShedLayer (semaphore-based concurrency limit)
- `packages/server-rust/src/service/middleware/pipeline.rs` -- build_operation_pipeline() compositor
- `packages/server-rust/src/service/domain/mod.rs` -- 7 domain service stubs (CrdtService, SyncService, QueryService, MessagingService, CoordinationService, SearchService, PersistenceService)

### Files Modified

- `packages/server-rust/Cargo.toml` -- Added parking_lot 0.12 and thiserror 2 dependencies
- `packages/server-rust/src/lib.rs` -- Added `pub mod service;` and key type re-exports, integration tests

### Acceptance Criteria Status

- [x] AC1: ServiceRegistry::register() stores service retrievable via get::<T>() and get_by_name()
- [x] AC2: ServiceRegistry::init_all() calls init() in registration order
- [x] AC3: ServiceRegistry::shutdown_all() calls shutdown() in reverse registration order
- [x] AC4: Operation enum has 31 variants (30 client-to-server + GarbageCollect system)
- [x] AC5: OperationService::classify() converts all 77 Message variants correctly
- [x] AC6: OperationRouter::call() dispatches to registered service, returns UnknownService for unregistered
- [x] AC7: TimeoutLayer rejects operations exceeding call_timeout_ms with OperationError::Timeout
- [x] AC8: LoadShedLayer rejects when concurrent count exceeds max with OperationError::Overloaded
- [x] AC9: MetricsLayer records operation duration via tracing spans
- [x] AC10: BackgroundWorker::start() spawns tokio task, stop() shuts down gracefully
- [x] AC11: All 7 domain service stubs compile and return OperationResponse::NotImplemented
- [x] AC12: cargo test passes (183 tests, 50 new); cargo clippy reports no warnings
- [x] AC13: All types follow PROJECT.md Rust type mapping rules

### Deviations

- OperationResponse::Message variant uses `Box<Message>` instead of `Message` to avoid large enum variant clippy warning (Message is ~360 bytes)
- CounterRequestPayload.name used for partition routing (spec said "map_name" but actual struct field is "name")
- JournalSubscribeData.map_name and JournalReadData.map_name are Option<String>, so partition routing is optional for these operations
- JournalUnsubscribeData has no map_name field, so partition routing is not possible for unsubscribe

### Test Summary

| Module | Tests |
|--------|-------|
| service::registry | 7 |
| service::operation | 8 |
| service::classify | 15 |
| service::router | 3 |
| service::middleware::timeout | 2 |
| service::middleware::metrics | 1 |
| service::middleware::load_shed | 2 |
| service::middleware::pipeline | 1 |
| service::worker | 4 |
| service::domain | 8 |
| integration_tests | 6 (including re-export test) |
| **Total new** | **50** |

---

## Review History

### Review v1 (2026-02-22 16:45)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. **Phase reference in code comment**
   - File: `packages/server-rust/src/service/classify.rs:505`
   - Issue: Comment says "by a separate cluster message path in Phase 3+." which violates the project convention in CLAUDE.md: "Do NOT add phase/spec/bug references in code comments." Should use a WHY-comment instead.
   - Fix: Change to "by a separate cluster message path (not yet implemented)." or similar phrasing that explains intent without referencing project phases.

**Passed:**
- [v] AC1: `ServiceRegistry::register()` correctly stores services retrievable via both `get::<T>()` (TypeId-based) and `get_by_name()` (string-based). Tests in `registry.rs` verify both paths including the "not found" cases.
- [v] AC2: `ServiceRegistry::init_all()` calls `init()` in registration order. Test `init_all_calls_in_registration_order` proves ordering via an atomic log.
- [v] AC3: `ServiceRegistry::shutdown_all()` calls `shutdown()` in reverse registration order. Test `shutdown_all_calls_in_reverse_order` proves reverse ordering.
- [v] AC4: `Operation` enum has 31 variants (30 client-to-server + `GarbageCollect`). The compile-time exhaustive match test in `operation_variant_count_covers_all_30_client_plus_1_system` guarantees this.
- [v] AC5: `OperationService::classify()` handles all 77 `Message` variants exhaustively. The Rust compiler enforces this -- the match is exhaustive with no wildcard arm. 30 produce `Ok(Operation)`, 35 produce `Err(ClassifyError::ServerToClient)`, 9 produce `Err(ClassifyError::ServerToClient)` for cluster-internal, 1 produces `Err(ClassifyError::TransportEnvelope)`, 2 produce `Err(ClassifyError::AuthMessage)`. Tests cover representative variants from each category.
- [v] AC6: `OperationRouter::call()` dispatches correctly. Tests verify routing to the right service among multiple, and `UnknownService` error for unregistered names.
- [v] AC7: `TimeoutLayer` rejects operations exceeding `call_timeout_ms`. Test `exceeds_timeout_returns_error` verifies with a 200ms slow service and 50ms timeout.
- [v] AC8: `LoadShedLayer` rejects when overloaded. Test `rejects_when_overloaded` verifies with max_concurrent=1 and a blocking in-flight operation.
- [v] AC9: `MetricsLayer` records via tracing spans with `info_span!("operation", ...)` including `duration_ms` and `outcome` fields. Test verifies passthrough behavior.
- [v] AC10: `BackgroundWorker::start()` spawns tokio task, processes tasks, calls `on_tick()` periodically, and `stop()` calls `shutdown()`. Four tests cover task processing, tick interval, graceful shutdown, and post-stop error handling.
- [v] AC11: All 7 domain service stubs compile and return `OperationResponse::NotImplemented`. Individual tests for each stub plus a combined lifecycle test via `ServiceRegistry`.
- [v] AC12: `cargo test` passes with 183 tests total (50 new). `cargo clippy -- -D warnings` reports zero warnings. Verified by running both commands during review.
- [v] AC13: All types use proper Rust types: `u64` for call_id/timeout_ms, `u32` for partition_id/max_concurrent_operations, enums for CallerOrigin/ClassifyError/OperationError/OperationResponse. No `f64` for integer semantics. `ServerConfig` derives `Default`. `OperationContext` uses constructor (documented exemption from `Default`).
- [v] All 13 files created match the spec's Required Artifacts list exactly.
- [v] Both files modified as specified: `Cargo.toml` has `parking_lot = "0.12"` and `thiserror = "2"`; `lib.rs` has `pub mod service;` and re-exports.
- [v] No deletions required; no existing files modified beyond spec.
- [v] No `.unwrap()` in production code; all unwraps are in `#[cfg(test)]` blocks.
- [v] No unnecessary `.clone()` calls; all clones are on `Arc` types or `Vec` copies from lock reads.
- [v] No hardcoded secrets or security issues.
- [v] No serde derives on internal-only types (per constraint).
- [v] `domain_stub!` macro reduces boilerplate effectively for the 7 stubs, each implementing both `ManagedService` and `Service<Operation> for Arc<T>`.
- [v] Deviations are sensible: `Box<Message>` avoids clippy large-variant warning; field name corrections match actual struct definitions.
- [v] Tower `Service` implemented for `Arc<DomainService>` as specified in Key Links.
- [v] `parking_lot::RwLock` used for `init_order` as specified (not `std::sync::RwLock`).
- [v] Constraints respected: no business logic in stubs, no `MigrationAwareService`, no partition/migration/auth layers, existing traits untouched, service names are `&'static str` constants.

**Rust Idiom Check:**
- [v] No unnecessary `.clone()` calls in production code
- [v] Error handling uses `?` operator and `Result<T, E>` throughout; no `.unwrap()` or `.expect()` in production
- [v] No `unsafe` blocks present
- [v] Proper `Send + Sync` bounds on `ManagedService` trait and Tower service implementations
- [v] No `Box<dyn Any>` type erasure where concrete types would work; the `DashMap<TypeId, Arc<dyn Any + Send + Sync>>` usage in `ServiceRegistry` is the correct pattern for type-erased lookup

**Architecture Check:**
- [v] Follows established module factory pattern from PROJECT.md
- [v] Operation routing pattern correctly documented in STATE.md project patterns
- [v] Dependencies flow correctly: `service/` depends on `topgun_core` types only; no circular deps
- [v] Proper separation of concerns: classify (classification), router (dispatch), middleware (cross-cutting), domain (stubs), worker (background)

**Non-Duplication Check:**
- [v] Uses existing `topgun_core::hash_to_partition()` for partition routing
- [v] Uses existing `topgun_core::Timestamp` and `HLC` types
- [v] Leverages existing `tower`, `dashmap`, `parking_lot` crate patterns
- [v] `domain_stub!` macro eliminates copy-paste across 7 services

**Cognitive Load Check:**
- [v] Code is clearly organized by domain with consistent patterns
- [v] Naming is clear and consistent (service_names constants, CallerOrigin variants, ClassifyError variants)
- [v] No unnecessary abstractions -- the `ServiceWrapper` in router.rs is the minimum needed for type erasure
- [v] Logic flow is easy to follow: Message -> classify -> Operation -> pipeline -> router -> domain stub

**Implementation Reality Check:**
- No strategic red flags detected. The implementation matches the spec's architectural vision precisely. The Tower middleware pattern, service registry, and operation routing are all standard Rust server patterns applied correctly.

**Summary:** The implementation is thorough, well-structured, and fully compliant with all 13 acceptance criteria. Code quality is high with clean separation of concerns, idiomatic Rust patterns, comprehensive test coverage (50 new tests), and zero clippy warnings. The only finding is a minor comment convention violation (phase reference in a code comment). All documented deviations from the spec are reasonable adaptations to actual struct definitions in `topgun_core`. The `domain_stub!` macro is a nice touch that eliminates significant boilerplate. The integration tests in `lib.rs` demonstrate the complete end-to-end pipeline flow.

---

## Completion

**Completed:** 2026-02-22
**Total Commits:** 6
**Audit Cycles:** 3
**Review Cycles:** 1
