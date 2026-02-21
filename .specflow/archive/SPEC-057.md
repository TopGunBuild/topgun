> **SPLIT:** This specification was decomposed into:
> - SPEC-057a: Networking Types, Config, and Core Abstractions
> - SPEC-057b: Networking HTTP and WebSocket Handlers
> - SPEC-057c: Networking Middleware, NetworkModule, and Integration
>
> See child specifications for implementation.

---
id: SPEC-057
type: feature
status: draft
priority: P0
complexity: large
created: 2026-02-20
depends_on: [SPEC-052e, SPEC-056]
todo: TODO-064
research: RES-006
---

# Implement Rust Networking Layer (axum + WebSocket)

## Context

TopGun's Rust server currently has only 3 traits (`ServerStorage`, `MapProvider`, `SchemaProvider`) and no networking code. Phase 3 begins with the networking foundation -- an axum-based HTTP/WebSocket server that all other server components (operations, cluster, storage) build upon.

The design is fully informed by RES-006 (`RUST_NETWORKING_PATTERNS.md`), which researched connection management, backpressure, middleware ordering, graceful shutdown, and TLS patterns. All decisions from that research document are pre-approved and treated as requirements here.

**Prior Research:** `.specflow/reference/RUST_NETWORKING_PATTERNS.md` (RES-006) -- contains all architectural decisions. Key choices:
- axum-native + ConnectionHandle (NOT protocol-agnostic Channel trait)
- Bounded mpsc channel (256 cap) for per-connection backpressure
- Tower HTTP middleware: RequestId > Tracing > Compression > CORS > Timeout
- Deferred startup: `NetworkModule::start()` binds port, `serve()` starts serving
- Graceful shutdown with HealthState FSM (Starting -> Ready -> Draining -> Stopped)
- DashMap-based ConnectionRegistry for 10K+ concurrent connections

**Architecture Boundary:** This spec delivers the transport layer BELOW the Operation pipeline defined in `RUST_SERVICE_ARCHITECTURE.md`. The Operation pipeline (message decode -> classify -> Tower middleware -> domain services) is a separate future spec. This spec provides the HTTP/WS server, connection management, and the bridge point where decoded messages will be handed off.

## Goal Analysis

### Goal Statement
A running axum HTTP/WebSocket server with per-connection state management, backpressure, health endpoints, graceful shutdown, and TLS support -- providing the transport foundation for all Rust server components.

### Observable Truths

| # | Truth | Verification |
|---|-------|-------------|
| OT1 | `GET /health` returns JSON with state, connection count, in-flight ops, uptime | Integration test: start server, hit endpoint, parse JSON |
| OT2 | `GET /health/live` returns 200 always; `GET /health/ready` returns 200 when Ready, 503 otherwise | Integration test: check during Starting and Ready states |
| OT3 | `GET /ws` upgrades to WebSocket; connection gets unique `ConnectionId` and appears in `ConnectionRegistry` | Integration test: connect WS client, verify registry count |
| OT4 | Outbound messages are bounded at 256; `try_send()` returns false when buffer full | Unit test: fill channel, verify try_send failure |
| OT5 | `NetworkModule::start()` binds port and returns actual port; `serve()` accepts connections | Integration test: start(), verify port > 0, serve(), connect |
| OT6 | Graceful shutdown transitions HealthState through Starting -> Ready -> Draining -> Stopped and drains connections | Integration test: trigger shutdown, poll health endpoint for 503 |
| OT7 | `POST /sync` accepts MsgPack body and returns MsgPack response (stub: echoes or returns empty batch) | Integration test: POST MsgPack payload, verify response content-type |

### Required Artifacts

```
packages/server-rust/src/
  network/
    mod.rs              -- Re-exports all public types
    config.rs           -- NetworkConfig, TlsConfig, ConnectionConfig
    connection.rs       -- ConnectionId, ConnectionKind, ConnectionHandle,
                           ConnectionMetadata, ConnectionRegistry, OutboundMessage, SendError
    shutdown.rs         -- ShutdownController, HealthState
    handlers/
      mod.rs            -- Re-exports handler functions
      health.rs         -- health_handler, liveness_handler, readiness_handler
      websocket.rs      -- ws_upgrade_handler, handle_socket, outbound_task
      http_sync.rs      -- http_sync_handler (stub: no OperationService yet)
    middleware.rs        -- build_http_middleware() function
    module.rs           -- NetworkModule struct with start()/serve()/build_router()
```

