# SPEC-057a: Networking Types, Config, and Core Abstractions

---
id: SPEC-057a
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-20
parent: SPEC-057
depends_on: [SPEC-052e, SPEC-056]
todo_ref: TODO-064
---

## Context

TopGun's Rust server is beginning Phase 3 -- the networking foundation. This sub-spec delivers the foundational types, configuration structs, connection management abstractions, and shutdown controller that all subsequent networking code depends on. No HTTP handlers, no middleware, no router -- just the pure data types and state management primitives.

The design is fully informed by RES-006 (`RUST_NETWORKING_PATTERNS.md`), which researched connection management, backpressure, and graceful shutdown patterns. Key decisions from that research:
- Bounded mpsc channel (256 cap) for per-connection backpressure
- DashMap-based ConnectionRegistry for 10K+ concurrent connections
- ArcSwap-based HealthState for lock-free state transitions
- ConnectionHandle abstraction with try_send/send_timeout

**Architecture Boundary:** This spec delivers types and abstractions BELOW the Operation pipeline. No message decoding, no handler logic, no routing -- those belong to SPEC-057b (handlers) and SPEC-057c (wiring).

**Prior Art:** `.specflow/reference/RUST_NETWORKING_PATTERNS.md` (RES-006)

## Task

Create the foundational `network` module in `packages/server-rust/src/` with configuration structs, connection management types, and shutdown controller. Update `Cargo.toml` with all networking dependencies needed by the full SPEC-057 chain.

### Approach

1. Add all networking dependencies to `packages/server-rust/Cargo.toml` (axum, tokio features, tower, tower-http, dashmap, arc-swap, etc.)
2. Create `network/config.rs` with NetworkConfig, TlsConfig, ConnectionConfig -- all with Default impls
3. Create `network/connection.rs` with ConnectionId, ConnectionKind, OutboundMessage, SendError, ConnectionHandle, ConnectionMetadata, ConnectionRegistry
4. Create `network/shutdown.rs` with HealthState, ShutdownController, InFlightGuard
5. Create `network/mod.rs` with re-exports of all public types
6. Add unit tests for ConnectionRegistry operations, backpressure, HealthState transitions, and Default values

## Requirements

### File Organization

All files live under `packages/server-rust/src/network/`:

```
network/
  mod.rs                 # Re-exports all public types
  config.rs              # NetworkConfig, TlsConfig, ConnectionConfig
  connection.rs          # ConnectionId, ConnectionKind, ConnectionHandle,
                         #   ConnectionMetadata, ConnectionRegistry, OutboundMessage, SendError
  shutdown.rs            # ShutdownController, HealthState, InFlightGuard
```

### Dependency Additions (`Cargo.toml`)

Add to `packages/server-rust/Cargo.toml`:

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

Note: Some of these dependencies (axum, tower-http, etc.) will not be used until SPEC-057b/c, but adding them all now avoids Cargo.toml churn across specs.

### Types: `config.rs`

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

All config structs implement `Default` with the values shown above. TlsConfig does not implement Default (no sensible defaults for cert paths).

### Types: `connection.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Client,
    ClusterPeer,
}

