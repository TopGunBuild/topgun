# SPEC-057c: Networking Middleware, NetworkModule, and Integration

---
id: SPEC-057c
type: feature
status: done
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

Create the HTTP middleware stack, the NetworkModule struct with deferred startup, wire all routes together, update `network/mod.rs` to expose new modules, and write integration tests that verify the full server lifecycle.

### Approach

1. Create `middleware.rs` with `build_http_layers()` function that builds the Tower middleware stack
2. Create `module.rs` with `NetworkModule` struct implementing `new()`, `registry()`, `shutdown_controller()`, `build_router()`, `start()`, `serve()`
3. Update `network/mod.rs` to add `pub mod middleware;` and `pub mod module;`
4. Write integration tests for full server lifecycle: start, connect, health check, WS upgrade, shutdown drain

## Requirements

### File Organization

```
network/
  middleware.rs           # build_http_layers() -> Tower middleware stack
  module.rs              # NetworkModule with start()/serve()/build_router()
```

Plus updates to:
- `network/mod.rs` -- add `pub mod middleware;` and `pub mod module;` with re-exports

**Note:** `lib.rs` already contains `pub mod network;` (added during SPEC-057b). No changes needed there.

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
- When config.tls is Some: use `axum_server::from_tcp_rustls(listener.into_std()?, tls_config)` to reuse the pre-bound listener with TLS. If `from_tcp_rustls` is unavailable in the axum-server version, rebind with `axum_server::bind_rustls(addr, tls_config)` using the same host:port and log a warning that the listener was rebound. TLS is not integration-tested (see Assumption 3).
- After shutdown signal received: transitions health state to Draining, drains connections via registry.drain_all(), sends Close to each, transitions to Stopped

### Update `network/mod.rs`

Add to existing mod.rs the following new lines (note: `pub use handlers::AppState;` already exists and must NOT be duplicated):

```rust
pub mod middleware;
pub mod module;

// Already present -- do not add again:
// pub use handlers::AppState;
pub use module::NetworkModule;
```

### Integration Tests

Integration tests verify the full server lifecycle. They should be placed in `packages/server-rust/src/network/module.rs` (as `#[cfg(test)] mod tests`) or in a separate test file.

**Dev-dependencies required:** Add `reqwest` (with JSON feature) and `tokio-tungstenite` to `[dev-dependencies]` in `packages/server-rust/Cargo.toml`.

**Test helper:** Extract a shared `start_server()` async function that creates a `NetworkModule` with default config (port 0), calls `start()`, spawns `serve()` in a background tokio task with a `tokio::sync::oneshot` shutdown trigger, and returns `(u16, Arc<ConnectionRegistry>, Arc<ShutdownController>, oneshot::Sender<()>)`. All integration tests below should use this helper to reduce boilerplate.

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
- Verify registry.count() == 0 (poll with 50ms intervals, up to 2s timeout)

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
| `packages/server-rust/src/network/module.rs` | Create: NetworkModule + integration tests |
| `packages/server-rust/src/network/mod.rs` | Update: add `pub mod middleware;`, `pub mod module;`, re-exports |
| `packages/server-rust/Cargo.toml` | Update: add dev-dependencies (reqwest, tokio-tungstenite) |

**Total: 2 new files + 2 modified = 4 file touches** (within Rust 5-file limit)

## Acceptance Criteria

