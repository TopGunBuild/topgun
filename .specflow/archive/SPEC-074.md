---
id: SPEC-074
type: bugfix
status: done
priority: P1
complexity: medium
created: 2026-03-02
depends_on: [SPEC-073e]
todo_ref: TODO-108
---

# Fix RecordStoreFactory Ephemeral Stores, Partition Mismatch, and AUTH_FAIL Race

## Context

19 integration tests fail across 3 test suites (connection-auth, crdt-lww, queries) when running against the Rust server. All failures are pre-existing and unrelated to the SPEC-073e search tests (which pass 10/10). Root cause analysis identified 3 distinct bugs plus 1 architectural gap (QueryMutationObserver not wired — addressed separately).

**Bug 1 (FATAL): Ephemeral stores cause data loss.** `RecordStoreFactory::create()` returns a brand-new empty `HashMapStorage` on every call. There is no caching. CrdtService writes into store A (immediately dropped after the handler returns), then QueryService calls `create()` again and gets store B (empty). All data written by CrdtService is lost between operations.

**Bug 2 (SIGNIFICANT): Partition ID mismatch.** `ClientOp` gets `partition_id = hash_to_partition(key)` (range 0..270), but `QuerySub` gets `partition_id = None` which defaults to `0`. Even with store caching, queries would only scan partition 0 while data lives in partition N.

**Bug 3 (MODERATE): AUTH_FAIL + Close frame race.** After JWT failure, the auth handler sends AUTH_FAIL then immediately sends a Close frame via the same mpsc channel. The Close frame can terminate the WebSocket before the client processes AUTH_FAIL, causing `waitForMessage('AUTH_FAIL')` to time out.

**Bug 4 (ARCHITECTURAL — separate spec): QueryMutationObserver not wired.** `QueryMutationObserver` is defined in `query.rs` but only instantiated in `#[cfg(test)]` unit test blocks. The test server binary does not wire it as an observer factory, so no `QUERY_UPDATE` messages are sent when data changes. This affects 6 live query tests. This is a separate architectural concern and will be addressed in its own spec.

**Failure breakdown (total: 19 failing):**
- `connection-auth.test.ts`: 1/6 fail (Bug 3)
- `crdt-lww.test.ts`: 4/8 fail (Bugs 1+2)
- `queries.test.ts`: 14/16 fail — 8 snapshot tests fail (Bugs 1+2), 6 live update tests fail (Bug 4, out of scope)

**This spec fixes Bugs 1, 2, 3.** Bug 4 (QueryObserverFactory wiring) is tracked separately.

## Goal Analysis

**Goal Statement:** After this fix, writing data via `ClientOp` and reading it back via `QuerySub` returns the written data. Authentication failures are reported to the client before disconnection. Live query updates (ENTER/UPDATE/LEAVE) are out of scope — they require QueryObserverFactory wiring (separate spec).

**Observable Truths:**
1. A `ClientOp` PUT followed by a `QuerySub` on the same map returns the written record in `QueryResp.results`
2. Multiple `ClientOp` writes to different keys in the same map are all visible to a subsequent `QuerySub`
3. `QuerySub` with filter predicates correctly filters across all partitions
4. Invalid JWT auth produces an `AUTH_FAIL` message the client can observe before disconnection
5. Existing passing tests (search 10/10, pubsub, crdt-ormap, connection-auth 5/6, crdt-lww 4/8, queries 2/16) continue to pass

**Required Artifacts:**
- `factory.rs`: DashMap-based store cache, `get_or_create()`, `get_all_for_map()`
- `crdt.rs`: Uses `get_or_create()` instead of `create()`
- `query.rs`: Uses `get_all_for_map()` to scan all partitions
- `sync.rs`: Uses `get_or_create()` instead of `create()`
- `auth.rs`: Removes Close frame send (only sends AUTH_FAIL, lets caller handle disconnect)

**Key Links (fragile):**
- CrdtService write partition ID MUST match QueryService read partitions (Bug 2 fix)
- `DashMap` cache key `(map_name, partition_id)` MUST use `Arc<dyn RecordStore>` (not `Box`) for shared ownership between cache and callers
- Observer factories MUST still fire on first `get_or_create()` for a new `(map_name, partition_id)` pair, preserving search indexing behavior
- `for_each_boxed()` aggregation across partitions does NOT need deduplication — keys are deterministically mapped to exactly one partition via `hash_to_partition`, so the same key cannot exist in multiple partitions

