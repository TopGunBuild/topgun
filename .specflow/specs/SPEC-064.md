# SPEC-064: Implement MessagingService (Topic Pub/Sub)

---
id: SPEC-064
type: feature
status: auditing
priority: P1
complexity: small
created: 2026-02-25
todo: TODO-087
---

## Context

Phase 3b domain service replacement. Three domain services have been replaced so far (CoordinationService SPEC-061, CrdtService SPEC-062, SyncService SPEC-063). The `MessagingService` is currently a `domain_stub!` macro that returns `OperationResponse::NotImplemented` for all operations.

Topic pub/sub is an ephemeral messaging channel independent of the CRDT data path. Clients subscribe to named topics, publish arbitrary data to topics, and receive messages published by other clients. The service manages per-topic subscriber sets and fan-out delivery. Topics are auto-removed when the last subscriber disconnects.

### TS Behavioral Reference

The TypeScript `TopicManager` (`packages/server/src/topic/TopicManager.ts`) provides the behavioral spec:
- `subscribe(clientId, topic)`: validates topic name, enforces subscription limit (100), adds to `topic -> Set<clientId>` map
- `unsubscribe(clientId, topic)`: removes from set, deletes topic entry if set becomes empty
- `unsubscribeAll(clientId)`: cleanup on disconnect (iterates all topics)
- `publish(topic, data, senderId)`: validates topic, constructs `TOPIC_MESSAGE` payload, sends to all subscribers except publisher, cluster broadcast (deferred for this spec)

The TS `TopicHandler` wraps `TopicManager` with security checks. Security/ACL is deferred (no security framework in Rust server yet).

### Key Design Observations

1. `ConnectionMetadata` already has a `topics: HashSet<String>` field for tracking per-connection topic subscriptions
2. `ConnectionRegistry` provides `broadcast()` for all-clients fan-out but not topic-targeted delivery; topic fan-out requires iterating subscribers
3. All three message types (`TopicSubPayload`, `TopicUnsubPayload`, `TopicPubPayload`) and the response type (`TopicMessageEventPayload`) already exist in `packages/core-rust/src/messages/messaging.rs`
4. The `Message::TopicMessage` variant already exists in the `Message` enum
5. Classification already routes `TOPIC_SUB`/`TOPIC_UNSUB`/`TOPIC_PUB` to `Operation::TopicSubscribe`/`TopicUnsubscribe`/`TopicPublish` with `service_name = "messaging"`

## Task

Replace the `domain_stub!(MessagingService, ...)` macro invocation with a real `MessagingService` struct that implements `ManagedService` + `tower::Service<Operation>`. The service manages an in-memory `TopicRegistry` (topic name -> set of `ConnectionId`) and delivers published messages to subscribers via the `ConnectionRegistry`.

## Requirements

### Files to Create

1. **`packages/server-rust/src/service/domain/messaging.rs`** -- Real `MessagingService` implementation

   - `TopicRegistry` struct: thread-safe mapping of topic names to subscriber sets
     - Inner type: `DashMap<String, DashSet<ConnectionId>>` (or `DashMap<String, HashSet<ConnectionId>>` with fine-grained locking)
     - `subscribe(topic: &str, conn_id: ConnectionId) -> Result<(), TopicError>`: adds subscriber; validates topic name (non-empty, max 256 chars, pattern `[\w\-.:/]+`)
     - `unsubscribe(topic: &str, conn_id: ConnectionId)`: removes subscriber; removes topic entry if set is empty
     - `unsubscribe_all(conn_id: ConnectionId)`: removes connection from all topics; removes empty topic entries
     - `subscribers(topic: &str) -> Vec<ConnectionId>`: returns current subscriber list for a topic (empty vec if topic does not exist)
     - `topic_count() -> usize`: returns number of active topics (for testing)
   - `TopicError` enum:
     - `InvalidTopicName { topic: String }` -- topic fails validation
   - `MessagingService` struct with fields:
     - `topic_registry: Arc<TopicRegistry>`
     - `connection_registry: Arc<ConnectionRegistry>`
   - Constructor: `MessagingService::new(connection_registry: Arc<ConnectionRegistry>) -> Self` (creates internal `TopicRegistry`)
   - `ManagedService` impl: `name()` returns `"messaging"`; `init`/`reset`/`shutdown` are no-ops (ephemeral state)
   - `Service<Operation>` impl on `Arc<MessagingService>`:
     - `TopicSubscribe`: validate topic name, add `ctx.connection_id` to topic registry, also add topic to `ConnectionMetadata.topics`; return `OperationResponse::Empty`
     - `TopicUnsubscribe`: remove `ctx.connection_id` from topic registry, remove topic from `ConnectionMetadata.topics`; return `OperationResponse::Empty`
     - `TopicPublish`: validate topic name, build `TopicMessageEventPayload` (topic, data, publisher_id from `ctx.client_id`, timestamp as `u64` milliseconds since UNIX epoch via `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64`), serialize as `Message::TopicMessage`, send to each subscriber's connection via `ConnectionRegistry::get()` + `try_send()`, skip the publisher's own connection; return `OperationResponse::Empty`
     - All other operations: return `Err(OperationError::WrongService)`

