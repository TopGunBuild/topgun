---
id: SPEC-066
type: feature
status: done
priority: P2
complexity: large
created: 2026-02-26
todo: TODO-089
---

# PersistenceService: Counters, Journal, and Resolver/EntryProcess Stubs

## Context

The `PersistenceService` is the sixth domain service replacing the `domain_stub!` macro. It handles 10 `Operation` variants grouped into four sub-domains:

1. **PN-Counter CRDT** (2 ops): `CounterRequest`, `CounterSync` -- atomic counter operations with subscriber-based state broadcasting
2. **Journal** (3 ops): `JournalSubscribe`, `JournalUnsubscribe`, `JournalRead` -- mutation event audit/CDC stream
3. **Entry Processing** (2 ops): `EntryProcess`, `EntryProcessBatch` -- user-defined read-modify-write transforms
4. **Conflict Resolvers** (3 ops): `RegisterResolver`, `UnregisterResolver`, `ListResolvers` -- server-side conflict resolution policies

Previous domain services (SPEC-061 through SPEC-065) each handled 2-3 operation variants. This service handles 10, requiring decomposition.

### Critical Architectural Gap: Sandboxed Code Execution

The TS `EntryProcessorHandler` and `ConflictResolverHandler` rely on `ProcessorSandbox` (isolated-vm / vm.Script) to execute user-supplied JavaScript code server-side. Rust has no equivalent infrastructure yet. WASM-based sandboxing is planned for v2.0 (TODO-072).

**Decision for this spec:** Implement Counters and Journal fully. For Entry Processing and Resolvers, implement the service routing and response wire protocol but return `NotImplemented`-style error responses with clear error codes. This unblocks the `domain_stub!` macro removal while deferring the sandbox dependency to SPEC-072 (WASM).

### TS Behavioral Reference

| Sub-domain | TS Source | Key Behavior |
|------------|-----------|--------------|
| Counter | `handlers/CounterHandler.ts`, `coordinator/counter-handler-adapter.ts` | In-memory PNCounter map, subscriber tracking per counter name, CRDT merge on sync, broadcast merged state to other subscribers |
| Journal | `coordinator/journal-handler.ts`, `EventJournalService.ts` | Subscribe with filters (mapName, types, fromSequence), unsubscribe by ID, read paginated events from ring buffer |
| Entry Processor | `handlers/EntryProcessorHandler.ts`, `coordinator/entry-processor-adapter.ts` | Sandbox execution, per-key locking, read-modify-write atomicity |
| Resolver | `handlers/ConflictResolverHandler.ts`, `coordinator/resolver-handler.ts`, `ConflictResolverService.ts` | Register/unregister/list resolvers per map, priority-ordered execution, key pattern matching |

## Goal-Backward Analysis

### Goal Statement
Remove the `domain_stub!(PersistenceService, ...)` macro and replace it with a real `tower::Service<Operation>` implementation that handles all 10 persistence-domain operations.

### Observable Truths
1. `CounterRequest` returns a `CounterResponse` with the current PN-Counter state for the named counter
2. `CounterSync` merges incoming state, returns `CounterUpdate` to sender, and broadcasts `CounterUpdate` to other subscribers of that counter
3. `JournalSubscribe` registers a subscription and returns `Empty` (events delivered via side-channel)
4. `JournalUnsubscribe` removes a subscription and returns `Empty`
5. `JournalRead` returns a `JournalReadResponse` with paginated events
6. `EntryProcess` / `EntryProcessBatch` return `EntryProcessResponse` / `EntryProcessBatchResponse` with a "not implemented" error (sandbox not available)
7. `RegisterResolver` / `UnregisterResolver` / `ListResolvers` return their respective response messages with "not implemented" errors (sandbox not available)
8. Non-persistence operations dispatched to `PersistenceService` return `OperationError::WrongService`
9. `ManagedService::name()` returns `"persistence"`

### Required Artifacts
- `packages/server-rust/src/service/domain/persistence.rs` -- PersistenceService struct + handlers
- `packages/server-rust/src/service/domain/counter.rs` -- CounterRegistry (in-memory PN-Counter state + subscriber tracking)
- `packages/server-rust/src/service/domain/journal.rs` -- JournalStore (in-memory ring buffer + subscription management)
- `packages/server-rust/src/service/domain/mod.rs` -- register new modules, remove `domain_stub!(PersistenceService, ...)`

