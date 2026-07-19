//! Record types for the storage layer.
//!
//! Defines the core data structures stored in [`StorageEngine`](super::StorageEngine):
//! [`Record`], [`RecordMetadata`], [`RecordValue`], and [`OrMapEntry`].

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::tombstone_gauge::with_sink;

/// Process-monotonic counter for minting per-write identity tokens.
///
/// Initialized at 1 so the first `fetch_add(1)` returns 1. Token `0` is
/// reserved exclusively for `Default`-constructed `RecordMetadata`, which must
/// never enter the engine in a dirty state. Every live write constructs its
/// metadata via `RecordMetadata::new()`, which mints a token ≥ 1.
/// `Relaxed` ordering is sufficient because token publication is already
/// synchronized by the `DashMap` shard lock at `get_mut`/put.
static WRITE_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Adds `n` bytes to the process-global OR-Map tombstone-bytes gauge and
/// emits the same delta to both exported Prometheus series: the
/// `topgun_ormap_tombstone_bytes_total` monotonic creation-rate counter
/// (mirrors the `topgun_operations_total` emit pattern in
/// `service/middleware/metrics.rs`), and the `topgun_ormap_tombstone_bytes`
/// gauge — the decrementable series a prune path can move back down, and the
/// one the soak monitor's plateau/slope fit reads over `GET /metrics`.
///
/// Call sites pass `tag.len() as u64` — the tombstone's contribution is its
/// removed tag's UTF-8 byte length, which is the direct measure of what grows
/// in `OrMap.tombstones: Vec<String>`. Fixed per-entry allocator overhead is
/// intentionally excluded: the slope, not the absolute total, is the signal
/// this gauge exists to expose, and a constant per-entry factor only scales
/// it without changing what it reveals.
pub fn add_tombstone_bytes(n: u64) {
    with_sink(|s| s.add(n));
}

/// Subtracts `n` bytes from the process-global OR-Map tombstone-bytes gauge,
/// and mirrors the same decrement onto the exported `topgun_ormap_tombstone_bytes`
/// Prometheus gauge — the byte-for-byte counterpart of [`add_tombstone_bytes`]'s
/// increment.
///
/// This is the decrement path for the OR-Map epoch-GC prune: it is called as
/// the prune drops fully-superseded tombstone epochs, passing the same
/// `tag.len() as u64` accounting [`add_tombstone_bytes`] used when the tag was
/// first tombstoned, so the two calls are exact inverses over a tag's
/// lifetime.
///
/// Unlike the `topgun_ormap_tombstone_bytes_total` counter (which follows the
/// `_total` *monotonic* convention and therefore cannot legally decrease),
/// `topgun_ormap_tombstone_bytes` has no such constraint: it is a plain
/// Prometheus gauge, so this function's decrement is externally visible over
/// `GET /metrics` without needing the in-process [`tombstone_bytes`] accessor
/// — the surface an out-of-process soak monitor actually scrapes.
///
/// Saturating-safe by construction: the prune is only ever expected to
/// subtract bytes it previously observed being added, so it should never
/// underflow in practice, but `fetch_sub` wraps on underflow rather than
/// panicking — acceptable for a monitoring counter.
pub fn sub_tombstone_bytes(n: u64) {
    with_sink(|s| s.sub(n));
}

/// Reads the current value of the process-global OR-Map tombstone-bytes gauge.
///
/// This is the authoritative in-process accessor referenced by the
/// [`sub_tombstone_bytes`] Prometheus-divergence note above, and by the
/// soak-test monitor that watches for unbounded tombstone growth.
#[must_use]
// The closure is redundant only in the release arm, where the resolver hands
// over a concrete `&ProcessGauge`. Test builds receive a `&dyn
// TombstoneGaugeSink`, and a bare method path cannot satisfy that
// higher-ranked bound. The closure is the one form that compiles in both.
#[allow(clippy::redundant_closure_for_method_calls)]
pub fn tombstone_bytes() -> u64 {
    with_sink(|s| s.read())
}

