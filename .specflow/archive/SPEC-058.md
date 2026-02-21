> **SPLIT:** This specification was decomposed into:
> - SPEC-058a: Storage Traits, Types, and MutationObserver
> - SPEC-058b: Storage In-Memory Implementations (HashMapStorage + NullDataStore)
> - SPEC-058c: DefaultRecordStore, RecordStoreFactory, and Integration
>
> See child specifications for implementation.

---
id: SPEC-058
type: feature
status: split
priority: P0
complexity: large
created: 2026-02-21
depends_on: [SPEC-055]
todo_ref: TODO-067
---

# Multi-Layer Storage System (Traits + In-Memory Framework)

## Context

The Rust server currently has a flat `ServerStorage` trait (9 methods) that maps directly from the TypeScript `IServerStorage` interface. This single-layer design cannot support the server's future needs: TTL/expiry, eviction policies, record metadata, write-behind persistence, tiered hot/cold storage, or mutation observation for indexes and Merkle trees.

The Phase 2.5 Storage Architecture Research (`.specflow/reference/RUST_STORAGE_ARCHITECTURE.md`) defines a Hazelcast-informed three-layer hierarchy that separates concerns cleanly:

- **Layer 1 (StorageEngine):** Low-level in-memory key-value with cursor-based iteration
- **Layer 2 (RecordStore):** Record metadata, TTL/expiry, eviction, mutation observers
- **Layer 3 (MapDataStore):** External persistence abstraction (write-through / write-behind)

This spec implements the **Phase 3 scope**: all traits, all types, and in-memory implementations (HashMapStorage, NullDataStore, DefaultRecordStore, RecordStoreFactory). PostgreSQL persistence (`WriteThroughPostgresDataStore`) is a separate spec. The existing `ServerStorage` trait is NOT deleted -- the switchover happens in a future Phase 3 integration spec.

### Design Source

All trait definitions, struct layouts, and file organization are drawn from `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` (sections 3-8, 11). That document has been reviewed and validated. This spec codifies it into implementable tasks.

## Goal Analysis

### Goal Statement

Provide a three-layer storage framework that operation handlers use to read/write CRDT records with metadata tracking, expiry support, and pluggable persistence -- all backed by in-memory DashMap for zero-latency access.

### Observable Truths

1. A `HashMapStorage` can store, retrieve, iterate, and evict-sample `Record` values via the `StorageEngine` trait
2. A `DefaultRecordStore` orchestrates `StorageEngine` + `MapDataStore` + `MutationObserver` to provide metadata-tracked CRUD with expiry checks
3. A `NullDataStore` implements `MapDataStore` as a no-op for testing and ephemeral data
4. `RecordStoreFactory` creates fully-wired `DefaultRecordStore` instances for a given `(map_name, partition_id)` pair
5. `CompositeMutationObserver` fans out mutation notifications to multiple observers
6. All traits compile with `Arc<dyn Trait>` usage patterns (no `Sized` leakage)
7. Trait signatures accommodate future Phase 5 extensions (S3, write-behind, tiered) without modification

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `storage/engine.rs` | `StorageEngine` trait, `IterationCursor`, `FetchResult` |
| `storage/record.rs` | `Record`, `RecordMetadata`, `RecordValue`, `OrMapEntry` |
| `storage/record_store.rs` | `RecordStore` trait, `CallerProvenance`, `ExpiryPolicy`, `ExpiryReason` |
| `storage/map_data_store.rs` | `MapDataStore` trait |
| `storage/mutation_observer.rs` | `MutationObserver` trait, `CompositeMutationObserver` |
| `storage/engines/hashmap.rs` | `HashMapStorage` (DashMap-backed `StorageEngine`) |
| `storage/datastores/null.rs` | `NullDataStore` (no-op `MapDataStore`) |
| `storage/impls/default_record_store.rs` | `DefaultRecordStore` (Layer 2 impl) |
| `storage/factory.rs` | `RecordStoreFactory`, `StorageConfig` |
| `storage/mod.rs` | Module declarations and re-exports |
| `storage/engines/mod.rs` | Sub-module re-exports |
| `storage/datastores/mod.rs` | Sub-module re-exports |
| `storage/impls/mod.rs` | Sub-module re-exports |
| `lib.rs` | Add `pub mod storage;` |

