# SPEC-052d: Message Schema -- Messaging and Client Events Domain Structs

---
id: SPEC-052d
type: feature
status: blocked
priority: P0
complexity: small
created: 2026-02-15
parent: SPEC-052
depends_on: [SPEC-052a]
todo_ref: TODO-062
---

## Context

This sub-spec implements Rust serde structs for the Messaging and Client Events message domains. The Messaging domain is the largest single domain (~20 types) covering topics, locks, counters, heartbeat, entry processors, journal events, and conflict resolvers. The Client Events domain covers all server-to-client push messages.

The Client Events domain has cross-domain dependencies: it uses `ChangeEventType` from base, `CursorStatus` from query (SPEC-052b), and `LWWRecord`/`ORMapRecord`/`Timestamp` from base/hlc. However, the `CursorStatus` dependency is only needed for `QueryUpdatePayload` -- if SPEC-052b is not yet complete, `CursorStatus` can be temporarily defined locally or the struct can use a `String` with a TODO. To avoid this complexity, SPEC-052d depends only on SPEC-052a, and `CursorStatus` is re-defined in `client_events.rs` (it is a simple 4-variant string enum that is trivial to duplicate, or imported from query if SPEC-052b is complete first).

**Design decision:** `CursorStatus` is defined in `query.rs` (SPEC-052b) and re-exported. Since SPEC-052b and SPEC-052d can run in parallel after SPEC-052a, `client_events.rs` defines its own `CursorStatus` if needed, or imports from query. The `Message` enum unification in SPEC-052e will resolve any duplication.

### Critical Compatibility Issues (Inherited)

1. **Named encoding:** Must use `rmp_serde::to_vec_named()` for wire messages.
2. **camelCase:** Every struct needs `#[serde(rename_all = "camelCase")]`.
3. **Optional fields:** `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none", default)]`.
4. **Rust `type` keyword:** `JournalEventData` has a `type` field in TS. Must use `#[serde(rename = "type")]` with a different Rust field name.
5. **`ServerEventType` vs `ChangeEventType`:** These are DISTINCT enums. `ServerEventType` has PUT/REMOVE/OR_ADD/OR_REMOVE (for CRDT operations). `ChangeEventType` has ENTER/UPDATE/LEAVE (for subscription changes). Do not confuse them.

## Goal

Implement all Messaging and Client Events domain message structs so they can be deserialized from TS-produced MsgPack and re-serialized to TS-decodable MsgPack.

## Task

Create `messages/messaging.rs` and `messages/client_events.rs` with all structs from `messaging-schemas.ts` and `client-message-schemas.ts`. Register both submodules in `messages/mod.rs`.

### Approach

1. Create `messages/messaging.rs` with all messaging domain structs (topics, locks, counters, heartbeat, entry processors, journal, conflict resolvers).
2. Create `messages/client_events.rs` with all client event structs (auth ack/fail, error, server events, query update, GC prune, lock granted/released, sync reset).
3. Update `messages/mod.rs` to declare and re-export both submodules.
4. Add unit tests for serde round-trip of representative structs.

## Requirements

### Domain 6: Messaging Messages (~20 types)
**Source:** `messaging-schemas.ts`

**Topic:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `TopicSubMessage` | `TopicSubMessageSchema` | type = "TOPIC_SUB" |
| `TopicUnsubMessage` | `TopicUnsubMessageSchema` | type = "TOPIC_UNSUB" |
| `TopicPubMessage` | `TopicPubMessageSchema` | type = "TOPIC_PUB" |
| `TopicMessageEvent` | `TopicMessageEventSchema` | type = "TOPIC_MESSAGE" |

**Lock:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `LockRequestMessage` | `LockRequestMessageSchema` | type = "LOCK_REQUEST" |
| `LockReleaseMessage` | `LockReleaseMessageSchema` | type = "LOCK_RELEASE" |

**Counter:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `PNCounterState` | `PNCounterStateSchema` | increments, decrements maps |
| `CounterRequestMessage` | `CounterRequestMessageSchema` | type = "COUNTER_REQUEST" |
| `CounterSyncMessage` | `CounterSyncMessageSchema` | type = "COUNTER_SYNC" |
| `CounterResponseMessage` | `CounterResponseMessageSchema` | type = "COUNTER_RESPONSE" |
| `CounterUpdateMessage` | `CounterUpdateMessageSchema` | type = "COUNTER_UPDATE" |

