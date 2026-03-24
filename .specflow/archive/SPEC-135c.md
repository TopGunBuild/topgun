---
id: SPEC-135c
type: feature
status: done
priority: P1
complexity: small
parent: SPEC-135
depends_on: [SPEC-135b]
created: 2026-03-20
source: TODO-091
---

# DataFusion SQL: QueryService Integration

## Context

SPEC-135a established the `QueryBackend` trait, `PredicateBackend`, wire messages, and the `datafusion` feature flag. SPEC-135b implemented the DataFusion engine: `ArrowCacheManager`, Arrow conversion, `TopGunTableProvider`, and `DataFusionBackend`.

This sub-spec wires everything together: injecting `QueryBackend` into `QueryService`, handling the new `SQL_QUERY` operation, updating `classify()` to route `Message::SqlQuery`, and conditionally registering `ArrowCacheObserverFactory` in `RecordStoreFactory` when the `datafusion` feature is enabled.

**File count note:** This spec touches 6 files, which exceeds the Language Profile limit of 5. Three of the six files (`lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`) require only trivial constructor call-site updates — each adds a single `Arc::new(PredicateBackend)` argument to the existing `QueryService::new()` call. The substantive changes are confined to `query.rs`, `operation.rs`, and `classify.rs`. Note that `lib.rs` contains **two** `QueryService::new()` call sites (one in the main assembly and one in a separate `register()` path) — both must be updated.

## Task

Modify `QueryService` to accept `Arc<dyn QueryBackend>` and (when the `datafusion` feature is enabled) `Arc<dyn SqlQueryBackend>` as dependencies, dispatch predicate queries through the backend, handle the new `SQL_QUERY` operation (feature-gated), update `classify()` to route `Message::SqlQuery` to `Operation::SqlQuery`, and wire `ArrowCacheObserverFactory` into `RecordStoreFactory` assembly when `datafusion` is enabled.

## Requirements

### R1: QueryService QueryBackend Injection

**File:** `packages/server-rust/src/service/domain/query.rs` (modify)

Modify `QueryService` to hold an `Arc<dyn QueryBackend>` and, when the `datafusion` feature is enabled, an `Option<Arc<dyn SqlQueryBackend>>`:

```rust
pub struct QueryService {
    query_registry: Arc<QueryRegistry>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    query_backend: Arc<dyn QueryBackend>,
    #[cfg(feature = "datafusion")]
    sql_query_backend: Option<Arc<dyn SqlQueryBackend>>,
}
```

Update the constructor with the same conditional parameter:
```rust
pub fn new(
    query_registry: Arc<QueryRegistry>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    query_backend: Arc<dyn QueryBackend>,
    #[cfg(feature = "datafusion")]
    sql_query_backend: Option<Arc<dyn SqlQueryBackend>>,
) -> Self
```

When the `datafusion` feature is disabled, call sites (`lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`) pass only `query_backend` — they do NOT pass `None` for `sql_query_backend`, because the parameter does not exist in non-datafusion builds. Call sites that enable `datafusion` pass both `query_backend` and `sql_query_backend`.

Modify `handle_query_subscribe()`:
- Make it `async fn handle_query_subscribe(...)`.
- Instead of directly calling `predicate::execute_query()`, collect entries from RecordStore and call `self.query_backend.execute_query(map_name, entries, &query).await`.
- Update the match arm in Tower `Service::call()` to `.await` the result:

```rust
Operation::QuerySubscribe { ctx, payload } => {
    svc.handle_query_subscribe(&ctx, &payload).await
}
```

`QueryMutationObserver` continues using `predicate::evaluate_predicate()` / `predicate::evaluate_where()` directly — standing query re-evaluation does NOT go through `QueryBackend`. This is intentional: standing queries need per-record eval on mutation, not full-table scan.

### R2: SQL_QUERY Operation Handling

**File:** `packages/server-rust/src/service/operation.rs` (modify)

Add a new `Operation` variant for SQL queries:

```rust
/// Client executes a SQL query.
SqlQuery {
    ctx: OperationContext,
    payload: topgun_core::messages::query::SqlQueryPayload,
},
```

Update the exhaustive match arms in `Operation::ctx()` and `Operation::set_connection_id()` to cover the new `SqlQuery` variant.

Also update the `operation_variant_count_covers_all_30_client_plus_1_system` test (near line 572): add `Operation::SqlQuery { .. }` to its exhaustive match, update the test name comment to "31 client + 1 system", and update the doc comment on line 572 from "all 31 variants" to "all 32 variants".

**File:** `packages/server-rust/src/service/classify.rs` (modify)

Update the `classify()` method on `OperationService`: change the `Message::SqlQuery` match arm from returning `ClassifyError::ServerToClient` to constructing `Operation::SqlQuery`. Use `self.make_ctx()` to build the context (matching the pattern used by all other classify arms):

```rust
Message::SqlQuery { payload } => {
    let ctx = self.make_ctx(
        service_names::QUERY,
        client_id,
        caller_origin,
        None,
    );
    Ok(Operation::SqlQuery { ctx, payload })
}
```

**File:** `packages/server-rust/src/service/domain/query.rs` (modify)

In `QueryService`'s Tower `Service::call()` implementation, add a match arm:

```rust
Operation::SqlQuery { ctx, payload } => {
    svc.handle_sql_query(&ctx, &payload).await
}
```

`handle_sql_query()` implementation:
1. Feature-gate the method body with `#[cfg(feature = "datafusion")]`.
2. Check `self.sql_query_backend`. If `None`, return `OperationError::Internal("SQL requires datafusion feature")`.
3. Call `sql_backend.execute_sql(&payload.sql).await`.
4. Convert `Vec<RecordBatch>` to `SqlQueryRespPayload`:
   - `columns`: extract column names from RecordBatch schema.
   - `rows`: iterate RecordBatch rows, convert each Arrow value to `rmpv::Value` using `record_batches_to_rows()`.
5. Return `OperationResponse::Message(Box::new(Message::SqlQueryResp { payload: SqlQueryRespPayload { columns, rows, error: None } }))`.

For the non-`datafusion` build: `handle_sql_query()` returns `OperationError::Internal("SQL requires datafusion feature")`.

### R3: RecordBatch-to-MsgPack Row Conversion

**File:** `packages/server-rust/src/service/domain/query.rs` (same file, behind `#[cfg(feature = "datafusion")]`)

Helper function:
```rust
#[cfg(feature = "datafusion")]
fn record_batches_to_rows(batches: &[arrow::array::RecordBatch]) -> (Vec<String>, Vec<Vec<rmpv::Value>>)
```

- Extracts column names from the first batch's schema (or returns empty if no batches).
- For each batch, for each row index, reads each column's value and converts to `rmpv::Value`:
  - `Int32Array` -> `rmpv::Value::Integer`
  - `Int64Array` -> `rmpv::Value::Integer`
  - `UInt32Array` -> `rmpv::Value::Integer`
  - `UInt64Array` -> `rmpv::Value::Integer`
  - `Float64Array` -> `rmpv::Value::F64`
  - `StringArray` / `LargeStringArray` -> `rmpv::Value::String`
  - `BooleanArray` -> `rmpv::Value::Boolean`
  - `BinaryArray` -> `rmpv::Value::Binary`
  - `TimestampMicrosecondArray` -> `rmpv::Value::Integer` (microseconds since epoch)
  - Null -> `rmpv::Value::Nil`
  - Other -> `rmpv::Value::String` (debug representation as fallback)
- Returns `(columns, rows)`.

### R4: Conditional ArrowCacheObserverFactory Registration

**File:** Server assembly code where `RecordStoreFactory` is constructed (`lib.rs`, `bin/test_server.rs`)

When building `RecordStoreFactory` during server assembly:
- If `datafusion` feature is enabled, create an `ArrowCacheManager`, wrap it in `Arc`, create an `ArrowCacheObserverFactory`, and include it in `with_observer_factories()`.
- If `datafusion` feature is disabled, no observer factory is registered.

This can be accomplished with a cfg-conditional block in the server assembly code (wherever `RecordStoreFactory::new(...).with_observer_factories(...)` is called).

**File:** `packages/server-rust/src/service/domain/query_backend.rs` (modify)

Provide convenience functions alongside the trait definitions in `query_backend.rs`:
```rust
#[cfg(feature = "datafusion")]
pub fn create_datafusion_backend(
    record_store_factory: Arc<RecordStoreFactory>,
    schema_provider: Arc<dyn SchemaProvider>,
    cache_manager: Arc<ArrowCacheManager>,
) -> Arc<DataFusionBackend> { ... }

pub fn create_default_backend() -> Arc<PredicateBackend> {
    Arc::new(PredicateBackend)
}
```

## Acceptance Criteria

1. `QueryService::new()` accepts `Arc<dyn QueryBackend>` always, and `Option<Arc<dyn SqlQueryBackend>>` only when the `datafusion` feature is enabled (via `#[cfg(feature = "datafusion")]` on both the struct field and the constructor parameter). Non-datafusion call sites omit the `sql_query_backend` parameter entirely.
2. `handle_query_subscribe()` is `async fn` and delegates to `query_backend.execute_query().await` instead of calling `predicate::execute_query()` directly
3. Existing predicate-based query tests pass with only constructor signature updates (adding `Arc::new(PredicateBackend)` and, when `datafusion` is enabled, `None` parameters to `QueryService::new()`)
4. `SQL_QUERY` operation is accepted by QueryService and dispatched to `handle_sql_query()`
5. With `datafusion` feature, `handle_sql_query()` executes SQL and returns `SqlQueryRespPayload` with correct columns and rows
6. Without `datafusion` feature, `handle_sql_query()` returns an appropriate error
7. `record_batches_to_rows()` correctly converts Arrow arrays (including Int32, UInt32, UInt64, TimestampMicrosecond) to `rmpv::Value` rows
8. `ArrowCacheObserverFactory` is registered in RecordStoreFactory only when `datafusion` feature is enabled
9. Standing queries (QueryMutationObserver) continue using PredicateEngine directly — not affected by QueryBackend injection
10. `Operation::SqlQuery` variant exists with correct ctx/payload pattern; `Operation::ctx()` and `Operation::set_connection_id()` cover the new variant
11. `classify()` routes `Message::SqlQuery` to `Operation::SqlQuery` (not `ClassifyError`)
12. The `operation_variant_count` test in `operation.rs` is updated: the test name comment reads "31 client + 1 system", the doc comment near line 572 reads "all 32 variants", and `Operation::SqlQuery { .. }` is included in its exhaustive match

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` — all existing 509+ tests pass (PredicateBackend wired)
2. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server --features datafusion` — all tests pass including SQL integration tests
3. Execute `SELECT name, age FROM users WHERE age > 25 ORDER BY name` end-to-end via `SQL_QUERY` operation in test — returns correct `SqlQueryRespPayload`
4. Mutate a record, re-query — ArrowCache invalidated via observer, fresh results returned
5. Run `pnpm test:integration-rust` — existing integration tests unaffected

