---
id: SPEC-081
type: bugfix
status: done
priority: P1
complexity: medium
created: 2026-03-07
todo: TODO-112
---

# SPEC-081: Subscription-Aware CRDT Broadcast Filtering

## Context

`CrdtService.broadcast_event()` sends every `ServerEvent` to ALL connected clients via `ConnectionRegistry.broadcast()`, ignoring query subscriptions. This wastes bandwidth and deviates from the TS server architecture, where `BroadcastHandler.broadcast()` filters by `queryRegistry.getSubscribedClientIds(mapName)`.

The `QueryService` already maintains a `QueryRegistry` with per-map subscription tracking (`get_subscriptions_for_map`), and `QueryMutationObserver` correctly targets only subscribed connections. Only `CrdtService` bypasses this filtering.

**TS Server Reference:** `BroadcastHandler.broadcast()` in `packages/server/src/coordinator/broadcast-handler.ts`:
1. Calls `queryRegistry.getSubscribedClientIds(mapName)` for subscriber set
2. Early exit if no subscribers (event dropped)
3. Iterates only subscriber connections, excludes originator
4. Applies Field-Level Security (FLS) filtering per client role

## Goal Analysis

**Goal Statement:** CRDT write events reach only clients with active query subscriptions for the affected map, eliminating bandwidth waste from unfiltered broadcast.

**Observable Truths:**
1. A client with a live query on map "users" receives `ServerEvent` when another client writes to "users"
2. A client with NO query on map "users" does NOT receive `ServerEvent` when "users" is written to
3. The writing client itself does NOT receive its own `ServerEvent` back
4. When zero clients subscribe to a map, serialization is skipped entirely (no `rmp_serde::to_vec_named` call)
5. Existing unit tests continue to pass (no connection_id = internal/system calls still work)

**Required Artifacts:**
- `query.rs` -- `QueryRegistry.get_subscribed_connection_ids(map_name)` method
- `connection.rs` -- `ConnectionRegistry.send_to_connections(ids, bytes)` method
- `crdt.rs` -- Updated `broadcast_event()` using QueryRegistry + targeted send
- `lib.rs` + `test_server.rs` -- Wire `QueryRegistry` into `CrdtService` constructor

**Key Links:**
- `CrdtService` -> `QueryRegistry`: lookup subscriber connection IDs by map_name
- `CrdtService` -> `ConnectionRegistry.send_to_connections()`: targeted delivery (not broadcast)
- `OperationContext.connection_id` -> exclude writer from recipients

## Task

Replace `CrdtService.broadcast_event()` with a subscription-filtered version that queries `QueryRegistry` for subscribed connection IDs and sends only to those connections, excluding the writing client.

## Requirements

### 1. Add `get_subscribed_connection_ids` to QueryRegistry

**File:** `packages/server-rust/src/service/domain/query.rs`

Add a public method to `QueryRegistry`:

```rust
/// Returns the set of unique connection IDs with active subscriptions for `map_name`.
pub fn get_subscribed_connection_ids(&self, map_name: &str) -> HashSet<ConnectionId>
```

Implementation: iterate `get_subscriptions_for_map(map_name)`, collect `sub.connection_id` into a `HashSet<ConnectionId>`.

### 2. Add `send_to_connections` to ConnectionRegistry

**File:** `packages/server-rust/src/network/connection.rs`

Add a public method to `ConnectionRegistry`:

```rust
/// Sends a binary message to a specific set of connection IDs.
///
/// Uses non-blocking `try_send`. Skips connections that are missing,
/// not found, or have full channels (same semantics as `broadcast`).
pub fn send_to_connections(&self, ids: &HashSet<ConnectionId>, msg_bytes: &[u8])
```

Implementation: iterate `ids`, call `self.get(id)` for each, `try_send(OutboundMessage::Binary(...))`. This avoids iterating ALL connections (which `broadcast` does).

`HashSet<ConnectionId>` requires `ConnectionId` to implement `Hash` + `Eq` -- it already does (see line 46).

### 3. Update CrdtService to use subscription-aware broadcast

