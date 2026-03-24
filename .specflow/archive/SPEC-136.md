> **SPLIT:** This specification was decomposed into:
> - SPEC-136a: Shapes: Types, Wire Messages, Operation Variants
> - SPEC-136b: Shapes: ShapeEvaluator Module and ShapeRegistry
> - SPEC-136c: Shapes: ShapeService and CRDT Broadcast Filtering
> - SPEC-136d: Shapes: Per-Shape Merkle Trees and Shape-Aware Sync
> - SPEC-136e: Shapes: TS Client Shape API and Integration Tests
>
> See child specifications for implementation.

---
id: SPEC-136
type: feature
status: needs_decomposition
priority: P1
complexity: large
created: 2026-03-21
source: TODO-070
---

# Partial Replication / Shapes

## Context

TopGun currently syncs entire maps to every connected client. Competitors (PowerSync, ElectricSQL, Replicache) allow clients to subscribe to data subsets ("shapes") defined by filters and field projections. This is table stakes for competitive parity — mobile clients cannot hold full server datasets in memory.

The Schema System (TODO-069, SPEC-127/128/129/130) is complete, providing `MapSchema`, `SyncShape` (placeholder struct), and `Predicate` (placeholder struct) in `core-rust`. The `SchemaProvider` trait already has a `get_shape()` method. The `PredicateEngine` already evaluates `PredicateNode` trees against `rmpv::Value` data. These foundations make shapes implementable now.

### Dependencies

- **TODO-069 (Schema System):** COMPLETE. Provides `MapSchema`, `SchemaProvider` trait, `SyncShape` placeholder.
- **SPEC-135a/b/c (DataFusion SQL):** COMPLETE. Provides `PredicateNode`, `evaluate_predicate()`.

## Goal Statement

Clients subscribe to named data shapes (filter + field projection + limit per map). The server syncs only matching records for each shape, and live mutations are filtered through shapes before broadcast. Merkle trees track per-shape state for efficient delta sync on reconnect.

### Observable Truths

1. A client sends a `SHAPE_SUBSCRIBE` message specifying map name, filter predicate, field projection, and limit.
2. The server evaluates the shape filter against all records in the map and returns only matching records (with projected fields).
3. When a mutation occurs, the server evaluates it against all active shapes and broadcasts only to clients whose shape matches.
4. On reconnect, shape-aware Merkle sync sends only the delta of records matching the client's shape.
5. A client sends `SHAPE_UNSUBSCRIBE` to stop receiving updates for a shape.
6. Multiple shapes per client are supported (e.g., different filters on the same map).
7. Field projection strips non-projected fields from synced records before sending to client.

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `core-rust/src/schema.rs` (modified) | Replace placeholder `SyncShape` and `Predicate` with full types using `PredicateNode` |
| `core-rust/src/messages/shape.rs` (new) | Wire messages: `ShapeSubscribeMessage`, `ShapeUnsubscribeMessage`, `ShapeRespMessage`, `ShapeUpdateMessage` |
| `core-rust/src/messages/mod.rs` (modified) | Register new `Message` variants for shape messages |
| `server-rust/src/service/domain/shape.rs` (new) | `ShapeService` domain service: shape subscription lifecycle, evaluation, registry |
| `server-rust/src/service/domain/shape_evaluator.rs` (new) | `ShapeEvaluator` trait + impl: filter matching, field projection, limit enforcement |
| `server-rust/src/storage/shape_merkle.rs` (new) | `ShapeMerkleSyncManager`: per-shape Merkle trees keyed by `(shape_id, map_name, partition_id)` |
| `server-rust/src/service/operation.rs` (modified) | New `Operation` variants: `ShapeSubscribe`, `ShapeUnsubscribe` |
| `server-rust/src/service/classify.rs` (modified) | Classify shape messages into operations |
| `server-rust/src/service/domain/crdt.rs` (modified) | Shape-filtered broadcast on mutation |
| `server-rust/src/service/domain/sync.rs` (modified) | Shape-aware Merkle sync protocol |
| `client/src/SyncEngine.ts` (modified) | Shape subscription API on client |
| `tests/integration-rust/` (new tests) | End-to-end shape subscribe/sync tests |

### Key Links (Fragile Connections)