1. `NetworkModule::start()` binds to port 0 and returns an actual port > 0; `serve()` accepts connections on that port
2. `ShutdownController::trigger_shutdown()` causes `serve()` to begin graceful shutdown; connections are drained
3. HTTP middleware stack applies RequestId header (X-Request-Id) to all responses
4. `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
5. Integration tests pass: health endpoint, WS upgrade + registry count, POST /sync content-type, graceful shutdown, RequestId header
6. `topgun_server::network::NetworkModule` is publicly accessible from crate root

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
2. **tokio-tungstenite** does not need to be a direct runtime dependency -- axum's `ws` feature provides WebSocket support internally. It is added as a dev-dependency for integration test WS clients only.
3. **TLS testing** is deferred -- TlsConfig struct exists (from SPEC-057a) and TLS code path is wired in serve(), but integration tests do not test TLS. Manual verification suffices.
4. **Port 0** is used in tests for OS-assigned ports to avoid port conflicts.
5. **Integration test clients** -- use `reqwest` (with JSON feature) for HTTP tests, `tokio-tungstenite` as dev-dependency for WS tests.
6. **Graceful shutdown timeout** -- serve() waits for axum's built-in graceful shutdown which drains active connections. Additional drain logic (sending Close to all connections) is done after the shutdown signal fires.

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `middleware.rs` (build_http_layers with Tower middleware stack: RequestId, Tracing, Compression, CORS, Timeout) | -- | ~10% |
| G2 | 1 | Create `module.rs` (NetworkModule with new, registry, shutdown_controller, build_router, start, serve), update `network/mod.rs` and `Cargo.toml` dev-deps | -- | ~20% |
| G3 | 2 | Write integration tests: health endpoints, WS upgrade + registry, POST /sync, graceful shutdown, RequestId header | G1, G2 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~55% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~55% | <=50% | Warning |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <-- Current estimate |
| 70%+ | POOR | - |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | middleware.rs | ~10% | 10% |
| G2 | 1 | module.rs + mod.rs + Cargo.toml | ~20% | 30% |
| G3 | 2 | Integration tests (7 tests) | ~25% | 55% |

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (N/A -- no new data structs with numeric fields)
- [x] No `r#type: String` on message structs (N/A -- no message structs)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A)
- [x] Enums used for known value sets (N/A)
- [x] Wire compatibility (N/A -- no serialization in new code)
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- NetworkModule is not serialized)
- [x] `#[serde(skip_serializing_if)]` on Option fields (N/A)

**Strategic fit:** Aligned with project goals -- completes transport layer required for all Phase 3 server work.

**Project compliance:** Honors PROJECT.md decisions (axum, tokio, Tower middleware, MsgPack, no phase refs in code).

**Language profile:** Compliant with Rust profile (4 file touches within 5-file limit; no traits to define so trait-first N/A).

**Issues corrected during audit (applied to spec above):**
1. Removed stale instruction to add `pub mod network;` to `lib.rs` (already present from SPEC-057b)
2. Updated Files Modified table to include `mod.rs` and `Cargo.toml` (dev-deps) instead of `lib.rs`
3. Corrected file count from "3 file touches" to "4 file touches"
4. Updated AC6 to reference `NetworkModule` path instead of generic `pub mod network;`
5. Added explicit dev-dependency requirement for `reqwest` and `tokio-tungstenite`
6. Changed WS disconnect test from "brief delay" to "poll with 50ms intervals, up to 2s timeout"
7. Revised context estimates: G1 20%->10%, G2 35%->20%, G3 45%->25% (original estimates were inflated)
8. Removed step 4 from Approach (lib.rs already done)

**Recommendations:**
1. [Strategic] The TLS code path in `serve()` is underspecified. `axum-server` binds its own listener via `bind_rustls(addr, config)`, which conflicts with the pre-bound `TcpListener` from `start()`. The implementer should use `axum_server::from_tcp_rustls()` or similar API if available, or bind separately for TLS. Since TLS testing is deferred (Assumption 3), this is non-blocking but worth noting.
2. Consider extracting a `start_server()` test helper that creates a NetworkModule, calls start(), spawns serve(), and returns (port, registry, shutdown_controller) to reduce test boilerplate across the 7 integration tests.

**Comment:** Well-structured spec with clear architecture boundary, good separation of concerns, and concrete acceptance criteria. The corrections above (stale lib.rs reference, missing Cargo.toml in file list, vague polling in WS test) are minor and have been applied directly to the spec text. Ready for implementation.

### Response v1 (2026-02-21)
**Applied:** Both audit recommendations (1, 2)

**Changes:**
1. [✓] TLS code path clarification — updated `serve()` method spec to specify `axum_server::from_tcp_rustls()` for reusing pre-bound listener, with fallback to `bind_rustls()` rebind strategy
2. [✓] Test helper extraction — added `start_server()` helper specification that returns (port, registry, shutdown_controller, shutdown_sender) for use across all 7 integration tests

