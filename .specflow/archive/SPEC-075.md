# SPEC-075: Wire QueryObserverFactory for Live Query Updates

```yaml
id: SPEC-075
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-03-03
todo: TODO-109
```

## Context

`QueryMutationObserver` is fully implemented in `packages/server-rust/src/service/domain/query.rs` and has comprehensive unit tests covering ENTER, UPDATE, LEAVE, and edge cases. However, it is only instantiated in `#[cfg(test)]` unit tests. The test server binary (`test_server.rs`) does not wire it as an observer factory, so no `QUERY_UPDATE` messages are sent when data changes via `CrdtService`.

The `ObserverFactory` trait (from SPEC-073e) already exists in `factory.rs`. `SearchObserverFactory` is already wired in `test_server.rs` as a working reference pattern. The fix is to create an analogous `QueryObserverFactory` that shares `QueryRegistry` and `ConnectionRegistry` with `QueryService`, and register it alongside `SearchObserverFactory` in `RecordStoreFactory::with_observer_factories()`.

**Root cause:** `CrdtService` writes to `RecordStore` which fires `MutationObserver` callbacks. Without a `QueryMutationObserver` wired as an observer on each store, the `on_put`/`on_update`/`on_remove` callbacks never reach the query subscription evaluation logic.

**Dependency:** SPEC-074 (store caching) must be complete first -- observers persist with cached stores, and `QueryMutationObserver` relies on the `QueryRegistry` being shared. SPEC-074 is already completed and archived.

## Task

Add a `QueryObserverFactory` struct to `test_server.rs` implementing `ObserverFactory`, and wire it in `build_services()` so that every `RecordStore` created by the factory receives a `QueryMutationObserver` instance connected to the shared `QueryRegistry` and `ConnectionRegistry`.

## Requirements

### Files to Modify

1. **`packages/server-rust/src/bin/test_server.rs`**
   - Add `QueryObserverFactory` struct with fields: `query_registry: Arc<QueryRegistry>`, `connection_registry: Arc<ConnectionRegistry>`
   - Implement `ObserverFactory` for `QueryObserverFactory`: `create_observer()` returns `Some(Arc::new(QueryMutationObserver::new(registry, connection_registry, map_name, partition_id)))` for every map
   - In `build_services()`, instantiate `QueryObserverFactory` sharing the same `Arc<QueryRegistry>` used by `QueryService`
   - Pass both `search_observer_factory` and `query_observer_factory` to `RecordStoreFactory::with_observer_factories(vec![search_observer_factory, query_observer_factory])`
   - **Ordering change:** Move `query_registry` creation (currently line 212) to BEFORE the `RecordStoreFactory` construction (line 174), since `QueryObserverFactory` must be passed to `with_observer_factories()` at factory creation time
   - **Arc sharing change:** Change line 216 from moving `query_registry` into `QueryService::new(query_registry, ...)` to cloning: `QueryService::new(Arc::clone(&query_registry), ...)`. This retains a reference for `QueryObserverFactory` and avoids a use-after-move compiler error.

### New Imports Required

In `test_server.rs`, add:
```rust
use topgun_server::service::domain::query::QueryMutationObserver;
```

### No New Files

No new files are created. The `QueryObserverFactory` lives in `test_server.rs` (same pattern as `SearchObserverFactory`).

## Acceptance Criteria

1. **AC1:** `QueryObserverFactory` struct exists in `test_server.rs` and implements `ObserverFactory` trait
2. **AC2:** `QueryObserverFactory::create_observer()` returns `Some(Arc<QueryMutationObserver>)` for every `(map_name, partition_id)` pair
3. **AC3:** The `QueryObserverFactory` shares the same `Arc<QueryRegistry>` instance as the `QueryService` registered in the `OperationRouter`
4. **AC4:** The `QueryObserverFactory` shares the same `Arc<ConnectionRegistry>` instance used by all services
5. **AC5:** `cargo build --bin test-server --release` compiles without warnings or errors
6. **AC6:** Integration test `queries.test.ts` line 516 (ENTER) passes -- new record triggers `QUERY_UPDATE` with `changeType: "ENTER"`
7. **AC7:** Integration test `queries.test.ts` line 593 (UPDATE) passes -- modified record triggers `QUERY_UPDATE` with `changeType: "UPDATE"`
8. **AC8:** Integration test `queries.test.ts` line 685 (LEAVE) passes -- record no longer matching filter triggers `QUERY_UPDATE` with `changeType: "LEAVE"`
9. **AC9:** Integration test `queries.test.ts` line 780 (UNSUB) passes -- after `QUERY_UNSUB`, no more `QUERY_UPDATE` messages arrive
10. **AC10:** Integration test `queries.test.ts` line 879 (multi-client) passes -- subscriber receives updates from multiple writers
11. **AC11:** Integration test `queries.test.ts` line 975 (multi-query) passes -- two queries with different `where` filters both receive correct updates
12. **AC12:** All previously passing integration tests (44/50) continue to pass (no regression)

## Constraints

