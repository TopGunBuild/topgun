---
id: RES-006
topic: Networking Layer Patterns for Rust IMDG Server
created: 2026-02-20
status: complete
blocks: TODO-064 (Networking Layer)
---

# Research: Networking Layer Patterns for Rust IMDG Server

## Summary

TopGun's Rust server should use axum natively (no protocol-agnostic Channel trait) for Phase 3, with a thin `ConnectionHandle` abstraction layered on top for per-connection state and backpressure. A bounded mpsc channel per outgoing WebSocket connection provides natural slow-client handling without custom flow control. Tower middleware should be ordered Metrics > LoadShed > Auth > Timeout > PartitionRouting (outermost to innermost). gRPC for inter-node communication can be added in Phase 4+ using tonic on a separate port, sharing Tower middleware with axum via the common `tower::Service` trait.

## Background

TopGun needs a networking layer that supports:
1. **Client connections** (10K+ concurrent WebSocket connections for browser/app sync)
2. **Inter-node cluster connections** (WebSocket mesh between server nodes)
3. **HTTP endpoints** (health checks, HTTP sync)
4. **Future extensibility** to gRPC (inter-node) and potentially QUIC (mobile clients)

The TS server has a basic networking implementation (`packages/server/src/modules/network-module.ts`) with no backpressure, no per-connection flow control, and a simple `ConnectionManager` wrapping a `Map<string, ClientConnection>`. The Rust server needs to be designed for 10x the scale from day one.