## Task

Fix Bugs 1, 2, and 3 so that the 13 fixable integration tests pass (8 snapshot query + 4 crdt-lww + 1 auth) while maintaining all currently-passing tests. The 6 live query update tests remain failing until QueryObserverFactory is wired (separate spec).

## Requirements

### File 1: `packages/server-rust/src/storage/factory.rs`

**Changes:**
1. Add `DashMap<(String, u32), Arc<dyn RecordStore>>` field named `store_cache` to `RecordStoreFactory`
2. Rename `create()` to `get_or_create()`:
   - Check `store_cache` for existing `(map_name.to_string(), partition_id)` entry
   - If found, return `Arc::clone()` of the cached store
   - If not found, create new `DefaultRecordStore` (with observer factories), wrap in `Arc`, insert into cache, return clone
   - Return type changes from `Box<dyn RecordStore>` to `Arc<dyn RecordStore>`
3. Add `get_all_for_map(&self, map_name: &str) -> Vec<Arc<dyn RecordStore>>`:
   - Iterate `store_cache` entries where key.0 == `map_name`
   - Return all matching `Arc<dyn RecordStore>` values
   - Return empty vec if no stores exist for the map
   - Note: This is O(N) over the entire cache. Acceptable at current scale; consider a secondary index (`DashMap<String, Vec<u32>>`) if map count grows large
4. Remove the old `create()` method entirely — all callers (CrdtService, QueryService, SyncService) are updated in this spec. Update all unit tests to use `get_or_create()` directly
5. Update existing unit tests:
   - `factory_creates_independent_stores` test must be updated since same `(map_name, partition_id)` now returns the same store
   - Add new test verifying cache hit returns same store instance
   - Add new test for `get_all_for_map`

**Return type change:** `Box<dyn RecordStore>` -> `Arc<dyn RecordStore>`. This is the critical API change. All callers of `create()` currently use `Box<dyn RecordStore>` which does not support shared ownership. `Arc<dyn RecordStore>` enables the cache to retain a reference while callers also hold one.

### File 2: `packages/server-rust/src/service/domain/crdt.rs`

**Changes:**
1. Replace `self.record_store_factory.create(&op.map_name, partition_id)` (line 263) with `self.record_store_factory.get_or_create(&op.map_name, partition_id)`
2. Update `store` binding type from `Box<dyn RecordStore>` to `Arc<dyn RecordStore>` (both types implement `Deref<Target = dyn RecordStore>` so method calls remain identical)

### File 3: `packages/server-rust/src/service/domain/query.rs`

**Changes:**
1. In `handle_query_subscribe()` (line ~460), replace:
   ```rust
   let store = self.record_store_factory.create(&map_name, partition_id);
   ```
   with:
   ```rust
   let stores = self.record_store_factory.get_all_for_map(&map_name);
   ```
2. Iterate ALL stores, collecting entries from each:
   ```rust
   let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
   for store in &stores {
       store.for_each_boxed(
           &mut |key, record| {
               if let RecordValue::Lww { ref value, .. } = record.value {
                   entries.push((key.to_string(), value_to_rmpv(value)));
               }
           },
           false,
       );
   }
   ```
3. If `stores` is empty (no data written yet for this map), return empty results (current behavior preserved).

### File 4: `packages/server-rust/src/service/domain/sync.rs`

**Changes:**
1. Replace all 4 `self.record_store_factory.create(...)` calls (lines 174, 281, 354, 433) with `self.record_store_factory.get_or_create(...)`
2. Update `store` binding types from `Box<dyn RecordStore>` to `Arc<dyn RecordStore>`

### File 5: `packages/server-rust/src/network/handlers/auth.rs`

**Changes:**
1. In the `Err(e)` branch of `handle_auth()` (lines 121-135), remove the Close frame send:
   ```rust
   // REMOVE these lines:
   tx.send(OutboundMessage::Close(Some(
       "authentication failed".to_string(),
   )))
   .await?;
   ```