- Do NOT move `QueryObserverFactory` into the library crate (`src/`). It belongs in the test binary, same as `SearchObserverFactory`.
- Do NOT modify `QueryMutationObserver` or `QueryService` -- they are already correct.
- Do NOT modify `factory.rs` -- the `ObserverFactory` trait and `with_observer_factories()` builder are already correct.
- Do NOT change the `MutationObserver` trait.

## Assumptions

- `QueryMutationObserver::create_observer()` returns `Some(...)` for every map (not filtered by map name), matching `SearchObserverFactory` behavior. All maps can have query subscriptions.
- The existing 431 core-rust tests and 494 server-rust tests continue to pass unchanged.
- The `QueryMutationObserver` implementation (evaluate_change, send_update, matches_query) is correct as written -- the 7 unit tests in `query.rs` validate this.

## Audit History

### Audit v1 (2026-03-03)
**Status:** APPROVED

**Context Estimate:** ~14% total

**Dimensions:**
- Clarity: PASS -- title, context, root cause, and task are all concrete and specific
- Completeness: PASS -- single file modification fully described with struct fields, trait impl, constructor args, and wiring location
- Testability: PASS -- 12 acceptance criteria, all measurable (5 structural + 6 integration tests + 1 regression)
- Scope: PASS -- complexity "small" is accurate for ~20 lines of new code in 1 file
- Feasibility: PASS -- all constructor signatures verified against source; import path confirmed valid
- Architecture fit: PASS -- follows established SearchObserverFactory pattern exactly
- Non-duplication: PASS -- wiring new functionality, not reimplementing
- Cognitive load: PASS -- structurally identical to existing SearchObserverFactory
- Strategic fit: PASS -- fixes 6 failing integration tests (44/50 to 50/50), proportional effort
- Project compliance: PASS -- honors all PROJECT.md decisions and constraints
- Language profile: PASS -- 1 file modified (max 5), no trait-first ordering needed for small complexity

**Recommendations:**
1. The "Files to Modify" section should explicitly note that line 216 of `test_server.rs` currently **moves** the `query_registry` Arc into `QueryService::new()`. To share it with `QueryObserverFactory` (AC3), the implementer must change this to `Arc::clone(&query_registry)`. The Rust compiler will catch this (use-after-move error), but making it explicit prevents a moment of confusion. Additionally, the `QueryObserverFactory` must be instantiated **before** `record_store_factory` is created (since it needs to be passed to `with_observer_factories`), which means `query_registry` must be created earlier in `build_services()` -- before the `RecordStoreFactory` construction on line 174, not on line 212 where it currently lives.

**Comment:** Excellent spec -- clean, focused, well-scoped bugfix with a clear reference pattern (SearchObserverFactory). The 1-file scope with 12 concrete acceptance criteria makes this highly implementable.

### Response v1 (2026-03-03)
**Applied:** Recommendation 1 from Audit v1

**Changes:**
1. [v] Recommendation 1 (ordering + Arc sharing) -- Added two explicit sub-bullets to "Files to Modify" section: (a) "Ordering change" noting that `query_registry` creation must move before `RecordStoreFactory` construction, (b) "Arc sharing change" noting that `QueryService::new()` must use `Arc::clone(&query_registry)` instead of moving ownership

### Audit v2 (2026-03-03)
**Status:** APPROVED

**Context Estimate:** ~12% total

**Dimensions:**
- Clarity: PASS -- root cause, task, and wiring instructions are concrete and unambiguous
- Completeness: PASS -- struct fields, trait impl, import path, ordering change, and Arc sharing all explicitly documented
- Testability: PASS -- 12 acceptance criteria (5 structural, 6 integration, 1 regression), all measurable
- Scope: PASS -- ~25 lines of new code in 1 file; "small" complexity is accurate
- Feasibility: PASS -- all constructor signatures verified against source (`QueryMutationObserver::new`, `QueryService::new`, `with_observer_factories`); import path `topgun_server::service::domain::query::QueryMutationObserver` confirmed valid
- Architecture fit: PASS -- identical pattern to `SearchObserverFactory` (lines 104-125 of test_server.rs)
- Non-duplication: PASS -- pure wiring, no reimplementation
- Cognitive load: PASS -- copy-paste-adapt from SearchObserverFactory; two fewer fields (no indexes HashMap)
- Strategic fit: PASS -- fixes 6 failing integration tests (44/50 -> 50/50), minimal effort for high value
- Project compliance: PASS -- 1 file in server-rust (max 5), no new dependencies, no out-of-scope items, honors all PROJECT.md constraints
- Language profile: PASS -- 1 file modified (max 5), small complexity (no trait-first ordering needed)

**Revision check:** Recommendation 1 from Audit v1 was correctly applied. The "Ordering change" and "Arc sharing change" sub-bullets are now explicit in the "Files to Modify" section, preventing implementer confusion around variable declaration order and move semantics.

**Comment:** Spec is ready for implementation. Clean, focused, well-scoped bugfix with a proven reference pattern. No remaining issues.

---

## Completion

**Completed:** 2026-03-03
**Total Commits:** 1
**Audit Cycles:** 2
**Result:** All 12 acceptance criteria met. Integration tests: 50/50 passing (up from 44/50). No regressions.