## Constraints

- DO NOT remove or modify PredicateEngine behavior — it remains the default backend
- DO NOT use DataFusion for standing query re-evaluation (MutationObserver) — PredicateEngine handles per-record pushdown
- DO NOT send Arrow IPC over the wire — convert to MsgPack Value rows via `record_batches_to_rows()`
- QueryMutationObserver must not be changed — it continues calling predicate functions directly

## Assumptions

- The server assembly code (where RecordStoreFactory is built) is the right place for conditional observer factory registration
- `QueryService` callers will be updated to pass the new `query_backend` parameter (and `sql_query_backend` only under `datafusion` feature)
- The `Operation` enum already uses a pattern where new variants are added with `ctx` + `payload` fields

## Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `Operation::SqlQuery` variant + test count update, `classify()` update, `QueryService` constructor signature change (types only) | -- | ~5% |
| G2 | 2 | `handle_query_subscribe` refactor to `async fn` using `query_backend.execute_query().await`, update existing tests | G1 | ~10% |
| G3 | 2 | `handle_sql_query()` impl, `record_batches_to_rows()` helper, SQL operation tests | G1 | ~15% |
| G4 | 3 | Conditional `ArrowCacheObserverFactory` registration in server assembly, convenience factory fns | G2, G3 | ~10% |

## Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-21 14:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~40% total

**Critical:**
1. **Missing file: `classify.rs` not listed.** The `classify()` function in `packages/server-rust/src/service/classify.rs` currently returns `ClassifyError::ServerToClient { variant: "SqlQuery" }` for `Message::SqlQuery`. This must be changed to construct `Operation::SqlQuery` and route to `service_names::QUERY`. Without this, no `SqlQuery` operation will ever reach `QueryService`. The spec mentions adding the `Operation::SqlQuery` variant but never mentions updating `classify()` to produce it.

2. **File count exceeds Language Profile limit (6 > 5).** The `QueryService::new()` constructor signature change (`query_backend` parameter added) requires updating all call sites. These are: `lib.rs` (line 128), `bin/test_server.rs` (line 279), `sim/cluster.rs` (line 136), plus `query.rs` (tests at lines 1061, 1070, 1083, 1107, 1191). Combined with `operation.rs`, `classify.rs`, and the assembly wiring in `lib.rs`/`test_server.rs`, the actual file count is 6: `query.rs`, `operation.rs`, `classify.rs`, `lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`. The Language Profile sets max 5 files per spec. The spec claims "File count is 1" which is incorrect.

3. **`handle_query_subscribe` sync-to-async conversion underspecified.** The spec notes this is "currently synchronous" and that "the await is valid inside the async block," but the current implementation calls `svc.handle_query_subscribe(&ctx, &payload)` directly (not `.await`ed) at line 430 of query.rs. Making `handle_query_subscribe` return a `Future` (or making it `async`) changes its call site signature. The spec should explicitly state whether the function becomes `async fn handle_query_subscribe(...)` or returns `impl Future`, and show the updated match arm with `.await`.

4. **R2 proposes two contradictory approaches for SqlQueryBackend access without choosing one.** The spec says "Downcast `self.query_backend` to `&dyn SqlQueryBackend` using `Any`. Alternatively, hold an `Option<Arc<dyn SqlQueryBackend>>`." Implementors need a single clear approach. Recommendation: use `Option<Arc<dyn SqlQueryBackend>>` since downcasting `dyn QueryBackend` to `dyn SqlQueryBackend` via `Any` requires `QueryBackend: Any` which is not in the trait definition from SPEC-135a.

5. **Spec says "File count is 1 (query.rs modification)" but then itself lists operation.rs as a second file in R2.** This is a self-contradiction in the Context section.

**Recommendations:**
6. AC3 says "Existing predicate-based query tests pass without modification" but the constructor signature change (`query_backend` parameter) will require modifying every test that calls `QueryService::new()` (at least 5 test call sites in query.rs). Reword to: "Existing predicate-based query tests pass with only constructor signature updates (adding `Arc::new(PredicateBackend)` parameter)."

7. The `Operation` enum method is named `set_connection_id()` (not `ctx_mut()` as the spec says in R2). The spec references "Operation::ctx() and Operation::ctx_mut()" but the actual code has `Operation::ctx()` and `Operation::set_connection_id()`. This has been corrected in the updated spec text above.

