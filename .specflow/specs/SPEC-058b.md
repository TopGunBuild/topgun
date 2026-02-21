---
id: SPEC-058b
type: feature
status: draft
priority: P0
complexity: small
created: 2026-02-21
parent: SPEC-058
depends_on: [SPEC-058a]
todo_ref: TODO-067
---

# Storage In-Memory Implementations (HashMapStorage + NullDataStore)

## Context

This is the second sub-spec of SPEC-058 (Multi-Layer Storage System). It delivers the concrete in-memory implementations of the Layer 1 and Layer 3 traits defined in SPEC-058a:

- **HashMapStorage:** DashMap-backed `StorageEngine` implementation for zero-latency in-memory key-value access with cursor-based iteration and eviction sampling
- **NullDataStore:** No-op `MapDataStore` for testing and ephemeral data scenarios

These are the Wave 2 implementations that can be built and tested independently against the traits from SPEC-058a. They have no dependency on each other and no dependency on `DefaultRecordStore` (SPEC-058c).

### Design Source

- `HashMapStorage`: RUST_STORAGE_ARCHITECTURE.md section 3.3
- `NullDataStore`: RUST_STORAGE_ARCHITECTURE.md section 6.4
- Parent spec SPEC-058 Implementation Details section

### Key Links

- `HashMapStorage` implements `StorageEngine` from `crate::storage::engine`
- `NullDataStore` implements `MapDataStore` from `crate::storage::map_data_store`
- Both use `Record` and `RecordValue` from `crate::storage::record`
- `HashMapStorage::random_samples()` requires the `rand` crate (new dependency)

## Task

Create `engines/` and `datastores/` sub-directories under `packages/server-rust/src/storage/` with `HashMapStorage` and `NullDataStore` implementations, comprehensive unit tests, and the `rand` dependency.

## Requirements

### Files to Create

```
packages/server-rust/src/storage/
  engines/
    mod.rs                  # Re-export HashMapStorage
    hashmap.rs              # HashMapStorage (DashMap-backed StorageEngine)
  datastores/
    mod.rs                  # Re-export NullDataStore
    null.rs                 # NullDataStore (no-op MapDataStore)
```

### Files to Modify

- `packages/server-rust/Cargo.toml` -- add `rand = "0.8"`
- `packages/server-rust/src/storage/mod.rs` -- add `pub mod engines;` and `pub mod datastores;` declarations + re-exports

**Total: 4 new + 2 modified = 6 file touches.** Both modifications are small (1-2 lines each for `mod.rs`; 1 line for `Cargo.toml`). Same precedent as SPEC-057a (6 touches, approved).

### Implementation: `HashMapStorage` (`engines/hashmap.rs`)

```rust
use dashmap::DashMap;

/// In-memory storage backed by DashMap for concurrent read access.
pub struct HashMapStorage {
    entries: DashMap<String, Record>,
}
```

**Constructor:**
- `new() -> Self` -- creates empty DashMap

**StorageEngine implementation:**

- `put(&self, key, record)` -- `self.entries.insert(key.to_string(), record)` returns `Option<Record>`
- `get(&self, key)` -- `self.entries.get(key).map(|r| r.clone())`
- `remove(&self, key)` -- `self.entries.remove(key).map(|(_, r)| r)`
- `contains_key(&self, key)` -- `self.entries.contains_key(key)`
- `len(&self)` -- `self.entries.len()`
- `is_empty(&self)` -- `self.entries.is_empty()`
- `clear(&self)` -- `self.entries.clear()` (DashMap::clear takes `&self`)
- `destroy(&self)` -- calls `self.clear()`
- `estimated_cost(&self)` -- iterates all entries, sums `record.metadata.cost`
- `snapshot_iter(&self)` -- collects all entries into `Vec<(String, Record)>` via DashMap iteration (point-in-time snapshot)
- `random_samples(&self, sample_count)` -- uses reservoir sampling: iterate all entries, for each entry at index `i`, if `i < sample_count` add to result, else replace a random existing sample with probability `sample_count / (i + 1)`. Uses `rand::thread_rng()`. Returns at most `min(sample_count, len())` entries.
- `fetch_keys(&self, cursor, size)` -- takes a snapshot via `snapshot_iter()`, decodes cursor state as `u64` offset (little-endian, empty = 0), skips `offset` entries, takes `size`, returns new cursor with updated offset. Sets `finished = true` when `offset + size >= total`.
- `fetch_entries(&self, cursor, size)` -- same cursor logic as `fetch_keys`, returns `(String, Record)` tuples.

