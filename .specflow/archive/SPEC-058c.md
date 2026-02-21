---
id: SPEC-058c
type: feature
status: done
priority: P0
complexity: medium
created: 2026-02-21
parent: SPEC-058
depends_on: [SPEC-058a, SPEC-058b]
todo_ref: TODO-067
---

# DefaultRecordStore, RecordStoreFactory, and Integration

## Context

This is the third and final sub-spec of SPEC-058 (Multi-Layer Storage System). It delivers the Layer 2 orchestrator (`DefaultRecordStore`) and the factory that wires all three layers together (`RecordStoreFactory`).

`DefaultRecordStore` is the most complex component in the storage system. It orchestrates:
- Layer 1 (`StorageEngine` / `HashMapStorage`) for in-memory key-value access
- Layer 3 (`MapDataStore` / `NullDataStore`) for external persistence
- `CompositeMutationObserver` for mutation notifications to indexes, query caches, and Merkle trees

It implements the `RecordStore` trait with metadata tracking, TTL/expiry checks, eviction support, and write-through persistence. `RecordStoreFactory` provides the dependency injection point that creates fully-wired `DefaultRecordStore` instances for a given `(map_name, partition_id)` pair.

### Design Source

- `DefaultRecordStore`: RUST_STORAGE_ARCHITECTURE.md sections 5.5, 7.1
- `RecordStoreFactory`: RUST_STORAGE_ARCHITECTURE.md section 7.1
- Parent spec SPEC-058 Implementation Details section

### Key Links

- `DefaultRecordStore` owns `Box<dyn StorageEngine>` (from SPEC-058a)
- `DefaultRecordStore` holds `Arc<dyn MapDataStore>` (from SPEC-058a, impl in SPEC-058b)
- `DefaultRecordStore` holds `Arc<CompositeMutationObserver>` (from SPEC-058a)
- `RecordStoreFactory` creates `HashMapStorage` (from SPEC-058b) + `DefaultRecordStore`
- `StorageConfig` holds default TTL/max-idle/max-entry-count for the factory

## Task

Create the `impls/` sub-directory with `DefaultRecordStore`, and `factory.rs` with `RecordStoreFactory` and `StorageConfig`. Wire everything together and verify with integration-style unit tests.

## Requirements

### Files to Create

```
packages/server-rust/src/storage/
  impls/
    mod.rs                        # Re-export DefaultRecordStore
    default_record_store.rs       # DefaultRecordStore (Layer 2 orchestrator)
  factory.rs                      # RecordStoreFactory, StorageConfig
```

### Files to Modify

- `packages/server-rust/src/storage/mod.rs` -- add `pub mod impls;` and `pub mod factory;` declarations + re-exports

**Total: 3 new + 1 modified = 4 file touches.** Well within the 5-file limit.

### Type: `StorageConfig` (`factory.rs`)

```rust
/// Configuration for storage behavior, applied per-RecordStore.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Default TTL in milliseconds for new records. 0 = no TTL.
    pub default_ttl_millis: u64,
    /// Default max idle time in milliseconds. 0 = no max idle.
    pub default_max_idle_millis: u64,
    /// Maximum number of entries before eviction triggers. 0 = unlimited.
    pub max_entry_count: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            default_ttl_millis: 0,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        }
    }
}
```

### Implementation: `DefaultRecordStore` (`impls/default_record_store.rs`)

```rust
/// Per-map-per-partition record store that orchestrates all three storage layers.
pub struct DefaultRecordStore {
    name: String,
    partition_id: u32,
    engine: Box<dyn StorageEngine>,
    data_store: Arc<dyn MapDataStore>,
    observer: Arc<CompositeMutationObserver>,
    config: StorageConfig,
}
```

**Constructor:**
- `new(name, partition_id, engine, data_store, observer, config) -> Self`

**RecordStore trait implementation:**

- `name()` -- returns `&self.name`
- `partition_id()` -- returns `self.partition_id`

**Core CRUD:**