/// Re-baselines the OR-Map tombstone-bytes gauge to an absolute `total`.
///
/// This is the **only** absolute-set path for the gauge. It exists exclusively
/// for the one-time startup reconciliation that runs after WAL recovery
/// completes (see `bin/topgun_server.rs`): the process-local
/// [`ProcessGauge`](super::tombstone_gauge::ProcessGauge) atomic and both exported Prometheus series
/// (`topgun_ormap_tombstone_bytes_total` and `topgun_ormap_tombstone_bytes`)
/// reset to zero/absent on every process start and never re-count rehydrated
/// (redb-persisted) tombstones, so without this boot seed the scraped series
/// sawtooths back to 0 on every `kill -9` restart and the cross-restart leak
/// becomes invisible. Seeding from the true reconciled corpus (rather than a
/// literal `0`) means a genuine leak still shows as net upward drift from a
/// real starting point, and a monitor that only trusts non-zero samples sees
/// one from the first scrape after boot. It MUST NEVER be called on the hot
/// read/write path — only once, at boot, before the listener accepts
/// connections.
///
/// It performs THREE writes, in order, because they are **separate sinks**:
///  1. `bytes.store(total)` re-baselines the in-process `AtomicU64`
///     — an absolute store, never `fetch_add`: a per-rehydration increment would
///     reintroduce the eviction double-count the gauge's cardinal rule forbids.
///  2. `metrics::counter!(...).increment(total)` seeds the monotonic
///     `_total` Prometheus counter, a *different* sink living in the process
///     `PrometheusHandle` recorder; on a fresh process it starts at 0/absent,
///     so a single `increment(total)` from zero lands the exported series at
///     `total` while staying a legal monotonic-from-zero counter (a Prometheus
///     counter has no `.set`/`.store`).
///  3. `metrics::gauge!(...).set(total)` re-baselines the decrementable
///     `topgun_ormap_tombstone_bytes` gauge — the series the soak monitor's
///     plateau/slope fit actually scrapes over `GET /metrics`. Unlike the
///     counter this is a plain absolute `.set`, so it carries no additive
///     double-count risk and can be called safely even if this boot seed ever
///     ran more than once.
pub fn set_tombstone_bytes(total: u64) {
    with_sink(|s| s.set(total));
}

/// The startup warning for a detected legacy [`RecordValue::OrTombstones`] corpus,
/// or `None` when the store holds none.
///
/// Pre-epoch stores are not supported for tombstone reclamation: an untouched
/// legacy blob is excluded from the epoch scan and so never becomes GC-eligible.
/// The supported way to reclaim such a corpus is a fresh datastore (server wipe +
/// clients re-sync). This surfaces that at boot so an operator sees the pinned
/// bytes are deliberate, not a leak.
///
/// Kept a pure `count -> Option<message>` decision so it is unit-testable off the
/// bin-only boot walk that supplies the count.
#[must_use]
pub fn legacy_tombstone_warning(legacy_row_count: u64) -> Option<String> {
    (legacy_row_count > 0).then(|| {
        format!(
            "legacy tombstone corpus present ({legacy_row_count} row(s)) — excluded from GC; \
             recreate the datastore to reclaim"
        )
    })
}

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

/// Per-`OrMapEntry` structural framing constant (bytes).
///
/// Approximates the fixed `MsgPack` overhead of one `OrMapEntry` map — the map
/// header plus the `value`/`tag`/`timestamp` field-name strings and the nested
/// `Timestamp` map's `millis`/`counter`/`nodeId` keys, headers, and fixed-width
/// numeric fields — everything except the variable `value`, `tag`, and
/// `node_id` byte lengths, which are added separately. Chosen so the cheap
/// structural estimate lands close to (within ~1.5× of) the true
/// `rmp_serde::to_vec_named` size for representative OR entries.
const OR_ENTRY_FRAMING_BYTES: u64 = 61;

/// Fixed `MsgPack` framing (bytes) for the outer `OrMap` map itself (the map
/// header + `records`/`tombstones` field names + array headers).
const OR_MAP_FRAMING_BYTES: u64 = 12;

/// Structural byte-size estimate of a [`Value`], approximating its `MsgPack`
/// encoding without serializing.
///
/// `Value` is an externally-tagged enum, so each variant carries a small
/// map/variant-name framing overhead in addition to its payload bytes; the
/// per-variant constants below fold that in. This is a monitoring/eviction size
/// signal, not an exact wire measurement.
fn value_byte_estimate(value: &Value) -> u64 {
    match value {
        Value::Null => 5,
        Value::Bool(_) => 7,
        Value::Int(_) => 14,
        Value::Float(_) => 16,
        Value::String(s) => 10 + s.len() as u64,
        Value::Bytes(b) => 9 + b.len() as u64,
        Value::Array(items) => 8 + items.iter().map(value_byte_estimate).sum::<u64>(),
        Value::Map(entries) => {
            6 + entries
                .iter()
                .map(|(k, v)| k.len() as u64 + 3 + value_byte_estimate(v))
                .sum::<u64>()
        }
    }
}

/// Cheap structural byte-size estimate of an `OrMap` slot — the sum of each
/// record's value/tag/timestamp byte lengths plus fixed framing, and each
/// tombstone tag's UTF-8 byte length — WITHOUT serializing the whole snapshot.
///
/// This reuses the same `tag.len()` accounting the SPEC-345 tombstone gauge
/// uses. It replaces the former per-put `rmp_serde::to_vec_named` of the entire
/// (often ~130 KB) OR snapshot, which was a dominant source of per-op allocator
/// churn on the OR write path.
fn or_map_estimated_cost(records: &[OrMapEntry], tombstones: &[String]) -> u64 {
    let records_bytes: u64 = records
        .iter()
        .map(|e| {
            OR_ENTRY_FRAMING_BYTES
                + value_byte_estimate(&e.value)
                + e.tag.len() as u64
                + e.timestamp.node_id.len() as u64
        })
        .sum();
    let tombstones_bytes: u64 = tombstones.iter().map(|t| t.len() as u64 + 2).sum();
    OR_MAP_FRAMING_BYTES + records_bytes + tombstones_bytes
}

