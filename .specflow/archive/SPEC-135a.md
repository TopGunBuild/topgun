---
id: SPEC-135a
type: feature
status: done
priority: P1
complexity: small
parent: SPEC-135
depends_on: []
created: 2026-03-20
source: TODO-091
---

# DataFusion SQL: Traits, Types, Wire Messages

## Context

TopGun currently uses a `PredicateEngine` for query evaluation -- a pure-function L1-L2 predicate evaluator operating on `rmpv::Value`. This covers ~90% of single-Map queries (Eq, Neq, Gt, Lt, And, Or, Not) but cannot support SQL, JOINs, GROUP BY, aggregations, or cross-map queries.

This sub-spec establishes the trait boundary (`QueryBackend` / `SqlQueryBackend`), the error type, the default `PredicateBackend` implementation, the wire protocol messages for SQL queries, and the Cargo feature flag. It is the foundation that SPEC-135b (DataFusion implementation) and SPEC-135c (QueryService integration) build upon.

**Dependencies satisfied:** TODO-069 (Schema with Arrow types) is complete via SPEC-127-130. `MapSchema::to_arrow_schema()` and the `arrow` feature flag on core-rust are already in place.

**Key audit fixes applied:**
- `QueryBackend` is split into a base trait (always available) and `SqlQueryBackend` (feature-gated behind `datafusion`) per audit issue 4.
- `SqlQueryRespPayload` omits redundant `row_count` field per audit issue 3.
- Wire message structs include full serde annotations per audit issue 2.
- `SqlQueryRespPayload.rows` uses `rmpv::Value` per audit issue 7.
- DistributedPlanner is fully deferred per audit issue 5.

## Task

Define the `QueryBackend` trait (always available), `SqlQueryBackend` trait (feature-gated), `QueryBackendError` enum, `PredicateBackend` implementation, `SqlQuery`/`SqlQueryResp` wire protocol messages, and the `datafusion` Cargo feature flag.

## Requirements

### R1: QueryBackend Trait (always available)

**File:** `packages/server-rust/src/service/domain/query_backend.rs` (new)

Define a base `QueryBackend` trait that abstracts predicate-based query execution. This trait is always compiled (no feature gate):

```rust
#[async_trait]
pub trait QueryBackend: Send + Sync {
    /// Execute a predicate-based query (existing Query struct).
    /// Returns filtered results as QueryResultEntry vec (backward compat).
    async fn execute_query(
        &self,
        map_name: &str,
        entries: Vec<(String, rmpv::Value)>,
        query: &Query,
    ) -> Result<Vec<QueryResultEntry>, QueryBackendError>;

    /// Register a map as a queryable table.
    async fn register_map(&self, map_name: &str) -> Result<(), QueryBackendError>;

    /// Deregister a map (on map destroy).
    async fn deregister_map(&self, map_name: &str) -> Result<(), QueryBackendError>;
}
```

### R2: SqlQueryBackend Trait (feature-gated)

**File:** `packages/server-rust/src/service/domain/query_backend.rs` (same file, behind `#[cfg(feature = "datafusion")]`)

```rust
#[cfg(feature = "datafusion")]
#[async_trait]
pub trait SqlQueryBackend: QueryBackend {
    /// Execute a SQL query string, returning Arrow RecordBatches.
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, QueryBackendError>;
}
```

This uses `arrow::array::RecordBatch` which is only available under the `datafusion` feature.

### R3: QueryBackendError Enum

**File:** `packages/server-rust/src/service/domain/query_backend.rs` (same file)

```rust
#[derive(Debug)]
pub enum QueryBackendError {
    /// SQL parsing or syntax error.
    SqlParse(String),
    /// Execution error during query processing.
    Execution(String),
    /// Map requires a schema for SQL queries but none is registered.
    SchemaRequired(String),
    /// Internal error wrapping anyhow.
    Internal(anyhow::Error),
}

impl std::fmt::Display for QueryBackendError { ... }
impl std::error::Error for QueryBackendError { ... }
```

### R4: PredicateBackend (default, no feature gate)

**File:** `packages/server-rust/src/service/domain/query_backend.rs` (same file)

`PredicateBackend` implements `QueryBackend`:
- `execute_query()` delegates to existing `predicate::execute_query()`. The `map_name` parameter is ignored by `PredicateBackend` -- it exists on the trait so the future SQL backend (DataFusionBackend) can identify which table to query. `PredicateBackend` operates purely on the provided `entries` slice.
- `register_map()` / `deregister_map()` are no-ops returning `Ok(())`.

