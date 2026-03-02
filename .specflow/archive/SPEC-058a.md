---
id: SPEC-058a
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-21
parent: SPEC-058
depends_on: [SPEC-055]
todo_ref: TODO-067
---

# Storage Traits, Types, and MutationObserver

## Context

This is the first sub-spec of SPEC-058 (Multi-Layer Storage System). It delivers the foundational layer: all four storage traits (`StorageEngine`, `RecordStore`, `MapDataStore`, `MutationObserver`), all shared types (`Record`, `RecordMetadata`, `RecordValue`, `OrMapEntry`, `CallerProvenance`, `ExpiryPolicy`, `ExpiryReason`, `IterationCursor`, `FetchResult`), and the `CompositeMutationObserver` implementation.

No storage engine implementations, no data store implementations, no `DefaultRecordStore`, no factory. Those are delivered by SPEC-058b (in-memory implementations) and SPEC-058c (record store and factory).

### Design Source

All trait definitions and struct layouts are drawn from `.specflow/reference/RUST_STORAGE_ARCHITECTURE.md` (sections 3.2, 4.2, 5.2-5.5, 6.2). The parent spec SPEC-058 codifies adjustments to those definitions that MUST be applied:

1. `StorageEngine::clear()` and `destroy()` use `&self` (not `&mut self`) for `Arc<dyn StorageEngine>` compatibility
2. `StorageEngine::update_value()` is dropped (premature optimization)
3. `RecordStore::for_each()` is sync (not async_trait) with a `for_each_boxed()` variant for object safety
4. `RecordMetadata` timestamps are `i64`, `version` is `u32`, `hits` is `u32`, `cost` is `u64`
5. `RecordMetadata` does NOT derive `Serialize`/`Deserialize`
6. `RecordValue` uses `Value` from `topgun_core::types` and `Timestamp` from `topgun_core::hlc`

### Key Links

- `RecordValue` references `topgun_core::types::Value` and `topgun_core::hlc::Timestamp` -- any changes to those types break storage
- `RecordStore` uses `#[async_trait]` -- the `for_each` / `for_each_boxed` methods are the exception (sync)
- All traits must be object-safe for their intended usage patterns: `Arc<dyn StorageEngine>`, `Arc<dyn MapDataStore>`, `Arc<dyn MutationObserver>`, `Box<dyn RecordStore>` all compile (verified by type alias or test)

## Task

Create the `storage/` module scaffold in `packages/server-rust/src/` with all trait definitions, shared types, and the `CompositeMutationObserver` implementation. Wire the module into `lib.rs`.

## Requirements

### Files to Create

```
packages/server-rust/src/storage/
  mod.rs                    # Module declarations + pub use re-exports
  engine.rs                 # StorageEngine trait, IterationCursor, FetchResult
  record.rs                 # Record, RecordMetadata, RecordValue, OrMapEntry
  record_store.rs           # RecordStore trait, CallerProvenance, ExpiryPolicy, ExpiryReason
  map_data_store.rs         # MapDataStore trait
  mutation_observer.rs      # MutationObserver trait, CompositeMutationObserver (struct + impl)
```

### Files to Modify

- `packages/server-rust/src/lib.rs` -- add `pub mod storage;`

**Total: 6 new + 1 modified = 7 file touches.** The 5-file limit applies to borrow checker cascade risk in implementations. This spec contains only trait/type definitions with no implementations (except `CompositeMutationObserver` which is self-contained). The `mod.rs` is structural boilerplate, and `lib.rs` is a 1-line change. Risk is minimal; same pattern as SPEC-057a (6 file touches, approved).

### Trait: `StorageEngine` (`engine.rs`)

From RUST_STORAGE_ARCHITECTURE.md section 3.2, with parent spec adjustments applied:

```rust
/// Opaque cursor for resumable iteration over storage entries.
#[derive(Debug, Clone)]
pub struct IterationCursor {
    /// Opaque state for the storage implementation to resume iteration.
    pub state: Vec<u8>,
    /// Whether iteration has completed (no more entries).
    pub finished: bool,
}

impl IterationCursor {
    /// Creates a cursor positioned at the beginning of the storage.
    pub fn start() -> Self {
        Self {
            state: Vec::new(),
            finished: false,
        }
    }
}

/// Result of a cursor-based fetch operation.
#[derive(Debug)]
pub struct FetchResult<T> {
    /// The fetched items.
    pub items: Vec<T>,
    /// Updated cursor for the next fetch call.
    pub next_cursor: IterationCursor,
}

/// Low-level typed key-value storage with cursor-based iteration.
///
/// Innermost storage layer (analogous to Hazelcast's `Storage<K,R>`).
/// Implementations are in-memory (HashMap, BTreeMap, etc.).
/// All operations are synchronous.
///
/// Wrapped in `Arc<dyn StorageEngine>` for sharing across async boundaries.
pub trait StorageEngine: Send + Sync + 'static {
    /// Insert or replace a record by key. Returns the previous record if any.
    fn put(&self, key: &str, record: Record) -> Option<Record>;

    /// Retrieve a record by key, or `None` if not present.
    fn get(&self, key: &str) -> Option<Record>;

    /// Remove a record by key, returning the removed record.
    fn remove(&self, key: &str) -> Option<Record>;

    /// Check if a key exists without returning the record.
    fn contains_key(&self, key: &str) -> bool;

    /// Return the number of entries.
    fn len(&self) -> usize;

    /// Check if the storage is empty.
    fn is_empty(&self) -> bool;

    /// Clear all entries. Takes `&self` for `Arc<dyn StorageEngine>` compatibility.
    fn clear(&self);

    /// Destroy the storage, releasing all resources. Takes `&self`.
    fn destroy(&self);

    /// Estimated heap cost of all stored entries in bytes.
    fn estimated_cost(&self) -> u64;

    /// Fetch at least `size` keys starting from `cursor`.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch at least `size` entries (key + record) starting from `cursor`.
    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)>;

    /// Return a point-in-time snapshot of all entries.
    /// The snapshot is mutation-tolerant (concurrent modifications do not fail).
    fn snapshot_iter(&self) -> Vec<(String, Record)>;

    /// Return `sample_count` random entries for eviction sampling.
    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)>;
}
```

**Note:** `update_value()` from the architecture document is intentionally omitted per parent spec adjustment #2.

### Types: `record.rs`

From RUST_STORAGE_ARCHITECTURE.md section 4.2, with parent spec adjustments:

```rust
use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

/// Metadata tracked for every record in the RecordStore.
/// Server-internal -- NOT serialized to the wire protocol.
#[derive(Debug, Clone)]
pub struct RecordMetadata {
    /// Record version, incremented on every update.
    pub version: u32,
    /// Wall-clock time (millis since epoch) when this record was created.
    pub creation_time: i64,
    /// Wall-clock time of the last read access. Used by LRU eviction.
    pub last_access_time: i64,
    /// Wall-clock time of the last write.
    pub last_update_time: i64,
    /// Wall-clock time when last persisted to MapDataStore. 0 = never stored.
    pub last_stored_time: i64,
    /// Number of read accesses. Used by LFU eviction.
    pub hits: u32,
    /// Estimated heap cost of this record in bytes.
    pub cost: u64,
}
```

Include `RecordMetadata::new(now, cost)`, `on_access(now)`, `on_update(now)`, `on_store(now)`, `is_dirty()`, and `Default` impl as shown in the architecture document section 4.2.

```rust
/// The value portion of a record, representing the actual CRDT data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordValue {
    /// Last-Write-Wins value with HLC timestamp.
    Lww {
        value: Value,
        timestamp: Timestamp,
    },
    /// Observed-Remove Map value with tagged entries.
    OrMap {
        records: Vec<OrMapEntry>,
    },
    /// Tombstone markers for OR-Map deletions.
    OrTombstones {
        tags: Vec<String>,
    },
}

/// A single entry in an OR-Map record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrMapEntry {
    pub value: Value,
    pub tag: String,
    pub timestamp: Timestamp,
}

/// A complete record: CRDT value + server-internal metadata.
#[derive(Debug, Clone)]
pub struct Record {
    /// The CRDT value (LWW or OR-Map data).
    pub value: RecordValue,
    /// Server-internal metadata (NOT sent over the wire).
    pub metadata: RecordMetadata,
}
```