2. The caller (`ws_handler`) already handles the `Err(AuthError::InvalidToken)` return by breaking out of the message loop, which drops the connection. This matches the TS server behavior where AUTH_FAIL is sent and the client is expected to disconnect.

## Acceptance Criteria

1. **AC1:** `RecordStoreFactory::get_or_create("users", 5)` called twice returns the same `Arc` instance (pointer equality via `Arc::ptr_eq`)
2. **AC2:** `RecordStoreFactory::get_all_for_map("users")` returns all stores created for map "users" across different partition IDs
3. **AC3:** Data written via CrdtService `ClientOp` is visible to QueryService `QuerySub` on the same map (the 4 failing crdt-lww tests pass)
4. **AC4:** `QuerySub` snapshot tests aggregate results from all partitions, not just partition 0 (the 8 failing snapshot query tests pass: QUERY_RESP all records, where filter, gt/lt/gte/lte/neq predicates, sort asc/desc, limit). Note: 6 live update tests (ENTER/UPDATE/LEAVE/UNSUB/multi-client/multi-query) remain failing — they require QueryObserverFactory wiring (separate spec)
5. **AC5:** Invalid JWT auth sends AUTH_FAIL that is received by the client before disconnection (the 1 failing connection-auth test passes)
6. **AC6:** All currently-passing integration tests continue to pass (search 10/10, pubsub, crdt-ormap, connection-auth 5/6, crdt-lww 4/8, queries 2/16)
7. **AC7:** All existing Rust unit tests pass (`cargo test --release -p topgun-server`)
8. **AC8:** Observer factories still fire on first store creation for each `(map_name, partition_id)` pair (search indexing unbroken)

## Constraints

- Do NOT modify any TypeScript integration test files -- the tests define correct behavior
- Do NOT change the `RecordStore` trait behavior -- only the factory's return type changes. Doc comment update (`Box` -> `Arc`) is acceptable
- Do NOT add `QueryObserverFactory` wiring in this spec -- it is a separate architectural task requiring its own spec (see TODO-109)
- Do NOT change partition assignment in `classify.rs` -- the partition hashing is correct; QueryService must scan all partitions
- Keep `RecordStoreFactory` as a concrete struct (not a trait) -- it is not a seam point
- The `DashMap` cache must be `Send + Sync` (it is by default)

## Assumptions

- `Arc<dyn RecordStore>` supports all the same method calls as `Box<dyn RecordStore>` since both deref to `dyn RecordStore`. Verified: `async_trait` methods on `RecordStore` are called via `&self`, which works with both `Arc` and `Box`.
- The `factory_creates_independent_stores` unit test in `factory.rs` uses different `(map_name, partition_id)` pairs ("map-a"/0 and "map-b"/1), so it will still test independence after caching is added. If it uses the same pair, the test assertion needs updating.
- Removing the Close frame from auth.rs is safe because the WebSocket connection handler's message loop already breaks on `Err(AuthError::InvalidToken)`, which drops the sender half of the mpsc channel, causing the writer task to shut down and close the socket.
- `SyncService` calls `factory.create()` with correct partition IDs from the classified operation context, so the `get_or_create()` rename is a drop-in replacement.
- The `#[allow(dead_code)]` `record_store_factory` in SearchService does not need updating since it never calls `create()`.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `factory.rs`: Add `DashMap` cache field, implement `get_or_create()` and `get_all_for_map()`, update return type to `Arc<dyn RecordStore>`, update unit tests | -- | ~15% |
| G2 | 2 | `crdt.rs` + `sync.rs`: Replace all `factory.create()` calls with `factory.get_or_create()`, update binding types from `Box` to `Arc` | G1 | ~15% |
| G3 | 2 | `query.rs`: Replace single-partition `create()` with `get_all_for_map()` multi-partition scan, aggregate entries | G1 | ~12% |
| G4 | 2 | `auth.rs`: Remove Close frame send after AUTH_FAIL | -- | ~5% |
| G5 | 3 | Run full integration test suite, verify 13 fixable tests now pass (8 snapshot query + 4 crdt-lww + 1 auth), verify 6 live query tests still fail (expected — no QueryObserverFactory), verify no regressions | G2, G3, G4 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4 | Yes | 3 |
| 3 | G5 | No | 1 |