`PredicateBackend` is a zero-field struct (unit-like or empty struct). It does NOT implement `SqlQueryBackend`.

### R5: SqlQuery / SqlQueryResp Wire Messages

**File:** `packages/core-rust/src/messages/query.rs` (modify)

Add two new message payload structs after the existing `QueryRespMessage`:

```rust
/// Payload for a SQL query request from client to server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryPayload {
    /// SQL query string to execute.
    pub sql: String,
    /// Unique identifier for correlating request/response.
    pub query_id: String,
}

/// Payload for a SQL query response from server to client.
///
/// Results are serialized as rows of `rmpv::Value` (not Arrow IPC)
/// for cross-language client compatibility.
/// On error, `rows` and `columns` are empty and `error` contains a description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryRespPayload {
    /// Identifier correlating to the request.
    pub query_id: String,
    /// Column names in result order.
    pub columns: Vec<String>,
    /// Row data: each inner Vec corresponds to one row, values ordered by `columns`.
    pub rows: Vec<Vec<rmpv::Value>>,
    /// Error message if the query failed; `None` on success.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}
```

Note: `row_count` is intentionally omitted -- `rows.len()` provides the count.

Add corresponding `Message` enum variants:

```rust
/// SQL query request from client.
#[serde(rename = "SQL_QUERY")]
SqlQuery { payload: SqlQueryPayload },

/// SQL query response from server.
#[serde(rename = "SQL_QUERY_RESP")]
SqlQueryResp { payload: SqlQueryRespPayload },
```

**File:** `packages/core-rust/src/messages/mod.rs` (modify)

Add `SqlQueryPayload` and `SqlQueryRespPayload` to the existing `pub use query::{...}` re-export block so both types are available at the `messages::` level, consistent with all other message types in the codebase.

Add roundtrip and camelCase tests for both message types. Include one test case where `error` is `Some(...)` and `rows`/`columns` are empty (error path), and one where `error` is `None` (success path).

### R6: Cargo Feature Flag

**File:** `packages/server-rust/Cargo.toml` (modify)

Add to `[features]`:
```toml
datafusion = ["dep:datafusion", "dep:arrow"]
```

Add to `[dependencies]`:
```toml
datafusion = { version = "45", optional = true }
arrow = { version = "55", optional = true }
```

When `datafusion` feature is disabled, only `QueryBackend`, `QueryBackendError`, and `PredicateBackend` are compiled from `query_backend.rs`. The `SqlQueryBackend` trait is behind `#[cfg(feature = "datafusion")]`.

### R7: Module Registration

**File:** `packages/server-rust/src/service/domain/mod.rs` (modify)

Add:
```rust
pub mod query_backend;
```

## Acceptance Criteria

