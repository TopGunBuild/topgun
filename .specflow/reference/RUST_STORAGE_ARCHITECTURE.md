# TopGun Rust Storage Architecture Design

**Research ID:** RES-003
**Created:** 2026-02-19
**Status:** Complete
**Blocks:** TODO-067 (Multi-Layer Storage System), TODO-080 (Storage Architecture Research)

---

## 1. Executive Summary

This document defines a three-layer storage architecture for the TopGun Rust server, informed by Hazelcast's `Storage` / `RecordStore` / `MapDataStore` hierarchy, with Rust implementation patterns drawn from TiKV's `engine_traits` and Databend's OpenDAL integration.

The architecture separates concerns into:

1. **Layer 1 — `StorageEngine`** (low-level KV): Raw key-value operations, cursor-based iteration, mutation-tolerant iterators
2. **Layer 2 — `RecordStore`** (record management): CRDT record metadata, TTL/expiry, eviction, access statistics, MutationObserver pattern
3. **Layer 3 — `MapDataStore`** (external persistence): Write-through / write-behind to PostgreSQL, S3, or any backing store via async queue

All three layers are defined as Rust traits with no concrete dependencies. Phase 3 implements PostgreSQL + in-memory backends. Phase 5 adds S3 (via OpenDAL), tiered hot/cold, and write-behind queues without modifying any existing trait signatures.

---

## 2. Architecture Overview

```
    ┌───────────────────────────────────────────────────┐
    │                  Operation Layer                   │
    │   (handlers receive partition-routed operations)   │
    └──────────────────────┬────────────────────────────┘
                           │
    ┌──────────────────────▼────────────────────────────┐
    │              Layer 2: RecordStore                  │
    │  ┌─────────────────────────────────────────────┐  │
    │  │ - CRDT merge (LWW/OR conflict resolution)   │  │
    │  │ - Record metadata (version, timestamps, HLC) │  │
    │  │ - TTL / expiry system                        │  │
    │  │ - Eviction policies (LRU, LFU, random)       │  │
    │  │ - MutationObserver notifications             │  │
    │  │ - CallerProvenance tracking                  │  │
    │  └──────────────┬──────────────┬───────────────┘  │
    │                 │              │                   │
    │    ┌────────────▼──────┐  ┌───▼──────────────┐    │
    │    │ Layer 1: Storage  │  │ Layer 3: MapData  │    │
    │    │ Engine (in-memory)│  │ Store (external)  │    │
    │    │                   │  │                   │    │
    │    │ - HashMap/BTree   │  │ - write-through   │    │
    │    │ - cursor iteration│  │ - write-behind    │    │
    │    │ - cost estimation │  │ - soft/hard flush │    │
    │    └───────────────────┘  └───────────────────┘    │
    └───────────────────────────────────────────────────┘

    Layer 1 backends:          Layer 3 backends:
    - InMemoryStorage          - PostgresDataStore
    - (future: TieredStorage)  - S3DataStore (OpenDAL)
                               - NullDataStore (no persistence)
                               - WriteBehindDataStore (async queue)
```

---

## 3. Layer 1: StorageEngine Trait

### 3.1 Design Rationale

Hazelcast's `Storage<K,R>` is the lowest layer: a typed in-memory map with cursor-based iteration, cost estimation, and sampling for eviction. TiKV's approach of separating `Peekable`, `Iterable`, and `Mutable` into independent traits provides better composability in Rust.

For TopGun, Layer 1 differs from both references because our keys are always `(map_name, key)` string pairs and our values are CRDT records. We do NOT need column families (TiKV) or raw byte KV (RocksDB). Instead, we need a typed store optimized for `HashMap`-style access with cursor iteration support.

### 3.2 Trait Definition