**Heartbeat:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `PingMessage` | `PingMessageSchema` | type = "PING" |
| `PongMessage` | `PongMessageSchema` | type = "PONG" |

**EntryProcessor:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `EntryProcessor` | `EntryProcessorSchema` | name, code, optional config |
| `EntryProcessRequest` | `EntryProcessRequestSchema` | type = "ENTRY_PROCESS_REQUEST" |
| `EntryProcessBatchRequest` | `EntryProcessBatchRequestSchema` | type = "ENTRY_PROCESS_BATCH_REQUEST" |
| `EntryProcessKeyResult` | `EntryProcessKeyResultSchema` | key, result (rmpv::Value), optional error |
| `EntryProcessResponse` | `EntryProcessResponseSchema` | type = "ENTRY_PROCESS_RESPONSE" |
| `EntryProcessBatchResponse` | `EntryProcessBatchResponseSchema` | type = "ENTRY_PROCESS_BATCH_RESPONSE" |

**Journal:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `JournalEventType` | `JournalEventTypeSchema` | Enum: SET, DELETE, OR_ADD, OR_REMOVE, PROCESS, BATCH_PROCESS, MERGE_REJECTED |
| `JournalEventData` | `JournalEventDataSchema` | Has `#[serde(rename = "type")] event_type: JournalEventType` (Rust keyword conflict) |
| `JournalSubscribeRequest` | `JournalSubscribeRequestSchema` | type = "JOURNAL_SUBSCRIBE" |
| `JournalUnsubscribeRequest` | `JournalUnsubscribeRequestSchema` | type = "JOURNAL_UNSUBSCRIBE" |
| `JournalEventMessage` | `JournalEventMessageSchema` | type = "JOURNAL_EVENT" |
| `JournalReadRequest` | `JournalReadRequestSchema` | type = "JOURNAL_READ" |
| `JournalReadResponse` | `JournalReadResponseSchema` | type = "JOURNAL_READ_RESPONSE" |

**ConflictResolver:**

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `ConflictResolver` | `ConflictResolverSchema` | name, code, optional config/priority |
| `RegisterResolverRequest` | `RegisterResolverRequestSchema` | type = "REGISTER_RESOLVER" |
| `RegisterResolverResponse` | `RegisterResolverResponseSchema` | type = "REGISTER_RESOLVER_RESPONSE" |
| `UnregisterResolverRequest` | `UnregisterResolverRequestSchema` | type = "UNREGISTER_RESOLVER" |
| `UnregisterResolverResponse` | `UnregisterResolverResponseSchema` | type = "UNREGISTER_RESOLVER_RESPONSE" |
| `MergeRejectedMessage` | `MergeRejectedMessageSchema` | type = "MERGE_REJECTED" |
| `ResolverInfo` | `ResolverInfoSchema` | name, mapNames, priority |
| `ListResolversRequest` | `ListResolversRequestSchema` | type = "LIST_RESOLVERS" |
| `ListResolversResponse` | `ListResolversResponseSchema` | type = "LIST_RESOLVERS_RESPONSE" |

### Domain 7: Client Event Messages (~12 types)
**Source:** `client-message-schemas.ts`

| Rust Type | TS Source | Notes |
|-----------|-----------|-------|
| `AuthAckMessage` | `AuthAckMessageSchema` | type = "AUTH_ACK", optional protocolVersion |
| `AuthFailMessage` | `AuthFailMessageSchema` | type = "AUTH_FAIL" |
| `ErrorPayload` | `ErrorPayloadSchema` | code (number), message (String), optional details (rmpv::Value) |
| `ErrorMessage` | `ErrorMessageSchema` | type = "ERROR" |
| `ServerEventType` | `ServerEventTypeSchema` | Enum: PUT, REMOVE, OR_ADD, OR_REMOVE -- DISTINCT from ChangeEventType |
| `ServerEventPayload` | `ServerEventPayloadSchema` | mapName, key, eventType (ServerEventType), value, timestamp |
| `ServerEventMessage` | `ServerEventMessageSchema` | type = "SERVER_EVENT" |
| `ServerBatchEventMessage` | `ServerBatchEventMessageSchema` | type = "SERVER_BATCH_EVENT", events: Vec<ServerEventPayload> |
| `QueryUpdatePayload` | `QueryUpdatePayloadSchema` | subscriptionId, changeType (ChangeEventType), key, value, optional totalCount/cursorStatus |
| `QueryUpdateMessage` | `QueryUpdateMessageSchema` | type = "QUERY_UPDATE" |
| `GcPrunePayload` | `GcPrunePayloadSchema` | mapName, keys |
| `GcPruneMessage` | `GcPruneMessageSchema` | type = "GC_PRUNE" |
| `LockGrantedPayload` | `LockGrantedPayloadSchema` | name, lockId, optional ttl |
| `LockGrantedMessage` | `LockGrantedMessageSchema` | type = "LOCK_GRANTED" |
| `LockReleasedPayload` | `LockReleasedPayloadSchema` | name, lockId |
| `LockReleasedMessage` | `LockReleasedMessageSchema` | type = "LOCK_RELEASED" |
| `SyncResetRequiredPayload` | `SyncResetRequiredPayloadSchema` | mapName, reason |
| `SyncResetRequiredMessage` | `SyncResetRequiredMessageSchema` | type = "SYNC_RESET_REQUIRED" |

