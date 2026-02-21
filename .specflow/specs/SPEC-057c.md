# SPEC-057c: Networking Middleware, NetworkModule, and Integration

---
id: SPEC-057c
type: feature
status: draft
priority: P0
complexity: small
created: 2026-02-20
parent: SPEC-057
depends_on: [SPEC-057b]
todo_ref: TODO-064
---

## Context

This sub-spec completes the networking layer by wiring together the types (SPEC-057a) and handlers (SPEC-057b) into a running server. It creates the Tower HTTP middleware stack, the NetworkModule struct that manages server lifecycle (deferred startup pattern: start binds, serve accepts), and integration tests that verify the full stack end-to-end.

The design is fully informed by RES-006 (`RUST_NETWORKING_PATTERNS.md`). Key decisions from that research applied here:
- Tower HTTP middleware ordering: RequestId > Tracing > Compression > CORS > Timeout
- Deferred startup pattern: `NetworkModule::start()` binds port, `serve()` starts serving
- Graceful shutdown with connection draining
- TLS via axum-server when config.tls is Some

**Architecture Boundary:** This spec delivers the complete transport layer. The Operation pipeline (message decode -> classify -> Tower middleware -> domain services) from `RUST_SERVICE_ARCHITECTURE.md` is a separate future spec. This spec provides the HTTP/WS server that the operation pipeline will plug into.

**Available from SPEC-057a:**
- `NetworkConfig`, `ConnectionConfig`, `TlsConfig`
- `ConnectionId`, `ConnectionKind`, `OutboundMessage`, `SendError`
- `ConnectionHandle`, `ConnectionMetadata`, `ConnectionRegistry`
- `HealthState`, `ShutdownController`, `InFlightGuard`

**Available from SPEC-057b:**
- `AppState`
- `health_handler`, `liveness_handler`, `readiness_handler`
- `ws_upgrade_handler`
- `http_sync_handler`

## Task

Create the HTTP middleware stack, the NetworkModule struct with deferred startup, wire all routes together, update `lib.rs` to expose the network module, and write integration tests that verify the full server lifecycle.

### Approach

1. Create `middleware.rs` with `build_http_layers()` function that builds the Tower middleware stack
2. Create `module.rs` with `NetworkModule` struct implementing `new()`, `registry()`, `shutdown_controller()`, `build_router()`, `start()`, `serve()`
3. Update `network/mod.rs` to add `pub mod middleware;` and `pub mod module;`
4. Update `packages/server-rust/src/lib.rs` to add `pub mod network;`
5. Write integration tests for full server lifecycle: start, connect, health check, WS upgrade, shutdown drain

## Requirements

### File Organization

```
network/
  middleware.rs           # build_http_layers() -> Tower middleware stack
  module.rs              # NetworkModule with start()/serve()/build_router()
```

Plus updates to:
- `network/mod.rs` -- add `pub mod middleware;` and `pub mod module;` with re-exports
- `packages/server-rust/src/lib.rs` -- add `pub mod network;`

### `middleware.rs`

A function that builds the HTTP-level Tower middleware stack:

```rust
pub fn build_http_layers(config: &NetworkConfig) -> impl Layer<...> + Clone {
    ServiceBuilder::new()
        .set_x_request_id(MakeRequestUuid)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::new()
            .allow_origin(/* from config.cors_origins */)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any))
        .layer(TimeoutLayer::new(config.request_timeout))
        .propagate_x_request_id()
}
```

**Middleware ordering (outermost to innermost):**
1. **SetRequestId** -- assigns X-Request-Id to every incoming request (using UUID v4)
2. **Tracing** -- logs request/response with trace spans
3. **Compression** -- gzip response compression
4. **CORS** -- Cross-Origin Resource Sharing based on config.cors_origins
5. **Timeout** -- request timeout based on config.request_timeout
6. **PropagateRequestId** -- copies X-Request-Id to response headers

This is HTTP-level middleware only. The Operation-level middleware (Metrics, LoadShed, Auth, PartitionRouting, MigrationBarrier) belongs to a future spec (Service Architecture).

### `module.rs`

```rust
pub struct NetworkModule {
    config: NetworkConfig,
    listener: Option<TcpListener>,
    registry: Arc<ConnectionRegistry>,
    shutdown: Arc<ShutdownController>,
}
```

**Methods:**

**`new(config: NetworkConfig) -> Self`**
- Creates new ConnectionRegistry and ShutdownController
- Wraps both in Arc for sharing
- Does NOT bind port (deferred startup)
- Sets listener to None

**`registry(&self) -> Arc<ConnectionRegistry>`**
- Returns clone of Arc<ConnectionRegistry> for other modules to access

**`shutdown_controller(&self) -> Arc<ShutdownController>`**
- Returns clone of Arc<ShutdownController> for other modules to access

**`build_router(&self) -> Router`**
- Assembles axum Router with all routes:
  - `GET /health` -> health_handler
  - `GET /health/live` -> liveness_handler
  - `GET /health/ready` -> readiness_handler
  - `GET /ws` -> ws_upgrade_handler
  - `POST /sync` -> http_sync_handler