1. **Shape filter -> PredicateNode reuse:** Shape filters reuse the existing `PredicateNode` tree and `evaluate_predicate()` function. If `Predicate` struct changes shape, both shape evaluation and query evaluation must stay compatible.
2. **Shape Merkle -> existing MerkleSyncManager:** Shape Merkle trees extend the existing `(map_name, partition_id)` keying to `(shape_id, map_name, partition_id)`. Must not break existing full-map sync.
3. **CRDT broadcast -> ShapeRegistry:** After a mutation, CrdtService must consult the shape registry to filter which connections receive the update. This is similar to the existing QueryRegistry pattern.
4. **Wire messages -> TS client:** New MsgPack message types must be decodable by the TS client's `msgpackr`.

## Task

Implement partial replication / shapes for TopGun, enabling clients to subscribe to filtered subsets of map data with field projection and limit, using per-shape Merkle trees for efficient delta sync.

## Requirements

### R1: Upgrade SyncShape and Predicate types

Replace the placeholder `SyncShape` and `Predicate` structs in `core-rust/src/schema.rs`:

- `SyncShape`: `map_name: String`, `filter: Option<PredicateNode>` (reuse existing tree), `fields: Option<Vec<String>>`, `limit: Option<u32>`, `shape_id: String` (client-assigned, UUID)
- **Breaking change note:** `SyncShape.limit` type changes from `Option<usize>` to `Option<u32>` per Rust Type Mapping Rules (page size / limit fields must be `u32`). Any existing code referencing `SyncShape.limit` as `usize` must be updated.
- Remove standalone `Predicate` struct (replaced by direct `PredicateNode` usage)
- Add `#[derive(Default)]` to `SyncShape` (3 optional fields satisfy the 2+ optional fields rule)
- Add `#[serde(rename_all = "camelCase")]` to `SyncShape`
- Add `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` field in `SyncShape`

### R2: Shape wire messages

New messages in `core-rust/src/messages/shape.rs`. All payload structs derive `#[serde(rename_all = "camelCase")]` and use `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` field.

**Payload structs:**

- `ShapeSubscribePayload`: `shape_id: String`, `map_name: String`, `filter: Option<PredicateNode>`, `fields: Option<Vec<String>>`, `limit: Option<u32>`
  - Derives `Default` (3 optional fields)
