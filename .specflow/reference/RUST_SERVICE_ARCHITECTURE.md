# Rust Service & Operation Architecture

**Status:** Design Reference (from RES-005)
**Created:** 2026-02-20
**Blocks:** TODO-065 (Operation Routing and Execution)
**Informed by:** Hazelcast SPI (WHAT), Quickwit+TiKV+Databend (HOW), TopGun TS Server (behavior)

---

## 1. Architecture Overview

TopGun's Rust server uses a **hybrid architecture** combining four patterns, each applied where it fits best:

| Pattern | Source | Used For |
|---------|--------|----------|
| **ServiceRegistry** | Hazelcast ServiceManager | Service lifecycle, interface-based discovery, shutdown ordering |
| **Tower Middleware** | Quickwit tower layers | Request pipeline: auth, timeout, partition routing, metrics, backpressure |
| **Operation Enum** | Hazelcast Operation (adapted) | Type-safe partition-routable operations with provenance |
| **Background Workers** | TiKV Worker/LazyWorker | GC, heartbeat, persistence flush, periodic tasks |

```
Client Message (MsgPack)
    |
    v
[WebSocket / HTTP Handler] -- axum extractors
    |
    v
[Message Decoder] -- deserialize to Message enum
    |
    v
[OperationService::dispatch()] -- classify into Operation
    |
    v
[Tower Middleware Stack]
    Auth -> Timeout -> PartitionRoute -> MigrationBarrier -> Metrics -> LoadShed
    |
    v
[Domain Service] -- impl tower::Service<DomainRequest>
    |                  CrdtService, SyncService, QueryService, etc.
    v
[Response Encoder] -- serialize to MsgPack
    |
    v
Client Response
```

---

## 2. ServiceRegistry

### 2.1 Trait Definitions

```rust
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

/// Lifecycle hooks for managed services. Inspired by Hazelcast's ManagedService.
#[async_trait]
pub trait ManagedService: Send + Sync + Any {
    /// Human-readable service name (e.g., "crdt", "sync", "query").
    fn name(&self) -> &'static str;

    /// Initialize the service. Called once after all services are registered.
    /// Services may look up other services via the registry during init.
    async fn init(&self, ctx: &ServiceContext) -> anyhow::Result<()>;

    /// Reset to initial state. Used during split-brain merge or force-start.
    async fn reset(&self) -> anyhow::Result<()>;

    /// Graceful shutdown. `terminate=true` means skip cleanup (process exit).
    async fn shutdown(&self, terminate: bool) -> anyhow::Result<()>;
}

/// Migration hooks for services that own partition data.
/// Inspired by Hazelcast's MigrationAwareService.
#[async_trait]
pub trait MigrationAwareService: ManagedService {
    /// Called before migration starts (on both source and destination).
    /// Migration blocks until this returns.
    async fn before_migration(&self, event: &PartitionMigrationEvent) -> anyhow::Result<()>;

    /// Called after migration succeeds on all participants.
    /// Must not fail -- exceptions are logged and suppressed.
    async fn commit_migration(&self, event: &PartitionMigrationEvent);

    /// Called if migration fails at any step.
    /// Must not fail -- exceptions are logged and suppressed.
    async fn rollback_migration(&self, event: &PartitionMigrationEvent);

    /// Prepare replication data for a partition being migrated.
    /// Returns serialized data that the destination will apply.
    async fn prepare_replication(
        &self,
        event: &PartitionReplicationEvent,
    ) -> anyhow::Result<Option<Vec<u8>>>;
}

/// Context passed to services during initialization.
pub struct ServiceContext {
    pub registry: Arc<ServiceRegistry>,
    pub config: Arc<ServerConfig>,
    pub node_id: String,
}
```

### 2.2 Registry Implementation

