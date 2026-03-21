---
id: SPEC-135b
type: feature
status: done
priority: P1
complexity: medium
parent: SPEC-135
depends_on: [SPEC-135a]
created: 2026-03-20
source: TODO-091
---

# DataFusion SQL: Engine Implementation

## Context

SPEC-135a established the `QueryBackend` trait, `SqlQueryBackend` trait, `QueryBackendError`, `PredicateBackend`, wire messages, and the `datafusion` Cargo feature flag. This sub-spec implements the DataFusion engine: the Arrow cache, MsgPack-to-Arrow conversion, DataFusion `TableProvider`, and the `DataFusionBackend` that wires everything together.

All code in this spec is feature-gated behind `#[cfg(feature = "datafusion")]`. It compiles only when `cargo build --features datafusion` is used.

**Key audit fixes applied:**
- ArrowCacheManager is fully feature-gated behind `datafusion` per audit issue 6.
- DistributedPlanner is deferred entirely per audit issue 5.
- File count is 4 (within the 5-file limit).
- G1 is trait-first (types only).

## Task

Implement the DataFusion SQL query engine behind the `datafusion` feature flag: `ArrowCacheManager` (lazy Arrow cache with mutation invalidation), `build_record_batch` (MsgPack-to-Arrow conversion), `TopGunTableProvider` (DataFusion `TableProvider` + `ExecutionPlan`), and `DataFusionBackend` (wires SessionContext, implements both `QueryBackend` and `SqlQueryBackend`).

## Requirements

### R1: ArrowCacheManager

**File:** `packages/server-rust/src/service/domain/arrow_cache.rs` (new, `#[cfg(feature = "datafusion")]`)

Lazy MsgPack-to-Arrow cache with mutation invalidation:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use arrow::array::RecordBatch;
use dashmap::DashMap;

pub struct CachedBatch {
    pub batch: RecordBatch,
    pub version: u64,
}

pub struct ArrowCacheManager {
    cache: DashMap<(String, u32), CachedBatch>,
    /// Version counters per (map_name, partition_id). Incremented on invalidation.
    versions: DashMap<(String, u32), Arc<AtomicU64>>,
}
```

Methods:
- `new() -> Self`: creates empty cache and version maps.
- `get_or_build(map_name, partition_id, build_fn) -> Result<RecordBatch>`: returns cached batch if version matches current counter; otherwise calls `build_fn()` to produce a new `RecordBatch`, stores it, and returns it. `build_fn` is `FnOnce() -> Result<RecordBatch>`.
- `invalidate(map_name, partition_id)`: increments the version counter for the key and removes the cached batch entry.
- `current_version(map_name, partition_id) -> u64`: returns the current version counter (0 if no entry).

`ArrowCacheObserverFactory` implements `ObserverFactory` (from `crate::storage::factory`):
- `create_observer(map_name, partition_id)` returns `Some(Arc::new(ArrowCacheObserver { ... }))`.
- `ArrowCacheObserver` implements `MutationObserver`. On `on_put`, `on_update`, `on_remove`, `on_clear`, `on_reset`, and `on_replication_put`, it calls `cache_manager.invalidate(map_name, partition_id)`. Other methods (`on_evict`, `on_load`, `on_destroy`) are no-ops.

### R2: Value-to-Arrow Batch Conversion

**File:** `packages/server-rust/src/service/domain/arrow_convert.rs` (new, `#[cfg(feature = "datafusion")]`)

`build_record_batch(entries, schema) -> Result<RecordBatch>`:
- Takes `entries: &[(String, rmpv::Value)]` (key-value pairs from RecordStore iteration) and `arrow_schema: &arrow::datatypes::Schema`.
- The first field in the schema is always `_key` (Utf8). Remaining fields correspond to `MapSchema` fields.
- For each entry, extracts the key for `_key` column and field values from the `rmpv::Value::Map`. If an entry's LWW value is not `rmpv::Value::Map` (e.g., a scalar), log `tracing::warn!` and skip that entry — do not append any row.
- Appends to per-column Arrow array builders based on the Arrow schema field type:
  - `DataType::Int64` -> `Int64Builder` (from `rmpv::Value::Integer`)
  - `DataType::Float64` -> `Float64Builder` (from `rmpv::Value::F64`)
  - `DataType::Utf8` -> `StringBuilder` (from `rmpv::Value::String`)
  - `DataType::Boolean` -> `BooleanBuilder` (from `rmpv::Value::Boolean`)
  - `DataType::Binary` -> `BinaryBuilder` (from `rmpv::Value::Binary`)
  - `DataType::Timestamp(TimeUnit::Millisecond, _)` -> `TimestampMillisecondBuilder` (from `rmpv::Value::Integer`, which carries `i64` millis)
  - `DataType::List(_)` -> `ListBuilder` (from `rmpv::Value::Array`). The inner builder type must match the list's inner field DataType (e.g., `ListBuilder::new(Int64Builder::new())` for `List(Field::new("item", Int64, true))`). For unsupported or complex inner types, fall back to `ListBuilder::new(StringBuilder::new())` with JSON-serialized elements.
  - Any other type or `rmpv::Value::Map`/complex -> serialize to JSON string -> `StringBuilder`
  - `rmpv::Value::Nil` -> `append_null` on any builder
- Returns `RecordBatch::try_new(Arc::new(schema.clone()), columns)`.