### Trait: `RecordStore` (`record_store.rs`)

From RUST_STORAGE_ARCHITECTURE.md section 5.2-5.5:

```rust
/// Origin of a write operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerProvenance {
    Client,
    Backup,
    Replication,
    Load,
    CrdtMerge,
}

/// Expiry configuration for a record.
#[derive(Debug, Clone)]
pub struct ExpiryPolicy {
    /// Time-to-live in milliseconds from creation. 0 = no TTL.
    pub ttl_millis: u64,
    /// Maximum idle time in milliseconds since last access. 0 = no max idle.
    pub max_idle_millis: u64,
}

impl ExpiryPolicy {
    pub const NONE: Self = Self {
        ttl_millis: 0,
        max_idle_millis: 0,
    };
}

/// Reason a record expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryReason {
    NotExpired,
    Ttl,
    MaxIdle,
}
```

The `RecordStore` trait. Based on architecture document section 5.5, with adjustments: `for_each` replaced by `for_each_boxed` for object safety, all async methods use `#[async_trait]`, used as `Box<dyn RecordStore>`:

```rust
use async_trait::async_trait;

/// Per-map-per-partition record store.
///
/// Primary interface that operation handlers interact with.
/// Orchestrates Layer 1 (StorageEngine) and Layer 3 (MapDataStore),
/// adding metadata tracking, expiry, eviction, and mutation observation.
#[async_trait]
pub trait RecordStore: Send + Sync {
    /// Name of the map this record store manages.
    fn name(&self) -> &str;

    /// Partition ID this record store belongs to.
    fn partition_id(&self) -> u32;

    // --- Core CRUD ---

    /// Get a record, loading from MapDataStore if not in memory.
    /// Updates access statistics if `touch` is true.
    async fn get(&self, key: &str, touch: bool) -> anyhow::Result<Option<Record>>;

    /// Check if a key exists in memory (does NOT load from MapDataStore).
    fn exists_in_memory(&self, key: &str) -> bool;

    /// Put a value, returning the old value if it existed.
    /// Handles write-through to MapDataStore based on provenance.
    async fn put(
        &self,
        key: &str,
        value: RecordValue,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Remove a record, returning the old value.
    async fn remove(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Put a record received from backup replication.
    async fn put_backup(
        &self,
        key: &str,
        record: Record,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()>;

    /// Remove a record on backup.
    async fn remove_backup(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()>;

    // --- Batch operations ---

    /// Get multiple records.
    async fn get_all(&self, keys: &[String]) -> anyhow::Result<Vec<(String, Record)>>;

    // --- Iteration ---

    /// Fetch keys with cursor-based pagination.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch entries with cursor-based pagination.
    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)>;

    /// Iterate all records with an object-safe consumer.
    /// Calls `consumer` for each non-expired entry.
    /// Uses `&mut dyn FnMut` instead of generic `F: FnMut` for `Box<dyn RecordStore>` compatibility.
    fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool);

    // --- Size and cost ---

    /// Number of entries in the record store.
    fn size(&self) -> usize;

    /// Whether the record store is empty.
    fn is_empty(&self) -> bool;

    /// Total estimated heap cost of all entries.
    fn owned_entry_cost(&self) -> u64;

    // --- Expiry ---

    /// Check if a record has expired.
    fn has_expired(&self, key: &str, now: i64, is_backup: bool) -> ExpiryReason;

    /// Evict expired entries up to a percentage of total expirable entries.
    fn evict_expired(&self, percentage: u32, now: i64, is_backup: bool);

    /// Whether this record store has any entries that can expire.
    fn is_expirable(&self) -> bool;

    // --- Eviction ---

    /// Evict a single entry (e.g., due to memory pressure).
    fn evict(&self, key: &str, is_backup: bool) -> Option<RecordValue>;

    /// Evict all non-locked entries.
    fn evict_all(&self, is_backup: bool) -> u32;

    /// Whether eviction should be triggered based on current memory usage.
    fn should_evict(&self) -> bool;

    // --- Lifecycle ---

    /// Initialize the record store (create backing storage, register observers).
    fn init(&mut self);

    /// Clear all data (used by IMap.clear()).
    fn clear(&self, is_backup: bool) -> u32;

    /// Reset to initial state (used during migration).
    fn reset(&self);

    /// Destroy the record store and release all resources.
    fn destroy(&self);

    // --- MapDataStore integration ---

    /// Flush pending writes to the backing MapDataStore.
    /// Returns the sequence number of the last flushed operation.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Access the underlying StorageEngine (Layer 1).
    fn storage(&self) -> &dyn StorageEngine;

    /// Access the underlying MapDataStore (Layer 3).
    fn map_data_store(&self) -> &dyn MapDataStore;
}
```

