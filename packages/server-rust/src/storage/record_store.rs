//! Per-map-per-partition record store trait.
//!
//! Defines [`RecordStore`], the primary interface that operation handlers
//! interact with. Orchestrates Layer 1 ([`StorageEngine`](super::StorageEngine))
//! and Layer 3 ([`MapDataStore`](super::MapDataStore)), adding metadata tracking,
//! expiry, eviction, and mutation observation.
//!
//! Also defines supporting types: [`CallerProvenance`], [`ExpiryPolicy`],
//! [`ExpiryReason`], and [`MutateOutcome`].

use async_trait::async_trait;

use super::engine::{FetchResult, IterationCursor, StorageEngine};
use super::key_writer::KeyWriteToken;
use super::map_data_store::MapDataStore;
use super::record::{Record, RecordValue};
use super::wal::OrDelta;

/// Origin of a write operation.
///
/// Determines how the `RecordStore` processes the write (e.g., whether to
/// trigger write-through, update access statistics, or notify observers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerProvenance {
    /// Write originated from a client request.
    Client,
    /// Write is a backup replication from the primary partition owner.
    Backup,
    /// Write is a replication event from another cluster node.
    Replication,
    /// Write is a load from the backing `MapDataStore`.
    Load,
    /// Write is the result of a CRDT merge operation.
    CrdtMerge,
}

/// Expiry configuration for a record.
///
/// Controls time-to-live and maximum idle time for automatic expiration.
/// Use [`ExpiryPolicy::NONE`] for records that should never expire.
#[derive(Debug, Clone)]
pub struct ExpiryPolicy {
    /// Time-to-live in milliseconds from creation. 0 = no TTL.
    pub ttl_millis: u64,
    /// Maximum idle time in milliseconds since last access. 0 = no max idle.
    pub max_idle_millis: u64,
}

impl ExpiryPolicy {
    /// No expiration policy. Both TTL and max-idle are disabled.
    pub const NONE: Self = Self {
        ttl_millis: 0,
        max_idle_millis: 0,
    };
}

/// Reason a record expired.
///
/// Returned by [`RecordStore::has_expired`] to indicate whether and why
/// a record has expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryReason {
    /// The record has not expired.
    NotExpired,
    /// The record expired due to time-to-live.
    Ttl,
    /// The record expired due to exceeding the maximum idle time.
    MaxIdle,
}

/// What a [`RecordStore::update_in_place`] mutate closure reports back.
///
/// `changed` carries the persist obligation that the closure's old `bool`
/// return carried: `true` exactly when the closure altered the value and a
/// write-through is owed, `false` for a no-op.
///
/// `witness` optionally carries the OR delta the closure just applied to the
/// value it was handed. It exists so a store that durably records deltas
/// receives the one the mutation point already built, instead of re-deriving it
/// from a before/after comparison — which would need the pre-image and so
/// reintroduce the whole-slot clone the in-place seam exists to avoid. It is
/// `Some` only when something beneath the store actually consumes a witness
/// ([`RecordStore::or_witness_wanted`]) and the mutation took effect; every
/// caller with no per-op delta (LWW writes, bulk and SYNC ingestion, cross-node
/// merge, rehydration) leaves it `None`. A layer that cannot forward it drops
/// it, which is always safe — the full value is written either way.
///
/// Defined in the storage layer, not alongside the CRDT apply that populates
/// it, so this trait never names a `service::domain` type and the storage →
/// service dependency direction stays one-way.
#[derive(Debug)]
pub struct MutateOutcome {
    /// Whether the closure altered the value, so a write-through is owed.
    pub changed: bool,
    /// The OR delta that was applied, when a consumer demanded one.
    pub witness: Option<OrDelta>,
}

/// What a refused mutator call says to its caller when the writer token it
/// was handed was acquired for another `(map, key)` (TG-KEY-002).
///
/// This is the outermost context of the returned error, and so the only part
/// of it that an operation error prints into a client frame. It names no map
/// and no key on purpose: the two pairs are a server-side identity. They stay
/// on the typed error beneath this context, where the store's caller can
/// still find them.
pub(crate) const KEY_WRITER_MISMATCH_CONTEXT: &str =
    "record write refused: the key writer token was acquired for another key";

