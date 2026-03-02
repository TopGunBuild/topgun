---
id: SPEC-067
type: feature
status: done
priority: P0
complexity: medium
created: 2026-02-27
todo: TODO-090
---

# SPEC-067: PostgreSQL MapDataStore Adapter

## Context

TopGun's storage architecture has three layers: Layer 1 (in-memory `HashMapStorage`), Layer 2 (`DefaultRecordStore` orchestration), and Layer 3 (`MapDataStore` external persistence). Currently the only Layer 3 implementation is `NullDataStore`, which discards all writes. For v1.0, TopGun needs durable persistence so data survives server restarts.

The `MapDataStore` trait (TODO-067, complete) defines the contract. This spec implements the first real persistence backend: PostgreSQL via `sqlx`. The adapter uses **write-through** mode -- every `add()` and `remove()` call persists synchronously before returning. On startup, `load_all()` enables bulk cache warm-up from PostgreSQL.

### Reference Implementations

- **TS behavioral reference:** `packages/server/src/storage/PostgresAdapter.ts` -- uses `pg` Pool, stores LWW and OR-Map records in a single table with composite `(map_name, key)` primary key. Uses JSONB for values and an `ORMAP_MARKER` sentinel in `ts_node_id` to distinguish record types.
- **Hazelcast architectural reference:** `map/impl/MapStoreWrapper.java` -- write-through path where every map mutation calls through to the configured `MapStore` implementation synchronously.

### Design Decisions (vs TS)

1. **BYTEA with MsgPack, not JSONB:** The Rust `RecordValue` is already `Serialize`/`Deserialize` with `rmp-serde`. Storing as BYTEA avoids JSON round-trip overhead and preserves exact binary fidelity. The TS adapter uses JSONB because JavaScript naturally works with JSON; Rust has no such affinity.

2. **Expiration column:** The `MapDataStore` trait passes `expiration_time` on `add()`. The TS adapter ignores this. This spec stores it as `expiration_time BIGINT` for future server-side expiry queries (e.g., cleanup of expired records on startup).

3. **No `is_deleted` column:** The TS adapter stores tombstones as rows with `is_deleted = true`. In the Rust storage model, `remove()` deletes the row from PostgreSQL entirely. Tombstone semantics are handled by the CRDT layer above, not the persistence layer.

4. **Backup records:** The trait has `add_backup()` and `remove_backup()` for replica data. These use the same table with an `is_backup BOOLEAN` column to distinguish primary from backup records.

## Task

Implement `PostgresDataStore` as a `MapDataStore` trait implementation backed by PostgreSQL via `sqlx::PgPool`. Write-through mode: all mutations are persisted synchronously. Connection pooling handles concurrency. Schema auto-migration on construction.

## Goal Analysis

**Goal Statement:** Server data persists across restarts via PostgreSQL, with zero code changes to the `RecordStore` or service layers.

**Observable Truths:**
1. A `PostgresDataStore` struct exists and implements all 14 methods of the `MapDataStore` trait
2. Records written via `add()` are retrievable via `load()` after a server restart (round-trip through PostgreSQL)
3. `load_all()` retrieves multiple records in a single SQL query for cache warm-up
4. Connection pooling via `PgPool` allows concurrent access without connection exhaustion
5. Schema migration creates the `topgun_maps` table and indices on first call to `initialize()`
6. `RecordStoreFactory::new()` accepts `Arc<PostgresDataStore>` as the `data_store` parameter without modification

**Required Artifacts:**
- `packages/server-rust/src/storage/datastores/postgres.rs` -- the implementation
- `packages/server-rust/src/storage/datastores/mod.rs` -- re-export
- `packages/server-rust/Cargo.toml` -- sqlx dependency

**Key Links:**
- `RecordValue` serialization (rmp-serde to BYTEA) must round-trip exactly
- `PgPool` lifetime must outlive all `RecordStore` instances that hold `Arc<PostgresDataStore>`

## Requirements

### Files to Create

#### 1. `packages/server-rust/src/storage/datastores/postgres.rs`

**Struct: `PostgresDataStore`**
```rust
pub struct PostgresDataStore {
    pool: PgPool,
    table_name: String,
}
```

**Constructor: `PostgresDataStore::new(pool: PgPool, table_name: Option<String>) -> Self`**
- Stores the pool and table name (default: `"topgun_maps"`)
- Validates table name matches `^[a-zA-Z_][a-zA-Z0-9_]*$` or returns error