```rust
use std::sync::Arc;

/// Opaque cursor for resumable iteration over storage entries.
/// Implementations define their own internal representation.
/// The cursor is `Send + Sync` so it can be passed across async boundaries.
#[derive(Debug, Clone)]
pub struct IterationCursor {
    /// Opaque state that the storage implementation uses to resume iteration.
    /// For HashMap-based storage, this might be a bucket index.
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
pub struct FetchResult<T> {
    /// The fetched items.
    pub items: Vec<T>,
    /// Updated cursor for the next fetch call.
    pub next_cursor: IterationCursor,
}

/// Low-level typed key-value storage with cursor-based iteration.
///
/// This is the innermost storage layer (analogous to Hazelcast's `Storage<K,R>`).
/// Implementations are expected to be in-memory (HashMap, BTreeMap, etc.).
/// All operations are synchronous — this layer does not touch disk or network.
///
/// # Concurrency
///
/// Implementations must be safe for single-writer / multi-reader access.
/// The `RecordStore` layer serializes writes per partition; concurrent reads
/// are allowed (e.g., iteration during query processing).
///
/// # TiKV Pattern
///
/// Following TiKV's engine_traits, this trait is wrapped in `Arc<dyn StorageEngine>`
/// for cheap cloning across async boundaries.
pub trait StorageEngine: Send + Sync + 'static {
    /// Insert or replace a record by key.
    fn put(&self, key: &str, record: Record) -> Option<Record>;

    /// Update a record's value in place, returning the updated record.
    /// If the record supports in-place updates (e.g., same size), this
    /// avoids allocation. Otherwise, replaces the record.
    fn update_value(&self, key: &str, record: &Record, new_value: RecordValue) -> Record;

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

    /// Clear all entries.
    fn clear(&mut self);

    /// Destroy the storage, releasing all resources.
    fn destroy(&mut self);

    /// Estimated heap cost of all stored entries in bytes.
    fn estimated_cost(&self) -> u64;

    // --- Cursor-based iteration (Hazelcast pattern) ---

    /// Fetch at least `size` keys starting from `cursor`.
    /// Returns fetched keys and an updated cursor.
    /// The implementation MAY return more than `size` items.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch at least `size` entries (key + record) starting from `cursor`.
    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)>;

    /// Return a mutation-tolerant iterator over all entries.
    /// The iterator does NOT fail-fast on concurrent modification.
    /// This matches Hazelcast's `Storage.mutationTolerantIterator()`.
    fn snapshot_iter(&self) -> Vec<(String, Record)>;

    // --- Eviction support ---

    /// Return `sample_count` random entries for eviction sampling.
    /// Used by LRU/LFU/random eviction policies.
    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)>;
}
```

### 3.3 Phase 3 Implementation: `HashMapStorage`

```rust
use dashmap::DashMap;

/// In-memory storage backed by DashMap for concurrent read access.
pub struct HashMapStorage {
    entries: DashMap<String, Record>,
}
```

`DashMap` provides lock-free reads and sharded writes, matching the single-writer/multi-reader pattern. The `snapshot_iter()` method creates a point-in-time `Vec` clone for mutation-tolerant iteration, acceptable for TopGun's partition sizes (entries per partition, not entire dataset).

### 3.4 Phase 5 Extension Point: `TieredStorage`

A future `TieredStorage` implementation wraps a hot `HashMapStorage` and a cold on-disk backend (e.g., RocksDB via `engine_rocks` or a custom B-tree). The `StorageEngine` trait accommodates this without modification because:

- `get()` can check hot tier first, then cold tier (transparent to callers)
- `fetch_entries()` can merge cursors from both tiers
- `estimated_cost()` can sum both tiers for memory pressure decisions

---

## 4. Record and RecordMetadata

### 4.1 Design Rationale

Hazelcast's `Record<V>` interface carries rich metadata: version, creation/access/update/stored timestamps, hit count, cost, and sequence number. TopGun needs a subset of this, adapted for CRDTs:

- **HLC timestamp** (replaces Hazelcast's version + lastUpdateTime): The HLC IS the version for LWW conflict resolution
- **Access tracking** (lastAccessTime, hits): Required for LRU/LFU eviction
- **Storage tracking** (lastStoredTime): Required for write-behind to know which records are dirty
- **Creation time**: Required for TTL calculation
- **Cost estimation**: Required for memory-aware eviction

### 4.2 Struct Definitions

```rust
use crate::hlc::Timestamp;
use crate::types::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Metadata tracked for every record in the RecordStore.
/// Corresponds to Hazelcast's Record<V> metadata methods.
///
/// Deliberately NOT serialized to MsgPack wire protocol — this is
/// server-internal metadata. Only the CRDT value + HLC timestamp
/// cross the wire.
#[derive(Debug, Clone)]
pub struct RecordMetadata {
    /// Record version, incremented on every update.
    /// Used for optimistic concurrency checks in backup operations.
    pub version: u32,

    /// Wall-clock time (millis since epoch) when this record was created.
    pub creation_time: i64,

    /// Wall-clock time of the last read access. Used by LRU eviction.
    pub last_access_time: i64,

    /// Wall-clock time of the last write. Redundant with HLC for LWW,
    /// but needed for wall-clock-based TTL calculations.
    pub last_update_time: i64,

    /// Wall-clock time when this record was last persisted to MapDataStore.
    /// `0` means never stored. Used by write-behind to identify dirty records.
    pub last_stored_time: i64,

    /// Number of read accesses. Used by LFU eviction.
    pub hits: u32,

    /// Estimated heap cost of this record in bytes.
    /// Updated on every put/update for memory-aware eviction.
    pub cost: u64,
}

impl RecordMetadata {
    /// Create metadata for a newly created record.
    pub fn new(now: i64, cost: u64) -> Self {
        Self {
            version: 0,
            creation_time: now,
            last_access_time: now,
            last_update_time: now,
            last_stored_time: 0,
            hits: 0,
            cost,
        }
    }

    /// Called on every read access.
    pub fn on_access(&mut self, now: i64) {
        self.hits = self.hits.saturating_add(1);
        self.last_access_time = now;
    }

    /// Called on every write/update.
    pub fn on_update(&mut self, now: i64) {
        self.version = self.version.wrapping_add(1);
        self.last_update_time = now;
    }

    /// Called when the record is persisted to external storage.
    pub fn on_store(&mut self, now: i64) {
        self.last_stored_time = now;
    }

    /// Returns true if the record has been modified since last storage flush.
    pub fn is_dirty(&self) -> bool {
        self.last_stored_time < self.last_update_time
    }
}

impl Default for RecordMetadata {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// The value portion of a record, representing the actual CRDT data.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct OrMapEntry {
    pub value: Value,
    pub tag: String,
    pub timestamp: Timestamp,
}

/// A complete record: CRDT value + server-internal metadata.
/// This is the unit stored in Layer 1 (StorageEngine).
#[derive(Debug, Clone)]
pub struct Record {
    /// The CRDT value (LWW or OR-Map data).
    pub value: RecordValue,
    /// Server-internal metadata (NOT sent over the wire).
    pub metadata: RecordMetadata,
}
```

### 4.3 Key Design Decisions

1. **`RecordMetadata` is NOT `Serialize/Deserialize`**: It is server-internal. Only `RecordValue` crosses the wire via MsgPack. This prevents metadata leakage to clients and keeps the wire protocol clean.

2. **`RecordValue` replaces `StorageValue`**: The current `StorageValue` in `core-rust/src/types.rs` is an opaque `Vec<u8>`. The new `RecordValue` enum provides typed access needed by the `RecordStore` layer for merge decisions and TTL calculations.

3. **`version` is `u32` (wrapping)**: Matches Hazelcast's `Record.getVersion()` semantics. The HLC timestamp is the authoritative version for conflict resolution; this version counter is for backup consistency checks.

4. **`cost` is `u64`**: Estimated heap cost in bytes. Required for memory-aware eviction policies to know when to trigger eviction.

---

## 5. Layer 2: RecordStore Trait

### 5.1 Design Rationale

Hazelcast's `RecordStore<R>` is the primary interface that operation handlers interact with. It orchestrates Layer 1 (Storage) and Layer 3 (MapDataStore), adding:

- TTL/expiry management
- Eviction
- Record metadata tracking
- MutationObserver notifications (for indexes, query caches, event publishers)
- CallerProvenance tracking (local, backup, replication, client)
- Map loading from external stores

For TopGun, the `RecordStore` is per-map-per-partition (one instance per `(map_name, partition_id)` pair). This matches Hazelcast's model where each `RecordStore` belongs to a specific partition.

### 5.2 CallerProvenance

```rust
/// Origin of a write operation, used to decide persistence and event behavior.
///
/// Hazelcast equivalent: `CallerProvenance` (WAN vs NOT_WAN).
/// TopGun extends this for CRDT-specific provenance tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerProvenance {
    /// Write originated from a client (primary write path).
    Client,
    /// Write originated from a backup replication operation.
    Backup,
    /// Write originated from inter-cluster replication (future: WAN).
    Replication,
    /// Write originated from a map-loader (initial data load).
    Load,
    /// Write originated from a CRDT merge during sync.
    CrdtMerge,
}
```

### 5.3 ExpiryPolicy

```rust
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

### 5.4 MutationObserver

```rust
use std::sync::Arc;

/// Observer for record mutations within a RecordStore.
///
/// Hazelcast equivalent: `MutationObserver<R>` with `CompositeMutationObserver`.
/// Used to notify indexes, query caches, event publishers, and Merkle trees
/// of record changes without coupling RecordStore to those subsystems.
///
/// Multiple observers can be composed via `CompositeMutationObserver`.
pub trait MutationObserver: Send + Sync {
    /// Called when a new record is inserted.
    fn on_put(&self, key: &str, record: &Record, old_value: Option<&RecordValue>, is_backup: bool);

    /// Called when an existing record is updated.
    fn on_update(
        &self,
        key: &str,
        record: &Record,
        old_value: &RecordValue,
        new_value: &RecordValue,
        is_backup: bool,
    );

    /// Called when a record is removed.
    fn on_remove(&self, key: &str, record: &Record, is_backup: bool);

    /// Called when a record is evicted (not removed by user, but by eviction policy).
    fn on_evict(&self, key: &str, record: &Record, is_backup: bool);

    /// Called when a record is loaded from external storage.
    fn on_load(&self, key: &str, record: &Record, is_backup: bool);

    /// Called when a record is inserted due to replication.
    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool);

    /// Called when the RecordStore is cleared.
    fn on_clear(&self);

    /// Called when the RecordStore is reset (e.g., during migration).
    fn on_reset(&self);

    /// Called when the RecordStore is destroyed.
    fn on_destroy(&self, is_shutdown: bool);
}