### Key Links (fragile connections between artifacts)

| Link | Risk | Mitigation |
|------|------|------------|
| ConnectionHandle.tx capacity <-> ConnectionConfig.outbound_channel_capacity | Mismatch causes wrong backpressure | ConnectionRegistry::register() reads from config |
| HealthState shared between ShutdownController and health handlers | Stale reads | ArcSwap provides lock-free atomic swaps |
| WebSocket handler split (sender/receiver) + outbound task spawn | Dropped sender closes channel | Outbound task holds sole ownership of WS sender |
| Module start() vs serve() ordering | serve() without start() panics | Option<TcpListener> with expect() message |

## Task

Create the `network` module in `packages/server-rust/src/` with the following capabilities:

1. **Types and configuration** -- All networking types, enums, config structs
2. **Connection management** -- ConnectionHandle, ConnectionRegistry (DashMap-based), bounded mpsc backpressure
3. **HTTP handlers** -- Health endpoints (health, liveness, readiness), POST /sync stub
4. **WebSocket handler** -- WS upgrade, socket split, inbound loop, outbound task
5. **HTTP middleware** -- Tower middleware stack (RequestId, Tracing, Compression, CORS, Timeout)
6. **NetworkModule** -- Deferred startup (start/serve pattern), router assembly
7. **Graceful shutdown** -- ShutdownController with HealthState FSM, connection draining

## Split Recommendation

This spec involves 10 files (8 new + 2 modified), which **exceeds the Rust "Max 5 files per spec" limit**. It MUST be split into sub-specs:

| Sub-Spec | Scope | Files | Wave |
|----------|-------|-------|------|
| **SPEC-057a** | Types, config, connection, shutdown | `mod.rs`, `config.rs`, `connection.rs`, `shutdown.rs` + `Cargo.toml` | 1 |
| **SPEC-057b** | HTTP + WebSocket handlers | `handlers/mod.rs`, `handlers/health.rs`, `handlers/websocket.rs`, `handlers/http_sync.rs` | 2 |
| **SPEC-057c** | Middleware, NetworkModule, wiring | `middleware.rs`, `module.rs`, update `lib.rs` | 3 |

**Dependency chain:** SPEC-057a -> SPEC-057b -> SPEC-057c (strictly sequential due to trait-first and handler-before-wiring ordering).

## Requirements

### File Organization

All files live under `packages/server-rust/src/network/`:

```
network/
  mod.rs                 # Re-exports
  config.rs              # NetworkConfig, TlsConfig, ConnectionConfig
  connection.rs          # ConnectionId, ConnectionKind, ConnectionHandle, ConnectionMetadata,
                         #   ConnectionRegistry, OutboundMessage, SendError
  shutdown.rs            # ShutdownController, HealthState
  handlers/
    mod.rs               # Re-exports
    health.rs            # health_handler, liveness_handler, readiness_handler
    websocket.rs         # ws_upgrade_handler, handle_socket (with outbound_task)
    http_sync.rs         # http_sync_handler (stub)
  middleware.rs           # build_http_middleware() -> ServiceBuilder stack
  module.rs              # NetworkModule with start()/serve()/build_router()
```

### Types (SPEC-057a scope)

#### `config.rs`

```rust
pub struct NetworkConfig {
    pub host: String,                  // Default: "0.0.0.0"
    pub port: u16,                     // Default: 0 (OS-assigned)
    pub tls: Option<TlsConfig>,
    pub connection: ConnectionConfig,
    pub cors_origins: Vec<String>,     // Default: ["*"]
    pub request_timeout: Duration,     // Default: 30s
}

pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_cert_path: Option<PathBuf>,
}

pub struct ConnectionConfig {
    pub outbound_channel_capacity: usize,  // Default: 256
    pub send_timeout: Duration,            // Default: 5s
    pub idle_timeout: Duration,            // Default: 60s
    pub ws_write_buffer_size: usize,       // Default: 128KB (131_072)
    pub ws_max_write_buffer_size: usize,   // Default: 512KB (524_288)
}
```

All config structs implement `Default` with the values shown above.

#### `connection.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Client,
    ClusterPeer,
}

pub enum OutboundMessage {
    Binary(Vec<u8>),
    Close(Option<String>),
}

pub enum SendError {
    Timeout,
    Disconnected,
    Full,
}