#[derive(Debug)]
pub enum OutboundMessage {
    Binary(Vec<u8>),
    Close(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    Timeout,
    Disconnected,
    Full,
}

#[derive(Debug)]
pub struct ConnectionHandle {
    pub id: ConnectionId,
    pub tx: mpsc::Sender<OutboundMessage>,
    pub metadata: Arc<RwLock<ConnectionMetadata>>,
    pub connected_at: Instant,
    pub kind: ConnectionKind,
}

#[derive(Debug)]
pub struct ConnectionMetadata {
    pub authenticated: bool,
    pub principal: Option<Principal>,
    pub subscriptions: HashSet<String>,
    pub topics: HashSet<String>,
    pub last_heartbeat: Instant,
    pub last_hlc: Option<Timestamp>,
    pub peer_node_id: Option<String>,
}

impl Default for ConnectionMetadata {
    fn default() -> Self {
        Self {
            authenticated: false,
            principal: None,
            subscriptions: HashSet::new(),
            topics: HashSet::new(),
            last_heartbeat: Instant::now(),
            last_hlc: None,
            peer_node_id: None,
        }
    }
}

#[derive(Debug)]
pub struct ConnectionRegistry {
    connections: DashMap<ConnectionId, Arc<ConnectionHandle>>,
    next_id: AtomicU64,
}
```

**Note on `Principal`:** If a `Principal` type does not yet exist in `server-rust`, use `String` as a placeholder. The auth system is a future spec.

**Note on `Timestamp`:** Use the `Timestamp` type from `topgun-core` if available, otherwise `u64`.

**ConnectionHandle methods:**
- `try_send(&self, msg: OutboundMessage) -> bool` -- non-blocking send via `tx.try_send()`, returns false if channel full or disconnected
- `send_timeout(&self, msg: OutboundMessage, timeout: Duration) -> Result<(), SendError>` -- async send with `tokio::time::timeout`, maps errors to SendError variants
- `is_connected(&self) -> bool` -- checks `!tx.is_closed()`

**ConnectionRegistry methods:**
- `new() -> Self` -- creates empty registry with next_id starting at 1
- `register(&self, kind: ConnectionKind, config: &ConnectionConfig) -> (Arc<ConnectionHandle>, mpsc::Receiver<OutboundMessage>)` -- atomically increments next_id, creates bounded mpsc channel with config.outbound_channel_capacity, creates ConnectionHandle with default metadata (`authenticated: false`, `principal: None`, `subscriptions: HashSet::new()`, `topics: HashSet::new()`, `last_heartbeat: Instant::now()`, `last_hlc: None`, `peer_node_id: None`), inserts into DashMap, returns both ends
- `remove(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>>` -- removes and returns handle
- `get(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>>` -- lookup by ID (returns cloned Arc)
- `count(&self) -> usize` -- `connections.len()`
- `count_by_kind(&self, kind: ConnectionKind) -> usize` -- iterates and filters by kind
- `iter(&self) -> impl Iterator<Item = Arc<ConnectionHandle>>` -- iterate all connections (clones Arcs)
- `broadcast(&self, msg_bytes: Vec<u8>, kind: ConnectionKind)` -- iterates connections of given kind, calls `try_send(Binary(msg_bytes.clone()))` on each; skips full channels without blocking
- `drain_all(&self) -> Vec<Arc<ConnectionHandle>>` -- removes and returns all connections (for shutdown)

### Types: `shutdown.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    Starting,
    Ready,
    Draining,
    Stopped,
}

#[derive(Debug)]
pub struct ShutdownController {
    shutdown_signal: watch::Sender<bool>,
    in_flight: Arc<AtomicU64>,
    health_state: Arc<ArcSwap<HealthState>>,
}

#[derive(Debug)]
pub struct InFlightGuard {
    in_flight: Arc<AtomicU64>,
}
```

**InFlightGuard:** RAII guard. Constructor increments `in_flight` counter (fetch_add). Drop implementation decrements (fetch_sub). This ensures in-flight count is always accurate even if handlers panic.

**ShutdownController methods:**
- `new() -> Self` -- initial state: Starting, shutdown_signal: false, in_flight: 0
- `set_ready(&self)` -- stores HealthState::Ready via ArcSwap
- `shutdown_receiver(&self) -> watch::Receiver<bool>` -- clones the watch receiver for listeners
- `trigger_shutdown(&self)` -- stores HealthState::Draining, sends true on shutdown_signal
- `health_state(&self) -> HealthState` -- loads current state from ArcSwap
- `health_state_handle(&self) -> Arc<ArcSwap<HealthState>>` -- returns clone of Arc for sharing with handlers
- `in_flight_guard(&self) -> InFlightGuard` -- creates new InFlightGuard (increments counter)
- `in_flight_count(&self) -> u64` -- loads current in_flight count
- `wait_for_drain(&self, timeout: Duration) -> bool` -- async: polls in_flight_count until 0 or timeout expires; on successful drain (in_flight reaches 0), stores HealthState::Stopped via ArcSwap and returns true; on timeout, returns false without changing state

### `mod.rs` Re-exports

The `network/mod.rs` file declares submodules and re-exports all public types:

```rust
pub mod config;
pub mod connection;
pub mod shutdown;

pub use config::*;
pub use connection::*;
pub use shutdown::*;
```

Note: `pub mod handlers;`, `pub mod middleware;`, and `pub mod module;` will be added by SPEC-057b and SPEC-057c respectively.

### Files Modified

| File | Action |
|------|--------|
| `packages/server-rust/Cargo.toml` | Add networking dependencies |
| `packages/server-rust/src/lib.rs` | Add `pub mod network;` declaration |
| `packages/server-rust/src/network/mod.rs` | Create: re-exports |
| `packages/server-rust/src/network/config.rs` | Create: NetworkConfig, TlsConfig, ConnectionConfig |
| `packages/server-rust/src/network/connection.rs` | Create: ConnectionHandle, ConnectionRegistry, etc. |
| `packages/server-rust/src/network/shutdown.rs` | Create: ShutdownController, HealthState, InFlightGuard |

**Total: 4 new files + 2 modified = 6 file touches** (1 over Rust 5-file limit; see Audit v1 note -- `lib.rs` is a 1-line change bundled with G1)

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings after adding all dependencies and types
2. `ConnectionRegistry::register()` creates a `ConnectionHandle` with a bounded mpsc channel of capacity 256 (configurable via `ConnectionConfig`)
3. `ConnectionHandle::try_send()` returns `false` when channel is full (unit test: send 257 messages to 256-capacity channel)
4. `ConnectionRegistry::count()` accurately reflects active connections after register/remove operations
5. `HealthState` transitions are correct: `Starting -> Ready` (via `set_ready()`), `Ready -> Draining` (via `trigger_shutdown()`), `Draining -> Stopped` (via `wait_for_drain()` setting Stopped on successful drain)
6. All `Default` implementations produce the values specified in Requirements (256 channel cap, 5s send timeout, 60s idle timeout, 128KB write buffer, 512KB max write buffer, port 0, host "0.0.0.0", cors_origins ["*"], request_timeout 30s)
7. `broadcast()` sends to all connections of specified kind; skips full channels without blocking

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
3. **AppState** will be defined in SPEC-057b (handlers scope). This spec only defines the types that AppState will contain.
4. **TLS testing** is deferred -- TlsConfig struct is created but TLS integration tests require certificate generation which is out of scope.
5. **Port 0** is used in tests for OS-assigned ports to avoid port conflicts.
6. **Principal type** -- if not yet defined in server-rust, use `String` as a placeholder type alias.

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add all networking dependencies to `Cargo.toml`, add `pub mod network` to `lib.rs`, create `network/mod.rs` (initial re-exports), create `config.rs` (NetworkConfig, TlsConfig, ConnectionConfig with Default impls) | -- | ~15% |
| G2 | 1 | Create `connection.rs` (ConnectionId, ConnectionKind, OutboundMessage, SendError, ConnectionHandle, ConnectionMetadata, ConnectionRegistry with all methods). Note: Wave 1 contains implementations (not just types) because this entire spec IS the foundational types layer -- there are no higher-level consumers within this spec to separate into Wave 2. | -- | ~20% |
| G3 | 2 | Create `shutdown.rs` (HealthState, ShutdownController, InFlightGuard), update `mod.rs` re-exports, add unit tests for all types | G1, G2 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-20)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total (realistic; spec's own estimates of 100% are overstated)

**Critical:**
1. **Missing `lib.rs` modification:** The spec omits adding `pub mod network;` to `packages/server-rust/src/lib.rs`. Without this declaration, the network module will not compile. This brings the file count to 6 (1 over the Rust 5-file limit), but since `lib.rs` is a 1-line addition, it should be bundled with G1 rather than counted as a separate unit of work. The Files Modified table and G1 task description must be updated.
2. **HealthState::Stopped is unreachable:** AC5 references a `Stopped` state "after drain," but no `ShutdownController` method transitions to `Stopped`. `trigger_shutdown()` sets `Draining`, and `wait_for_drain()` returns a bool without changing state. Either: (a) `wait_for_drain()` should store `HealthState::Stopped` on successful drain completion, or (b) a separate `set_stopped(&self)` method should be added to `ShutdownController`. Update AC5 to match the chosen approach.

**Recommendations:**
3. `OutboundMessage` and `SendError` are missing standard derives. `SendError` should have `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` for ergonomic error handling and test assertions. `OutboundMessage` should have at least `#[derive(Debug)]`.
4. `ConnectionMetadata` initial values are not specified for `register()`. Recommend documenting defaults: `authenticated: false`, `principal: None`, `subscriptions: HashSet::new()`, `topics: HashSet::new()`, `last_heartbeat: Instant::now()`, `last_hlc: None`, `peer_node_id: None`.
5. Task Group context estimates are inflated (25% + 40% + 35% = 100%). For a small spec of pure Rust types and straightforward impls, realistic estimates are ~15% + ~20% + ~15% = ~50%. Overstated estimates could trigger unnecessary decomposition.
6. [Compliance] G2 is in Wave 1 and contains both type definitions AND full method implementations (ConnectionRegistry with 9 methods). The Language Profile says "G1 (Wave 1) defines traits/types only." This is acceptable since the entire spec IS the types layer, but consider adding a brief note explaining this intentional deviation.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs
- [x] `Default` derived where appropriate
- [x] Enums used for known value sets (ConnectionKind, HealthState, SendError)
- [x] Wire compatibility: N/A (internal server types, not wire-serialized)
- [x] `#[serde(rename_all = "camelCase")]`: N/A (not wire-serialized)
- [x] `#[serde(skip_serializing_if...)]`: N/A (not wire-serialized)

**Strategic fit:** Aligned with project goals -- Phase 3 networking foundation is the correct next step.
**Project compliance:** Honors PROJECT.md decisions (no f64 misuse, enums for value sets, MsgPack wire format preserved).

### Response v1 (2026-02-20)
**Applied:** All (critical issues 1-2 + recommendations 3-6)

**Changes:**
1. [✓] Missing `lib.rs` modification — Added `lib.rs` to Files Modified table, updated file count note, added `pub mod network` to G1 task description
2. [✓] HealthState::Stopped unreachable — Updated AC5 to specify `wait_for_drain()` sets Stopped on successful drain; updated `wait_for_drain` method description in Requirements to match
3. [✓] Missing derives on OutboundMessage/SendError — Added `#[derive(Debug)]` to OutboundMessage, `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` to SendError
4. [✓] ConnectionMetadata initial values undocumented — Added explicit default values to `register()` method description
5. [✓] Inflated context estimates — Deflated from 25%+40%+35%=100% to 15%+20%+15%=50%
6. [✓] Wave 1 trait-first deviation — Added explanatory note to G2 task description

### Audit v2 (2026-02-20)
**Status:** APPROVED

**Context Estimate:** ~50% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`ConnectionId(u64)`, `next_id: AtomicU64`, `in_flight: AtomicU64`, capacity/buffer sizes are `usize`)
- [x] No `r#type: String` on message structs (N/A -- internal types, not wire-serialized)
- [x] `Default` derived where appropriate (NetworkConfig, ConnectionConfig have Default; TlsConfig intentionally omitted)
- [x] Enums used for known value sets (ConnectionKind, HealthState, SendError, OutboundMessage)
- [x] Wire compatibility: N/A (internal server types, not wire-serialized)
- [x] `#[serde(rename_all = "camelCase")]`: N/A (not wire-serialized)
- [x] `#[serde(skip_serializing_if...)]`: N/A (not wire-serialized)

**Strategic fit:** Aligned with project goals -- Phase 3 networking foundation is the correct next step.
**Project compliance:** Honors PROJECT.md decisions. No violations detected.
**Language profile:** Compliant with Rust profile (minor file count deviation of 6 vs 5 acknowledged; `lib.rs` is a 1-line change).

**Recommendations:**
1. `ConnectionHandle` and `ConnectionRegistry` are missing `#[derive(Debug)]`. Both types' inner fields support Debug (DashMap, AtomicU64, mpsc::Sender, Arc all implement Debug). Without Debug, these types cannot appear in tracing spans or error messages. Consider adding `#[derive(Debug)]` to both.
2. `ConnectionMetadata` has 3 Option fields and could benefit from a `Default` impl for test ergonomics, even though the `register()` method documents explicit initial values. This is optional since ConnectionMetadata is not a wire-serialized payload struct.

**Comment:** Well-structured spec with clear architecture boundaries, precise type definitions, and thorough method specifications. All critical issues from Audit v1 have been properly addressed. The spec is ready for implementation.

### Response v2 (2026-02-20)
**Applied:** Both recommendations from Audit v2

**Changes:**
1. [✓] Add `#[derive(Debug)]` to `ConnectionHandle` and `ConnectionRegistry` — Added `#[derive(Debug)]` to both struct definitions in the Types: connection.rs section
2. [✓] Add `Default` impl for `ConnectionMetadata` — Added explicit `impl Default for ConnectionMetadata` block in the Types: connection.rs section with the same initial values already documented for `register()`

### Audit v3 (2026-02-20)
**Status:** NEEDS_REVISION

**Context Estimate:** ~35% total

**Critical:**
1. **`ConnectionMetadata` missing `#[derive(Debug)]` -- compilation failure.** `ConnectionHandle` has `#[derive(Debug)]` and contains `metadata: Arc<RwLock<ConnectionMetadata>>`. Tokio's `RwLock<T>` only implements `Debug` when `T: Debug`. Without `#[derive(Debug)]` on `ConnectionMetadata`, the `#[derive(Debug)]` on `ConnectionHandle` will fail to compile with: "the trait `Debug` is not implemented for `ConnectionMetadata`". This directly violates AC1 (zero compilation errors). Add `#[derive(Debug)]` to `ConnectionMetadata`.

**Recommendations:**
2. `ShutdownController` and `InFlightGuard` have no derive attributes in the spec. For consistency with `ConnectionHandle`/`ConnectionRegistry` (which have `#[derive(Debug)]`) and for tracing/logging usability, consider adding Debug. Note: `watch::Sender<bool>` implements `Debug`, `Arc<AtomicU64>` implements `Debug`, and `Arc<ArcSwap<HealthState>>` implements `Debug` (ArcSwap implements Debug when inner does, and HealthState derives Debug). So `#[derive(Debug)]` should work on both `ShutdownController` and `InFlightGuard`.

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (N/A)
- [x] `Default` derived where appropriate (NetworkConfig, ConnectionConfig, ConnectionMetadata)
- [x] Enums used for known value sets (ConnectionKind, HealthState, SendError, OutboundMessage)
- [x] Wire compatibility: N/A (internal server types)
- [x] `#[serde(rename_all = "camelCase")]`: N/A
- [x] `#[serde(skip_serializing_if...)]`: N/A

**Strategic fit:** Aligned with project goals.
**Project compliance:** Honors PROJECT.md decisions. No violations detected.
**Language profile:** Compliant with Rust profile.

### Response v3 (2026-02-20)
**Applied:** All (critical issue 1 + recommendation 2)

**Changes:**
1. [✓] `ConnectionMetadata` missing `#[derive(Debug)]` — Added `#[derive(Debug)]` to the `ConnectionMetadata` struct definition in Types: connection.rs section
2. [✓] Add `#[derive(Debug)]` to `ShutdownController` and `InFlightGuard` — Added `#[derive(Debug)]` to both struct definitions in Types: shutdown.rs section

### Audit v4 (2026-02-20)
**Status:** APPROVED

**Context Estimate:** ~35% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`ConnectionId(u64)`, `next_id: AtomicU64`, `in_flight: AtomicU64`, capacity/buffer sizes are `usize`)
- [x] No `r#type: String` on message structs (N/A -- internal types, not wire-serialized)
- [x] `Default` derived where appropriate (NetworkConfig, ConnectionConfig, ConnectionMetadata all have Default; TlsConfig intentionally omitted)
- [x] Enums used for known value sets (ConnectionKind, HealthState, SendError, OutboundMessage)
- [x] Wire compatibility: N/A (internal server types, not wire-serialized)
- [x] `#[serde(rename_all = "camelCase")]`: N/A (not wire-serialized)
- [x] `#[serde(skip_serializing_if...)]`: N/A (not wire-serialized)