/// Composite observer that fans out to multiple observers.
/// Matches Hazelcast's `CompositeMutationObserver` pattern.
pub struct CompositeMutationObserver {
    observers: Vec<Arc<dyn MutationObserver>>,
}

impl CompositeMutationObserver {
    pub fn new() -> Self {
        Self {
            observers: Vec::with_capacity(4),
        }
    }

    pub fn add(&mut self, observer: Arc<dyn MutationObserver>) {
        self.observers.push(observer);
    }
}

// Implementation fans out each method to all observers, collecting the first error.
// (Mirrors Hazelcast's CompositeMutationObserver exactly.)
```

### 5.5 RecordStore Trait

```rust
use async_trait::async_trait;

/// Per-map-per-partition record store.
///
/// This is the primary interface that operation handlers interact with.
/// It orchestrates Layer 1 (StorageEngine) and Layer 3 (MapDataStore),
/// adding metadata tracking, expiry, eviction, and mutation observation.
///
/// Hazelcast equivalent: `RecordStore<R extends Record>`
///
/// # Ownership Model
///
/// One `RecordStore` instance per `(map_name, partition_id)`.
/// The `RecordStore` owns its `StorageEngine` instance and holds
/// an `Arc<dyn MapDataStore>` reference to the shared backing store.
///
/// # Concurrency
///
/// Operations on a RecordStore are serialized by the partition thread/task.
/// Multiple RecordStores (different partitions) run in parallel.
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

    /// Iterate all records, calling the consumer for each non-expired entry.
    fn for_each<F>(&self, consumer: F, is_backup: bool)
    where
        F: FnMut(&str, &Record);

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

---

## 6. Layer 3: MapDataStore Trait

### 6.1 Design Rationale

Hazelcast's `MapDataStore<K,V>` abstracts over write-through and write-behind persistence strategies. The key insight is that `RecordStore` calls `MapDataStore.add()` on every write, and the `MapDataStore` decides whether to persist immediately (write-through) or queue the write (write-behind).