**Total workers needed:** 3 (max in any wave)

**Note:** G1 is not purely types/traits (it modifies a concrete struct, not a trait), but it defines the new API surface that G2 and G3 depend on. The `RecordStore` trait itself is unchanged. This is a bugfix spec where the trait-first pattern applies to the API change in G1 rather than a new trait definition.

## Audit History

### Audit v1 (2026-03-02)
**Status:** APPROVED

**Context Estimate:** ~52% total (with worker overhead)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~52% | <=50% | -- |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <- Current estimate |
| 70%+ | POOR | - |

Note: The 52% total is across ALL groups including the test verification pass. With parallel execution (3 workers in Wave 2), no single worker exceeds ~20%. The original spec estimated ~100% total which was inflated; corrected estimates are used above.

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (PUT then QuerySub) has artifacts | OK | factory.rs cache + query.rs multi-partition |
| Truth 2 (multiple keys visible) has artifacts | OK | factory.rs cache |
| Truth 3 (filter across partitions) has artifacts | OK | query.rs get_all_for_map |
| Truth 4 (live query updates) has artifacts | OK | observers persist with cached stores |
| Truth 5 (AUTH_FAIL observable) has artifacts | OK | auth.rs Close frame removal |
| Truth 6 (no regressions) has artifacts | OK | G5 integration test verification |
| All artifacts have purpose | OK | No orphan artifacts |
| CrdtService->QueryService wiring | OK | Key link identified |
| Observer factory first-creation | OK | Key link identified |

**Assumptions Validated Against Source Code:**

| # | Assumption | Verified |
|---|------------|----------|
| A1 | `Arc<dyn RecordStore>` supports all `&self` trait methods | YES - RecordStore trait uses `&self` throughout |
| A2 | `factory_creates_independent_stores` uses different pairs | YES - ("map-a", 0) and ("map-b", 1) confirmed at lines 184-185 |
| A3 | ws_handler breaks on AuthError::InvalidToken | YES - confirmed `break` at websocket.rs line 152 |
| A4 | SyncService uses correct partition_ids from ctx | YES - all 4 calls use `ctx.partition_id.unwrap_or(0)` |
| A5 | SearchService never calls `create()` | YES - `#[allow(dead_code)]` on field, no call sites |

**Project Compliance:** OK - Honors PROJECT.md decisions. No new dependencies (DashMap already used). No trait changes. MsgPack wire format unchanged.

**Strategic Fit:** OK - Aligned with project goals. Fixing 18 integration test failures is high-value P1 work directly on the critical path for Rust migration validation.

**Language Profile:** OK - 5 files (at limit). Trait-first deviation justified (bugfix modifying concrete struct, not adding new trait). No compilation gate in PROJECT.md to check.

**Rust Auditor Checklist:** OK - No new structs, no new message types, no new serialization. All changes are to method signatures and call sites.

**Recommendations:**
1. Requirement 4 in File 1 is ambiguous: "Keep `create()` as deprecated wrapper... or remove if all callers updated." Since all callers ARE updated in this spec, recommend deciding explicitly: either always remove `create()` (cleaner) or always keep it deprecated (safer for external callers). Suggest: remove it and update all unit tests to use `get_or_create()`.
2. Key Links section mentions `for_each_boxed()` "MUST deduplicate" across partitions, but the implementation code in Requirements does not include deduplication. This is correct (keys are deterministically mapped to exactly one partition via `hash_to_partition`), but the Key Links text is misleading. Recommend removing the deduplication requirement from Key Links or adding a note that it is unnecessary due to deterministic partitioning.
3. `get_all_for_map()` iterates the entire DashMap with a filter. For production use with many maps/partitions, consider a secondary index (`DashMap<String, Vec<u32>>`). Acceptable for current scale but worth noting for future optimization.

**Comment:** Exceptionally well-analyzed spec. All three bugs are precisely identified with root cause analysis. Line references, code snippets, and assumptions all verified against source code. The fix approach is minimal and surgical. The only issues found are minor documentation inconsistencies.