### Files to Modify

2. **`packages/server-rust/src/service/domain/mod.rs`** -- Wire up the new module
   - Add `pub mod messaging;` and `pub use messaging::MessagingService;`
   - Remove the `domain_stub!(MessagingService, ...)` macro invocation
   - Update the existing test `messaging_service_returns_not_implemented` to either remove it or change it to test real behavior (the stub test will no longer compile since `MessagingService` will require constructor args)
   - Update the `all_stubs_implement_managed_service` test: remove `MessagingService` from the stub registration block (it no longer uses `domain_stub!` and requires constructor args). This follows the same pattern used in SPEC-061/062/063 when promoting stubs to real services.

3. **`packages/server-rust/src/lib.rs`** -- Update integration test wiring
   - Change `router.register(service_names::MESSAGING, Arc::new(MessagingService));` to `router.register(service_names::MESSAGING, Arc::new(MessagingService::new(Arc::clone(&connection_registry))));`
   - Update `ServiceRegistry` registration in the `setup()` helper similarly: `registry.register(MessagingService::new(Arc::clone(&connection_registry)));`
   - Update the `service_registry_lifecycle` test (line 290): change `registry.register(MessagingService);` to `registry.register(MessagingService::new(Arc::clone(&connection_registry)));` (or a dedicated `ConnectionRegistry` instance), following the same pattern already used for `CrdtService`, `SyncService`, and `CoordinationService` in that test
   - Rename the test function `full_pipeline_topic_subscribe_to_not_implemented` to `full_pipeline_topic_subscribe_returns_empty` to reflect the updated expected behavior (`OperationResponse::Empty` instead of `NotImplemented`)

## Goal Analysis

**Goal Statement:** Clients can subscribe to topics and receive messages published by other clients via the `MessagingService`, with automatic cleanup when connections end.

**Observable Truths:**
1. A client sending `TOPIC_SUB` becomes a subscriber for that topic
2. A client sending `TOPIC_PUB` delivers a `TOPIC_MESSAGE` to all other subscribers of that topic (not back to publisher)
3. A client sending `TOPIC_UNSUB` stops receiving messages for that topic
4. Invalid topic names are rejected with an error
5. Topics are removed from the registry when their last subscriber leaves
6. The service integrates with the existing Tower pipeline (classify -> route -> handle)

**Required Artifacts:**
- `messaging.rs` (service implementation + topic registry + tests)
- Modified `mod.rs` (module wiring, stub removal)
- Modified `lib.rs` (integration test wiring)

**Key Links:**
- `TopicRegistry` -> `ConnectionRegistry`: subscriber delivery depends on looking up `ConnectionHandle` by `ConnectionId`
- `OperationContext.connection_id` -> `TopicRegistry`: subscribe/unsubscribe uses `connection_id` to identify the caller
- `OperationContext.client_id` -> `TopicMessageEventPayload.publisher_id`: publisher identity flows through

**Fragile Links:**
- `connection_id` must be `Some` for subscribe/publish to work; if absent, the operation should return an error (not panic)

## Acceptance Criteria

**AC1: TopicSubscribe adds connection to topic registry and connection metadata**
- Given a `TopicSubscribe` operation with `topic = "chat/room-1"` and `ctx.connection_id = Some(conn_id)`
- When the service handles the operation
- Then `topic_registry.subscribers("chat/room-1")` contains `conn_id`
- And `ConnectionMetadata.topics` for `conn_id` contains `"chat/room-1"`
- And the response is `OperationResponse::Empty`

**AC2: TopicUnsubscribe removes connection from topic registry and connection metadata**
- Given a connection subscribed to `"chat/room-1"`
- When a `TopicUnsubscribe` operation is handled for that topic and connection
- Then `topic_registry.subscribers("chat/room-1")` does not contain `conn_id`
- And `ConnectionMetadata.topics` for `conn_id` does not contain `"chat/room-1"`