**File:** `packages/server-rust/src/service/domain/crdt.rs`

3a. Add `query_registry: Arc<QueryRegistry>` field to `CrdtService` struct.

3b. Update `CrdtService::new()` to accept `Arc<QueryRegistry>` parameter.

3c. Replace `broadcast_event(&self, payload)` with `broadcast_event(&self, payload, exclude_connection_id: Option<ConnectionId>)`:
  - Call `self.query_registry.get_subscribed_connection_ids(&payload.map_name)`
  - If the returned set is empty, return `Ok(())` immediately (skip serialization)
  - Remove `exclude_connection_id` from the set (if `Some`)
  - If set is now empty, return `Ok(())` (skip serialization)
  - Serialize with `rmp_serde::to_vec_named`
  - Call `self.connection_registry.send_to_connections(&ids, &bytes)`

3d. Update call sites in `handle_client_op` and `handle_op_batch` to pass `ctx.connection_id` as the exclude parameter.

### 4. Wire QueryRegistry into CrdtService at construction sites

**File:** `packages/server-rust/src/lib.rs`

There are TWO `CrdtService::new()` call sites in this file:

4a. **Primary construction** (around line 105): Move `let query_registry = Arc::new(QueryRegistry::new());` BEFORE `CrdtService::new()` call. Pass `Arc::clone(&query_registry)` to `CrdtService::new()`.

4b. **ServiceRegistry test** (around line 339): Move `let query_registry = Arc::new(QueryRegistry::new());` BEFORE the `CrdtService::new()` call at line 339. Pass `Arc::clone(&query_registry)` to `CrdtService::new()`.

**File:** `packages/server-rust/src/bin/test_server.rs`

Same wiring: pass the existing `query_registry` Arc to `CrdtService::new()`.

### 5. Update existing CrdtService tests

**File:** `packages/server-rust/src/service/domain/crdt.rs` (test module)

Update `make_service()` and all test helpers that construct `CrdtService` to pass an `Arc<QueryRegistry>` (new empty registry). Existing tests use `connection_id: None` (internal calls) so broadcast behavior is unchanged -- they continue to pass.

## Acceptance Criteria

- **AC1:** `QueryRegistry::get_subscribed_connection_ids("users")` returns an empty `HashSet` when no subscriptions exist for "users"
- **AC2:** `QueryRegistry::get_subscribed_connection_ids("users")` returns `{conn_1, conn_2}` when two connections have active query subscriptions on "users"
- **AC3:** `ConnectionRegistry::send_to_connections` delivers bytes only to the specified connection IDs, not to other registered connections
- **AC4:** `ConnectionRegistry::send_to_connections` silently skips connection IDs not in the registry (no panic, no error)
- **AC5:** `CrdtService.broadcast_event()` sends `ServerEvent` only to connections with active query subscriptions for the affected `map_name`
- **AC6:** `CrdtService.broadcast_event()` excludes `ctx.connection_id` (the writer) from recipients
- **AC7:** When zero subscribers exist for a map, `broadcast_event()` returns `Ok(())` without calling `rmp_serde::to_vec_named`
- **AC8:** All existing `CrdtService` unit tests pass without modification to test logic (only constructor signature changes)
- **AC9:** All existing `QueryService` and `QueryRegistry` tests pass
- **AC10:** `cargo test --release -p topgun-server` passes (509+ tests, 0 failures)
- **AC11:** `cargo clippy -p topgun-server` produces no warnings
- **AC12:** Integration tests (`pnpm test:integration-rust`) pass (51+ tests)

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server 2>&1` -- all tests pass, 0 failures
2. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo clippy -p topgun-server -- -D warnings 2>&1` -- no warnings
3. Run `pnpm test:integration-rust` -- all integration tests pass
4. Write a unit test: register 2 client connections, subscribe conn_1 to "users" via QueryRegistry, send a CRDT write from conn_2 targeting "users" -- verify conn_1 receives ServerEvent, conn_2 does not
5. Write a unit test: CRDT write targeting "orders" with zero subscribers -- verify no bytes sent to any connection

## Constraints