### Trait: `MapDataStore` (`map_data_store.rs`)

From RUST_STORAGE_ARCHITECTURE.md section 6.2, with `#[async_trait]`. Used as `Arc<dyn MapDataStore>`:

```rust
use async_trait::async_trait;

/// External persistence backend for a RecordStore.
///
/// Provides the abstraction over write-through and write-behind strategies.
/// The `RecordStore` calls `add()` / `remove()` on every mutation. The
/// implementation decides when and how to actually persist the data.
#[async_trait]
pub trait MapDataStore: Send + Sync {
    /// Persist a record (or queue it for async persistence).
    /// `expiration_time` is absolute millis since epoch (0 = no expiry).
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()>;

    /// Persist a backup record.
    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()>;

    /// Remove a record from the backing store (or queue the removal).
    async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()>;

    /// Remove a backup record.
    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()>;

    /// Load a single record from the backing store.
    /// Returns `None` if the key does not exist.
    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>>;

    /// Load multiple records from the backing store.
    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>>;

    /// Remove all specified keys from the backing store.
    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()>;

    /// Check if a key is safe to load (not queued for write-behind).
    /// For write-through implementations, always returns `true`.
    fn is_loadable(&self, key: &str) -> bool;

    /// Number of pending (not yet flushed) operations.
    /// For write-through, always returns 0.
    fn pending_operation_count(&self) -> u64;

    /// Mark the store as flushable. Actual flushing happens on a background task.
    /// Returns the sequence number of the last queued operation, or 0 if empty.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Flush all pending writes immediately in the calling task.
    /// Called during node shutdown for data safety.
    async fn hard_flush(&self) -> anyhow::Result<()>;

    /// Flush a single key immediately (used during eviction).
    async fn flush_key(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        is_backup: bool,
    ) -> anyhow::Result<()>;

    /// Reset the data store to initial state (clear queues, etc.).
    fn reset(&self);

    /// Whether this is a null (no-op) implementation.
    fn is_null(&self) -> bool {
        false
    }
}
```

### Trait + Impl: `MutationObserver` (`mutation_observer.rs`)

From RUST_STORAGE_ARCHITECTURE.md section 5.4:

**Trait:**
```rust
/// Observer for record mutations within a RecordStore.
pub trait MutationObserver: Send + Sync {
    fn on_put(&self, key: &str, record: &Record, old_value: Option<&RecordValue>, is_backup: bool);
    fn on_update(&self, key: &str, record: &Record, old_value: &RecordValue, new_value: &RecordValue, is_backup: bool);
    fn on_remove(&self, key: &str, record: &Record, is_backup: bool);
    fn on_evict(&self, key: &str, record: &Record, is_backup: bool);
    fn on_load(&self, key: &str, record: &Record, is_backup: bool);
    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool);
    fn on_clear(&self);
    fn on_reset(&self);
    fn on_destroy(&self, is_shutdown: bool);
}
```