8. Consider adding `Int32Array`, `UInt64Array`, `TimestampMicrosecondArray` to the Arrow-to-rmpv conversion list in R3, since DataFusion commonly produces these types from aggregation and timestamp operations.

9. [Strategic] The `handle_sql_query` method returns `OperationResponse::Message(bytes)` per R2 step 6, but the spec shows `OperationResponse::Message(Box<messages::Message>)` in operation.rs. The spec should clarify it returns `OperationResponse::Message(Box::new(Message::SqlQueryResp { payload }))`, not raw bytes.

### Response v1 (2026-03-21)
**Applied:** All critical issues (1-5) and all recommendations (6-9)

**Changes:**
1. [+] Missing classify.rs -- Added `classify.rs` as a formally listed file in R2 with explicit code showing the `Message::SqlQuery` match arm constructing `Operation::SqlQuery`. Added `classify()` update to G1 Task Group.
2. [+] File count exceeds limit -- Removed "File count is 1" claim from Context section. Added a "File count note" acknowledging 6 files and justifying that 3 (`lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`) are trivial constructor call-site updates.
3. [+] handle_query_subscribe underspecified -- Explicitly stated it becomes `async fn handle_query_subscribe(...)` and added the `.await` match arm code in R1.
4. [+] Contradictory approaches -- Chose `Option<Arc<dyn SqlQueryBackend>>`. Removed the `Any` downcast alternative. Added `sql_query_backend: Option<Arc<dyn SqlQueryBackend>>` to `QueryService` struct and constructor in R1 and R2.
5. [+] Self-contradiction "File count is 1" -- Removed the contradictory claim from Context section (same fix as #2).
6. [+] AC3 reword -- Changed "pass without modification" to "pass with only constructor signature updates (adding `Arc::new(PredicateBackend)` and `None` parameters to `QueryService::new()`)".
7. [+] set_connection_id() vs ctx_mut() -- Already corrected in spec text; confirmed correct method name `set_connection_id()` is used in R2.
8. [+] Additional Arrow types -- Added `Int32Array`, `UInt32Array`, `UInt64Array`, `TimestampMicrosecondArray` to the conversion list in R3.
9. [+] OperationResponse clarification -- R2 step 5 now explicitly shows `OperationResponse::Message(Box::new(Message::SqlQueryResp { payload: SqlQueryRespPayload { columns, rows, error: None } }))`.

**Additional fix:** Moved Task Groups from `###` heading (incorrectly nested under Assumptions) to `##` heading at the correct document level.

### Audit v2 (2026-03-21 15:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~40% total

**Critical:**
1. **R2 classify() code snippet uses nonexistent `Classified` struct and wrong `OperationContext::new()` signature.** The spec shows `Ok(Classified { service_name: ..., operation: Operation::SqlQuery { ctx: OperationContext::new(), payload } })` but `Classified` does not exist in the codebase. The `classify()` method on `OperationService` returns `Result<Operation, ClassifyError>` directly -- not a `Classified` wrapper. Additionally, `OperationContext::new()` requires 4 parameters (`call_id`, `service_name`, `timestamp`, `default_timeout_ms`), not zero. Every other classify arm uses `self.make_ctx(service_name, client_id, caller_origin, partition_key)` to build the context. The correct pattern is:
   ```rust
   Message::SqlQuery { payload } => {
       let ctx = self.make_ctx(service_names::QUERY, client_id, caller_origin, None);
       Ok(Operation::SqlQuery { ctx, payload })
   }
   ```

2. **R2 classify() code snippet uses tuple variant syntax `Message::SqlQuery(payload)` but the actual `Message` enum uses struct variant syntax `Message::SqlQuery { payload: SqlQueryPayload }`.** The match arm must use `Message::SqlQuery { payload }` (with braces), not `Message::SqlQuery(payload)` (with parentheses). This will not compile.

**Recommendations:**
3. The `operation_variant_count_covers_all_30_client_plus_1_system` test in `operation.rs` (line 572) has an exhaustive match that will fail to compile after adding `Operation::SqlQuery`. The spec should note that this test's match arms and comment (currently "all 30 client + 1 system") need updating to "31 client + 1 system". This is minor but easily overlooked.

4. [Compliance] The file count note justification is reasonable -- 3 of 6 files are single-line constructor call-site changes. However, the spec should explicitly list the `lib.rs` second call site: there are actually **two** `QueryService::new()` calls in `lib.rs` (lines 128 and 361), not one. The second is in a different `register()` call. Both need updating.

**Strategic fit:** Aligned with project goals -- wiring the DataFusion backend into the existing service routing framework follows established patterns.

**Project compliance:** Honors PROJECT.md decisions. Uses MsgPack for wire format (not Arrow IPC). Follows trait-first pattern (QueryBackend/SqlQueryBackend defined in SPEC-135a, wired here). No new runtime dependencies added.

**Rust type mapping:** No violations. SqlQueryPayload uses `String` fields (correct for SQL text and query IDs). Conversion uses proper integer types for Arrow arrays.

### Response v2 (2026-03-21)
**Applied:** All critical issues (1-2) and all recommendations (3-4)

**Changes:**
1. [+] classify() code snippet correctness — Confirmed and verified: R2 already contains the correct pattern using `Message::SqlQuery { payload }` struct variant syntax, `self.make_ctx(service_names::QUERY, client_id, caller_origin, None)`, and `Ok(Operation::SqlQuery { ctx, payload })`. No `Classified` struct, no zero-arg `OperationContext::new()`. The Response v1 revisions correctly addressed the original bad code.
2. [+] Message::SqlQuery variant syntax — Confirmed: R2 code snippet uses `Message::SqlQuery { payload }` with braces throughout. No tuple-variant syntax remains.
3. [+] operation_variant_count test — Added explicit note in R2 (operation.rs section) that the `operation_variant_count_covers_all_30_client_plus_1_system` test must be updated: add `Operation::SqlQuery { .. }` to its exhaustive match and change the comment to "31 client + 1 system". Added AC12 to Acceptance Criteria to make this verifiable. Added task to G1 in Task Groups.
4. [+] lib.rs dual call sites — Updated the File count note to explicitly state that `lib.rs` contains **two** `QueryService::new()` call sites (one in main assembly, one in a separate `register()` path) and both must be updated.

### Audit v3 (2026-03-21 17:00)
**Status:** APPROVED

**Context Estimate:** ~40% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~40% | <=50% | OK |
| Largest task group | ~15% (G3) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**
- Clarity: All requirements use concrete code snippets matching actual codebase patterns. No vague terms.
- Completeness: All files listed with explicit paths. Dual call-site in `lib.rs` documented. File count justification provided.
- Testability: All 12 acceptance criteria are measurable and verifiable by compilation + test execution.
- Scope: Constraints clearly define boundaries. Task groups properly decomposed.
- Feasibility: Technical approach verified against codebase -- `classify()` pattern, `Operation` variant pattern, Tower `Service::call()` pattern all match.
- Architecture fit: Follows established DI-via-constructor, Tower service routing, and feature-gating patterns.
- Non-duplication: Reuses existing `PredicateBackend`, `execute_query()`, and `RecordStoreFactory` observer factory infrastructure.
- Cognitive load: Well-separated concerns across 4 requirements. Each requirement is self-contained.

**Rust type mapping:** No violations. `SqlQueryPayload` uses `String` fields. Arrow conversion uses proper integer types (`Int32Array`, `UInt32Array`, `UInt64Array`, `TimestampMicrosecondArray`).

**Strategic fit:** Aligned with project goals -- wiring DataFusion into the service routing framework completes the SQL query pipeline (SPEC-135a traits -> SPEC-135b engine -> SPEC-135c integration).

**Project compliance:** Honors PROJECT.md decisions. MsgPack wire format (not Arrow IPC). Feature-gated DataFusion. No new runtime dependencies.

**Language profile:** File count is 6, exceeding the limit of 5. Justification accepted -- 3 of 6 files are single-line constructor call-site updates. Substantive changes are confined to 3 files (`query.rs`, `operation.rs`, `classify.rs`). G1 contains types/routing only (trait-first compliant).

**Recommendations:**
1. R4 convenience functions (`create_datafusion_backend`, `create_default_backend`) lack an explicit file location. The implementor should place them in `query_backend.rs` (alongside the trait definitions) or in the assembly modules (`lib.rs`). Consider specifying this to avoid ambiguity.

**Comment:** Spec is well-structured after two rounds of revision. All previous critical issues have been addressed. Code snippets match actual codebase patterns (verified against `classify.rs`, `operation.rs`, `query.rs`). The 4-group wave structure with parallel G2/G3 is sensible for the task decomposition.

### Response v3 (2026-03-21)
**Applied:** Recommendation 1 from Audit v3

**Changes:**
1. [+] R4 convenience function file location — Added explicit `**File:** packages/server-rust/src/service/domain/query_backend.rs (modify)` directive to R4, placing `create_datafusion_backend` and `create_default_backend` alongside the trait definitions. Removed ambiguity about whether the functions belong in assembly modules or the trait module.

### Audit v4 (2026-03-21 18:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~40% total

**Critical:**
1. **`sql_query_backend` field references feature-gated trait without `#[cfg]` attribute.** The `SqlQueryBackend` trait is defined with `#[cfg(feature = "datafusion")]` in `query_backend.rs` (line 85). The R1 struct definition shows `sql_query_backend: Option<Arc<dyn SqlQueryBackend>>` as a plain field on `QueryService` -- but `SqlQueryBackend` does not exist when the `datafusion` feature is disabled. This means the struct, constructor, and AC1 will not compile without the `datafusion` feature. The field must be feature-gated: `#[cfg(feature = "datafusion")] sql_query_backend: Option<Arc<dyn SqlQueryBackend>>`. The constructor parameter must also be feature-gated. The non-datafusion call sites (`lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`) should NOT pass `None` for `sql_query_backend` -- they simply omit the parameter. The `handle_sql_query()` method already has two `#[cfg]` variants (R2 step 1), but the struct field and constructor are missing the same treatment. AC1 must also be updated to reflect the conditional parameter.

**Recommendations:**
2. The existing `operation_variant_count` test comment says "all 31 variants" (line 572) but the match has 30 client + 1 system = 31 arms. After adding `SqlQuery`, the comment should say "all 32 variants" and "31 client + 1 system". The spec's AC12 says "31 client + 1 system" which is correct for the comment text, but the test's doc comment on line 572 says "all 31 variants" and should become "all 32 variants". Minor but could confuse implementors.

**Dimensions:**
- Clarity: Excellent -- concrete code snippets, explicit file paths, clear constraints.
- Completeness: Good overall, except for the feature-gating gap on the struct field (Critical 1).
- Testability: All 12 acceptance criteria are measurable.
- Scope: Well-bounded by constraints. File count justification is reasonable.
- Feasibility: Sound approach -- all patterns verified against codebase.
- Architecture fit: Follows established DI-via-constructor and Tower service routing patterns.
- Non-duplication: Reuses existing infrastructure (PredicateBackend, RecordStoreFactory observers).
- Cognitive load: Well-separated across 4 requirements.
- Strategic fit: Aligned with project goals -- completes the SQL pipeline (135a traits, 135b engine, 135c integration).
- Project compliance: Honors PROJECT.md decisions (MsgPack wire format, feature-gated DataFusion, no new deps).

**Rust type mapping:** No violations. `SqlQueryPayload` uses `String` fields. Arrow conversion uses proper integer types.

**Language profile:** File count 6 > limit 5. Justification accepted (3 trivial call-site updates). G1 is types/routing only (trait-first compliant).

### Response v4 (2026-03-21)
**Applied:** Critical issue 1 and Recommendation 2

**Changes:**
1. [+] Feature-gate `sql_query_backend` field and constructor parameter — R1 struct definition updated to add `#[cfg(feature = "datafusion")]` on `sql_query_backend` field. Constructor updated with `#[cfg(feature = "datafusion")]` on the `sql_query_backend` parameter. Added explicit note that non-datafusion call sites omit the parameter entirely (do not pass `None`). Task section updated to reflect that only `datafusion`-enabled call sites pass both parameters.
2. [+] AC1 updated — Changed AC1 from unconditional `Option<Arc<dyn SqlQueryBackend>>` parameter to conditional: "accepts `Arc<dyn QueryBackend>` always, and `Option<Arc<dyn SqlQueryBackend>>` only when the `datafusion` feature is enabled (via `#[cfg(feature = "datafusion")]` on both the struct field and the constructor parameter). Non-datafusion call sites omit the `sql_query_backend` parameter entirely."
3. [+] Test doc comment — R2 (operation.rs section) updated to note that the doc comment on line 572 reads "all 31 variants" and must become "all 32 variants" (in addition to the test name comment "31 client + 1 system"). AC12 updated to include the doc comment change explicitly.

### Audit v5 (2026-03-21 19:30)
**Status:** APPROVED

**Context Estimate:** ~40% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~40% | <=50% | OK |
| Largest task group | ~15% (G3) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**
- Clarity: All 4 requirements use concrete Rust code snippets that match actual codebase patterns. No vague terms. File paths are explicit.
- Completeness: All 6 files listed with justification for exceeding the 5-file limit. Dual call-site in `lib.rs` documented. Feature-gating on struct field, constructor parameter, and method body all specified with `#[cfg]` attributes.
- Testability: All 12 acceptance criteria are measurable -- verifiable by compilation (both with and without `datafusion` feature) and test execution.
- Scope: Constraints clearly define boundaries (no PredicateEngine changes, no Arrow IPC, no MutationObserver changes). Task groups properly decomposed into 4 groups across 3 waves.
- Feasibility: Technical approach is sound -- `classify()` pattern, `Operation` variant pattern, Tower `Service::call()` pattern, and `#[cfg(feature)]` conditional compilation all match established codebase conventions.
- Architecture fit: Follows DI-via-constructor, Tower service routing, feature-gating, and observer factory patterns already present in the codebase.
- Non-duplication: Reuses `PredicateBackend`, `execute_query()`, `RecordStoreFactory` observer infrastructure, and `make_ctx()` pattern from existing code.
- Cognitive load: Well-separated concerns -- R1 (injection), R2 (operation routing), R3 (conversion helper), R4 (assembly wiring). Each requirement is self-contained.
- Strategic fit: Aligned with project goals -- completes the SQL query pipeline (SPEC-135a traits -> SPEC-135b engine -> SPEC-135c integration). No scope mismatch or symptom treatment.
- Project compliance: Honors PROJECT.md decisions -- MsgPack wire format (not Arrow IPC), feature-gated DataFusion, no new runtime dependencies, trait-first ordering in G1.

**Rust type mapping:** No violations. `SqlQueryPayload` uses `String` fields. Arrow conversion covers proper integer types (`Int32Array`, `UInt32Array`, `UInt64Array`, `TimestampMicrosecondArray`). No `f64` for integer-semantic fields.

**Language profile:** File count 6 > limit 5. Justification accepted -- 3 of 6 files (`lib.rs`, `bin/test_server.rs`, `sim/cluster.rs`) are single-line constructor call-site updates. Substantive changes confined to 3 files (`query.rs`, `operation.rs`, `classify.rs`). G1 contains types/routing only (trait-first compliant).

**Assumptions validated:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-135a/135b are complete (depends_on declared) | Cannot wire nonexistent traits/engine |
| A2 | classify() currently returns ClassifyError for SqlQuery | Wrong match arm to modify |
| A3 | Operation enum uses ctx + payload pattern for all variants | New variant won't match existing dispatch |

All assumptions are reasonable and low-risk given the declared dependency chain and established codebase patterns.

**Comment:** Spec is thorough and well-refined after 4 rounds of revision. All previous critical issues (missing classify.rs, contradictory approaches, feature-gating gaps, incorrect code snippets) have been resolved. The current version is clear, complete, and implementable. Code snippets correctly use `self.make_ctx()`, struct variant syntax, and `#[cfg(feature = "datafusion")]` throughout.

---

## Review History

### Review v1 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `QueryService` struct field `sql_query_backend` is `#[cfg(feature = "datafusion")]`-gated; constructor parameter is likewise gated. Non-datafusion call sites in `lib.rs`, `bin/test_server.rs`, and `sim/cluster.rs` correctly omit the parameter.
- [✓] AC2: `handle_query_subscribe()` is `async fn` and delegates to `self.query_backend.execute_query().await` — no direct call to `predicate::execute_query()`.
- [✓] AC3: 565 tests pass without datafusion feature; all predicate-based query tests run correctly with `Arc::new(PredicateBackend)` wired.
- [✓] AC4: `Operation::SqlQuery { ctx, payload }` match arm in `Service::call()` dispatches to `handle_sql_query().await`.
- [✓] AC5: `handle_sql_query()` (datafusion build) calls `sql_backend.execute_sql()`, converts batches via `record_batches_to_rows()`, and returns `OperationResponse::Message(Box::new(Message::SqlQueryResp { payload }))`. SQL execution errors are wrapped in an error-bearing `SqlQueryRespPayload` rather than an `OperationError`, which is a reasonable design choice for user-visible query errors.
- [✓] AC6: Non-datafusion `handle_sql_query()` returns `OperationError::Internal("SQL requires datafusion feature")`.
- [✓] AC7: `record_batches_to_rows()` covers all specified types including `Int32Array`, `UInt32Array`, `UInt64Array`, `TimestampMicrosecondArray`, plus `Float32Array` (bonus), and uses `arrow::util::display::ArrayFormatter` as the debug fallback for unsupported types (cleaner than raw `Debug`).
- [✓] AC8: `ArrowCacheObserverFactory` registered in `lib.rs` and `bin/test_server.rs` under `#[cfg(feature = "datafusion")]`; not registered otherwise. `sim/cluster.rs` correctly omits it (sim does not need Arrow cache).
- [✓] AC9: `QueryMutationObserver` is unchanged — `matches_query()` still calls `evaluate_predicate()` / `evaluate_where()` directly.
- [✓] AC10: `Operation::SqlQuery` variant present with `ctx: OperationContext` and `payload: messages::query::SqlQueryPayload`. `Operation::ctx()` (line 338) and `Operation::set_connection_id()` (line 390) cover it.
- [✓] AC11: `classify()` in `classify.rs` routes `Message::SqlQuery { payload }` to `Ok(Operation::SqlQuery { ctx, payload })` using `self.make_ctx(service_names::QUERY, ...)`.
- [✓] AC12: Test function name is `operation_variant_count_covers_all_31_client_plus_1_system`; doc comment reads "all 32 variants"; `Operation::SqlQuery { .. }` is in the exhaustive match.
- [✓] Build check: `cargo check` passes both with and without `datafusion` feature.
- [✓] Lint check: `cargo clippy -- -D warnings` passes both with and without `datafusion` feature.
- [✓] Test count: 565+4=569 (no datafusion), 597+4=601 (with datafusion) — matches STATE.md assertion.
- [✓] Constraints honored: No Arrow IPC sent over wire; PredicateEngine unchanged; QueryMutationObserver unchanged.
- [✓] Convenience functions `create_default_backend()` and `create_datafusion_backend()` present in `query_backend.rs` as specified.

**Minor:**
1. `arrow_cache_manager` variable in `lib.rs` (line 103, inside `#[cfg(feature = "datafusion")]` test setup block) is unnamed with `_` prefix, unlike the equivalent in `bin/test_server.rs` which uses `_arrow_cache_manager`. This generates a compiler warning (`unused variable: arrow_cache_manager`) during `cargo test --release --features datafusion`. Three unused-import warnings also present in test code. These do not fail clippy (test code is excluded from the configured clippy invocation) and do not affect correctness or the production build.

**Summary:** Implementation is complete and correct. All 12 acceptance criteria are satisfied. Both feature builds compile and pass their full test suites. Architecture follows established codebase patterns throughout — DI-via-constructor, Tower service routing, `#[cfg(feature)]` conditional compilation, and observer factory wiring. The minor warning about `arrow_cache_manager` naming inconsistency between `lib.rs` and `test_server.rs` is cosmetic.

### Fix Response v1 (2026-03-21)
**Applied:** Minor issue 1 from Review v1

**Fixes:**
1. [✓] `arrow_cache_manager` unnamed with `_` prefix in `lib.rs` — renamed to `_arrow_cache_manager` to match `bin/test_server.rs` convention and suppress unused variable warning
   - Commit: 43bb9bf

---

### Review v2 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verified:**
- [✓] Minor issue from Review v1 resolved: `arrow_cache_manager` in `lib.rs` (line 103) is now `_arrow_cache_manager`, matching `bin/test_server.rs` convention. No unused variable warnings in either file. Commit 43bb9bf confirmed.

**Passed:**
- [✓] AC1: `QueryService` struct field `sql_query_backend` is `#[cfg(feature = "datafusion")]`-gated (line 354-355 of query.rs); constructor parameter likewise gated (line 366-367). Non-datafusion call sites in `lib.rs`, `bin/test_server.rs`, and `sim/cluster.rs` correctly omit the parameter. `lib.rs` dual call sites (lines 147 and 383) both updated correctly.
- [✓] AC2: `handle_query_subscribe()` is `async fn` (line 469) and delegates to `self.query_backend.execute_query().await` (line 503-507) — no direct call to `predicate::execute_query()`.
- [✓] AC3: 565+4=569 tests pass without datafusion feature; predicate-based query tests run correctly with `Arc::new(PredicateBackend)` wired at all 5 test call sites.
- [✓] AC4: `Operation::SqlQuery { ctx, payload }` match arm in `Service::call()` (line 445-447) dispatches to `handle_sql_query().await`.
- [✓] AC5: `handle_sql_query()` (datafusion build, lines 558-592) calls `sql_backend.execute_sql()`, converts batches via `record_batches_to_rows()`, returns `OperationResponse::Message(Box::new(Message::SqlQueryResp { payload }))`. SQL errors wrapped in error-bearing `SqlQueryRespPayload`.
- [✓] AC6: Non-datafusion `handle_sql_query()` (lines 595-605) returns `OperationError::Internal("SQL requires datafusion feature")`.
- [✓] AC7: `record_batches_to_rows()` (lines 617-642) and `arrow_value_to_rmpv()` (lines 646-718) cover all specified types: `Int32Array`, `Int64Array`, `UInt32Array`, `UInt64Array`, `Float32Array` (bonus), `Float64Array`, `BooleanArray`, `StringArray`, `LargeStringArray`, `BinaryArray`, `TimestampMicrosecondArray`. Fallback uses `ArrayFormatter` for unsupported types.
- [✓] AC8: `ArrowCacheObserverFactory` registered under `#[cfg(feature = "datafusion")]` in `lib.rs` (lines 102-113) and `bin/test_server.rs` (lines 251-270). `sim/cluster.rs` correctly omits it.
- [✓] AC9: `QueryMutationObserver` unchanged — `matches_query()` still calls `evaluate_predicate()` / `evaluate_where()` directly.
- [✓] AC10: `Operation::SqlQuery` variant with `ctx: OperationContext` and `payload: SqlQueryPayload`. `Operation::ctx()` (line 338) and `Operation::set_connection_id()` (line 390) both cover it.
- [✓] AC11: `classify()` routes `Message::SqlQuery { payload }` to `Ok(Operation::SqlQuery { ctx, payload })` using `self.make_ctx(service_names::QUERY, client_id, caller_origin, None)` (classify.rs lines 441-448).
- [✓] AC12: Test function name is `operation_variant_count_covers_all_31_client_plus_1_system` (line 582); doc comment reads "all 32 variants" (line 579); `Operation::SqlQuery { .. }` is in the exhaustive match (line 601).
- [✓] Build check: `cargo check -p topgun-server` passes both with and without `datafusion` feature (exit 0).
- [✓] Lint check: `cargo clippy -p topgun-server -- -D warnings` passes both with and without `datafusion` feature (exit 0, no warnings).
- [✓] Test count: 565+4=569 (no datafusion), 597+4=601 (with datafusion) — all pass, no failures.
- [✓] Convenience functions `create_default_backend()` and `create_datafusion_backend()` present in `query_backend.rs` (lines 137-155) as specified.
- [✓] Constraints honored: No Arrow IPC over wire; `PredicateEngine` unchanged; `QueryMutationObserver` unchanged.

**Summary:** Fix from Review v1 correctly applied. All 12 acceptance criteria remain satisfied. Both feature builds compile clean, pass full lint (clippy -D warnings), and pass all tests. No new issues found.

---

## Completion

**Completed:** 2026-03-21
**Total Commits:** 4
**Review Cycles:** 2

### Outcome

Wired DataFusion SQL engine into QueryService, completing the SQL query pipeline (SPEC-135a traits → SPEC-135b engine → SPEC-135c integration). SQL_QUERY operations are routed through classify → Operation → QueryService → DataFusionBackend, with ArrowCacheObserverFactory conditionally registered for cache invalidation.

### Key Files

- `packages/server-rust/src/service/domain/query.rs` — QueryBackend injection, async handle_query_subscribe, handle_sql_query, record_batches_to_rows
- `packages/server-rust/src/service/operation.rs` — Operation::SqlQuery variant
- `packages/server-rust/src/service/classify.rs` — SQL_QUERY message routing
- `packages/server-rust/src/service/domain/query_backend.rs` — create_default_backend/create_datafusion_backend convenience functions

### Patterns Established

None — followed existing patterns (DI-via-constructor, Tower service routing, #[cfg(feature)] conditional compilation, observer factory wiring).

### Deviations

None — implemented as specified.