### Audit v2 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~15% | <=10% | Warning |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | middleware.rs | ~10% | 10% |
| G2 | 1 | module.rs + mod.rs + Cargo.toml | ~20% | 30% |
| G3 | 2 | Integration tests (7 tests) | ~15% | 45% |

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (N/A -- no new data structs with numeric fields)
- [x] No `r#type: String` on message structs (N/A -- no message structs)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A)
- [x] Enums used for known value sets (N/A)
- [x] Wire compatibility (N/A -- no serialization in new code)
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- NetworkModule is not serialized)
- [x] `#[serde(skip_serializing_if)]` on Option fields (N/A)

**Source verification (fresh audit):**
- Verified `lib.rs` already has `pub mod network;` (line 3) -- no changes needed
- Verified `network/mod.rs` already has `pub use handlers::AppState;` (line 10) -- must not duplicate
- Verified `Cargo.toml` has no existing `reqwest` or `tokio-tungstenite` dev-deps -- addition is correct
- Verified all handler function signatures match spec claims (health_handler, liveness_handler, readiness_handler, ws_upgrade_handler, http_sync_handler)
- Verified AppState fields match spec (registry, shutdown, config, start_time)
- Verified ConnectionRegistry has `count()`, `drain_all()`, and other methods referenced by spec
- Verified ShutdownController has `set_ready()`, `trigger_shutdown()`, `health_state()`, `wait_for_drain()`

**Strategic fit:** Aligned with project goals -- completes transport layer for Phase 3.

**Project compliance:** Honors PROJECT.md decisions (axum, tokio, Tower, MsgPack, no phase refs in code).

**Language profile:** Compliant with Rust profile (4 file touches within 5-file limit; no traits to define so trait-first N/A).

**Issues corrected during audit (applied to spec above):**
1. Clarified `mod.rs` update section to explicitly note that `pub use handlers::AppState;` already exists and must not be duplicated
2. Clarified Assumption 2 wording to distinguish runtime dependency (not needed) from dev-dependency (needed for test clients)

**Recommendations:**
1. The spec is marked `complexity: small` but includes 7 integration tests, 2 new files, 2 modified files, lifecycle management, and TLS branching. This is closer to "medium" complexity. Not blocking but may affect execution time expectations.

**Comment:** This is a well-polished spec after the v1 audit/revision cycle. Both prior recommendations were applied. All handler function signatures and type references verified against source code. The mod.rs duplicate re-export issue has been clarified. Ready for implementation.

## Execution Summary

**Executed:** 2026-02-21
**Mode:** orchestrated
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/server-rust/src/network/middleware.rs` -- Tower HTTP middleware stack (RequestId, Tracing, Compression, CORS, Timeout)
- `packages/server-rust/src/network/module.rs` -- NetworkModule with deferred startup lifecycle + integration tests

### Files Modified
- `packages/server-rust/src/network/mod.rs` -- added `pub mod middleware;`, `pub mod module;`, `pub use module::NetworkModule;`
- `packages/server-rust/Cargo.toml` -- added dev-dependencies: reqwest, tokio-tungstenite

### Acceptance Criteria Status
- [x] `NetworkModule::start()` binds to port 0 and returns an actual port > 0; `serve()` accepts connections on that port
- [x] `ShutdownController::trigger_shutdown()` causes `serve()` to begin graceful shutdown; connections are drained
- [x] HTTP middleware stack applies RequestId header (X-Request-Id) to all responses
- [x] `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
- [x] Integration tests pass: health endpoint, WS upgrade + registry count, POST /sync content-type, graceful shutdown, RequestId header
- [x] `topgun_server::network::NetworkModule` is publicly accessible from crate root

### Test Results
- 59 total tests (6 new integration tests + 10 new unit tests + 43 existing)
- All 59 pass with 0 failures
- Clippy clean (including test code)

### Deviations
(none)

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 3
**Review Cycles:** 2

---

## Review History

### Review v1 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**