**AC3: TopicPublish delivers TOPIC_MESSAGE to subscribers (excluding publisher)**
- Given connections A, B, C subscribed to `"news"`
- When connection A publishes `{ data: "hello" }` to `"news"`
- Then connections B and C each receive a serialized `Message::TopicMessage` with the published data
- And connection A does NOT receive the message
- And the `TopicMessageEventPayload` contains `publisher_id` matching A's `client_id` and a `timestamp` within 1 second of now

**AC4: TopicPublish to topic with no subscribers returns Empty (no error)**
- Given no subscribers for topic `"empty-topic"`
- When a `TopicPublish` operation targets `"empty-topic"`
- Then the response is `OperationResponse::Empty` (no error)

**AC5: Invalid topic name rejected**
- Given a `TopicSubscribe` with topic `""` (empty) or `"bad topic!"` (invalid chars) or a topic exceeding 256 characters
- When the service handles the operation
- Then an `OperationError::Internal` is returned with an appropriate message

**AC6: Topic auto-removed when last subscriber unsubscribes**
- Given only one connection subscribed to `"ephemeral"`
- When that connection unsubscribes from `"ephemeral"`
- Then `topic_registry.topic_count()` decreases by one (the topic entry is deleted)

**AC7: Missing connection_id on TopicSubscribe returns error**
- Given a `TopicSubscribe` operation with `ctx.connection_id = None`
- When the service handles the operation
- Then an `OperationError::Internal` is returned (not a panic)

**AC8: Wrong service returns WrongService error**
- Given a non-messaging operation (e.g., `GarbageCollect`)
- When the service handles it
- Then `Err(OperationError::WrongService)` is returned

**AC9: ManagedService name is "messaging"**
- `MessagingService::new(...).name()` returns `"messaging"`

**AC10: Integration test wiring compiles and routes messaging operations**
- The `lib.rs` integration test (the `setup()` helper and the `full_pipeline_topic_subscribe_returns_empty` test) successfully creates a `MessagingService` with constructor args and registers it in the `OperationRouter`
- A `TopicSubscribe` operation routed through the pipeline returns `OperationResponse::Empty` (not `NotImplemented`)

**AC11: unsubscribe_all removes connection from all topics and cleans up empty entries**
- Given a connection subscribed to `"topic-a"`, `"topic-b"`, and `"topic-c"`
- When `unsubscribe_all` is called for that connection
- Then `topic_registry.subscribers("topic-a")`, `topic_registry.subscribers("topic-b")`, and `topic_registry.subscribers("topic-c")` each return an empty vec
- And `topic_registry.topic_count()` reflects removal of any topics that had only that connection as subscriber

**AC12: Missing connection_id on TopicPublish returns error**
- Given a `TopicPublish` operation with `ctx.connection_id = None`
- When the service handles the operation
- Then an `OperationError::Internal` is returned (not a panic)

## Constraints

- Do NOT implement cluster-level topic broadcast (cross-node pub/sub). That requires ClusterManager integration and is out of scope.
- Do NOT implement security/ACL checks on topics. No security framework exists yet in the Rust server.
- Do NOT add per-client subscription limits in this spec. The TS TopicManager has a 100-subscription limit, but implementing it requires efficient per-connection subscription counting. The `ConnectionMetadata.topics` field can serve this purpose in the future but enforcing limits is deferred.
- Do NOT modify any files in `packages/core-rust/` -- all message types already exist.
- The `TopicRegistry` is ephemeral (in-memory only, no persistence). This is correct for pub/sub.
- Use `DashMap` for the topic registry (consistent with `ConnectionRegistry` pattern).

## Assumptions

