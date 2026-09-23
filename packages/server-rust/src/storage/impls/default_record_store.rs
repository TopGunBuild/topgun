//! Per-map-per-partition record store that orchestrates all three storage layers.
//!
//! [`DefaultRecordStore`] is the Layer 2 component in the storage hierarchy.
//! It coordinates the in-memory [`StorageEngine`](crate::storage::StorageEngine)
//! (Layer 1) with the [`MapDataStore`](crate::storage::MapDataStore) (Layer 3),
//! managing metadata, expiry, eviction, and mutation observation.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use crate::storage::engine::{FetchResult, IterationCursor, PutIfAbsentOutcome, StorageEngine};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use crate::storage::record::{Record, RecordMetadata, RecordValue};
use crate::storage::record_store::{
    CallerProvenance, ExpiryPolicy, ExpiryReason, MutateOutcome, RecordStore,
};
use crate::storage::wal::OrDelta;

/// Returns the current wall-clock time as milliseconds since the Unix epoch.
///
/// Millisecond timestamps fit comfortably in i64 until the year 292 million.
#[allow(clippy::cast_possible_truncation)]
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Attempts `update_in_place` makes to materialize a non-resident key before it
/// gives up with a retryable error. An attempt is repeated only when a removal
/// of a key sharing the key's vacancy-generation stripe landed during its load.
const MATERIALIZE_MAX_ATTEMPTS: u32 = 8;

/// Configuration for storage behavior, applied per-RecordStore.
///
/// Controls default TTL, max-idle, and eviction thresholds. Imported from
/// [`factory`](crate::storage::factory) when wiring via `RecordStoreFactory`,
/// or constructed directly for tests.
#[derive(Debug, Clone, Default)]
pub struct StorageConfig {
    /// Default TTL in milliseconds for new records. 0 = no TTL.
    pub default_ttl_millis: u64,
    /// Default max idle time in milliseconds. 0 = no max idle.
    pub default_max_idle_millis: u64,
    /// Maximum number of entries before eviction triggers. 0 = unlimited.
    pub max_entry_count: u64,
}

/// Per-map-per-partition record store that orchestrates all three storage layers.
///
/// Coordinates:
/// - Layer 1 ([`StorageEngine`]): in-memory key-value storage
/// - Layer 3 ([`MapDataStore`]): external persistence backend
/// - [`CompositeMutationObserver`]: mutation notification fan-out
///
/// Provides metadata tracking, TTL/max-idle expiry checks, eviction support,
/// and write-through persistence based on caller provenance.
pub struct DefaultRecordStore {
    name: String,
    partition_id: u32,
    engine: Box<dyn StorageEngine>,
    data_store: Arc<dyn MapDataStore>,
    observer: Arc<CompositeMutationObserver>,
    config: StorageConfig,
}

impl DefaultRecordStore {
    /// Creates a new `DefaultRecordStore` with the given dependencies.
    #[must_use]
    pub fn new(
        name: String,
        partition_id: u32,
        engine: Box<dyn StorageEngine>,
        data_store: Arc<dyn MapDataStore>,
        observer: Arc<CompositeMutationObserver>,
        config: StorageConfig,
    ) -> Self {
        Self {
            name,
            partition_id,
            engine,
            data_store,
            observer,
            config,
        }
    }

    /// Computes `expiration_time` from the expiry policy and config defaults.
    ///
    /// Returns 0 if no TTL applies (the record does not expire based on absolute time).
    fn compute_expiration_time(&self, expiry: &ExpiryPolicy, creation_time: i64) -> i64 {
        let ttl = if expiry.ttl_millis > 0 {
            expiry.ttl_millis
        } else {
            self.config.default_ttl_millis
        };

        if ttl > 0 {
            // TTL values are always reasonable millisecond durations, not near u64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let ttl_signed = ttl as i64;
            creation_time + ttl_signed
        } else {
            0
        }
    }
}