**Implementation: `CompositeMutationObserver`:**
```rust
/// Composite observer that fans out to multiple observers.
pub struct CompositeMutationObserver {
    observers: Vec<Arc<dyn MutationObserver>>,
}
```

- `new(observers: Vec<Arc<dyn MutationObserver>>) -> Self`
- `Default` impl creates an empty observer list
- Implements `MutationObserver` by iterating all observers for each method
- `add(&mut self, observer: Arc<dyn MutationObserver>)` for post-construction registration

### `storage/mod.rs`

Declares sub-modules and re-exports all public types:

```rust
pub mod engine;
pub mod map_data_store;
pub mod mutation_observer;
pub mod record;
pub mod record_store;

pub use engine::*;
pub use map_data_store::*;
pub use mutation_observer::*;
pub use record::*;
pub use record_store::*;
```

Note: `engines/`, `datastores/`, `impls/` sub-module declarations will be added by SPEC-058b and SPEC-058c respectively.

### `lib.rs` Modification

Add `pub mod storage;` to `packages/server-rust/src/lib.rs`.

## Acceptance Criteria

1. `cargo build -p topgun-server` compiles with zero errors and zero warnings
2. `cargo clippy -p topgun-server -- -D warnings` passes clean
3. All traits are object-safe for their intended patterns: `Arc<dyn StorageEngine>`, `Arc<dyn MapDataStore>`, `Arc<dyn MutationObserver>`, `Box<dyn RecordStore>` all compile (verified by type alias or test)
4. `RecordMetadata` does NOT derive `Serialize` or `Deserialize`
5. `RecordValue` derives `Serialize` and `Deserialize` with `#[serde(rename_all = "camelCase")]`
6. `CompositeMutationObserver` unit tests verify: empty observer list calls succeed without panic, multiple observers all receive each notification (use `AtomicUsize` counters in test observer)
7. `RecordMetadata::new()` sets all fields correctly, `on_access()` increments `hits` and updates `last_access_time`, `on_update()` increments `version` and updates `last_update_time`, `is_dirty()` returns correct values
8. All new public items have doc comments

## Constraints

- Do NOT delete or modify the existing `ServerStorage` trait in `traits.rs`
- Do NOT add PostgreSQL/sqlx dependencies
- Do NOT implement write-behind, tiered storage, or OpenDAL
- Do NOT add `#[serde(rename_all = "camelCase")]` to `RecordMetadata`
- Do NOT use `f64` for any integer-semantic field (version, cost, hits, timestamps)
- RecordMetadata timestamps are `i64` (not `u64`)
- No phase/spec/bug references in code comments

## Assumptions

- `CompositeMutationObserver` lives in the same file as the `MutationObserver` trait because they are tightly coupled and the composite is small (~50 lines of fan-out)
- `ExpiryPolicy` does not derive `Default` because there is no single sensible default (use `ExpiryPolicy::NONE` instead)
- `RecordStore` trait includes `for_each_boxed` (object-safe) instead of generic `for_each` to maintain `Box<dyn RecordStore>` compatibility

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `storage/mod.rs` (scaffold), `storage/record.rs` (Record, RecordMetadata, RecordValue, OrMapEntry), `storage/engine.rs` (StorageEngine trait, IterationCursor, FetchResult). Update `lib.rs`. | -- | ~15% |
| G2 | 1 | Create `storage/record_store.rs` (RecordStore trait, CallerProvenance, ExpiryPolicy, ExpiryReason), `storage/map_data_store.rs` (MapDataStore trait) | -- | ~10% |
| G3 | 2 | Create `storage/mutation_observer.rs` (MutationObserver trait, CompositeMutationObserver impl + unit tests). Add unit tests for RecordMetadata methods. Add object-safety compile tests. | G1, G2 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in Wave 1)

## Audit History

### Audit v1 (2026-02-21)
**Status:** APPROVED