```rust
use std::any::TypeId;
use dashmap::DashMap;

/// Service metadata stored in the registry.
struct ServiceEntry {
    name: &'static str,
    service: Arc<dyn Any + Send + Sync>,
    is_core: bool,
}

/// Thread-safe service registry with lifecycle management.
/// Inspired by Hazelcast's ServiceManagerImpl.
pub struct ServiceRegistry {
    /// Services indexed by name (primary lookup).
    by_name: DashMap<&'static str, ServiceEntry>,
    /// Services indexed by TypeId (for type-safe get::<T>()).
    by_type: DashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Ordered list for init/shutdown sequencing.
    init_order: parking_lot::RwLock<Vec<&'static str>>,
}

impl ServiceRegistry {
    pub fn new() -> Self { /* ... */ }

    /// Register a service. Core services are registered first and cannot be replaced.
    pub fn register<S: ManagedService + 'static>(
        &self,
        service: Arc<S>,
        is_core: bool,
    ) {
        let name = service.name();
        self.by_name.insert(name, ServiceEntry {
            name,
            service: service.clone() as Arc<dyn Any + Send + Sync>,
            is_core,
        });
        self.by_type.insert(TypeId::of::<S>(), service as Arc<dyn Any + Send + Sync>);
        self.init_order.write().push(name);
    }

    /// Get a service by concrete type.
    pub fn get<S: 'static>(&self) -> Option<Arc<S>> {
        self.by_type
            .get(&TypeId::of::<S>())
            .and_then(|entry| entry.value().clone().downcast::<S>().ok())
    }

    /// Get a service by name.
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.by_name.get(name).map(|e| e.service.clone())
    }

    /// Find all services implementing MigrationAwareService.
    /// Used by PartitionService during migration.
    pub fn get_migration_aware(&self) -> Vec<Arc<dyn MigrationAwareService>> {
        // Implementation: iterate by_name, downcast to MigrationAwareService
        todo!()
    }

    /// Initialize all services in registration order.
    pub async fn init_all(&self, ctx: &ServiceContext) -> anyhow::Result<()> {
        let order = self.init_order.read().clone();
        for name in &order {
            if let Some(entry) = self.by_name.get(name) {
                // downcast to ManagedService and call init
                // ...
            }
        }
        Ok(())
    }

    /// Shutdown all services in reverse order. Core services last.
    pub async fn shutdown_all(&self, terminate: bool) {
        let mut order = self.init_order.read().clone();
        order.reverse();
        for name in &order {
            // downcast to ManagedService and call shutdown
            // ...
        }
    }
}
```

---

## 3. Operation Model

### 3.1 Operation Enum

Unlike Hazelcast's class hierarchy, TopGun uses a Rust enum for operations. This is more idiomatic and avoids dynamic dispatch overhead.