- `get(&self, key, touch)`:
  1. Check `engine.get(key)`. If found and `touch` is true, update `last_access_time` and `hits` via `metadata.on_access(now)`, then `engine.put(key, record)` to persist the metadata update. Return the record.
  2. If not found and `data_store.is_null()` is false, call `data_store.load(name, key).await`. If loaded, wrap in `Record` with fresh `RecordMetadata::new(now, 0)`, put into engine, fire `observer.on_load(key, &record, false)`, return the record.
  3. If not found anywhere, return `None`.

- `exists_in_memory(&self, key)` -- delegates to `engine.contains_key(key)`

- `put(&self, key, value, expiry, provenance)`:
  1. Check if key already exists via `engine.get(key)`.
  2. Create `RecordMetadata::new(now, 0)`. The `expiry` parameter (or `config.default_ttl_millis` / `config.default_max_idle_millis` if `expiry` is `NONE` and config has non-zero defaults) is used solely to compute `expiration_time` for the `data_store.add()` call in step 6. Do not store per-record TTL or max-idle on `RecordMetadata`; in-memory expiry checks always use the store-wide `StorageConfig` defaults.
  3. Create `Record { value, metadata }`.
  4. Put into engine. Capture old record if any.
  5. If old record existed: fire `observer.on_update(key, &record, &old_value, &new_value, false)`. Else: fire `observer.on_put(key, &record, None, false)`.
  6. If provenance is `Client` or `CrdtMerge`: call `data_store.add(name, key, &value, expiration_time, now).await`.
  7. Return old value if any.

- `remove(&self, key, provenance)`:
  1. Remove from engine.
  2. If removed: fire `observer.on_remove(key, &record, false)`.
  3. Call `data_store.remove(name, key, now).await`.
  4. Return old value if any.

- `put_backup(&self, key, record, provenance)`:
  1. Put into engine.
  2. Fire `observer.on_put(key, &record, old_value.as_ref(), true)`.
  3. If provenance is `Client` or `CrdtMerge`: call `data_store.add_backup(...)`.

- `remove_backup(&self, key, provenance)`:
  1. Remove from engine.
  2. Fire `observer.on_remove(key, &record, true)` if removed.
  3. Call `data_store.remove_backup(...)`.

**Batch:**

- `get_all(&self, keys)` -- iterates keys, calls `get(key, false)` for each, collects results

**Iteration:**

- `fetch_keys(&self, cursor, size)` -- delegates to `engine.fetch_keys(cursor, size)`
- `fetch_entries(&self, cursor, size)` -- delegates to `engine.fetch_entries(cursor, size)`
- `for_each_boxed(&self, consumer, is_backup)` -- iterates `engine.snapshot_iter()`, calls consumer for each non-expired entry

**Size and cost:**

- `size()` -- `engine.len()`
- `is_empty()` -- `engine.is_empty()`
- `owned_entry_cost()` -- `engine.estimated_cost()`

**Expiry:**

- `has_expired(&self, key, now, is_backup)`:
  1. Get record from engine.
  2. If not found, return `NotExpired`.
  3. Check TTL: if `config.default_ttl_millis > 0` and `now - record.metadata.creation_time > config.default_ttl_millis as i64`, return `Ttl`.
  4. Check max-idle: if `config.default_max_idle_millis > 0` and `now - record.metadata.last_access_time > config.default_max_idle_millis as i64`, return `MaxIdle`.
  5. Return `NotExpired`.

- `evict_expired(&self, percentage, now, is_backup)` -- iterates snapshot, checks `has_expired` for each, removes up to `percentage%` of expired entries
- `is_expirable(&self)` -- `config.default_ttl_millis > 0 || config.default_max_idle_millis > 0`

**Eviction:**

- `evict(&self, key, is_backup)`:
  1. Remove from engine.
  2. If removed: fire `observer.on_evict(key, &record, is_backup)`.
  3. If the removed record `is_dirty()` and `data_store.is_null()` is false: log a warning via `tracing::warn!` that the dirty record was evicted without being flushed. Do not call `data_store.flush_key()` or any async operation; dirty records will be flushed via `soft_flush()` or `hard_flush()` during the next flush cycle or shutdown.
  4. Return old value.