**Method: `PostgresDataStore::initialize(&self) -> anyhow::Result<()>`**
- Runs the CREATE TABLE IF NOT EXISTS migration
- Creates indices on `(map_name)` and `(map_name, expiration_time)` for efficient queries
- Called once after construction, before the store is handed to `RecordStoreFactory`

**Schema:**
```sql
CREATE TABLE IF NOT EXISTS {table_name} (
    map_name    TEXT    NOT NULL,
    key         TEXT    NOT NULL,
    value       BYTEA   NOT NULL,
    expiration_time BIGINT NOT NULL DEFAULT 0,
    is_backup   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  BIGINT  NOT NULL,
    updated_at  BIGINT  NOT NULL,
    PRIMARY KEY (map_name, key, is_backup)
);

CREATE INDEX IF NOT EXISTS idx_{table_name}_map
    ON {table_name} (map_name);

CREATE INDEX IF NOT EXISTS idx_{table_name}_expiry
    ON {table_name} (map_name, expiration_time)
    WHERE expiration_time > 0;
```

**MapDataStore trait implementation:**

- `add()`: Serialize `RecordValue` via `rmp_serde::to_vec_named()` into BYTEA. UPSERT (`INSERT ... ON CONFLICT DO UPDATE`) with `is_backup = false`. Set `updated_at = now`.
- `add_backup()`: Same as `add()` but with `is_backup = true`.
- `remove()`: `DELETE FROM {table} WHERE map_name = $1 AND key = $2 AND is_backup = false`.
- `remove_backup()`: Same as `remove()` but with `is_backup = true`.
- `load()`: `SELECT value FROM {table} WHERE map_name = $1 AND key = $2 AND is_backup = false`. Deserialize BYTEA via `rmp_serde::from_slice()`. Return `None` if no row.
- `load_all()`: `SELECT key, value FROM {table} WHERE map_name = $1 AND key = ANY($2) AND is_backup = false`. Return `Vec<(String, RecordValue)>`.
- `remove_all()`: `DELETE FROM {table} WHERE map_name = $1 AND key = ANY($2) AND is_backup = false`.
- `is_loadable()`: Always returns `true` (write-through, no queued writes).
- `pending_operation_count()`: Always returns `0` (write-through).
- `soft_flush()`: Returns `Ok(0)` (no pending writes to flush).
- `hard_flush()`: Returns `Ok(())` (no pending writes to flush).
- `flush_key()`: Performs the same UPSERT as `add()` / `add_backup()` depending on `is_backup` parameter. Since the trait signature does not pass `expiration_time` or `now`, use `expiration_time = 0` and `now = current system time (millis)` for the UPSERT columns. In write-through mode this is a safety net (data should already be persisted).
- `reset()`: No-op for write-through (all data already persisted).
- `is_null()`: Returns `false` (inherits default from trait, no override needed).

**Helper: `PostgresDataStore::load_all_keys(&self, map: &str) -> anyhow::Result<Vec<String>>`**
- `SELECT key FROM {table} WHERE map_name = $1 AND is_backup = false`
- Public method for startup cache warm-up (not part of `MapDataStore` trait, called by bootstrap logic)

**Note on SQL injection:** Table name is validated at construction time. All queries use `sqlx::query!` or `sqlx::query_as!` with parameterized values for `map_name`, `key`, etc. The table name is interpolated via `format!()` but is safe because it is validated against `^[a-zA-Z_][a-zA-Z0-9_]*$`.

Since `sqlx::query!` requires a compile-time-known query string, use `sqlx::query()` (runtime) with `.bind()` for all queries. The table name is interpolated via `format!()` into the query string (safe after validation).

#### 2. `packages/server-rust/src/storage/datastores/mod.rs` (modify)

- Add `mod postgres;`
- Add `pub use postgres::PostgresDataStore;`
- Gate behind `#[cfg(feature = "postgres")]` so the dependency is optional

### Files to Modify

#### 3. `packages/server-rust/Cargo.toml`

Add `sqlx` as an optional dependency gated behind a `postgres` feature:

```toml
[features]
default = []
postgres = ["dep:sqlx"]

[dependencies]
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres"], optional = true }
```

This keeps the server binary lean when PostgreSQL is not needed (e.g., tests, development with NullDataStore).