**Cursor encoding:** The cursor `state` field stores a `u64` offset as 8 little-endian bytes. An empty `state` (from `IterationCursor::start()`) is treated as offset 0.

### Implementation: `NullDataStore` (`datastores/null.rs`)

```rust
/// No-op MapDataStore for testing and ephemeral data.
/// All operations succeed immediately without side effects.
pub struct NullDataStore;
```

**MapDataStore implementation:**

- `add()` -- returns `Ok(())`
- `add_backup()` -- returns `Ok(())`
- `remove()` -- returns `Ok(())`
- `remove_backup()` -- returns `Ok(())`
- `load()` -- returns `Ok(None)`
- `load_all()` -- returns `Ok(Vec::new())`
- `remove_all()` -- returns `Ok(())`
- `is_loadable()` -- returns `true`
- `pending_operation_count()` -- returns `0`
- `soft_flush()` -- returns `Ok(0)`
- `hard_flush()` -- returns `Ok(())`
- `flush_key()` -- returns `Ok(())`
- `reset()` -- no-op
- `is_null()` -- returns `true`

### Sub-module re-exports

`storage/engines/mod.rs`:
```rust
mod hashmap;
pub use hashmap::HashMapStorage;
```

`storage/datastores/mod.rs`:
```rust
mod null;
pub use null::NullDataStore;
```

Update `storage/mod.rs` to add:
```rust
pub mod engines;
pub mod datastores;

pub use engines::*;
pub use datastores::*;
```

### Dependencies

Add to `packages/server-rust/Cargo.toml` under `[dependencies]`:
```toml
rand = "0.8"
```

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero warnings
2. `cargo clippy -p topgun-server -- -D warnings` passes clean
3. `HashMapStorage` unit tests verify:
   - `put`/`get`/`remove` round-trip
   - `contains_key` returns true after put, false after remove
   - `len`/`is_empty` reflect current state
   - `clear` empties the storage
   - `fetch_keys` with cursor pagination: first page returns correct keys, second page returns remaining, finished cursor at end
   - `fetch_entries` with cursor pagination: same cursor logic as `fetch_keys`
   - `snapshot_iter` returns all entries
   - `random_samples` returns at most `sample_count` entries, returns 0 for empty storage
   - `estimated_cost` reflects sum of `record.metadata.cost` across all stored records
4. `NullDataStore` unit tests verify:
   - All async methods return `Ok`
   - `load()` returns `Ok(None)`
   - `load_all()` returns `Ok(Vec::new())`
   - `is_null()` returns `true`
   - `is_loadable()` returns `true`
   - `pending_operation_count()` returns `0`
   - `soft_flush()` returns `Ok(0)`
5. `HashMapStorage` and `NullDataStore` are accessible via `crate::storage::HashMapStorage` and `crate::storage::NullDataStore` re-exports

## Constraints

- Do NOT modify any traits defined in SPEC-058a
- Do NOT add PostgreSQL/sqlx dependencies
- Do NOT use `f64` for any integer-semantic field
- `HashMapStorage::random_samples()` must not panic on empty storage
- `fetch_keys`/`fetch_entries` must handle cursor past-end gracefully (return empty items, finished = true)
- No phase/spec/bug references in code comments
- All new public items must have doc comments

## Assumptions

- `rand = "0.8"` is acceptable as a dependency (standard crate, no heavy transitive deps)
- Reservoir sampling is acceptable for `random_samples()` in Phase 3; a more efficient approach can be optimized later
- Cursor-based iteration using offset into a snapshot Vec is acceptable for Phase 3 partition sizes
- `NullDataStore` is a unit struct (no fields) -- it has no state to manage
- `HashMapStorage` does not need `Default` derive (use `HashMapStorage::new()` instead) but can optionally derive it

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `rand` to Cargo.toml. Create `storage/engines/mod.rs` + `storage/engines/hashmap.rs` (HashMapStorage implementation). Update `storage/mod.rs` with `pub mod engines;`. Add HashMapStorage unit tests. | -- | ~20% |
| G2 | 1 | Create `storage/datastores/mod.rs` + `storage/datastores/null.rs` (NullDataStore implementation). Update `storage/mod.rs` with `pub mod datastores;`. Add NullDataStore unit tests. | -- | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |

**Total workers needed:** 2 (max in Wave 1)