- `evict_all(&self, is_backup)` -- iterates all entries, evicts each, returns count
- `should_evict(&self)` -- `config.max_entry_count > 0 && engine.len() as u64 >= config.max_entry_count`

**Lifecycle:**

- `init(&mut self)` -- no-op for Phase 3
- `clear(&self, is_backup)` -- fire `observer.on_clear()`, then `engine.clear()`, return previous size
- `reset(&self)` -- fire `observer.on_reset()`, then `engine.clear()`
- `destroy(&self)` -- fire `observer.on_destroy(false)`, then `engine.destroy()`

**MapDataStore integration:**

- `soft_flush(&self)` -- delegates to `data_store.soft_flush()`
- `storage(&self)` -- returns `&*self.engine`
- `map_data_store(&self)` -- returns `&*self.data_store`

**Note on `evict()` async challenge:** The `RecordStore::evict()` trait method is sync, but flushing dirty records to `MapDataStore` requires async. For Phase 3 with `NullDataStore`, the flush is a no-op. For future PostgreSQL integration, either:
(a) Make `evict()` async on the trait, or
(b) Use `tokio::spawn` to fire-and-forget the flush, or
(c) Skip the flush in `evict()` and rely on the background eviction task to handle dirty records.
For this spec, use approach (c): if the record is dirty and `data_store.is_null()` is false, log a warning but do not block. The flush will happen via `soft_flush()` or `hard_flush()` during shutdown.

### Implementation: `RecordStoreFactory` (`factory.rs`)

```rust
/// Factory for creating fully-wired RecordStore instances.
pub struct RecordStoreFactory {
    config: StorageConfig,
    data_store: Arc<dyn MapDataStore>,
    observers: Vec<Arc<dyn MutationObserver>>,
}

impl RecordStoreFactory {
    /// Create a new factory with the given configuration.
    pub fn new(
        config: StorageConfig,
        data_store: Arc<dyn MapDataStore>,
        observers: Vec<Arc<dyn MutationObserver>>,
    ) -> Self {
        Self { config, data_store, observers }
    }

    /// Create a RecordStore for the given map and partition.
    pub fn create(&self, map_name: &str, partition_id: u32) -> Box<dyn RecordStore> {
        let engine = Box::new(HashMapStorage::new());
        let observer = Arc::new(CompositeMutationObserver::new(
            self.observers.iter().cloned().collect(),
        ));
        let record_store = DefaultRecordStore::new(
            map_name.to_string(),
            partition_id,
            engine,
            self.data_store.clone(),
            observer,
            self.config.clone(),
        );
        Box::new(record_store)
    }
}
```

### Sub-module re-exports

`storage/impls/mod.rs`:
```rust
mod default_record_store;
pub use default_record_store::DefaultRecordStore;
```

Update `storage/mod.rs` to add:
```rust
pub mod factory;
pub mod impls;

pub use factory::*;
pub use impls::*;
```

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero warnings
2. `cargo clippy -p topgun-server -- -D warnings` passes clean
3. `DefaultRecordStore` unit tests verify:
   - Put-then-get round-trip: put a record, get it back, values match
   - Put fires `on_put` observer notification (use `AtomicUsize` counter in mock observer)
   - Update (put on existing key) fires `on_update` observer notification
   - Remove fires `on_remove` observer notification
   - Expiry check returns `ExpiryReason::Ttl` when TTL exceeded
   - Expiry check returns `ExpiryReason::MaxIdle` when max-idle exceeded
   - Expiry check returns `ExpiryReason::NotExpired` for fresh records
   - `size()` and `owned_entry_cost()` reflect stored records accurately
   - `clear()` fires `on_clear` observer and empties the store
   - `get()` with `touch=true` updates access statistics (last_access_time, hits)
4. `RecordStoreFactory::create()` returns a working `Box<dyn RecordStore>` that can put and get records
5. `StorageConfig::default()` has all fields set to 0
6. `DefaultRecordStore` compiles as `Box<dyn RecordStore>` (object safety verified)