## Acceptance Criteria

1. **AC1:** `PostgresDataStore` struct exists in `packages/server-rust/src/storage/datastores/postgres.rs` and implements all 14 methods of the `MapDataStore` trait.

2. **AC2:** `PostgresDataStore::new()` validates the table name against `^[a-zA-Z_][a-zA-Z0-9_]*$` and returns an error for invalid names.

3. **AC3:** `PostgresDataStore::initialize()` creates the table and indices via `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`. Running initialize twice on the same database is idempotent.

4. **AC4:** `add()` serializes `RecordValue` via `rmp_serde::to_vec_named()` and persists it as BYTEA. A subsequent `load()` for the same `(map, key)` deserializes via `rmp_serde::from_slice()` and returns an identical `RecordValue`.

5. **AC5:** `add()` followed by `add()` with the same `(map, key)` performs an UPSERT (updates the existing row, does not create a duplicate).

6. **AC6:** `remove()` deletes the row for `(map, key, is_backup=false)`. A subsequent `load()` returns `None`.

7. **AC7:** `load_all()` returns all matching records for a batch of keys in a single SQL query using `ANY($1)`.

8. **AC8:** `remove_all()` deletes all specified keys in a single SQL query.

9. **AC9:** `add_backup()` and `remove_backup()` operate on rows where `is_backup = true`, independent of primary record rows.

10. **AC10:** `is_loadable()` returns `true`, `pending_operation_count()` returns `0`, `is_null()` returns `false`.

11. **AC11:** `soft_flush()` returns `Ok(0)`, `hard_flush()` returns `Ok(())`, `reset()` is a no-op.

12. **AC12:** `flush_key()` performs an UPSERT identical to `add()` / `add_backup()` based on the `is_backup` parameter.

13. **AC13:** `load_all_keys()` returns all non-backup keys for a given map name.

14. **AC14:** `sqlx` dependency is gated behind the `postgres` Cargo feature. Building without the feature compiles successfully and does not include sqlx.

15. **AC15:** `PostgresDataStore` is re-exported from `datastores/mod.rs` behind `#[cfg(feature = "postgres")]`.

16. **AC16:** Unit tests compile and pass (using `#[cfg(test)]` with mock or `#[sqlx::test]` with a real test database). At minimum: round-trip test (add + load), upsert test, remove test, load_all test, remove_all test, backup isolation test, table name validation test.

## Constraints

- **No coupling to PostgreSQL internals beyond sqlx:** The adapter uses only `PgPool`, `sqlx::query()`, and standard SQL. No PostgreSQL-specific extensions (LISTEN/NOTIFY, advisory locks, etc.).
- **No write-behind queuing:** This is a write-through implementation. Every mutation is persisted before the async method returns. Write-behind is a separate future spec.
- **No schema versioning / migration framework:** The single `CREATE TABLE IF NOT EXISTS` is sufficient for v1.0. A migration framework (e.g., `sqlx migrate`) is deferred to a future spec.
- **No connection string parsing:** The caller provides a configured `PgPool`. Connection string handling belongs to the server bootstrap/config layer.
- **Do not modify `RecordStoreFactory`:** The factory already accepts `Arc<dyn MapDataStore>`. No changes needed.
- **Do not modify `MapDataStore` trait:** The trait is frozen (SPEC-067/TODO-067 complete).
- **Feature-gated:** All PostgreSQL code behind `#[cfg(feature = "postgres")]` to keep default builds lean.

## Assumptions