### Response v1 (2026-03-02)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] Removed ambiguous `create()` deprecated/remove option — now explicitly states: remove `create()` entirely, update all unit tests to use `get_or_create()`
2. [✓] Fixed misleading deduplication claim in Key Links — replaced "MUST deduplicate" with explanation that deduplication is unnecessary due to deterministic `hash_to_partition` mapping
3. [✓] Added O(N) performance note to `get_all_for_map()` requirement — documents current acceptable scale and future secondary index optimization path

### Audit v2 (2026-03-02)
**Status:** NEEDS_REVISION

**Context Estimate:** ~52% total (with worker overhead)

**Critical:**
1. **AC4 is likely unsatisfiable: live query update tests require `QueryMutationObserver` wiring that does not exist.** The constraint says "Do NOT add a QueryObserverFactory -- live query updates already work via the CompositeMutationObserver pattern once stores are cached (observers persist with the store)." This claim is incorrect. Source code verification shows: (a) `QueryMutationObserver` is defined in `query.rs` but is ONLY instantiated in `#[cfg(test)]` blocks -- never in production or the test server binary. (b) `test_server.rs` creates `RecordStoreFactory::new(StorageConfig::default(), Arc::new(NullDataStore), Vec::new())` -- the `Vec::new()` means NO static mutation observers are registered. (c) Only `SearchObserverFactory` is wired via `.with_observer_factories()`. Without `QueryMutationObserver` wired as either a static observer or an observer factory, no `QUERY_UPDATE` messages will be sent when data changes. The live query integration tests (`queries.test.ts` lines 516, 593, 685, 780, 879, 975) explicitly wait for `QUERY_UPDATE` messages and will time out. This means AC4's claim that "the 13 failing queries tests pass" cannot be fully satisfied -- the ~6 live update tests will still fail. **Resolution options:** (a) Reduce AC4 scope to snapshot-only query tests and acknowledge live update tests remain failing (separate spec), or (b) Add `QueryObserverFactory` implementation + wiring in `test_server.rs` (adds 1 file to the spec, exceeding the 5-file Language Profile limit, so would require splitting), or (c) Wire `QueryMutationObserver` as a static observer in `test_server.rs` (but static observers lack per-map/partition context needed by `QueryMutationObserver::new()`).

**Recommendations:**
2. The `RecordStore` trait has a doc comment at line 75 of `record_store.rs` saying "Used as `Box<dyn RecordStore>`." After this change, callers will use `Arc<dyn RecordStore>`. While the constraint says "Do NOT change the `RecordStore` trait itself," updating a doc comment is not a behavioral change. Consider updating this comment to "Used as `Arc<dyn RecordStore>`" for accuracy, or clarify in the constraint that doc comment updates are acceptable.
3. The test count in `queries.test.ts` appears to be 16 (not 15 as stated in the failure breakdown). Verify the actual count and update the "13/15 fail" figure if needed. This does not affect the fix approach but the acceptance criteria should reference the correct numbers.

### Response v2 (2026-03-02)
**Applied:** Critical issue #1 + recommendations #2, #3 from Audit v2

**Changes:**
1. [✓] **Scoped down AC4 to snapshot-only query tests.** Live query updates (ENTER/UPDATE/LEAVE/UNSUB/multi-client/multi-query) explicitly excluded — they require QueryObserverFactory wiring which is a separate architectural task (TODO-109). Spec now fixes 13 tests (not 18): 8 snapshot queries + 4 crdt-lww + 1 auth.
2. [✓] **Fixed test counts.** `queries.test.ts` has 16 tests (not 15). Updated failure breakdown: 14/16 fail (8 snapshot from Bugs 1+2, 6 live from Bug 4). Total fixable by this spec: 13 tests.
3. [✓] **Removed incorrect constraint** about QueryObserverFactory not being needed. Replaced with explicit deferral to separate spec.
4. [✓] **Updated doc comment constraint** — clarified that updating `Box` -> `Arc` in RecordStore doc comment is acceptable (not a behavioral change).
5. [✓] **Removed Observable Truth #4** (live query updates) — out of scope for this spec.
6. [✓] **Updated Goal Statement** to explicitly state live updates are out of scope.

### Audit v3 (2026-03-02)
**Status:** APPROVED

**Context Estimate:** ~52% total (with worker overhead)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~52% | <=50% | -- |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <- Current estimate |
| 70%+ | POOR | - |