Key questions this research addresses:
- Should TopGun abstract over transport protocols (like Hazelcast's `Channel` interface)?
- How should per-connection state be managed at 10K+ scale?
- What backpressure strategy prevents slow clients from consuming unbounded memory?
- What is the correct Tower middleware ordering?
- How should graceful shutdown work with connection draining?

---

## 1. Connection Abstraction: Options Explored

### Option A: Protocol-Agnostic Channel Trait (Hazelcast Pattern)

Hazelcast's `Channel` interface (`internal/networking/Channel.java`) abstracts over the transport layer:

```java
public interface Channel extends Closeable {
    ChannelOptions options();
    ConcurrentMap attributeMap();
    InboundPipeline inboundPipeline();
    OutboundPipeline outboundPipeline();
    boolean write(OutboundFrame frame);
    long lastReadTimeMillis();
    long lastWriteTimeMillis();
    void start();
    void close();
    boolean isClosed();
    void addCloseListener(ChannelCloseListener listener);
    boolean isClientMode();
}
```

This is transport-agnostic: `NioChannel` (TCP), `TlsChannel`, and theoretically UDP channels all implement the same interface. Hazelcast uses this because they need to support raw TCP, TLS, and multi-protocol negotiation.

**Pros:**
- Protocol-agnostic: gRPC, QUIC, raw TCP all fit behind the same trait
- Clean separation of transport from application logic
- Hazelcast proves it works at enterprise scale

**Cons:**
- Significant upfront complexity for TopGun (we only need WebSocket in Phase 3)
- Hazelcast's `Channel` carries NIO-specific concepts (SocketChannel, Selector) that do not map to tokio
- Axum already provides excellent abstractions via extractors and handlers
- The `InboundPipeline`/`OutboundPipeline` pattern is a Java NIO concern, not a Rust async concern
- Requires implementing all protocol adapters from scratch

### Option B: Axum-Native with Thin ConnectionHandle (Recommended)

Use axum's native abstractions (extractors, State, WebSocket) and layer a thin `ConnectionHandle` on top for per-connection state management and outbound message delivery.

```rust
/// Per-connection handle for sending messages and tracking state.
/// This is NOT a transport abstraction -- it wraps a specific outbound
/// channel (tokio mpsc::Sender) and connection metadata.
pub struct ConnectionHandle {
    /// Unique connection identifier.
    pub id: ConnectionId,
    /// Bounded channel for outbound messages (backpressure).
    pub tx: mpsc::Sender<OutboundMessage>,
    /// Connection metadata (auth state, subscriptions, etc.).
    pub metadata: Arc<RwLock<ConnectionMetadata>>,
    /// When the connection was established.
    pub connected_at: Instant,
    /// Connection kind (client or cluster peer).
    pub kind: ConnectionKind,
}
```

**Pros:**
- Leverages axum's battle-tested WebSocket handling
- Thin abstraction: only wraps what TopGun needs (outbound channel + metadata)
- No impedance mismatch with tokio's async model
- axum extractors provide ergonomic access to connection state
- gRPC can be added later via tonic on a separate port (same Tower middleware)
- Minimal code to write in Phase 3

**Cons:**
- Not protocol-agnostic: switching transport requires changing the handler layer
- When gRPC is added (Phase 4+), inter-node connections use a different code path

### Option C: Unified Tower Service Abstraction

Define all protocols as `tower::Service<Request, Response=Response>` and compose them through Tower's `ServiceBuilder`.

**Pros:**
- Maximum composability via Tower
- Middleware is truly shared across all protocols

**Cons:**
- Over-engineering for Phase 3 (only WebSocket needed)
- Tower's `Service` trait requires `poll_ready` which adds complexity to WebSocket streams
- The Operation pipeline in `RUST_SERVICE_ARCHITECTURE.md` already uses Tower for the *operation* layer -- duplicating it for the *transport* layer adds no value

### Recommendation: Option B

**Reasoning:**
1. Hazelcast's `Channel` solves a Java NIO problem that does not exist in async Rust (tokio handles multiplexing, not the application)
2. TopGun only needs WebSocket for Phase 3; gRPC (Phase 4+) and QUIC (future) can be separate entry points that feed into the shared Operation pipeline
3. The `RUST_SERVICE_ARCHITECTURE.md` already defines Tower middleware at the *operation* level, which is protocol-agnostic by design -- the transport layer does not need its own abstraction
4. Axum's ecosystem (extractors, State, middleware) provides more than enough abstraction without a custom trait

**Future-proofing for gRPC/QUIC:**
- When gRPC is added (Phase 4+), tonic runs on a separate port
- Both axum (HTTP/WS) and tonic (gRPC) decode incoming messages into the same `Message` enum
- Both feed into `OperationService::classify()` -> Tower middleware pipeline -> domain services
- The protocol-agnostic boundary is the Operation enum, NOT the transport layer

```
Client (WS)  ──> axum handler ──> Message decode ──> OperationService ──> Tower pipeline
Cluster (WS) ──> axum handler ──> ClusterMessage decode ──> OperationService ──> Tower pipeline
Cluster (gRPC, Phase 4) ──> tonic handler ──> ClusterMessage decode ──> OperationService ──> Tower pipeline
```

---

## 2. Per-Connection State Management

### Current TS Pattern

The TS server uses `ConnectionManager` with a simple `Map<string, ClientConnection>`:

```typescript
// packages/server/src/coordinator/connection-manager.ts
interface ClientConnection {
    id: string;
    socket: WebSocket;
    writer: CoalescingWriter;
    isAuthenticated: boolean;
    subscriptions: Set<string>;
    lastActiveHlc: Timestamp;
    lastPingReceived: number;
    principal?: Principal;
}
```

This works for single-threaded Node.js but does not scale to multi-threaded Rust.

### Rust Design: DashMap + Arc<ConnectionHandle>

```rust
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Unique identifier for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Whether this is a client (browser/app) or cluster peer connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Client,
    ClusterPeer,
}

/// Mutable metadata for a connection.
/// Protected by RwLock for concurrent read access from broadcast operations.
pub struct ConnectionMetadata {
    /// Whether the client has completed authentication.
    pub authenticated: bool,
    /// Auth principal (set after successful auth).
    pub principal: Option<Principal>,
    /// Active query subscriptions for this connection.
    pub subscriptions: HashSet<String>,
    /// Active topic subscriptions.
    pub topics: HashSet<String>,
    /// Last received heartbeat time (for idle detection).
    pub last_heartbeat: Instant,
    /// Client's last known HLC timestamp.
    pub last_hlc: Option<Timestamp>,
    /// Node ID (for cluster peer connections).
    pub peer_node_id: Option<String>,
}

/// Thread-safe connection registry.
/// Uses DashMap for lock-free concurrent access from:
///   - WebSocket handler tasks (per-connection)
///   - Broadcast operations (iterate all connections)
///   - Heartbeat checker (periodic sweep)
///   - Graceful shutdown (drain all)
pub struct ConnectionRegistry {
    connections: DashMap<ConnectionId, Arc<ConnectionHandle>>,
    next_id: AtomicU64,
}
```

**Why DashMap (not `RwLock<HashMap>`):**
- 10K+ connections means frequent concurrent reads (broadcasts) and writes (connect/disconnect)
- DashMap's sharded locking means broadcast operations do not block new connections
- Consistent with the pattern used in `PartitionTable` and `ClusterState` (from `RUST_CLUSTER_ARCHITECTURE.md`)

**Memory budget at 10K connections:**
- `ConnectionHandle` ~256 bytes (id, sender, Arc<RwLock<metadata>>, instant, kind)
- `ConnectionMetadata` ~512 bytes (sets, strings, optional fields)
- DashMap overhead ~64 bytes per entry
- Total: ~8 MB for 10K connections -- trivial

### Per-Connection Task Structure

```
For each WebSocket connection:
    1. axum handler upgrades HTTP -> WebSocket
    2. Split WebSocket into sender + receiver (axum::extract::ws)
    3. Spawn outbound task: reads from mpsc::Receiver, writes to WS sender
    4. Inbound loop: reads from WS receiver, decodes MsgPack, dispatches to OperationService
    5. On disconnect: remove from ConnectionRegistry, clean up subscriptions
```

This is the standard pattern from the axum WebSocket example and matches SurrealDB's approach for per-connection session isolation.

---

## 3. Backpressure Strategy

### Problem

Without backpressure, a slow client (or a client on a degraded network) causes the server to buffer messages indefinitely, leading to OOM. The TS server has no backpressure -- `CoalescingWriter` buffers in memory without bounds.

### Option A: Bounded mpsc Channel Per Connection (Recommended)

Each `ConnectionHandle` has a bounded `mpsc::Sender<OutboundMessage>`. When the channel fills up, `try_send()` fails, and the server takes action.

```rust
/// Outbound message destined for a client.
pub enum OutboundMessage {
    /// Serialized MsgPack bytes ready to send.
    Binary(Vec<u8>),
    /// Close the connection with a reason.
    Close(Option<String>),
}

impl ConnectionHandle {
    /// Attempt to send a message to this connection.
    /// Returns false if the connection's outbound buffer is full.
    pub fn try_send(&self, msg: OutboundMessage) -> bool {
        self.tx.try_send(msg).is_ok()
    }

    /// Send a message, waiting for capacity (with timeout).
    pub async fn send_timeout(
        &self,
        msg: OutboundMessage,
        timeout: Duration,
    ) -> Result<(), SendError> {
        tokio::time::timeout(timeout, self.tx.send(msg))
            .await
            .map_err(|_| SendError::Timeout)?
            .map_err(|_| SendError::Disconnected)
    }
}
```

**Slow client policy:** When `try_send()` fails:
- **For subscription updates:** Drop the message (client will catch up via Merkle sync)
- **For request responses:** Wait with timeout; if timeout expires, close the connection
- **For partition map pushes:** Wait briefly; these are critical for routing correctness

**Channel capacity:** 256 messages per connection. This provides ~2 seconds of buffer at 128 msg/sec (typical subscription throughput for an active client).

**Pros:**
- Natural backpressure via tokio's built-in channel semantics
- No custom flow control implementation needed
- Memory bounded: 256 messages * avg ~1KB = ~256KB per connection = ~2.5GB at 10K connections (acceptable)
- Slow client detection is automatic (repeated `try_send()` failures)

**Cons:**
- Coarse-grained: all message types share the same channel capacity
- Dropped messages require client-side reconciliation (Merkle sync handles this)

### Option B: Per-Connection Rate Limiter (tower-governor)

Apply rate limiting at the connection level using token bucket or leaky bucket.

**Pros:**
- Fine-grained control over message rates
- Industry-standard pattern

**Cons:**
- Overkill for TopGun: the issue is slow *consumption*, not fast *production*
- Rate limiting constrains the server's output, which is backwards (we want to constrain slow clients, not throttle the server)
- Does not prevent buffer growth -- it just slows down the server

### Option C: Write Buffer Size Limit (axum native)

Axum's `WebSocketUpgrade` supports `max_write_buffer_size` for backpressure when writes to the underlying stream fail.

**Pros:**
- Zero custom code
- Built into axum

**Cons:**
- Only protects against TCP buffer exhaustion, not application-level slow consumption
- Does not provide message-level backpressure (operates at byte level)
- Cannot selectively drop messages by priority

### Recommendation: Option A (bounded mpsc) with Option C as supplementary

Use bounded mpsc channels (Option A) as the primary backpressure mechanism. Configure axum's `max_write_buffer_size` (Option C) as a safety net for TCP-level issues.

**Configuration:**
```rust
pub struct ConnectionConfig {
    /// Maximum outbound messages queued per connection.
    pub outbound_channel_capacity: usize,  // Default: 256
    /// Timeout for waiting on a full outbound channel.
    pub send_timeout: Duration,            // Default: 5s
    /// Maximum idle time before disconnecting.
    pub idle_timeout: Duration,            // Default: 60s
    /// axum WebSocket write buffer size.
    pub ws_write_buffer_size: usize,       // Default: 128KB
    /// axum WebSocket max write buffer (backpressure limit).
    pub ws_max_write_buffer_size: usize,   // Default: 512KB
}
```

---

## 4. Tower Middleware Ordering

### Existing Design (from RUST_SERVICE_ARCHITECTURE.md)

The service architecture document defines middleware at the *operation* level:

```
Metrics -> Timeout -> Auth -> LoadShed -> PartitionRouting -> MigrationBarrier -> Inner
```

### Revised Ordering

After researching Quickwit and other projects, the recommended ordering is adjusted. The key principle is: **outermost layers run on every request (even errors), innermost layers only run on valid requests**.

#### HTTP/WebSocket Layer (axum middleware)

These are axum-level middleware, applied before the message reaches the Operation pipeline:

```
1. RequestId         (assign unique ID for tracing)
2. Tracing           (structured logging with request ID)
3. Compression       (for HTTP responses, not WebSocket)
4. CorsLayer         (for HTTP endpoints)
5. TimeoutLayer      (global request timeout, prevents hung connections)
```

#### Operation Layer (Tower middleware on Operation)

These are applied after message decoding, within the `OperationService` pipeline:

```
1. MetricsLayer           (outermost -- always runs, records all operations)
2. LoadShedLayer          (reject when overloaded -- semaphore-based)
3. AuthLayer              (verify caller permissions)
4. TimeoutLayer           (per-operation timeout, shorter than connection timeout)
5. PartitionRoutingLayer  (forward to correct node if not local owner)
6. MigrationBarrierLayer  (block writes on migrating partitions)
7. OperationRouter        (innermost -- dispatch to domain service)
```

**Changes from original design:**

| Change | Reason |
|--------|--------|
| LoadShed moved before Auth | Reject overloaded requests BEFORE spending CPU on auth verification |
| Metrics moved to outermost | Must record rejected (shed) requests for monitoring |
| CorsLayer at HTTP level only | WebSocket connections negotiate CORS during upgrade, not per-message |
| RequestId added | SurrealDB pattern: every request gets a UUID for distributed tracing |

### Quickwit Patterns Adopted

Quickwit's serve module demonstrates a consistent Tower layering pattern:
- **Client layers:** Metrics -> Retry -> Timeout -> Concurrency Limit -> Rate Limiting
- **Server layers:** Metrics -> Load Shedding -> Circuit Breaking -> Event Listeners

TopGun adopts:
- **Metrics as outermost** (from both Quickwit client and server patterns)
- **Load shedding before auth** (from Quickwit server pattern)
- **No retry layer** (TopGun clients handle retry; server does not retry operations)
- **No circuit breaking** (TopGun uses partition routing instead; a failing partition is migrated, not circuit-broken)

---

## 5. Graceful Shutdown

### Requirements

1. Stop accepting new connections
2. Drain in-flight operations with timeout
3. Close WebSocket connections cleanly (send Close frame)
4. Shut down background workers
5. Flush persistence
6. Leave cluster (migrate partitions away)
7. Health check transitions: `ready -> draining -> not-ready`

### Design

```rust
pub struct ShutdownController {
    /// Signal to stop accepting new connections.
    shutdown_signal: tokio::sync::watch::Sender<bool>,
    /// Tracks in-flight operations for drain.
    in_flight: Arc<AtomicU64>,
    /// Health state for readiness probes.
    health_state: Arc<ArcSwap<HealthState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Starting up, not ready to serve.
    Starting,
    /// Fully operational.
    Ready,
    /// Shutting down, draining in-flight operations.
    Draining,
    /// Shutdown complete.
    Stopped,
}
```

**Shutdown sequence (adapted from Quickwit's shutdown choreography):**

```
1. Receive shutdown signal (SIGTERM/SIGINT)
2. Set health_state = Draining
3. Stop accepting new WebSocket connections (shutdown_signal.send(true))
4. Wait for in_flight operations to reach 0 (with 30s timeout)
5. For each client connection:
   a. Send Close frame with reason "server shutting down"
   b. Wait up to 5s for client Close acknowledgment
   c. Force-close if no ack
6. Decommission from cluster:
   a. Notify master: LeaveRequest
   b. Wait for partition migrations (with timeout)
7. Stop background workers (GC, heartbeat, flush)
8. Flush all dirty records to PostgreSQL (hard_flush)
9. Shutdown services in reverse registration order
10. Set health_state = Stopped
```

**Health check endpoints:**

```
GET /health/live    -> 200 (always, unless process is dead)
GET /health/ready   -> 200 if Ready, 503 if Starting/Draining/Stopped
GET /health         -> { state, connections, in_flight_ops, uptime }
```

This matches Kubernetes liveness/readiness probe conventions.

---

## 6. Deferred Startup Pattern

### Current TS Pattern

The TS server uses deferred startup: `network.start()` is called after all modules are assembled.

```typescript
// packages/server/src/modules/network-module.ts
return {
    start: () => new Promise<number>((resolve) => {
        httpServer.listen(config.port, () => {
            resolve(actualPort);
        });
    }),
};
```

### Rust Design

```rust
pub struct NetworkModule {
    /// Configuration for the network layer.
    config: NetworkConfig,
    /// TCP listener (NOT bound yet -- deferred).
    listener: Option<TcpListener>,
    /// Shared application state for axum handlers.
    app_state: Arc<AppState>,
}

impl NetworkModule {
    /// Create the module (assembly phase). Does NOT bind to port.
    pub fn new(config: NetworkConfig, app_state: Arc<AppState>) -> Self {
        Self { config, listener: None, app_state }
    }

    /// Build the axum Router (can be called during assembly for testing).
    pub fn build_router(&self) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/health/live", get(liveness_handler))
            .route("/health/ready", get(readiness_handler))
            .route("/sync", post(http_sync_handler))
            .route("/ws", get(ws_upgrade_handler))
            .with_state(self.app_state.clone())
            .layer(/* HTTP middleware stack */)
    }

    /// Start listening (deferred startup). Returns actual port.
    pub async fn start(&mut self) -> anyhow::Result<u16> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));
        let listener = TcpListener::bind(addr).await?;
        let actual_port = listener.local_addr()?.port();
        self.listener = Some(listener);
        Ok(actual_port)
    }

    /// Serve requests (blocks until shutdown signal).
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
        let listener = self.listener.expect("start() must be called before serve()");
        let app = self.build_router();

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}
```

**Key differences from TS:**
- `start()` binds the port but does NOT serve (allows port capture for cluster config)
- `serve()` is a separate call that blocks until shutdown signal
- `port: 0` correctly captures OS-assigned port (avoiding the `config.clusterPort || 0` bug from TS)

---

## 7. TLS Support

### Design

Use `axum-server` with `rustls` for TLS termination:

```rust
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_cert_path: Option<PathBuf>,
    pub min_version: TlsVersion,  // Default: TLS 1.2
}

// For client connections: rustls with axum-server
// For cluster connections: separate rustls config (may use mutual TLS)
```

**Phase 3:** Single TLS config for all connections (client + cluster on same port).
**Phase 4+:** Separate TLS configs for client and cluster connections when they move to separate ports.

---

## 8. Codebase Findings

### TS Server Networking

- `packages/server/src/modules/network-module.ts` -- Deferred startup, HTTP + WebSocket creation, TLS support, rate limiting, socket-level configuration (NoDelay, KeepAlive)
- `packages/server/src/coordinator/connection-manager.ts` -- Simple `Map<string, ClientConnection>` with heartbeat tracking, no backpressure
- `packages/server/src/coordinator/websocket-handler.ts` -- WebSocket lifecycle: rate limit check, client registration, MsgPack deserialization (with JSON fallback), auth state machine, message routing via registry, disconnect cleanup
- `packages/server/src/utils/BackpressureRegulator.ts` -- Global (NOT per-connection) backpressure based on pending operation count. Sync window + capacity limiting. Not tied to connection management.
- `packages/server/src/modules/types.ts` -- Module interface definitions including `NetworkModule`, `NetworkModuleConfig`

### Rust Server (Existing)

- `packages/server-rust/src/traits.rs` -- 3 traits (ServerStorage, MapProvider, SchemaProvider). No networking traits yet.
- `packages/server-rust/src/lib.rs` -- Minimal crate, just re-exports traits

### Reference Architecture Documents

- `.specflow/reference/RUST_SERVICE_ARCHITECTURE.md` -- Defines Tower middleware pipeline for Operation routing. The `build_operation_pipeline()` function establishes: Metrics > Timeout > Auth > LoadShed > PartitionRouting > MigrationBarrier > Inner. Also defines `OperationService::classify()` which converts `Message` to `Operation`.
- `.specflow/reference/RUST_CLUSTER_ARCHITECTURE.md` -- Defines `ClusterMessage` enum for inter-node communication. Cluster connections use WebSocket. `ClusterChannels` struct with tokio channels for internal communication.
- `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` -- Three-layer storage model with `RecordStore` factory pattern.

### Hazelcast Networking

- `hazelcast/internal/networking/Channel.java` -- Protocol-agnostic channel with inbound/outbound pipelines, attribute map, close listeners. Comments note: "Flow and congestion control: On the Channel level we have no flow or congestion control; frames are always accepted."
- `hazelcast/internal/networking/nio/NioChannel.java` -- NIO implementation: inbound/outbound pipeline separation, drain write queues on close, close listener executor offloading
- `hazelcast/internal/nio/Connection.java` -- Higher-level connection interface: alive check, read/write timestamps, remote UUID, write/writeOrdered, close with reason

**Key Hazelcast observation:** Hazelcast's `Channel` explicitly acknowledges it has NO flow control (see the comment in `Channel.java`). This is a known limitation they intend to fix. TopGun should not replicate this limitation -- bounded channels provide what Hazelcast lacks.

---

## 9. Reference Project Patterns

### SurrealDB

- **Multi-protocol support:** HTTP REST + WebSocket (RPC) on the same port. Connection protocol negotiated at upgrade time.
- **Per-connection session:** Each WebSocket connection maintains a stateful session with auth context, namespace/database selection. Session state is NOT shared between connections.
- **WebSocket limits:** Configurable (max message size, buffer capacity). Recent fixes for memory leaks in WebSocket implementation.
- **Channel capacity:** Bounded channels used internally for routing responses, with configurable HashMap capacity for request-response correlation.
- **Applicable to TopGun:** Per-connection session isolation pattern. Each TopGun client connection should have isolated auth state, subscription set, and HLC tracking.

### Quickwit

- **Tower middleware composition:** Consistent layering: Metrics (outermost) -> LoadShed -> CircuitBreaker -> EventListener. GrpcMetricsLayer is lazily initialized per service.
- **Startup choreography:** Careful ordering: cluster init -> conditional service startup -> pool init -> gRPC/REST servers -> readiness reporting.
- **Graceful shutdown:** Signal handler -> ingester decommission -> actor universe shutdown -> server shutdown signals -> cluster departure.
- **Health/readiness:** `node_readiness_reporting_task()` reports readiness every 10s, waiting for metastore connectivity before marking ready.
- **Applicable to TopGun:** Startup ordering, shutdown choreography, readiness reporting pattern. TopGun should adopt the "report readiness periodically" approach rather than a single transition.

### Grafbase

- **Extension model:** WebAssembly-based extensions compiled to WASI components for custom auth and resolvers.
- **Lock-free execution:** DAG-based query execution with Steiner tree planner.
- **Applicable to TopGun:** Limited applicability. Grafbase is a GraphQL gateway, not an IMDG. The WASM extension concept aligns with TopGun's WASM strategy (TODO-072) but is not networking-relevant.

### Hyperswitch (Unified gRPC + REST)

- **Approach:** Single API definition in Protobuf, auto-generate both tonic (gRPC) and axum (REST) endpoints.
- **Code generation:** Custom `prost_build::Config` service generator hooks into build process to create typed axum routes mirroring tonic services.
- **Applicable to TopGun:** The *concept* of shared service definitions across protocols is relevant. TopGun's `Message` enum already serves this purpose -- both WebSocket and future gRPC endpoints decode to the same `Message` type.

---

## 10. Trade-offs Analysis

### Connection Abstraction

| Aspect | Option A: Channel Trait | Option B: axum-native + ConnectionHandle | Option C: Unified Tower |
|--------|------------------------|------------------------------------------|------------------------|
| Phase 3 complexity | High | **Low** | Medium |
| Future extensibility | **High** (any protocol) | Medium (new entry points) | **High** |
| Code to write | ~1500 LOC | **~500 LOC** | ~800 LOC |
| axum ecosystem compat | Low (custom) | **High** (native) | Medium |
| Protocol migration cost | **Low** (swap impl) | Medium (new handler) | **Low** |
| Alignment with existing arch | Low | **High** (Operation pipeline is the abstraction) | Medium |

### Backpressure Strategy

| Aspect | Bounded mpsc | Rate Limiter | WS Buffer Limit |
|--------|-------------|--------------|-----------------|
| Slow client detection | **Automatic** | Manual threshold | No |
| Memory bound | **Yes** (channel capacity) | No (still buffers) | Partial (byte-level) |
| Message priority | No (FIFO) | No | No |
| Implementation effort | **Trivial** (tokio built-in) | Medium (tower-governor) | **Trivial** (axum config) |
| Drop policy | **Configurable** | N/A | Implicit |

### Middleware Ordering

| Order | Scenario: Overloaded Server | Scenario: Unauthenticated Request | Scenario: Wrong Partition |
|-------|---------------------------|----------------------------------|--------------------------|
| Metrics first | Counts rejected requests | Counts auth failures | Counts routing |
| LoadShed before Auth | **Rejects fast** without auth CPU | N/A | N/A |
| Auth before LoadShed | Wastes CPU on auth before rejecting | Rejects sooner | N/A |
| PartitionRouting last | N/A | N/A | **Correctly forwards after auth** |

---

## 11. Recommendations

### Recommended Approach

**Connection management:** axum-native with `ConnectionHandle` (Option B)
**Backpressure:** Bounded mpsc channel (256 messages) + axum write buffer limit (512KB)
**Middleware ordering:** Metrics > LoadShed > Auth > Timeout > PartitionRouting > MigrationBarrier
**Graceful shutdown:** Signal -> Drain -> Close connections -> Leave cluster -> Flush -> Stop services
**Deferred startup:** `NetworkModule::start()` binds port, `NetworkModule::serve()` starts serving
**TLS:** rustls via axum-server, single config Phase 3, separate client/cluster configs Phase 4+
**Future gRPC:** Separate tonic server on cluster port (Phase 4+), shared Operation pipeline

### Reasoning

1. **The Operation pipeline IS the protocol abstraction.** All transports (WS, HTTP, gRPC) decode to `Message`/`ClusterMessage`, classify into `Operation`, and enter the Tower pipeline. Adding a transport-level abstraction below this would be a second abstraction layer with no benefit.

2. **Bounded mpsc is the simplest correct backpressure.** tokio's channel semantics are well-tested, and the pattern (one sender per broadcast, one receiver per connection task) is idiomatic. TopGun's CRDT foundation means dropped subscription messages are self-healing via Merkle sync.

3. **LoadShed before Auth saves CPU under load.** When the server is overloaded, rejecting requests before auth verification saves the most CPU. An attacker sending many requests is shed before consuming any auth resources.

4. **Deferred startup matches TS pattern and enables port: 0.** The TS server's deferred startup is well-tested and allows OS-assigned ports. The Rust version should mirror this but fix the `port || 0` falsy bug.

### Implementation Notes

1. **File organization:**
   ```
   packages/server-rust/src/
   ├── network/
   │   ├── mod.rs                 # Re-exports
   │   ├── config.rs              # NetworkConfig, TlsConfig, ConnectionConfig
   │   ├── connection.rs          # ConnectionHandle, ConnectionMetadata, ConnectionRegistry
   │   ├── handlers/
   │   │   ├── mod.rs
   │   │   ├── websocket.rs       # ws_upgrade_handler, handle_socket
   │   │   ├── health.rs          # health, liveness, readiness endpoints
   │   │   └── http_sync.rs       # POST /sync handler
   │   ├── middleware.rs           # HTTP-level tower middleware stack
   │   ├── shutdown.rs            # ShutdownController, HealthState
   │   └── module.rs              # NetworkModule (deferred startup)
   ```

2. **Crate dependencies to add:**
   ```toml
   axum = { version = "0.8", features = ["ws"] }
   axum-server = { version = "0.7", features = ["tls-rustls"] }  # TLS
   tokio-tungstenite = "0.24"    # WebSocket protocol (used by axum internally)
   tower = "0.5"
   tower-http = { version = "0.6", features = ["trace", "cors", "timeout", "request-id"] }
   dashmap = "6"                 # ConnectionRegistry
   ```

3. **Per-connection task budget:** At 10K connections, this means ~10K tokio tasks for inbound processing + ~10K tasks for outbound message writing = ~20K tasks. tokio handles this comfortably (its work-stealing scheduler is designed for millions of tasks).

4. **Cluster connections reuse the same `ConnectionHandle` with `kind: ClusterPeer`.** This avoids a separate connection management system for inter-node connections.

5. **Message coalescing (TS `CoalescingWriter` equivalent):** Instead of a custom coalescing writer, batch multiple outbound messages by reading all available from the mpsc receiver in a single poll and sending them as a single WebSocket binary frame containing a MsgPack array. This is simpler and achieves the same effect.

---

## 12. Open Questions for Spec Phase

1. **WebSocket compression:** The TS server has `wsCompression: false` by default. Should the Rust server enable per-message deflate? Recommendation: No for Phase 3 (compression adds latency for small MsgPack messages), evaluate in Phase 4.

2. **HTTP sync endpoint format:** The TS server uses `POST /sync` for HTTP-based synchronization (non-WebSocket clients). Should the Rust server support this in Phase 3? Recommendation: Yes, it is needed for SSR and serverless environments.

3. **Connection authentication flow:** The TS server sends `AUTH_REQUIRED` immediately on connection, then waits for `AUTH` message. Should the Rust server use the same flow, or integrate with axum's authentication extractors? Recommendation: Same flow (wire protocol compatibility), but implement as a per-connection state machine rather than middleware.

4. **Inter-node connection multiplexing:** Should cluster nodes maintain one WebSocket connection per peer (like TS), or use multiple connections? Recommendation: One connection per peer for Phase 3 (matches TS behavior), with the option to add per-stream multiplexing in Phase 4 if needed.

---

## References

### Codebase Files
- `packages/server/src/modules/network-module.ts` -- TS network module (deferred startup, TLS, rate limiting)
- `packages/server/src/coordinator/connection-manager.ts` -- TS connection management
- `packages/server/src/coordinator/websocket-handler.ts` -- TS WebSocket lifecycle
- `packages/server/src/utils/BackpressureRegulator.ts` -- TS backpressure (global, not per-connection)
- `packages/server/src/modules/types.ts` -- TS module type definitions
- `packages/server-rust/src/traits.rs` -- Existing Rust server traits
- `.specflow/reference/RUST_SERVICE_ARCHITECTURE.md` -- Tower middleware + operation routing design
- `.specflow/reference/RUST_CLUSTER_ARCHITECTURE.md` -- Cluster protocol + inter-node messaging
- `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` -- Multi-layer storage architecture

### Hazelcast Reference
- `/Users/koristuvac/Projects/hazelcast/hazelcast/src/main/java/com/hazelcast/internal/networking/Channel.java` -- Protocol-agnostic channel interface
- `/Users/koristuvac/Projects/hazelcast/hazelcast/src/main/java/com/hazelcast/internal/networking/nio/NioChannel.java` -- NIO channel implementation
- `/Users/koristuvac/Projects/hazelcast/hazelcast/src/main/java/com/hazelcast/internal/nio/Connection.java` -- Connection interface

### External References
- [SurrealDB GitHub](https://github.com/surrealdb/surrealdb) -- WebSocket session management, multi-protocol support
- [Quickwit GitHub](https://github.com/quickwit-oss/quickwit) -- Tower middleware composition, startup choreography, graceful shutdown
- [Grafbase GitHub](https://github.com/grafbase/grafbase) -- WASM extension model, gateway architecture
- [Hyperswitch gRPC+REST Unification](https://github.com/juspay/hyperswitch/wiki/Bridging-Worlds:-How-we-Unified-gRPC-and-REST-APIs-in-Rust) -- Unified API definition across protocols
- [axum WebSocket example](https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs) -- Per-connection task pattern with socket splitting
- [axum graceful shutdown](https://github.com/tokio-rs/axum/blob/main/examples/graceful-shutdown/src/main.rs) -- Signal handling and connection draining
- [tokio mpsc channels](https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html) -- Bounded channel backpressure semantics
- [tower-http crate](https://docs.rs/crate/tower-http/latest) -- HTTP middleware (trace, CORS, timeout, request-id)
- [Tower middleware with axum and tonic](https://leapcell.io/blog/unpacking-the-tower-abstraction-layer-in-axum-and-tonic) -- Tower as shared abstraction between axum and tonic

---

*Research for TODO-083 (Networking Layer Research). This document informs TODO-064 (Networking Layer implementation). Spec creators should use Section 11 (Recommendations) as the primary design input.*