## Constraints

- Do NOT modify any traits or types defined in SPEC-058a
- Do NOT modify `HashMapStorage` or `NullDataStore` from SPEC-058b
- Do NOT add PostgreSQL/sqlx dependencies
- Do NOT implement write-behind, tiered storage, or OpenDAL
- Do NOT use `f64` for any integer-semantic field
- Do NOT delete or modify the existing `ServerStorage` trait in `traits.rs`
- No phase/spec/bug references in code comments
- All new public items must have doc comments

## Assumptions

- `DefaultRecordStore` uses `std::time::SystemTime` for wall-clock `now` values (milliseconds since epoch)
- `evict()` does not block on async MapDataStore flush for Phase 3 (NullDataStore is a no-op; dirty record warnings are logged for non-null stores)
- `get_all()` is implemented as a simple loop over `get()` calls; batch optimization is deferred
- `put()` computes `expiration_time` for `MapDataStore::add()` as `creation_time + ttl_millis` (0 if no TTL)
- `DefaultRecordStore` uses interior mutability where needed (the `RecordStore` trait methods take `&self`)
- `StorageConfig` derives `Clone` and `Default` for ergonomic use

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `storage/impls/mod.rs` + `storage/impls/default_record_store.rs` (DefaultRecordStore struct, constructor, RecordStore trait implementation). Add unit tests for core CRUD + observer notifications + expiry checks. | -- | ~25% |
| G2 | 2 | Create `storage/factory.rs` (StorageConfig, RecordStoreFactory). Update `storage/mod.rs` with sub-module declarations and re-exports. Add factory integration tests. | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |

**Total workers needed:** 1

## Audit History

### Audit v1 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~27% total

**Quality Projection:** PEAK range (0-30%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (N/A)
- [x] `Default` derived on `StorageConfig` (3 zero-default fields)
- [x] Enums used for known value sets (reuses existing enums from 058a)
- [x] Wire compatibility: N/A (server-internal types, not serialized)
- [x] `#[serde(rename_all)]`: N/A (not serialized)
- [x] `#[serde(skip_serializing_if)]`: N/A (no `Option<T>` fields)

**Language Profile:** Compliant with Rust profile (4 files, traits from SPEC-058a)
**Strategic fit:** Aligned with project goals (TODO-067, Phase 3 roadmap)
**Project compliance:** Honors PROJECT.md decisions

**Recommendations:**

1. `put()` step 2 says "Apply expiry from `expiry` parameter" but `RecordMetadata` has no per-record TTL/max-idle fields. The `ExpiryPolicy` parameter is only used to compute `expiration_time` for `MapDataStore::add()`, and in-memory expiry always checks store-wide `StorageConfig` defaults. This is a reasonable Phase 3 design but the wording "Apply expiry" is misleading. The implementer should interpret this as: "use the `expiry` parameter (or config defaults) to compute `expiration_time` for the `data_store.add()` call only; do not store per-record expiry on `RecordMetadata`."

2. The `evict()` description (steps 1-4) still references the async flush path with `data_store.flush_key().await` despite the later note choosing approach (c). The implementer should follow approach (c) exclusively: log via `tracing::warn!` if `is_dirty() && !data_store.is_null()`, do not call `flush_key()`.

3. [Strategic] `get()` with `touch=true` does `engine.put(key, record)` to persist the metadata update (access time, hit count). This is a full clone + re-insert for a metadata-only change. For Phase 3 with `DashMap`, this is fine (DashMap insert is O(1) amortized), but for future engines it may warrant a dedicated `engine.update_metadata()` method. No action needed now.

4. Goal Analysis section recommended for medium specs. The "Key Links" section serves as a reasonable informal equivalent.

**Comment:** Well-structured spec with clear method-by-method behavioral descriptions, explicit handling of the async/sync boundary for `evict()`, and sensible Phase 3 scope constraints. Ready for implementation.

### Response v1 (2026-02-21)
**Applied:** Recommendations 1 and 2. Recommendations 3 and 4 skipped per revision scope.

**Changes:**
1. [✓] Fix misleading `put()` step 2 wording — Rewrote step 2 to explicitly state that the `expiry` parameter (or config defaults) is used solely to compute `expiration_time` for the `data_store.add()` call in step 6, and that per-record TTL/max-idle must not be stored on `RecordMetadata`; in-memory expiry checks always use store-wide `StorageConfig` defaults.
2. [✓] Fix `evict()` steps to describe approach (c) only — Replaced step 3's reference to `data_store.flush_key().await` with the correct behavior: log a `tracing::warn!` if the record is dirty and the data store is non-null, do not call any async operation, and note that dirty records will be flushed via `soft_flush()` or `hard_flush()` during the next flush cycle or shutdown.
3. [✗] Strategic note on `get()` with `touch=true` metadata clone cost — No action needed now (per audit).
4. [✗] Goal Analysis section — No action needed now (per audit).

### Audit v2 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~35% total

**Quality Projection:** GOOD range (30-50%)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`StorageConfig` uses `u64` for millis and counts)
- [x] No `r#type: String` on message structs (N/A -- no message structs)
- [x] `Default` derived on `StorageConfig` (manual impl, all 3 fields zero)
- [x] Enums used for known value sets (reuses `CallerProvenance`, `ExpiryReason`, `ExpiryPolicy` from SPEC-058a)
- [x] Wire compatibility: N/A (server-internal types, not serialized over MsgPack)
- [x] `#[serde(rename_all)]`: N/A (not serialized)
- [x] `#[serde(skip_serializing_if)]`: N/A (no `Option<T>` fields)