For TopGun Phase 3, only write-through to PostgreSQL is needed. Phase 5 adds:
- Write-behind with async queue and batched flushes
- S3 object storage via OpenDAL
- Tiered write strategies (hot writes go to PostgreSQL, cold archival goes to S3)

The trait must support all of these from day 1.

### 6.2 Trait Definition

```rust
use async_trait::async_trait;

/// External persistence backend for a RecordStore.
///
/// Hazelcast equivalent: `MapDataStore<K, V>`
///
/// Provides the abstraction over write-through and write-behind strategies.
/// The `RecordStore` calls `add()` / `remove()` on every mutation. The
/// implementation decides when and how to actually persist the data.
///
/// # Implementations
///
/// - `WriteThroughDataStore` — synchronous persistence (Phase 3)
/// - `WriteBehindDataStore` — async queue with batched flush (Phase 5)
/// - `NullDataStore` — no persistence (testing, ephemeral data)
///
/// # Phase 5: OpenDAL Integration
///
/// S3/GCS/Azure backends use OpenDAL's `Operator` trait internally.
/// The `MapDataStore` trait does NOT expose OpenDAL types — it operates
/// on TopGun `RecordValue` and `String` keys. The OpenDAL `Operator`
/// is an implementation detail of specific `MapDataStore` backends.
///
/// ```text
/// WriteThroughDataStore<PostgresBackend>     — Phase 3
/// WriteThroughDataStore<OpenDalBackend<S3>>  — Phase 5
/// WriteBehindDataStore<PostgresBackend>      — Phase 5
/// TieredDataStore { hot: Pg, cold: S3 }      — Phase 5
/// ```
#[async_trait]
pub trait MapDataStore: Send + Sync {
    /// Persist a record (or queue it for async persistence).
    ///
    /// For write-through: persists immediately and returns the (possibly
    /// post-processed) value.
    /// For write-behind: queues the write and returns the value immediately.
    ///
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
    ///
    /// Hazelcast equivalent: `MapDataStore.softFlush()`
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Flush all pending writes immediately in the calling task.
    /// Called during node shutdown for data safety.
    ///
    /// Hazelcast equivalent: `MapDataStore.hardFlush()`
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

### 6.3 Phase 3 Implementation: `WriteThroughPostgresDataStore`

```rust
/// Write-through implementation backed by PostgreSQL via sqlx.
///
/// Every `add()` / `remove()` call executes a SQL query immediately.
/// This is the simplest and safest strategy for Phase 3.
pub struct WriteThroughPostgresDataStore {
    pool: sqlx::PgPool,
    table_name: String,
}
```

Key implementation details:
- Uses `sqlx::PgPool` with compile-time checked queries
- `store()` uses `INSERT ... ON CONFLICT DO UPDATE` (upsert)
- `store_all()` uses a transaction with batched inserts
- The table schema matches the existing TS PostgresAdapter for wire compatibility:
  - `(map_name TEXT, key TEXT, value JSONB, ts_millis BIGINT, ts_counter INT, ts_node_id TEXT, is_deleted BOOL)`

### 6.4 Phase 3 Implementation: `NullDataStore`

```rust
/// No-op implementation for testing and ephemeral data.
/// All operations succeed immediately without side effects.
pub struct NullDataStore;

impl MapDataStore for NullDataStore {
    fn is_null(&self) -> bool { true }
    // ... all other methods are no-ops
}
```

### 6.5 Phase 5 Extension: `OpenDalDataStore`

```rust
/// Object storage backend using Apache OpenDAL.
///
/// Supports S3, GCS, Azure, local filesystem, and 50+ other backends
/// via OpenDAL's `Operator` trait with layer composition.
///
/// ```text
/// let op = Operator::new(services::S3::default())?
///     .layer(TimeoutLayer::new().with_timeout(Duration::from_secs(10)))
///     .layer(RetryLayer::new().with_jitter())
///     .layer(MetricsLayer::default())
///     .finish();
/// ```
///
/// Records are serialized to MsgPack and stored as objects with key path:
/// `{map_name}/{key}.msgpack`
pub struct OpenDalDataStore {
    operator: opendal::Operator,
}
```

### 6.6 Phase 5 Extension: `WriteBehindDataStore`

```rust
/// Async write-behind wrapper around any MapDataStore.
///
/// Hazelcast equivalent: `WriteBehindStore`
///
/// Queues writes to an internal ring buffer and flushes them
/// in batches on a background tokio task. Provides:
/// - Configurable batch size and flush interval
/// - Coalescing of multiple writes to the same key
/// - soft_flush (mark as flushable) vs hard_flush (drain now)
///
/// ```text
/// let pg = WriteThroughPostgresDataStore::new(pool);
/// let wb = WriteBehindDataStore::new(
///     Arc::new(pg),
///     WriteBehindConfig {
///         batch_size: 100,
///         flush_interval: Duration::from_secs(5),
///         coalesce: true,
///     },
/// );
/// ```
pub struct WriteBehindDataStore {
    delegate: Arc<dyn MapDataStore>,
    queue: tokio::sync::mpsc::Sender<DelayedEntry>,
    config: WriteBehindConfig,
}
```

### 6.7 Phase 5 Extension: `TieredDataStore`

```rust
/// Tiered storage: hot writes go to primary (PostgreSQL),
/// cold data is archived to secondary (S3 via OpenDAL).
///
/// Write path: always writes to `hot` backend.
/// Read path: tries `hot` first, falls back to `cold`.
/// Archive task: periodically moves old records from hot to cold.
pub struct TieredDataStore {
    hot: Arc<dyn MapDataStore>,
    cold: Arc<dyn MapDataStore>,
    archive_policy: ArchivePolicy,
}
```

---

## 7. Integration: How Layers Compose

### 7.1 Factory Pattern

Following Hazelcast's `MapContainer` factory pattern, a `RecordStoreFactory` creates the full stack for a given map + partition:

```rust
pub struct RecordStoreFactory {
    config: StorageConfig,
    data_store: Arc<dyn MapDataStore>,
    observers: Vec<Arc<dyn MutationObserver>>,
}

impl RecordStoreFactory {
    /// Create a RecordStore for the given map and partition.
    pub fn create(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Box<dyn RecordStore> {
        let storage = Arc::new(HashMapStorage::new());
        let record_store = DefaultRecordStore::new(
            map_name.to_string(),
            partition_id,
            storage,
            self.data_store.clone(),
            self.observers.clone(),
            self.config.clone(),
        );
        Box::new(record_store)
    }
}
```

### 7.2 Default Wiring (Phase 3)

```text
RecordStoreFactory
  ├── StorageEngine: HashMapStorage (DashMap)
  ├── MapDataStore: WriteThroughPostgresDataStore (sqlx)
  └── MutationObservers:
      ├── MerkleTreeObserver (updates Merkle tree on mutations)
      ├── QueryCacheObserver (notifies live query subscriptions)
      └── EventPublisherObserver (publishes to topic subscribers)
```

### 7.3 Phase 5 Wiring

```text
RecordStoreFactory
  ├── StorageEngine: TieredStorage { hot: HashMapStorage, cold: RocksDB }
  ├── MapDataStore: WriteBehindDataStore {
  │     delegate: TieredDataStore {
  │       hot: WriteThroughPostgresDataStore,
  │       cold: OpenDalDataStore<S3>,
  │     }
  │   }
  └── MutationObservers:
      ├── MerkleTreeObserver
      ├── QueryCacheObserver
      ├── EventPublisherObserver
      └── ArchiveObserver (triggers hot-to-cold migration)
```

---

## 8. Replacing the Current `ServerStorage` Trait

### 8.1 Current State

The existing `ServerStorage` trait in `packages/server-rust/src/traits.rs` is a flat interface matching the TS `IServerStorage`:

```rust
pub trait ServerStorage: Send + Sync {
    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<StorageValue>>;
    async fn store(&self, map: &str, key: &str, value: &StorageValue) -> anyhow::Result<()>;
    // ... (8 methods total)
}
```

### 8.2 Migration Strategy

The `ServerStorage` trait will be **replaced** (not extended) by the three-layer hierarchy:

| Old `ServerStorage` method | New location |
|---|---|
| `load()` | `MapDataStore::load()` |
| `load_all()` | `MapDataStore::load_all()` |
| `load_all_keys()` | Removed (cursor-based iteration replaces full key scan) |
| `store()` | `MapDataStore::add()` (via `RecordStore::put()`) |
| `store_all()` | Batched via `RecordStore::put()` loop inside a transaction |
| `delete()` | `MapDataStore::remove()` (via `RecordStore::remove()`) |
| `delete_all()` | `MapDataStore::remove_all()` |
| `initialize()` | `MapDataStore` constructor + `RecordStore::init()` |
| `close()` | `MapDataStore::hard_flush()` + drop |

The `MapProvider` trait remains as-is — it creates `RecordStore` instances through the factory.

---

## 9. Patterns from Reference Codebases

### 9.1 TiKV: `Arc<DB>` Wrapping

TiKV wraps its engine in `Arc<RocksEngine>` for cheap cloning across async boundaries. TopGun should follow this pattern:

```rust
// Storage engine is wrapped in Arc for sharing across partition tasks
type SharedStorage = Arc<dyn StorageEngine>;

// MapDataStore is wrapped in Arc because multiple RecordStores
// (different partitions) share the same PostgreSQL connection pool
type SharedDataStore = Arc<dyn MapDataStore>;
```

### 9.2 TiKV: Extension Trait Pattern

TiKV uses `WriteBatchExt` as an extension trait on `KvEngine` to associate the `WriteBatch` type. TopGun can adopt a simpler version:

```rust
/// Extension trait for StorageEngine implementations that support
/// atomic batch writes.
pub trait WriteBatchExt: StorageEngine {
    type WriteBatch: WriteBatch;

