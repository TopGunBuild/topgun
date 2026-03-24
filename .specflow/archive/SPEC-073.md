> **SPLIT:** This specification was decomposed into:
> - SPEC-073a: Server Wiring: WebSocket Dispatch, Auth Handshake, and Test Binary
> - SPEC-073b: TS Test Harness: Spawn Rust Server, Client Factory, Cleanup
> - SPEC-073c: Core Integration Tests: Connection, Auth, LWW CRDT, and ORMap
> - SPEC-073d: Query and Pub/Sub Integration Tests
> - SPEC-073e: Search Integration Tests
>
> See child specifications for implementation.

---
id: SPEC-073
type: feature
status: split
priority: P0
complexity: large
created: 2026-03-01
depends_on: [SPEC-071, SPEC-072]
todo_ref: TODO-068
---

# Integration Test Suite: TS Client to Rust Server Behavioral Equivalence

## Context

All 7 domain services (Coordination, CRDT, Sync, Messaging, Query, Persistence, Search) are implemented in the Rust server. PostgreSQL persistence, write validation, and observability are complete. The Rust server has 467 passing unit tests.

However, no end-to-end test has ever verified that a TS client can connect to the Rust server, authenticate, send operations, and receive correct responses over WebSocket. This spec creates the integration test suite that gates the `rust-migration` branch merge into `main` and the v0.12.0-rc.1 pre-release.

### Critical Blocker: WebSocket Message Dispatch

The Rust server's WebSocket handler (`packages/server-rust/src/network/handlers/websocket.rs`, line 62-63) currently contains a **stub** for inbound message processing:

```rust
// Stub: no OperationService dispatch yet. Binary messages
// are logged but not processed.
```

Before any integration test can pass, this stub must be replaced with actual message dispatch: deserialize MsgPack binary -> classify via `OperationService` -> route through operation pipeline -> serialize response -> send via outbound channel. Additionally, the auth flow (AUTH_REQUIRED on connect, AUTH/AUTH_ACK handshake) must be wired.

### No Binary Entry Point

The Rust server has no `main.rs` -- only `lib.rs`. The integration test harness needs either:
- A `main.rs` binary that tests spawn as a subprocess, OR
- A Rust integration test binary that constructs and starts the server programmatically, then exposes the port for TS test clients

### Test Strategy

The existing 13 e2e test files (`tests/e2e/`) use a TS test client (`createTestClient`) that connects via raw WebSocket with MsgPack serialization. Many existing tests verify behavior by inspecting server internal state (`server.getMap()`) -- this is not possible with the Rust server from TS.

The integration test approach:
1. **Wire the Rust server WS handler** to dispatch messages through the operation pipeline
2. **Create a Rust test binary** (`packages/server-rust/src/bin/test_server.rs`) that starts a fully-wired server on port 0 and prints the bound port to stdout
3. **Create a TS test harness** (`tests/integration-rust/helpers/`) that spawns the Rust binary, captures the port, and provides `createRustTestClient()` / `createRustTestContext()`
4. **Port e2e test scenarios** to `tests/integration-rust/` that use the harness, verifying behavior purely through message exchange (no `server.getMap()` calls)

### Source Tests

The following TS e2e tests define the behavioral contract to verify:

| File | Scenarios | Domain |
|------|-----------|--------|
| `basic-sync.test.ts` | Connection, auth, LWW write/read, ORMap ops, multi-client sync, topic pub/sub | Core |
| `pubsub.test.ts` | Topic sub/unsub, multi-subscriber, message types, ordering, edge cases | Messaging |
| `live-queries.test.ts` | Query sub, filtering, comparison operators, sorting, limits, multi-client, unsubscribe | Query |
| `offline-online.test.ts` | OP_BATCH sync, OP_ACK, reconnect merge, conflict resolution, HLC ordering | CRDT + Sync |
| `merkle-sync.test.ts` | Full sync on connect, delta sync on reconnect | Sync |
| `fulltext-search.test.ts` | One-shot search, live search sub, ENTER/UPDATE/LEAVE, multi-client | Search |
| `pncounter.test.ts` | PN counter increment/decrement/merge | CRDT |
| `json-fallback.test.ts` | JSON serialization fallback | Serialization |
| `cluster/replication.test.ts` | 3-node cluster, write replication, consistency levels | Cluster |
| `cluster/node-failure.test.ts` | Node failure, failover | Cluster |
| `cluster/partition-routing.test.ts` | Partition routing | Cluster |
| `security/uat-security-hardening.test.ts` | Rate limiting, JWT validation, HLC drift | Security |