```rust
/// Provenance tracks where an operation originated.
/// Inspired by Hazelcast's caller UUID + our TS OperationContextHandler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerOrigin {
    /// Direct client request.
    Client,
    /// Forwarded from another node (partition routing).
    Forwarded,
    /// Backup replication from primary.
    Backup,
    /// WAN replication from remote cluster (future).
    Wan,
    /// Internal system operation (GC, migration, etc.).
    System,
}

/// Metadata attached to every operation.
/// Inspired by Hazelcast's Operation base class fields.
#[derive(Debug, Clone)]
pub struct OperationContext {
    /// Unique ID for request-response correlation.
    pub call_id: u64,
    /// Which partition this operation targets. None = generic operation.
    pub partition_id: Option<u32>,
    /// Which service handles this operation.
    pub service_name: &'static str,
    /// Who initiated this operation.
    pub caller_origin: CallerOrigin,
    /// Client ID (if caller_origin == Client).
    pub client_id: Option<String>,
    /// Node ID of the caller (if forwarded).
    pub caller_node_id: Option<String>,
    /// HLC timestamp of the operation.
    pub timestamp: topgun_core::Timestamp,
    /// Call timeout in milliseconds.
    pub call_timeout_ms: u64,
}

/// The core operation enum. Each variant maps to a message type.
/// Grouped by domain for clarity.
#[derive(Debug)]
pub enum Operation {
    // === CRDT Domain ===
    ClientOp {
        ctx: OperationContext,
        payload: topgun_core::messages::ClientOp,
    },
    OpBatch {
        ctx: OperationContext,
        ops: Vec<topgun_core::messages::ClientOp>,
    },

    // === Sync Domain ===
    SyncInit {
        ctx: OperationContext,
        map_name: String,
        map_type: topgun_core::MapType,
    },
    MerkleReqBucket {
        ctx: OperationContext,
        map_name: String,
        prefix: String,
    },
    // ... ORMap sync variants ...

    // === Query Domain ===
    QuerySubscribe {
        ctx: OperationContext,
        query: topgun_core::messages::Query,
    },
    QueryUnsubscribe {
        ctx: OperationContext,
        query_id: String,
    },

    // === Messaging Domain ===
    TopicSubscribe {
        ctx: OperationContext,
        topic: String,
    },
    TopicUnsubscribe {
        ctx: OperationContext,
        topic: String,
    },
    TopicPublish {
        ctx: OperationContext,
        topic: String,
        data: topgun_core::Value,
    },

    // === Coordination Domain ===
    LockRequest {
        ctx: OperationContext,
        lock_name: String,
        ttl_ms: u64,
    },
    LockRelease {
        ctx: OperationContext,
        lock_name: String,
        fencing_token: u64,
    },
    PartitionMapRequest {
        ctx: OperationContext,
    },

    // === Search Domain ===
    Search {
        ctx: OperationContext,
        map_name: String,
        query_text: String,
    },

    // === Persistence Domain ===
    // Journal, Counter, EntryProcessor, Resolver variants...

    // === Internal/System ===
    GarbageCollect {
        ctx: OperationContext,
        map_name: String,
        max_age_ms: u64,
    },
    Heartbeat {
        ctx: OperationContext,
    },
}

impl Operation {
    /// Get the operation context (always present).
    pub fn ctx(&self) -> &OperationContext {
        match self {
            Operation::ClientOp { ctx, .. } => ctx,
            Operation::OpBatch { ctx, .. } => ctx,
            // ... all variants
            _ => todo!(),
        }
    }

    /// Is this operation partition-specific?
    pub fn is_partition_specific(&self) -> bool {
        self.ctx().partition_id.is_some()
    }

    /// Get the service name that handles this operation.
    pub fn service_name(&self) -> &'static str {
        self.ctx().service_name
    }
}
```

### 3.2 Operation Classification

When a `Message` arrives, `OperationService` classifies it into an `Operation` with computed partition ID and service name:

```rust
impl OperationService {
    /// Classify an incoming message into an operation.
    pub fn classify(
        &self,
        message: topgun_core::messages::Message,
        client_id: &str,
        call_id: u64,
    ) -> Operation {
        match message {
            Message::ClientOp(payload) => {
                let partition_id = self.partition_service
                    .get_partition_id(&payload.key);
                Operation::ClientOp {
                    ctx: OperationContext {
                        call_id,
                        partition_id: Some(partition_id),
                        service_name: "crdt",
                        caller_origin: CallerOrigin::Client,
                        client_id: Some(client_id.to_string()),
                        caller_node_id: None,
                        timestamp: self.hlc.now(),
                        call_timeout_ms: self.default_timeout_ms,
                    },
                    payload,
                }
            }
            Message::TopicPublish(payload) => {
                // Topics are not partition-bound
                Operation::TopicPublish {
                    ctx: OperationContext {
                        call_id,
                        partition_id: None,  // generic operation
                        service_name: "messaging",
                        caller_origin: CallerOrigin::Client,
                        // ...
                    },
                    topic: payload.topic,
                    data: payload.data,
                }
            }
            // ... other message types
        }
    }
}
```

---

## 4. Tower Middleware Pipeline

### 4.1 Middleware Layers

Each layer is a `tower::Layer` that wraps a `tower::Service<Operation>`.