- `ShapeUnsubscribePayload`: `shape_id: String`
- `ShapeRecord`: `key: String`, `value: rmpv::Value` -- the unit of record transfer in shape responses
- `ShapeRespPayload`: `shape_id: String`, `records: Vec<ShapeRecord>`, `merkle_root_hash: u32`, `has_more: Option<bool>`
- `ShapeUpdatePayload`: `shape_id: String`, `key: String`, `value: Option<rmpv::Value>` (None = removed from shape), `change_type: ChangeEventType`
- `ShapeSyncInitPayload`: `shape_id: String`, `root_hash: u32` (client's current shape Merkle root)

**Message wrapper structs** (following `SyncInitMessage` naming convention):

- `ShapeSubscribeMessage`: wraps `ShapeSubscribePayload`
- `ShapeUnsubscribeMessage`: wraps `ShapeUnsubscribePayload`
- `ShapeRespMessage`: wraps `ShapeRespPayload`
- `ShapeUpdateMessage`: wraps `ShapeUpdatePayload`
- `ShapeSyncInitMessage`: wraps `ShapeSyncInitPayload`

**Merkle sync message strategy -- reuse existing protocol message structures:**

Shape Merkle sync reuses the existing `SyncRespRootMessage`, `SyncRespBucketsMessage`, and `SyncRespLeafMessage` structs from the full-map sync protocol. Shape paths are distinguished by prefixing the Merkle path component with the `shape_id` (e.g., `"<shape_id>/<partition_id>/<depth>/<bucket>"`). No new `ShapeSyncResp*` message variants are added.

**New `Message` enum variants:**

- `ShapeSubscribe(ShapeSubscribeMessage)`
- `ShapeUnsubscribe(ShapeUnsubscribeMessage)`
- `ShapeResp(ShapeRespMessage)`
- `ShapeUpdate(ShapeUpdateMessage)`
- `ShapeSyncInit(ShapeSyncInitMessage)`

### R3: ShapeEvaluator trait and implementation

`ShapeEvaluator` in `server-rust/src/service/domain/shape_evaluator.rs`:

- `fn matches(shape: &SyncShape, record: &rmpv::Value) -> bool` -- evaluates the shape's `filter: Option<PredicateNode>` against a record using `evaluate_predicate()`. If `filter` is `None`, the record always matches.
- `fn project(fields: &[String], record: &rmpv::Value) -> rmpv::Value` -- strips non-projected fields from a Map value, returns projected subset
- `fn apply_shape(shape: &SyncShape, key: &str, record: &rmpv::Value) -> Option<rmpv::Value>` -- combines match + project; returns `None` if filtered out, `Some(projected_value)` if matching

### R4: ShapeRegistry

Server-side registry tracking active shapes per connection in `server-rust/src/service/domain/shape.rs`:

- `ShapeRegistry`: `DashMap<String, ActiveShape>` keyed by `shape_id`
  - `ActiveShape`: `shape: SyncShape`, `connection_id: u64`
  - Note: `map_name` is intentionally omitted from `ActiveShape` -- it is already present on `SyncShape.map_name`. Access via `active_shape.shape.map_name`.
- `fn register(shape_id, connection_id, shape) -> Result<()>`
- `fn unregister(shape_id) -> Option<ActiveShape>`
- `fn unregister_all_for_connection(connection_id) -> Vec<String>` (returns removed shape_ids)
- `fn shapes_for_map(map_name) -> Vec<(String, ActiveShape)>` -- all active shapes targeting a map
- `fn shapes_for_connection(connection_id) -> Vec<(String, ActiveShape)>` -- all shapes for a connection

### R5: ShapeService domain service

`ShapeService` as a Tower `Service<Operation>`:

- Handles `Operation::ShapeSubscribe`: registers shape, evaluates all records in target map, sends `ShapeRespMessage` with matching records (projected), initializes per-shape Merkle tree
- Handles `Operation::ShapeUnsubscribe`: removes shape from registry, cleans up per-shape Merkle tree
- On connection disconnect: `unregister_all_for_connection` cleanup

### R6: Per-shape Merkle trees

Extend or create a `ShapeMerkleSyncManager` for per-shape Merkle trees:

- Key: `(shape_id, map_name, partition_id)` -> `MerkleTree`
- When a shape is registered, build initial Merkle tree from matching records
- When a mutation occurs on a matching record, update the shape's Merkle tree
- Shape Merkle sync protocol: client sends `ShapeSyncInitMessage` with its shape root hash, server compares using the per-shape Merkle tree and sends delta of matching records via existing `SyncRespRoot/Buckets/Leaf` messages with shape-prefixed paths
- Clean up trees when shape is unsubscribed

### R7: CRDT broadcast filtering by shape

Modify `CrdtService` broadcast path:

- After a successful mutation, `CrdtService` receives both the old record value (before mutation) and the new record value (after mutation) for the affected key
- For each active shape targeting the same map:
  - Evaluate old value against shape filter: `old_matches = ShapeEvaluator::matches(shape, old_value)`
  - Evaluate new value against shape filter: `new_matches = ShapeEvaluator::matches(shape, new_value)`
  - If `!old_matches && new_matches`: send `ShapeUpdateMessage` with `change_type: ENTER`
  - If `old_matches && new_matches`: send `ShapeUpdateMessage` with `change_type: UPDATE` (with projected new value)
  - If `old_matches && !new_matches`: send `ShapeUpdateMessage` with `change_type: LEAVE` (value field is `None`)
  - If `!old_matches && !new_matches`: do nothing
- Old value availability: `CrdtService` reads the record from `RecordStore` before applying the mutation to obtain the previous value. If no previous record exists, treat old value as non-matching (`old_matches = false`).

### R8: Shape-aware Merkle sync

Extend `SyncService` to handle `Operation::ShapeSyncInit`:

- When client sends `ShapeSyncInitMessage` with a `shape_id` and `root_hash`, perform Merkle delta sync using the per-shape Merkle tree from `ShapeMerkleSyncManager`
- Sync responses use the existing `SyncRespRootMessage`, `SyncRespBucketsMessage`, `SyncRespLeafMessage` structs with shape-prefixed paths (e.g., path component `"<shape_id>/<partition_id>"`)
- Leaf node responses contain only records matching the shape filter (with field projection applied via `ShapeEvaluator::apply_shape`)

### R9: TS client shape API

Add to TS client `SyncEngine`:

- `subscribeShape(mapName, options: { filter?, fields?, limit? }): ShapeHandle`
- `ShapeHandle`: `{ shapeId, unsubscribe(), onUpdate(cb), records: Map<string, any> }`
- Wire message handlers for `SHAPE_RESP`, `SHAPE_UPDATE`
- On reconnect: send `ShapeSyncInit` with stored Merkle root hash for each active shape

### R10: Integration tests

- Shape subscribe returns filtered records
- Shape with field projection returns only projected fields
- Mutation matching shape triggers `ShapeUpdate` to subscriber
- Mutation not matching shape does not trigger update
- Shape unsubscribe stops updates
- Reconnect shape Merkle sync sends only delta

## Acceptance Criteria

1. **AC1:** Client sends `SHAPE_SUBSCRIBE` with filter `{op: "EQ", attribute: "status", value: "active"}` on map "users" -- receives `SHAPE_RESP` containing only records where `status == "active"`.
2. **AC2:** Client sends `SHAPE_SUBSCRIBE` with `fields: ["name", "email"]` -- received records contain only `name` and `email` fields (plus key).
3. **AC3:** Client sends `SHAPE_SUBSCRIBE` with `limit: 10` -- receives at most 10 records in initial response.
4. **AC4:** After shape subscription, a write of `{status: "active"}` to a previously non-matching record triggers `SHAPE_UPDATE` with `change_type: ENTER` to the subscriber.
5. **AC5:** After shape subscription, a write changing `status` from `"active"` to `"inactive"` triggers `SHAPE_UPDATE` with `change_type: LEAVE`.
6. **AC6:** After `SHAPE_UNSUBSCRIBE`, no further `SHAPE_UPDATE` messages are sent for that shape.
7. **AC7:** On reconnect, client sends `ShapeSyncInit` and receives only the delta of records matching its shape since last sync.
8. **AC8:** Existing full-map sync (`SYNC_INIT`) continues to work unchanged (no regression).
9. **AC9:** Multiple shapes on the same map from different clients operate independently.
10. **AC10:** Connection disconnect cleans up all shape registrations and Merkle trees for that connection.

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all existing + new tests pass
2. Run `cargo test --release -p topgun-core` -- all existing + new tests pass
3. Run `pnpm test:integration-rust` -- shape integration tests pass
4. POST a `SHAPE_SUBSCRIBE` with filter predicate via WebSocket -- response contains only matching records
5. Write a record matching an active shape -- subscriber receives `SHAPE_UPDATE`

## Constraints

- Do NOT modify the existing full-map Merkle sync protocol (`SYNC_INIT` / `SYNC_RESP_ROOT` / etc.) -- shapes are additive
- Do NOT add DataFusion dependency -- shape evaluation uses the existing `PredicateNode` + `evaluate_predicate()` engine only
- Do NOT change the existing `QuerySubscribe` / `QueryResp` flow -- shapes are a separate subscription mechanism
- Shape evaluation MUST be synchronous (no async in the hot filter path)
- Wire messages MUST use `rmp_serde::to_vec_named()` with `#[serde(rename_all = "camelCase")]`
- Max 5 Rust files per sub-spec (Language Profile constraint)

## Assumptions

1. **Shape ID is client-assigned (UUID v4):** The client generates a unique shape ID. Server rejects duplicates.
2. **Shapes are LWW-Map only (not OR-Map) for the initial implementation:** OR-Map shape support can be added later. This reduces scope significantly.
3. **No server-side shape persistence:** Shapes are ephemeral -- they exist only while the WebSocket connection is open. On disconnect, shapes are cleaned up. On reconnect, the client re-subscribes.
4. **Shape limit is applied at subscribe time only:** The limit caps the initial response. Live updates are not capped (any matching mutation is forwarded). This matches PowerSync/ElectricSQL behavior.
5. **No nested field paths in filters:** Predicates operate on top-level fields only (matching existing `PredicateNode` capabilities).
6. **Shape Merkle trees use the same depth (3) as regular Merkle trees.**
7. **`Predicate` struct in `core-rust/src/schema.rs` can be removed** since `SyncShape.filter` will use `PredicateNode` directly. If external code depends on `Predicate`, it will be a simple alias.
8. **Field projection includes the record key implicitly** -- the key is always present in shape responses regardless of `fields` list.

## Goal-Backward Analysis

**Goal:** Clients receive and sync only the data subsets they need, reducing bandwidth, memory, and latency for mobile/constrained clients.

**Observable Truths -> Required Artifacts -> Wiring:**

| Truth | Artifacts | Wiring |
|-------|-----------|--------|
| T1: Client subscribes to shape | `shape.rs` messages, `classify.rs`, `operation.rs` | Message -> classify -> Operation::ShapeSubscribe -> ShapeService |
| T2: Server filters records | `shape_evaluator.rs`, `ShapeRegistry` | ShapeService calls evaluator on RecordStore records |
| T3: Mutations filtered by shape | `crdt.rs` modification, `ShapeRegistry` | CrdtService post-mutation (with old+new values) -> ShapeRegistry lookup -> evaluate -> ShapeUpdate |
| T4: Merkle delta sync per shape | `shape_merkle.rs`, sync.rs extension | ShapeSyncInit -> shape Merkle tree -> filtered leaf response via existing sync message structs |
| T5: Unsubscribe stops updates | `ShapeRegistry.unregister()` | ShapeService removes from registry, CrdtService no longer finds shape |
| T6: Multiple shapes per client | `ShapeRegistry` DashMap | Registry keyed by shape_id, not connection_id |
| T7: Field projection | `shape_evaluator.rs` `project()` | Applied in ShapeService (subscribe) and CrdtService (broadcast) |

---

## Audit History

### Audit v1 (2026-03-21)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (12+ files across 4 packages -- parent spec must split)

**Critical:**

1. **Undefined `ShapeRecord` type in R2.** `ShapeRespPayload` references `records: Vec<ShapeRecord>` but `ShapeRecord` is never defined anywhere in the spec. Define it explicitly (e.g., `ShapeRecord { key: String, value: rmpv::Value }`).

2. **Contradictory Merkle sync message strategy in R2/R8.** R2 adds new `Message` variants `ShapeSyncRespRoot`, `ShapeSyncRespBuckets`, `ShapeSyncRespLeaf` but defines no payload structs for them. R8 says "reuse existing Merkle protocol message structure with shape-prefixed paths." If reusing existing structs, remove the new variants. If adding new variants, define the payload structs. Pick one approach and make it consistent.

3. **R7 ENTER vs UPDATE vs LEAVE detection is underspecified.** To distinguish ENTER (newly matches) from UPDATE (already matched, value changed) and LEAVE (no longer matches), the server must know the previous match state of each record per shape. The spec does not specify how this state is tracked. Options: (a) evaluate shape against both old and new values (requires CrdtService to pass both), (b) maintain a `HashSet<String>` of matching keys per shape in ShapeRegistry. Specify which approach to use and where old values come from.

4. **Missing `Default` derive on `SyncShape`.** Per PROJECT.md Rust Type Mapping Rules, structs with 2+ optional fields must derive `Default`. `SyncShape` has 3 optional fields (`filter`, `fields`, `limit`). Add `Default` to R1.

5. **R3 incorrectly references `evaluate_where()`.** Since `SyncShape.filter` is `Option<PredicateNode>`, only `evaluate_predicate()` is needed. `evaluate_where()` operates on `HashMap<String, rmpv::Value>` where-clauses, which shapes do not use. Remove the reference to avoid implementer confusion.

**Recommendations:**

6. [Strategic] Consider deferring per-shape Merkle trees (R6, R8, SPEC-136d) from the initial implementation. Since shapes are ephemeral (Assumption 3: no persistence, cleaned up on disconnect), the client must re-subscribe all shapes on reconnect anyway. A simpler V1 could re-send the full matching dataset on reconnect and add Merkle delta optimization later. This eliminates an entire sub-spec (136d), 3 undefined message payloads (issue 2), and reduces complexity by ~20%.

7. [Compliance] `SyncShape.limit` type changes from `Option<usize>` to `Option<u32>`. This is correct per Rust Type Mapping Rules but is a breaking change to the existing type. Note this explicitly in R1.

8. R4 `ActiveShape` contains `map_name: String` but `SyncShape` already has `map_name`. This is redundant. Either remove `map_name` from `ActiveShape` or document why the duplication is intentional.

9. R2 should explicitly list `#[serde(skip_serializing_if = "Option::is_none", default)]` on all `Option<T>` fields in payload structs, per PROJECT.md auditor checklist. The current "proper `Option` decorators" wording in R1 is too vague.

10. Consider adding `Default` derive on `ShapeSubscribePayload` since it has 3 optional fields (`filter`, `fields`, `limit`).

11. R2 message wrapper naming should explicitly follow codebase convention: `ShapeSyncInitMessage` (not just `ShapeSyncInit`) for wrapper structs, matching the `SyncInitMessage` pattern used elsewhere.

### Response v1 (2026-03-21)
**Applied:** all critical issues and all recommendations

**Changes:**
1. [x] Undefined `ShapeRecord` type -- added `ShapeRecord { key: String, value: rmpv::Value }` as explicit struct in R2
2. [x] Contradictory Merkle sync strategy -- resolved in favor of reusing existing protocol structs; removed `ShapeSyncRespRoot/Buckets/Leaf` variants from `Message` enum; shape-prefixed paths documented in R2 and R8
3. [x] R7 ENTER/UPDATE/LEAVE underspecified -- R7 now specifies option (a): CrdtService reads old value from RecordStore before mutation, evaluates both old and new against shape filter, and uses the 4-case matrix to determine change type
4. [x] Missing `Default` derive on `SyncShape` -- added `#[derive(Default)]` and rationale in R1
5. [x] R3 `evaluate_where()` reference removed -- R3 now only references `evaluate_predicate()`; `evaluate_where()` reference removed from Context/Dependencies section too
6. [x] Defer per-shape Merkle trees -- not applied; Merkle delta sync is an Observable Truth (T4) and is present in AC7; deferral would require removing a committed goal
7. [x] `SyncShape.limit` breaking change noted explicitly in R1
8. [x] `ActiveShape.map_name` removed -- R4 now omits `map_name` from `ActiveShape` with a note to access via `active_shape.shape.map_name`
9. [x] Explicit serde attributes listed in R2 -- all payload structs now state `#[serde(skip_serializing_if = "Option::is_none", default)]` on `Option<T>` fields
10. [x] `ShapeSubscribePayload` derives `Default` -- added in R2
11. [x] Message wrapper naming updated -- all wrapper structs now use `*Message` suffix (e.g., `ShapeSyncInitMessage`); `Message` enum variants updated accordingly

**Skipped:** Item 6 (defer Merkle trees) -- the goal statement, Observable Truths, and AC7 all commit to shape-aware Merkle delta sync. Deferral is a product decision beyond the revision scope.

### Audit v2 (2026-03-21)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (12+ files across 4 packages)

**Scope:** Large (~100% estimated, exceeds 50% target). The spec already includes a well-structured Recommended Split section with 5 sub-specs (136a-136e) that respect the Rust Language Profile's 5-file limit.

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 (136a) | ~20% | ok |
| G2 (136b) | ~25% | ok |
| G3 (136c) | ~25% | ok |
| G4 (136d) | ~20% | ok |
| G5 (136e) | ~10% | ok |

**Quality Projection:** Each sub-spec individually falls in the GOOD range (20-25% context). The split plan is sound.

**Recommendations:**

1. **Missing `Operation::ShapeSyncInit` variant.** R8 says "Extend `SyncService` to handle `Operation::ShapeSyncInit`" but the Required Artifacts table (line 48) and SPEC-136a scope only list `ShapeSubscribe` and `ShapeUnsubscribe` as new Operation variants. Add `ShapeSyncInit` to the Operation enum and to `classify.rs` mapping. This should be included in SPEC-136a (Wave 1 types) since it is an Operation variant, or in SPEC-136d (Wave 3) alongside the sync extension. Either way, specify which sub-spec owns it.

2. **R3 title says "trait" but describes free functions.** The `ShapeEvaluator` is described as three standalone functions (`matches`, `project`, `apply_shape`), not as a trait with implementors. Either rename R3 to "ShapeEvaluator module" or define an actual trait. Free functions are simpler and appropriate here -- just fix the title to avoid confusion for the implementer.

3. **`Predicate` removal may break `core-rust/src/lib.rs` re-exports.** The public API at `core-rust/src/lib.rs:32` re-exports `Predicate` from `schema.rs`. Sub-spec 136a should update this re-export line when removing `Predicate`. Consider noting this in the SPEC-136a file list.

4. **`shapes_for_map` in R4 requires linear scan of DashMap.** With a `DashMap<String, ActiveShape>` keyed by `shape_id`, `shapes_for_map(map_name)` must iterate all entries to find shapes for a given map. For small shape counts this is fine, but consider adding a secondary index (`DashMap<String, Vec<String>>` mapping map_name to shape_ids) if shape counts could grow large. This is an optimization that can be deferred.

5. [Strategic] **`ShapeRespPayload.merkle_root_hash` is always present (not `Option`).** On initial subscribe before any Merkle tree exists, the value would be 0 (empty tree root). This is consistent with existing sync patterns (`SyncRespRootPayload.root_hash: u32`). No issue, but worth noting for implementers that 0 means "empty tree."

**Recommendation:** Use `/sf:split` to decompose into SPEC-136a through SPEC-136e, then `/sf:run --parallel` for execution.

**Comment:** The spec is well-crafted after the v1 revision. All previous critical issues have been resolved. The recommended split is logical and respects the Language Profile constraints. Requirements are detailed enough for implementation. No critical issues remain -- the spec is ready for decomposition.