**Strategic fit:** Aligned with project goals -- Phase 3 networking foundation is the correct next step per roadmap.
**Project compliance:** Honors PROJECT.md decisions. No violations detected.
**Language profile:** Compliant with Rust profile (6 files acknowledged; `lib.rs` is a 1-line change).

**Codebase Verification:**
- `Principal` type confirmed in `topgun-core` (`packages/core-rust/src/types.rs:118`) with `id: String, roles: Vec<String>` -- spec's fallback to `String` is unnecessary but harmless
- `Timestamp` type confirmed in `topgun-core` (`packages/core-rust/src/hlc.rs:161`) and re-exported from crate root -- available via `topgun_core::Timestamp`
- Existing `server-rust/Cargo.toml` has only `topgun-core`, `async-trait`, `anyhow` -- no dependency conflicts with proposed additions
- Existing `server-rust/src/lib.rs` has `pub mod traits;` -- adding `pub mod network;` is straightforward

**Recommendations:**
1. The `config.rs` struct definitions lack explicit `#[derive(...)]` annotations in the spec text, unlike `connection.rs` and `shutdown.rs` which show derives explicitly. For implementer clarity, `NetworkConfig` and `ConnectionConfig` should show `#[derive(Debug, Clone)]` (both are plain data structs with all-Clone fields), and `TlsConfig` should show `#[derive(Debug, Clone)]`. This is cosmetic -- an implementer would naturally add these -- but spec consistency is improved.
2. `ConnectionRegistry::iter()` is specified as returning `impl Iterator<Item = Arc<ConnectionHandle>>`. In practice, DashMap iteration yields `RefMulti` guard types that borrow the map, making it impossible to return `impl Iterator` from a `&self` method without collecting first. The implementer will likely need to return `Vec<Arc<ConnectionHandle>>` or collect internally. This is a minor implementation detail that does not affect the API contract.