### Key Links
- `persistence.rs` depends on `counter.rs` (CounterRegistry) and `journal.rs` (JournalStore)
- `persistence.rs` depends on `ConnectionRegistry` (for counter broadcast and journal event delivery)
- `counter.rs` depends on `topgun_core::messages::PNCounterState` (wire format)
- `journal.rs` depends on `topgun_core::messages::JournalEventData` and `JournalEventType` (wire format)
- All message payload types already exist in `packages/core-rust/src/messages/messaging.rs`
- All Operation variants already exist in `packages/server-rust/src/service/operation.rs`

## Task

Replace the `domain_stub!(PersistenceService, ...)` macro invocation in `packages/server-rust/src/service/domain/mod.rs` with a real `PersistenceService` struct that:

1. Implements `ManagedService` (name = `"persistence"`)
2. Implements `tower::Service<Operation>` for `Arc<PersistenceService>`
3. Dispatches all 10 persistence-domain `Operation` variants
4. Fully implements Counter operations (in-memory PNCounter CRDT with subscriber broadcasting)
5. Fully implements Journal operations (in-memory ring buffer with filtered subscriptions)
6. Returns structured error responses for Entry Process and Resolver operations (sandbox not available)

## Requirements

### New Files

#### 1. `packages/server-rust/src/service/domain/counter.rs`

**CounterRegistry** -- thread-safe in-memory PN-Counter store with subscriber tracking.

```rust
pub struct CounterRegistry {
    // counter_name -> PNCounterState (merged state)
    // counter_name -> HashSet<ConnectionId> (subscribers)
}
```

Public API:
- `fn new(node_id: String) -> Self`
- `fn get_or_create(&self, name: &str) -> PNCounterState` -- returns current state, creating empty counter if absent
- `fn merge(&self, name: &str, incoming: &PNCounterState) -> PNCounterState` -- merges incoming state, returns merged result
- `fn subscribe(&self, name: &str, conn_id: ConnectionId)` -- adds subscriber
- `fn unsubscribe(&self, name: &str, conn_id: ConnectionId)` -- removes subscriber
- `fn unsubscribe_all(&self, conn_id: ConnectionId)` -- removes from all counters (for disconnect)
- `fn subscribers(&self, name: &str) -> Vec<ConnectionId>` -- returns all subscribers for the named counter
- `fn counter_value(&self, name: &str) -> f64` -- returns current computed value (sum of positive - sum of negative)

PN-Counter merge: for each node in incoming.p, take `max(local.p[node], incoming.p[node])`. Same for incoming.n.

Uses `DashMap` for concurrent access, consistent with `TopicRegistry` pattern.

#### 2. `packages/server-rust/src/service/domain/journal.rs`

**JournalStore** -- thread-safe in-memory ring buffer for mutation events with subscription management.

```rust
pub struct JournalStore {
    // Ring buffer of JournalEventData with monotonic sequence counter
    // subscription_id -> JournalSubscription (filters + ConnectionId)
}

pub struct JournalSubscription {
    pub connection_id: ConnectionId,
    pub map_name: Option<String>,
    pub types: Option<Vec<JournalEventType>>,
}
```

Public API:
- `fn new(capacity: usize) -> Self`
- `fn append(&self, mut event: JournalEventData) -> u64` -- appends event, returns sequence number. Sets `event.sequence` to the `String` representation of the internal `u64` counter before storing (since `JournalEventData.sequence` is `String` on the wire but the store tracks sequences as `u64` internally)
- `fn subscribe(&self, subscription_id: String, sub: JournalSubscription)` -- registers subscription
- `fn unsubscribe(&self, subscription_id: &str)` -- removes subscription
- `fn unsubscribe_by_connection(&self, conn_id: ConnectionId)` -- removes all subscriptions for a connection
- `fn read(&self, from_sequence: u64, limit: u32, map_name: Option<&str>) -> (Vec<JournalEventData>, bool)` -- returns events + has_more flag

The ring buffer uses a `VecDeque<JournalEventData>` behind a `RwLock`, with an `AtomicU64` for the sequence counter. Capacity defaults to 10000.

Note: In this initial implementation, the `append()` method stores events in the ring buffer. Subscribers are tracked but delivery is NOT actively pushed (no spawn/channel). Instead, the `PersistenceService` returns `Empty` for `JournalSubscribe` and the journal serves as a queryable store via `JournalRead`. Active push delivery (spawning tasks per subscriber) is deferred to integration testing (SPEC-068) when the full connection pipeline is available. The subscription metadata is stored so `JournalUnsubscribe` works correctly.

#### 3. `packages/server-rust/src/service/domain/persistence.rs`

**PersistenceService** -- real domain service struct.