- Do NOT create a separate `BroadcastService` or `broadcast.rs` module -- keep the broadcast logic inside `CrdtService.broadcast_event()` for now. A shared broadcast abstraction can be extracted later if more services need it.
- Do NOT implement Field-Level Security (FLS) filtering in this spec -- that is a separate concern (TS server's `securityManager.filterObject`). All subscribers receive the full payload.
- Do NOT implement `CoalescingWriter` / batched broadcast -- that is a performance optimization for a future spec.
- Do NOT change the `broadcast()` method on `ConnectionRegistry` -- it remains for non-event messages and cluster broadcasts.
- Do NOT modify `QueryMutationObserver` -- it already handles its own targeted delivery correctly.

## Assumptions

- **No FLS in v1.0:** Field-Level Security filtering per client role is deferred to a future spec. All subscribed clients receive identical event payloads.
- **Empty QueryRegistry in tests is safe:** Existing unit tests construct `CrdtService` without registering query subscriptions. Since `connection_id` is `None` in those tests (internal/system calls), broadcast targets zero connections anyway -- this matches current behavior where `broadcast` sends to an empty `ConnectionRegistry`.
- **HashSet iteration order is irrelevant:** Send order to subscribers does not matter; all receive the same bytes.
- **`send_to_connections` uses `try_send` (non-blocking):** Matches existing `broadcast` semantics -- full channels are silently skipped rather than blocking the CRDT write path.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `get_subscribed_connection_ids` to `QueryRegistry` (query.rs) + `send_to_connections` to `ConnectionRegistry` (connection.rs) -- public API additions only | -- | ~15% |
| G2 | 2 | Update `CrdtService` struct, constructor, and `broadcast_event` method (crdt.rs) | G1 | ~20% |
| G3 | 2 | Wire `QueryRegistry` into `CrdtService` at all construction sites (lib.rs x2, test_server.rs) | G1 | ~10% |
| G4 | 3 | Update existing tests + add new subscription-aware broadcast tests (crdt.rs tests, connection.rs tests) | G2, G3 | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-07)
**Status:** APPROVED

**Context Estimate:** ~65% total (orchestrated across 4 workers)

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~15% | OK |
| G2 | ~20% | OK |
| G3 | ~10% | OK |
| G4 | ~20% | OK |

**Quality Projection:** GOOD range (each worker in 15-25% range, well within PEAK/GOOD)

**Fixes applied during audit:**
1. Removed duplicate Requirement #2 (section appeared twice with identical content)
2. Updated Requirement #4 to explicitly reference BOTH `CrdtService::new()` call sites in `lib.rs` (primary construction at line 105 AND ServiceRegistry test at line 339) -- original spec only mentioned one
3. Corrected inflated context estimates in Implementation Tasks (G1: 25%->15%, G2: 35%->20%, G4: 30%->20%)
4. Updated G3 description to say "all construction sites (lib.rs x2, test_server.rs)"

**Recommendations:**
1. [Strategic] Consider whether `get_subscribed_connection_ids` should be implemented directly on `DashMap` iteration rather than calling `get_subscriptions_for_map` (which allocates a `Vec`). The current spec approach is correct and simple, but for high-frequency broadcast paths, avoiding the intermediate `Vec` allocation could matter. Defer to implementer judgment.

**Comment:** Well-structured spec with clear problem statement, precise code references, and thorough acceptance criteria. Goal Analysis provides good traceability. All 5 observable truths are testable. Constraints and assumptions are explicit and reasonable.

---

## Review History

