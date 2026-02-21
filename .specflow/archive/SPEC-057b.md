# SPEC-057b: Networking HTTP and WebSocket Handlers

---
id: SPEC-057b
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-20
parent: SPEC-057
depends_on: [SPEC-057a]
todo_ref: TODO-064
---

## Context

This sub-spec builds on the networking types and abstractions from SPEC-057a to implement the HTTP and WebSocket handler layer. It creates health endpoints, a WebSocket upgrade handler with inbound/outbound message loops, and a stub HTTP sync endpoint. It also defines the `AppState` struct that ties all shared state together for axum handlers.

The design is fully informed by RES-006 (`RUST_NETWORKING_PATTERNS.md`). Key decisions from that research applied here:
- WebSocket socket split pattern: separate sender (owned by outbound_task) and receiver (owned by inbound loop)
- Message coalescing in outbound_task: batch multiple ready messages into single writes
- Health endpoint FSM: expose HealthState via JSON for orchestrator health checks
- Stub handlers: POST /sync returns empty MsgPack (no OperationService yet)

**Architecture Boundary:** This spec implements handlers that use the types from SPEC-057a but does NOT wire them into a running server. The router assembly, middleware stack, and NetworkModule are SPEC-057c's scope. Handlers are pure functions that can be tested with axum's test utilities.

**Available from SPEC-057a:**
- `NetworkConfig`, `ConnectionConfig`, `TlsConfig`
- `ConnectionId`, `ConnectionKind`, `OutboundMessage`, `SendError`
- `ConnectionHandle`, `ConnectionMetadata`, `ConnectionRegistry`
- `HealthState`, `ShutdownController`, `InFlightGuard`

## Task

Create the `handlers` submodule under `packages/server-rust/src/network/` with health, WebSocket, and HTTP sync handlers. Define the `AppState` struct that carries shared state through axum's state extraction.

### Approach

1. Define `AppState` struct (in `handlers/mod.rs` or a dedicated file)
2. Create `handlers/health.rs` with health_handler, liveness_handler, readiness_handler
3. Create `handlers/websocket.rs` with ws_upgrade_handler, handle_socket, outbound_task (including message coalescing)
4. Create `handlers/http_sync.rs` with http_sync_handler stub
5. Create `handlers/mod.rs` with re-exports
6. Update `network/mod.rs` to add `pub mod handlers;`
7. Add unit tests for health endpoint responses and handler logic

## Requirements

### File Organization

```
network/
  handlers/
    mod.rs               # AppState definition + re-exports
    health.rs            # health_handler, liveness_handler, readiness_handler
    websocket.rs         # ws_upgrade_handler, handle_socket, outbound_task
    http_sync.rs         # http_sync_handler (stub)
```

### AppState (shared axum state)

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

AppState is the axum State extractor type. It holds Arc references to shared resources. It will grow as future specs add fields (e.g., OperationService, ClusterState).

### `handlers/health.rs`

Three axum handler functions:

**`health_handler(State(state): State<AppState>) -> impl IntoResponse`**
- Returns JSON: `{ "state": "<health_state>", "connections": <count>, "in_flight": <count>, "uptime_secs": <seconds> }`
- `state` field is the string representation of `HealthState` (e.g., "starting", "ready", "draining", "stopped") -- use lowercase
- `connections` comes from `state.registry.count()`
- `in_flight` comes from `state.shutdown.in_flight_count()`
- `uptime_secs` is `state.start_time.elapsed().as_secs()`
- Always returns 200 with Content-Type: application/json

**Implementation note on HealthState serialization:** `HealthState` is defined in SPEC-057a's `shutdown.rs`. The idiomatic approaches for producing lowercase strings are: (a) add a `Display` impl that matches on each variant and writes the lowercase name, or (b) add `#[serde(rename_all = "lowercase")]` to the `HealthState` derive and use `serde_json::to_string` for serialization. If approach (b) is chosen, the implementer may need to add `#[derive(Serialize)]` to `HealthState` in `shutdown.rs` (a small update to the SPEC-057a file). Either approach is acceptable; the spec does not mandate which one to use.

**`liveness_handler() -> StatusCode`**
- Always returns `200 OK`
- No state needed (Kubernetes liveness probe)

**`readiness_handler(State(state): State<AppState>) -> StatusCode`**
- Returns `200 OK` if `state.shutdown.health_state() == HealthState::Ready`
- Returns `503 Service Unavailable` otherwise (Starting, Draining, Stopped)

### `handlers/websocket.rs`