    fn write_batch(&self) -> Self::WriteBatch;
    fn write_batch_with_capacity(&self, cap: usize) -> Self::WriteBatch;
}

pub trait WriteBatch: Send {
    fn put(&mut self, key: &str, record: Record);
    fn delete(&mut self, key: &str);
    fn commit(self) -> anyhow::Result<()>;
    fn count(&self) -> usize;
    fn clear(&mut self);
}
```

### 9.3 Databend: OpenDAL Layer Composition

Databend's `build_operator()` function demonstrates the layer composition pattern:

```rust
// Timeout -> Runtime -> HttpClient -> Retry -> Logging -> Tracing -> Metrics
let op = Operator::new(services::S3::default())?
    .layer(TimeoutLayer::new())
    .layer(RuntimeLayer::new(runtime))
    .finish()
    .layer(HttpClientLayer::new(http_client))
    .layer(RetryLayer::new().with_jitter())
    .layer(LoggingLayer::default())
    .layer(MetricsLayer::default());
```

TopGun's `OpenDalDataStore` should use this exact pattern. The layer order matters:
1. **Timeout** (innermost): Cancel operations that take too long
2. **Runtime**: Offload I/O to a dedicated runtime
3. **Retry**: Retry transient failures with jitter
4. **Logging/Metrics**: Observability (outermost)

### 9.4 Hazelcast: CompositeMutationObserver

Hazelcast's `CompositeMutationObserver` fans out mutations to multiple observers (indexes, event publishers, query caches). Each observer is called in sequence, and the first error is collected and re-thrown after all observers have been called. This "notify all, fail on first error" pattern ensures:

- All observers see every mutation (no silent drops)
- A failing observer does not prevent other observers from being notified
- The error is eventually propagated to the caller

TopGun should implement the same pattern using `Vec<Arc<dyn MutationObserver>>`.

---

## 10. Migration Path: Phase 3 to Phase 5

### 10.1 What Exists at End of Phase 3

- `StorageEngine` trait + `HashMapStorage` (DashMap-backed)
- `RecordStore` trait + `DefaultRecordStore` (orchestrates Layer 1 + 3)
- `MapDataStore` trait + `WriteThroughPostgresDataStore` + `NullDataStore`
- `Record`, `RecordMetadata`, `RecordValue` structs
- `MutationObserver` trait + `CompositeMutationObserver`
- `CallerProvenance` enum

### 10.2 What Phase 5 Adds (NO trait changes needed)

| Feature | Implementation | Trait changes |
|---|---|---|
| S3 storage | `OpenDalDataStore` implementing `MapDataStore` | None |
| Write-behind | `WriteBehindDataStore` wrapping any `MapDataStore` | None |
| Tiered hot/cold | `TieredDataStore` composing two `MapDataStore`s | None |
| Tiered in-memory | `TieredStorage` implementing `StorageEngine` | None |
| Archive policy | `ArchiveObserver` implementing `MutationObserver` | None |
| Memory-aware eviction | Eviction config on `RecordStore`, uses `estimated_cost()` | None |

### 10.3 Verification: Trait Stability

Each Phase 5 feature is verified against the trait surface:

1. **S3 storage**: `MapDataStore::add/remove/load` — the trait operates on `RecordValue` + `String` keys, not SQL. S3 backend serializes to MsgPack and stores as objects. **No trait change.**

2. **Write-behind**: Wraps `MapDataStore` with a queue. `soft_flush()` and `hard_flush()` are already on the trait. `pending_operation_count()` is already on the trait. **No trait change.**

3. **Tiered hot/cold**: Composes two `MapDataStore` instances. Read path: try hot, fallback to cold. Write path: always hot. **No trait change.**

4. **Tiered in-memory**: `StorageEngine::get()` checks hot tier first, then cold. `estimated_cost()` sums both. `random_samples()` samples from hot tier only (cold is for evicted data). **No trait change.**

5. **Time-travel queries**: Uses `MapDataStore::load()` with a version/timestamp qualifier. This WOULD require extending `MapDataStore::load()` with an optional timestamp parameter. **Mitigation**: Define `load()` to accept an optional snapshot context in Phase 3, defaulting to `None` (latest version). Alternatively, add a separate `load_at()` method in Phase 5 — this is an additive change, not a breaking one.

---

## 11. File Organization

```
packages/server-rust/src/
├── storage/
│   ├── mod.rs                   # Re-exports
│   ├── engine.rs                # StorageEngine trait + IterationCursor
│   ├── record.rs                # Record, RecordMetadata, RecordValue
│   ├── record_store.rs          # RecordStore trait, CallerProvenance, ExpiryPolicy
│   ├── map_data_store.rs        # MapDataStore trait
│   ├── mutation_observer.rs     # MutationObserver trait + CompositeMutationObserver
│   ├── factory.rs               # RecordStoreFactory
│   ├── engines/
│   │   ├── mod.rs
│   │   ├── hashmap.rs           # HashMapStorage (Phase 3)
│   │   └── tiered.rs            # TieredStorage (Phase 5)
│   ├── datastores/
│   │   ├── mod.rs
│   │   ├── null.rs              # NullDataStore (Phase 3)
│   │   ├── postgres.rs          # WriteThroughPostgresDataStore (Phase 3)
│   │   ├── opendal.rs           # OpenDalDataStore (Phase 5)
│   │   ├── write_behind.rs      # WriteBehindDataStore (Phase 5)
│   │   └── tiered.rs            # TieredDataStore (Phase 5)
│   └── impls/
│       ├── mod.rs
│       └── default_record_store.rs  # DefaultRecordStore (Phase 3)
```

---

## 12. Open Questions for Spec Phase

1. **Record cost estimation**: Should `cost` be computed by `StorageEngine` (knows memory layout) or by `RecordStore` (knows record structure)? Recommendation: `RecordStore` computes cost from `RecordValue` size, `StorageEngine` tracks aggregate.

2. **Partition-level locking**: Hazelcast's `RecordStore` includes entry-level locking (`txnLock`, `lock`, `unlock`). TopGun uses CRDTs which eliminate write conflicts. Do we need entry locking for `EntryProcessor` semantics? Recommendation: Defer to TODO-082 (Service Architecture Research).

3. **Map loading on startup**: Hazelcast has a complex `MapKeyLoader` for pre-loading data from MapDataStore on startup. TopGun should lazy-load via `MapProvider::get_or_load_map()`. Full pre-loading is a Phase 5 optimization.

4. **JSON metadata store**: Hazelcast has a `JsonMetadataStore` for JSON path indexing. TopGun uses MsgPack, not JSON. Defer to TODO-071 (Search/Tantivy integration).

---

*Generated by SpecFlow research for TODO-080. This document is the primary architectural reference for SPEC writers implementing TODO-067 (Multi-Layer Storage System).*
