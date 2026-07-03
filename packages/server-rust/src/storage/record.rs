//! Record types for the storage layer.
//!
//! Defines the core data structures stored in [`StorageEngine`](super::StorageEngine):
//! [`Record`], [`RecordMetadata`], [`RecordValue`], and [`OrMapEntry`].

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

/// Process-monotonic counter for minting per-write identity tokens.
///
/// Initialized at 1 so the first `fetch_add(1)` returns 1. Token `0` is
/// reserved exclusively for `Default`-constructed `RecordMetadata`, which must
/// never enter the engine in a dirty state. Every live write constructs its
/// metadata via `RecordMetadata::new()`, which mints a token ≥ 1.
/// `Relaxed` ordering is sufficient because token publication is already
/// synchronized by the `DashMap` shard lock at `get_mut`/put.
static WRITE_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Minimum elapsed milliseconds before a read access updates `last_access_time`.
///
/// Coalesces recency updates to bound write amplification under read-heavy load
/// (Cassandra SEDA pattern). Updates within this window still increment `hits`
/// for LFU compatibility but do not write to the LRU timestamp field.
pub const RECENCY_COALESCE_MS: i64 = 100;

/// Metadata tracked for every record in the [`RecordStore`](super::RecordStore).
///
/// Server-internal -- NOT serialized to the wire protocol.
/// Tracks version, access statistics, and timestamps for eviction and persistence.
///
/// **Non-leakage invariant:** This struct intentionally has NO `Serialize` or
/// `Deserialize` derive. Only `RecordValue` crosses the wire or reaches the
/// persistence layer. The `cost` and `write_token` fields in particular are
/// local, never-wire, never-disk values — they must never be serialized.
///
/// **Write-token invariant:** `write_token == 0` means the record was
/// `Default`-constructed and must never enter the engine dirty. Every live or
/// hydrated write constructs metadata via `new()`, which mints a token ≥ 1.
/// `mark_stored` uses the token as an exact per-write identity check: it marks a
/// record clean only when the resident record's token matches the one the caller
/// just persisted, so a concurrent same-key write (any timestamp, equal or newer)
/// carrying a different token is never prematurely marked clean.
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
    ///
    /// LOCAL ONLY — never serialized to the wire or persisted to the datastore.
    /// Set at write time via [`estimated_cost`] + `key.len()`, then summed by
    /// the eviction orchestrator each tick to compare against the RAM ceiling.
    /// Re-derived on every load/hydrate so records entering via any path carry
    /// a real cost (eviction → reload steady state).
    pub cost: u64,
    /// Process-monotonic per-write identity token.
    ///
    /// Minted once per logical write via `WRITE_TOKEN.fetch_add(1, Relaxed)`
    /// inside `new()`. Token `0` is reserved for `Default`-constructed metadata
    /// (which must never enter the engine dirty); every live write mints a
    /// token ≥ 1. `mark_stored` uses this token as an exact identity check —
    /// only the record whose token matches the one the caller persisted is
    /// marked clean.
    ///
    /// LOCAL ONLY — never serialized to the wire or persisted to the datastore.
    pub write_token: u64,
}

impl RecordMetadata {
    /// Mints a fresh per-write identity token from the process-monotonic counter.
    ///
    /// Returns a value ≥ 1. Token `0` is reserved for `Default`-constructed
    /// metadata. All minting goes through this single helper — never
    /// `load`+`store`, which would break uniqueness under concurrent writers.
    fn mint_token() -> u64 {
        WRITE_TOKEN.fetch_add(1, Ordering::Relaxed)
    }