### Required Wiring

- `DefaultRecordStore` owns a `Box<dyn StorageEngine>` and holds `Arc<dyn MapDataStore>` + `Arc<CompositeMutationObserver>`
- `RecordStoreFactory` holds `Arc<dyn MapDataStore>` + `Vec<Arc<dyn MutationObserver>>` + `StorageConfig`; its `create()` method constructs `HashMapStorage` + `DefaultRecordStore`
- `Record` contains `RecordValue` (serializable CRDT data) + `RecordMetadata` (server-internal, NOT serialized)
- `RecordValue` references `Value` and `Timestamp` from `topgun-core`

### Key Links (fragile/critical)

- `RecordValue` must align with core-rust `Value` and `Timestamp` types -- any change to those breaks storage
- `StorageEngine::clear()` and `destroy()` take `&mut self` but the trait is used as `Arc<dyn StorageEngine>` -- must use interior mutability (DashMap handles this) or change signatures to `&self`
- `RecordStore` is `#[async_trait]` -- the `for_each` method with `FnMut` closure cannot be async (must remain sync)

## Task

Create a `storage/` module in `packages/server-rust/src/` implementing the three-layer storage architecture with:

1. **Traits:** `StorageEngine`, `RecordStore`, `MapDataStore`, `MutationObserver`
2. **Types:** `Record`, `RecordMetadata`, `RecordValue`, `OrMapEntry`, `CallerProvenance`, `ExpiryPolicy`, `ExpiryReason`, `IterationCursor`, `FetchResult`, `StorageConfig`
3. **Implementations:** `HashMapStorage` (DashMap), `NullDataStore`, `CompositeMutationObserver`, `DefaultRecordStore`, `RecordStoreFactory`

## Requirements

### Files to Create

**Module structure:**
```
packages/server-rust/src/storage/
  mod.rs                          # Module declarations, pub use re-exports
  engine.rs                       # StorageEngine trait, IterationCursor, FetchResult
  record.rs                       # Record, RecordMetadata, RecordValue, OrMapEntry
  record_store.rs                 # RecordStore trait, CallerProvenance, ExpiryPolicy, ExpiryReason
  map_data_store.rs               # MapDataStore trait
  mutation_observer.rs            # MutationObserver trait, CompositeMutationObserver
  factory.rs                      # RecordStoreFactory, StorageConfig
  engines/
    mod.rs                        # Re-export HashMapStorage
    hashmap.rs                    # HashMapStorage (DashMap-backed StorageEngine)
  datastores/
    mod.rs                        # Re-export NullDataStore
    null.rs                       # NullDataStore (no-op MapDataStore)
  impls/
    mod.rs                        # Re-export DefaultRecordStore
    default_record_store.rs       # DefaultRecordStore (Layer 2 orchestrator)
```

### Files to Modify

- `packages/server-rust/src/lib.rs` -- add `pub mod storage;`

### Trait Definitions

All trait signatures are defined in `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` sections 3.2, 5.4-5.5, 6.2. The implementer MUST use those definitions with the following adjustments:

1. **`StorageEngine::clear()` and `destroy()` signatures:** Change from `&mut self` to `&self` because the trait will be used as `Arc<dyn StorageEngine>`. DashMap supports `clear()` via `&self`. `destroy()` calls `clear()`.

2. **`StorageEngine::update_value()`:** Drop this method from the trait. In-place update optimization is premature for Phase 3. A `put()` suffices.

3. **`RecordStore::for_each()`:** This method takes a `FnMut` closure. It cannot be an `async_trait` method because closures with `FnMut` and `async_trait` do not compose. Keep it as a synchronous method on the trait.

4. **`RecordMetadata` fields:** All timestamp fields (`creation_time`, `last_access_time`, `last_update_time`, `last_stored_time`) use `i64` (milliseconds since epoch). `version` is `u32`. `hits` is `u32`. `cost` is `u64`. These follow the Rust Type Mapping Rules.

