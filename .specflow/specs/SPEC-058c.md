---
id: SPEC-058c
type: feature
status: draft
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
  2. Create `RecordMetadata::new(now, 0)`. Apply expiry from `expiry` parameter (or fall back to `config.default_ttl_millis` / `config.default_max_idle_millis` if expiry is `NONE` and config has non-zero defaults).
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
  3. If record `is_dirty()`: call `data_store.flush_key(name, key, &record.value, is_backup).await` (note: evict is sync in trait but needs async for flush; handle via `tokio::task::block_in_place` or make evict async).
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