Note: `CursorStatus` is needed for `QueryUpdatePayload.cursorStatus`. It is defined in `query.rs` (SPEC-052b). If SPEC-052b is complete, import it. Otherwise, define a local copy (simple 4-variant enum).

### Files to Create

| File | Contents |
|------|----------|
| `packages/core-rust/src/messages/messaging.rs` | All messaging domain structs (~30 types including sub-types) |
| `packages/core-rust/src/messages/client_events.rs` | All client event structs (~18 types including sub-types) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/core-rust/src/messages/mod.rs` | Add `pub mod messaging;` and `pub mod client_events;` declarations + re-exports |

**Total: 2 new + 1 modified = 3 files**

## Acceptance Criteria

1. **AC-messaging-roundtrip:** All messaging domain structs (topic, lock, counter, heartbeat, entry processor, journal, conflict resolver messages) round-trip through `to_vec_named()` / `from_slice()` without data loss.

2. **AC-client-events-roundtrip:** All client event structs (`AuthAckMessage`, `AuthFailMessage`, `ErrorMessage`, `ServerEventMessage`, `ServerBatchEventMessage`, `QueryUpdateMessage`, `GcPruneMessage`, `LockGrantedMessage`, `LockReleasedMessage`, `SyncResetRequiredMessage`) round-trip through `to_vec_named()` / `from_slice()` without data loss.

3. **AC-journal-type-field:** `JournalEventData` with `#[serde(rename = "type")]` on its `event_type` field serializes the field as `"type"` in the MsgPack map. Verified by byte inspection or round-trip.

4. **AC-event-type-distinction:** `ServerEventType` (PUT/REMOVE/OR_ADD/OR_REMOVE) and `ChangeEventType` (ENTER/UPDATE/LEAVE) are separate Rust enums with different variant sets. Both serialize to their expected string values.

5. **AC-7 (from parent): cargo test passes.** All existing core-rust tests pass. All new messaging/client-events serde tests pass. No regressions.

## Constraints

- Do NOT implement message handler logic -- strictly struct definitions and serde configuration.
- Do NOT change the TS wire format -- Rust must conform to what TS already produces.
- Do NOT use `rmp_serde::to_vec()` for wire messages -- always use `rmp_serde::to_vec_named()`.
- Do NOT confuse `ServerEventType` with `ChangeEventType` -- they are distinct enums for different purposes.
- Max 5 files modified/created.

## Assumptions

- `ChangeEventType`, `Timestamp` are available from SPEC-052a.
- `CursorStatus` from SPEC-052b is available or can be locally defined (simple 4-variant string enum).
- Payload structs are nested under a `payload` field (not flattened), matching the TS wire format.
- `JournalEventData.type` field does NOT conflict with the `Message` enum discriminant (different nesting level).

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `messages/messaging.rs` with topic, lock, counter, heartbeat structs | -- | ~8% |
| G2 | 1 | Add entry processor, journal, conflict resolver structs to `messaging.rs` | -- | ~8% |
| G3 | 1 | Create `messages/client_events.rs` with all server-to-client event structs | -- | ~8% |
| G4 | 2 | Update `messages/mod.rs`, add unit tests for round-trip | G1, G2, G3 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |
| 2 | G4 | No | 1 |

**Total workers needed:** 3 (max in any wave)

---
*Child of SPEC-052. Created by /sf:split on 2026-02-15.*