```rust
pub struct PersistenceService {
    counter_registry: Arc<CounterRegistry>,
    journal_store: Arc<JournalStore>,
    connection_registry: Arc<ConnectionRegistry>,
}
```

Constructor: `fn new(connection_registry: Arc<ConnectionRegistry>, node_id: String) -> Self`

Implements `ManagedService` (name = `"persistence"`) and `tower::Service<Operation>` for `Arc<PersistenceService>`.

Handler methods:
- `handle_counter_request(ctx, payload) -> Result<OperationResponse, OperationError>` -- calls `counter_registry.get_or_create()`, subscribes connection, returns `Message::CounterResponse`
- `handle_counter_sync(ctx, payload) -> Result<OperationResponse, OperationError>` -- calls `counter_registry.merge()`, subscribes connection, broadcasts `Message::CounterUpdate` to other subscribers via `connection_registry`, returns `Message::CounterUpdate` to sender
- `handle_journal_subscribe(ctx, payload) -> Result<OperationResponse, OperationError>` -- registers subscription in `journal_store`, returns `OperationResponse::Empty`
- `handle_journal_unsubscribe(ctx, payload) -> Result<OperationResponse, OperationError>` -- removes subscription from `journal_store`, returns `OperationResponse::Empty`
- `handle_journal_read(ctx, payload) -> Result<OperationResponse, OperationError>` -- reads from `journal_store.read()` using `payload.limit.unwrap_or(100)` as the page size when `JournalReadData.limit` is `None`, returns `Message::JournalReadResponse`
- `handle_entry_process(ctx, payload) -> Result<OperationResponse, OperationError>` -- returns `Message::EntryProcessResponse` with `success: false, error: "Entry processing not available: WASM sandbox required"`
- `handle_entry_process_batch(ctx, payload) -> Result<OperationResponse, OperationError>` -- returns `Message::EntryProcessBatchResponse` with per-key errors
- `handle_register_resolver(ctx, payload) -> Result<OperationResponse, OperationError>` -- returns `Message::RegisterResolverResponse` with `success: false, error: "Conflict resolvers not available: WASM sandbox required"`
- `handle_unregister_resolver(ctx, payload) -> Result<OperationResponse, OperationError>` -- returns `Message::UnregisterResolverResponse` with `success: false, error: "Conflict resolvers not available: WASM sandbox required"`
- `handle_list_resolvers(ctx, payload) -> Result<OperationResponse, OperationError>` -- returns `Message::ListResolversResponse` with empty resolver list

### Modified Files

#### 4. `packages/server-rust/src/service/domain/mod.rs`

- Add `pub mod counter;`, `pub mod journal;`, `pub mod persistence;`
- Add `pub use persistence::PersistenceService;`
- Remove `domain_stub!(PersistenceService, service_names::PERSISTENCE);`
- Update existing stub test (`persistence_service_returns_not_implemented`) to test new constructor-based `PersistenceService`
- Update `all_stubs_implement_managed_service` test to remove `PersistenceService` from stub list (it now requires constructor args)

## Acceptance Criteria

### Counter Operations

- **AC1**: `CounterRequest` with a new counter name returns `Message::CounterResponse` containing `PNCounterState` with empty `p` and `n` maps and the requested counter name
- **AC2**: `CounterRequest` with an existing counter returns `Message::CounterResponse` with the current merged state
- **AC3**: `CounterSync` merges incoming `PNCounterState` using max-per-node semantics (for each node ID, `result.p[node] = max(local.p[node], incoming.p[node])` and same for `n`) and returns `Message::CounterUpdate` with the merged state
- **AC4**: `CounterSync` broadcasts `Message::CounterUpdate` (serialized via `rmp_serde::to_vec_named`) to all other subscribers of that counter via `ConnectionRegistry`, excluding the sender's connection
- **AC5**: `CounterRequest` auto-subscribes the caller's `connection_id` to the counter; subsequent `CounterSync` from another connection broadcasts to the first

### Journal Operations

- **AC6**: `JournalSubscribe` stores subscription metadata (connection_id, map_name filter, types filter) keyed by `request_id` and returns `OperationResponse::Empty`
- **AC7**: `JournalUnsubscribe` removes the subscription by `subscription_id` and returns `OperationResponse::Empty`
- **AC8**: `JournalRead` with `from_sequence` and `limit` returns `Message::JournalReadResponse` containing matching events from the ring buffer and a `has_more` flag indicating whether more events exist beyond the requested page
- **AC9**: `JournalRead` with optional `map_name` filter returns only events matching that map name

### Stub Operations (Sandbox-Dependent)