#[async_trait]
impl RecordStore for DefaultRecordStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn partition_id(&self) -> u32 {
        self.partition_id
    }

    // --- Core CRUD ---

    async fn get(&self, key: &str, touch: bool) -> anyhow::Result<Option<Record>> {
        // Step 1: Check engine. A touch stamps the access in place under the
        // key's lock; writing a stamped copy back would overwrite any write that
        // landed between the read and the write-back.
        let resident = if touch {
            self.engine.touch(key, now_millis())
        } else {
            self.engine.get(key)
        };
        if let Some(record) = resident {
            return Ok(Some(record));
        }

        // Step 2: Try loading from data store if non-null. The generation is
        // read BEFORE the load, so a removal of the key that lands during the
        // load makes the insert below refuse the loaded value (TG-OR-007).
        if !self.data_store.is_null() {
            let generation = self.engine.vacancy_generation(key);
            if let Some(value) = self.data_store.load(&self.name, key).await? {
                let now = now_millis();
                let cost = crate::storage::record::estimated_cost(&value) + key.len() as u64;
                // A record loaded from the datastore is already persisted, so it
                // enters the engine clean (last_stored_time = now) and is
                // immediately eligible for re-eviction in the evict→reload cycle.
                let mut metadata = RecordMetadata::new(now, cost);
                metadata.on_store(now);
                let record = Record { value, metadata };
                return Ok(Some(
                    match self
                        .engine
                        .put_if_absent_at(key, record.clone(), generation)
                    {
                        PutIfAbsentOutcome::Inserted => {
                            self.observer.on_load(key, &record, false);
                            record
                        }
                        // A write materialized the key during the load: the
                        // resident is newer than what was loaded.
                        PutIfAbsentOutcome::Resident(resident) => resident,
                        // A removal intervened: answer with what was read, but
                        // never cache it.
                        PutIfAbsentOutcome::Stale => record,
                    },
                ));
            }
        }

        // Step 3: Not found anywhere
        Ok(None)
    }

    fn exists_in_memory(&self, key: &str) -> bool {
        self.engine.contains_key(key)
    }

    async fn put(
        &self,
        key: &str,
        value: RecordValue,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>> {
        let now = now_millis();

        // Step 1: Check if key already exists
        let old_record = self.engine.get(key);

        // Step 2: Create metadata — measure before value is moved into the record
        let cost = crate::storage::record::estimated_cost(&value) + key.len() as u64;
        let metadata = RecordMetadata::new(now, cost);

        // Step 3: Create record. Capture the token from this record's metadata
        // before it is moved — this is the exact token that identifies this write.
        // Never re-allocate or re-read the token off the resident after put(),
        // since a concurrent writer may have already replaced the slot.
        let write_token = metadata.write_token;
        let record = Record { value, metadata };

        // Sanity check: a live write must carry a minted token (>= 1). Token 0 is
        // reserved for Default-constructed/hydrated metadata, which must never
        // enter the engine dirty on this path. We assert our OWN captured token,
        // not the resident's: a concurrent same-key write can replace the slot
        // between our put() and any read, and tokens are minted at new() time
        // rather than put() time, so the resident's token has no ordering
        // relationship to ours — comparing against it would panic spuriously.
        debug_assert_ne!(
            write_token, 0,
            "live write must carry a minted token (>= 1)"
        );

        // Step 4: Put into engine
        self.engine.put(key, record.clone());

        // Step 5: Fire observer notifications
        if let Some(ref old) = old_record {
            self.observer
                .on_update(key, &record, &old.value, &record.value, false);
        } else {
            self.observer.on_put(key, &record, None, false);
        }

        // Step 6: Write-through for Client or CrdtMerge provenance
        if matches!(
            provenance,
            CallerProvenance::Client | CallerProvenance::CrdtMerge
        ) {
            let expiration_time = self.compute_expiration_time(&expiry, now);
            self.data_store
                .add(&self.name, key, &record.value, expiration_time, now)
                .await?;

            // Mark the record clean only if the value was written to a real
            // persistent store (not a no-op null store) AND the token of the
            // resident record still matches the one we persisted. The WAL append
            // + fsync inside add() completes before returning Ok on real backends,
            // so the value is durable when we reach here. The per-write token
            // ensures that only the exact write just persisted is marked clean:
            // a concurrent same-key write in the same millisecond carries a
            // different token and stays dirty until its own persist completes.
            // Mark in place under the engine's per-key lock (mark_stored) — the
            // in-place mark never re-puts the value, so concurrent writes are
            // never clobbered or lost.
            if !self.data_store.is_null() && !self.engine.mark_stored(key, now, write_token) {
                // Record was evicted between the engine put and this mark, or a
                // concurrent write landed and owns the slot. Harmless: the value
                // is durable via add(), and the newer write will mark itself clean
                // when its own persist completes.
                tracing::trace!(
                    key,
                    "mark_stored found no eligible record after persist (evicted or superseded)"
                );
            }
        }

        // Step 7: Return old value
        Ok(old_record.map(|r| r.value))
    }

    /// Mutates the key's record in place, materializing it from the data store
    /// first when it is durable but not resident (TG-OR-007).
    ///
    /// Caller obligation: every caller holds the key's per-key writer across the
    /// whole call — the load, the mutate and the staging of the write-through —
    /// and every whole-key [`remove`](RecordStore::remove) of the key holds the
    /// same writer. The vacancy generation guards against readers and eviction;
    /// only the writer excludes a remove that stages its delete while this write
    /// mutates a still-resident slot and re-stages it over that delete.
    ///
    /// Retries at most `MATERIALIZE_MAX_ATTEMPTS` times when a removal moves the
    /// generation during the load, then returns an error the caller surfaces as
    /// a transient failure (the client re-sends the op).
    async fn update_in_place(
        &self,
        key: &str,
        init: Option<RecordValue>,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
        mutate: &mut (dyn for<'a> FnMut(&'a mut RecordValue) -> MutateOutcome + Send),
    ) -> anyhow::Result<bool> {
        use crate::storage::engine::UpdateInPlaceOutcome;

        let now = now_millis();
        // Cost estimate mirrors put(): value bytes + the key string's heap
        // contribution. For the OrMap arm this is now a cheap structural
        // estimate (no per-op full serialize).
        let cost_of =
            |value: &RecordValue| crate::storage::record::estimated_cost(value) + key.len() as u64;

        // Re-borrow the `&mut (dyn FnMut + Send)` as the plain `&mut dyn FnMut`
        // the engine seam expects, and split the outcome so the engine keeps its
        // `-> bool` closure: only `changed` is what the engine decides on. The
        // witness is stashed in a local instead of travelling through the engine,
        // and the closure's borrow of `witness` ends with the last engine call,
        // which makes the captured delta readable at the write-through below.
        let mut witness: Option<OrDelta> = None;
        let mut engine_mutate = |value: &mut RecordValue| {
            let mutated = mutate(value);
            witness = mutated.witness;
            mutated.changed
        };

        // The durable value loaded to materialize a non-resident key, kept as
        // the pre-image for `on_load` when the materialized record is inserted.
        let mut pre_image: Option<RecordValue> = None;
        let outcome = if self.data_store.is_null() {
            self.engine
                .update_in_place(key, now, init, None, &mut engine_mutate, &cost_of)
        } else {
            // Materialize a durable-but-non-resident key before mutating it, so the
            // write merges into the key's durable value instead of replacing it
            // with a slot built from `init` alone (TG-OR-007). The closure runs at
            // most once per attempt and an attempt that runs it ends the loop:
            // only `Absent` (resident probe) and `Stale` (generation moved during
            // the load) continue, and both return before the closure (TG-OR-001).
            let mut attempts = 0_u32;
            loop {
                if attempts == MATERIALIZE_MAX_ATTEMPTS {
                    metrics::counter!("topgun_update_in_place_materialize_exhausted_total")
                        .increment(1);
                    return Err(anyhow::anyhow!("materialize retries exhausted"));
                }
                attempts += 1;

                let resident =
                    self.engine
                        .update_in_place(key, now, None, None, &mut engine_mutate, &cost_of);
                if !matches!(resident, UpdateInPlaceOutcome::Absent) {
                    break resident;
                }

                // Read the generation BEFORE the load: a removal of the key that
                // lands after this point moves it, and the insert below refuses a
                // value loaded across that removal.
                let generation = self.engine.vacancy_generation(key);
                let loaded = self.data_store.load(&self.name, key).await?;
                if loaded.is_none() && init.is_none() {
                    return Ok(false);
                }
                pre_image.clone_from(&loaded);
                let base = loaded.or_else(|| init.clone());
                let attempt = self.engine.update_in_place(
                    key,
                    now,
                    base,
                    Some(generation),
                    &mut engine_mutate,
                    &cost_of,
                );
                if !matches!(attempt, UpdateInPlaceOutcome::Stale) {
                    break attempt;
                }
            }
        };

        let (record, inserted) = match outcome {
            UpdateInPlaceOutcome::Absent
            | UpdateInPlaceOutcome::Unchanged
            | UpdateInPlaceOutcome::Stale => return Ok(false),
            UpdateInPlaceOutcome::Written { record, inserted } => (record, inserted),
        };

        // Capture the token off the record just written, before any concurrent
        // writer can replace the slot — the exact-identity key for mark_stored.
        let write_token = record.metadata.write_token;

        // Fire observer notifications matching put(): on_put for a fresh insert,
        // on_update for an existing record. The OR observers never read the
        // pre-image (the index observer skips OrMap; search/embedding handle only
        // Lww; merkle/query use the new value), so the new value is passed as the
        // old value rather than re-cloning the resident slot this seam exists to
        // stop churning.
        //
        // A record materialized from the data store is a residency transition
        // followed by a write: `on_load` with the durable pre-image, then
        // `on_update` with the post-image (TG-OR-002), never `on_put` — the key
        // existed before this write.
        //
        // CONTRACT: any observer added later that reads `old_value` for an OrMap
        // record would receive the post-image here, not the true pre-image. That
        // is only safe because this seam is OrMap-only; if a future observer needs
        // the OrMap pre-image, capture a pre-mutation clone in the engine's
        // Occupied arm and thread it through instead of reusing the new value.
        match (inserted, pre_image) {
            (true, Some(pre)) => {
                let loaded_record = Record {
                    value: pre,
                    metadata: record.metadata.clone(),
                };
                self.observer.on_load(key, &loaded_record, false);
                self.observer
                    .on_update(key, &record, &record.value, &record.value, false);
            }
            (true, None) => self.observer.on_put(key, &record, None, false),
            (false, _) => {
                self.observer
                    .on_update(key, &record, &record.value, &record.value, false);
            }
        }

        // Write-through for Client or CrdtMerge provenance — byte-identical to
        // put()'s full-snapshot durable write, so crash recovery is unchanged.
        if matches!(
            provenance,
            CallerProvenance::Client | CallerProvenance::CrdtMerge
        ) {
            let expiration_time = self.compute_expiration_time(&expiry, now);
            // The witness rides alongside the very value it describes, so a store
            // that records deltas can never pair one with a different write. A
            // store that records none takes the defaulted body, which drops the
            // witness and persists exactly the bytes a plain add() would have.
            self.data_store
                .add_with_witness(
                    &self.name,
                    key,
                    &record.value,
                    expiration_time,
                    now,
                    witness.as_ref(),
                )
                .await?;

            // Mark clean in place under the engine's per-key lock only when the
            // resident record is still the exact write we persisted (token match).
            if !self.data_store.is_null() && !self.engine.mark_stored(key, now, write_token) {
                tracing::trace!(
                    key,
                    "mark_stored found no eligible record after in-place persist (evicted or superseded)"
                );
            }
        }

        Ok(true)
    }

    /// Delegates to the backing [`MapDataStore`], the only layer beneath this
    /// store that a witness can reach: the in-place write-through is the sole
    /// place one is forwarded, and it forwards to exactly this store. Asking the
    /// backend directly keeps the answer honest when the backend is swapped,
    /// instead of pinning a constant here that a witness-recording backend would
    /// silently contradict.
    fn or_witness_wanted(&self) -> bool {
        self.data_store.wants_or_witness()
    }

    /// Removes the key durably and from memory (TG-OR-007).
    ///
    /// The durable delete is staged FIRST, so from then on every data-store
    /// load of the key returns `None`; only then is the engine entry removed,
    /// which advances the key's vacancy generation. A reader that hits the
    /// engine before that sees the pre-remove value (the remove has not taken
    /// effect in memory); a reader that loaded the key before the delete was
    /// staged either inserts before the engine removal, which then removes its
    /// copy, or is refused by the generation. If the durable delete fails, the
    /// engine is left untouched.
    ///
    /// Caller obligation: the caller holds the key's per-key writer across the
    /// call, the same writer every in-place write of the key holds (see
    /// [`update_in_place`](RecordStore::update_in_place)), so no in-place write
    /// can mutate the still-resident slot and re-stage it over the delete.
    async fn remove(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>> {
        // Step 1: Stage the durable delete.
        let now = now_millis();
        let _ = provenance; // provenance available for future use
        self.data_store.remove(&self.name, key, now).await?;

        // Step 2: Remove from the engine, advancing the vacancy generation.
        let old_record = self.engine.remove(key);

        // Step 3: Notify observers. A key that was not resident still had its
        // durable row deleted, so observers that track durable keys are told
        // by key; removing a key an observer never held is a no-op there.
        match old_record {
            Some(ref record) => self.observer.on_remove(key, record, false),
            None => self.observer.on_remove_key(key, false),
        }

        // Step 4: Return old value
        Ok(old_record.map(|r| r.value))
    }

    async fn put_backup(
        &self,
        key: &str,
        record: Record,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()> {
        // Step 1: Put into engine
        let old = self.engine.put(key, record.clone());

        // Step 2: Fire observer
        self.observer
            .on_put(key, &record, old.as_ref().map(|r| &r.value), true);

        // Step 3: Write-through for Client or CrdtMerge provenance
        if matches!(
            provenance,
            CallerProvenance::Client | CallerProvenance::CrdtMerge
        ) {
            let now = now_millis();
            self.data_store
                .add_backup(&self.name, key, &record.value, 0, now)
                .await?;
        }

        Ok(())
    }

    async fn remove_backup(&self, key: &str, provenance: CallerProvenance) -> anyhow::Result<()> {
        // Step 1: Remove from engine
        let old = self.engine.remove(key);

        // Step 2: Fire observer if removed
        if let Some(ref record) = old {
            self.observer.on_remove(key, record, true);
        }

        // Step 3: Remove from data store backup
        let now = now_millis();
        let _ = provenance;
        self.data_store.remove_backup(&self.name, key, now).await?;

        Ok(())
    }

    // --- Batch ---

    async fn get_all(&self, keys: &[String]) -> anyhow::Result<Vec<(String, Record)>> {
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(record) = self.get(key, false).await? {
                results.push((key.clone(), record));
            }
        }
        Ok(results)
    }

    // --- Iteration ---

    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String> {
        self.engine.fetch_keys(cursor, size)
    }

    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)> {
        self.engine.fetch_entries(cursor, size)
    }

    fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool) {
        let now = now_millis();
        let _ = is_backup;
        for (key, record) in self.engine.snapshot_iter() {
            // Skip expired entries
            if self.check_expired(&record, now) == ExpiryReason::NotExpired {
                consumer(&key, &record);
            }
        }
    }

    // --- Size and cost ---

    fn size(&self) -> usize {
        self.engine.len()
    }

    fn is_empty(&self) -> bool {
        self.engine.is_empty()
    }

    fn owned_entry_cost(&self) -> u64 {
        self.engine.estimated_cost()
    }

    // --- Expiry ---

    fn has_expired(&self, key: &str, now: i64, _is_backup: bool) -> ExpiryReason {
        let Some(record) = self.engine.get(key) else {
            return ExpiryReason::NotExpired;
        };

        self.check_expired(&record, now)
    }

    fn evict_expired(&self, percentage: u32, now: i64, _is_backup: bool) {
        let snapshot = self.engine.snapshot_iter();
        let total = snapshot.len();
        if total == 0 {
            return;
        }

        // Calculate how many expired entries to remove (percentage of total)
        let max_removals = ((total as u64 * u64::from(percentage)) / 100) as usize;
        if max_removals == 0 {
            return;
        }

        let mut removed = 0_usize;
        for (key, record) in &snapshot {
            if removed >= max_removals {
                break;
            }
            if self.check_expired(record, now) != ExpiryReason::NotExpired {
                if let Some(removed_record) = self.engine.remove(key) {
                    self.observer.on_evict(key, &removed_record, false);
                    removed += 1;
                }
            }
        }
    }

    fn is_expirable(&self) -> bool {
        self.config.default_ttl_millis > 0 || self.config.default_max_idle_millis > 0
    }

    // --- Eviction ---

    fn evict(&self, key: &str, is_backup: bool) -> Option<RecordValue> {
        // Step 1: Remove from engine
        let old_record = self.engine.remove(key);

        // Step 2: Fire observer if removed
        if let Some(ref record) = old_record {
            self.observer.on_evict(key, record, is_backup);

            // Step 3: Log warning if dirty and data store is non-null
            if record.metadata.is_dirty() && !self.data_store.is_null() {
                tracing::warn!(
                    map = %self.name,
                    key = %key,
                    "Dirty record evicted without being flushed; \
                     will be handled by next flush cycle or shutdown"
                );
            }
        }

        // Step 4: Return old value
        old_record.map(|r| r.value)
    }

    fn evict_all(&self, is_backup: bool) -> u32 {
        let snapshot = self.engine.snapshot_iter();
        let mut count = 0_u32;
        for (key, _) in &snapshot {
            if self.evict(key, is_backup).is_some() {
                count = count.saturating_add(1);
            }
        }
        count
    }

    fn should_evict(&self) -> bool {
        self.config.max_entry_count > 0 && self.engine.len() as u64 >= self.config.max_entry_count
    }

    fn evict_lru(&self, target_count: u32, is_backup: bool) -> u32 {
        // Snapshot once to get a stable, mutation-tolerant view of all entries.
        let mut candidates: Vec<(String, Record)> = self
            .engine
            .snapshot_iter()
            .into_iter()
            // Never evict dirty records — evicting a dirty record discards an
            // acked write that has not yet been flushed to the backing store,
            // violating the durability contract (never-evict-dirty invariant).
            .filter(|(_, r)| !r.metadata.is_dirty())
            .collect();

        // Sort ascending by last_access_time so the oldest (least-recently-used)
        // candidates appear first. i64 is Copy + Ord so sort_by_key is idiomatic.
        candidates.sort_by_key(|(_, r)| r.metadata.last_access_time);

        // Evict only up to target_count of the oldest non-dirty candidates. The
        // snapshot can be stale by the time a candidate is removed, so the
        // removal re-checks under the key's lock that the resident is still the
        // clean write the snapshot saw (TG-EVI-001).
        let mut evicted: usize = 0;
        for (key, snapshot) in candidates.into_iter().take(target_count as usize) {
            let snapshot_token = snapshot.metadata.write_token;
            let removed = self.engine.remove_if(&key, &|resident: &Record| {
                !resident.metadata.is_dirty() && resident.metadata.write_token == snapshot_token
            });
            if let Some(record) = removed {
                self.observer.on_evict(&key, &record, is_backup);
                evicted += 1;
            }
        }

        // Saturating cast: usize → u32 so that a very large candidate set
        // (unreachable in practice, but mandated by the trait contract) does not
        // wrap or panic.
        u32::try_from(evicted).unwrap_or(u32::MAX)
    }

    fn dirty_count(&self) -> u64 {
        // Count records whose in-memory state has not yet been flushed to the
        // backing MapDataStore (last_update_time > last_stored_time).
        let count = self
            .engine
            .snapshot_iter()
            .into_iter()
            .filter(|(_, r)| r.metadata.is_dirty())
            .count();
        // Saturating cast: snapshot sizes are far below u64::MAX in practice.
        count as u64
    }

    // --- Lifecycle ---

    fn init(&mut self) {
        // No-op for Phase 3
    }

    fn clear(&self, _is_backup: bool) -> u32 {
        self.observer.on_clear();
        let previous_size = self.engine.len();
        self.engine.clear();
        // Storage sizes are always small enough for u32 in practice
        #[allow(clippy::cast_possible_truncation)]
        let size = previous_size as u32;
        size
    }

    fn reset(&self) {
        self.observer.on_reset();
        self.engine.clear();
    }

    fn destroy(&self) {
        self.observer.on_destroy(false);
        self.engine.destroy();
    }

    // --- MapDataStore integration ---

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        self.data_store.soft_flush().await
    }

    fn storage(&self) -> &dyn StorageEngine {
        &*self.engine
    }

    fn map_data_store(&self) -> &dyn MapDataStore {
        &*self.data_store
    }
}