**`ws_upgrade_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse`**
- Configures WebSocket write buffer sizes from `state.config.connection`:
  - `ws_write_buffer_size` -> write_buffer_size
  - `ws_max_write_buffer_size` -> max_write_buffer_size
- Calls `ws.on_upgrade(|socket| handle_socket(socket, state))`

**Implementation note on axum 0.8 WebSocket API:** The implementer should verify axum 0.8's exact method names for configuring write buffer sizes on `WebSocketUpgrade`. Method names (e.g., `write_buffer_size()`, `max_write_buffer_size()`) should be confirmed against the axum 0.8 changelog or docs.rs, as they may differ slightly from the names used in this spec.

**`handle_socket(socket: WebSocket, state: AppState)`**
- Registers connection in `state.registry` with `ConnectionKind::Client` and `state.config.connection`
- Gets back `(Arc<ConnectionHandle>, mpsc::Receiver<OutboundMessage>)`
- Splits socket into sender and receiver (`socket.split()`)
- Spawns `outbound_task(sender, rx)` as a tokio task
- Runs inbound loop:
  - Reads messages from receiver
  - Binary messages: logs "received N bytes from connection {id}" (stub -- no OperationService dispatch yet)
  - Close messages: breaks loop
  - Ping: handled automatically by axum
  - Text messages: logs warning and ignores (TopGun uses binary MsgPack only)
- On loop exit (disconnect): removes connection from registry via `state.registry.remove(handle.id)`

**Implementation note on `socket.split()` imports:** `socket.split()` returns a `(SplitSink, SplitStream)` pair and requires `StreamExt` to be in scope. The implementer should check whether axum re-exports `StreamExt` from `futures_util` or whether `futures-util` must be added as an explicit dependency in `Cargo.toml` (e.g., `futures-util = { version = "0.3", default-features = false, features = ["sink"] }`).

**`outbound_task(mut sender: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<OutboundMessage>)`**
- Reads from mpsc::Receiver, writes to WS sender
- Message coalescing: after receiving first message, checks `rx.try_recv()` for additional ready messages and batches them (sends all available without waiting)
- Handles `OutboundMessage::Binary(data)` -> sends as WS Binary message
- Handles `OutboundMessage::Close(reason)` -> sends WS Close frame and exits
- Exits when receiver channel is closed (sender dropped) or after sending Close
- On exit: closes WS sender gracefully

### `handlers/http_sync.rs`

**`http_sync_handler(State(state): State<AppState>, body: Bytes) -> impl IntoResponse`**
- Stub implementation: accepts MsgPack body (raw bytes), returns empty MsgPack response
- Response body: `rmp_serde::to_vec_named(&Vec::<()>::new()).unwrap()` (empty MsgPack array)
- Response Content-Type: `application/msgpack`
- Full implementation depends on OperationService (future spec)

### `handlers/mod.rs`

```rust
pub mod health;
pub mod http_sync;
pub mod websocket;

// Re-export handler functions for convenient access
pub use health::{health_handler, liveness_handler, readiness_handler};
pub use http_sync::http_sync_handler;
pub use websocket::ws_upgrade_handler;

// AppState definition
#[derive(Clone)]
pub struct AppState { /* ... */ }
```

### Update `network/mod.rs`

Add `pub mod handlers;` and `pub use handlers::AppState;` to existing mod.rs.

### Files Modified

| File | Action |
|------|--------|
| `packages/server-rust/src/network/handlers/mod.rs` | Create: AppState + re-exports |
| `packages/server-rust/src/network/handlers/health.rs` | Create: health endpoint handlers |
| `packages/server-rust/src/network/handlers/websocket.rs` | Create: WS upgrade + socket handler + outbound_task |
| `packages/server-rust/src/network/handlers/http_sync.rs` | Create: POST /sync stub handler |

**Total: 4 new files** (within Rust 5-file limit)

**Note:** `network/mod.rs` is also updated to add `pub mod handlers;`, but this is a one-line addition, not a new file.

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
2. `GET /health` returns JSON with `state`, `connections`, `in_flight`, `uptime_secs` fields (unit test with axum test utilities)
3. `GET /health/live` always returns `200`; `GET /health/ready` returns `200` when Ready, `503` when not Ready
4. `GET /ws` successfully upgrades to WebSocket; connection appears in `ConnectionRegistry` (test via axum WS test client or documented as integration test for SPEC-057c)
5. WebSocket disconnect (client-initiated close) removes connection from `ConnectionRegistry`
6. `POST /sync` accepts a request body and returns MsgPack response with correct content-type header (`application/msgpack`)

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

