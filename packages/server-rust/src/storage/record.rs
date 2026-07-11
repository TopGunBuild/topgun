//! Record types for the storage layer.
//!
//! Defines the core data structures stored in [`StorageEngine`](super::StorageEngine):
//! [`Record`], [`RecordMetadata`], [`RecordValue`], and [`OrMapEntry`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// Process-global running total of OR-Map tombstone bytes (sum of removed
/// tags' UTF-8 byte lengths currently tracked across all `OrMap.tombstones`
/// sets).
///
/// This is the in-process source of truth behind [`add_tombstone_bytes`],
/// [`sub_tombstone_bytes`], and [`tombstone_bytes`]. It exists to give the
/// unbounded-tombstone-growth soak monitor (and the `/metrics` Prometheus
/// surface) a cheap, lock-free signal of how much tombstone data has
/// accumulated, without walking every resident `OrMap` record on each check.
/// `Relaxed` ordering is sufficient: this is a monitoring counter, not a
/// correctness-critical value guarding any invariant, so no other memory
/// operation needs to be ordered against it.
static OR_TOMBSTONE_BYTES: AtomicU64 = AtomicU64::new(0);

/// Fail-loud tripwire recording whether [`add_tombstone_bytes`] has fired in
/// this process.
///
/// [`set_tombstone_bytes`] pairs an **absolute** `AtomicU64` store with an
/// **additive** Prometheus `increment`. The one-time boot seed is only correct
/// on a fresh recorder: if any `add_tombstone_bytes` landed between recorder
/// install and the boot seed, the `AtomicU64` would still self-correct (it is a
/// store), but the Prometheus counter — the sink the soak harness scrapes —
/// would silently double-count (it is additive). No `add_tombstone_bytes` call
/// site is reachable before `set_ready()` today, so the boot seed is the sole
/// gauge mutator in the recovery→ready window; this flag turns a future refactor
/// that violates that ordering into a loud failure — a `tracing::error!` in every
/// build (the soak harness and production run release, where `debug_assert!` is a
/// no-op) plus a hard `debug_assert!` in debug/test — rather than a silent
/// double-count of the Prometheus counter the harness scrapes. Written with
/// `Release` / read with `Acquire` so the boot seed reliably observes a prior arm
/// even if the two ever run on different threads.
static TOMBSTONE_ADD_FIRED: AtomicBool = AtomicBool::new(false);

/// Adds `n` bytes to the process-global OR-Map tombstone-bytes gauge and
/// emits the same delta to the `topgun_ormap_tombstone_bytes_total`
/// Prometheus counter (mirrors the `topgun_operations_total` emit pattern in
/// `service/middleware/metrics.rs`).
///
/// Call sites pass `tag.len() as u64` — the tombstone's contribution is its
/// removed tag's UTF-8 byte length, which is the direct measure of what grows
/// in `OrMap.tombstones: Vec<String>`. Fixed per-entry allocator overhead is
/// intentionally excluded: the slope, not the absolute total, is the signal
/// this gauge exists to expose, and a constant per-entry factor only scales
/// it without changing what it reveals.
pub fn add_tombstone_bytes(n: u64) {
    // Arm the tripwire that makes the boot-seed dual-write asymmetry fail loud
    // (see TOMBSTONE_ADD_FIRED). A single store on the write path is negligible
    // next to the map mutation that precedes it; `Release` pairs with the boot
    // seed's `Acquire` load so the seed reliably observes this arm cross-thread.
    TOMBSTONE_ADD_FIRED.store(true, Ordering::Release);
    OR_TOMBSTONE_BYTES.fetch_add(n, Ordering::Relaxed);
    metrics::counter!("topgun_ormap_tombstone_bytes_total").increment(n);
}

/// Subtracts `n` bytes from the process-global OR-Map tombstone-bytes gauge.
///
/// This is the decrement path for the future OR-Map epoch-GC prune
/// (TODO-566): once that prune wires in, it will call this function as it
/// drops fully-superseded tombstone epochs. It is intentionally unused on
/// today's additive-only write path (nothing yet prunes tombstones), so it is
/// exercised solely by a unit test to keep it out of `-D warnings` dead-code
/// territory until the prune lands.
///
/// **Prometheus divergence (forward-looking):** unlike [`add_tombstone_bytes`],
/// this function does NOT emit to `topgun_ormap_tombstone_bytes_total` — that
/// metric follows the `_total` *monotonic*-counter convention, so it cannot
/// legally decrease. That is harmless today because usage is purely additive,
/// but once TODO-566's prune starts calling this decrement path, the exported
/// Prometheus series will diverge from (stay flat relative to) the
/// authoritative in-process [`tombstone_bytes`] value. TODO-566 must reconcile
/// the external surface at that point — e.g. switch the exported metric to a
/// gauge, or have consumers (soak harness, dashboards) read the in-process
/// accessor instead of scraping the counter.
///
/// Saturating-safe by construction: the future prune is only ever expected to
/// subtract bytes it previously observed being added, so it should never
/// underflow in practice, but `fetch_sub` wraps on underflow rather than
/// panicking — acceptable for a monitoring counter.
pub fn sub_tombstone_bytes(n: u64) {
    OR_TOMBSTONE_BYTES.fetch_sub(n, Ordering::Relaxed);
}