```rust
use tower::{Layer, Service, ServiceBuilder};

/// The full middleware stack applied to all operations.
pub fn build_operation_pipeline<S>(
    inner: S,
    config: &ServerConfig,
    partition_service: Arc<PartitionService>,
    hlc: Arc<HLC>,
) -> impl Service<Operation, Response = OperationResponse, Error = OperationError>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError>,
{
    ServiceBuilder::new()
        // Outermost: metrics (always runs, even on error)
        .layer(MetricsLayer::new())
        // Timeout: reject operations that wait too long
        .layer(TimeoutLayer::new(config.default_operation_timeout))
        // Auth: verify caller has permission
        .layer(AuthLayer::new(config.jwt_secret.clone()))
        // Backpressure: reject when overloaded
        .layer(LoadShedLayer::new(config.max_concurrent_operations))
        // Partition routing: forward to correct node if not local owner
        .layer(PartitionRoutingLayer::new(partition_service.clone()))
        // Migration barrier: block operations on migrating partitions
        .layer(MigrationBarrierLayer::new(partition_service.clone()))
        // Innermost: dispatch to domain service
        .service(inner)
}
```

### 4.2 PartitionRoutingLayer

The most TopGun-specific layer. For partition-bound operations, checks if the local node owns the partition and forwards if not.

```rust
/// Tower layer that routes partition-specific operations to the correct node.
/// Inspired by Hazelcast's OperationRunnerImpl.ensureNoPartitionProblems().
pub struct PartitionRoutingLayer {
    partition_service: Arc<PartitionService>,
}

impl<S> Layer<S> for PartitionRoutingLayer {
    type Service = PartitionRouting<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PartitionRouting {
            inner,
            partition_service: self.partition_service.clone(),
        }
    }
}

pub struct PartitionRouting<S> {
    inner: S,
    partition_service: Arc<PartitionService>,
}

impl<S> Service<Operation> for PartitionRouting<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError>,
{
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = /* ... */;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        if let Some(partition_id) = op.ctx().partition_id {
            let owner = self.partition_service.get_owner(partition_id);
            if !self.partition_service.is_local(&owner) {
                // Forward to the owning node
                return Box::pin(async move {
                    Err(OperationError::NotOwner {
                        partition_id,
                        owner_node_id: owner,
                    })
                    // The connection layer catches NotOwner and forwards
                });
            }
        }
        // Local execution
        self.inner.call(op)
    }
}
```

### 4.3 MigrationBarrierLayer

Blocks operations on partitions that are currently migrating.

```rust
/// Rejects operations on partitions that are migrating.
/// Inspired by Hazelcast's PartitionMigratingException.
pub struct MigrationBarrier<S> {
    inner: S,
    partition_service: Arc<PartitionService>,
}

impl<S> Service<Operation> for MigrationBarrier<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError>,
{
    // ...
    fn call(&mut self, op: Operation) -> Self::Future {
        if let Some(pid) = op.ctx().partition_id {
            if self.partition_service.is_migrating(pid) {
                // Readonly operations with stale reads enabled can proceed
                if !op.is_readonly() {
                    return Box::pin(async move {
                        Err(OperationError::PartitionMigrating { partition_id: pid })
                    });
                }
            }
        }
        self.inner.call(op)
    }
}
```

---

## 5. Domain Services

### 5.1 Service Trait

Each domain implements `tower::Service<Operation>` and filters for its own operations:

```rust
/// CRDT domain service handling ClientOp, OpBatch, GC operations.
pub struct CrdtService {
    hlc: Arc<HLC>,
    storage_manager: Arc<dyn MapProvider>,
    query_registry: Arc<QueryRegistry>,
    // ... other dependencies
}

impl ManagedService for CrdtService {
    fn name(&self) -> &'static str { "crdt" }

    async fn init(&self, ctx: &ServiceContext) -> anyhow::Result<()> {
        // Load maps from storage, initialize indices, etc.
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        // Clear in-memory state
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        // Flush pending writes
        Ok(())
    }
}

impl MigrationAwareService for CrdtService {
    async fn before_migration(&self, event: &PartitionMigrationEvent) -> anyhow::Result<()> {
        // Block writes to migrating partition
        Ok(())
    }

    async fn commit_migration(&self, event: &PartitionMigrationEvent) {
        // Remove data for partition we no longer own
    }

    async fn rollback_migration(&self, event: &PartitionMigrationEvent) {
        // Restore pre-migration state
    }

    async fn prepare_replication(
        &self,
        event: &PartitionReplicationEvent,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        // Serialize all data for the given partition
        Ok(None)
    }
}

impl Service<Operation> for CrdtService {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        match op {
            Operation::ClientOp { ctx, payload } => {
                let hlc = self.hlc.clone();
                let storage = self.storage_manager.clone();
                Box::pin(async move {
                    // Apply CRDT operation
                    // ...
                    Ok(OperationResponse::Ack { call_id: ctx.call_id })
                })
            }
            Operation::OpBatch { ctx, ops } => {
                // Process batch
                Box::pin(async move {
                    // ...
                    Ok(OperationResponse::BatchAck {
                        call_id: ctx.call_id,
                        last_id: ops.last().map(|o| o.id.clone()),
                    })
                })
            }
            _ => Box::pin(async { Err(OperationError::WrongService) }),
        }
    }
}
```

### 5.2 OperationRouter

Routes operations to the correct domain service:

```rust
/// Routes operations to domain services based on service_name.
pub struct OperationRouter {
    services: HashMap<&'static str, Box<dyn Service<
        Operation,
        Response = OperationResponse,
        Error = OperationError,
        Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>,
    > + Send>>,
}

impl Service<Operation> for OperationRouter {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let service_name = op.service_name();
        if let Some(svc) = self.services.get_mut(service_name) {
            svc.call(op)
        } else {
            Box::pin(async move {
                Err(OperationError::UnknownService {
                    name: service_name.to_string(),
                })
            })
        }
    }
}
```

---

## 6. Background Workers

### 6.1 Worker Pattern

For tasks that run independently of the request pipeline:

```rust
/// Simplified TiKV-style worker for background tasks.
pub struct BackgroundWorker<T: Send + 'static> {
    name: String,
    tx: tokio::sync::mpsc::Sender<T>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// What the worker actually does with tasks.
#[async_trait]
pub trait BackgroundRunnable: Send + 'static {
    type Task: Send + 'static;

    async fn run(&mut self, task: Self::Task);
    async fn on_tick(&mut self) {}
    async fn shutdown(&mut self) {}
}

impl<T: Send + 'static> BackgroundWorker<T> {
    pub fn new(name: impl Into<String>, buffer_size: usize) -> (Self, tokio::sync::mpsc::Receiver<T>) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
        (
            BackgroundWorker {
                name: name.into(),
                tx,
                handle: None,
            },
            rx,
        )
    }

    pub fn start<R: BackgroundRunnable<Task = T>>(&mut self, mut runner: R, mut rx: tokio::sync::mpsc::Receiver<T>) {
        let name = self.name.clone();
        self.handle = Some(tokio::spawn(async move {
            tracing::info!(worker = %name, "background worker started");
            while let Some(task) = rx.recv().await {
                runner.run(task).await;
            }
            runner.shutdown().await;
            tracing::info!(worker = %name, "background worker stopped");
        }));
    }

    pub fn scheduler(&self) -> tokio::sync::mpsc::Sender<T> {
        self.tx.clone()
    }

    pub async fn stop(self) {
        drop(self.tx); // close channel
        if let Some(handle) = self.handle {
            let _ = handle.await;
        }
    }
}
```

### 6.2 Worker Instances

```rust
// GC Worker
pub enum GcTask {
    CollectMap { map_name: String, max_age_ms: u64 },
    CollectAll { max_age_ms: u64 },
}

struct GcRunner { /* storage_manager, query_registry, etc. */ }

#[async_trait]
impl BackgroundRunnable for GcRunner {
    type Task = GcTask;
    async fn run(&mut self, task: GcTask) { /* ... */ }
}

// Heartbeat Worker
pub enum HeartbeatTask {
    CheckClients,
    BroadcastHeartbeat,
}

// Persistence Flush Worker
pub enum FlushTask {
    FlushMap { map_name: String },
    FlushAll,
}
```

---

## 7. TS Handler to Rust Operation Mapping