5. **`RecordValue`:** References `Value` from `topgun_core::types` and `Timestamp` from `topgun_core::hlc`. Uses `#[serde(rename_all = "camelCase")]` on each variant's fields where applicable.

6. **`RecordMetadata` is NOT `Serialize`/`Deserialize`:** It is server-internal metadata that never crosses the wire.

### Implementation Details

**HashMapStorage (`engines/hashmap.rs`):**
- Uses `DashMap<String, Record>` as backing store
- `snapshot_iter()` collects all entries into a `Vec` (point-in-time snapshot)
- `random_samples()` uses `rand` crate to select random entries by iterating and sampling
- `fetch_keys()` / `fetch_entries()` use `snapshot_iter()` with offset-based cursor (cursor `state` stores a `u64` offset serialized as little-endian bytes)
- `estimated_cost()` sums `record.metadata.cost` across all entries
- `is_empty()` delegates to `DashMap::is_empty()`
- `clear()` delegates to `DashMap::clear()` (takes `&self`)

**NullDataStore (`datastores/null.rs`):**
- All `async fn` methods return `Ok(())` or `Ok(None)` / `Ok(Vec::new())`
- `is_null()` returns `true`
- `is_loadable()` returns `true`
- `pending_operation_count()` returns `0`
- `soft_flush()` returns `Ok(0)`

**CompositeMutationObserver (`mutation_observer.rs`):**
- Holds `Vec<Arc<dyn MutationObserver>>`
- Each method iterates all observers and calls the corresponding method
- Implements `MutationObserver` trait itself (composable)
- `Default` impl creates an empty observer list

**DefaultRecordStore (`impls/default_record_store.rs`):**
- Owns `Box<dyn StorageEngine>` (Layer 1)
- Holds `Arc<dyn MapDataStore>` (Layer 3)
- Holds `Arc<CompositeMutationObserver>` for mutation notifications
- `get()`: checks StorageEngine first; if not found and MapDataStore is not null, loads from MapDataStore, wraps in Record with fresh metadata, puts into StorageEngine, fires `on_load`
- `put()`: creates Record with RecordMetadata, puts into StorageEngine, fires `on_put` or `on_update`, then calls `MapDataStore::add()` if provenance is `Client` or `CrdtMerge`
- `remove()`: removes from StorageEngine, fires `on_remove`, then calls `MapDataStore::remove()`
- `has_expired()`: checks TTL against `creation_time` and max-idle against `last_access_time`
- `evict()`: removes from StorageEngine, fires `on_evict`, flushes to MapDataStore if dirty
- `size()`, `is_empty()`, `owned_entry_cost()` delegate to StorageEngine
- `fetch_keys()`, `fetch_entries()` delegate to StorageEngine
- `clear()` fires `on_clear`, then clears StorageEngine
- `destroy()` fires `on_destroy`, then destroys StorageEngine

**RecordStoreFactory (`factory.rs`):**
- Holds `StorageConfig`, `Arc<dyn MapDataStore>`, `Vec<Arc<dyn MutationObserver>>`
- `create(map_name, partition_id) -> Box<dyn RecordStore>`: constructs `HashMapStorage` + `CompositeMutationObserver` + `DefaultRecordStore`
- `StorageConfig` struct with fields: `default_ttl_millis: u64` (default 0), `default_max_idle_millis: u64` (default 0), `max_entry_count: u64` (default 0 = unlimited)

### Dependencies

Add to `packages/server-rust/Cargo.toml`:
- `rand = "0.8"` (for `HashMapStorage::random_samples`)

`dashmap = "6"` is already present.

### Deletions