**Context Estimate:** ~35% total

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (N/A -- no message structs)
- [x] `Default` derived on `RecordMetadata` (7 fields)
- [x] Enums used for known value sets (`CallerProvenance`, `ExpiryReason`)
- [x] Wire compatibility: `RecordValue` uses `rmp_serde`-compatible serde derives
- [x] `#[serde(rename_all = "camelCase")]` on serializable types (`RecordValue`, `OrMapEntry`)
- [x] `Option<T>` skip/default: N/A (no optional fields on serializable structs)

**Language Profile:**
- File count: 7 touches vs 5-file limit. Spec provides justified deviation (trait-only, precedent from SPEC-057a). Accepted.
- Trait-first: All content is traits/types with one small self-contained impl (`CompositeMutationObserver`). Compliant.

**Strategic fit:** Aligned with project goals -- foundational storage layer for Phase 3 Rust server.

**Project compliance:** Honors PROJECT.md decisions (no new dependencies, MsgPack wire format, integer types).

**Comment:** Well-structured specification with comprehensive inline code for most definitions. Clear scope boundaries with explicit constraints. Trait bodies for `RecordStore` and `MapDataStore` were added inline during audit to make the spec self-contained (previously referenced architecture document by section number only). The `CompositeMutationObserver::new()` signature intentionally differs from the architecture document (accepts `Vec` argument vs empty constructor) -- both `new(observers)` and `Default` (empty) are provided, which is the better API.

**Recommendations:**
1. Consider adding a comment to `RecordValue` documenting the serde representation strategy (externally tagged by default). Since `RecordValue` is persisted to MapDataStore (PostgreSQL), the serialization format affects future compatibility. If internally tagged is preferred, add `#[serde(tag = "type")]`.

## Execution Summary

**Executed:** 2026-02-21
**Mode:** orchestrated
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created

- `packages/server-rust/src/storage/mod.rs` -- module scaffold with re-exports
- `packages/server-rust/src/storage/record.rs` -- Record, RecordMetadata, RecordValue, OrMapEntry
- `packages/server-rust/src/storage/engine.rs` -- StorageEngine trait, IterationCursor, FetchResult
- `packages/server-rust/src/storage/record_store.rs` -- RecordStore trait, CallerProvenance, ExpiryPolicy, ExpiryReason
- `packages/server-rust/src/storage/map_data_store.rs` -- MapDataStore trait
- `packages/server-rust/src/storage/mutation_observer.rs` -- MutationObserver trait, CompositeMutationObserver impl + 15 tests

### Files Modified

- `packages/server-rust/src/lib.rs` -- added `pub mod storage;`

### Acceptance Criteria Status

- [x] `cargo build -p topgun-server` compiles with zero errors and zero warnings
- [x] `cargo clippy -p topgun-server --tests -- -D warnings` passes clean
- [x] All traits are object-safe: `Arc<dyn StorageEngine>`, `Arc<dyn MapDataStore>`, `Arc<dyn MutationObserver>`, `Box<dyn RecordStore>` all compile (verified by compile tests)
- [x] `RecordMetadata` does NOT derive `Serialize` or `Deserialize`
- [x] `RecordValue` derives `Serialize` and `Deserialize` with `#[serde(rename_all = "camelCase")]`
- [x] `CompositeMutationObserver` unit tests verify: empty observer list calls succeed without panic, multiple observers all receive each notification (uses `AtomicUsize` counters)
- [x] `RecordMetadata::new()` sets all fields correctly, `on_access()` increments `hits` and updates `last_access_time`, `on_update()` increments `version` and updates `last_update_time`, `is_dirty()` returns correct values
- [x] All new public items have doc comments

### Deviations

None.

---

## Review History