1. **Router construction duplicated in `serve()`**
   - File: `packages/server-rust/src/network/module.rs:151-168`
   - Issue: `serve()` rebuilds the router inline (routes + middleware + state) instead of calling `build_router()`. This means the route list is defined in two places: `build_router()` (lines 82-100) and `serve()` (lines 151-168). If a route is added or changed, both locations must be updated.
   - Context: This is a pragmatic workaround for Rust's ownership rules -- `serve()` consumes `self` and must extract `self.listener` first, making a subsequent `self.build_router()` call impossible due to partial move. Not a correctness issue.
   - Suggestion: A future refactor could extract route/middleware setup into a standalone function that takes `&NetworkConfig`, `Arc<ConnectionRegistry>`, and `Arc<ShutdownController>` as parameters, callable from both `build_router()` and `serve()`.

2. **Avoidable `config.clone()` in `serve()`**
   - File: `packages/server-rust/src/network/module.rs:155`
   - Issue: `config.clone()` is used to wrap in `Arc::new()` while keeping the original for `build_http_layers(&config)`. This clone could be avoided by calling `build_http_layers(&config)` first, then consuming `config` into `Arc::new(config)`.
   - Impact: Negligible (NetworkConfig is small), but worth noting for idiomatic Rust style.

3. **Graceful shutdown test does not verify `serve()` completion**
   - File: `packages/server-rust/src/network/module.rs:453-479`
   - Issue: The spec says "Verify serve() completes" but the test (`graceful_shutdown_drains_and_stops`) only checks that health state transitions to Draining or Stopped. It does not join on the spawned serve task to verify it actually completes. This is a minor gap -- the state transition to Stopped implicitly proves serve() ran its drain logic, but explicit task completion verification would be more thorough.

**Passed:**
- [x] AC1: `NetworkModule::start()` binds port 0, returns port > 0; `serve()` accepts connections -- verified by `start_binds_to_os_assigned_port` and all integration tests successfully sending HTTP/WS requests
- [x] AC2: `ShutdownController::trigger_shutdown()` causes graceful shutdown with connection draining -- verified by `graceful_shutdown_drains_and_stops` test (health state transitions to Draining/Stopped)
- [x] AC3: HTTP middleware applies X-Request-Id to all responses -- verified by `request_id_header_is_present_in_response` test (checks UUID v4 format, 36 chars)
- [x] AC4: `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings -- verified by running both commands
- [x] AC5: All 7 spec-defined integration tests present and passing (health, liveness/readiness, WS upgrade + registry, POST /sync, graceful shutdown, RequestId) -- 59/59 tests pass
- [x] AC6: `topgun_server::network::NetworkModule` publicly accessible -- verified via `pub mod network;` in lib.rs and `pub use module::NetworkModule;` in mod.rs
- [x] Middleware ordering matches spec (SetRequestId > Tracing > Compression > CORS > Timeout > PropagateRequestId)
- [x] All 5 routes match spec (`/health`, `/health/live`, `/health/ready`, `/ws`, `/sync`)
- [x] NetworkModule struct fields match spec (config, listener, registry, shutdown)
- [x] All 6 methods implemented per spec (new, registry, shutdown_controller, build_router, start, serve)
- [x] TLS path correctly uses `axum_server::from_tcp_rustls()` with pre-bound listener
- [x] `start_server()` test helper extracted per spec with correct return type
- [x] No phase/spec references in code comments
- [x] No `f64` for integer-semantic fields (N/A)
- [x] No unnecessary `.unwrap()` in production code (only `expect` on listener with clear message, per spec)
- [x] No `unsafe` blocks
- [x] Proper `Arc::clone()` usage (not `.clone()` on Arc)
- [x] Error handling uses `?` operator throughout production code
- [x] `#[must_use]` annotations on pure functions
- [x] `mod.rs` does not duplicate `pub use handlers::AppState;`
- [x] Dev-dependencies added correctly (reqwest with json feature, tokio-tungstenite)
- [x] CORS layer correctly handles wildcard `"*"` and specific origins
- [x] Drain logic sends Close frames to all connections and waits up to 30s
- [x] Deferred startup pattern correctly implemented (new -> start -> serve)
- [x] `build_http_layers()` returns concrete type via type alias (good practice for Tower stacks)
- [x] `TimeoutLayer::with_status_code(408)` is an improvement over spec's `TimeoutLayer::new()` -- returns proper HTTP 408 on timeout