None. The existing `ServerStorage` trait in `traits.rs` is NOT modified or deleted.

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero warnings
2. `cargo clippy -p topgun-server -- -D warnings` passes clean
3. `HashMapStorage` unit tests verify: `put`/`get`/`remove`, `contains_key`, `len`/`is_empty`, `clear`, `fetch_keys` with cursor pagination, `fetch_entries` with cursor pagination, `snapshot_iter` returns all entries, `random_samples` returns at most `sample_count` entries, `estimated_cost` reflects stored records
4. `NullDataStore` unit tests verify: all methods return success, `is_null()` returns true, `pending_operation_count()` returns 0
5. `CompositeMutationObserver` unit tests verify: empty observer list calls succeed, multiple observers all receive each notification
6. `DefaultRecordStore` unit tests verify: put-then-get round-trip, put fires `on_put` observer, update fires `on_update` observer, remove fires `on_remove`, expiry check returns correct `ExpiryReason`, size/cost tracking is accurate
7. `RecordStoreFactory::create()` returns a working `Box<dyn RecordStore>` that can put/get records
8. All traits are object-safe: `Arc<dyn StorageEngine>`, `Arc<dyn MapDataStore>`, `Arc<dyn MutationObserver>`, `Box<dyn RecordStore>` all compile
9. `RecordMetadata` does NOT derive `Serialize` or `Deserialize`
10. `RecordValue` derives `Serialize` and `Deserialize` with `#[serde(rename_all = "camelCase")]`

## Constraints

- Do NOT delete or modify the existing `ServerStorage` trait in `traits.rs`
- Do NOT add PostgreSQL/sqlx dependencies -- persistence is a separate spec
- Do NOT implement write-behind, tiered storage, or OpenDAL -- those are Phase 5
- Do NOT add `#[serde(rename_all = "camelCase")]` to `RecordMetadata` -- it is not serialized
- Do NOT use `f64` for any integer-semantic field (version, cost, hits, timestamps)
- RecordMetadata timestamps are `i64` (wall-clock millis), not `u64`, because they must be comparable with 0 sentinel values and potential negative offsets in tests
- All new public items must have doc comments

## Assumptions

- `rand = "0.8"` is acceptable as a new dependency for eviction sampling (it is a standard Rust crate with no heavy transitive deps)
- Cursor-based iteration using offset into a snapshot Vec is acceptable for Phase 3; a more efficient approach (e.g., DashMap shard-based cursoring) can be optimized later
- `DefaultRecordStore` expiry tracking uses a simple per-key check rather than a background timer; a background eviction task is a future enhancement
- `RecordStore::put()` calls `MapDataStore::add()` synchronously in the write path (write-through); write-behind is Phase 5
- The `for_each` method on `RecordStore` trait uses a generic `F: FnMut` parameter, not a `Box<dyn FnMut>`, because the method is sync and the closure type can be monomorphized. Note: this makes `RecordStore` NOT object-safe for `for_each`. The trait will use a separate `for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool)` method instead to maintain object safety.
- `StorageConfig` derives `Clone` and `Default` for ergonomic use in tests

### Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `storage/engine.rs` (StorageEngine trait, IterationCursor, FetchResult), `storage/record.rs` (Record, RecordMetadata, RecordValue, OrMapEntry), `storage/record_store.rs` (RecordStore trait, CallerProvenance, ExpiryPolicy, ExpiryReason), `storage/map_data_store.rs` (MapDataStore trait), `storage/mutation_observer.rs` (MutationObserver trait + CompositeMutationObserver) | -- | ~35% |
| G2 | 2 | Create `storage/engines/hashmap.rs` (HashMapStorage implementation) with unit tests | G1 | ~20% |
| G3 | 2 | Create `storage/datastores/null.rs` (NullDataStore implementation) with unit tests | G1 | ~10% |
| G4 | 2 | Create CompositeMutationObserver implementation + unit tests (in `mutation_observer.rs`) | G1 | ~10% |
| G5 | 3 | Create `storage/impls/default_record_store.rs` (DefaultRecordStore) with unit tests | G1, G2, G3, G4 | ~20% |
| G6 | 3 | Create `storage/factory.rs` (RecordStoreFactory, StorageConfig) + `storage/mod.rs` + sub-module `mod.rs` files + update `lib.rs` | G1, G2, G3, G4, G5 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4 | Yes | 3 |
| 3 | G5, G6 | No | 1 |

**Total workers needed:** 3 (max in Wave 2)

**Note:** This spec has 14 files (13 new + 1 modified), exceeding the Rust max of 5 files per spec. It MUST be split via `/sf:split` before implementation.

---
*Generated by SpecFlow spec-creator on 2026-02-21. Design source: `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md`*
