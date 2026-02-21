//! Record types for the storage layer.
//!
//! Defines the core data structures stored in [`StorageEngine`](super::StorageEngine):
//! [`Record`], [`RecordMetadata`], [`RecordValue`], and [`OrMapEntry`].

use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

/// Metadata tracked for every record in the [`RecordStore`](super::RecordStore).
///
/// Server-internal -- NOT serialized to the wire protocol.
/// Tracks version, access statistics, and timestamps for eviction and persistence.
#[derive(Debug, Clone, Default)]
pub struct RecordMetadata {
    /// Record version, incremented on every update.
    pub version: u32,
    /// Wall-clock time (millis since epoch) when this record was created.
    pub creation_time: i64,
    /// Wall-clock time of the last read access. Used by LRU eviction.
    pub last_access_time: i64,
    /// Wall-clock time of the last write.
    pub last_update_time: i64,
    /// Wall-clock time when last persisted to `MapDataStore`. 0 = never stored.
    pub last_stored_time: i64,
    /// Number of read accesses. Used by LFU eviction.
    pub hits: u32,
    /// Estimated heap cost of this record in bytes.
    pub cost: u64,
}

impl RecordMetadata {
    /// Creates new metadata with the given wall-clock time and estimated cost.
    ///
    /// Sets `creation_time`, `last_access_time`, and `last_update_time` to `now`.
    /// Version starts at 1, hits at 0, and `last_stored_time` at 0 (never stored).
    #[must_use]
    pub fn new(now: i64, cost: u64) -> Self {
        Self {
            version: 1,
            creation_time: now,
            last_access_time: now,
            last_update_time: now,
            last_stored_time: 0,
            hits: 0,
            cost,
        }
    }

    /// Records a read access: increments `hits` and updates `last_access_time`.
    pub fn on_access(&mut self, now: i64) {
        self.hits = self.hits.saturating_add(1);
        self.last_access_time = now;
    }

    /// Records a write: increments `version` and updates `last_update_time`.
    pub fn on_update(&mut self, now: i64) {
        self.version = self.version.saturating_add(1);
        self.last_update_time = now;
    }

    /// Records a persistence event: updates `last_stored_time`.
    pub fn on_store(&mut self, now: i64) {
        self.last_stored_time = now;
    }

    /// Returns `true` if the record has been modified since it was last stored.
    ///
    /// A record is dirty if `last_update_time > last_stored_time`, meaning
    /// there are changes not yet persisted to the backing `MapDataStore`.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.last_update_time > self.last_stored_time
    }
}

/// The value portion of a record, representing the actual CRDT data.
///
/// Each variant corresponds to a different CRDT strategy. Serialized to
/// `MsgPack` for persistence in the `MapDataStore` layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordValue {
    /// Last-Write-Wins value with HLC timestamp.
    Lww {
        /// The actual data value.
        value: Value,
        /// HLC timestamp of the last write.
        timestamp: Timestamp,
    },
    /// Observed-Remove Map value with tagged entries.
    OrMap {
        /// All currently active entries in the OR-Map.
        records: Vec<OrMapEntry>,
    },
    /// Tombstone markers for OR-Map deletions.
    OrTombstones {
        /// Tags of removed OR-Map entries.
        tags: Vec<String>,
    },
}

/// A single entry in an OR-Map record.
///
/// Each entry carries a unique tag for observed-remove semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrMapEntry {
    /// The actual data value.
    pub value: Value,
    /// Unique tag for observed-remove conflict resolution.
    pub tag: String,
    /// HLC timestamp when this entry was created.
    pub timestamp: Timestamp,
}

/// A complete record: CRDT value + server-internal metadata.
///
/// This is the primary unit of storage in the [`StorageEngine`](super::StorageEngine).
/// The `metadata` is server-internal and never sent over the wire.
#[derive(Debug, Clone)]
pub struct Record {
    /// The CRDT value (LWW or OR-Map data).
    pub value: RecordValue,
    /// Server-internal metadata (NOT sent over the wire).
    pub metadata: RecordMetadata,
}