## Goal Analysis

### Goal Statement
Verify that the Rust server produces identical observable behavior to the TS server for all client-facing operations, as measured by TS client integration tests.

### Observable Truths
1. A TS WebSocket client connects to the Rust server and completes the AUTH_REQUIRED -> AUTH -> AUTH_ACK handshake
2. LWW write operations (CLIENT_OP with PUT/REMOVE) are processed and acknowledged with OP_ACK
3. ORMap operations (OR_ADD/OR_REMOVE) are processed correctly
4. Query subscriptions (QUERY_SUB) return correct snapshots and live updates (QUERY_UPDATE with ENTER/UPDATE/LEAVE)
5. Topic pub/sub (TOPIC_SUB/TOPIC_PUB) delivers messages to subscribers, excludes publisher, preserves order
6. Full-text search (SEARCH/SEARCH_SUB) returns BM25-ranked results and live updates
7. OP_BATCH operations are processed atomically with correct OP_ACK.lastId

### Required Artifacts
| Artifact | Purpose |
|----------|---------|
| `packages/server-rust/src/network/handlers/websocket.rs` (modified) | Wire inbound binary messages to operation pipeline dispatch |
| `packages/server-rust/src/network/handlers/auth.rs` (new) | AUTH_REQUIRED/AUTH/AUTH_ACK/AUTH_FAIL handshake logic |
| `packages/server-rust/src/bin/test_server.rs` (new) | Binary entry point that starts fully-wired server, prints port to stdout |
| `tests/integration-rust/helpers/index.ts` (new) | TS harness: spawn Rust binary, capture port, client factory |
| `tests/integration-rust/*.test.ts` (new, ~8 files) | Integration test files organized by domain |

### Required Wiring
- `websocket.rs` -> `OperationService.classify()` -> `build_operation_pipeline()` -> domain services -> response serialization -> `OutboundMessage::Binary` via connection handle
- `auth.rs` -> JWT verification -> `ConnectionRegistry` auth state -> gate message dispatch behind auth
- `test_server.rs` -> `NetworkModule` + `ServiceRegistry` + all domain services wired together + `module.serve()`
- TS harness -> `child_process.spawn('cargo run --bin test-server')` -> capture stdout port -> `createTestClient(port)`

### Key Links (fragile/critical)
1. **MsgPack wire format compatibility**: TS `msgpackr` serialized messages must deserialize correctly via Rust `rmp-serde`. Field naming (`camelCase`), type encoding (u64 vs f64), and `Option` handling must match exactly.
2. **Auth handshake timing**: The Rust server must send AUTH_REQUIRED immediately on WS connect (before any message processing), matching TS server behavior.
3. **BATCH framing**: The Rust server's CoalescingWriter output format (4-byte LE count + [4-byte LE length + MsgPack payload]*) must match what the TS test client's BATCH parser expects.
4. **Query notification delivery**: After a CRDT write, the QueryRegistry must notify active QUERY_SUB subscribers with QUERY_UPDATE messages via their connection handles.

## Task

Create the integration test suite in phases. Due to the large scope (estimated >200k tokens), this spec defines the full architecture and decomposition plan. It MUST be split via `/sf:split` before implementation.

### Phase A: Server Wiring (Rust, ~3-4 files)
Wire the WebSocket handler to dispatch inbound MsgPack messages through the operation pipeline. Implement the auth handshake (AUTH_REQUIRED on connect, JWT verification, AUTH_ACK/AUTH_FAIL). Create the test server binary.

### Phase B: Test Infrastructure (TS, ~2-3 files)
Create the TS test harness that spawns the Rust test server binary, captures the port, provides client factories compatible with the existing `TestClient` interface.

### Phase C: Core Integration Tests (TS, ~3 files)
Port connection/auth, LWW CRDT write/read, and OP_BATCH tests from `basic-sync.test.ts` and `offline-online.test.ts`.