1. **axum 0.8** is the target version. WebSocketUpgrade and State extractors follow axum 0.8 API.
2. **tokio-tungstenite** does not need to be a direct dependency -- axum's `ws` feature provides WebSocket support internally.
3. **AppState** will grow as future specs add fields (e.g., OperationService, ClusterState). For now it contains only registry, shutdown, config, and start_time.
4. **Message coalescing** (batching multiple outbound messages into single WS writes) is included in the outbound task implementation as described in RES-006, not deferred.
5. **`POST /sync` stub** returns an empty MsgPack array. Full implementation requires OperationService.
6. **Health state string format** uses lowercase (e.g., "ready" not "Ready") for JSON serialization consistency.
7. **Handler tests** that require a full running server (e.g., actual WS connections) may be deferred to SPEC-057c integration tests. Unit tests here use axum's `TestClient` or similar where feasible.

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `handlers/mod.rs` with AppState definition and module declarations | -- | ~10% |
| G2 | 1 | Create `handlers/health.rs` (health_handler, liveness_handler, readiness_handler) | G1 | ~20% |
| G3 | 2 | Create `handlers/websocket.rs` (ws_upgrade_handler, handle_socket, outbound_task with coalescing) | G1 | ~40% |
| G4 | 2 | Create `handlers/http_sync.rs` (http_sync_handler stub), update `network/mod.rs` with `pub mod handlers`, add unit tests | G1, G2 | ~30% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |

**Note on Wave 1 parallelism:** G2 depends on G1 (health.rs uses AppState defined in handlers/mod.rs). In practice, both are Wave 1 and G1 is small (~10% context), so G2 can start after G1 completes within the same wave. Workers should execute G1 first, then G2 can begin.

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~43% total (single worker sequential execution)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (N/A -- handler functions, not message structs)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- AppState has no optional fields)
- [x] Enums used for known value sets (HealthState is already an enum from SPEC-057a)
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()` (specified for http_sync stub)
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- AppState is not serialized; health JSON uses explicit field names)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` (N/A -- no Option fields in handler types)

**Language Profile:**
- File count: 4 new files (within 5-file limit)
- Trait-first: N/A (this spec creates handlers, not traits; SPEC-057a already established the type layer)

**Strategic fit:** Aligned with project goals. Directly implements TODO-064 (Networking Layer), Phase 3.

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire protocol, no f64 for integers, WHY-comments only, axum + tokio stack, max 5 files per spec).

**Comment:** Well-structured specification with clear architecture boundaries between SPEC-057a (types), SPEC-057b (handlers), and SPEC-057c (wiring). Handler signatures, response formats, and edge cases are explicitly documented. The spec correctly defers integration testing to SPEC-057c while requiring unit-testable acceptance criteria.

**Recommendations:**
1. G2 has an implicit dependency on G1 (health.rs uses AppState from handlers/mod.rs) but is listed with no dependencies. Both are Wave 1 so this works in practice, but marking the dependency would be more precise for parallel worker execution.
2. The implementer should verify axum 0.8's exact API for configuring WebSocket write buffer sizes on `WebSocketUpgrade` -- the method names may differ slightly from the spec's description.
3. `socket.split()` requires `StreamExt` from `futures_util`. The implementer should check whether axum re-exports this or if `futures-util` needs to be added as an explicit dependency in `Cargo.toml`.
4. The spec leaves HealthState-to-lowercase-string conversion as an implementation detail. A `Display` impl or `serde::Serialize` with `#[serde(rename_all = "lowercase")]` on `HealthState` would be idiomatic approaches.

### Response v1 (2026-02-21)
**Applied:** All 4 recommendations

**Changes:**
1. [✓] G2 dependency on G1 -- Added "G1" to G2's Dependencies column in Task Groups table. Added a note below the Execution Plan table explaining that G2 depends on G1 within Wave 1 and that G1 should complete before G2 begins.
2. [✓] axum 0.8 WebSocket write buffer API note -- Added an "Implementation note on axum 0.8 WebSocket API" paragraph in the `handlers/websocket.rs` requirements section, between ws_upgrade_handler and handle_socket, directing the implementer to verify method names against axum 0.8 docs.rs.
3. [✓] `socket.split()` / `futures_util` StreamExt dependency note -- Added an "Implementation note on `socket.split()` imports" paragraph in the `handlers/websocket.rs` requirements section, after handle_socket, noting that `StreamExt` must be in scope and giving an example `futures-util` Cargo.toml entry.
4. [✓] HealthState-to-lowercase-string guidance -- Added an "Implementation note on HealthState serialization" paragraph in the `handlers/health.rs` requirements section, after the health_handler spec, describing the `Display` impl and `#[serde(rename_all = "lowercase")]` approaches and noting that the latter may require adding `#[derive(Serialize)]` to `HealthState` in SPEC-057a's `shutdown.rs`.