/// Per-map-per-partition record store.
///
/// Primary interface that operation handlers interact with.
/// Orchestrates Layer 1 ([`StorageEngine`]) and Layer 3 ([`MapDataStore`]),
/// adding metadata tracking, expiry, eviction, and mutation observation.
///
/// **never-evict-dirty invariant:** Any record whose
/// [`RecordMetadata::is_dirty`](super::record::RecordMetadata::is_dirty) returns
/// `true` MUST NOT be evicted by [`RecordStore::evict_lru`]. Evicting a dirty
/// record discards an acknowledged write that has not yet been flushed to the
/// backing [`MapDataStore`], violating the durability contract. All
/// implementations that handle LRU eviction are required to enforce this
/// invariant; the orchestrator surfaces violations via [`RecordStore::dirty_count`]
/// backpressure logging.
///
/// Used as `Arc<dyn RecordStore>`.
#[async_trait]
pub trait RecordStore: Send + Sync {
    /// Name of the map this record store manages.
    fn name(&self) -> &str;

    /// Partition ID this record store belongs to.
    fn partition_id(&self) -> u32;

    // --- Core CRUD ---

    /// Get a record, loading from `MapDataStore` if not in memory.
    ///
    /// Updates access statistics if `touch` is true.
    async fn get(&self, key: &str, touch: bool) -> anyhow::Result<Option<Record>>;

    /// Check if a key exists in memory (does NOT load from `MapDataStore`).
    fn exists_in_memory(&self, key: &str) -> bool;

    /// Put a value, returning the old value if it existed.
    ///
    /// Handles write-through to `MapDataStore` based on provenance.
    async fn put(
        &self,
        writer: &KeyWriteToken<'_>,
        key: &str,
        value: RecordValue,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Mutate a resident record's value in place, firing the same observer
    /// notifications and durable write-through as [`put`](RecordStore::put)
    /// WITHOUT a full get→build→put round trip.
    ///
    /// `mutate` runs synchronously under the engine's per-key write lock and
    /// returns a [`MutateOutcome`] whose `changed` is `true` if it made a change
    /// that must be persisted, or `false` for a no-op (e.g. a prune whose target
    /// tag was already gone). `changed` MUST be `true` whenever the closure
    /// altered the value, and the closure is invoked **at most once** per call —
    /// callers may rely on single invocation. Its `witness` is the OR delta just
    /// applied, present only when [`or_witness_wanted`](RecordStore::or_witness_wanted)
    /// answered `true`; an implementation that has nowhere to forward a witness
    /// drops it and still writes the full value.
    ///
    /// If the key is absent: when `init` is `Some`, a fresh record is created
    /// from it, `mutate` is applied, and (on a `changed` outcome) `on_put`
    /// fires; when `init` is `None`, the call is a no-op. Returns `true` when a
    /// record was created or updated (a write-through was owed), `false`
    /// otherwise.
    ///
    /// This exists for the OR-Map write path, whose per-op read-modify-write
    /// otherwise cloned the whole ~130 KB resident snapshot on every op. The OR
    /// observers ignore the pre-image, so no old value is materialized.
    ///
    /// The default implementation falls back to a get→mutate→put round trip so
    /// non-optimized stores remain correct; [`DefaultRecordStore`] overrides it
    /// with the true in-place seam.
    async fn update_in_place(
        &self,
        writer: &KeyWriteToken<'_>,
        key: &str,
        init: Option<RecordValue>,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
        mutate: &mut (dyn for<'a> FnMut(&'a mut RecordValue) -> MutateOutcome + Send),
    ) -> anyhow::Result<bool> {
        // Refused before the read, and so before `mutate` can run on a copy
        // and leave its side effects on whatever the caller's closure
        // captured. Checked here and not left to `put`, so the refusal does
        // not depend on what an implementor's `put` does with the token.
        writer.check(self.name(), key).map_err(|mismatch| {
            anyhow::Error::new(mismatch).context(KEY_WRITER_MISMATCH_CONTEXT)
        })?;
        let existing = self.get(key, false).await?;
        let mut value = match existing {
            Some(record) => record.value,
            None => match init {
                Some(v) => v,
                None => return Ok(false),
            },
        };
        let outcome = mutate(&mut value);
        // The witness is dropped here by construction: this fallback persists
        // through `put`, which has no witness-carrying seam beneath it, so a
        // store reached this way cannot observe a delta even in principle.
        //
        // Reaching here WITH a witness means a store answered the demand signal
        // affirmatively while leaving this fallback in place — the caller then
        // pays to build a delta that nothing can receive, and its consumer sees
        // nothing while believing it is being fed. The two must be overridden
        // together, so the half-override is caught loudly in tests rather than
        // becoming a silent divergence.
        debug_assert!(
            outcome.witness.is_none(),
            "a store whose fallback update_in_place is in use must not demand a \
             witness: this path cannot deliver one"
        );
        if !outcome.changed {
            return Ok(false);
        }
        self.put(writer, key, value, expiry, provenance).await?;
        Ok(true)
    }