/// Reads the current value of the process-global OR-Map tombstone-bytes gauge.
///
/// This is the authoritative in-process accessor referenced by the
/// [`sub_tombstone_bytes`] Prometheus-divergence note above, and by the
/// soak-test monitor that watches for unbounded tombstone growth.
#[must_use]
pub fn tombstone_bytes() -> u64 {
    OR_TOMBSTONE_BYTES.load(Ordering::Relaxed)
}

/// Re-baselines the OR-Map tombstone-bytes gauge to an absolute `total`.
///
/// This is the **only** absolute-set path for the gauge. It exists exclusively
/// for the one-time startup reconciliation that runs after WAL recovery
/// completes (see `bin/topgun_server.rs`): both the process-local
/// [`OR_TOMBSTONE_BYTES`] atomic and the exported Prometheus
/// `topgun_ormap_tombstone_bytes_total` counter reset to zero on every process
/// start and never re-count rehydrated (redb-persisted) tombstones, so without
/// this boot seed the scraped series sawtooths back to 0 on every `kill -9`
/// restart and the cross-restart leak becomes invisible. It MUST NEVER be called
/// on the hot read/write path — only once, at boot, before the listener accepts
/// connections.
///
/// It performs BOTH writes, in order, because the two are **separate sinks**:
///  1. `OR_TOMBSTONE_BYTES.store(total)` re-baselines the in-process `AtomicU64`
///     — an absolute store, never `fetch_add`: a per-rehydration increment would
///     reintroduce the eviction double-count the gauge's cardinal rule forbids.
///  2. `metrics::counter!(...).increment(total)` seeds the Prometheus counter,
///     which is a *different* sink living in the process `PrometheusHandle`
///     recorder. The soak harness scrapes THIS counter via `GET /metrics`, not
///     the `AtomicU64`; on a fresh process the recorder's counter starts at
///     0/absent, so a single `increment(total)` from zero lands the exported
///     series at `total` while staying a legal monotonic-from-zero `_total`
///     counter (a Prometheus counter has no `.set`/`.store`). A `.store()`-only
///     path would never reach the sink the harness actually reads.
pub fn set_tombstone_bytes(total: u64) {
    OR_TOMBSTONE_BYTES.store(total, Ordering::Relaxed);
    // The Prometheus increment below is additive; correct only on a fresh-zero
    // recorder. No add_tombstone_bytes site is reachable before this boot seed,
    // so the tripwire must still be un-armed here — otherwise the counter the
    // harness scrapes would silently double-count. Fail loud in EVERY build: the
    // soak harness and production run release, where `debug_assert!` alone is a
    // no-op, so an error log carries the signal there while the debug_assert hard
    // -fails tests.
    let armed = TOMBSTONE_ADD_FIRED.load(Ordering::Acquire);
    if armed {
        tracing::error!(
            target: "topgun_server::bootstrap",
            "set_tombstone_bytes boot seed ran after add_tombstone_bytes — the additive \
             Prometheus counter will double-count; the recovery→ready gauge-window invariant \
             was violated by a reachable pre-set_ready write path"
        );
    }
    debug_assert!(
        !armed,
        "set_tombstone_bytes boot seed ran after add_tombstone_bytes — the additive \
         Prometheus counter would double-count"
    );
    metrics::counter!("topgun_ormap_tombstone_bytes_total").increment(total);
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
    use std::sync::Mutex;

    // `OR_TOMBSTONE_BYTES` is a single process-global static. No production
    // call site exists yet (added by later task groups), so the only writers
    // today are these three tests themselves — but the Rust test harness runs
    // them concurrently on separate threads, and a delta-based assertion in
    // one test can still observe an in-flight mutation from another. A
    // test-local mutex serializes just this module's tests against each
    // other, which is sufficient today; asserting deltas (rather than
    // absolute values) additionally keeps these tests robust once real call
    // sites (G2/G3) start mutating the gauge from other test modules too.
    static TEST_SERIALIZE: Mutex<()> = Mutex::new(());

    #[test]
    fn add_tombstone_bytes_is_monotonic_and_visible_via_accessor() {
        let _guard = TEST_SERIALIZE.lock().unwrap();
        let baseline = tombstone_bytes();

        add_tombstone_bytes(5);
        assert_eq!(
            tombstone_bytes(),
            baseline + 5,
            "add_tombstone_bytes must increase the gauge by exactly n"
        );

        add_tombstone_bytes(3);
        assert_eq!(
            tombstone_bytes(),
            baseline + 8,
            "successive adds must accumulate monotonically"
        );
    }

    #[test]
    fn sub_tombstone_bytes_decrements_the_gauge() {
        let _guard = TEST_SERIALIZE.lock().unwrap();
        // Add first so the subtraction has headroom and cannot underflow
        // relative to this test's own contribution, independent of whatever
        // baseline other parallel tests have left in the global counter.
        add_tombstone_bytes(10);
        let after_add = tombstone_bytes();

        sub_tombstone_bytes(4);
        assert_eq!(
            tombstone_bytes(),
            after_add - 4,
            "sub_tombstone_bytes must decrease the gauge by exactly n"
        );
    }

    /// Serializing/deserializing an `OrMap` `RecordValue` must not touch the
    /// gauge — the gauge is updated only by explicit call sites at the
    /// `OR_REMOVE` tombstone-push / inbound-sync-union points (later task
    /// groups), never as a side effect of encoding.
    #[test]
    fn ormap_serde_round_trip_does_not_touch_gauge() {
        let _guard = TEST_SERIALIZE.lock().unwrap();
        let baseline = tombstone_bytes();

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
            baseline,
            "serde round-trip must not mutate the tombstone-bytes gauge"
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