### Phase D: Query + Messaging Tests (TS, ~2-3 files)
Port live query and pub/sub tests from `live-queries.test.ts` and `pubsub.test.ts`.

### Phase E: Search + ORMap Tests (TS, ~2 files)
Port full-text search and ORMap tests from `fulltext-search.test.ts` and `basic-sync.test.ts` (ORMap section).

### Phase F: Cluster Tests (TS, deferred)
Port cluster tests from `cluster/*.test.ts`. These require multi-node Rust server orchestration and are lower priority for initial behavioral equivalence.

## Requirements

### Files to Create

1. **`packages/server-rust/src/network/handlers/auth.rs`** -- Auth handshake module
   - Send `AUTH_REQUIRED` message (MsgPack-serialized) to client immediately on WebSocket connect
   - Receive `AUTH { token }` message, verify JWT signature using configured secret
   - On valid token: mark connection as authenticated in `ConnectionRegistry`, send `AUTH_ACK`
   - On invalid token: send `AUTH_FAIL { reason }`, close connection
   - Reject all non-AUTH messages from unauthenticated connections

2. **`packages/server-rust/src/bin/test_server.rs`** -- Test server binary
   - Construct `NetworkModule` with port 0, `ServiceRegistry` with all 7 domain services wired
   - Start server, print bound port to stdout as `PORT=<number>\n`
   - Handle SIGTERM/SIGINT for graceful shutdown
   - Use `NullDataStore` (no PostgreSQL dependency for tests)
   - Configure JWT secret as `test-e2e-secret` (matches TS test helpers)

3. **`tests/integration-rust/helpers/index.ts`** -- TS test harness
   - `spawnRustServer()`: spawns `cargo run --bin test-server --release`, captures `PORT=\d+` from stdout, returns `{ port, process, cleanup }`
   - `createRustTestClient(port)`: wraps existing `createTestClient` logic with Rust server URL
   - `createRustTestContext(numClients)`: spawns server + N clients with cleanup
   - Timeout: 30s for server startup (includes cargo build if needed)
   - Configurable: `RUST_SERVER_BINARY` env var for pre-built binary path

4. **`tests/integration-rust/connection-auth.test.ts`** -- Connection and auth tests
   - Client connects to Rust server
   - Client receives AUTH_REQUIRED on connect
   - Client receives AUTH_ACK after valid JWT
   - Client receives AUTH_FAIL for invalid JWT
   - Client reconnects after disconnect

5. **`tests/integration-rust/crdt-lww.test.ts`** -- LWW CRDT tests
   - CLIENT_OP PUT writes data, OP_ACK received
   - OP_BATCH processes multiple ops, OP_ACK.lastId correct
   - LWW conflict resolution: later HLC timestamp wins
   - Tombstone (value: null) via REMOVE op
   - HLC ordering: later timestamp wins even if sent first
   - Deterministic tie-breaking by nodeId

6. **`tests/integration-rust/crdt-ormap.test.ts`** -- ORMap tests
   - OR_ADD adds item, OR_REMOVE removes by tag
   - Multiple values per key
   - Tombstone synchronization between clients

7. **`tests/integration-rust/queries.test.ts`** -- Live query tests
   - QUERY_SUB returns initial snapshot
   - QUERY_UPDATE ENTER/UPDATE/LEAVE on data changes
   - Equality and comparison operator filtering ($gt, $lt, $gte, $lte, $ne)
   - Sorting (asc, desc)
   - Limit and pagination
   - QUERY_UNSUB stops updates
   - Multi-client: subscriber receives writer's updates
   - Multiple queries on same collection with different filters

8. **`tests/integration-rust/pubsub.test.ts`** -- Topic pub/sub tests
   - TOPIC_SUB/TOPIC_PUB/TOPIC_MESSAGE flow
   - Publisher excluded from own messages
   - Multiple subscribers all receive
   - TOPIC_UNSUB stops delivery
   - Multiple topics isolation
   - Message ordering preserved
   - Various data types (string, number, boolean, object, array, null)

9. **`tests/integration-rust/search.test.ts`** -- Full-text search tests
   - SEARCH one-shot returns BM25-ranked results
   - SEARCH with limit, minScore, boost options
   - SEARCH_SUB returns initial results and live SEARCH_UPDATE (ENTER/UPDATE/LEAVE)
   - SEARCH_UNSUB stops updates
   - Non-indexed map returns error