### 7.1 Complete Mapping Table

| TS Message Type | TS Handler | Rust Operation | Rust Service | Partition? |
|----------------|-----------|---------------|-------------|-----------|
| CLIENT_OP | OperationHandler | `Operation::ClientOp` | CrdtService | Yes |
| OP_BATCH | BatchProcessingHandler | `Operation::OpBatch` | CrdtService | Yes (per-op) |
| SYNC_INIT | LwwSyncHandler | `Operation::SyncInit` | SyncService | Yes |
| MERKLE_REQ_BUCKET | LwwSyncHandler | `Operation::MerkleReqBucket` | SyncService | Yes |
| ORMAP_SYNC_INIT | ORMapSyncHandler | `Operation::OrMapSyncInit` | SyncService | Yes |
| ORMAP_MERKLE_REQ_BUCKET | ORMapSyncHandler | `Operation::OrMapMerkleReqBucket` | SyncService | Yes |
| ORMAP_DIFF_REQUEST | ORMapSyncHandler | `Operation::OrMapDiffRequest` | SyncService | Yes |
| ORMAP_PUSH_DIFF | ORMapSyncHandler | `Operation::OrMapPushDiff` | SyncService | Yes |
| QUERY_SUB | QueryHandler | `Operation::QuerySubscribe` | QueryService | No (distributed) |
| QUERY_UNSUB | QueryHandler | `Operation::QueryUnsubscribe` | QueryService | No |
| TOPIC_SUB | TopicHandler | `Operation::TopicSubscribe` | MessagingService | No |
| TOPIC_UNSUB | TopicHandler | `Operation::TopicUnsubscribe` | MessagingService | No |
| TOPIC_PUB | TopicHandler | `Operation::TopicPublish` | MessagingService | No |
| LOCK_REQUEST | LockHandler | `Operation::LockRequest` | CoordinationService | Yes |
| LOCK_RELEASE | LockHandler | `Operation::LockRelease` | CoordinationService | Yes |
| PARTITION_MAP_REQUEST | PartitionHandler | `Operation::PartitionMapRequest` | CoordinationService | No |
| SEARCH | SearchHandler | `Operation::Search` | SearchService | No (distributed) |
| SEARCH_SUB | SearchHandler | `Operation::SearchSubscribe` | SearchService | No |
| SEARCH_UNSUB | SearchHandler | `Operation::SearchUnsubscribe` | SearchService | No |
| COUNTER_REQUEST | CounterHandlerAdapter | `Operation::CounterRequest` | PersistenceService | No |
| COUNTER_SYNC | CounterHandlerAdapter | `Operation::CounterSync` | PersistenceService | No |
| ENTRY_PROCESS | EntryProcessorAdapter | `Operation::EntryProcess` | PersistenceService | Yes |
| ENTRY_PROCESS_BATCH | EntryProcessorAdapter | `Operation::EntryProcessBatch` | PersistenceService | Yes |
| REGISTER_RESOLVER | ResolverHandler | `Operation::RegisterResolver` | PersistenceService | No |
| UNREGISTER_RESOLVER | ResolverHandler | `Operation::UnregisterResolver` | PersistenceService | No |
| LIST_RESOLVERS | ResolverHandler | `Operation::ListResolvers` | PersistenceService | No |
| JOURNAL_SUBSCRIBE | JournalHandler | `Operation::JournalSubscribe` | PersistenceService | No |
| JOURNAL_UNSUBSCRIBE | JournalHandler | `Operation::JournalUnsubscribe` | PersistenceService | No |
| JOURNAL_READ | JournalHandler | `Operation::JournalRead` | PersistenceService | No |

### 7.2 Service Consolidation

The 26 TS handlers consolidate into **7 Rust services** plus **3 background workers**:

| Rust Service | TS Handlers | Operation Count |
|-------------|------------|----------------|
| `CrdtService` | OperationHandler, BatchProcessingHandler, GCHandler | 3 |
| `SyncService` | LwwSyncHandler, ORMapSyncHandler | 6 |
| `QueryService` | QueryHandler, QueryConversionHandler | 2 |
| `MessagingService` | TopicHandler, BroadcastHandler | 3 |
| `CoordinationService` | LockHandler, PartitionHandler | 3 |
| `SearchService` | SearchHandler | 3 |
| `PersistenceService` | JournalHandler, CounterHandler, EntryProcessor, Resolver | 9 |
| **Background Workers** | | |
| `GcWorker` | GCHandler (periodic) | -- |
| `HeartbeatWorker` | HeartbeatHandler | -- |
| `FlushWorker` | PersistenceHandler | -- |

Connection management (WebSocketHandler, AuthHandler, ClientMessageHandler) is handled by the axum layer, not as operations.

---

## 8. Server Assembly

### 8.1 Startup Sequence

```rust
pub async fn start_server(config: ServerConfig) -> anyhow::Result<ServerHandle> {
    // Phase 1: Create registry and register core services
    let registry = Arc::new(ServiceRegistry::new());

    // Core services (cannot be replaced)
    let hlc = Arc::new(HLC::new(config.node_id.clone(), Box::new(SystemClock)));
    let partition_service = Arc::new(PartitionService::new(config.partition_count));

    // Phase 2: Create and register domain services
    let crdt_service = Arc::new(CrdtService::new(/* deps */));
    let sync_service = Arc::new(SyncService::new(/* deps */));
    let query_service = Arc::new(QueryService::new(/* deps */));
    let messaging_service = Arc::new(MessagingService::new(/* deps */));
    let coordination_service = Arc::new(CoordinationService::new(/* deps */));
    let search_service = Arc::new(SearchService::new(/* deps */));
    let persistence_service = Arc::new(PersistenceService::new(/* deps */));

    registry.register(crdt_service.clone(), false);
    registry.register(sync_service.clone(), false);
    registry.register(query_service.clone(), false);
    registry.register(messaging_service.clone(), false);
    registry.register(coordination_service.clone(), false);
    registry.register(search_service.clone(), false);
    registry.register(persistence_service.clone(), false);

    // Phase 3: Initialize all services (may cross-reference each other)
    let ctx = ServiceContext {
        registry: registry.clone(),
        config: Arc::new(config.clone()),
        node_id: config.node_id.clone(),
    };
    registry.init_all(&ctx).await?;

    // Phase 4: Build operation router + middleware pipeline
    let router = OperationRouter::new()
        .register("crdt", crdt_service)
        .register("sync", sync_service)
        .register("query", query_service)
        .register("messaging", messaging_service)
        .register("coordination", coordination_service)
        .register("search", search_service)
        .register("persistence", persistence_service);

    let pipeline = build_operation_pipeline(
        router,
        &config,
        partition_service.clone(),
        hlc.clone(),
    );

    // Phase 5: Start background workers
    let gc_worker = start_gc_worker(/* deps */);
    let heartbeat_worker = start_heartbeat_worker(/* deps */);
    let flush_worker = start_flush_worker(/* deps */);

    // Phase 6: Start network (axum + WebSocket)
    let operation_service = Arc::new(OperationService::new(pipeline, hlc.clone(), partition_service.clone()));
    let app = build_axum_app(operation_service, &config);
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    let server_handle = axum::serve(listener, app);

    // Phase 7: Start cluster (after network is ready)
    // ...

    Ok(ServerHandle { registry, server_handle, gc_worker, heartbeat_worker, flush_worker })
}
```

### 8.2 Graceful Shutdown

```rust
impl ServerHandle {
    pub async fn shutdown(self) {
        // 1. Stop accepting new connections
        // 2. Drain in-flight operations (with timeout)
        // 3. Stop background workers
        self.gc_worker.stop().await;
        self.heartbeat_worker.stop().await;
        self.flush_worker.stop().await;
        // 4. Shutdown services (reverse order: domain first, core last)
        self.registry.shutdown_all(false).await;
    }
}
```

---

## 9. Testing Strategy

### 9.1 Service Tests

Each domain service is independently testable because it implements `tower::Service`:

```rust
#[tokio::test]
async fn test_crdt_service_client_op() {
    let mut svc = CrdtService::new(/* test deps */);
    let op = Operation::ClientOp {
        ctx: test_ctx("crdt", Some(42)),
        payload: ClientOp { /* ... */ },
    };
    let response = svc.call(op).await.unwrap();
    assert!(matches!(response, OperationResponse::Ack { .. }));
}
```

### 9.2 Middleware Tests

Tower layers are testable in isolation:

```rust
#[tokio::test]
async fn test_partition_routing_forwards_non_local() {
    let mock_service = MockService::new();
    let partition_service = Arc::new(PartitionService::new(271));
    // Configure partition 42 to be owned by "other-node"

    let mut routing = PartitionRouting {
        inner: mock_service,
        partition_service,
    };

    let op = Operation::ClientOp {
        ctx: test_ctx("crdt", Some(42)),
        payload: ClientOp { /* ... */ },
    };

    let result = routing.call(op).await;
    assert!(matches!(result, Err(OperationError::NotOwner { .. })));
}
```

### 9.3 Integration Tests

Full pipeline tests with real services:

```rust
#[tokio::test]
async fn test_full_pipeline_client_op() {
    let pipeline = build_test_pipeline();
    let msg = Message::ClientOp(ClientOp { /* ... */ });
    let op = OperationService::classify(msg, "client-1", 1);
    let response = pipeline.oneshot(op).await.unwrap();
    // Verify CRDT state was updated
}
```

---

## 10. Migration Path

### 10.1 Phase 3 (Minimum Viable)

- `ServiceRegistry` with `ManagedService` only (no `MigrationAwareService` yet)
- `Operation` enum with all 29 message types
- `OperationRouter` with 7 domain services
- Tower pipeline: Timeout + Metrics + LoadShed
- No partition routing layer yet (single-node first)
- Background workers: GC only

### 10.2 Phase 3+ (Cluster)

- Add `PartitionRoutingLayer` and `MigrationBarrierLayer`
- Implement `MigrationAwareService` for CrdtService, SyncService, CoordinationService
- Add `CallerOrigin::Forwarded` and `CallerOrigin::Backup` handling
- Full `BackpressureRegulator` integration

### 10.3 Phase 4-5 (Extensions)

- `ServiceRegistry` supports runtime extension registration (TODO-036)
- `ManagedService::reset()` used for split-brain merge recovery
- Additional Tower layers: RetryLayer (WAN replication), AuthLayer (per-tenant)
- Schema validation as a Tower layer

---

## 11. Design Decisions Summary

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Operation representation | Enum, not trait | Rust enums are zero-cost, exhaustive, and avoid dynamic dispatch. Hazelcast's class hierarchy reflects Java patterns, not Rust idioms. |
| Middleware framework | Tower | Ecosystem standard. Quickwit demonstrates it works for similar use cases. |
| Service lifecycle | Custom `ManagedService` trait | Neither Tower nor tokio provide lifecycle hooks. Hazelcast's 3-method interface (init/reset/shutdown) is proven. |
| Service discovery | TypeId + name-based | Hazelcast uses Class-based. Rust's TypeId is the equivalent. Name-based lookup needed for dynamic dispatch. |
| Background tasks | Dedicated workers (not actors) | TopGun's background tasks (GC, heartbeat) are simple periodic loops. Actor framework is overkill. TiKV's pattern is minimal and sufficient. |
| Partition routing | Tower layer | Cross-cutting concern that applies to all partition-bound operations. Must not be duplicated per-handler. |
| Backpressure | Semaphore-based LoadShed | Quickwit's pattern. Simple, effective, no per-partition tracking needed initially (Phase 3). Hazelcast's per-partition backpressure can be added later. |
| Request-response | Direct async | No actor mailbox overhead. `tower::Service::call()` returns a Future directly. |

---

*Architecture reference for TODO-065 (Operation Routing and Execution). Informed by RES-005 research. Spec creators should consult this document alongside Hazelcast SPI code and Quickwit Tower layers when writing implementation specs.*