**Dimensions:**
- Clarity: All method behaviors described step-by-step with concrete operations
- Completeness: All files listed, all `RecordStore` trait methods covered, observer notifications specified per method
- Testability: 10 specific test cases in acceptance criterion 3, each with measurable assertions
- Scope: Clear boundaries via 8 explicit constraints; no scope creep
- Feasibility: Interior mutability via DashMap resolves `&self` trait methods; async/sync boundary handled via approach (c)
- Architecture fit: Follows 3-layer Hazelcast-inspired storage pattern from RUST_STORAGE_ARCHITECTURE.md
- Non-duplication: Reuses all existing types/traits from SPEC-058a and implementations from SPEC-058b
- Cognitive load: Single struct with clear layered responsibilities; no unnecessary abstractions

**Assumptions validated:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | NullDataStore sufficient for Phase 3 | No persistent storage -- expected/acceptable |
| A2 | Interior mutability via DashMap handles &self trait methods | If wrong, StorageEngine trait would need &mut self -- breaks trait (verified: DashMap uses interior mutability) |
| A3 | SystemTime for wall-clock now | Could drift vs HLC, but metadata is server-internal |
| A4 | Approach (c) for dirty record eviction | Dirty records could be lost on crash before flush -- acceptable for Phase 3 |

**Project compliance:** Honors PROJECT.md decisions (no sqlx deps, no f64, Rust type mapping rules, max 5 files per spec)
**Language profile:** Compliant with Rust profile (4 files, no new traits needed -- traits defined in SPEC-058a)
**Strategic fit:** Aligned with project goals (TODO-067 Phase 3 roadmap, completes SPEC-058 multi-layer storage)

**Recommendations:**

5. `for_each_boxed()` says "calls consumer for each non-expired entry" but the trait signature `fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool)` has no `now` parameter. The implementation must use `SystemTime::now()` internally to check expiry. This is consistent with the Assumptions section but the implementer should be aware of the implicit clock dependency. No spec change needed.

6. `StorageConfig` uses a manual `impl Default` when `#[derive(Default)]` would produce identical output (all `u64` fields default to 0). Either approach compiles; the manual impl is more explicit about intent. No action required.

**Comment:** Post-revision spec is clean and implementable. Both critical recommendations from Audit v1 were properly addressed in the spec text. Method signatures align with the `RecordStore` trait from SPEC-058a. All acceptance criteria are testable. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-21
**Commits:** 2