Note: The 52% total is distributed across 5 task groups with parallel execution. No single worker exceeds ~20% context. The DEGRADING range applies to a hypothetical sequential execution; with parallel workers the effective per-worker context is well within GOOD range.

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | factory.rs: DashMap cache, get_or_create(), get_all_for_map() | ~15% | 15% |
| G2 | 2 | crdt.rs + sync.rs: replace create() calls | ~15% | 30% |
| G3 | 2 | query.rs: multi-partition scan | ~12% | 42% |
| G4 | 2 | auth.rs: remove Close frame | ~5% | 47% |
| G5 | 3 | Integration test verification | ~5% | 52% |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (PUT then QuerySub) has artifacts | OK | factory.rs cache + query.rs multi-partition |
| Truth 2 (multiple keys visible) has artifacts | OK | factory.rs cache |
| Truth 3 (filter across partitions) has artifacts | OK | query.rs get_all_for_map |
| Truth 4 (AUTH_FAIL observable) has artifacts | OK | auth.rs Close frame removal |
| Truth 5 (no regressions) has artifacts | OK | G5 integration test verification |
| All artifacts have purpose | OK | No orphan artifacts |
| CrdtService->QueryService wiring | OK | Key link: partition ID alignment |
| Observer factory first-creation | OK | Key link: only fires on cache miss |

**Assumptions Validated Against Source Code:**

| # | Assumption | Verified |
|---|------------|----------|
| A1 | `Arc<dyn RecordStore>` supports all `&self` trait methods | YES - RecordStore trait uses `&self` throughout (record_store.rs lines 77-148) |
| A2 | `factory_creates_independent_stores` uses different pairs | YES - ("map-a", 0) and ("map-b", 1) confirmed at factory.rs lines 184-185 |
| A3 | ws_handler breaks on AuthError::InvalidToken | YES - confirmed `break` at websocket.rs line 152 |
| A4 | SyncService uses correct partition_ids from ctx | YES - all 4 calls use `ctx.partition_id.unwrap_or(0)` at sync.rs lines 174, 281, 354, 433 |
| A5 | SearchService never calls `create()` | YES - `#[allow(dead_code)]` on field, no call sites in search.rs |

**Strategic Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Store caching is safe (no need for store-per-request isolation) | Data corruption if stores need request-scoped state; mitigated by RecordStore being stateless beyond its data |
| A2 | Removing Close frame allows client to read AUTH_FAIL before disconnect | Client may hang if ws_handler break does not close socket fast enough; mitigated by mpsc drop causing write-task shutdown |
| A3 | 13 tests will pass with just Bugs 1-3 fixed | Some tests may have additional failure causes; mitigated by precise root-cause tracing in spec |

**Project Compliance:** OK - Honors PROJECT.md decisions. No new dependencies (DashMap already in Cargo.toml). No trait changes. MsgPack wire format unchanged. No out-of-scope items included.

**Strategic Fit:** OK - Aligned with project goals. Fixing 13 integration test failures is high-value P1 work directly on the critical path for Rust migration validation (v1.0 roadmap item).

**Language Profile:** OK - 5 files (at limit). Trait-first deviation justified (bugfix modifying concrete struct, not adding new trait).

**Rust Auditor Checklist:** OK - No new structs, no new message types, no new serialization. All changes are to method signatures and call sites.

**Recommendations:**
1. `crdt-lww.test.ts` has 7 tests, not 8. The Context section says "4/8 fail" and AC6 says "crdt-lww 4/8". The denominator should be 7. This does not affect the fix approach or acceptance criteria (4 tests to fix is correct), but the documentation is slightly inaccurate. Consider updating to "4/7" in a future revision.
2. In query.rs `handle_query_subscribe()`, after replacing `create()` with `get_all_for_map()`, the existing `partition_id` variable (line 454) becomes unused. The implementer should remove or comment it to avoid a compiler warning. This is implicit but worth noting for smooth implementation.

**Comment:** Spec is well-revised after two audit cycles. All critical issues from v1 and v2 have been properly addressed. The scope is now correctly bounded to 13 fixable tests (Bugs 1-3), with Bug 4 (live query updates) explicitly deferred. Root cause analysis is precise and verified against source code. The fix approach is minimal, surgical, and sound. Ready for implementation.

## Execution Summary