1. `QueryBackend` trait compiles without `datafusion` feature -- has `execute_query`, `register_map`, `deregister_map` methods only
2. `SqlQueryBackend` trait compiles only with `datafusion` feature -- has `execute_sql` returning `Vec<RecordBatch>`
3. `PredicateBackend` implements `QueryBackend` and passes all existing query tests unchanged (backward compatible)
4. `PredicateBackend` does NOT implement `SqlQueryBackend`
5. `SqlQueryPayload` and `SqlQueryRespPayload` serialize/deserialize correctly via MsgPack roundtrip
6. `SqlQueryRespPayload` has no `row_count` field
7. `Message::SqlQuery` and `Message::SqlQueryResp` variants exist with correct serde rename
8. Building with `cargo build -p topgun-server` (no datafusion feature) compiles successfully
9. Building with `cargo build -p topgun-server --features datafusion` compiles the `SqlQueryBackend` trait
10. Existing query tests (predicate-based) pass without modification
11. `SqlQueryRespPayload.error` is `Some(String)` when a query fails and `None` on success; both cases roundtrip correctly via MsgPack
12. `SqlQueryPayload` and `SqlQueryRespPayload` are re-exported from `messages::` via the `pub use query::{...}` block in `mod.rs`

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all existing 509+ tests pass
2. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-core` -- all core tests pass including new message roundtrip tests
3. Run `cargo build -p topgun-server` (no features) -- compiles, no datafusion symbols
4. Run `cargo build -p topgun-server --features datafusion` -- compiles with SqlQueryBackend

## Constraints

- DO NOT remove or modify PredicateEngine behavior -- it remains the default backend
- DO NOT make DataFusion a required dependency -- it must be feature-gated
- DO NOT add DataFusion to core-rust -- it belongs in server-rust only
- DO NOT add `row_count` to `SqlQueryRespPayload` -- `rows.len()` provides this
- `SqlQueryRespPayload.rows` uses `rmpv::Value` (matching existing wire protocol structs)

## Assumptions

- DataFusion v45 is the target version (latest stable as of March 2026, compatible with arrow v55)
- `arrow` crate v55 aligns with the `arrow-schema` v55 already in core-rust
- SchemaProvider must have a schema registered for a map before it can be queried via SQL; schemaless maps return `QueryBackendError::SchemaRequired`

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | `QueryBackend` trait, `SqlQueryBackend` trait (cfg-gated), `QueryBackendError` enum, `PredicateBackend` struct (types only) | -- | ~10% |
| G2 | 2 | `PredicateBackend` impl (delegates to predicate::execute_query), tests | G1 | ~10% |
| G3 | 2 | `SqlQueryPayload`, `SqlQueryRespPayload` structs, `Message` enum variants, mod.rs re-export update, roundtrip tests | G1 | ~10% |
| G4 | 2 | Cargo.toml feature flag + optional deps, mod.rs registration | G1 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4 | Yes | 3 |

**Total workers needed:** 3 (max in any wave)

## Audit History

### Audit v1 (2026-03-20)
**Status:** APPROVED

**Context Estimate:** ~35% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~35% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Rust Type Mapping Compliance:**
- No f64 for integer-semantic fields: OK
- No r#type on message structs: OK
- Default not needed (no structs with 2+ optional fields): OK
- Enums for known value sets: OK
- serde(rename_all = "camelCase") on all structs: OK
- No Option fields requiring skip_serializing_if: OK

**Language Profile:** Compliant with Rust profile (4 files <= 5 max, trait-first ordering correct)

**Project Compliance:** Honors PROJECT.md decisions (MsgPack wire format, feature-gated DataFusion, no DataFusion in core-rust)

**Strategic Fit:** Aligned with project goals (TODO-091 DataFusion SQL on roadmap, trait-first approach establishes clean boundary for SPEC-135b/c)

**Comment:** Well-structured spec with clear trait boundary between always-available predicate queries and feature-gated SQL. Code snippets are precise and match existing codebase patterns. Task groups respect trait-first ordering. The `PredicateBackend` delegation to existing `predicate::execute_query()` ensures backward compatibility without behavioral changes.

**Recommendations:**
1. [Strategic] Consider defining an `SQL_QUERY_ERROR` wire message (or an `error: Option<String>` field on `SqlQueryRespPayload`) in this spec rather than deferring to SPEC-135c. Request/response patterns with `query_id` correlation need an error path for client-side error handling. Without it, SPEC-135c will need to either add a new message type (wire protocol change) or retrofit this payload.
2. The `QueryBackend::execute_query()` trait method includes `map_name: &str` but the underlying `predicate::execute_query()` does not use it. Add a brief note in R4 that `PredicateBackend` ignores `map_name` (it exists for the future SQL backend which needs table context).

### Response v1 (2026-03-20)
**Applied:** both recommendations

**Changes:**
1. [✓] Add `error: Option<String>` to `SqlQueryRespPayload` — added field with `#[serde(skip_serializing_if = "Option::is_none")]`, updated doc comment to describe error-path semantics, added acceptance criterion 11, and added test requirement covering both success and error roundtrip cases.
2. [✓] Note in R4 that `PredicateBackend` ignores `map_name` — added explanatory note clarifying that `map_name` is unused by `PredicateBackend` but exists on the trait for the future `DataFusionBackend` which needs table context.

### Audit v2 (2026-03-20 re-audit)
**Status:** NEEDS_REVISION

**Context Estimate:** ~35% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~35% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Rust Type Mapping Compliance:**
- No f64 for integer-semantic fields: OK
- No r#type on message structs: OK
- Default not needed (no structs with 2+ optional fields): OK
- Enums for known value sets: OK
- serde(rename_all = "camelCase") on all structs: OK
- skip_serializing_if + default on Option fields: VIOLATION (see Critical 1)

**Language Profile:** Compliant with Rust profile (4 files <= 5 max, trait-first ordering correct)

**Project Compliance:** 1 violation found (see Critical 1)

**Strategic Fit:** Aligned with project goals