/// Returns the estimated heap cost of a [`RecordValue`] in bytes.
///
/// For the [`RecordValue::OrMap`] arm this is a cheap structural size estimate
/// (see [`or_map_estimated_cost`]) — the sum of the records' value/tag/timestamp
/// byte lengths plus the tombstone tag byte lengths — so the write path never
/// serializes the whole (often ~130 KB) OR snapshot just to size it. Every
/// other arm serializes via `rmp_serde::to_vec_named` (the same encoding used by
/// the persistence layer) and returns the byte length. The result is floored to
/// `≥ 1` so empty/`Null` values still contribute non-zero cost, keeping the
/// eviction orchestrator from going blind to maps of empty values.
///
/// The non-OrMap serialize is a deliberate *second* encode — the persistence
/// path (`MapDataStore::add`) encodes the value again to write it. Reusing the
/// persisted bytes would require threading the encoded length back out through
/// the datastore trait (every backend impl), so the estimate stays
/// self-contained here. It runs only on the write path (not per eviction tick).
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
    if let RecordValue::OrMap {
        records,
        tombstones,
    } = value
    {
        return or_map_estimated_cost(records, tombstones).max(1);
    }

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
        /// (TODO-566; contract in core-rust `tombstone_frontier.rs`).
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
mod tombstone_bytes_gauge_tests {
    use super::*;
    use crate::storage::tombstone_gauge::with_isolated_gauge;

    // Every test here binds a private sink for the duration of its own future,
    // so it observes only the writes it makes itself. `#[tokio::test]` is
    // load-bearing rather than cosmetic: the override lives in a task-local, so
    // a plain `#[test]` has no runtime to resolve it from, the resolver falls
    // back to the process-global gauge, and the body would silently run on the
    // shared counter every other OR-remove in the crate is also writing to —
    // with nothing failing to compile to say so.

    #[tokio::test]
    async fn add_tombstone_bytes_is_monotonic_and_visible_via_accessor() {
        let ((), delta) = with_isolated_gauge(async {
            add_tombstone_bytes(5);
            assert_eq!(
                tombstone_bytes(),
                5,
                "add_tombstone_bytes must increase the gauge by exactly n"
            );

            add_tombstone_bytes(3);
            assert_eq!(
                tombstone_bytes(),
                8,
                "successive adds must accumulate monotonically"
            );
        })
        .await;

        assert_eq!(delta, 8, "the scope's net delta is the sum of its adds");
    }

    #[tokio::test]
    async fn sub_tombstone_bytes_decrements_the_gauge() {
        // Add before subtracting so this exercises the add→sub pairing rather
        // than an underflow off an empty sink.
        let ((), delta) = with_isolated_gauge(async {
            add_tombstone_bytes(10);
            sub_tombstone_bytes(4);
            assert_eq!(
                tombstone_bytes(),
                6,
                "sub_tombstone_bytes must decrease the gauge by exactly n"
            );
        })
        .await;

        assert_eq!(delta, 6, "add(10) then sub(4) nets to 6");
    }

    /// Serializing/deserializing an `OrMap` `RecordValue` must not touch the
    /// gauge — the gauge is updated only by explicit call sites at the
    /// `OR_REMOVE` tombstone-push / inbound-sync-union points (later task
    /// groups), never as a side effect of encoding.
    #[tokio::test]
    async fn ormap_serde_round_trip_does_not_touch_gauge() {
        let ((), delta) = with_isolated_gauge(async {
            let value = RecordValue::OrMap {
                records: vec![OrMapEntry {
                    value: Value::String("hello".to_string()),
                    tag: "tag-1".to_string(),
                    timestamp: Timestamp {
                        millis: 100,
                        counter: 0,
                        node_id: "node-a".to_string(),
                    },
                }],
                tombstones: vec!["tag-removed".to_string()],
            };

            let bytes = rmp_serde::to_vec_named(&value).expect("serialize OrMap");
            let _decoded: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize OrMap");

            assert_eq!(
                tombstone_bytes(),
                0,
                "serde round-trip must not mutate the tombstone-bytes gauge"
            );
        })
        .await;

        assert_eq!(
            delta, 0,
            "a serde round-trip contributes nothing to the scope"
        );
    }
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

    /// The boot-warn decision: a non-zero legacy-row count yields a warning
    /// carrying the count; a clean store (count 0) yields none, so startup on a
    /// store with no legacy corpus logs nothing.
    #[test]
    fn legacy_tombstone_warning_fires_only_with_a_nonzero_count() {
        assert_eq!(
            legacy_tombstone_warning(0),
            None,
            "a clean store logs no legacy-corpus warning"
        );

        let warning = legacy_tombstone_warning(3).expect("legacy rows present → warn");
        assert!(
            warning.contains('3'),
            "the warning reports the accurate legacy-row count"
        );
        assert!(
            warning.contains("excluded from GC") && warning.contains("recreate the datastore"),
            "the warning states the clean-slate reclamation posture"
        );
    }
}