**Comment:** Spec is in excellent shape after 3 revision cycles. All derive annotations are now consistent across all struct definitions. Type references (`Principal`, `Timestamp`) are verified against the actual codebase. Method signatures are precise and implementable. The spec is ready for implementation.

## Execution Summary

**Executed:** 2026-02-20
**Mode:** orchestrated (sequential fallback -- subagent CLI unavailable)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/server-rust/src/network/mod.rs` -- module re-exports
- `packages/server-rust/src/network/config.rs` -- NetworkConfig, TlsConfig, ConnectionConfig
- `packages/server-rust/src/network/connection.rs` -- ConnectionId, ConnectionKind, ConnectionHandle, ConnectionRegistry, etc.
- `packages/server-rust/src/network/shutdown.rs` -- HealthState, ShutdownController, InFlightGuard

### Files Modified
- `packages/server-rust/Cargo.toml` -- added networking dependencies (axum, tokio, tower, dashmap, arc-swap, etc.)
- `packages/server-rust/src/lib.rs` -- added `pub mod network;`

### Acceptance Criteria Status
- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero clippy warnings
- [x] AC2: `ConnectionRegistry::register()` creates ConnectionHandle with bounded mpsc channel of configurable capacity (default 256)
- [x] AC3: `ConnectionHandle::try_send()` returns false when channel is full (tested with 257th message on 256-capacity channel equivalent)
- [x] AC4: `ConnectionRegistry::count()` accurately reflects active connections after register/remove operations
- [x] AC5: HealthState transitions correct: Starting -> Ready (set_ready), Ready -> Draining (trigger_shutdown), Draining -> Stopped (wait_for_drain on successful drain)
- [x] AC6: All Default implementations produce specified values (256 channel cap, 5s send timeout, 60s idle timeout, 128KB write buffer, 512KB max write buffer, port 0, host "0.0.0.0", cors_origins ["*"], request_timeout 30s)
- [x] AC7: broadcast() sends to all connections of specified kind; skips full channels without blocking

### Deviations
- `ConnectionRegistry::iter()` renamed to `connections()` per Audit v4 recommendation: clippy warns that a method named `iter` should return an `Iterator`, but DashMap requires collecting into Vec. The method returns `Vec<Arc<ConnectionHandle>>` with the same semantics.
- `ConnectionRegistry::broadcast()` takes `&[u8]` instead of `Vec<u8>` per clippy's `needless_pass_by_value` lint; callers can pass slices or Vec references.
- Added `#[must_use]` annotations on all pure accessor methods per clippy recommendations.
- Added `Default` impl for `ConnectionRegistry` (delegates to `new()`) for ergonomic construction.