Helper: `make_arrow_schema(map_schema: &MapSchema) -> arrow::datatypes::Schema`:
- Prepends `Field::new("_key", DataType::Utf8, false)` to the fields from `map_schema.to_arrow_schema()`.

### R3: TopGunTableProvider

**File:** `packages/server-rust/src/service/domain/table_provider.rs` (new, `#[cfg(feature = "datafusion")]`)

Implements DataFusion's `TableProvider` trait:

`TopGunTableProvider` fields:
- `map_name: String`
- `arrow_schema: Arc<arrow::datatypes::Schema>`  (with `_key` prepended)
- `record_store_factory: Arc<RecordStoreFactory>`
- `cache_manager: Arc<ArrowCacheManager>`

`TableProvider` impl:
- `schema()` returns `self.arrow_schema.clone()`.
- `table_type()` returns `TableType::Base`.
- `scan(state, projection, filters, limit)` returns `Ok(Arc::new(TopGunExec::new(...)))`.

`TopGunExec` implements `ExecutionPlan`:
- `schema()` returns the Arrow schema (projected if projection is set, full otherwise).
- `children()` returns an empty vec (leaf node).
- `with_new_children(children)` returns `Ok(self.clone())` if children is empty, error otherwise (leaf node has no children).
- `properties()` returns `PlanProperties` with `EquivalenceProperties::new(schema)`, `Partitioning::UnknownPartitioning(1)`, and `ExecutionMode::Bounded`.
- `output_partitioning()` returns `Partitioning::UnknownPartitioning(1)`. The scan aggregates all TopGun partitions into a single DataFusion partition.
- `execute(partition_idx, context)` caches and aggregates per TopGun partition using option (a): call `get_or_build` once per TopGun partition_id (from `RecordStoreFactory::get_all_for_map()`), building a `RecordBatch` for each partition individually. Concatenate the resulting batches via `arrow::compute::concat_batches` into one final `RecordBatch`. This ensures that invalidating partition N only triggers a rebuild of partition N on the next read.
  - The build closure for each partition:
    1. Gets the RecordStore for that `(map_name, partition_id)` from the factory.
    2. Iterates the store's records via `for_each_boxed()`, collecting `(key, rmpv::Value)` pairs (only `RecordValue::Lww` entries; skip `OrMap` with `tracing::warn`).
    3. Calls `build_record_batch()` to convert to Arrow.
- Returns a `RecordBatchStream` wrapping the concatenated `RecordBatch`.
- Supports projection pushdown: if `projection` is `Some`, only materializes requested columns. Apply projection after building full batch via `RecordBatch::project()`.
- Filter pushdown: best-effort. For this spec, filters are NOT pushed down to PredicateEngine (DataFusion applies them post-scan). Filter pushdown optimization is deferred.

**Note:** Use DataFusion's `MemoryExec` as a reference implementation for the mechanical `ExecutionPlan` trait methods (`schema`, `children`, `with_new_children`, `properties`).

### R4: DataFusionBackend

**File:** `packages/server-rust/src/service/domain/datafusion_backend.rs` (new, `#[cfg(feature = "datafusion")]`)

`DataFusionBackend` implements both `QueryBackend` and `SqlQueryBackend`:

Fields:
- `ctx: SessionContext` (DataFusion)
- `record_store_factory: Arc<RecordStoreFactory>`
- `schema_provider: Arc<dyn SchemaProvider>`
- `cache_manager: Arc<ArrowCacheManager>`

Constructor: `new(record_store_factory, schema_provider, cache_manager) -> Self`:
- Creates a `SessionContext::new()`.

`QueryBackend` impl:
- `execute_query(map_name, entries, query)` delegates to `predicate::execute_query()` (same as `PredicateBackend`). DataFusion is not used for predicate queries.
- `register_map(map_name)`:
  1. Calls `schema_provider.get_schema(map_name).await`.
  2. If `None`, returns `Err(QueryBackendError::SchemaRequired(...))`.
  3. Builds `arrow_schema` via `make_arrow_schema(&map_schema)`.
  4. Creates `TopGunTableProvider` with the schema, record_store_factory, and cache_manager.
  5. Registers as table: `ctx.register_table(map_name, Arc::new(provider))`.
- `deregister_map(map_name)`: calls `ctx.deregister_table(map_name)`.

`SqlQueryBackend` impl:
- `execute_sql(sql)`:
  1. `let df = self.ctx.sql(sql).await.map_err(|e| QueryBackendError::SqlParse(e.to_string()))?;`
  2. `let batches = df.collect().await.map_err(|e| QueryBackendError::Execution(e.to_string()))?;`
  3. Returns `Ok(batches)`.

### R5: Module Registration

**File:** `packages/server-rust/src/service/domain/mod.rs` (modify -- already touched by SPEC-135a for query_backend)

Add conditionally compiled modules:
```rust
#[cfg(feature = "datafusion")]
pub mod arrow_cache;
#[cfg(feature = "datafusion")]
pub mod arrow_convert;
#[cfg(feature = "datafusion")]
pub mod table_provider;
#[cfg(feature = "datafusion")]
pub mod datafusion_backend;
```

## Acceptance Criteria