**Executed:** 2026-03-03
**Mode:** orchestrated
**Commits:** 6

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3, G4 | complete |
| 3 | G5 | complete |

### Files Modified
- `packages/server-rust/src/storage/factory.rs` -- DashMap cache, get_or_create(), get_all_for_map(), return type Arc
- `packages/server-rust/src/storage/record_store.rs` -- doc comment Box -> Arc
- `packages/server-rust/src/service/domain/crdt.rs` -- create() -> get_or_create(), fix rmpv_to_value Map key quoting
- `packages/server-rust/src/service/domain/sync.rs` -- create() -> get_or_create() (4 call sites)
- `packages/server-rust/src/service/domain/query.rs` -- single-partition create() -> get_all_for_map() multi-partition scan
- `packages/server-rust/src/network/handlers/auth.rs` -- remove Close frame, simplify JWT claims to sub only
- `packages/server-rust/src/network/handlers/websocket.rs` -- graceful outbound task drain instead of abort

### Acceptance Criteria Status
- [x] AC1: get_or_create() called twice returns same Arc instance (pointer equality)
- [x] AC2: get_all_for_map() returns all stores for map across partitions
- [x] AC3: Data written via CrdtService visible to QueryService (7/7 crdt-lww tests pass)
- [x] AC4: QuerySub snapshot tests aggregate across all partitions (10/16 queries pass; 6 live update tests expected to fail -- separate spec)
- [x] AC5: Invalid JWT sends AUTH_FAIL before disconnection (6/6 connection-auth tests pass)
- [x] AC6: All previously-passing tests continue to pass (search 10/10, pubsub 7/7, crdt-ormap 4/4)
- [x] AC7: All 502 Rust unit tests pass (498 unit + 4 metrics integration)
- [x] AC8: Observer factories fire only on first store creation per (map_name, partition_id) pair

### Deviations
1. **Additional fix: rmpv_to_value Map key quoting (crdt.rs).** `rmpv::Value::to_string()` wraps strings in quotes via its Display impl, causing Map keys to be stored as `"\"name\""` instead of `"name"`. Fixed by extracting raw string via `as_str()` for String variants. This bug was not identified in the spec but was discovered during integration testing and was necessary to pass the crdt-lww and queries tests.
2. **Additional fix: websocket.rs outbound task drain.** The outbound task was `abort()`ed immediately when the inbound loop exited, preventing AUTH_FAIL from being flushed to the WebSocket. Fixed by dropping the mpsc sender and awaiting the outbound task with a 2-second timeout. This was the actual root cause of the AUTH_FAIL race (not just the Close frame).
3. **auth.rs includes JWT sub claim simplification** from SPEC-073e that was in the working directory but not yet committed to the auth handler.
4. **File count: 7 files modified** (exceeds 5-file Language Profile limit). The additional 2 files (record_store.rs doc comment, websocket.rs drain fix) were necessary to fully resolve the bugs.

---

## Review History