### Review v1 (2026-02-21)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: `cargo build -p topgun-server` compiles with zero errors and zero warnings -- verified
- [x] AC2: `cargo clippy -p topgun-server --tests -- -D warnings` passes clean -- verified
- [x] AC3: Object safety verified for all four trait patterns (`Arc<dyn StorageEngine>`, `Arc<dyn MapDataStore>`, `Arc<dyn MutationObserver>`, `Box<dyn RecordStore>`) -- compile tests at `mutation_observer.rs:434-456`
- [x] AC4: `RecordMetadata` derives `Debug, Clone, Default` only -- no `Serialize`/`Deserialize` -- verified at `record.rs:14`
- [x] AC5: `RecordValue` derives `Serialize, Deserialize` with `#[serde(rename_all = "camelCase")]` -- verified at `record.rs:81-82`
- [x] AC6: `CompositeMutationObserver` tests: empty list no-panic (`mutation_observer.rs:246`), multiple observers fan-out (`mutation_observer.rs:295`), `AtomicUsize` counters (`mutation_observer.rs:162-172`) -- verified
- [x] AC7: `RecordMetadata` methods: `new()` tested (`mutation_observer.rs:339`), `on_access()` tested (`mutation_observer.rs:354`), `on_update()` tested (`mutation_observer.rs:369`), `is_dirty()` tested (`mutation_observer.rs:384`), plus saturation edge cases (`mutation_observer.rs:416,424`) -- verified
- [x] AC8: All new public items have doc comments -- verified by scanning all `pub` items
- [x] Constraint: `traits.rs` not modified -- confirmed via `git diff`
- [x] Constraint: No PostgreSQL/sqlx dependencies added -- confirmed in `Cargo.toml`
- [x] Constraint: No `f64` for integer-semantic fields -- no `f64` found in storage module
- [x] Constraint: `RecordMetadata` timestamps are `i64` -- verified at `record.rs:19-24`
- [x] Constraint: No phase/spec/bug references in code comments -- verified
- [x] All 6 files created in correct locations
- [x] `lib.rs` modified with `pub mod storage;` -- verified at `lib.rs:4`
- [x] No files to delete -- N/A
- [x] 74/74 tests pass (15 new storage tests + 59 existing network tests)
- [x] `RecordMetadata::on_access()` uses `saturating_add` for `hits` -- good overflow protection
- [x] `RecordMetadata::on_update()` uses `saturating_add` for `version` -- good overflow protection
- [x] `#[must_use]` on `new()`, `start()`, `is_dirty()` -- good Rust practice
- [x] Module doc comments on all 6 files -- clean module-level documentation
- [x] `CompositeMutationObserver` derives `Default` and provides `new(observers)` -- both construction paths available
- [x] Implementation matches spec exactly -- all trait signatures, type definitions, and field types are faithful reproductions

**Rust Idiom Check:**
- [x] No unnecessary `.clone()` calls
- [x] No `.unwrap()` or `.expect()` in production code
- [x] No `unsafe` blocks
- [x] Proper `Send + Sync` bounds on all traits
- [x] No `Box<dyn Any>` type erasure

**Non-Duplication Check:**
- [x] Reuses `topgun_core::types::Value` and `topgun_core::hlc::Timestamp` rather than redefining
- [x] No copy-paste from existing code

**Cognitive Load Check:**
- [x] Clear naming consistent with Hazelcast reference architecture
- [x] No unnecessary abstractions
- [x] Layered architecture (Engine -> RecordStore -> MapDataStore) is well-documented in `mod.rs`

**Implementation Reality Check:**
- No concerns. Implementation is appropriately simple for a trait/type definition spec. No strategic red flags.

**Summary:** Implementation is a clean, faithful reproduction of the specification. All 8 acceptance criteria met. All constraints honored. Code quality is excellent with proper Rust idioms (saturating arithmetic, `#[must_use]`, comprehensive doc comments, object-safety compile tests). 15 well-structured unit tests cover the `CompositeMutationObserver` fan-out, `RecordMetadata` lifecycle methods, edge cases (saturation at `u32::MAX`), and object safety. No issues found.

---

## Completion

**Completed:** 2026-02-21
**Total Commits:** 2
**Audit Cycles:** 1
**Review Cycles:** 1