- **sqlx 0.8 is acceptable:** The latest stable sqlx version. If the workspace pins a different version, the auditor will flag it.
- **`rmp_serde::to_vec_named()` / `from_slice()` round-trips `RecordValue` correctly:** RecordValue already derives `Serialize` + `Deserialize` with `#[serde(rename_all = "camelCase")]`. MsgPack named encoding preserves field names for forward compatibility.
- **Test database available in CI:** Integration tests using `#[sqlx::test]` require a PostgreSQL instance. This is standard for Rust projects with sqlx (GitHub Actions provides `services: postgres`). Unit tests that do not touch the database should work without PostgreSQL.
- **Table name default `topgun_maps` matches TS convention:** Both TS adapters use this default.
- **`now` parameter in trait methods is wall-clock millis:** Used for `created_at` / `updated_at` columns. The `add()` signature passes `now: i64`.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context | Segments |
|-------|------|-------|--------------|--------------|----------|
| G1 | 1 | Cargo.toml: add sqlx optional dependency with `postgres` feature | -- | ~5% | 1 |
| G2 | 1 | `postgres.rs`: struct definition, constructor with table name validation, `initialize()` schema migration | -- | ~15% | 1 |
| G3 | 2 | `postgres.rs`: implement all 14 `MapDataStore` trait methods + `load_all_keys()` helper | G2 | ~25% | 1 |
| G4 | 2 | `datastores/mod.rs`: add feature-gated `mod postgres` + re-export | G2 | ~3% | 1 |
| G5 | 3 | `postgres.rs`: unit tests (table name validation, write-through invariants) and integration tests (`#[sqlx::test]` round-trip, upsert, remove, batch, backup isolation) | G3, G4 | ~15% | 1 |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-27)
**Status:** APPROVED

**Context Estimate:** ~48% total (within target)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~48% | <=50% | OK |
| Largest task group | ~25% (G3) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (timestamps use `i64`, counts use `u64` -- correct)
- [x] No `r#type: String` on message structs (N/A -- not a message struct)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- no optional fields)
- [x] Enums used for known value sets (N/A)
- [x] Wire compatibility: uses `rmp_serde::to_vec_named()`, not `to_vec()`
- [x] `#[serde(rename_all = "camelCase")]` on every struct (N/A -- `PostgresDataStore` is not serialized; `RecordValue` already has it)
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` (N/A -- no `Option<T>` fields)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 has artifacts | OK | postgres.rs |
| Truth 2 has artifacts | OK | postgres.rs (add + load) |
| Truth 3 has artifacts | OK | postgres.rs (load_all) |
| Truth 4 has artifacts | OK | postgres.rs (PgPool) |
| Truth 5 has artifacts | OK | postgres.rs (initialize) |
| Truth 6 has artifacts | OK | No changes needed (factory already accepts Arc<dyn MapDataStore>) |
| RecordValue serialization link | OK | Covered by AC4 |
| PgPool lifetime link | OK | Covered by Arc ownership |

**Assumptions Validated:**

| # | Assumption | Validated | Notes |
|---|------------|-----------|-------|
| A1 | sqlx 0.8 is acceptable | OK | No workspace version pin exists; no conflict |
| A2 | rmp_serde round-trips RecordValue | OK | RecordValue has `Serialize + Deserialize` with `#[serde(rename_all = "camelCase")]` confirmed in source |
| A3 | Test database in CI | OK | Standard for sqlx projects |
| A4 | Table name default matches TS | OK | Reasonable convention |
| A5 | `now` is wall-clock millis | OK | Confirmed `now: i64` in trait signature |

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| MsgPack wire format | Uses `rmp_serde::to_vec_named()` for BYTEA storage | OK |
| PostgreSQL database | Implements PostgreSQL adapter per tech stack | OK |
| No code comment references | Spec constraints don't add spec/bug references | OK |
| Cargo (Rust build) | Uses Cargo features correctly | OK |
| `cargo test + proptest` testing | Uses `#[sqlx::test]` and `#[tokio::test]` | OK |

Project compliance: OK -- honors PROJECT.md decisions

**Strategic Fit:** OK -- aligned with project goals. PostgreSQL persistence is on the v1.0 critical path (TODO-090). Write-through is the simplest correct implementation; write-behind is explicitly deferred. No scope creep.

**Language Profile:**
- File count: 3 (1 create + 2 modify) <= 5 max -- OK
- Trait-first: The `MapDataStore` trait is pre-existing (frozen). No new traits needed. Wave 1 contains struct definition and foundational setup. Acceptable deviation -- OK
- Compilation gate: Largest group modifies 1 file (postgres.rs) -- OK

**Recommendations:**

1. `flush_key()` behavior should be explicit about `expiration_time` and timestamp values. The trait signature does not pass `expiration_time` or `now`, but the UPSERT needs values for these columns. Recommend: use `expiration_time = 0` and `now = SystemTime::now() millis` for the UPSERT. In write-through mode this is a safety net since data should already be persisted via `add()`. (Applied to spec -- see updated `flush_key` description in Requirements.)