1. With `datafusion` feature enabled, `DataFusionBackend` can execute `SELECT * FROM <map>` and return correct RecordBatch results
2. With `datafusion` feature enabled, `SELECT * FROM <map> WHERE <field> > <value>` filters correctly
3. With `datafusion` feature enabled, `SELECT <field>, COUNT(*) FROM <map> GROUP BY <field>` aggregates correctly
4. `TopGunTableProvider::schema()` returns Arrow schema matching MapSchema with prepended `_key` column (Utf8, non-nullable)
5. `TopGunTableProvider::scan()` supports projection pushdown (only requested columns in output RecordBatch)
6. `ArrowCacheManager` returns cached RecordBatch on repeated queries without calling build_fn
7. `ArrowCacheManager` invalidates cache when `invalidate()` is called (next `get_or_build` calls build_fn)
8. `ArrowCacheObserverFactory` creates observers that call `invalidate` on `on_put`, `on_update`, `on_remove`, `on_clear`, `on_reset`, `on_replication_put`
9. `build_record_batch()` converts all Value variants correctly: Int->Int64, Float->Float64, String->Utf8, Bool->Boolean, Bytes->Binary, Null->null, Array->List, Timestamp(Millis)->TimestampMillisecond, Map->JSON string
10. Building with `cargo build -p topgun-server` (no datafusion feature) compiles without these modules
11. Building with `cargo build -p topgun-server --features datafusion` compiles all four modules

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server --features datafusion` -- all tests pass including new DataFusion tests
2. Run `cargo build -p topgun-server` (no features) -- compiles, no datafusion symbols
3. Execute `SELECT name, age FROM users WHERE age > 25 ORDER BY name` in test -- returns correct filtered, sorted rows
4. Mutate a record (invalidate cache), re-query -- ArrowCache rebuilds and returns updated data

## Constraints

- DO NOT remove or modify PredicateEngine behavior -- it remains the default backend
- DO NOT send Arrow IPC over the wire -- this spec handles only in-process Arrow
- DO NOT implement filter pushdown from DataFusion to PredicateEngine in this spec -- filters applied post-scan by DataFusion
- DO NOT add DataFusion to core-rust -- it belongs in server-rust only
- Arrow cache invalidation must be per-(map_name, partition_id), not per-map (too coarse)
- OR-Map records are not SQL-queryable -- skip with `tracing::warn`

## Assumptions

- DataFusion v45 is the target version (latest stable as of March 2026, compatible with arrow v55)
- `arrow` crate v55 aligns with the `arrow-schema` v55 already in core-rust
- OR-Map records are not SQL-queryable (only LWW records appear in tables)
- The `_key` synthetic column is always the first column in every table (Utf8, non-nullable)
- Filter pushdown is deferred -- DataFusion handles all filtering via its own execution engine post-scan
- `DataFusionBackend::execute_query()` delegates to `predicate::execute_query()` (not DataFusion) for backward compatibility with standing query subscriptions

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `ArrowCacheManager` struct, `CachedBatch` struct, `ArrowCacheObserver`/`ArrowCacheObserverFactory` types (types only, no method bodies) | -- | ~5% |
| G2 | 2 | `ArrowCacheManager` impl (get_or_build, invalidate), `ArrowCacheObserver` MutationObserver impl, tests | G1 | ~15% |
| G3 | 2 | `build_record_batch()`, `make_arrow_schema()`, per-column builder logic, tests | G1 | ~15% |
| G4 | 3 | `TopGunTableProvider`, `TopGunExec`, projection pushdown, tests | G2, G3 | ~15% |
| G5 | 3 | `DataFusionBackend` (SessionContext, register_map, execute_sql, execute_query), integration tests | G2, G3 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4, G5 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-20 14:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~65% total

**Critical:**
1. **R2 missing `DataType::Timestamp` handling.** `MapSchema::to_arrow_schema()` produces `DataType::Timestamp(TimeUnit::Millisecond, None)` for `FieldType::Timestamp` fields, but the R2 conversion mapping does not cover this type. The catch-all "any other type -> serialize to JSON string" would silently store timestamp values as JSON strings instead of proper Arrow `TimestampMillisecondBuilder` values. Add an explicit mapping: `DataType::Timestamp(TimeUnit::Millisecond, _)` -> `TimestampMillisecondBuilder` (from `rmpv::Value::Integer`, which carries `i64` millis).

2. **R3 cache key mismatch between invalidation granularity and read aggregation.** `ArrowCacheManager` caches per `(map_name, partition_id)`, and `ArrowCacheObserver` invalidates per `(map_name, partition_id)`. However, `TopGunExec::execute()` aggregates ALL TopGun partitions for a map via `get_all_for_map()` into a single `RecordBatch`. The spec does not specify what `(map_name, partition_id)` key `TopGunExec` passes to `get_or_build()`. Two viable fixes:
   - **(a) Cache per partition, merge at read:** `TopGunExec` calls `get_or_build` once per TopGun partition, then concatenates the resulting batches. Invalidation of partition N only rebuilds partition N. This is more efficient.
   - **(b) Cache per map name only:** Change the cache key to just `map_name` (String). Any partition mutation invalidates the entire map's cache. Simpler but coarser.
   Either approach must be specified explicitly.

3. **R3 `TopGunExec` DataFusion partition count unspecified.** `ExecutionPlan::output_partitioning()` must return the number of DataFusion partitions (not TopGun partitions). Since the scan aggregates all TopGun partitions into one batch, this should return `UnknownPartitioning(1)` or `Partitioning::UnknownPartitioning(1)`. The spec must specify this, as DataFusion uses it to determine parallelism.

**Recommendations:**
4. **R2: Consider accepting `topgun_core::Value` directly.** The current approach converts `Value` -> `rmpv::Value` (via `predicate::value_to_rmpv`) -> Arrow arrays. Accepting `Value` directly in `build_record_batch` would eliminate one conversion layer and avoid rmpv as an intermediary. The existing `rmpv::Value` approach is consistent with QueryService but adds overhead for the Arrow path.

5. **R2: Specify behavior for non-Map LWW values.** If a record's LWW value is a scalar (e.g., `Value::String("hello")`) rather than `Value::Map(...)`, the R2 logic that "extracts field values from the `rmpv::Value::Map`" will fail or produce unexpected results. Specify: skip with `tracing::warn` (matching the OR-Map handling), or treat as a single-value row.

6. **G1 trait-first concern.** G1 defines struct types but `ArrowCacheObserver`/`ArrowCacheObserverFactory` are concrete types, not traits. This is acceptable since the Language Profile's trait-first rule targets ensuring traits/interfaces come before implementations. Here, the traits (`ObserverFactory`, `MutationObserver`) already exist from prior specs. The G1 grouping is reasonable as-is.

7. **AC9 does not mention Timestamp conversion.** If critical issue 1 is addressed, AC9 should be updated to include `Timestamp->TimestampMillisecond` in its list of correct conversions.

### Response v1 (2026-03-20)
**Applied:** all critical issues and all recommendations

**Changes:**
1. [✓] R2 missing `DataType::Timestamp` handling — Added explicit `DataType::Timestamp(TimeUnit::Millisecond, _)` -> `TimestampMillisecondBuilder` mapping in R2 bullet list, sourcing `i64` millis from `rmpv::Value::Integer`.
2. [✓] R3 cache key mismatch — Chose option (a): specified that `TopGunExec::execute()` calls `get_or_build` once per TopGun partition_id, then concatenates results via `arrow::compute::concat_batches`. Updated R3 to remove ambiguity and make the per-partition cache/merge strategy explicit.
3. [✓] R3 `TopGunExec` DataFusion output partitioning unspecified — Added explicit `output_partitioning()` method to `TopGunExec` returning `Partitioning::UnknownPartitioning(1)`.
4. [✓] R2: Recommendation to accept `topgun_core::Value` directly — Noted but not applied. The `rmpv::Value` approach is kept for consistency with QueryService; the recommendation is a valid future optimization but switching input types constitutes a broader interface change that is out of scope for this spec.
5. [✓] R2: Specify behavior for non-Map LWW values — Added: "If an entry's LWW value is not `rmpv::Value::Map`, log `tracing::warn!` and skip that entry — do not append any row."
6. [✗] G1 trait-first concern — Audit noted this is acceptable as-is; no change needed.
7. [✓] AC9 Timestamp conversion — Updated AC9 to include `Timestamp(Millis)->TimestampMillisecond` in the conversion list.

**Skipped:** Item 4 (accepting `topgun_core::Value` directly) is a valid optimization but changes the function signature in a way that affects the broader call chain. Deferred to a follow-up spec or refactor. Item 6 required no change per audit's own assessment.

### Audit v2 (2026-03-20 18:30)
**Status:** APPROVED

**Context Estimate:** ~65% total

**Audit Dimensions:**
- Clarity: All requirements specify exact struct layouts, method signatures, and behavior. No vague terms.
- Completeness: All files listed (4 new + 1 modify = 5). Edge cases covered (OR-Map skip, non-Map skip, Nil handling).
- Testability: All 11 acceptance criteria are concrete and measurable.
- Scope: Well-bounded by 6 explicit constraints. Filter pushdown and distributed planner deferred.
- Feasibility: DataFusion TableProvider is the standard integration pattern. Approach is sound.
- Architecture fit: Uses existing ObserverFactory, MutationObserver, RecordStoreFactory, SchemaProvider traits. Feature-gated to avoid default build impact.
- Non-duplication: execute_query delegates to existing predicate engine. No wheel reinvention.
- Cognitive load: Four files with clear single responsibilities. Per-partition cache strategy well-documented.
- Strategic fit: Aligned with v2.0 roadmap (TODO-091, DataFusion SQL). Effort proportional to value.
- Project compliance: Feature-gated, no new runtime deps beyond what SPEC-135a added, no out-of-scope items.

**Rust Auditor Checklist:**
- [x] No f64 for integer-semantic fields (version is u64, partition_id is u32)
- [x] No r#type on message structs (no wire structs in this spec)
- [x] Default derived where needed (not applicable -- no payload structs with 2+ optional fields)
- [x] Enums for known value sets (not applicable)
- [x] Wire compatibility (not applicable -- in-process Arrow only, per constraint)
- [x] serde rename_all (not applicable -- no new serialized structs)
- [x] Option skip_serializing_if (not applicable)

**Language Profile:**
- File count: 5 (4 new + 1 modify) -- at limit, compliant
- Trait-first: G1 contains types only; traits (ObserverFactory, MutationObserver) exist from prior specs -- compliant
- Compilation gate: Largest group ~15% (3 files max) -- compliant

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~65% | <=50% | (warning) |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~15% | <=10% | (warning) |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | -- Current estimate |
| 70%+ | POOR | - |

The total context estimate of ~65% is in the DEGRADING range. However, each individual task group is well within the ~15% range (far below the 30% per-group threshold). The existing 5-group decomposition with 3 waves and parallel execution effectively mitigates the total context concern. Each worker will operate in the PEAK range (~15-20% including overhead).

**Assumptions Verified:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | DataFusion v45 + arrow v55 compatible | Build failure; Cargo.toml already has these versions from SPEC-135a |
| A2 | for_each_boxed provides all non-expired records | Missing rows in SQL results; verified via DefaultRecordStore impl |
| A3 | OR-Map records safely skippable | Acceptable data gap per design; OR-Map has no columnar representation |
| A4 | Single DataFusion partition sufficient | Correct for aggregated scan pattern |

**Project Compliance:** Honors PROJECT.md decisions. No new dependencies beyond SPEC-135a. Feature-gated. No out-of-scope items.

**v1 Critical Issues Resolution:** All 3 critical issues from v1 have been properly addressed in the revised spec text. Timestamp handling added to R2. Per-partition cache/merge strategy explicit in R3. Output partitioning specified.

**Recommendations:**
1. **R3: TopGunExec requires additional ExecutionPlan trait methods.** DataFusion's ExecutionPlan trait requires `schema()`, `children()`, `with_new_children()`, and `properties()` in addition to the `output_partitioning()` and `execute()` specified. These are straightforward for a leaf scan node (schema from provider, empty children, identity with_new_children). The implementer should consult DataFusion's MemoryExec as a reference implementation. Not critical since these are mechanical trait requirements.

2. **R2: ListBuilder inner type resolution.** The `DataType::List(_)` mapping mentions `ListBuilder` but does not specify how to resolve the inner builder type from the Arrow schema's list item DataType. The implementer must match on the inner field's DataType and construct the appropriate typed ListBuilder (e.g., `ListBuilder::new(Int64Builder::new())` for `List<Int64>`). This is implementation detail but worth noting.

**Comment:** Well-structured spec with clear decomposition. All v1 critical issues resolved. The per-partition caching strategy (option a) is the right choice for a partitioned data grid. Ready for parallel execution.

### Response v2 (2026-03-20)
**Applied:** both recommendations from Audit v2

**Changes:**
1. [✓] R3: TopGunExec additional ExecutionPlan trait methods — Added `schema()`, `children()`, `with_new_children()`, and `properties()` to the `TopGunExec` description in R3, with specified behavior for each (leaf node semantics). Added a note directing implementers to consult DataFusion's `MemoryExec` as a reference implementation.
2. [✓] R2: ListBuilder inner type resolution — Updated the `DataType::List(_)` bullet in R2 to specify that the inner builder type must match the list's inner field DataType (e.g., `ListBuilder::new(Int64Builder::new())` for `List<Int64>`), with a fallback to `ListBuilder::new(StringBuilder::new())` with JSON-serialized elements for unsupported or complex inner types.

### Audit v3 (2026-03-20 19:00)
**Status:** APPROVED

**Context Estimate:** ~65% total

**v2 Recommendations Resolution:** Both recommendations from Audit v2 have been applied to the spec text. R3 now includes all required ExecutionPlan trait methods with leaf-node semantics and a MemoryExec reference note. R2 now specifies ListBuilder inner type resolution with fallback behavior.

**Fresh-eyes review:** No new critical issues. The spec is thorough, well-structured, and ready for implementation. All prior audit issues have been addressed across two revision rounds.

**Comment:** Spec is clean after three audit rounds. All critical issues resolved, all recommendations applied. The 5-group decomposition with 3-wave parallel execution is well-suited for the ~65% total context -- each worker stays in the PEAK range (~15-20% including overhead).

---

## Review History

### Review v1 (2026-03-20)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Clippy lint violations in new files (40+ errors with `-D warnings`)**
   - Files: `arrow_cache.rs`, `arrow_convert.rs`, `table_provider.rs`, `datafusion_backend.rs`
   - Issue: `cargo clippy -p topgun-server --features datafusion -- -D warnings` fails with 41 errors introduced by this spec's new files. The Language Profile requires clippy-clean code (the PROJECT.md Language Profile specifies clippy as the lint command and all prior specs have maintained clippy-clean status). Violations include:
     - `arrow_cache.rs:45` — `map_name`/`partition_id` in doc comment need backticks (doc_markdown, 2 errors)
     - `arrow_cache.rs:73` — missing `# Errors` section on `get_or_build` (missing_errors_doc)
     - `arrow_cache.rs:124` — `.map(...).unwrap_or(0)` should be `.map_or(0, ...)` (map_unwrap_or)
     - `arrow_convert.rs` — multiple doc_markdown, missing_errors_doc, let-else suggestion, redundant closure, len-zero comparison, wildcard-after-explicit-pattern, too_many_lines (>100 lines in `append_value`)
     - `table_provider.rs` — multiple doc_markdown, missing_panics_doc on `.expect()` in `new()`, unnecessary_literal_bound on `fn name(&self) -> &str`
     - `datafusion_backend.rs` — multiple doc_markdown errors
   - Fix: Fix all clippy violations in the four new files. Most are mechanical: add backticks to doc comments, add `# Errors`/`# Panics` sections, use `map_or`, use `let...else`, use `is_empty()`, use `ColumnBuilder::finish` method reference, use `&'static str` return type on `fn name()`.