**Language Profile Verification:**
- Build check: PASSED (zero errors)
- Lint check: PASSED (zero clippy warnings)
- Test check: PASSED (59/59 tests pass)
- Rust idiom check: No unnecessary clones (except minor item 2 above), proper error handling with `?`, no unsafe, no `Box<dyn Any>`

**Summary:** Clean, well-structured implementation that meets all 6 acceptance criteria. The code is idiomatic Rust with excellent documentation (doc comments on all public items). The 3 minor findings are style/optimization suggestions that do not affect correctness or functionality. The middleware stack, deferred startup lifecycle, graceful shutdown, and TLS path are all correctly implemented per the specification.

### Fix Response v1 (2026-02-21)
**Applied:** All 3 minor issues from Review v1

**Fixes:**
1. [✓] Router construction duplicated in `serve()` — extracted `build_app()` standalone function that both `build_router()` and `serve()` call, eliminating route duplication
2. [✓] Avoidable `config.clone()` in `serve()` — `build_app()` takes ownership of config, calls `build_http_layers(&config)` before `Arc::new(config)`. TLS config extracted via `.take()` before consuming config.
3. [✓] Graceful shutdown test doesn't verify `serve()` completion — `start_server()` now returns `JoinHandle<()>`. Shutdown test awaits serve task with 5s timeout and verifies it completes without panic.
   - Commit: 80e8a20

### Review v2 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Fix Verification:**

1. [VERIFIED] Router construction deduplication -- `build_app()` standalone function at `module.rs:159-181` is called by both `build_router()` (line 83) and `serve()` (line 142). Routes are defined in exactly one place. No duplication remains.
2. [VERIFIED] Avoidable `config.clone()` removed -- `serve()` moves `self.config` (line 136), extracts TLS via `.take()` (line 140), then passes owned config to `build_app()` (line 142). Inside `build_app()`, `build_http_layers(&config)` borrows first (line 164), then `Arc::new(config)` consumes (line 169). Zero unnecessary clones in the serve path.
3. [VERIFIED] Graceful shutdown test verifies `serve()` completion -- `start_server()` returns `JoinHandle<()>` as 5th tuple element (line 281-308). Test `graceful_shutdown_drains_and_stops` awaits the handle with `tokio::time::timeout(Duration::from_secs(5), serve_handle)` (line 472-475), verifying both completion and no panic.

**Findings:**

(No new issues found.)

**Passed:**
- [x] AC1: `NetworkModule::start()` binds port 0, returns port > 0; `serve()` accepts connections
- [x] AC2: `ShutdownController::trigger_shutdown()` causes graceful shutdown with connection draining
- [x] AC3: HTTP middleware applies X-Request-Id to all responses
- [x] AC4: `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
- [x] AC5: All 7 integration tests present and passing (59/59 total)
- [x] AC6: `topgun_server::network::NetworkModule` publicly accessible from crate root
- [x] All 3 Review v1 minor fixes correctly applied
- [x] No regressions introduced by fixes (59/59 tests still pass)
- [x] No phase/spec references in code comments
- [x] No `.unwrap()` in production code
- [x] No `unsafe` blocks
- [x] Idiomatic `Arc::clone()` usage throughout
- [x] `build_app()` eliminates route duplication between `build_router()` and `serve()`
- [x] Zero unnecessary clones in `serve()` path
- [x] `serve()` completion explicitly verified in shutdown test via JoinHandle

**Language Profile Verification:**
- Build check: PASSED (zero errors)
- Lint check: PASSED (zero clippy warnings)
- Test check: PASSED (59/59 tests pass, 0 failures)
- Rust idiom check: Clean -- no unnecessary clones, proper error propagation, no unsafe

**Summary:** All 3 minor fixes from Review v1 are correctly applied and verified. No new issues introduced. The implementation is clean, idiomatic, and fully compliant with the specification. Ready for finalization.