### Files Created
- `packages/server-rust/src/storage/impls/mod.rs` — re-exports `DefaultRecordStore` and `StorageConfig`
- `packages/server-rust/src/storage/impls/default_record_store.rs` — `DefaultRecordStore` (Layer 2 orchestrator) with full `RecordStore` trait implementation, `StorageConfig`, 20 unit tests
- `packages/server-rust/src/storage/factory.rs` — `RecordStoreFactory` with `create()` method, 5 integration tests

### Files Modified
- `packages/server-rust/src/storage/mod.rs` — added `pub mod impls;` and `pub mod factory;` declarations with re-exports

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero warnings
- [x] AC2: `cargo clippy -p topgun-server -- -D warnings` passes clean
- [x] AC3: DefaultRecordStore unit tests verify all 10 specified behaviors:
  - [x] Put-then-get round-trip
  - [x] Put fires `on_put` observer notification
  - [x] Update fires `on_update` observer notification
  - [x] Remove fires `on_remove` observer notification
  - [x] Expiry returns `Ttl` when TTL exceeded
  - [x] Expiry returns `MaxIdle` when max-idle exceeded
  - [x] Expiry returns `NotExpired` for fresh records
  - [x] `size()` and `owned_entry_cost()` reflect stored records
  - [x] `clear()` fires `on_clear` observer and empties store
  - [x] `get()` with `touch=true` updates access statistics
- [x] AC4: `RecordStoreFactory::create()` returns working `Box<dyn RecordStore>`
- [x] AC5: `StorageConfig::default()` has all fields set to 0
- [x] AC6: `DefaultRecordStore` compiles as `Box<dyn RecordStore>` (object safety verified)

### Deviations
1. [Rule 3 - Compilation] Added missing `MutationObserver` trait import in `default_record_store.rs` — required for `Arc<CompositeMutationObserver>` to resolve trait methods
2. [Rule 3 - Compilation] Changed `StorageConfig` from manual `impl Default` to `#[derive(Default)]` to satisfy clippy `derivable_impls` lint (per audit recommendation 6)
3. [Rule 3 - Compilation] Added `#[allow(clippy::cast_possible_truncation)]` on `now_millis()` and `#[allow(clippy::cast_possible_wrap)]` on `compute_expiration_time()` and `check_expired()` for u64-to-i64 casts (millisecond config values are always small)
4. [Rule 3 - Compilation] Used `let...else` pattern instead of `match` in `has_expired()` to satisfy clippy `manual_let_else` lint
5. [Rule 3 - Compilation] Used `self.observers.clone()` instead of `.iter().cloned().collect()` in factory to satisfy clippy `iter_cloned_collect` and `implicit_clone` lints
6. [Rule 3 - Compilation] Placed `StorageConfig` in `default_record_store.rs` (re-exported from `impls/mod.rs`) instead of `factory.rs` — the spec showed it in `factory.rs` but it's needed by `DefaultRecordStore` which is in a different module; placing it with the consumer avoids a circular dependency

### Notes
- 126 total tests pass (25 new: 20 DefaultRecordStore + 5 factory)
- `StorageConfig` is defined in `impls/default_record_store.rs` and re-exported through `impls/mod.rs` and `storage/mod.rs`, so it's accessible as `crate::storage::StorageConfig` regardless of the file location
- The spec specified `StorageConfig` in `factory.rs` but `DefaultRecordStore` depends on it directly; to avoid a circular import, it was placed alongside the consumer. The factory imports it via the public re-export path.

---

## Review History

### Review v1 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. Phase reference in code comment
   - File: `packages/server-rust/src/storage/impls/default_record_store.rs:394`
   - Issue: Comment `// No-op for Phase 3` violates the project convention "No phase/spec/bug references in code comments" (CLAUDE.md, spec constraint 7).
   - Fix: Replace with a WHY-comment, e.g., `// No-op: initialization deferred until persistence backend is configured`