    /// Creates new metadata with the given wall-clock time and estimated cost.
    ///
    /// Sets `creation_time`, `last_access_time`, and `last_update_time` to `now`.
    /// Version starts at 1, hits at 0, and `last_stored_time` at 0 (never stored).
    /// Mints a fresh per-write identity token (≥ 1) for use as the exact-identity
    /// key in `mark_stored`.
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
            write_token: Self::mint_token(),
        }
    }

    /// Records a read access: increments `hits` and conditionally updates `last_access_time`.
    ///
    /// Coalesces recency updates to bound write amplification under read-heavy load
    /// (Cassandra SEDA pattern). `hits` always increments for LFU compatibility;
    /// `last_access_time` only advances when the elapsed time exceeds
    /// [`RECENCY_COALESCE_MS`] to avoid thrashing on burst reads.
    pub fn on_access(&mut self, now: i64) {
        self.hits = self.hits.saturating_add(1);
        if now > self.last_access_time + RECENCY_COALESCE_MS {
            self.last_access_time = now;
        }
    }

    /// Records a write: increments `version`, updates `last_update_time`, and
    /// mints a fresh per-write identity token.
    ///
    /// A fresh token is minted so that every logical write boundary has a unique
    /// identity. Without this, an in-place update followed by `mark_stored` could
    /// match a stale token from the original `new()` call and mark the record clean
    /// before the updated value is persisted.
    pub fn on_update(&mut self, now: i64) {
        self.version = self.version.saturating_add(1);
        self.last_update_time = now;
        self.write_token = Self::mint_token();
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

/// Conservative fallback heap cost (bytes) used when a [`RecordValue`] cannot
/// be serialized for sizing.
///
/// Deliberately moderate, not `u64::MAX`-large: a value that fails to encode
/// here also cannot be persisted, so it stays dirty and un-evictable regardless.
/// A huge sentinel would make the orchestrator thrash — evicting innocent
/// records every tick trying to drop under a ceiling it can never reach, because
/// the offending record is itself un-evictable. A moderate constant keeps the
/// orchestrator roughly honest without that collateral damage; the real signal
/// is the `tracing::warn!` emitted alongside it.
pub const COST_ESTIMATE_FALLBACK: u64 = 4096;

/// Returns the estimated heap cost of a [`RecordValue`] in bytes.
///
/// Serializes the value via `rmp_serde::to_vec_named` (the same encoding used
/// by the persistence layer) and returns the byte length, floored to `≥ 1`.
/// The floor prevents empty/`Null` values from contributing zero cost, which
/// would leave the eviction orchestrator blind to maps of empty values.
///
/// This is a deliberate *second* serialization of the value — the persistence
/// path (`MapDataStore::add`) encodes it again to write it. Reusing the
/// persisted bytes would require threading the encoded length back out through
/// the datastore trait (every backend impl), so the estimate stays
/// self-contained here. The extra encode is bounded by the value size already
/// paid for the durable write and runs only on the write path (not per eviction
/// tick), so the cost is acceptable; folding the two encodes into one is a
/// tracked follow-up optimization.
///
/// On serialization failure the fallback is [`COST_ESTIMATE_FALLBACK`] plus a
/// `tracing::warn!` — never a silent `1`, which would blind the orchestrator to
/// the record's true size. Panicking on the write path is never acceptable.
///
/// **Call sites must add `key.len() as u64`** to capture the key string's heap
/// contribution alongside the value bytes. The helper stays value-only so that
/// callers retain the key in scope without the helper needing to take ownership.
#[must_use]
pub fn estimated_cost(value: &RecordValue) -> u64 {
    match rmp_serde::to_vec_named(value) {
        Ok(bytes) => (bytes.len() as u64).max(1),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "estimated_cost: RecordValue failed to serialize; using fallback cost estimate"
            );
            COST_ESTIMATE_FALLBACK
        }
    }
}

