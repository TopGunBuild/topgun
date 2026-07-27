//! Record types for the storage layer.
//!
//! Defines the core data structures stored in [`StorageEngine`](super::StorageEngine):
//! [`Record`], [`RecordMetadata`], [`RecordValue`], and [`OrMapEntry`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::map_data_store::MapDataStore;
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
/// completes (see [`reconcile_tombstone_bytes`], which the server bootstrap
/// invokes at that point): the process-local
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
/// Kept a pure `count -> Option<message>` decision so it is unit-testable
/// independently of [`reconcile_tombstone_bytes`], the boot walk that supplies
/// the count.
#[must_use]
pub fn legacy_tombstone_warning(legacy_row_count: u64) -> Option<String> {
    (legacy_row_count > 0).then(|| {
        format!(
            "legacy tombstone corpus present ({legacy_row_count} row(s)) — excluded from GC; \
             recreate the datastore to reclaim"
        )
    })
}

/// One-time startup reconciliation of the OR-Map tombstone-bytes gauge.
///
/// Runs exactly once, after WAL recovery completes and strictly before the
/// listener accepts connections, for durable backends only. Walks the full
/// persisted primary keyspace via the cursor-batched scan surface, sums
/// `tag.len()` over every tombstone string in every persisted `OrMap` record,
/// and re-baselines the gauge to that absolute total via [`set_tombstone_bytes`].
/// This makes the scraped `topgun_ormap_tombstone_bytes_total` series survive a
/// `kill -9` restart — the gauge otherwise resets to 0 on every process start
/// and never re-counts rehydrated (redb-persisted) tombstones.
///
/// Returns the reconciled total it passed to [`set_tombstone_bytes`], so a
/// caller (or a test) can assert the reconciled sum through this function
/// rather than re-deriving it from a second walk.
///
/// Never runs on any read/write/rehydration path — boot-only, exactly-once. On
/// scan failure it logs a WARN, returns `0` and leaves the process-local gauge
/// at its default (0): reconciliation is a monitoring-accuracy improvement, not
/// a durability invariant, so a failed reconcile degrades to today's
/// process-local behavior rather than aborting startup.
///
/// `pub` because its consumer is the server bootstrap in
/// `src/bin/topgun_server.rs` — a separate `[[bin]]` target that links this
/// crate externally and therefore cannot see `pub(crate)` items. The boot-only
/// contract travels with that exposure and binds every caller: like
/// [`set_tombstone_bytes`], which it re-baselines through, it MUST NEVER be
/// called on the hot read/write path — only once, at boot, before the listener
/// accepts connections. It is not a general-purpose runtime API: it is a full
/// O(keyspace) walk and an absolute gauge re-baseline, both of which are only
/// sound while nothing else is writing.
///
/// Legacy `RecordValue::OrTombstones { tags }` blobs are deliberately excluded
/// from the sum. The live `add_tombstone_bytes` gauge never counts them either:
/// the read path folds their tags into the unified `OrMap.tombstones` view
/// WITHOUT emitting to the gauge, so summing them here would over-count relative
/// to the live gauge this seed re-baselines.
///
/// `list_maps` on the redb backend enumerates primary partitions only, so backup
/// tombstones are excluded — correct for the single-node soak/demo tier this
/// targets; a future cluster/backup-accounting revisit is out of scope here.
pub async fn reconcile_tombstone_bytes(store: Arc<dyn MapDataStore>) -> u64 {
    // Every `tracing` site below keeps the `topgun_server::bootstrap` target even
    // though this walk now lives in the library rather than the binary: the target
    // string is an operator-facing boot-log filter contract, so re-targeting it to
    // this module's path would silently break deployed log filters. The gauge's own
    // tripwire in `tombstone_gauge` already emits under the same target from inside
    // the lib, so this is not a novel arrangement.
    let maps = match store.list_maps().await {
        Ok(maps) => maps,
        Err(err) => {
            tracing::warn!(
                target: "topgun_server::bootstrap",
                error = %err,
                "tombstone-bytes reconciliation skipped: list_maps failed; gauge stays process-local"
            );
            return 0;
        }
    };

    let mut total: u64 = 0;
    let mut records_scanned: u64 = 0;
    // Folded into this existing boot-only walk (no second O(keyspace) scan): the
    // count of legacy tombstone-only blobs, which a clean-slate store never grows
    // but a rehydrated pre-epoch store still carries. Durable backends only — this
    // reconcile runs solely under the redb/postgres startup arm.
    let mut legacy_rows: u64 = 0;

    for map in &maps {
        // Walk the whole map: first batch, then follow the cursor until exhausted
        // so every record is visited regardless of map size. Each batch is itself
        // cost-bounded, keeping resident cost under the TOPGUN_MAX_RAM_MB ceiling —
        // a memory-safety property, not just a startup-latency one.
        let mut batch = match store.scan_values(map, false, 0).await {
            Ok(batch) => batch,
            Err(err) => {
                tracing::warn!(
                    target: "topgun_server::bootstrap",
                    map = %map,
                    error = %err,
                    "tombstone-bytes reconciliation skipped: scan_values failed; gauge stays process-local"
                );
                return 0;
            }
        };

        loop {
            for (_key, value) in &batch.records {
                records_scanned += 1;
                match value {
                    RecordValue::OrMap { tombstones, .. } => {
                        for tag in tombstones {
                            total += tag.len() as u64;
                        }
                    }
                    RecordValue::OrTombstones { .. } => legacy_rows += 1,
                    RecordValue::Lww { .. } => {}
                }
            }

            match batch.next_cursor.take() {
                None => break,
                Some(cursor) => {
                    batch = match store.scan_values_batched(map, false, cursor, 0).await {
                        Ok(next) => next,
                        Err(err) => {
                            tracing::warn!(
                                target: "topgun_server::bootstrap",
                                map = %map,
                                error = %err,
                                "tombstone-bytes reconciliation skipped: scan_values_batched failed; gauge stays process-local"
                            );
                            return 0;
                        }
                    };
                }
            }
        }
    }

    set_tombstone_bytes(total);

    tracing::info!(
        target: "topgun_server::bootstrap",
        tombstone_bytes = total,
        records_scanned = records_scanned,
        maps = maps.len(),
        "OR-Map tombstone-bytes gauge reconciled from persisted keyspace"
    );

    if let Some(warning) = legacy_tombstone_warning(legacy_rows) {
        tracing::warn!(
            target: "topgun_server::bootstrap",
            legacy_tombstone_rows = legacy_rows,
            "{warning}"
        );
    }

    total
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
    use crate::storage::datastores::RedbDataStore;
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

    /// An `OrMap` slot carrying one live entry plus the given tombstones.
    ///
    /// The live entry's own `tag` is never a tombstone, so it must contribute
    /// nothing to the reconciled total — a walk that summed record tags as well
    /// would show up as an over-count.
    fn or_map_with_tombstones(tombstones: Vec<String>) -> RecordValue {
        RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String("live".to_string()),
                tag: "live-tag".to_string(),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "node-a".to_string(),
                },
            }],
            tombstones,
        }
    }

    /// A non-OR record, so the walk's arm selection is exercised over more than
    /// the OR arms alone.
    fn lww_record_value() -> RecordValue {
        RecordValue::Lww {
            value: Value::String("plain".to_string()),
            timestamp: Timestamp {
                millis: 2,
                counter: 0,
                node_id: "node-a".to_string(),
            },
        }
    }

    async fn seed_primary(
        store: &Arc<dyn MapDataStore>,
        map: &str,
        key: &str,
        value: &RecordValue,
    ) {
        store
            .add(map, key, value, 0, 0)
            .await
            .expect("seed a primary record");
    }

    /// The boot reconciliation recomputes the tombstone-byte total from what is
    /// actually persisted, so it is driven here against a real redb keyspace that
    /// has been seeded. A `MapDataStore` double returning an empty `ScanBatch`
    /// would walk nothing and pass while proving nothing.
    ///
    /// The same seeded corpus pins the walk's two deliberate exclusions. Each
    /// probe carries a tag whose length makes the corpus-with-that-probe total
    /// differ from the expected one, so no regression that starts counting it can
    /// leave the assertion coincidentally green:
    ///
    /// - legacy `OrTombstones` blobs are not summed, because the live
    ///   `add_tombstone_bytes` path never counts them either — summing them here
    ///   would over-count relative to the gauge this walk re-baselines;
    /// - backup rows are not summed, because `list_maps` enumerates primary
    ///   tables only and the walk scans each map with `is_backup = false`.
    #[tokio::test]
    async fn reconcile_re_baselines_the_gauge_to_the_durable_primary_ormap_tombstone_sum() {
        // 4 + 6 + 5 — the primary-partition `OrMap` tombstones and nothing else.
        const EXPECTED_TOTAL: u64 = 15;

        let dir = tempfile::tempdir().expect("tempdir");
        let store: Arc<dyn MapDataStore> =
            Arc::new(RedbDataStore::new(dir.path().join("reconcile.redb")).expect("redb open"));

        // Tag lengths are spelled as `repeat` counts rather than literal runs of
        // characters so the expected total cannot drift from a miscount.
        let excluded_legacy = "d".repeat(20);

        // Two maps, so the per-map loop over `list_maps` is genuinely iterated,
        // and one non-OR record, so the walk's arm selection is exercised.
        let counted = or_map_with_tombstones(vec!["a".repeat(4), "b".repeat(6)]);
        seed_primary(&store, "reconcile_a", "k1", &counted).await;
        let counted = or_map_with_tombstones(vec!["c".repeat(5)]);
        seed_primary(&store, "reconcile_b", "k1", &counted).await;
        seed_primary(&store, "reconcile_b", "k2", &lww_record_value()).await;
        let legacy = RecordValue::OrTombstones {
            tags: vec![excluded_legacy.clone()],
        };
        seed_primary(&store, "reconcile_a", "legacy", &legacy).await;
        let backup = or_map_with_tombstones(vec!["e".repeat(9)]);
        store
            .add_backup("reconcile_a", "backup-key", &backup, 0, 0)
            .await
            .expect("seed a backup OrMap row");

        // An exclusion asserted over a corpus that never held the excluded row is
        // vacuous, so both probes are confirmed present before the walk runs.
        match store
            .load("reconcile_a", "legacy")
            .await
            .expect("load the legacy row back")
        {
            Some(RecordValue::OrTombstones { tags }) => assert_eq!(
                tags,
                vec![excluded_legacy],
                "the legacy exclusion probe must be stored with its tag intact"
            ),
            other => panic!("legacy OrTombstones probe did not land: {other:?}"),
        }
        let backup_batch = store
            .scan_values("reconcile_a", true, 0)
            .await
            .expect("scan the backup table");
        assert_eq!(
            backup_batch.records.len(),
            1,
            "the backup exclusion probe must be stored in the backup table"
        );

        // Scoped sink: the walk's `set_tombstone_bytes` is an absolute set, which
        // trips the process-global gauge's tripwire once any test in this binary
        // has fired an add, and a shared counter could also be perturbed by
        // foreign traffic. The returned sink value is the walk's own re-baseline.
        let (returned_total, gauge_after) =
            with_isolated_gauge(reconcile_tombstone_bytes(Arc::clone(&store))).await;

        assert_eq!(
            returned_total, EXPECTED_TOTAL,
            "the reconciled total must be the exact sum of primary-partition \
             OrMap tombstone tag bytes — legacy blobs (20 bytes) and backup rows \
             (9 bytes) excluded, record tags never counted"
        );
        assert_eq!(
            gauge_after, EXPECTED_TOTAL,
            "the walk must re-baseline the gauge to the total it returns, so the \
             returned value and the observed gauge cannot drift apart"
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