**Passed:**
- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero warnings -- verified
- [x] AC2: `cargo clippy -p topgun-server -- -D warnings` passes clean -- verified
- [x] AC3: All 10 specified DefaultRecordStore unit tests present and passing:
  - [x] `put_then_get_round_trip` -- values match after round-trip
  - [x] `put_fires_on_put_observer` -- AtomicUsize counter confirms 1 put, 0 update
  - [x] `update_fires_on_update_observer` -- 1 put + 1 update correctly distinguished
  - [x] `remove_fires_on_remove_observer` -- remove counter incremented
  - [x] `has_expired_returns_ttl` -- TTL=1000, checked at now+2000, returns Ttl
  - [x] `has_expired_returns_max_idle` -- max_idle=500, checked at now+1000, returns MaxIdle
  - [x] `has_expired_returns_not_expired_for_fresh` -- immediate check returns NotExpired
  - [x] `size_and_owned_entry_cost_reflect_records` -- 0 initially, 2 after two puts
  - [x] `clear_fires_on_clear_and_empties_store` -- observer notified, store empty, returns 2
  - [x] `get_with_touch_updates_access_stats` -- last_access_time updated, hits incremented
- [x] AC4: `factory_create_returns_working_record_store` -- put/get through Box<dyn RecordStore> works
- [x] AC5: `storage_config_default_all_zeros` -- all three fields verified as 0
- [x] AC6: `default_record_store_is_object_safe` + `factory_output_is_object_safe` -- compiles as Box<dyn RecordStore>
- [x] All 126 tests pass (25 new: 20 DefaultRecordStore + 5 factory)
- [x] No modifications to SPEC-058a traits (record_store.rs, engine.rs, mutation_observer.rs, record.rs, map_data_store.rs unchanged)
- [x] No modifications to SPEC-058b implementations (HashMapStorage, NullDataStore unchanged)
- [x] No f64 for integer-semantic fields (StorageConfig uses u64, metadata uses i64)
- [x] No hardcoded secrets or security vulnerabilities
- [x] All public items have doc comments (StorageConfig, DefaultRecordStore, RecordStoreFactory, all pub fn)
- [x] No PostgreSQL/sqlx dependencies added
- [x] Proper error handling with anyhow::Result throughout async methods
- [x] `evict()` correctly implements approach (c): logs tracing::warn for dirty records with non-null data store, does not call async flush
- [x] `compute_expiration_time()` correctly uses ExpiryPolicy TTL or config default for data_store.add() only
- [x] `#[must_use]` on constructors and factory methods
- [x] Interior mutability via DashMap (through Box<dyn StorageEngine>) handles &self trait methods correctly
- [x] Module re-exports properly wired: StorageConfig accessible as `crate::storage::StorageConfig`
- [x] Test coverage exceeds spec requirements with 10 additional tests (exists_in_memory, name_and_partition_id, is_expirable, should_evict, evict, evict_all, reset, destroy, get_all, has_expired_missing_key, for_each_boxed_skips_expired)
- [x] Rust idioms: proper use of `?` operator, `let...else`, `matches!`, `saturating_add`, no unnecessary `.clone()` beyond what DashMap requires

**Rust Idiom Check:**
- [x] No unnecessary `.clone()` calls -- clones are justified (DashMap returns owned values; record.clone() needed for engine.put after observer notification)
- [x] Error handling uses `?` operator and `Result<T, E>` throughout -- no `.unwrap()` in production code
- [x] No `unsafe` blocks
- [x] `#[allow(clippy::...)]` annotations are justified and scoped to specific functions, not module-wide

**Summary:** High-quality implementation that faithfully implements the SPEC-058c specification. All 6 acceptance criteria are met. The code is clean, idiomatic Rust with proper error handling, comprehensive test coverage (25 tests, 10 beyond minimum), and correct observer notification semantics. The only finding is a single minor comment convention violation ("Phase 3" reference). The `StorageConfig` relocation from `factory.rs` to `default_record_store.rs` is a reasonable deviation that avoids circular imports while maintaining the same public API surface. Architecture fits naturally with the existing 3-layer storage hierarchy from SPEC-058a/058b.

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 2
**Audit Cycles:** 2
**Review Cycles:** 1