- **Topic validation regex `[\w\-.:/]+`**: Carried over from TS. Allows alphanumeric, underscore, hyphen, dot, colon, forward slash. No spaces or special characters.
- **`OperationResponse::Empty` for subscribe/unsubscribe**: The TS handler sends no response for these operations. `Empty` is the correct response variant.
- **Publisher excluded from delivery**: The TS TopicManager explicitly skips the sender (`clientId !== senderId`). Rust does the same by comparing `ctx.connection_id`.
- **No subscription acknowledgment message**: Subscribe/unsubscribe are fire-and-forget from the server's perspective. No `TOPIC_SUB_ACK` message exists.
- **Subscription limit deferred**: The TS has a 100-topic limit per client. Implementing this cleanly requires either iterating all topics (O(n)) or maintaining a reverse index. Deferred to avoid scope creep; `ConnectionMetadata.topics` tracks subscriptions for future enforcement.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define `TopicRegistry` struct, `TopicError` enum, `MessagingService` struct with constructor | -- | ~8% |
| G2 | 2 | Implement `ManagedService` + `Service<Operation>` for `MessagingService`, handler methods (subscribe, unsubscribe, publish) | G1 | ~15% |
| G3 | 2 | Wire module in `mod.rs` (remove stub, add module declaration), update `lib.rs` integration wiring | G1 | ~7% |
| G4 | 3 | Unit tests for all ACs (AC1-AC12) | G2, G3 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-25)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total (re-estimated: G1 ~8%, G2 ~15%, G3 ~7%, G4 ~15%)

**Critical:**
1. **Contradiction: `SubscriptionLimitReached` defined but limits deferred.** The `TopicError` enum defines `SubscriptionLimitReached { limit: usize }` and the `subscribe` method says "returns error if subscriber count exceeds limit," but the Constraints section explicitly states "Do NOT add per-client subscription limits in this spec." Either remove the `SubscriptionLimitReached` variant and the limit check from `subscribe`, or remove the constraint. Since the constraint is well-reasoned (deferred to avoid scope creep), the error variant and limit-check language should be removed from Requirements.

2. **Missing AC for `unsubscribe_all`.** The `TopicRegistry` defines `unsubscribe_all(conn_id)` for disconnect cleanup, and the Goal Statement says "automatic cleanup when connections end," but no acceptance criterion tests this method. Add an AC (e.g., AC11) that verifies: given a connection subscribed to multiple topics, when `unsubscribe_all` is called for that connection, then the connection is removed from all topics and empty topics are cleaned up. Without this AC, the method may be implemented but untested.

3. **Missing: `all_stubs_implement_managed_service` test update in `mod.rs`.** The spec mentions updating the `messaging_service_returns_not_implemented` test, but the `all_stubs_implement_managed_service` test at line 187 also creates `MessagingService` as a unit struct (`registry.register(MessagingService);`). This will fail to compile once `MessagingService` requires constructor args. The spec must mention updating this test as well (similar to how SPEC-061/062/063 handled removing real services from the stub registration test).

4. **AC10 references non-existent function name.** AC10 says "the `lib.rs` integration test (`create_pipeline_with_classify_and_route`)" but the actual integration test helper function is called `setup()` and the test is `full_pipeline_topic_subscribe_to_not_implemented`. Fix the reference to match the actual code.

**Recommendations:**
5. [Strategic] The spec's context estimates in the Implementation Tasks table sum to 100% (15+35+10+40), which is implausible for a small spec with 3 files. Realistic estimates are closer to ~45% total. Consider updating the estimates to be more accurate so they provide useful guidance during execution.
6. The `TopicPublish` handler specifies "timestamp from `SystemTime::now()`" -- clarify this means converting `SystemTime` to `u64` milliseconds since epoch (e.g., `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64`), since `TopicMessageEventPayload.timestamp` is `u64`.
7. Consider whether `TopicPublish` with `connection_id = None` should also be tested (AC7 only tests `TopicSubscribe` with missing `connection_id`). The `TopicPublish` handler also needs `connection_id` to exclude the publisher from delivery.

### Response v1 (2026-02-25)
**Applied:** All 4 critical issues and all 3 recommendations

**Changes:**
1. [x] Remove `SubscriptionLimitReached` variant and limit-check language -- removed `SubscriptionLimitReached { limit: usize }` from `TopicError` enum and removed "returns error if subscriber count exceeds limit" from `subscribe` method description in Requirements
2. [x] Add AC11 for `unsubscribe_all` -- added AC11 verifying that calling `unsubscribe_all` for a connection removes it from all subscribed topics and that `topic_count()` reflects cleanup of empty entries
3. [x] Mention `all_stubs_implement_managed_service` test update -- added explicit instruction to update this test in `mod.rs` requirements (Files to Modify, item 2), referencing the SPEC-061/062/063 pattern
4. [x] Fix AC10 function name reference -- corrected "create_pipeline_with_classify_and_route" to "`setup()` helper and the `full_pipeline_topic_subscribe_to_not_implemented` test"
5. [x] Update context estimates to realistic values -- changed G1 from ~15% to ~8%, G2 from ~35% to ~15%, G3 from ~10% to ~7%, G4 from ~40% to ~15% (total ~45% as auditor suggested)
6. [x] Clarify timestamp conversion -- updated `TopicPublish` handler description to explicitly state `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64`
7. [x] Add AC12 for `TopicPublish` with missing `connection_id` -- added AC12 verifying that `TopicPublish` with `ctx.connection_id = None` returns `OperationError::Internal` (not a panic)