pub struct ConnectionHandle {
    pub id: ConnectionId,
    pub tx: mpsc::Sender<OutboundMessage>,
    pub metadata: Arc<RwLock<ConnectionMetadata>>,
    pub connected_at: Instant,
    pub kind: ConnectionKind,
}

pub struct ConnectionMetadata {
    pub authenticated: bool,
    pub principal: Option<Principal>,
    pub subscriptions: HashSet<String>,
    pub topics: HashSet<String>,
    pub last_heartbeat: Instant,
    pub last_hlc: Option<Timestamp>,
    pub peer_node_id: Option<String>,
}

pub struct ConnectionRegistry {
    connections: DashMap<ConnectionId, Arc<ConnectionHandle>>,
    next_id: AtomicU64,
}
```

**ConnectionHandle methods:**
- `try_send(&self, msg: OutboundMessage) -> bool` -- non-blocking send, returns false if channel full
- `send_timeout(&self, msg: OutboundMessage, timeout: Duration) -> Result<(), SendError>` -- async send with timeout
- `is_connected(&self) -> bool` -- checks if channel receiver is still alive (not closed)

**ConnectionRegistry methods:**
- `new() -> Self` -- creates empty registry
- `register(&self, kind: ConnectionKind, config: &ConnectionConfig) -> (Arc<ConnectionHandle>, mpsc::Receiver<OutboundMessage>)` -- creates handle with bounded channel, returns both ends
- `remove(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>>` -- removes and returns handle
- `get(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>>` -- lookup by ID
- `count(&self) -> usize` -- total connections
- `count_by_kind(&self, kind: ConnectionKind) -> usize` -- count by type
- `iter(&self) -> impl Iterator<Item = Arc<ConnectionHandle>>` -- iterate all connections
- `broadcast(&self, msg_bytes: Vec<u8>, kind: ConnectionKind)` -- try_send Binary to all connections of given kind
- `drain_all(&self) -> Vec<Arc<ConnectionHandle>>` -- remove and return all connections (for shutdown)

#### `shutdown.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    Starting,
    Ready,
    Draining,
    Stopped,
}

pub struct ShutdownController {
    shutdown_signal: watch::Sender<bool>,
    in_flight: Arc<AtomicU64>,
    health_state: Arc<ArcSwap<HealthState>>,
}
```

**ShutdownController methods:**
- `new() -> Self` -- initial state: Starting, shutdown_signal: false
- `set_ready(&self)` -- transitions to Ready
- `shutdown_receiver(&self) -> watch::Receiver<bool>` -- clone receiver for listening
- `trigger_shutdown(&self)` -- sets Draining, sends shutdown signal
- `health_state(&self) -> HealthState` -- current state (load from ArcSwap)
- `health_state_handle(&self) -> Arc<ArcSwap<HealthState>>` -- shared handle for handlers
- `in_flight_guard(&self) -> InFlightGuard` -- RAII guard that increments/decrements in_flight counter
- `in_flight_count(&self) -> u64` -- current in-flight operations
- `wait_for_drain(&self, timeout: Duration) -> bool` -- waits until in_flight reaches 0 or timeout; returns true if drained

### Handlers (SPEC-057b scope)

#### `handlers/health.rs`

Three axum handler functions:

- `health_handler(State(state): State<AppState>) -> impl IntoResponse` -- returns JSON: `{ "state": "ready", "connections": 42, "in_flight": 3, "uptime_secs": 120 }`
- `liveness_handler() -> StatusCode` -- always returns `200 OK`
- `readiness_handler(State(state): State<AppState>) -> StatusCode` -- returns `200` if `HealthState::Ready`, `503` otherwise

#### `handlers/websocket.rs`

- `ws_upgrade_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse` -- upgrades connection, configures write buffer sizes from config, calls `handle_socket`
- `handle_socket(socket: WebSocket, state: AppState)` -- splits socket, registers in ConnectionRegistry, spawns outbound_task, runs inbound loop
- Inbound loop: reads WS messages, logs binary messages received (stub -- no OperationService dispatch yet), handles Close/Ping/Pong
- Outbound task: reads from mpsc::Receiver, writes to WS sender; batches available messages into single frames when multiple are ready; exits when receiver is closed or Close message received
- On disconnect: removes connection from registry

#### `handlers/http_sync.rs`

- `http_sync_handler(State(state): State<AppState>, body: Bytes) -> impl IntoResponse` -- stub: accepts MsgPack body, returns empty MsgPack response (`rmp_serde::to_vec_named(&Vec::<()>::new())`). Full implementation depends on OperationService (future spec).

### Middleware (SPEC-057c scope)

#### `middleware.rs`

A function that builds the HTTP-level Tower middleware stack:

```rust
pub fn build_http_layers(config: &NetworkConfig) -> impl Layer<...> {
    ServiceBuilder::new()
        .set_x_request_id(MakeRequestUuid)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::new().allow_origin(/* from config */))
        .layer(TimeoutLayer::new(config.request_timeout))
        .propagate_x_request_id()
}
```

This is HTTP-level middleware only. The Operation-level middleware (Metrics, LoadShed, Auth, PartitionRouting, MigrationBarrier) belongs to a future spec (Service Architecture).

### NetworkModule (SPEC-057c scope)

#### `module.rs`

```rust
pub struct NetworkModule {
    config: NetworkConfig,
    listener: Option<TcpListener>,
    registry: Arc<ConnectionRegistry>,
    shutdown: Arc<ShutdownController>,
}
```

**Methods:**
- `new(config: NetworkConfig) -> Self` -- creates registry and shutdown controller; does NOT bind port
- `registry(&self) -> Arc<ConnectionRegistry>` -- access shared registry (for other modules)
- `shutdown_controller(&self) -> Arc<ShutdownController>` -- access shared shutdown controller
- `build_router(&self) -> Router` -- assembles axum Router with all routes + middleware + state
- `start(&mut self) -> anyhow::Result<u16>` -- binds TcpListener, returns actual port
- `serve(self, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>` -- consumes self, serves until shutdown signal; calls `axum::serve(listener, app).with_graceful_shutdown(shutdown)`
- Note: TLS via axum-server is wired in serve() when config.tls is Some

#### `AppState` (shared axum state)

Defined in `handlers/mod.rs`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<ConnectionRegistry>,
    pub shutdown: Arc<ShutdownController>,
    pub config: Arc<NetworkConfig>,
    pub start_time: Instant,
}
```

### Dependency Additions (`Cargo.toml`)

```toml
[dependencies]
axum = { version = "0.8", features = ["ws"] }
axum-server = { version = "0.7", features = ["tls-rustls"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "sync", "time", "net"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "timeout", "request-id", "compression-gzip"] }
dashmap = "6"
arc-swap = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rmp-serde = "1"
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }
bytes = "1"
http = "1"
```

### Files Modified

| File | Action |
|------|--------|
| `packages/server-rust/Cargo.toml` | Add networking dependencies |
| `packages/server-rust/src/lib.rs` | Add `pub mod network;` |
| `packages/server-rust/src/network/mod.rs` | Create: re-exports |
| `packages/server-rust/src/network/config.rs` | Create: NetworkConfig, TlsConfig, ConnectionConfig |
| `packages/server-rust/src/network/connection.rs` | Create: ConnectionHandle, ConnectionRegistry, etc. |
| `packages/server-rust/src/network/shutdown.rs` | Create: ShutdownController, HealthState |
| `packages/server-rust/src/network/handlers/mod.rs` | Create: handler re-exports |
| `packages/server-rust/src/network/handlers/health.rs` | Create: health endpoint handlers |
| `packages/server-rust/src/network/handlers/websocket.rs` | Create: WS upgrade + socket handler |
| `packages/server-rust/src/network/handlers/http_sync.rs` | Create: POST /sync stub handler |
| `packages/server-rust/src/network/middleware.rs` | Create: HTTP middleware stack |
| `packages/server-rust/src/network/module.rs` | Create: NetworkModule |

**Total: 10 new files + 2 modified = 12 file touches**

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
2. `ConnectionRegistry::register()` creates a `ConnectionHandle` with a bounded mpsc channel of capacity 256 (configurable via `ConnectionConfig`)
3. `ConnectionHandle::try_send()` returns `false` when channel is full (unit test: send 257 messages to 256-capacity channel)
4. `ConnectionRegistry::count()` accurately reflects active connections after register/remove operations
5. `HealthState` transitions are correct: `Starting -> Ready` (via `set_ready()`), `Ready -> Draining` (via `trigger_shutdown()`), final `Stopped` after drain
6. `GET /health` returns JSON with `state`, `connections`, `in_flight`, `uptime_secs` fields
7. `GET /health/live` always returns `200`; `GET /health/ready` returns `200` when Ready, `503` when not Ready
8. `GET /ws` successfully upgrades to WebSocket; connection appears in `ConnectionRegistry`
9. WebSocket disconnect (client-initiated close) removes connection from `ConnectionRegistry`
10. `POST /sync` accepts a request body and returns MsgPack response with correct content-type header
11. `NetworkModule::start()` binds to port 0 and returns an actual port > 0; `serve()` accepts connections on that port
12. `ShutdownController::trigger_shutdown()` causes `serve()` to begin graceful shutdown; connections are drained
13. HTTP middleware stack applies RequestId header (X-Request-Id) to all responses
14. All `Default` implementations produce the values specified in Requirements (256 channel cap, 5s send timeout, 60s idle timeout, etc.)
15. `broadcast()` sends to all connections of specified kind; skips full channels without blocking

## Constraints

1. **No Operation pipeline** -- handlers receive raw bytes and log/stub them. OperationService dispatch is a future spec.
2. **No authentication logic** -- ConnectionMetadata.authenticated starts as false. Auth handling is a future spec.
3. **No cluster-specific logic** -- ConnectionKind::ClusterPeer exists as a type but no cluster handshake or protocol.
4. **No WebSocket compression** -- `per_message_deflate` is disabled (Phase 4 evaluation per RES-006).
5. **No rate limiting at WS level** -- backpressure is via bounded mpsc, not per-message rate limiting.
6. **Transport layer only** -- this spec does not decode MsgPack messages into `Message` enum. That is the Operation pipeline's job.
7. **No phase/spec references in code comments** -- use WHY-comments only.
8. **No `f64` for integer-semantic fields** -- follow Rust Type Mapping Rules from PROJECT.md.

## Assumptions

1. **axum 0.8** is the target version (latest stable as of 2026-02-20). If 0.9 is released, adapter changes may be needed.
2. **tokio-tungstenite** does not need to be a direct dependency -- axum's `ws` feature provides WebSocket support internally.
3. **AppState** will grow as future specs add fields (e.g., OperationService, ClusterState). For now it contains only registry, shutdown, config, and start_time.
4. **TLS testing** is deferred -- TLS config struct is created but TLS integration tests require certificate generation which is out of scope for this spec. The code path (axum-server with rustls) is wired but a manual test suffices.
5. **Message coalescing** (batching multiple outbound messages into single WS frame) is included in the outbound task implementation as described in RES-006, not deferred.
6. **`POST /sync` stub** returns an empty MsgPack array. Full implementation requires OperationService.
7. **Port 0** is used in tests for OS-assigned ports to avoid port conflicts.

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `config.rs` (NetworkConfig, TlsConfig, ConnectionConfig with Default impls), `connection.rs` (ConnectionId, ConnectionKind, OutboundMessage, SendError, ConnectionHandle, ConnectionMetadata, ConnectionRegistry), `shutdown.rs` (HealthState, ShutdownController, InFlightGuard), `network/mod.rs` (re-exports), update `Cargo.toml` with dependencies | -- | ~30% |
| G2 | 2 | Create `handlers/health.rs` (health, liveness, readiness), `handlers/websocket.rs` (ws_upgrade, handle_socket, outbound_task with message coalescing), `handlers/http_sync.rs` (POST /sync stub), `handlers/mod.rs` (re-exports). Define `AppState` struct. | G1 | ~35% |
| G3 | 3 | Create `middleware.rs` (build_http_layers), `module.rs` (NetworkModule with start/serve/build_router). Update `lib.rs` to add `pub mod network`. Integration tests for full server lifecycle. | G1, G2 | ~35% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |
| 3 | G3 | No | 1 |

**Total workers needed:** 1 (strictly sequential due to Rust compilation dependencies)

## Recommended Split

**This spec is `large` and exceeds the Rust 5-file limit.** It should be split via `/sf:split SPEC-057` into:

| Sub-Spec | Wave | Files | Focus |
|----------|------|-------|-------|
| SPEC-057a | 1 | `Cargo.toml`, `mod.rs`, `config.rs`, `connection.rs`, `shutdown.rs` (5) | All types, config, and core abstractions |
| SPEC-057b | 2 | `handlers/mod.rs`, `handlers/health.rs`, `handlers/websocket.rs`, `handlers/http_sync.rs` (4) | All handler implementations + AppState |
| SPEC-057c | 3 | `middleware.rs`, `module.rs`, update `lib.rs` (3) | Wiring, middleware, NetworkModule, integration tests |

Each sub-spec stays within the 5-file Rust limit. SPEC-057a is trait-first (types only). SPEC-057b and SPEC-057c contain implementations that depend on G1 types.