**Major:**
2. **TOCTOU race in `ArrowCacheManager::get_or_build`**
   - File: `packages/server-rust/src/service/domain/arrow_cache.rs:83-100`
   - Issue: `current_ver` is read at line 83, then the cache is checked at line 86, then `build_fn` is called at line 93, and the result is stored with the old `current_ver` at line 94-99. If `invalidate()` is called concurrently between line 83 and line 94, the fresh batch is stored with the version that was current before the invalidation, making it appear valid when it is actually stale. On the next read, `current_version` will return the post-invalidation version (e.g., 1), but the stored entry has version 0, so the cache miss will correctly trigger a rebuild. However, the batch built from the window between version read and store could be stored under a version that has already been superseded — meaning a subsequent read would hit the stale entry if no further invalidation has happened. This is a subtle but real correctness issue in concurrent usage.
   - Fix: Read `current_version` again after `build_fn` completes and only store the batch if the version hasn't changed (CAS pattern), or use a lock around the build+store. Alternatively, document as a known limitation for single-threaded callers.

**Minor:**
3. **`rmpv_to_json_string` does not escape special characters in strings**
   - File: `packages/server-rust/src/service/domain/arrow_convert.rs:324-326`
   - Issue: `format!("\"{}\"", s.as_str().unwrap_or(""))` does not escape embedded double-quotes, backslashes, or control characters. A string value like `he said "hello"` would produce invalid JSON `"he said "hello""`. This affects the JsonFallback column serialization path.
   - Fix: Use a proper JSON string escaper (e.g., `serde_json::to_string()` or manual escape of `"`, `\`, and control characters).

4. **`build_record_batch` early-return condition is over-complex and inconsistent**
   - File: `packages/server-rust/src/service/domain/arrow_convert.rs:84`
   - Issue: The condition `columns.is_empty() || (num_fields > 0 && columns[0].len() == 0 && entries.is_empty())` does not handle the case where all entries were non-Map and were skipped (columns exist but have 0 rows, yet `entries` is non-empty). `RecordBatch::try_new` handles empty arrays correctly, so the entire early-return block is unnecessary.
   - Fix: Remove the special-case early return entirely. `RecordBatch::try_new` with zero-length arrays produces a valid empty batch.

**Passed:**
- [✓] AC1: `select_all_returns_all_rows` test passes — `SELECT * FROM users` returns 3 rows correctly
- [✓] AC2: `select_with_where_filters_correctly` test passes — `WHERE age > 25` returns 2 rows
- [✓] AC3: `select_with_group_by_aggregates` test passes — `GROUP BY age` returns 2 distinct groups
- [✓] AC4: `table_provider_schema_matches` test passes — schema has `_key` first, non-nullable, Utf8
- [✓] AC5: `exec_supports_projection_pushdown` test passes — projected schema has only 2 columns
- [✓] AC6: `get_or_build_caches_batch` test passes — second call does not invoke build_fn
- [✓] AC7: `invalidate_increments_version_and_forces_rebuild` test passes — rebuild triggered after invalidate
- [✓] AC8: `observer_invalidates_on_mutation_events` test passes — all 6 events increment version; evict/load/destroy do not
- [✓] AC9: `build_record_batch_timestamp_column` test passes — timestamps stored as i64 millis in TimestampMillisecondArray
- [✓] AC10: `cargo build -p topgun-server` (no feature) compiles cleanly — verified, 0 errors
- [✓] AC11: `cargo build -p topgun-server --features datafusion` compiles cleanly — verified, 0 errors
- [✓] All 597 tests pass with `--features datafusion`
- [✓] R1 struct layout matches spec exactly — `CachedBatch`, `ArrowCacheManager` with correct field types
- [✓] R2 all value type mappings implemented — Int64, Float64, Utf8, Boolean, Binary, Timestamp, List (with inner type dispatch), JsonFallback
- [✓] R3 `TopGunExec` is a proper leaf node — `children()` empty, `with_new_children([])` succeeds, `with_new_children([x])` errors
- [✓] R3 per-partition caching (option a) correctly implemented — `get_or_build` called once per partition, `concat_batches` used to merge
- [✓] R4 `execute_query` delegates to predicate engine — verified in `execute_query_delegates_to_predicate_engine` test
- [✓] R4 `register_map` returns `SchemaRequired` when no schema — verified in `register_map_without_schema_returns_error` test
- [✓] R5 all four modules registered with `#[cfg(feature = "datafusion")]` in mod.rs — matches spec exactly
- [✓] No Arrow IPC over wire — constraint honored, Arrow is in-process only
- [✓] PredicateEngine not modified — constraint honored
- [✓] DataFusion not added to core-rust — constraint honored
- [✓] 32 new tests added across 4 new modules

**Summary:** All acceptance criteria pass and all functional requirements are met. The implementation is blocked by a clippy failure (41 errors) that violates the project's established quality standard of clippy-clean code. The major TOCTOU issue in `get_or_build` is a real correctness concern under concurrent write+query load, and the JSON string escaping issue could produce invalid output for string fields with special characters. These should be fixed before marking done.

### Fix Response v1 (2026-03-20)
**Applied:** all (issues 1-4)

**Fixes:**
1. [✓] Clippy lint violations (41 errors → 0) — Fixed all doc_markdown (backticks), missing_errors_doc, missing_panics_doc, map_unwrap_or→map_or, let-else, is_empty, method reference, &'static str, wildcard_in_or_patterns, extracted 4 list-append helpers to fix too_many_lines in `append_value`
   - Commit: 302c20c
2. [✓] TOCTOU race in `ArrowCacheManager::get_or_build` — Added CAS pattern: re-reads current_version after build_fn completes, only caches if version unchanged
   - Commit: 302c20c
3. [✓] `rmpv_to_json_string` string escaping — Replaced `format!("\"{}\"", ...)` with `serde_json::to_string()` for proper JSON escaping
   - Commit: 302c20c
4. [✓] Unnecessary early-return in `build_record_batch` — Removed over-complex condition; `RecordBatch::try_new` handles empty arrays correctly
   - Commit: 302c20c

**Verification:** Clippy 0 warnings, 597 tests pass.

### Review v2 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**All v1 issues resolved:**
- [✓] Issue 1 (Clippy 41 errors) — `cargo clippy -p topgun-server --features datafusion -- -D warnings` exits 0, 0 warnings. Verified.
- [✓] Issue 2 (TOCTOU race) — CAS pattern present at `arrow_cache.rs:103-112`: `version_after_build` is read after `build_fn()` completes and compared to `current_ver` before inserting into the cache. A concurrent invalidation between the version read and the store will prevent a stale batch from being cached.
- [✓] Issue 3 (JSON string escaping) — `rmpv_to_json_string` now uses `serde_json::to_string(s.as_str().unwrap_or(""))` at `arrow_convert.rs:331`. The old `format!("\"{}\"", ...)` pattern is gone.
- [✓] Issue 4 (over-complex early-return) — The early-return block is entirely absent. `build_record_batch` proceeds directly to builder iteration and calls `RecordBatch::try_new` unconditionally.

**Minor:**
1. **`rmpv_to_json_string` map key is not escaped**
   - File: `packages/server-rust/src/service/domain/arrow_convert.rs:344-348`
   - Issue: In the `rmpv::Value::Map` branch, string keys are extracted as `s.as_str().unwrap_or("").to_string()` and then interpolated directly into the JSON output via `format!("\"{}\":{}", key_str, ...)`. If a map key contains `"`, `\`, or control characters the result is malformed JSON. This affects only the `JsonFallback` serialization path (nested map values, not primary data columns), so it has no impact on SQL query results and no data loss risk.
   - Fix: Apply `serde_json::to_string(&key_str).unwrap_or_default()` for the key — the same fix already applied to string values on line 331.

**Passed:**
- [✓] AC1-AC3: SQL SELECT / WHERE / GROUP BY tests pass — 597 tests, 0 failures
- [✓] AC4: `TopGunTableProvider::schema()` returns `_key` first, Utf8, non-nullable
- [✓] AC5: Projection pushdown — `exec_supports_projection_pushdown` passes; projected schema has 2 columns
- [✓] AC6: Cache hit on repeated get_or_build — `get_or_build_caches_batch` passes; build_fn not called twice
- [✓] AC7: Invalidation forces rebuild — `invalidate_increments_version_and_forces_rebuild` passes
- [✓] AC8: Observer fires on all 6 mutation events, not on evict/load/destroy — `observer_invalidates_on_mutation_events` passes
- [✓] AC9: Timestamp column — `build_record_batch_timestamp_column` passes; i64 millis in `TimestampMillisecondArray`
- [✓] AC10: `cargo build -p topgun-server` (no datafusion feature) — compiles cleanly, 0 errors
- [✓] AC11: `cargo build -p topgun-server --features datafusion` — compiles cleanly, 0 errors
- [✓] Build check passes — `cargo build -p topgun-server` exits 0
- [✓] Lint check passes — `cargo clippy -p topgun-server --features datafusion -- -D warnings` exits 0, 0 warnings
- [✓] Test check passes — 597 tests pass, 0 failures
- [✓] CAS pattern properly guards against stale-batch storage under concurrent invalidation
- [✓] Per-partition caching (option a) implemented correctly — `get_or_build` called once per partition, `concat_batches` merges
- [✓] `execute_query` delegates to predicate engine, not DataFusion — constraint honored
- [✓] No Arrow IPC over wire — constraint honored
- [✓] PredicateEngine not modified — constraint honored
- [✓] DataFusion not added to core-rust — constraint honored
- [✓] All 4 new modules registered with `#[cfg(feature = "datafusion")]` in `mod.rs`
- [✓] `Default` impl on `ArrowCacheManager` delegates to `new()` — idiomatic
- [✓] `fmt::Debug` impl on `ArrowCacheManager` avoids exposing internal DashMap contents — correct
- [✓] `TopGunExec` is `#[derive(Clone)]` — required by `with_new_children` returning `Ok(self)`
- [✓] `PlanProperties` uses `EmissionType::Incremental` and `Boundedness::Bounded` — correct for a scan
- [✓] No `unwrap()` in production paths — `expect("valid projection")` in `TopGunExec::new` is documented with `# Panics`

**Summary:** All three v1 issues (clippy, TOCTOU, JSON escaping) plus the over-complex early-return are confirmed fixed. The implementation is correct, well-tested, and clippy-clean. One minor issue remains in `rmpv_to_json_string` where map keys are not escaped — this affects only the JsonFallback path for nested map values, not primary data columns, and carries no data loss risk.

### Fix Response v2 (2026-03-21)
**Applied:** issue 1 (map key escaping)

**Fixes:**
1. [✓] `rmpv_to_json_string` map key escaping — Used `serde_json::to_string()` for map keys, consistent with string value fix on line 331
   - Commit: c2e970c

**Verification:** Clippy 0 warnings, 601 tests pass.

### Review v3 (2026-03-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**v2 Minor Issue Verified Fixed:**
- [✓] Issue 1 (map key escaping in `rmpv_to_json_string`) — `arrow_convert.rs:345-349`: String map keys now use `serde_json::to_string(s.as_str().unwrap_or("")).unwrap_or_default()`. The `_ =>` branch delegates to `rmpv_to_json_string(k)` which also uses `serde_json`. The raw `format!("\"{}\"", ...)` pattern is absent. The fix is consistent with the string value fix on line 331.

**Language Profile Checks:**
- [✓] Build check (no feature): `cargo build -p topgun-server` — exits 0
- [✓] Build check (with feature): `cargo build -p topgun-server --features datafusion` — exits 0, compiles in 9s
- [✓] Lint check: `cargo clippy -p topgun-server --features datafusion -- -D warnings` — exits 0, 0 warnings
- [✓] Test check: `cargo test --release -p topgun-server --features datafusion` — 602 tests pass, 0 failures

**Acceptance Criteria:**
- [✓] AC1: `SELECT * FROM users` returns all 3 rows — `select_all_returns_all_rows` passes
- [✓] AC2: `SELECT ... WHERE age > 25` filters to 2 rows — `select_with_where_filters_correctly` passes
- [✓] AC3: `SELECT age, COUNT(*) GROUP BY age` aggregates to 2 groups — `select_with_group_by_aggregates` passes
- [✓] AC4: `TopGunTableProvider::schema()` returns `_key` first (Utf8, non-nullable) — `table_provider_schema_matches` passes
- [✓] AC5: Projection pushdown — `exec_supports_projection_pushdown` passes; 2-column projected schema confirmed
- [✓] AC6: Cache hit on repeat — `get_or_build_caches_batch` passes; second call skips build_fn
- [✓] AC7: Invalidation forces rebuild — `invalidate_increments_version_and_forces_rebuild` passes
- [✓] AC8: Observer on 6 mutation events, not on evict/load/destroy — `observer_invalidates_on_mutation_events` passes
- [✓] AC9: All value variants convert correctly including Timestamp — `build_record_batch_timestamp_column` passes
- [✓] AC10: `cargo build -p topgun-server` (no feature) compiles cleanly
- [✓] AC11: `cargo build -p topgun-server --features datafusion` compiles all 4 modules

**Quality:**
- [✓] No `unwrap()` in production paths — `expect("valid projection")` in `TopGunExec::new` documented with `# Panics`
- [✓] CAS pattern guards stale-batch storage — version re-read after `build_fn` at `arrow_cache.rs:103`
- [✓] `Default` impl on `ArrowCacheManager` delegates to `new()` — idiomatic
- [✓] Rust idioms: `map_or`, `let...else`, `is_empty()`, `&'static str` on `fn name()` — all correct
- [✓] `#[cfg(feature = "datafusion")]` on all 4 modules in `mod.rs` — feature gate correct
- [✓] Constraints honored: no Arrow IPC over wire, no DataFusion in core-rust, PredicateEngine untouched

**Summary:** The v2 minor issue (map key escaping) is confirmed fixed via `serde_json::to_string()`. All 11 acceptance criteria pass. Build, lint, and tests are clean (602 tests, 0 failures, 0 clippy warnings). The implementation is correct, well-tested, and production-ready.

---

## Completion

**Completed:** 2026-03-21
**Total Commits:** 5 (ed56f6f, dfa63f4, 39e0e1d, 302c20c, c2e970c)
**Review Cycles:** 3

### Outcome

Implemented the DataFusion SQL query engine behind the `datafusion` feature flag: ArrowCacheManager (lazy per-partition cache with version-based invalidation via CAS), MsgPack-to-Arrow conversion for all value types, TopGunTableProvider with projection pushdown, and DataFusionBackend implementing both QueryBackend and SqlQueryBackend. 32 new tests, 602 total passing.

### Key Files

- `packages/server-rust/src/service/domain/arrow_cache.rs` — Lazy Arrow cache with per-partition invalidation via MutationObserver
- `packages/server-rust/src/service/domain/arrow_convert.rs` — MsgPack-to-Arrow batch conversion (all value types including List, Timestamp)
- `packages/server-rust/src/service/domain/table_provider.rs` — DataFusion TableProvider + ExecutionPlan with projection pushdown
- `packages/server-rust/src/service/domain/datafusion_backend.rs` — Wires SessionContext, implements QueryBackend + SqlQueryBackend

### Patterns Established

- CAS pattern for concurrent cache invalidation (read version before build, re-read after, only store if unchanged)
- Per-partition Arrow caching with concat_batches aggregation at scan time
- Feature-gated DataFusion integration (`#[cfg(feature = "datafusion")]`) keeping default builds unaffected

### Deviations

None — implemented as specified.