### Audit v2 (2026-02-25)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total

**Critical:**
1. **Missing `lib.rs` `service_registry_lifecycle` test update.** The spec instructs updating the `setup()` function (line 99) and the `full_pipeline_topic_subscribe_to_not_implemented` test, but does NOT mention the `service_registry_lifecycle` test at line 290 which also uses `MessagingService` as a unit struct: `registry.register(MessagingService);`. This will fail to compile once `MessagingService` requires constructor args. Add an instruction to Files to Modify item 3 (`lib.rs`) to also update the `service_registry_lifecycle` test, changing `registry.register(MessagingService);` to `registry.register(MessagingService::new(Arc::clone(&connection_registry)));` (or a dedicated `ConnectionRegistry` instance), following the same pattern already used for `CrdtService`, `SyncService`, and `CoordinationService` in that test.

**Recommendations:**
2. [Completeness] The `full_pipeline_topic_subscribe_to_not_implemented` test name will be misleading after this change since it will return `OperationResponse::Empty` instead of `NotImplemented`. The spec should explicitly mention renaming this test function (e.g., to `full_pipeline_topic_subscribe_returns_empty`) for clarity.
3. [Completeness] The spec says `TopicSubscribe` should "also add topic to `ConnectionMetadata.topics`" and `TopicUnsubscribe` should "remove topic from `ConnectionMetadata.topics`", but no AC tests this metadata update. Consider adding a verification step in AC1 and AC2 that checks the `ConnectionMetadata.topics` field, or note that this behavior is covered indirectly. Without this, the implementer might skip the metadata update since no test verifies it.

### Response v2 (2026-02-25)
**Applied:** All 1 critical issue and all 2 recommendations

**Changes:**
1. [x] Add `service_registry_lifecycle` test update to `lib.rs` instructions -- added explicit bullet to Files to Modify item 3 instructing the implementer to update `registry.register(MessagingService);` to `registry.register(MessagingService::new(Arc::clone(&connection_registry)));` in the `service_registry_lifecycle` test, following the same pattern used for `CrdtService`, `SyncService`, and `CoordinationService`
2. [x] Rename misleading test function -- added explicit instruction to rename `full_pipeline_topic_subscribe_to_not_implemented` to `full_pipeline_topic_subscribe_returns_empty` in the `lib.rs` wiring instructions, and updated the AC10 reference to use the new name
3. [x] Add `ConnectionMetadata.topics` verification to AC1 and AC2 -- AC1 now verifies that `ConnectionMetadata.topics` for `conn_id` contains `"chat/room-1"` after subscribe; AC2 now verifies that `ConnectionMetadata.topics` for `conn_id` does not contain `"chat/room-1"` after unsubscribe

### Audit v3 (2026-02-25)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Comment:** Specification is well-structured, complete, and ready for implementation. All prior critical issues (7 total across v1 and v2) have been properly addressed. The spec follows the established domain service replacement pattern (SPEC-061/062/063), all acceptance criteria are concrete and testable, scope boundaries are clear, and all compilation-breaking test updates are explicitly listed. Code verification confirms all `MessagingService` references in the codebase are accounted for in the modification instructions.

**Rust Auditor Checklist:** All applicable items pass. No new serialized structs are created (uses existing `core-rust` types). Timestamp uses `u64` (correct). `DashMap` is already a dependency.

**Dimensions:** Clarity (pass), Completeness (pass), Testability (pass), Scope (pass), Feasibility (pass), Architecture fit (pass), Non-duplication (pass), Cognitive load (pass), Strategic fit (aligned with project goals), Project compliance (honors PROJECT.md decisions), Language profile (compliant with Rust profile -- 3 files within 5-file limit, G1 is types-only).

**Recommendations:**
1. [Clarity] Files to Modify item 3, second bullet says "Update `ServiceRegistry` registration in the `setup()` helper" but the `setup()` function only uses `OperationRouter`, not `ServiceRegistry`. The `ServiceRegistry` usage is in the `service_registry_lifecycle` test (covered by the third bullet). This is a minor wording imprecision that will not cause implementation errors since the implementer will see the actual code, but could be cleaned up for consistency.