- Applies middleware from `build_http_layers(&self.config)`
- Injects AppState via `.with_state()`
- AppState is constructed with: registry, shutdown, Arc::new(config), Instant::now()

**`start(&mut self) -> anyhow::Result<u16>`**
- Binds TcpListener to `config.host:config.port`
- Stores listener in `self.listener`
- Returns actual bound port (from `listener.local_addr()?.port()`)
- If config.port is 0, OS assigns a free port

**`serve(self, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>`**
- Consumes self (takes ownership)
- Takes listener from self.listener (panics with clear message if start() was not called)
- Sets health state to Ready
- When config.tls is None: `axum::serve(listener, app).with_graceful_shutdown(shutdown).await`
- When config.tls is Some: uses axum-server with rustls configuration
- After shutdown signal received: transitions health state to Draining, drains connections via registry.drain_all(), sends Close to each, transitions to Stopped

### Update `network/mod.rs`

Add to existing mod.rs:

```rust
pub mod middleware;
pub mod module;

pub use handlers::AppState;
pub use module::NetworkModule;
```

### Update `lib.rs`

Add to `packages/server-rust/src/lib.rs`:

```rust
pub mod network;
```

### Integration Tests

Integration tests verify the full server lifecycle. They should be placed in `packages/server-rust/src/network/module.rs` (as `#[cfg(test)] mod tests`) or in a separate test file.

**Test: server start and port binding**
- Create NetworkModule with default config (port 0)
- Call start(), verify returned port > 0
- Verify start() can be called (no panic)

**Test: health endpoint responds**
- Start server, spawn serve() in background task
- Send GET /health to returned port
- Verify 200 response with JSON containing "state": "ready"

**Test: liveness and readiness**
- Start server, spawn serve()
- GET /health/live -> 200
- GET /health/ready -> 200 (after serve sets Ready)

**Test: WebSocket upgrade and registry**
- Start server, spawn serve()
- Connect WS client to ws://localhost:{port}/ws
- Verify registry.count() == 1
- Disconnect client
- Verify registry.count() == 0 (after brief delay for async cleanup)

**Test: POST /sync stub**
- Start server, spawn serve()
- POST to /sync with arbitrary body
- Verify response content-type is application/msgpack
- Verify response body is valid MsgPack (empty array)

**Test: graceful shutdown**
- Start server, spawn serve() with a controlled shutdown signal
- Connect a WS client
- Trigger shutdown
- Verify health transitions to Draining
- Verify serve() completes

**Test: RequestId header**
- Start server, spawn serve()
- Send any HTTP request
- Verify X-Request-Id header is present in response

All tests use port 0 for OS-assigned ports to avoid conflicts.

### Files Modified

| File | Action |
|------|--------|
| `packages/server-rust/src/network/middleware.rs` | Create: build_http_layers() |
| `packages/server-rust/src/network/module.rs` | Create: NetworkModule |
| `packages/server-rust/src/lib.rs` | Update: add `pub mod network;` |

**Total: 2 new files + 1 modified = 3 file touches** (within Rust 5-file limit)

**Note:** `network/mod.rs` is also updated with new module declarations, but this is a few-line addition, not a new file.

## Acceptance Criteria

1. `NetworkModule::start()` binds to port 0 and returns an actual port > 0; `serve()` accepts connections on that port
2. `ShutdownController::trigger_shutdown()` causes `serve()` to begin graceful shutdown; connections are drained
3. HTTP middleware stack applies RequestId header (X-Request-Id) to all responses
4. `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
5. Integration tests pass: health endpoint, WS upgrade + registry count, POST /sync content-type, graceful shutdown, RequestId header
6. `pub mod network;` in lib.rs makes the full networking API accessible as `topgun_server::network::*`

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

1. **axum 0.8** is the target version. `axum::serve()` API follows 0.8 conventions.
2. **tokio-tungstenite** does not need to be a direct dependency -- axum's `ws` feature provides WebSocket support internally.
3. **TLS testing** is deferred -- TlsConfig struct exists (from SPEC-057a) and TLS code path is wired in serve(), but integration tests do not test TLS. Manual verification suffices.
4. **Port 0** is used in tests for OS-assigned ports to avoid port conflicts.
5. **Integration test clients** -- use `reqwest` or `hyper` for HTTP tests, `tokio-tungstenite` (as dev-dependency) for WS tests. Add dev-dependencies as needed.
6. **Graceful shutdown timeout** -- serve() waits for axum's built-in graceful shutdown which drains active connections. Additional drain logic (sending Close to all connections) is done after the shutdown signal fires.

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `middleware.rs` (build_http_layers with Tower middleware stack: RequestId, Tracing, Compression, CORS, Timeout) | -- | ~20% |
| G2 | 1 | Create `module.rs` (NetworkModule with new, registry, shutdown_controller, build_router, start, serve), update `network/mod.rs` and `lib.rs` | -- | ~35% |
| G3 | 2 | Write integration tests: health endpoints, WS upgrade + registry, POST /sync, graceful shutdown, RequestId header | G1, G2 | ~45% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)