    /// Whether anything beneath this store consumes an [`OrDelta`] witness.
    ///
    /// Read once per op, **before** the mutate closure is built, so a caller
    /// that would otherwise materialize a delta into
    /// [`MutateOutcome::witness`] can skip constructing it altogether when
    /// nothing will take it. Defaulted to `false`, so a store with no witness
    /// consumer beneath it inherits the no-cost path without overriding
    /// anything.
    fn or_witness_wanted(&self) -> bool {
        false
    }

    /// Remove a record, returning the old value.
    async fn remove(
        &self,
        writer: &KeyWriteToken<'_>,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Put a record received from backup replication.
    async fn put_backup(
        &self,
        writer: &KeyWriteToken<'_>,
        key: &str,
        record: Record,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()>;

    /// Remove a record on backup.
    async fn remove_backup(
        &self,
        writer: &KeyWriteToken<'_>,
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
    fn fetch_entries(&self, cursor: &IterationCursor, size: usize)
        -> FetchResult<(String, Record)>;

    /// Iterate all records with an object-safe consumer.
    ///
    /// Calls `consumer` for each non-expired entry. Uses `&mut dyn FnMut`
    /// instead of generic `F: FnMut` for `Box<dyn RecordStore>` compatibility.
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

    /// Evict up to `target_count` least-recently-used non-dirty records.
    ///
    /// MUST skip records where `metadata.is_dirty()` is true — evicting a
    /// dirty record discards an acked write that has not yet flushed to the
    /// backend, violating durability. This is the never-evict-dirty invariant.
    ///
    /// Returns the number of records actually evicted (may be less than
    /// `target_count` if fewer non-dirty candidates exist).
    ///
    /// Implementations MUST use saturating cast (`u32::try_from(...).unwrap_or(u32::MAX)`)
    /// when converting the internal `usize` eviction count to the `u32` return type.
    /// Exceeding `u32::MAX` evictions in a single call is unreachable in practice
    /// but MUST NOT panic.
    fn evict_lru(&self, target_count: u32, is_backup: bool) -> u32;

    /// Number of records currently dirty (in-memory mutation not yet
    /// flushed to the backing `MapDataStore`). Surfaced for orchestrator
    /// backpressure logging.
    fn dirty_count(&self) -> u64;

    // --- Lifecycle ---

    /// Initialize the record store (create backing storage, register observers).
    fn init(&mut self);

    /// Clear all data (used by `IMap.clear()`).
    fn clear(&self, is_backup: bool) -> u32;

    /// Reset to initial state (used during migration).
    fn reset(&self);

    /// Destroy the record store and release all resources.
    fn destroy(&self);

    // --- MapDataStore integration ---

    /// Flush pending writes to the backing `MapDataStore`.
    ///
    /// Returns the sequence number of the last QUEUED (assigned) operation, or 0
    /// if empty — NOT the last flushed one. The implementation delegates to
    /// [`MapDataStore::soft_flush`], which notifies the background flush loop and
    /// returns the current assigned-sequence counter; the actual flush completes
    /// asynchronously. A caller that needs a real byte-durability signal must use
    /// [`MapDataStore::flushed_watermark`], not this return value.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Access the underlying `StorageEngine` (Layer 1).
    fn storage(&self) -> &dyn StorageEngine;

    /// Access the underlying `MapDataStore` (Layer 3).
    fn map_data_store(&self) -> &dyn MapDataStore;
}

#[cfg(test)]
mod tests {
    /// Every method of [`RecordStore`](super::RecordStore), `fn` and
    /// `async fn`, is on exactly one of the four lists below, and exactly the
    /// methods of the first list take the writer token. A method added to the
    /// trait fails this test until someone decides which list it belongs to —
    /// which is the moment to ask whether it writes a record value and so
    /// needs the token (TG-KEY-002).
    ///
    /// What this does NOT check: it checks classification, not behaviour. A
    /// method put on the wrong list passes. And it reads this file's source
    /// text, so it follows the trait's formatting: a method is a line of the
    /// trait body indented by four spaces that starts with `fn` or
    /// `async fn`.
    #[test]
    fn every_method_of_the_record_store_trait_is_classified() {
        const SOURCE: &str = include_str!("record_store.rs");

        // Writes a record value: takes the token.
        const TAKES_THE_TOKEN: [&str; 5] = [
            "put",
            "update_in_place",
            "remove",
            "put_backup",
            "remove_backup",
        ];
        // Reads only: changes no resident, engine or durable state.
        const READS_ONLY: [&str; 14] = [
            "name",
            "partition_id",
            "exists_in_memory",
            "or_witness_wanted",
            "fetch_keys",
            "fetch_entries",
            "for_each_boxed",
            "size",
            "is_empty",
            "owned_entry_cost",
            "has_expired",
            "is_expirable",
            "should_evict",
            "dirty_count",
        ];
        // Changes resident or engine state, or flushes, without writing a new
        // record value: `get` load-inserts a resident copy and stamps the
        // access, `get_all` is a loop over `get`, `soft_flush` flushes pending
        // writes, the rest evict, clear or tear down.
        const CHANGES_STATE_WITHOUT_A_NEW_VALUE: [&str; 11] = [
            "evict",
            "evict_all",
            "evict_expired",
            "evict_lru",
            "clear",
            "reset",
            "destroy",
            "init",
            "get",
            "get_all",
            "soft_flush",
        ];
        // Hands out the raw engine or data store, which take no token.
        const HANDS_OUT_THE_RAW_LAYERS: [&str; 2] = ["storage", "map_data_store"];

        let from = SOURCE
            .find("pub trait RecordStore")
            .expect("the trait is in this file");
        let body = &SOURCE[from..];
        let body = &body[..body.find("\n}\n").expect("the trait's closing brace")];

        // (name, signature text up to the `;` or the `{` that ends it)
        let mut methods: Vec<(&str, String)> = Vec::new();
        let mut lines = body.lines();
        while let Some(line) = lines.next() {
            let Some(rest) = line
                .strip_prefix("    async fn ")
                .or_else(|| line.strip_prefix("    fn "))
            else {
                continue;
            };
            let name = &rest[..rest
                .find(['(', '<'])
                .expect("a method name ends at its parameter list")];
            let mut signature = line.to_string();
            while !(signature.ends_with(';') || signature.ends_with('{')) {
                signature.push_str(lines.next().expect("a signature ends inside the trait"));
            }
            methods.push((name, signature));
        }

        let lists = [
            &TAKES_THE_TOKEN[..],
            &READS_ONLY[..],
            &CHANGES_STATE_WITHOUT_A_NEW_VALUE[..],
            &HANDS_OUT_THE_RAW_LAYERS[..],
        ];
        let listed = |name: &str| lists.iter().filter(|list| list.contains(&name)).count();
        let not_on_exactly_one_list: Vec<&str> = methods
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| listed(name) != 1)
            .collect();
        let listed_but_not_in_the_trait: Vec<&str> = lists
            .iter()
            .flat_map(|list| list.iter().copied())
            .filter(|name| !methods.iter().any(|(method, _)| method == name))
            .collect();
        let token_disagrees_with_the_list: Vec<&str> = methods
            .iter()
            .filter(|(name, signature)| {
                signature.contains("KeyWriteToken") != TAKES_THE_TOKEN.contains(name)
            })
            .map(|(name, _)| *name)
            .collect();

        assert_eq!(
            (
                methods.len(),
                not_on_exactly_one_list,
                listed_but_not_in_the_trait,
                token_disagrees_with_the_list,
            ),
            (32, Vec::new(), Vec::new(), Vec::new()),
            "(methods declared by the trait, methods not on exactly one list, listed names \
             the trait does not declare, methods whose signature names the token against \
             their list)"
        );
    }
}