impl DefaultRecordStore {
    /// Checks whether a record has expired based on store-wide config defaults.
    #[allow(clippy::cast_possible_wrap)]
    fn check_expired(&self, record: &Record, now: i64) -> ExpiryReason {
        // Check TTL: config values are reasonable millisecond durations
        if self.config.default_ttl_millis > 0
            && now - record.metadata.creation_time > self.config.default_ttl_millis as i64
        {
            return ExpiryReason::Ttl;
        }

        // Check max-idle
        if self.config.default_max_idle_millis > 0
            && now - record.metadata.last_access_time > self.config.default_max_idle_millis as i64
        {
            return ExpiryReason::MaxIdle;
        }

        ExpiryReason::NotExpired
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::engines::HashMapStorage;
    use crate::storage::mutation_observer::MutationObserver;

    /// Test observer that counts how many times each method is called.
    #[allow(clippy::struct_field_names)]
    struct CountingObserver {
        put_count: AtomicUsize,
        update_count: AtomicUsize,
        remove_count: AtomicUsize,
        evict_count: AtomicUsize,
        load_count: AtomicUsize,
        clear_count: AtomicUsize,
        reset_count: AtomicUsize,
        destroy_count: AtomicUsize,
    }

    impl CountingObserver {
        fn new() -> Self {
            Self {
                put_count: AtomicUsize::new(0),
                update_count: AtomicUsize::new(0),
                remove_count: AtomicUsize::new(0),
                evict_count: AtomicUsize::new(0),
                load_count: AtomicUsize::new(0),
                clear_count: AtomicUsize::new(0),
                reset_count: AtomicUsize::new(0),
                destroy_count: AtomicUsize::new(0),
            }
        }
    }

    impl MutationObserver for CountingObserver {
        fn on_put(&self, _: &str, _: &Record, _: Option<&RecordValue>, _: bool) {
            self.put_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_update(&self, _: &str, _: &Record, _: &RecordValue, _: &RecordValue, _: bool) {
            self.update_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_remove(&self, _: &str, _: &Record, _: bool) {
            self.remove_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_evict(&self, _: &str, _: &Record, _: bool) {
            self.evict_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_load(&self, _: &str, _: &Record, _: bool) {
            self.load_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_replication_put(&self, _: &str, _: &Record, _: bool) {}
        fn on_clear(&self) {
            self.clear_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_reset(&self) {
            self.reset_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_destroy(&self, _: bool) {
            self.destroy_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn make_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
        }
    }

    fn make_store_with_observer(
        observer: Arc<CountingObserver>,
        config: StorageConfig,
    ) -> DefaultRecordStore {
        let engine = Box::new(HashMapStorage::new());
        let data_store: Arc<dyn MapDataStore> = Arc::new(NullDataStore);
        let composite = Arc::new(CompositeMutationObserver::new(vec![
            observer as Arc<dyn MutationObserver>,
        ]));
        DefaultRecordStore::new(
            "test-map".to_string(),
            0,
            engine,
            data_store,
            composite,
            config,
        )
    }

    fn make_store() -> DefaultRecordStore {
        let engine = Box::new(HashMapStorage::new());
        let data_store: Arc<dyn MapDataStore> = Arc::new(NullDataStore);
        let observer = Arc::new(CompositeMutationObserver::default());
        DefaultRecordStore::new(
            "test-map".to_string(),
            42,
            engine,
            data_store,
            observer,
            StorageConfig::default(),
        )
    }

    // --- AC3: Put-then-get round-trip ---

    #[tokio::test]
    async fn put_then_get_round_trip() {
        let store = make_store();
        let value = make_value("hello");

        let old = store
            .put(
                "key1",
                value.clone(),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        assert!(old.is_none(), "first put should return None");

        let fetched = store.get("key1", false).await.unwrap();
        assert!(fetched.is_some());
        let record = fetched.unwrap();
        match &record.value {
            RecordValue::Lww { value, .. } => {
                assert_eq!(*value, Value::String("hello".to_string()));
            }
            _ => panic!("expected Lww variant"),
        }
    }

    // --- AC3: Put fires on_put observer ---

    #[tokio::test]
    async fn put_fires_on_put_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.update_count.load(Ordering::Relaxed), 0);
    }

    // --- AC3: Update fires on_update observer ---

    #[tokio::test]
    async fn update_fires_on_update_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        // First put
        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Second put on same key = update
        let old = store
            .put(
                "key1",
                make_value("v2"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        assert!(old.is_some(), "second put should return old value");
        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.update_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: Remove fires on_remove observer ---

    #[tokio::test]
    async fn remove_fires_on_remove_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let old = store
            .remove("key1", CallerProvenance::Client)
            .await
            .unwrap();
        assert!(old.is_some());
        assert_eq!(observer.remove_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: Expiry returns Ttl when TTL exceeded ---

    #[tokio::test]
    async fn has_expired_returns_ttl() {
        let config = StorageConfig {
            default_ttl_millis: 1000,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Use a now far in the future (creation_time + 2000ms > 1000ms TTL)
        let far_future = now_millis() + 2000;
        let reason = store.has_expired("key1", far_future, false);
        assert_eq!(reason, ExpiryReason::Ttl);
    }

    // --- AC3: Expiry returns MaxIdle when max-idle exceeded ---

    #[tokio::test]
    async fn has_expired_returns_max_idle() {
        let config = StorageConfig {
            default_ttl_millis: 0,
            default_max_idle_millis: 500,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Use a now far in the future (last_access_time + 1000ms > 500ms max-idle)
        let far_future = now_millis() + 1000;
        let reason = store.has_expired("key1", far_future, false);
        assert_eq!(reason, ExpiryReason::MaxIdle);
    }

    // --- AC3: Expiry returns NotExpired for fresh records ---

    #[tokio::test]
    async fn has_expired_returns_not_expired_for_fresh() {
        let config = StorageConfig {
            default_ttl_millis: 60_000,
            default_max_idle_millis: 30_000,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Check immediately -- should not be expired
        let now = now_millis();
        let reason = store.has_expired("key1", now, false);
        assert_eq!(reason, ExpiryReason::NotExpired);
    }

    // --- AC3: size() and owned_entry_cost() ---

    #[tokio::test]
    async fn size_and_owned_entry_cost_reflect_records() {
        let store = make_store();

        assert_eq!(store.size(), 0);
        assert_eq!(store.owned_entry_cost(), 0);

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        store
            .put(
                "key2",
                make_value("v2"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        assert_eq!(store.size(), 2);
        assert!(
            store.owned_entry_cost() > 0,
            "owned_entry_cost must be non-zero after puts: each record carries \
             a measured serialized-value cost + key length"
        );
    }

    // --- AC3: clear() fires on_clear observer and empties store ---

    #[tokio::test]
    async fn clear_fires_on_clear_and_empties_store() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        store
            .put(
                "key2",
                make_value("v2"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let count = store.clear(false);
        assert_eq!(count, 2);
        assert!(store.is_empty());
        assert_eq!(observer.clear_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: get() with touch=true updates access statistics ---

    #[tokio::test]
    async fn get_with_touch_updates_access_stats() {
        let store = make_store();

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Get the initial record to capture creation time
        let initial = store.get("key1", false).await.unwrap().unwrap();
        let initial_access_time = initial.metadata.last_access_time;
        let initial_hits = initial.metadata.hits;

        // Small delay so the access time measurably changes
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        // Touch the record
        let touched = store.get("key1", true).await.unwrap().unwrap();
        assert!(
            touched.metadata.last_access_time >= initial_access_time,
            "last_access_time should be updated"
        );
        assert_eq!(
            touched.metadata.hits,
            initial_hits + 1,
            "hits should increment"
        );
    }

    // --- Additional coverage ---

    #[tokio::test]
    async fn exists_in_memory_reflects_state() {
        let store = make_store();

        assert!(!store.exists_in_memory("key1"));
        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        assert!(store.exists_in_memory("key1"));

        store
            .remove("key1", CallerProvenance::Client)
            .await
            .unwrap();
        assert!(!store.exists_in_memory("key1"));
    }

    #[test]
    fn name_and_partition_id() {
        let store = make_store();
        assert_eq!(store.name(), "test-map");
        assert_eq!(store.partition_id(), 42);
    }

    #[test]
    fn is_expirable_reflects_config() {
        let store_no_expiry = make_store();
        assert!(!store_no_expiry.is_expirable());

        let config = StorageConfig {
            default_ttl_millis: 1000,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store_with_ttl = make_store_with_observer(Arc::new(CountingObserver::new()), config);
        assert!(store_with_ttl.is_expirable());
    }

    #[test]
    fn should_evict_reflects_config() {
        let store = make_store();
        assert!(
            !store.should_evict(),
            "unlimited config should not trigger eviction"
        );

        let config = StorageConfig {
            default_ttl_millis: 0,
            default_max_idle_millis: 0,
            max_entry_count: 2,
        };
        let store_limited = make_store_with_observer(Arc::new(CountingObserver::new()), config);
        assert!(
            !store_limited.should_evict(),
            "empty store should not trigger eviction"
        );
    }

    #[tokio::test]
    async fn evict_removes_and_fires_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let evicted = store.evict("key1", false);
        assert!(evicted.is_some());
        assert_eq!(observer.evict_count.load(Ordering::Relaxed), 1);
        assert!(!store.exists_in_memory("key1"));
    }

    #[tokio::test]
    async fn evict_all_removes_all_entries() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "a",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        store
            .put(
                "b",
                make_value("v2"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        store
            .put(
                "c",
                make_value("v3"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let count = store.evict_all(false);
        assert_eq!(count, 3);
        assert!(store.is_empty());
        assert_eq!(observer.evict_count.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn reset_fires_on_reset_and_clears() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        store.reset();
        assert!(store.is_empty());
        assert_eq!(observer.reset_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn destroy_fires_on_destroy() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        store.destroy();
        assert_eq!(observer.destroy_count.load(Ordering::Relaxed), 1);
    }

    /// Verifies `DefaultRecordStore` compiles as `Box<dyn RecordStore>`.
    #[test]
    fn default_record_store_is_object_safe() {
        let store = make_store();
        let _boxed: Box<dyn RecordStore> = Box::new(store);
    }

    #[tokio::test]
    async fn get_all_returns_matching_entries() {
        let store = make_store();

        store
            .put(
                "a",
                make_value("va"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();
        store
            .put(
                "b",
                make_value("vb"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let results = store
            .get_all(&["a".to_string(), "b".to_string(), "missing".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn has_expired_returns_not_expired_for_missing_key() {
        let store = make_store();
        let reason = store.has_expired("nonexistent", now_millis(), false);
        assert_eq!(reason, ExpiryReason::NotExpired);
    }

    #[tokio::test]
    async fn for_each_boxed_skips_expired() {
        let config = StorageConfig {
            default_ttl_millis: 100,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put(
                "key1",
                make_value("v1"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        // Wait for TTL to expire
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let mut count = 0_usize;
        store.for_each_boxed(&mut |_key, _record| count += 1, false);
        assert_eq!(count, 0, "expired entries should be skipped");
    }

    // --- Helper: build a clean record with a specific last_access_time ---
    //
    // A clean record has last_stored_time >= last_update_time so that is_dirty()
    // returns false. This lets us verify evict_lru skips dirty records and
    // correctly selects the oldest clean ones.
    fn make_clean_record(last_access_time: i64) -> Record {
        let metadata = RecordMetadata {
            version: 1,
            creation_time: 1,
            last_access_time,
            last_update_time: 1,
            last_stored_time: 1, // equal to last_update_time => not dirty
            hits: 0,
            cost: 0,
            write_token: 0, // test helper only — not used on the mark_stored path
            stored_token: 0,
        };
        Record {
            value: make_value("clean"),
            metadata,
        }
    }

    // --- Helper: build a dirty record ---
    //
    // A dirty record has last_update_time > last_stored_time. Records created
    // via store.put() are already dirty (last_stored_time=0, last_update_time=now).
    // This constructor makes the dirty state explicit for clarity in tests.
    fn make_dirty_record() -> Record {
        let metadata = RecordMetadata {
            version: 1,
            creation_time: 1,
            last_access_time: 1,
            last_update_time: 1000,
            last_stored_time: 0, // never stored => dirty
            hits: 0,
            cost: 0,
            // The current write differs from the (absent) stored one => dirty.
            write_token: 1,
            stored_token: 0,
        };
        Record {
            value: make_value("dirty"),
            metadata,
        }
    }

    // --- AC #1: evict_lru(0, false) returns 0 ---

    #[test]
    fn evict_lru_zero_target_returns_zero() {
        let store = make_store();
        // Insert a clean record; evict_lru(0) must still return 0.
        store.storage().put("k1", make_clean_record(1000));
        let evicted = store.evict_lru(0, false);
        assert_eq!(evicted, 0, "target_count=0 must evict nothing");
    }

    // --- AC #1 + #1: evict_lru(N) never returns more than N ---

    #[test]
    fn evict_lru_never_exceeds_target_count() {
        let store = make_store();
        // Insert 5 clean records; request eviction of only 3.
        for i in 0..5_u32 {
            store
                .storage()
                .put(&format!("k{i}"), make_clean_record(i64::from(i) + 100));
        }
        let evicted = store.evict_lru(3, false);
        assert!(
            evicted <= 3,
            "evict_lru must never return more than target_count; got {evicted}"
        );
    }

    // --- AC #2: never-evict-dirty invariant ---
    //
    // With all records dirty, evict_lru must return 0 regardless of target.

    #[test]
    fn evict_lru_skips_all_dirty_records() {
        let store = make_store();
        // Insert 5 dirty records.
        for i in 0..5_u32 {
            store.storage().put(&format!("k{i}"), make_dirty_record());
        }
        let evicted = store.evict_lru(100, false);
        assert_eq!(
            evicted, 0,
            "dirty records must never be evicted; expected 0 but got {evicted}"
        );
    }

    // --- AC #3: LRU order — oldest last_access_time evicted first ---

    #[test]
    fn evict_lru_selects_oldest_access_time_first() {
        let store = make_store();
        // Insert 3 clean records with distinct last_access_time values.
        // k_old: access time 100 (oldest), k_mid: 200, k_new: 300 (newest).
        store.storage().put("k_old", make_clean_record(100));
        store.storage().put("k_mid", make_clean_record(200));
        store.storage().put("k_new", make_clean_record(300));

        // Evict only 1 — must be the oldest.
        let evicted = store.evict_lru(1, false);
        assert_eq!(evicted, 1, "expected exactly 1 eviction");
        assert!(
            !store.exists_in_memory("k_old"),
            "k_old (oldest access time) must be evicted first"
        );
        // The two newer records must remain.
        assert!(store.exists_in_memory("k_mid"), "k_mid must survive");
        assert!(store.exists_in_memory("k_new"), "k_new must survive");
    }

    // --- AC #3 (mixed dirty/clean): only clean records are selected ---

    #[test]
    fn evict_lru_skips_dirty_in_mixed_snapshot() {
        let store = make_store();
        // dirty record with "older" timestamp — must be skipped.
        let mut dirty = make_dirty_record();
        dirty.metadata.last_access_time = 50;
        store.storage().put("k_dirty", dirty);

        // Two clean records.
        store.storage().put("k_clean_old", make_clean_record(100));
        store.storage().put("k_clean_new", make_clean_record(200));

        // Evict 1 — must pick the oldest CLEAN record, not the dirty one.
        let evicted = store.evict_lru(1, false);
        assert_eq!(evicted, 1);
        assert!(
            store.exists_in_memory("k_dirty"),
            "dirty record must not be evicted"
        );
        assert!(
            !store.exists_in_memory("k_clean_old"),
            "oldest clean record must be evicted"
        );
        assert!(
            store.exists_in_memory("k_clean_new"),
            "newer clean record must survive"
        );
    }

    // --- AC #4: dirty_count matches manual iteration ---

    #[test]
    fn dirty_count_matches_manual_count() {
        let store = make_store();
        // Insert 3 dirty + 2 clean records.
        for i in 0..3_u32 {
            store
                .storage()
                .put(&format!("dirty_{i}"), make_dirty_record());
        }
        for i in 0..2_u32 {
            store
                .storage()
                .put(&format!("clean_{i}"), make_clean_record(1000));
        }

        // Manual count via snapshot.
        let manual: u64 = store
            .storage()
            .snapshot_iter()
            .iter()
            .filter(|(_, r)| r.metadata.is_dirty())
            .count() as u64;

        assert_eq!(
            store.dirty_count(),
            manual,
            "dirty_count() must match manual iteration count"
        );
        assert_eq!(store.dirty_count(), 3, "expected exactly 3 dirty records");
    }

    // --- AC #5: evict_lru fires on_evict via self.evict() per record ---

    #[test]
    fn evict_lru_fires_evict_observer_per_record() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        // Insert 4 clean records.
        for i in 0..4_u32 {
            store.storage().put(
                &format!("k{i}"),
                make_clean_record(i64::from(i) * 100 + 100),
            );
        }

        // Evict 3 of the 4 clean records.
        let evicted = store.evict_lru(3, false);
        assert_eq!(evicted, 3, "expected 3 evictions");

        // The observer must have received exactly 3 on_evict calls, one per
        // record removed, because evict_lru delegates to self.evict() per entry.
        assert_eq!(
            observer.evict_count.load(Ordering::Relaxed),
            3,
            "on_evict must fire exactly once per evicted record"
        );
    }

    // --- AC #9: saturating-cast — evict_lru(u32::MAX) on small snapshot returns snapshot len ---

    #[test]
    fn evict_lru_saturating_cast_does_not_panic() {
        let store = make_store();
        // Insert a small number of clean records.
        for i in 0..5_u32 {
            store
                .storage()
                .put(&format!("k{i}"), make_clean_record(i64::from(i) * 100 + 1));
        }

        // Call with u32::MAX — iterator exhausts before target_count is reached.
        // Must return the actual count (5) without panicking.
        let evicted = store.evict_lru(u32::MAX, false);
        assert_eq!(
            evicted, 5,
            "must return the actual eviction count, not wrap or panic"
        );
        assert!(store.is_empty(), "all clean records must be evicted");
    }

    /// Writes on a key that is durable but not resident, and the races around
    /// materializing it (TG-OR-007, TG-OR-002, TG-EVI-001).
    mod materialize {
        use std::sync::Mutex;
        use std::time::Duration;

        use crate::storage::engine::{
            FetchResult as EngineFetch, PutIfAbsentOutcome, UpdateInPlaceOutcome,
        };
        use crate::storage::map_data_store::{LeafSink, ScanBatch, ScanCursor};

        use super::*;
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::map_data_store::merkle_leaf_hash;
        use crate::storage::merkle_sync::{MerkleMutationObserver, MerkleSyncManager};
        use crate::storage::record::OrMapEntry;
        use crate::storage::record_store::MutateOutcome;

        pub(super) const MAP: &str = "materialize_map";
        pub(super) const KEY: &str = "doc";

        pub(super) fn redb(dir: &tempfile::TempDir) -> Arc<dyn MapDataStore> {
            Arc::new(RedbDataStore::new(dir.path().join("materialize.redb")).expect("redb open"))
        }

        pub(super) fn entry(tag: &str) -> OrMapEntry {
            OrMapEntry {
                value: Value::String(format!("v-{tag}")),
                tag: tag.to_string(),
                timestamp: Timestamp {
                    millis: 1_000_000,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            }
        }

        pub(super) fn or_value(tags: &[&str], tombs: &[&str]) -> RecordValue {
            RecordValue::OrMap {
                records: tags.iter().map(|t| entry(t)).collect(),
                tombstones: tombs.iter().map(|t| (*t).to_string()).collect(),
            }
        }

        pub(super) fn store_over(
            data_store: Arc<dyn MapDataStore>,
            engine: Box<dyn StorageEngine>,
            observers: Vec<Arc<dyn MutationObserver>>,
        ) -> DefaultRecordStore {
            DefaultRecordStore::new(
                MAP.to_string(),
                0,
                engine,
                data_store,
                Arc::new(CompositeMutationObserver::new(observers)),
                StorageConfig::default(),
            )
        }

        /// An in-place OR add of `tag`, shaped like the CRDT service's `OR_ADD`
        /// (empty `OrMap` `init`, closure appends the entry).
        pub(super) async fn or_add(store: &DefaultRecordStore, tag: &str) -> anyhow::Result<bool> {
            let new_entry = entry(tag);
            let mut add = |value: &mut RecordValue| {
                if let RecordValue::OrMap { records, .. } = value {
                    if !records.iter().any(|e| e.tag == new_entry.tag) {
                        records.push(new_entry.clone());
                    }
                }
                MutateOutcome {
                    changed: true,
                    witness: None,
                }
            };
            store
                .update_in_place(
                    KEY,
                    Some(or_value(&[], &[])),
                    ExpiryPolicy::NONE,
                    CallerProvenance::CrdtMerge,
                    &mut add,
                )
                .await
        }

        pub(super) fn tags_of(value: &RecordValue) -> Vec<String> {
            match value {
                RecordValue::OrMap { records, .. } => {
                    let mut tags: Vec<String> = records.iter().map(|e| e.tag.clone()).collect();
                    tags.sort();
                    tags
                }
                other => panic!("not an OrMap: {other:?}"),
            }
        }

        /// The durable tag set of `KEY`, or `None` when no durable row exists.
        pub(super) async fn durable_tags(ds: &Arc<dyn MapDataStore>) -> Option<Vec<String>> {
            ds.load(MAP, KEY)
                .await
                .expect("load durable row")
                .map(|v| tags_of(&v))
        }

        pub(super) fn strings(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| (*s).to_string()).collect()
        }

        /// Records the order of observer callbacks.
        #[derive(Default)]
        pub(super) struct OrderObserver {
            pub(super) events: Mutex<Vec<&'static str>>,
        }

        impl OrderObserver {
            fn push(&self, event: &'static str) {
                self.events.lock().unwrap().push(event);
            }

            pub(super) fn take(&self) -> Vec<&'static str> {
                std::mem::take(&mut *self.events.lock().unwrap())
            }
        }

        impl MutationObserver for OrderObserver {
            fn on_put(&self, _: &str, _: &Record, _: Option<&RecordValue>, _: bool) {
                self.push("put");
            }
            fn on_update(&self, _: &str, _: &Record, _: &RecordValue, _: &RecordValue, _: bool) {
                self.push("update");
            }
            fn on_remove(&self, _: &str, _: &Record, _: bool) {
                self.push("remove");
            }
            fn on_evict(&self, _: &str, _: &Record, _: bool) {
                self.push("evict");
            }
            fn on_load(&self, _: &str, _: &Record, _: bool) {
                self.push("load");
            }
            fn on_replication_put(&self, _: &str, _: &Record, _: bool) {}
            fn on_clear(&self) {}
            fn on_reset(&self) {}
            fn on_destroy(&self, _: bool) {}
        }

        /// How long a test waits for a double to park before it declares that
        /// the setup never reached the park point.
        pub(crate) const PARK_BOUND: Duration = Duration::from_secs(2);

        /// The parked side of a one-shot async gate.
        pub(crate) struct Gate {
            parked: tokio::sync::oneshot::Sender<()>,
            release: tokio::sync::oneshot::Receiver<()>,
        }

        impl Gate {
            async fn pass(self) {
                let _ = self.parked.send(());
                let _ = self.release.await;
            }
        }

        /// The test's side of a one-shot async gate. Dropping it releases the
        /// parked caller, so a failing test never leaves a task parked.
        pub(crate) struct GateHandle {
            parked: Option<tokio::sync::oneshot::Receiver<()>>,
            release: Option<tokio::sync::oneshot::Sender<()>>,
        }

        impl GateHandle {
            /// Waits until the caller parks; panics after [`PARK_BOUND`].
            pub(crate) async fn wait_parked(&mut self) {
                let parked = self.parked.take().expect("wait_parked called once");
                tokio::time::timeout(PARK_BOUND, parked)
                    .await
                    .expect("the double never parked: the setup did not reach the park point")
                    .expect("gate dropped before parking");
            }

            pub(crate) fn release(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
            }
        }

        fn gate() -> (Gate, GateHandle) {
            let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            (
                Gate {
                    parked: parked_tx,
                    release: release_rx,
                },
                GateHandle {
                    parked: Some(parked_rx),
                    release: Some(release_tx),
                },
            )
        }

        /// A data store that forwards to `inner` and can park ONE call, armed
        /// just before the targeted caller runs: on RETURN from `load` (holding
        /// the loaded value) or on ENTRY to `remove` (before it delegates). A park is consumed by the
        /// first matching call, so any later call passes unparked.
        pub(crate) struct ParkingStore {
            inner: Arc<dyn MapDataStore>,
            after_load: Mutex<Option<Gate>>,
            before_remove: Mutex<Option<Gate>>,
        }

        impl ParkingStore {
            pub(crate) fn new(inner: Arc<dyn MapDataStore>) -> Self {
                Self {
                    inner,
                    after_load: Mutex::new(None),
                    before_remove: Mutex::new(None),
                }
            }

            fn arm(slot: &Mutex<Option<Gate>>) -> GateHandle {
                let (parked, handle) = gate();
                *slot.lock().unwrap() = Some(parked);
                handle
            }

            pub(crate) fn park_after_load(&self) -> GateHandle {
                Self::arm(&self.after_load)
            }

            pub(crate) fn park_before_remove(&self) -> GateHandle {
                Self::arm(&self.before_remove)
            }

            fn take(slot: &Mutex<Option<Gate>>) -> Option<Gate> {
                slot.lock().unwrap().take()
            }
        }

        #[async_trait]
        impl MapDataStore for ParkingStore {
            async fn add(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                self.inner.add(map, key, value, expiration_time, now).await
            }

            async fn add_backup(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                self.inner
                    .add_backup(map, key, value, expiration_time, now)
                    .await
            }

            async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                if let Some(parked) = Self::take(&self.before_remove) {
                    parked.pass().await;
                }
                self.inner.remove(map, key, now).await
            }

            async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                self.inner.remove_backup(map, key, now).await
            }

            async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
                let loaded = self.inner.load(map, key).await;
                if let Some(parked) = Self::take(&self.after_load) {
                    parked.pass().await;
                }
                loaded
            }

            async fn load_all(
                &self,
                map: &str,
                keys: &[String],
            ) -> anyhow::Result<Vec<(String, RecordValue)>> {
                self.inner.load_all(map, keys).await
            }

            async fn enumerate_leaves(
                &self,
                map: &str,
                is_backup: bool,
                sink: &mut dyn LeafSink,
            ) -> anyhow::Result<()> {
                self.inner.enumerate_leaves(map, is_backup, sink).await
            }

            async fn scan_values(
                &self,
                map: &str,
                is_backup: bool,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner.scan_values(map, is_backup, max_batch_cost).await
            }

            async fn scan_values_batched(
                &self,
                map: &str,
                is_backup: bool,
                cursor: ScanCursor,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner
                    .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                    .await
            }

            async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
                self.inner.remove_all(map, keys).await
            }

            async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
                self.inner.list_maps().await
            }

            fn is_loadable(&self, key: &str) -> bool {
                self.inner.is_loadable(key)
            }

            fn pending_operation_count(&self) -> u64 {
                self.inner.pending_operation_count()
            }

            async fn soft_flush(&self) -> anyhow::Result<u64> {
                self.inner.soft_flush().await
            }

            async fn hard_flush(&self) -> anyhow::Result<()> {
                self.inner.hard_flush().await
            }

            async fn flush_key(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                is_backup: bool,
            ) -> anyhow::Result<()> {
                self.inner.flush_key(map, key, value, is_backup).await
            }

            fn reset(&self) {
                self.inner.reset();
            }
        }

        /// The parked side of a one-shot blocking gate, for the synchronous
        /// engine calls. It parks on ENTRY to the engine method, before any
        /// entry lock is taken.
        struct SyncGate {
            parked: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        }

        /// The test's side of a [`SyncGate`]. Dropping it releases the caller.
        pub(crate) struct SyncGateHandle {
            parked: std::sync::mpsc::Receiver<()>,
            release: Option<std::sync::mpsc::Sender<()>>,
        }

        impl SyncGateHandle {
            /// Blocks until the caller parks; panics after [`PARK_BOUND`].
            pub(crate) fn wait_parked(&self) {
                self.parked.recv_timeout(PARK_BOUND).expect(
                    "the engine double never parked: the setup did not reach the park point",
                );
            }

            pub(crate) fn release(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
            }
        }

        #[derive(Default)]
        pub(crate) struct EngineParks {
            removal: Mutex<Option<SyncGate>>,
            write_back: Mutex<Option<SyncGate>>,
        }

        impl EngineParks {
            fn arm(slot: &Mutex<Option<SyncGate>>) -> SyncGateHandle {
                let (parked_tx, parked_rx) = std::sync::mpsc::channel();
                let (release_tx, release_rx) = std::sync::mpsc::channel();
                *slot.lock().unwrap() = Some(SyncGate {
                    parked: parked_tx,
                    release: release_rx,
                });
                SyncGateHandle {
                    parked: parked_rx,
                    release: Some(release_tx),
                }
            }

            /// Parks the next `remove` or `remove_if`.
            pub(crate) fn park_removal(&self) -> SyncGateHandle {
                Self::arm(&self.removal)
            }

            /// Parks the next `put` or `touch`.
            pub(crate) fn park_write_back(&self) -> SyncGateHandle {
                Self::arm(&self.write_back)
            }

            fn pass(slot: &Mutex<Option<SyncGate>>) {
                let parked = slot.lock().unwrap().take();
                if let Some(parked) = parked {
                    let _ = parked.parked.send(());
                    let _ = parked.release.recv();
                }
            }
        }

        /// A [`HashMapStorage`] that can park on entry to a removal or to a
        /// value write-back, before delegating.
        pub(crate) struct ParkingEngine {
            inner: HashMapStorage,
            parks: Arc<EngineParks>,
        }

        impl ParkingEngine {
            pub(crate) fn new() -> (Self, Arc<EngineParks>) {
                let parks = Arc::new(EngineParks::default());
                (
                    Self {
                        inner: HashMapStorage::new(),
                        parks: Arc::clone(&parks),
                    },
                    parks,
                )
            }
        }

        impl StorageEngine for ParkingEngine {
            fn put(&self, key: &str, record: Record) -> Option<Record> {
                EngineParks::pass(&self.parks.write_back);
                self.inner.put(key, record)
            }

            fn get(&self, key: &str) -> Option<Record> {
                self.inner.get(key)
            }

            fn mark_stored(&self, key: &str, now: i64, token: u64) -> bool {
                self.inner.mark_stored(key, now, token)
            }

            fn update_in_place(
                &self,
                key: &str,
                now: i64,
                init: Option<RecordValue>,
                init_generation: Option<u64>,
                mutate: &mut dyn FnMut(&mut RecordValue) -> bool,
                cost_of: &dyn Fn(&RecordValue) -> u64,
            ) -> UpdateInPlaceOutcome {
                self.inner
                    .update_in_place(key, now, init, init_generation, mutate, cost_of)
            }

            fn vacancy_generation(&self, key: &str) -> u64 {
                self.inner.vacancy_generation(key)
            }

            fn put_if_absent_at(
                &self,
                key: &str,
                record: Record,
                generation: u64,
            ) -> PutIfAbsentOutcome {
                self.inner.put_if_absent_at(key, record, generation)
            }

            fn remove(&self, key: &str) -> Option<Record> {
                EngineParks::pass(&self.parks.removal);
                self.inner.remove(key)
            }

            fn remove_if(&self, key: &str, predicate: &dyn Fn(&Record) -> bool) -> Option<Record> {
                EngineParks::pass(&self.parks.removal);
                self.inner.remove_if(key, predicate)
            }

            fn touch(&self, key: &str, now: i64) -> Option<Record> {
                EngineParks::pass(&self.parks.write_back);
                self.inner.touch(key, now)
            }

            fn contains_key(&self, key: &str) -> bool {
                self.inner.contains_key(key)
            }

            fn len(&self) -> usize {
                self.inner.len()
            }

            fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            fn clear(&self) {
                self.inner.clear();
            }

            fn destroy(&self) {
                self.inner.destroy();
            }

            fn estimated_cost(&self) -> u64 {
                self.inner.estimated_cost()
            }

            fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> EngineFetch<String> {
                self.inner.fetch_keys(cursor, size)
            }

            fn fetch_entries(
                &self,
                cursor: &IterationCursor,
                size: usize,
            ) -> EngineFetch<(String, Record)> {
                self.inner.fetch_entries(cursor, size)
            }

            fn snapshot_iter(&self) -> Vec<(String, Record)> {
                self.inner.snapshot_iter()
            }

            fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)> {
                self.inner.random_samples(sample_count)
            }
        }

        // AC-4: the materialize path is a residency transition then a write.
        #[tokio::test]
        async fn materialize_fires_on_load_then_on_update() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            ds.add(MAP, KEY, &or_value(&["a", "b"], &["x"]), 0, 0)
                .await
                .expect("seed");
            let order = Arc::new(OrderObserver::default());
            let store = store_over(
                Arc::clone(&ds),
                Box::new(HashMapStorage::new()),
                vec![order.clone() as Arc<dyn MutationObserver>],
            );

            assert!(or_add(&store, "c").await.expect("or_add"));

            assert_eq!(order.take(), vec!["load", "update"]);
            assert_eq!(durable_tags(&ds).await, Some(strings(&["a", "b", "c"])));

            // A key absent everywhere keeps `on_put`.
            let fresh = store_over(
                redb(&tempfile::tempdir().expect("tempdir")),
                Box::new(HashMapStorage::new()),
                vec![order.clone() as Arc<dyn MutationObserver>],
            );
            assert!(or_add(&fresh, "c").await.expect("or_add"));
            assert_eq!(order.take(), vec!["put"]);
        }

        // AC-4: the in-memory Merkle leaf after a materializing write is the
        // leaf of old ∪ op, and equals the leaf of the durable value.
        #[tokio::test]
        async fn materialize_merkle_leaf_is_the_leaf_of_old_union_op() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            ds.add(MAP, KEY, &or_value(&["a", "b"], &["x"]), 0, 0)
                .await
                .expect("seed");
            let manager = Arc::new(MerkleSyncManager::new(3));
            let merkle = Arc::new(MerkleMutationObserver::new(
                Arc::clone(&manager),
                MAP.to_string(),
                0,
            ));
            let store = store_over(
                Arc::clone(&ds),
                Box::new(HashMapStorage::new()),
                vec![merkle as Arc<dyn MutationObserver>],
            );

            assert!(or_add(&store, "c").await.expect("or_add"));

            let expected_union = or_value(&["a", "b", "c"], &["x"]);
            let expected_leaf = merkle_leaf_hash(KEY, &expected_union)
                .expect("OrMap yields a leaf")
                .1;
            let reference = MerkleSyncManager::new(3);
            reference.update_ormap(MAP, 0, KEY, expected_leaf);
            assert_eq!(
                manager.aggregate_ormap_root_hash(MAP),
                reference.aggregate_ormap_root_hash(MAP),
                "in-memory OR leaf must be the leaf of old ∪ op"
            );
            let durable = ds.load(MAP, KEY).await.expect("load").expect("durable row");
            assert_eq!(
                merkle_leaf_hash(KEY, &durable)
                    .expect("OrMap yields a leaf")
                    .1,
                expected_leaf,
                "durable leaf must equal the in-memory leaf"
            );
        }

        // AC-5: a reader that loaded `D` before a write materialized, persisted
        // and evicted `D+op` must not cache its stale `D` (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn reader_does_not_cache_a_load_that_an_eviction_superseded() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            ds.add(MAP, KEY, &or_value(&["a", "b"], &[]), 0, 0)
                .await
                .expect("seed");
            let parking = Arc::new(ParkingStore::new(Arc::clone(&ds)));
            let store = Arc::new(store_over(
                parking.clone(),
                Box::new(HashMapStorage::new()),
                Vec::new(),
            ));

            let mut reader_park = parking.park_after_load();
            let reader = {
                let store = Arc::clone(&store);
                tokio::spawn(async move { store.get(KEY, false).await })
            };
            reader_park.wait_parked().await;

            assert!(or_add(&store, "op").await.expect("or_add"));
            assert!(
                store.evict_lru(u32::MAX, false) > 0,
                "the materialized, persisted record must be evicted"
            );

            reader_park.release();
            reader.await.expect("reader task").expect("get");

            assert!(
                !store.exists_in_memory(KEY),
                "the reader must not cache the value it loaded before the eviction"
            );
            assert!(or_add(&store, "op2").await.expect("or_add"));
            assert_eq!(
                durable_tags(&ds).await,
                Some(strings(&["a", "b", "op", "op2"])),
                "no acked op may be lost to a stale cached load"
            );
        }

        // AC-6a: a reader that loaded `D` before a REMOVE ran must not cache
        // `D` after the REMOVE completed (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn reader_does_not_cache_a_load_that_a_remove_superseded() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            ds.add(MAP, KEY, &or_value(&["a", "b"], &[]), 0, 0)
                .await
                .expect("seed");
            let parking = Arc::new(ParkingStore::new(Arc::clone(&ds)));
            let store = Arc::new(store_over(
                parking.clone(),
                Box::new(HashMapStorage::new()),
                Vec::new(),
            ));

            let mut reader_park = parking.park_after_load();
            let reader = {
                let store = Arc::clone(&store);
                tokio::spawn(async move { store.get(KEY, false).await })
            };
            reader_park.wait_parked().await;

            store
                .remove(KEY, CallerProvenance::CrdtMerge)
                .await
                .expect("remove");

            reader_park.release();
            reader.await.expect("reader task").expect("get");

            assert!(
                !store.exists_in_memory(KEY),
                "the reader must not cache a value the REMOVE superseded"
            );
            assert!(or_add(&store, "op2").await.expect("or_add"));
            assert_eq!(
                durable_tags(&ds).await,
                Some(strings(&["op2"])),
                "the removed value must not be resurrected"
            );
        }

        // AC-6c: a REMOVE must stage its durable delete before it empties the
        // engine, or a reader in between caches the value being removed.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn reader_between_the_steps_of_a_remove_does_not_resurrect_it() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            ds.add(MAP, KEY, &or_value(&["a", "b"], &[]), 0, 0)
                .await
                .expect("seed");
            let parking = Arc::new(ParkingStore::new(Arc::clone(&ds)));
            let store = Arc::new(store_over(
                parking.clone(),
                Box::new(HashMapStorage::new()),
                Vec::new(),
            ));

            let mut remove_park = parking.park_before_remove();
            let remover = {
                let store = Arc::clone(&store);
                tokio::spawn(async move { store.remove(KEY, CallerProvenance::CrdtMerge).await })
            };
            remove_park.wait_parked().await;

            store.get(KEY, false).await.expect("get");

            remove_park.release();
            remover.await.expect("remover task").expect("remove");

            assert!(or_add(&store, "op2").await.expect("or_add"));
            assert_eq!(
                durable_tags(&ds).await,
                Some(strings(&["op2"])),
                "the removed value must not be resurrected"
            );
        }

        // AC-7: `evict_lru` must not remove a record that a write replaced after
        // the eviction snapshot was taken (TG-EVI-001).
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn evict_lru_keeps_a_record_written_after_its_snapshot() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            let (engine, parks) = ParkingEngine::new();
            let store = Arc::new(store_over(Arc::clone(&ds), Box::new(engine), Vec::new()));
            assert!(or_add(&store, "a").await.expect("or_add"));
            assert!(
                !store
                    .storage()
                    .get(KEY)
                    .expect("resident")
                    .metadata
                    .is_dirty(),
                "precondition: the record is clean, so it is an eviction candidate"
            );

            let mut removal_park = parks.park_removal();
            let evictor = {
                let store = Arc::clone(&store);
                std::thread::spawn(move || store.evict_lru(u32::MAX, false))
            };
            removal_park.wait_parked();

            assert!(or_add(&store, "op").await.expect("or_add"));
            let written = store.storage().get(KEY).expect("the writer's record");

            removal_park.release();
            evictor.join().expect("evictor thread");

            assert!(
                store.exists_in_memory(KEY),
                "a record written after the snapshot must not be evicted"
            );
            let resident = store.storage().get(KEY).expect("resident");
            assert_eq!(tags_of(&resident.value), tags_of(&written.value));
            assert_eq!(resident.metadata.write_token, written.metadata.write_token);
        }

        // AC-8: `get(touch = true)` must not write a stale copy back over a
        // write that landed after its read.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn touch_does_not_overwrite_a_concurrent_write() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            let (engine, parks) = ParkingEngine::new();
            let store = Arc::new(store_over(Arc::clone(&ds), Box::new(engine), Vec::new()));
            assert!(or_add(&store, "a").await.expect("or_add"));

            let mut write_back_park = parks.park_write_back();
            let reader = {
                let store = Arc::clone(&store);
                let runtime = tokio::runtime::Handle::current();
                std::thread::spawn(move || runtime.block_on(store.get(KEY, true)))
            };
            write_back_park.wait_parked();

            assert!(or_add(&store, "op").await.expect("or_add"));
            let written = store.storage().get(KEY).expect("the writer's record");

            write_back_park.release();
            reader
                .join()
                .expect("reader thread")
                .expect("get")
                .expect("resident");

            let resident = store.storage().get(KEY).expect("resident");
            assert_eq!(tags_of(&resident.value), tags_of(&written.value));
            assert_eq!(resident.metadata.write_token, written.metadata.write_token);
        }

        // AC-9 (store half): a write in the same millisecond as the previous
        // `mark_stored` is still dirty, so `evict_lru` keeps it (TG-EVI-001).
        #[test]
        fn a_write_in_the_mark_stored_millisecond_is_not_evicted() {
            let store = make_store();
            let t = 1_000_000;
            let cost = |_: &RecordValue| 1;
            let UpdateInPlaceOutcome::Written { record, .. } = store.storage().update_in_place(
                KEY,
                t,
                Some(or_value(&["a"], &[])),
                None,
                &mut |_| true,
                &cost,
            ) else {
                panic!("the first write must insert");
            };
            assert!(store
                .storage()
                .mark_stored(KEY, t, record.metadata.write_token));

            let UpdateInPlaceOutcome::Written { .. } = store.storage().update_in_place(
                KEY,
                t,
                None,
                None,
                &mut |value| {
                    if let RecordValue::OrMap { records, .. } = value {
                        records.push(entry("b"));
                    }
                    true
                },
                &cost,
            ) else {
                panic!("the second write must mutate the resident record");
            };

            store.evict_lru(u32::MAX, false);

            assert!(
                store.exists_in_memory(KEY),
                "an unflushed write must never be evicted"
            );
        }

        // AC-13: removing a key that was never made resident clears the
        // in-memory Merkle leaf the boot seed gave it.
        #[tokio::test]
        async fn remove_of_a_non_resident_key_clears_its_merkle_leaf() {
            let dir = tempfile::tempdir().expect("tempdir");
            let ds = redb(&dir);
            let durable = or_value(&["a", "b"], &[]);
            ds.add(MAP, KEY, &durable, 0, 0).await.expect("seed");
            let other = or_value(&["z"], &[]);
            let leaf = |key: &str, value: &RecordValue| {
                merkle_leaf_hash(key, value).expect("OrMap yields a leaf").1
            };

            let manager = Arc::new(MerkleSyncManager::new(3));
            manager.update_ormap(MAP, 0, KEY, leaf(KEY, &durable));
            manager.update_ormap(MAP, 0, "other", leaf("other", &other));
            let reference = MerkleSyncManager::new(3);
            reference.update_ormap(MAP, 0, "other", leaf("other", &other));

            let merkle = Arc::new(MerkleMutationObserver::new(
                Arc::clone(&manager),
                MAP.to_string(),
                0,
            ));
            let store = store_over(
                Arc::clone(&ds),
                Box::new(HashMapStorage::new()),
                vec![merkle as Arc<dyn MutationObserver>],
            );
            assert!(!store.exists_in_memory(KEY), "precondition: never resident");

            store
                .remove(KEY, CallerProvenance::CrdtMerge)
                .await
                .expect("remove");

            assert_eq!(
                manager.aggregate_ormap_root_hash(MAP),
                reference.aggregate_ormap_root_hash(MAP),
                "the removed key's leaf must be gone from the in-memory Merkle tree"
            );
        }
    }
}