- **AC10**: `EntryProcess` returns `Message::EntryProcessResponse` with `success: false` and `error` containing "WASM sandbox required"
- **AC11**: `EntryProcessBatch` returns `Message::EntryProcessBatchResponse` with per-key `EntryProcessKeyResult` entries each having `success: false` and `error` containing "WASM sandbox required"
- **AC12**: `RegisterResolver` returns `Message::RegisterResolverResponse` with `success: false` and `error` containing "WASM sandbox required"
- **AC13**: `UnregisterResolver` returns `Message::UnregisterResolverResponse` with `success: false` and `error` containing "WASM sandbox required"
- **AC14**: `ListResolvers` returns `Message::ListResolversResponse` with `request_id` echoed and empty `resolvers` vec

### Service Boilerplate

- **AC15**: Non-persistence `Operation` variants (e.g., `GarbageCollect`, `TopicPublish`) return `Err(OperationError::WrongService)`
- **AC16**: `ManagedService::name()` returns `"persistence"`
- **AC17**: `PersistenceService` is constructed with `Arc<ConnectionRegistry>` and `String` node_id; no default constructor
- **AC18**: The `domain_stub!(PersistenceService, ...)` macro invocation in `mod.rs` is removed; only `SearchService` remains as a stub
- **AC19**: All existing tests in `mod.rs` continue to pass (the `all_stubs_implement_managed_service` test is updated to exclude `PersistenceService`)

### Counter Merge Correctness

- **AC20**: PN-Counter merge is commutative: `merge(A, B)` produces the same state as `merge(B, A)`
- **AC21**: PN-Counter merge is idempotent: `merge(A, A)` produces A unchanged
- **AC22**: PN-Counter value is computed as `sum(p.values()) - sum(n.values())`

## Constraints

- Do NOT implement sandboxed code execution (entry processors, resolvers) -- return error responses instead
- Do NOT add a WASM runtime dependency
- Do NOT implement PostgreSQL persistence for journal events (in-memory ring buffer only; PostgreSQL persistence is SPEC-090)
- Do NOT implement active push delivery for journal subscriptions (store subscription metadata only; active delivery requires spawning per-subscriber tasks which is deferred to integration)
- Follow existing domain service patterns: `Arc` dependencies via constructor, `DashMap` for concurrent data structures, `rmp_serde::to_vec_named()` for serialization
- Maximum 5 files (3 new + 1 modified = 4 files, within limit)
- Counter `p` and `n` values are `f64` (matching the TS PNCounter which uses JS numbers and the existing `PNCounterState` Rust struct)

## Assumptions

- **Counter subscriber tracking uses `ConnectionId`** (not `client_id` string), consistent with `TopicRegistry` in `messaging.rs`. The TS uses `clientId` strings, but Rust services use `ConnectionId` for all subscriber tracking.
- **Journal ring buffer capacity of 10000** is adequate for the in-memory implementation. This matches the TS `DEFAULT_EVENT_JOURNAL_CONFIG.capacity` default.
- **JournalRead from_sequence is a string** on the wire (matching TS which uses BigInt serialized as string) but parsed to `u64` internally. If parsing fails, return an empty events list.
- **Counter auto-subscribe** requires `connection_id` in the `OperationContext`. If absent, counter operations still succeed (get/merge) but no subscription is created.
- **No disconnect cleanup** for counters/journal in this spec. Cleanup on connection disconnect is a cross-cutting concern handled by a lifecycle hook (future spec). The data structures support `unsubscribe_all` for when that hook is wired.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context | Segments |
|-------|------|-------|--------------|--------------|----------|
| G1 | 1 | Define `CounterRegistry` struct and public API in `counter.rs`; define `JournalStore` + `JournalSubscription` structs and public API in `journal.rs` | -- | ~25% | 2 |
| G2 | 2 | Implement `PersistenceService` struct, `ManagedService` impl, `tower::Service<Operation>` dispatch, and all 10 handler methods in `persistence.rs` | G1 | ~25% | 1 |
| G3 | 2 | Unit tests for `CounterRegistry` (merge semantics, subscribe/unsubscribe, commutativity, idempotency) and `JournalStore` (append, read, subscribe/unsubscribe, filtering) in their respective files | G1 | ~20% | 1 |
| G4 | 3 | Update `mod.rs`: remove `domain_stub!`, add module declarations, update existing tests; integration tests for `PersistenceService` (counter + journal flows, stub responses, wrong service) | G2, G3 | ~15% | 1 |