2. `is_null()` does not need an explicit override. The trait's default implementation already returns `false`, which is the desired behavior for `PostgresDataStore`. The implementer can omit it and rely on the default. (Applied to spec -- see updated description.)

3. [Compliance] The spec's original G3 context estimate was ~40%, which would have exceeded the 30% per-group threshold. Many of the 14 trait methods are trivial one-liners (is_loadable, pending_operation_count, soft_flush, hard_flush, reset, is_null). The realistic estimate is ~25% for the SQL-heavy methods. (Applied to spec -- estimates corrected in Implementation Tasks table.)

4. Consider adding `regex` as a dependency for table name validation, or use a simple manual check (iterate chars) to avoid pulling in the regex crate for a single validation. The implementer should choose the lighter approach.

5. AC16 lists tests behind `#[sqlx::test]` which requires the `postgres` feature. Ensure test configuration uses `--features postgres` when running integration tests. Unit tests (table name validation) should work without the feature.

**Comment:** Well-structured spec with clear design decisions, thorough acceptance criteria, and appropriate scope. The `MapDataStore` trait is verified against source code -- all 14 methods match. The feature-gating approach and BYTEA storage decision are sound. Context estimates corrected and spec approved for implementation.

## Execution Summary

**Executed:** 2026-02-27
**Mode:** orchestrated (sequential fallback -- subagent CLI unavailable)
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |
| 3 | G5 | complete |

### Files Created
- `packages/server-rust/src/storage/datastores/postgres.rs` -- PostgresDataStore implementation (struct, constructor, initialize, all 14 MapDataStore trait methods, load_all_keys helper, 22 tests)

### Files Modified
- `packages/server-rust/Cargo.toml` -- added sqlx optional dependency with `postgres` feature gate
- `packages/server-rust/src/storage/datastores/mod.rs` -- added feature-gated `mod postgres` + `pub use PostgresDataStore`

### Commits
- `3e5a1fe` feat(sf-067): add sqlx optional dependency with postgres feature gate
- `a43a465` feat(sf-067): implement PostgresDataStore with write-through persistence
- `2d2221e` test(sf-067): add unit and integration tests for PostgresDataStore

### Acceptance Criteria Status
- [x] AC1: PostgresDataStore implements all 14 MapDataStore trait methods
- [x] AC2: Constructor validates table name against `^[a-zA-Z_][a-zA-Z0-9_]*$`
- [x] AC3: initialize() creates table and indices with IF NOT EXISTS (idempotent)
- [x] AC4: add() serializes via rmp_serde::to_vec_named(), load() deserializes via from_slice()
- [x] AC5: add() performs UPSERT (ON CONFLICT DO UPDATE)
- [x] AC6: remove() deletes the row, subsequent load() returns None
- [x] AC7: load_all() uses ANY($1) for batch loading
- [x] AC8: remove_all() uses ANY($1) for batch deletion
- [x] AC9: add_backup()/remove_backup() operate on is_backup=true rows independently
- [x] AC10: is_loadable()=true, pending_operation_count()=0, is_null()=false
- [x] AC11: soft_flush()=Ok(0), hard_flush()=Ok(()), reset() is no-op
- [x] AC12: flush_key() performs UPSERT with expiration_time=0, now=SystemTime millis
- [x] AC13: load_all_keys() returns non-backup keys for a given map
- [x] AC14: sqlx gated behind postgres Cargo feature
- [x] AC15: PostgresDataStore re-exported from mod.rs behind #[cfg(feature = "postgres")]
- [x] AC16: 22 tests (8 unit + 14 integration). Unit tests pass without database. Integration tests skip gracefully when DATABASE_URL not set.