### Tests Added (31 total)
- 3 config tests (NetworkConfig defaults, ConnectionConfig defaults, TlsConfig construction)
- 17 connection tests (registry CRUD, backpressure, broadcast, drain, metadata defaults, channel capacity)
- 10 shutdown tests (state transitions, in-flight guards, drain with timeout, shutdown receiver notification)
- 1 pre-existing crate_loads test

---

## Review History

### Review v1 (2026-02-21 08:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Verification Results:**

**Build check:** `cargo build -p topgun-server` -- passed (zero errors)
**Lint check:** `cargo clippy -p topgun-server -- -D warnings` -- passed (zero warnings)
**Test check:** `cargo test -p topgun-server` -- passed (31/31 tests pass)

**Findings:**

**Minor:**
1. Execution Summary states "18 connection tests" but actual count from test output is 17. The `crate_loads` test (pre-existing, not part of this spec) was likely miscounted. This is a documentation-only issue in the Execution Summary, not a code defect.

**Passed:**
- [x] AC1: Build compiles with zero errors and zero clippy warnings -- verified via `cargo build` and `cargo clippy`
- [x] AC2: `ConnectionRegistry::register()` creates bounded mpsc channel with configurable capacity -- verified in `connection.rs:176` using `mpsc::channel(config.outbound_channel_capacity)`, tested by `register_uses_configured_channel_capacity`
- [x] AC3: `ConnectionHandle::try_send()` returns `false` when channel full -- verified in `connection.rs:75-77`, tested by `connection_handle_try_send_full` (fills capacity-2 channel, 3rd send returns false)
- [x] AC4: `ConnectionRegistry::count()` accurate after register/remove -- verified in `connection.rs:202-204`, tested by `registry_register_and_count` and `registry_remove`
- [x] AC5: HealthState transitions correct (Starting->Ready->Draining->Stopped) -- verified via `set_ready()`, `trigger_shutdown()`, `wait_for_drain()` in `shutdown.rs`, tested by `health_state_transitions_starting_ready_draining`, `wait_for_drain_immediate_success`, `wait_for_drain_timeout`
- [x] AC6: All Default values match spec -- verified in `config.rs:24-33` (NetworkConfig) and `config.rs:65-73` (ConnectionConfig), tested by `network_config_defaults` and `connection_config_defaults`
- [x] AC7: `broadcast()` sends to specified kind, skips full channels -- verified in `connection.rs:231-239`, tested by `broadcast_to_specific_kind` and `broadcast_skips_full_channels`
- [x] All 4 new files exist: `network/mod.rs`, `network/config.rs`, `network/connection.rs`, `network/shutdown.rs`
- [x] Both modified files correct: `Cargo.toml` has all 14 dependencies, `lib.rs` has `pub mod network;`
- [x] No files to delete (greenfield spec)
- [x] No spec/phase/bug references in code comments -- only WHY-comments present
- [x] No `f64` for integer-semantic fields -- `ConnectionId(u64)`, `AtomicU64` for counters, `usize` for sizes
- [x] No `unwrap()` in production code (only in test assertions)
- [x] No `unsafe` blocks
- [x] Proper use of `topgun_core::Principal` and `topgun_core::Timestamp` (not placeholder String/u64)
- [x] `Ordering::Relaxed` appropriate for all atomic operations (counter semantics only, no ordering constraints needed)
- [x] `#[must_use]` annotations on all pure accessor methods per idiomatic Rust
- [x] `Default` impls for `ConnectionRegistry` and `ShutdownController` for ergonomic construction
- [x] Doc comments on all public types and methods with `///` syntax
- [x] Module-level doc comments with `//!` syntax on all files
- [x] All deviations from spec are justified by clippy lints and documented in Execution Summary
- [x] Code follows existing codebase patterns (same style as `packages/core-rust/`)
- [x] No unnecessary `.clone()` calls -- only `Arc::clone()` which is O(1)
- [x] No code duplication -- test helpers (`test_config()`, `small_channel_config()`) used for DRY test setup
- [x] Naming is clear and consistent (`ConnectionHandle`, `ConnectionRegistry`, `ShutdownController`)
- [x] No unnecessary abstractions -- types are direct and purposeful
- [x] Architecture boundary respected -- no message decoding, no handler logic, no routing

**Implementation Reality Check:** No strategic concerns. The implementation is straightforward foundational types as the spec intended. Complexity is proportional to the task. No obvious better approach was missed.

**Summary:** Clean, well-structured implementation that faithfully follows the specification. All 7 acceptance criteria are met. All 31 tests pass. Build and clippy are clean. The code uses proper Rust idioms (RAII guards, `#[must_use]`, proper atomic orderings, `Arc::clone` for clarity). The 4 documented deviations from spec (`iter` -> `connections`, `Vec<u8>` -> `&[u8]`, `#[must_use]` additions, `Default` for `ConnectionRegistry`) are all improvements driven by clippy lints. The only finding is a trivial test count discrepancy in the Execution Summary documentation.

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 2
**Audit Cycles:** 4
**Review Cycles:** 1