**G1 Segments:**
- S1: Create `counter.rs` -- CounterRegistry struct, new(), get_or_create(), merge(), subscribe/unsubscribe methods, counter_value() -- ~13%
- S2: Create `journal.rs` -- JournalStore struct, JournalSubscription, new(), append(), subscribe/unsubscribe methods, read() -- ~12%

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-27 09:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~85% total (sum of G1+G2+G3+G4 before revision)

**Critical:**
1. **G2 context estimate exceeds threshold:** G2 was estimated at ~40%, well above the 30% per-group target. `persistence.rs` contains 10 handler methods plus `ManagedService` and `tower::Service<Operation>` impls. Revised to ~25% by recognizing that 5 of the 10 handlers are trivial stubs (3-5 lines each returning error responses) and the remaining 5 follow established patterns from `messaging.rs`. The original 40% estimate was inflated.
2. **`CounterRegistry::subscribers()` description/signature mismatch:** The description said "returns subscriber list excluding a specific conn_id" but the signature `fn subscribers(&self, name: &str) -> Vec<ConnectionId>` has no exclusion parameter. This must be one or the other. The `TopicRegistry` pattern (used in `messaging.rs`) returns all subscribers and lets the caller filter -- this is the correct approach. Fixed: description updated to "returns all subscribers for the named counter" to match the signature and the established pattern where the caller (in `handle_counter_sync`) excludes the sender.
3. **Orphan API method `pending_notifications`:** `JournalStore::pending_notifications()` was specified in the public API but never referenced by any handler method, acceptance criterion, or test. Given the constraint "Do NOT implement active push delivery," this method has no caller and adds dead code. Removed from the API specification.

**Recommendations:**
4. [Strategic] The `append()` method description says "notifies subscribers" but the note below explicitly says delivery is NOT actively pushed. The description should say "appends event, returns sequence number" without the notification claim, to avoid confusing implementers. Fixed in this revision.
5. The `JournalEventData.sequence` field is a `String` on the wire but the `JournalStore` uses `u64` internally. The `read()` method takes `from_sequence: u64`. The spec should clarify that `JournalStore.append()` sets the `sequence` field on the stored `JournalEventData` by converting the internal `u64` counter to a `String` (since `JournalEventData.sequence` is `String`). This is implied but worth making explicit for implementers.
6. G1 mixes implementation with types (CounterRegistry has merge logic, JournalStore has ring buffer logic), which deviates from trait-first. However, since this spec has no separate traits (consistent with SPEC-061 through SPEC-065 domain service pattern), this is an acceptable deviation. No action needed, noted for record.

### Response v1 (2026-02-27 09:15)
**Applied:** all (critical items 1-3 applied by auditor inline; recommendations 4-5 applied in revision; item 6 noted)

**Changes:**
1. [✓] G2 context estimate — revised from ~40% to ~25% (applied by auditor inline)
2. [✓] `CounterRegistry::subscribers()` description — updated to "returns all subscribers for the named counter" (applied by auditor inline)
3. [✓] Orphan `pending_notifications` method — removed from JournalStore API (applied by auditor inline)
4. [✓] `append()` description — removed "notifies subscribers" claim (applied by auditor inline)
5. [✓] `append()` u64-to-String conversion — added explicit note that `event.sequence` is set to `String` representation of internal `u64` counter before storing
6. [✗] Trait-first deviation — no action needed, acceptable deviation consistent with SPEC-061–065

### Audit v2 (2026-02-27 10:30)
**Status:** APPROVED

**Context Estimate:** ~85% total

**Scope:** Large (~85% estimated cumulative, but per-group estimates are within 30% target)

**Per-Group Breakdown:**
| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~25% | OK |
| G2 | ~25% | OK |
| G3 | ~20% | OK |
| G4 | ~15% | OK |

**Quality Projection:** Each worker invocation stays in the GOOD (25-30%) range due to wave-based execution. Cumulative 85% is the sum across all waves, not a single worker's load.