**Critical:**
1. `SqlQueryRespPayload.error` field uses `#[serde(skip_serializing_if = "Option::is_none")]` but is missing `default`. PROJECT.md mandates `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>`. Without `default`, deserialization will fail when the `error` field is absent from the wire payload (the success case, where `skip_serializing_if` omits it). Every existing `Option<T>` field in the codebase (e.g., `QueryRespPayload.next_cursor`, `.has_more`, `.cursor_status`) uses both annotations. Fix: change to `#[serde(skip_serializing_if = "Option::is_none", default)]`.

**Recommendations:**
2. R5 adds `SqlQueryPayload` and `SqlQueryRespPayload` to `core-rust/src/messages/query.rs` but does not mention updating the `pub use query::` re-export block in `core-rust/src/messages/mod.rs`. All other message types in the codebase are re-exported at the `messages::` level. Add `SqlQueryPayload, SqlQueryRespPayload` to the existing `pub use query::{...}` block for consistency.

### Response v2 (2026-03-20)
**Applied:** all items (Critical 1 + Recommendation 2)

**Changes:**
1. [✓] Add `default` to `SqlQueryRespPayload.error` serde annotation — changed `#[serde(skip_serializing_if = "Option::is_none")]` to `#[serde(skip_serializing_if = "Option::is_none", default)]` in R5 code snippet, satisfying the PROJECT.md mandate for all `Option<T>` fields.
2. [✓] Add mod.rs re-export instruction to R5 — added a paragraph in R5 directing the implementer to add `SqlQueryPayload` and `SqlQueryRespPayload` to the `pub use query::{...}` re-export block in `core-rust/src/messages/mod.rs`; also updated G3 task description to include "mod.rs re-export update"; added acceptance criterion 12 to verify the re-export is present.

### Audit v3 (2026-03-20 re-audit)
**Status:** APPROVED

**Context Estimate:** ~35% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~35% | <=50% | OK |
| Largest task group | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** GOOD range (30-50%)

**Rust Type Mapping Compliance:**
- No f64 for integer-semantic fields: OK
- No r#type on message structs: OK
- Default not needed (no structs with 2+ optional fields): OK
- Enums for known value sets: OK
- serde(rename_all = "camelCase") on all structs: OK
- skip_serializing_if + default on Option fields: OK (error field now has both)

**Language Profile:** Compliant with Rust profile (4 files <= 5 max, trait-first ordering correct)

**Project Compliance:** Honors PROJECT.md decisions (MsgPack wire format, feature-gated DataFusion, no DataFusion in core-rust, skip_serializing_if + default on all Option fields)

**Strategic Fit:** Aligned with project goals (TODO-091 DataFusion SQL on roadmap, trait-first approach establishes clean boundary for SPEC-135b/c)

**Comment:** All previous critical issues and recommendations have been addressed. The spec is clean and implementable: trait boundary is well-defined, wire messages follow existing codebase patterns exactly (camelCase, roundtrip tests, Option serde annotations), Cargo feature gating is correct, and the PredicateBackend delegation preserves backward compatibility. File count (4 unique files) is within the Rust language profile limit of 5.

## Execution Summary

**Executed:** 2026-03-20
**Mode:** orchestrated
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3, G4 | complete |

### Files Created
- `packages/server-rust/src/service/domain/query_backend.rs`

### Files Modified
- `packages/core-rust/src/messages/query.rs` (added SqlQueryPayload, SqlQueryRespPayload)
- `packages/core-rust/src/messages/mod.rs` (added Message variants, re-exports)
- `packages/server-rust/Cargo.toml` (added datafusion feature flag, optional deps)
- `packages/server-rust/src/service/domain/mod.rs` (registered query_backend module)
- `packages/server-rust/src/service/classify.rs` (handled new Message variants)

### Acceptance Criteria Status
- [x] 1. QueryBackend trait compiles without datafusion feature
- [x] 2. SqlQueryBackend trait compiles only with datafusion feature
- [x] 3. PredicateBackend implements QueryBackend, passes all query tests (backward compatible)
- [x] 4. PredicateBackend does NOT implement SqlQueryBackend
- [x] 5. SqlQueryPayload and SqlQueryRespPayload MsgPack roundtrip correctly
- [x] 6. SqlQueryRespPayload has no row_count field
- [x] 7. Message::SqlQuery and Message::SqlQueryResp variants exist with correct serde rename
- [x] 8. cargo build -p topgun-server (no datafusion feature) compiles successfully
- [x] 9. cargo build -p topgun-server --features datafusion compiles SqlQueryBackend trait
- [x] 10. Existing query tests pass without modification (565 server, 431+ core)
- [x] 11. SqlQueryRespPayload.error roundtrips correctly for both Some and None cases
- [x] 12. SqlQueryPayload and SqlQueryRespPayload re-exported from messages::mod.rs