### Audit v2 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~35% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (uptime_secs is u64, in_flight is u64, connections is usize)
- [x] No `r#type: String` on message structs (N/A -- handler functions, not message structs)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- AppState has no optional fields)
- [x] Enums used for known value sets (HealthState is already an enum from SPEC-057a)
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()` (specified for http_sync stub)
- [x] `#[serde(rename_all = "camelCase")]` (N/A -- AppState is not serialized; health JSON uses explicit field names)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` (N/A -- no Option fields in handler types)

**Source Verification:**
All SPEC-057a types verified against actual source code:
- `ConnectionRegistry::register(kind, config)` returns `(Arc<ConnectionHandle>, mpsc::Receiver<OutboundMessage>)` -- matches spec
- `ConnectionRegistry::count()` returns `usize` -- matches spec
- `ConnectionRegistry::remove(id)` returns `Option<Arc<ConnectionHandle>>` -- matches spec
- `ShutdownController::health_state()` returns `HealthState` -- matches spec
- `ShutdownController::in_flight_count()` returns `u64` -- matches spec
- `ConnectionConfig` has `ws_write_buffer_size: usize` and `ws_max_write_buffer_size: usize` fields -- matches spec
- `HealthState` has variants: Starting, Ready, Draining, Stopped -- matches spec
- `Cargo.toml` already has `axum = { version = "0.8", features = ["ws"] }`, `serde_json`, `rmp-serde`, `bytes` -- all required deps present

**Language Profile:** Compliant with Rust profile (4 files, within 5-file limit)

**Strategic fit:** Aligned with project goals. Directly implements TODO-064 (Networking Layer), Phase 3.

**Project compliance:** Honors PROJECT.md decisions (MsgPack wire protocol, no f64 for integers, WHY-comments only, axum + tokio stack, max 5 files per spec).

**Comment:** Re-audit after all 4 v1 recommendations were applied. The spec is well-structured and ready for implementation. All SPEC-057a dependency types have been verified against actual source code. The implementation notes for axum 0.8 API, futures-util imports, and HealthState serialization give the implementer appropriate flexibility while flagging potential pitfalls. No critical issues found.

## Execution Summary

**Executed:** 2026-02-21
**Mode:** orchestrated (sequential fallback -- subagent CLI unavailable)
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |

### Files Created

- `packages/server-rust/src/network/handlers/mod.rs` -- AppState definition + module re-exports
- `packages/server-rust/src/network/handlers/health.rs` -- health_handler, liveness_handler, readiness_handler (9 tests)
- `packages/server-rust/src/network/handlers/websocket.rs` -- ws_upgrade_handler, handle_socket, outbound_task with coalescing
- `packages/server-rust/src/network/handlers/http_sync.rs` -- http_sync_handler stub with MsgPack response (3 tests)

### Files Modified

- `packages/server-rust/src/network/mod.rs` -- added `pub mod handlers;` and `pub use handlers::AppState;`
- `packages/server-rust/src/network/shutdown.rs` -- added `as_str()` method and `Display` impl to `HealthState`
- `packages/server-rust/Cargo.toml` -- added `futures-util` dependency for `StreamExt`/`SinkExt` split support

### Acceptance Criteria Status

- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
- [x] AC2: `GET /health` returns JSON with `state`, `connections`, `in_flight`, `uptime_secs` fields (unit tested)
- [x] AC3: `GET /health/live` always returns 200; `GET /health/ready` returns 200 when Ready, 503 when not Ready (unit tested)
- [x] AC4: WebSocket upgrade handler implemented with connection registration in `ConnectionRegistry` (integration testing deferred to SPEC-057c)
- [x] AC5: WebSocket disconnect removes connection from `ConnectionRegistry` (implemented in handle_socket)
- [x] AC6: `POST /sync` accepts request body and returns MsgPack response with correct content-type (unit tested)

### Deviations

- HealthState serialization uses `as_str()` method + `Display` impl instead of `serde(rename_all = "lowercase")` approach. Both are acceptable per spec.
- `health_handler` return type changed from `impl IntoResponse` to `Json<serde_json::Value>` to enable direct unit testing of the JSON response body.