**Rust Type Mapping Checklist:**
- [x] No `f64` for integer-semantic fields -- counter p/n values are genuinely fractional (matching existing `PNCounterState` struct); `JournalReadData.limit` is `Option<u32>`; sequence is `String` on wire
- [x] No `r#type: String` on message structs -- no new message structs created
- [x] `Default` derived on payload structs with 2+ optional fields -- N/A (no new payload structs)
- [x] Enums used for known value sets -- uses existing `JournalEventType` enum
- [x] Wire compatibility: `rmp_serde::to_vec_named()` specified in constraints
- [x] `#[serde(rename_all = "camelCase")]` -- N/A (no new wire structs; internal structs like CounterRegistry and JournalStore are not serialized)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` -- N/A (no new wire structs)

**Goal-Backward Validation:**
- All 9 observable truths have corresponding artifacts
- All 4 artifacts map to observable truths
- Key links verified against actual source code (PNCounterState, JournalEventData, CounterStatePayload, ConnectionRegistry all confirmed to exist with expected structures)
- No orphan artifacts or uncovered truths

**Strategic fit:** Aligned with project goals -- sixth of seven domain service replacements, linear progression toward v1.0 IMDG milestone.

**Project compliance:** Honors PROJECT.md decisions -- MsgPack wire format, no new runtime dependencies, follows domain service replacement pattern, 4 files within 5-file Language Profile limit.

**Language profile:** Compliant with Rust profile -- 4 files (within 5-file limit); trait-first deviation accepted (consistent with SPEC-061-065 pattern where domain services have no separate traits).

**Recommendations:**
1. The `handle_journal_read` handler must handle the optional `limit` field in `JournalReadData` (which is `Option<u32>`) by unwrapping with a sensible default (e.g., `payload.limit.unwrap_or(100)` or using the ring buffer capacity). The spec should mention the default limit value, though the implementer can infer this from the `Option<u32>` type.
2. AC13 uses "not available" for the UnregisterResolver error text, while AC10/AC12 use "WASM sandbox required." Consider making the error messages consistent across all stub handlers for a uniform client experience. This is cosmetic and does not affect functionality.

**Comment:** Spec is well-structured after v1 revision. All 22 acceptance criteria are concrete and testable. The decomposition into 4 task groups with 3 waves is appropriate for the 10-operation scope. Previous audit issues have been cleanly resolved. Ready for implementation.

### Response v2 (2026-02-27 11:00)
**Applied:** both recommendations from Audit v2

**Changes:**
1. [✓] Default limit for `handle_journal_read` — updated handler description to specify `payload.limit.unwrap_or(100)` as the default page size when `JournalReadData.limit` is `None`
2. [✓] Consistent stub error messages — updated AC13 from "not available" to "WASM sandbox required"; updated `handle_unregister_resolver` handler description to include the same error string used by other stub handlers

### Audit v3 (2026-02-27 12:00)
**Status:** APPROVED

**Context Estimate:** ~85% total (cumulative across 4 task groups in 3 waves)

**Scope:** Large (~85% cumulative, per-group estimates within 30% target)

**Per-Group Breakdown:**
| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~25% | OK |
| G2 | ~25% | OK |
| G3 | ~20% | OK |
| G4 | ~15% | OK |

**Quality Projection:** Each worker invocation stays in the GOOD (25-30%) range due to wave-based execution. The ~85% cumulative figure is the sum across all waves, not a single worker's context load.

**Fresh-eyes source code verification:**
- `PNCounterState` struct confirmed at `packages/core-rust/src/messages/messaging.rs:122` with `p: HashMap<String, f64>` and `n: HashMap<String, f64>`
- `CounterStatePayload` confirmed at line 147 with `name: String` and `state: PNCounterState`
- `CounterRequestPayload` confirmed at line 135 with `name: String`
- `Message::CounterResponse` uses named `payload` field (not tuple), `Message::CounterUpdate` likewise -- handler construction will use `Message::CounterResponse { payload: ... }` syntax
- `Message::EntryProcessResponse(...)` and other response variants use tuple syntax -- handlers will use `Message::EntryProcessResponse(EntryProcessResponseData { ... })`
- `JournalEventData` confirmed at line 334 with `sequence: String`, `event_type: JournalEventType`, `map_name: String`
- `JournalReadData` confirmed at line 438 with `from_sequence: String` (not Option), `limit: Option<u32>`, `map_name: Option<String>`
- `JournalSubscribeData` confirmed at line 393 with `request_id: String`, `from_sequence: Option<String>`, `map_name: Option<String>`, `types: Option<Vec<JournalEventType>>`
- `JournalUnsubscribeData` confirmed at line 415 with `subscription_id: String`
- `ConnectionRegistry::get(id) -> Option<Arc<ConnectionHandle>>` confirmed -- matches broadcasting pattern used in `MessagingService`
- `ConnectionHandle::try_send(msg: OutboundMessage) -> bool` confirmed -- matches best-effort delivery pattern
- All 10 persistence Operation variants confirmed in `operation.rs` lines 244-295
- `domain_stub!(PersistenceService, service_names::PERSISTENCE)` confirmed at `mod.rs:102-105` -- ready for removal
- Existing tests at `mod.rs:149-179` confirmed -- both `persistence_service_returns_not_implemented` and `all_stubs_implement_managed_service` reference `PersistenceService`

**Rust Type Mapping Checklist:**
- [x] No `f64` for integer-semantic fields -- counter p/n values are genuinely fractional (verified `PNCounterState` struct); `JournalReadData.limit` is `Option<u32>`; sequence is `String` on wire
- [x] No `r#type: String` on message structs -- no new message structs created
- [x] `Default` derived on payload structs with 2+ optional fields -- N/A (no new payload structs)
- [x] Enums used for known value sets -- uses existing `JournalEventType` enum
- [x] Wire compatibility: `rmp_serde::to_vec_named()` specified in constraints
- [x] `#[serde(rename_all = "camelCase")]` -- N/A (no new wire structs)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` -- N/A (no new wire structs)

**Goal-Backward Validation:**
- All 9 observable truths have corresponding artifacts and acceptance criteria
- All 4 artifacts map to observable truths
- All key links verified against actual source code
- No orphan artifacts or uncovered truths

**Strategic fit:** Aligned with project goals -- sixth of seven domain service replacements, linear progression toward v1.0 IMDG milestone.

**Project compliance:** Honors PROJECT.md decisions -- MsgPack wire format, no new runtime dependencies, follows domain service replacement pattern, 4 files within 5-file Language Profile limit.

**Language profile:** Compliant with Rust profile -- 4 files (within 5-file limit); trait-first deviation accepted (consistent with SPEC-061-065 pattern).

**Comment:** Spec is clean and implementation-ready after two prior audit-revision cycles. All 22 acceptance criteria are concrete and testable. All referenced types, enums, and structs confirmed to exist in source code with expected shapes. The decomposition into 4 task groups across 3 waves keeps each worker invocation within the GOOD quality range. No critical issues or recommendations remain.

## Execution Summary

**Executed:** 2026-02-27
**Mode:** orchestrated (direct implementation due to tool constraints)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 (CounterRegistry + JournalStore) | complete |
| 2 | G2 (PersistenceService), G3 (Unit tests) | complete |
| 3 | G4 (mod.rs update, integration wiring) | complete |

**Note:** G4 tasks (mod.rs domain_stub! removal, lib.rs wiring) were executed alongside G2 because removing the stub was required for persistence.rs to compile.

### Files Created
- `packages/server-rust/src/service/domain/counter.rs` -- CounterRegistry (PN-Counter CRDT with subscriber tracking)
- `packages/server-rust/src/service/domain/journal.rs` -- JournalStore (ring buffer with subscription management)
- `packages/server-rust/src/service/domain/persistence.rs` -- PersistenceService (10 operation handlers)

### Files Modified
- `packages/server-rust/src/service/domain/mod.rs` -- module declarations, domain_stub! removal, test updates
- `packages/server-rust/src/lib.rs` -- PersistenceService wiring with constructor args

### Acceptance Criteria Status
- [x] AC1: CounterRequest with new counter returns empty PNCounterState
- [x] AC2: CounterRequest with existing counter returns current merged state
- [x] AC3: CounterSync merges using max-per-node semantics
- [x] AC4: CounterSync broadcasts CounterUpdate to other subscribers via ConnectionRegistry
- [x] AC5: CounterRequest auto-subscribes the caller's connection_id
- [x] AC6: JournalSubscribe stores subscription metadata keyed by request_id
- [x] AC7: JournalUnsubscribe removes subscription by subscription_id
- [x] AC8: JournalRead returns paginated events with has_more flag
- [x] AC9: JournalRead with map_name filter returns only matching events
- [x] AC10: EntryProcess returns error with "WASM sandbox required"
- [x] AC11: EntryProcessBatch returns per-key errors with "WASM sandbox required"
- [x] AC12: RegisterResolver returns error with "WASM sandbox required"
- [x] AC13: UnregisterResolver returns error with "WASM sandbox required"
- [x] AC14: ListResolvers returns empty resolvers vec with echoed request_id
- [x] AC15: Non-persistence operations return WrongService
- [x] AC16: ManagedService::name() returns "persistence"
- [x] AC17: PersistenceService constructed with Arc<ConnectionRegistry> and node_id
- [x] AC18: domain_stub!(PersistenceService) removed; only SearchService remains as stub
- [x] AC19: All existing tests in mod.rs continue to pass (updated for new constructor)
- [x] AC20: PN-Counter merge is commutative (tested)
- [x] AC21: PN-Counter merge is idempotent (tested)
- [x] AC22: PN-Counter value computed as sum(p) - sum(n)

### Test Results
- 446 total tests (27 new), 0 failures
- Clippy clean (0 warnings with -D warnings)

### Deviations
- G4 tasks were merged into G2 commit due to compilation dependency (domain_stub! removal required for persistence.rs to compile)

---

## Completion

**Completed:** 2026-02-27
**Total Commits:** 2
**Audit Cycles:** 3
**Review Cycles:** 1

---

## Review History

### Review v1 (2026-02-27)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: CounterRequest for new counter returns CounterResponse with empty PNCounterState — `get_or_create` creates empty p/n maps
- [✓] AC2: CounterRequest for existing counter returns current state — `get_or_create` returns stored state
- [✓] AC3: CounterSync merges using max-per-node semantics — `merge()` iterates per-node taking `max()` for both p and n maps
- [✓] AC4: CounterSync broadcasts CounterUpdate to other subscribers via `rmp_serde::to_vec_named` — broadcast loop excludes sender's conn_id
- [✓] AC5: CounterRequest auto-subscribes caller's connection_id — conditional `subscribe()` call when `ctx.connection_id` is Some
- [✓] AC6: JournalSubscribe stores metadata keyed by request_id and returns Empty — `journal_store.subscribe(payload.request_id.clone(), sub)`
- [✓] AC7: JournalUnsubscribe removes by subscription_id and returns Empty — `journal_store.unsubscribe(&payload.subscription_id)`
- [✓] AC8: JournalRead returns events with has_more flag — `journal_store.read()` returns `(Vec, bool)` correctly paginated
- [✓] AC9: JournalRead with map_name filter — passed as `payload.map_name.as_deref()` to `read()`
- [✓] AC10: EntryProcess returns success:false with "WASM sandbox required" in error — uses `WASM_SANDBOX_ERROR` constant
- [✓] AC11: EntryProcessBatch returns per-key errors with "WASM sandbox required" — iterates `payload.keys` to build error map
- [✓] AC12: RegisterResolver returns success:false with "WASM sandbox required" — uses `WASM_RESOLVER_ERROR` constant containing "WASM sandbox required"
- [✓] AC13: UnregisterResolver returns success:false with "WASM sandbox required" — uses `WASM_RESOLVER_ERROR` constant
- [✓] AC14: ListResolvers returns echoed request_id and empty resolvers vec
- [✓] AC15: Non-persistence operations return `OperationError::WrongService` — wildcard match arm at end of dispatch
- [✓] AC16: `ManagedService::name()` returns `"persistence"` — delegates to `service_names::PERSISTENCE`
- [✓] AC17: Constructor takes `Arc<ConnectionRegistry>` and `String` node_id; no `Default` impl
- [✓] AC18: `domain_stub!(PersistenceService, ...)` removed; only `SearchService` stub remains in `mod.rs`
- [✓] AC19: Existing mod.rs tests pass — `persistence_service_returns_wrong_service_for_non_persistence_ops` updated; `all_stubs_implement_managed_service` excludes PersistenceService
- [✓] AC20: PN-Counter merge is commutative — `merge_is_commutative` test verifies A+B == B+A
- [✓] AC21: PN-Counter merge is idempotent — `merge_is_idempotent` test verifies merge(A,A) == A
- [✓] AC22: Counter value = sum(p) - sum(n) — `counter_value_is_sum_p_minus_sum_n` test verifies
- [✓] Build passes clean — `cargo build` exits 0
- [✓] Clippy passes clean — `cargo clippy -- -D warnings` exits 0, 0 warnings
- [✓] 446 tests pass, 0 failures — matches execution summary count
- [✓] `domain_stub!(PersistenceService)` removed — confirmed by grep, no matches found
- [✓] File count within limit — 3 new files + 1 modified mod.rs + 1 modified lib.rs = 5 files (within 5-file Language Profile limit)
- [✓] DashMap/DashSet used for concurrent access — consistent with TopicRegistry pattern
- [✓] `unsubscribe` and `unsubscribe_all` use `remove_if` re-check pattern — avoids spurious DashMap entry removal under concurrent access
- [✓] No WASM runtime dependency added — constraints honored
- [✓] No PostgreSQL persistence added — constraints honored
- [✓] No active push delivery for journal subscriptions — constraints honored

**Summary:** All 22 acceptance criteria are fully implemented and verified. The implementation is clean, consistent with existing domain service patterns, and all tests pass with 0 failures and 0 clippy warnings. The decomposition into CounterRegistry, JournalStore, and PersistenceService is well-structured with proper separation of concerns.