### Files to Modify

10. **`packages/server-rust/src/network/handlers/websocket.rs`** -- Replace stub with dispatch
    - Deserialize binary data via `rmp_serde::from_slice::<Message>()`
    - Check auth state on connection; reject non-AUTH messages if unauthenticated
    - Classify message via `OperationService.classify()`
    - Route through `build_operation_pipeline()`
    - Serialize response via `rmp_serde::to_vec_named()` and send as `OutboundMessage::Binary`

11. **`packages/server-rust/src/network/handlers/mod.rs`** -- Add `auth` module export

12. **`packages/server-rust/Cargo.toml`** -- Add `[[bin]]` section for `test-server`

### Interfaces

**Auth state on ConnectionRegistry:**
```rust
// ConnectionRegistry needs auth state per connection
pub struct ConnectionInfo {
    pub kind: ConnectionKind,
    pub authenticated: bool,
    pub user_id: Option<String>,
    pub roles: Vec<String>,
}
```

**Test server stdout protocol:**
```
PORT=12345
```
Single line, TS harness reads with line-buffered parsing, timeout 30s.

## Acceptance Criteria

### Server Wiring (Phase A)
- AC1: Rust WS handler deserializes inbound MsgPack binary messages into `topgun_core::messages::Message`
- AC2: Unauthenticated connections receive `AUTH_REQUIRED` on connect and reject non-AUTH messages
- AC3: Valid JWT token in AUTH message results in AUTH_ACK response
- AC4: Invalid JWT token results in AUTH_FAIL response and connection close
- AC5: Authenticated messages are classified by `OperationService` and routed through operation pipeline
- AC6: Pipeline responses are serialized via `rmp_serde::to_vec_named()` and sent as binary WebSocket frames
- AC7: Test server binary starts on port 0, prints `PORT=<number>` to stdout, and shuts down on SIGTERM

### Test Infrastructure (Phase B)
- AC8: `spawnRustServer()` starts Rust binary, captures port within 30s, returns cleanup function
- AC9: `createRustTestClient(port)` connects via WebSocket, auto-authenticates, returns `TestClient` interface
- AC10: Server process is killed and cleaned up after each test suite

### Connection & Auth Tests (Phase C)
- AC11: TS client connects to Rust server successfully (WebSocket OPEN state)
- AC12: Client receives AUTH_REQUIRED message on connect
- AC13: Client sends valid JWT, receives AUTH_ACK
- AC14: Client sends invalid JWT, receives AUTH_FAIL

### CRDT Tests (Phase C)
- AC15: CLIENT_OP with PUT writes data, client receives OP_ACK
- AC16: OP_BATCH with multiple ops, OP_ACK.lastId equals last op id
- AC17: LWW conflict: later HLC timestamp wins (verified via QUERY_SUB snapshot)
- AC18: Tombstone (value: null) via REMOVE results in key absent from QUERY_SUB snapshot
- AC19: Deterministic tie-breaking: same millis + counter, lexicographically greater nodeId wins

### ORMap Tests (Phase C)
- AC20: OR_ADD creates entry, visible in QUERY_SUB snapshot
- AC21: OR_REMOVE by tag removes entry, absent in subsequent QUERY_SUB snapshot

### Query Tests (Phase D)
- AC22: QUERY_SUB on populated map returns QUERY_RESP with all records
- AC23: QUERY_SUB with `where` filter returns only matching records
- AC24: QUERY_SUB with comparison operators ($gt, $lt, $ne) returns correct results
- AC25: QUERY_SUB with `sort` returns results in specified order
- AC26: QUERY_SUB with `limit` returns at most N results
- AC27: After QUERY_SUB, new writes by another client trigger QUERY_UPDATE to subscriber
- AC28: QUERY_UNSUB stops QUERY_UPDATE delivery
- AC29: Record no longer matching filter triggers QUERY_UPDATE with changeType LEAVE

### Pub/Sub Tests (Phase D)
- AC30: TOPIC_SUB + TOPIC_PUB delivers TOPIC_MESSAGE to subscriber
- AC31: Publisher does NOT receive its own published message
- AC32: Multiple subscribers all receive published message
- AC33: TOPIC_UNSUB stops message delivery
- AC34: Messages published to topic A are not delivered to subscriber of topic B
- AC35: 10 sequential messages maintain publishing order