### Test Summary

- 12 new tests added (9 health, 3 http_sync)
- 43 total tests pass (31 from SPEC-057a + 12 new)
- Build: clean
- Clippy: clean (zero warnings with `-D warnings`)

### Commits

| Hash | Message |
|------|---------|
| 9f75504 | feat(server-rust): add handlers module with AppState definition |
| e4944b0 | feat(server-rust): add health endpoint handlers |
| 957d443 | feat(server-rust): add WebSocket handler with split socket and coalescing |
| 1021408 | feat(server-rust): add HTTP sync handler and fix clippy warnings |

---

## Review History

### Review v1 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: Build compiles with zero errors, zero clippy warnings (`cargo check`, `cargo clippy -- -D warnings` both clean)
- [x] AC2: `health_handler` returns JSON with all 4 required fields (`state`, `connections`, `in_flight`, `uptime_secs`), verified by 5 unit tests covering all health states and dynamic counts
- [x] AC3: `liveness_handler` always returns 200; `readiness_handler` returns 200 when Ready, 503 when Starting/Draining -- verified by 3 unit tests
- [x] AC4: `ws_upgrade_handler` implemented; `handle_socket` registers connection via `state.registry.register(ConnectionKind::Client, ...)` -- integration test deferred to SPEC-057c per spec Assumption 7
- [x] AC5: `handle_socket` calls `state.registry.remove(conn_id)` after inbound loop exits, plus aborts the outbound task to prevent leaked tasks
- [x] AC6: `http_sync_handler` returns MsgPack body with `application/msgpack` content-type -- verified by 3 unit tests (status, content-type, body validity)
- [x] All 4 new files exist: `handlers/mod.rs`, `handlers/health.rs`, `handlers/websocket.rs`, `handlers/http_sync.rs`
- [x] `network/mod.rs` updated with `pub mod handlers;` and `pub use handlers::AppState;`
- [x] `shutdown.rs` updated with `as_str()` const method and `Display` impl -- idiomatic, `#[must_use]` annotated
- [x] `futures-util` added to `Cargo.toml` with minimal feature flags (`default-features = false, features = ["sink"]`)
- [x] 43/43 tests pass (12 new + 31 from SPEC-057a)
- [x] No spec/phase/bug references in code comments -- all comments are WHY-comments
- [x] No `f64` for integer-semantic fields
- [x] No unnecessary `.clone()` calls
- [x] No `unsafe` blocks
- [x] `rmp_serde::to_vec_named()` used (not `to_vec()`) for wire compatibility
- [x] Message coalescing implemented with `try_recv()` loop + `flush()` after draining
- [x] Socket split pattern correctly separates sender (outbound task) and receiver (inbound loop)
- [x] `outbound_handle.abort()` prevents leaked tasks on disconnect
- [x] Error handling in websocket.rs: send errors break the loop, WS errors on inbound break the loop, channel close exits outbound
- [x] `health_handler` return type `Json<serde_json::Value>` is a reasonable deviation enabling direct unit testing -- functionally equivalent to `impl IntoResponse`
- [x] `HealthState::as_str()` is `const fn` returning `&'static str` -- zero-cost abstraction
- [x] AppState struct matches spec exactly (4 fields: registry, shutdown, config, start_time)
- [x] File count: 4 new files, within 5-file Rust spec limit
- [x] Architecture boundary respected: handlers are pure functions, no router/middleware wiring (SPEC-057c scope)

**Minor:**
1. Duplicate `test_state()` helper in both `health.rs` and `http_sync.rs` test modules. This is a common Rust pattern since test helpers are module-private, but could be extracted to a shared test fixture if the handlers module grows further.
2. The single `.expect()` in production code (`http_sync.rs:34`) is acceptable since serializing `Vec::<()>::new()` is infallible, and the message is descriptive. This is an improvement over the spec's suggested `.unwrap()`.

**Summary:** Clean, well-structured implementation that faithfully follows the specification. All 6 acceptance criteria are met. Code quality is high: idiomatic Rust with proper error handling, no unnecessary allocations, good WHY-comments, and comprehensive test coverage (12 new tests). The WebSocket handler demonstrates solid async patterns (socket split, message coalescing with flush, task abort on disconnect). The two noted deviations from spec (return type change, `as_str()` instead of serde) are both improvements. No critical or major issues found.

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 4
**Audit Cycles:** 2
**Review Cycles:** 1