### Review v1 (2026-03-07 14:25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] **AC1:** `get_subscribed_connection_ids_empty_when_no_subscriptions` test in query.rs confirms empty HashSet returned for unsubscribed map
- [x] **AC2:** `get_subscribed_connection_ids_returns_subscribers` test confirms {conn_1, conn_2} returned; `get_subscribed_connection_ids_deduplicates_same_connection` confirms HashSet deduplication
- [x] **AC3:** `send_to_connections_delivers_only_to_specified_ids` test in connection.rs confirms targeted delivery, non-targeted connection receives nothing
- [x] **AC4:** `send_to_connections_skips_missing_ids` test confirms no panic for non-existent ConnectionId(9999); `send_to_connections_skips_full_channels` confirms graceful handling of full channels
- [x] **AC5:** `broadcast_sends_only_to_subscribers_and_excludes_writer` and `broadcast_does_not_leak_to_other_map_subscribers` tests confirm subscription-filtered delivery
- [x] **AC6:** `broadcast_sends_only_to_subscribers_and_excludes_writer` test confirms writer (conn2) does not receive its own event
- [x] **AC7:** `broadcast_skips_serialization_when_no_subscribers` test confirms no bytes sent with zero subscribers; code shows early return before `rmp_serde::to_vec_named`
- [x] **AC8:** All existing CrdtService tests pass -- only constructor signatures changed (added `query_registry` parameter)
- [x] **AC9:** All QueryService and QueryRegistry tests pass (verified via cargo test)
- [x] **AC10:** `cargo test --release -p topgun-server` -- 522 tests passed, 0 failures (exceeds 509+ threshold)
- [x] **AC11:** `cargo clippy -p topgun-server -- -D warnings` -- no warnings
- [x] **AC12:** `pnpm test:integration-rust` -- 51/51 tests pass, 7/7 suites pass

**Compliance:**
- [x] Requirement 1: `get_subscribed_connection_ids` added to QueryRegistry with correct signature and implementation
- [x] Requirement 2: `send_to_connections` added to ConnectionRegistry with try_send semantics matching broadcast
- [x] Requirement 3: CrdtService updated with query_registry field, constructor parameter, and subscription-filtered broadcast_event with exclude_connection_id
- [x] Requirement 4: QueryRegistry wired into CrdtService at all 3 construction sites (lib.rs setup(), lib.rs service_registry_lifecycle test, test_server.rs build_services())
- [x] Requirement 5: All test helpers updated with Arc<QueryRegistry> parameter

**Quality:**
- [x] No unnecessary .clone() -- payload is borrowed (&ServerEventPayload), only cloned at serialization point
- [x] Error handling uses `?` operator and Result throughout broadcast_event
- [x] No unsafe blocks
- [x] No hardcoded secrets
- [x] Code follows existing patterns (Arc dependencies, DashMap iteration, try_send semantics)
- [x] Cognitive load is low -- broadcast_event is 15 lines with clear early-return logic

**Constraints respected:**
- [x] No separate BroadcastService created
- [x] No FLS filtering implemented
- [x] No CoalescingWriter/batching
- [x] Original broadcast() method on ConnectionRegistry unchanged
- [x] QueryMutationObserver not modified

**Summary:** Clean, well-structured implementation that precisely follows the specification. All 12 acceptance criteria are met with corresponding unit tests. The broadcast_event method is concise with proper early-return optimization. Wiring is correct at all 3 construction sites. No issues found.

---

## Completion

**Completed:** 2026-03-07
**Total Commits:** 5
**Review Cycles:** 1

### Outcome

Replaced unfiltered CRDT broadcast with subscription-aware delivery. Write events now reach only clients with active query subscriptions for the affected map, eliminating bandwidth waste and aligning with the TS server's BroadcastHandler behavior.

### Key Files

- `packages/server-rust/src/service/domain/query.rs` — `get_subscribed_connection_ids()` for subscriber lookup
- `packages/server-rust/src/network/connection.rs` — `send_to_connections()` for targeted delivery
- `packages/server-rust/src/service/domain/crdt.rs` — Updated `broadcast_event()` with subscription filtering and writer exclusion
- `packages/server-rust/src/lib.rs` — Wired QueryRegistry into CrdtService at both construction sites
- `packages/server-rust/src/bin/test_server.rs` — Wired QueryRegistry into test server CrdtService

### Patterns Established

- Subscription-aware broadcast pattern: `CrdtService.broadcast_event()` queries `QueryRegistry.get_subscribed_connection_ids()` for targeted delivery via `ConnectionRegistry.send_to_connections()`, excluding the writing client and skipping serialization when no subscribers exist

### Deviations

None — implemented as specified.