### Search Tests (Phase E)
- AC36: SEARCH returns BM25-ranked results for matching query
- AC37: SEARCH with `limit` returns at most N results
- AC38: SEARCH on non-indexed map returns error in SEARCH_RESP
- AC39: SEARCH_SUB returns initial results and SEARCH_UPDATE ENTER on new matching write

## Constraints

- Tests MUST NOT call Rust server internals from TS -- all verification through message exchange
- Tests MUST NOT require PostgreSQL -- use NullDataStore / in-memory storage
- Tests MUST NOT use hardcoded ports -- Rust server uses port 0, TS reads actual port
- Test server binary MUST NOT be added to the default `cargo build` (test-only binary)
- Auth JWT secret in Rust test server MUST match `test-e2e-secret` from TS test helpers
- Existing TS e2e tests (`tests/e2e/`) MUST NOT be modified -- the new integration tests live in `tests/integration-rust/`
- The Rust server MUST NOT have a `main.rs` in `src/` yet (the binary is in `src/bin/test_server.rs`, separate from the library)

## Assumptions

- The existing TS `msgpackr` serialization is wire-compatible with Rust `rmp-serde` for all message types (field naming, Option handling, number encoding). If incompatibilities are found during Phase A, they will be fixed as part of the wiring work.
- The CoalescingWriter BATCH framing format in the Rust outbound task matches what the TS test client parses. If not, the TS harness will handle both batched and individual messages.
- JWT verification in Rust will use the `jsonwebtoken` crate (already available in the ecosystem) with HS256 algorithm, matching the TS server's JWT implementation.
- Search tests (Phase E) require that the Rust test server has a way to enable FTS indexing for a map before tests run. This may require a control message or a startup configuration flag.
- Cluster integration tests (Phase F) are deferred to a follow-up spec because they require multi-process Rust server orchestration, which is a separate infrastructure concern.
- The security UAT tests (`uat-security-hardening.test.ts`) test TS-specific constructs (RateLimitedLogger, validateJwtSecret, TS HLC). They do NOT need porting -- the Rust equivalents (WriteValidator, HLC sanitization) are already tested by 467 Rust unit tests.

## Task Groups

### Decomposition Plan (for /sf:split)

This spec MUST be split into sub-specs before implementation. The recommended split follows the phases above:

| Sub-Spec | Phase | Scope | Language | Est. Files | Dependencies |
|----------|-------|-------|----------|-----------|--------------|
| SPEC-073a | A | WS dispatch + auth + test binary | Rust | 4-5 | -- |
| SPEC-073b | B | TS test harness | TypeScript | 2-3 | SPEC-073a |
| SPEC-073c | C | Connection, auth, CRDT (LWW + ORMap) tests | TypeScript | 3 | SPEC-073b |
| SPEC-073d | D | Query + pub/sub tests | TypeScript | 2 | SPEC-073c |
| SPEC-073e | E | Search tests | TypeScript | 1-2 | SPEC-073c |

### Task Groups (high-level, pre-split)

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Auth types + traits: ConnectionInfo auth state, JWT verification trait | -- | ~10% |
| G2 | 2 | WS dispatch: replace stub, deserialize MsgPack, route through pipeline | G1 | ~20% |
| G3 | 2 | Auth handshake: AUTH_REQUIRED on connect, AUTH/AUTH_ACK/AUTH_FAIL flow | G1 | ~15% |
| G4 | 3 | Test server binary: wire all services, port 0, stdout protocol | G2, G3 | ~15% |
| G5 | 4 | TS test harness: spawn binary, capture port, client factory | G4 | ~10% |
| G6 | 5 | Connection + CRDT tests | G5 | ~10% |
| G7 | 5 | Query + pub/sub tests | G5 | ~10% |
| G8 | 5 | Search + ORMap tests | G5 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |
| 4 | G5 | No | 1 |
| 5 | G6, G7, G8 | Yes | 3 |

**Total workers needed:** 3 (max in Wave 5)

---
*Generated by SpecFlow spec-creator on 2026-03-01*