/// The value portion of a record, representing the actual CRDT data.
///
/// Each variant corresponds to a different CRDT strategy. Serialized to
/// `MsgPack` for persistence in the `MapDataStore` layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        /// Tags of entries that have been removed but not yet garbage-collected.
        ///
        /// Coexists with `records` so both live additions and observed-remove
        /// tombstones are carried in the same storage slot. `OR_REMOVE` populates
        /// this field (see the CRDT service write path), so it is NOT empty once
        /// any removal has occurred for the key.
        ///
        /// Grows unbounded today. The designed bound is epoch/generation GC (M4):
        /// the server associates each tombstone with a server-authoritative epoch
        /// at remove-apply time (server-side metadata — this WIRE shape stays
        /// `Vec<String>`) and drops an epoch's set wholesale once the low-water-mark
        /// across ALL tracked clients has passed it. Migrating existing blobs to the
        /// epoch-indexed form (assigning the legacy corpus to one conservative
        /// pre-migration generation) is the downstream implementation's obligation
        /// (TODO-566; contract in core-rust tombstone_frontier.rs).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tombstones: Vec<String>,
    },
    /// Legacy tombstone-only markers for OR-Map deletions.
    ///
    /// Retained read-only: new code NEVER writes this variant — removals now live
    /// in `OrMap.tombstones` alongside active records. Kept decodable so blobs
    /// persisted by older servers still deserialize on restart, and folded into
    /// the unified `OrMap.tombstones` view when read by the merge path.
    OrTombstones {
        /// Tags of removed OR-Map entries.
        tags: Vec<String>,
    },
}

/// A single entry in an OR-Map record.
///
/// Each entry carries a unique tag for observed-remove semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[cfg(test)]
mod or_map_tombstone_tests {
    use super::*;

    fn ts(millis: u64, counter: u32) -> Timestamp {
        Timestamp {
            millis,
            counter,
            node_id: "node-a".to_string(),
        }
    }

    /// Active records and tombstones must coexist in the same serialized blob:
    /// the unified `OrMap` shape carries live additions and observed-remove
    /// tombstones together, so neither half may be dropped on round-trip.
    #[test]
    fn or_map_records_and_tombstones_coexist_round_trip() {
        let original = RecordValue::OrMap {
            records: vec![
                OrMapEntry {
                    value: Value::String("alice".to_string()),
                    tag: "tag-1".to_string(),
                    timestamp: ts(100, 0),
                },
                OrMapEntry {
                    value: Value::Int(42),
                    tag: "tag-2".to_string(),
                    timestamp: ts(101, 0),
                },
            ],
            tombstones: vec!["tag-removed-1".to_string(), "tag-removed-2".to_string()],
        };

        let bytes = rmp_serde::to_vec_named(&original).expect("serialize OrMap");
        let decoded: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize OrMap");

        match decoded {
            RecordValue::OrMap {
                records,
                tombstones,
            } => {
                assert_eq!(records.len(), 2, "both active records must survive");
                assert_eq!(records[0].tag, "tag-1");
                assert_eq!(records[1].tag, "tag-2");
                assert_eq!(
                    tombstones,
                    vec!["tag-removed-1".to_string(), "tag-removed-2".to_string()],
                    "tombstones must survive alongside records"
                );
            }
            other => panic!("expected OrMap, got {other:?}"),
        }
    }

    /// Legacy tombstone-only blobs persisted by older servers must still decode
    /// on restart — no migration, no un-decodable key (durability guarantee).
    #[test]
    fn legacy_or_tombstones_blob_round_trips() {
        let legacy = RecordValue::OrTombstones {
            tags: vec!["legacy-tag-1".to_string(), "legacy-tag-2".to_string()],
        };

        let bytes = rmp_serde::to_vec_named(&legacy).expect("serialize legacy OrTombstones");
        let decoded: RecordValue =
            rmp_serde::from_slice(&bytes).expect("legacy blob must still decode");

        match decoded {
            RecordValue::OrTombstones { tags } => {
                assert_eq!(
                    tags,
                    vec!["legacy-tag-1".to_string(), "legacy-tag-2".to_string()],
                    "legacy tombstone tags must round-trip intact"
                );
            }
            other => panic!("expected OrTombstones, got {other:?}"),
        }
    }
}