### Review v1 (2026-03-03)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. `for_each_boxed` doc comment in `record_store.rs:147` still references `Box<dyn RecordStore>` compatibility. Now that the primary usage is `Arc<dyn RecordStore>`, this comment is slightly stale. Could be updated to reference both `Arc` and `Box` for completeness.
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server-rust/src/storage/record_store.rs:147`
2. Deviation 4 notes 7 files modified against a 5-file Language Profile limit. Both additional files were necessary (doc comment fix + WebSocket drain fix). Acceptable for a bugfix spec where root cause discovery during testing exposed additional required changes, but worth noting as a pattern to watch.
3. The spec's Context section still says "crdt-lww.test.ts: 4/8 fail" but the file contains only 7 tests. This was noted in Audit v3 recommendation #1 but never corrected in the Context section. Minor documentation inaccuracy only -- does not affect implementation correctness.

**Passed:**
- [v] **AC1: Cache hit returns same Arc instance.** Verified by `cache_hit_returns_same_arc_instance` unit test in `factory.rs:259-273` using `Arc::ptr_eq`. Also verified `different_partitions_return_different_stores` test for negative case.
- [v] **AC2: get_all_for_map returns all partitions.** Verified by `get_all_for_map_returns_all_partitions` unit test in `factory.rs:295-318` testing 3 partitions for "users", 1 for "orders", 0 for "nonexistent".
- [v] **AC3: CrdtService data visible to QueryService.** Confirmed: 7/7 crdt-lww integration tests pass (PUT+readback, OP_BATCH, LWW conflict, tombstone, HLC ordering, deterministic winner).
- [v] **AC4: QuerySub snapshot tests aggregate across all partitions.** Confirmed: 10/16 query tests pass (all 10 snapshot tests including QUERY_RESP all records, where filter, gt/lt/gte/lte/neq predicates, sort asc, sort desc, limit). 6 live update tests fail as expected (separate spec for QueryObserverFactory wiring).
- [v] **AC5: AUTH_FAIL received before disconnection.** Confirmed: 6/6 connection-auth tests pass (including the previously-failing "client sends invalid JWT, receives AUTH_FAIL" test). Root cause properly addressed in both auth.rs (Close frame removal) and websocket.rs (graceful outbound task drain).
- [v] **AC6: No regressions in previously-passing tests.** Confirmed: search 10/10, pubsub 7/7, crdt-ormap 4/4 all pass. Total: 44 passing, 6 failing (all 6 are expected live query failures).
- [v] **AC7: All Rust unit tests pass.** Confirmed: 502 tests pass (498 unit + 4 metrics integration), 0 failures, clippy-clean with `-D warnings`.
- [v] **AC8: Observer factories fire only on first creation.** Verified by `observer_factory_fires_only_on_cache_miss` unit test in `factory.rs:322-362` using `AtomicUsize` counter to confirm factory fires on first call and cache hit, fires again for different partition.
- [v] **Constraint: No TS test files modified.** Verified via `git diff` -- no changes to `tests/integration-rust/` directory.
- [v] **Constraint: No RecordStore trait behavioral changes.** Only doc comment updated (`Box` -> `Arc` at line 75).
- [v] **Constraint: No factory.create() lingering references.** `grep` for `factory.create(` and `record_store_factory.create(` returns zero results in `packages/server-rust/src/`.
- [v] **Code quality: DashMap TOCTOU protection.** `get_or_create()` uses fast-path `get()` then falls back to `entry().or_insert_with()` -- atomic check-and-insert prevents race conditions.
- [v] **Code quality: rmpv_to_value Map key fix (Deviation 1).** Correctly extracts raw string via `as_str()` instead of using `Display` impl which wraps strings in quotes. Necessary fix for data integrity.
- [v] **Code quality: WebSocket outbound task drain (Deviation 2).** Properly drops `handle` (releasing mpsc sender), then awaits outbound task with 2-second timeout. Correct pattern for ensuring AUTH_FAIL is flushed before socket close.
- [v] **Rust idioms: No unwrap() in production code.** All `.unwrap()` and `.expect()` calls are in `#[cfg(test)]` blocks only.
- [v] **Rust idioms: No unnecessary clones.** All `.clone()` calls in production code are on `Arc`, `Vec<Arc<_>>`, `StorageConfig`, or `DashMap` entry values -- all necessary and appropriate.
- [v] **Rust idioms: No unsafe blocks.** Zero `unsafe` usage in any modified file.
- [v] **Build check passed.** `cargo check --release -p topgun-server` succeeds.
- [v] **Lint check passed.** `cargo clippy -p topgun-server -- -D warnings` succeeds with zero warnings.
- [v] **Test check passed.** `cargo test --release -p topgun-server` -- 502 passed, 0 failed.

**Summary:** All 8 acceptance criteria are met. All constraints honored. The implementation is clean, well-documented, and follows established Rust patterns. The 4 deviations from spec are all justified: the rmpv_to_value Map key quoting fix and the WebSocket drain fix were genuinely necessary to fully resolve the bugs, as the spec's root cause analysis was incomplete on these two points. Code quality is high -- proper use of DashMap entry API for thread safety, no production unwrap/expect, comprehensive unit test coverage for the new factory methods. The total result is 44/50 integration tests passing (up from 31/50), with the remaining 6 failures correctly scoped to the separate QueryObserverFactory wiring spec.

---

## Completion

**Completed:** 2026-03-03
**Total Commits:** 6
**Audit Cycles:** 3
**Review Cycles:** 1