### Deviations
- Integration tests use `#[tokio::test]` + `require_db!()` macro instead of `#[sqlx::test]`. Reason: `#[sqlx::test]` panics (hard failure) when DATABASE_URL is not set, which breaks the default `cargo test` run. The `require_db!()` macro gracefully skips when no database is available, keeping the default test suite green.
- Manual char validation for table names instead of regex crate. Reason: avoids pulling in a heavy dependency for a single validation check (per auditor recommendation #4).

### Test Results
- 468 total tests (22 new, 0 failures), clippy clean for spec files
- Builds clean with and without postgres feature

---

## Review History

### Review v1 (2026-02-27)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `PostgresDataStore` struct exists in `postgres.rs` and implements all 13 trait methods explicitly; `is_null()` correctly inherits the trait default (returns `false`) per spec
- [✓] AC2: Table name validated via `is_valid_table_name()` — iterates chars, enforces `^[a-zA-Z_][a-zA-Z0-9_]*$`, returns `bail!()` on violation. Manual char check avoids regex crate dependency (per auditor recommendation)
- [✓] AC3: `initialize()` uses `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` for all three SQL statements — fully idempotent
- [✓] AC4: `add()` uses `rmp_serde::to_vec_named()`, `load()` uses `rmp_serde::from_slice()`. MsgPack round-trip verified by three unit tests covering all `RecordValue` variants (Lww, OrMap, OrTombstones)
- [✓] AC5: UPSERT implemented via `INSERT ... ON CONFLICT (map_name, key, is_backup) DO UPDATE` — primary key conflict updates in place
- [✓] AC6: `remove()` uses `DELETE ... WHERE ... AND is_backup = false`. `remove_backup()` mirrors with `is_backup = true`. Integration test `remove_deletes_row` confirms behavior
- [✓] AC7: `load_all()` uses `ANY($2)` with `keys` bound as slice — single SQL query for batch load
- [✓] AC8: `remove_all()` uses `ANY($2)` — single SQL query for batch deletion
- [✓] AC9: Backup isolation proven by `backup_isolation` integration test: primary and backup share a (map, key) pair but different `is_backup` values; remove/load operations are fully independent
- [✓] AC10: `is_loadable()` returns `true`, `pending_operation_count()` returns `0`, `is_null()` defaults to `false`
- [✓] AC11: `soft_flush()` returns `Ok(0)`, `hard_flush()` returns `Ok(())`, `reset()` is empty no-op with WHY comment
- [✓] AC12: `flush_key()` uses UPSERT with hardcoded `expiration_time = 0` and `now = now_millis()` (SystemTime). `flush_key_backup` integration test confirms `is_backup = true` path does not appear in non-backup `load()`
- [✓] AC13: `load_all_keys()` uses `WHERE is_backup = false` — `load_all_keys_returns_non_backup_keys` integration test verifies backup-only keys are excluded
- [✓] AC14: `sqlx` declared as `optional = true` in `[dependencies]`, linked via `postgres = ["dep:sqlx"]` in `[features]`. `cargo check -p topgun-server` (no feature) passes clean
- [✓] AC15: `mod.rs` gates both `mod postgres` and `pub use postgres::PostgresDataStore` behind `#[cfg(feature = "postgres")]`
- [✓] AC16: 22 tests total (8 `#[test]` unit, 14 `#[tokio::test]` integration). All 22 pass. Integration tests skip gracefully via `require_db!()` macro when `DATABASE_URL` not set. Covers: round-trip, upsert, remove, load_all, remove_all, backup isolation, load_all_keys, initialize idempotency, flush_key, write-through invariants, expiration storage, custom table name, map isolation
- [✓] Build check (no feature): `cargo check -p topgun-server` — exit 0
- [✓] Build check (postgres feature): `cargo check -p topgun-server --features postgres` — exit 0
- [✓] Lint check: `cargo clippy -p topgun-server --features postgres -- -D warnings` — exit 0, no warnings
- [✓] Test suite: 468 passed, 0 failed, 0 ignored (excluding 1 expected ignored doctest)
- [✓] No spec/phase/bug references in code comments — WHY-comments used throughout (`// No-op for write-through: all data is already persisted`, `// i64 can hold millis until year 292_278_994 -- truncation is not a concern`)
- [✓] Security: table name SQL injection prevented by `is_valid_table_name()` at construction time. All runtime values bound via `.bind()` parameterized queries
- [✓] No unnecessary duplication: `add()` and `add_backup()` share identical structure, differentiated by literal `false`/`true` — appropriate given the simple binary distinction
- [✓] Deviations documented and justified: `require_db!()` over `#[sqlx::test]` (graceful skip vs hard panic), manual char validation over regex crate

**Summary:** Complete and correct implementation. All 16 acceptance criteria satisfied. Build, lint, and full test suite pass. The `require_db!()` deviation improves developer experience over the spec's suggested `#[sqlx::test]` approach by keeping default `cargo test` green without a database. No issues found.

---

## Completion

**Completed:** 2026-02-27
**Total Commits:** 3
**Audit Cycles:** 1
**Review Cycles:** 1