### Deviations
- Added SqlQuery/SqlQueryResp handling in classify.rs (not in spec but required by exhaustive match on Message enum). SqlQuery returns ServerToClient error until SPEC-135c wires it. SqlQueryResp returns ServerToClient error (server-to-client only).

---

## Review History

### Review v1 (2026-03-20)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: QueryBackend trait compiles without datafusion feature — verified via `cargo build -p topgun-server` (no features), exit 0
- [✓] AC2: SqlQueryBackend trait compiles only with datafusion feature — `#[cfg(feature = "datafusion")]` correctly gates the trait; `cargo build --features datafusion` succeeds
- [✓] AC3: PredicateBackend implements QueryBackend and delegates to predicate::execute_query — 565 server tests pass, backward compatible
- [✓] AC4: PredicateBackend does NOT implement SqlQueryBackend — no impl block present
- [✓] AC5: SqlQueryPayload and SqlQueryRespPayload MsgPack roundtrip — success-path and error-path tests present and passing
- [✓] AC6: SqlQueryRespPayload has no row_count field — confirmed by grep
- [✓] AC7: Message::SqlQuery and Message::SqlQueryResp variants with correct serde renames ("SQL_QUERY", "SQL_QUERY_RESP") — verified in mod.rs
- [✓] AC8: cargo build -p topgun-server (no features) compiles successfully — verified
- [✓] AC9: cargo build -p topgun-server --features datafusion compiles SqlQueryBackend — verified
- [✓] AC10: 565 server tests + 477 core tests pass, 0 failures
- [✓] AC11: SqlQueryRespPayload.error roundtrips for Some and None; error field omitted from wire when None (byte-level test present)
- [✓] AC12: SqlQueryPayload and SqlQueryRespPayload re-exported from messages::mod.rs pub use query block
- [✓] PROJECT.md rule: #[serde(skip_serializing_if = "Option::is_none", default)] on SqlQueryRespPayload.error — compliant
- [✓] PROJECT.md rule: serde(rename_all = "camelCase") on both new structs — compliant
- [✓] No row_count field on SqlQueryRespPayload — constraint honored
- [✓] Cargo.toml feature gate: datafusion = ["dep:datafusion", "dep:arrow"] — matches spec exactly
- [✓] Module registration: pub mod query_backend in domain/mod.rs — present
- [✓] Build check: exit 0 (both with and without datafusion feature)
- [✓] Lint (clippy -D warnings): exit 0, clean
- [✓] Test suite: all 565 server + 477 core tests pass
- [✓] classify.rs deviation is documented and reasonable — SqlQuery uses ClassifyError::ServerToClient as a temporary placeholder pending SPEC-135c; this is noted in Deviations

**Summary:** All 12 acceptance criteria are met. Both builds (with and without datafusion feature) succeed. All tests pass. The implementation matches the specification exactly: trait boundary, wire messages, serde annotations, feature gating, module registration, and re-exports are all correct. The classify.rs deviation (using ServerToClient error for SqlQuery as a placeholder) is a reasonable and documented temporary measure required by Rust's exhaustive match.

---

## Completion

**Completed:** 2026-03-20
**Total Commits:** 3
**Review Cycles:** 1

### Outcome

Established the QueryBackend trait boundary, PredicateBackend default implementation, SQL wire messages (SqlQuery/SqlQueryResp), and the `datafusion` Cargo feature flag — the foundation for SPEC-135b (DataFusion engine) and SPEC-135c (QueryService integration).

### Key Files

- `packages/server-rust/src/service/domain/query_backend.rs` — QueryBackend/SqlQueryBackend traits, QueryBackendError, PredicateBackend impl
- `packages/core-rust/src/messages/query.rs` — SqlQueryPayload, SqlQueryRespPayload wire message structs
- `packages/core-rust/src/messages/mod.rs` — Message::SqlQuery/SqlQueryResp enum variants and re-exports

### Patterns Established

- Feature-gated trait extension: base trait (always available) + extended trait behind `#[cfg(feature = "...")]` for optional capabilities
- Wire message error convention: `error: Option<String>` field on response payloads for inline error reporting (vs separate error message types)

### Deviations

- Added SqlQuery/SqlQueryResp handling in classify.rs (placeholder ServerToClient error until SPEC-135c wires the SQL_QUERY operation)
