//! Embedded `redb`-backed [`MapDataStore`] implementation.
//!
//! Provides zero-config durable persistence in a single file (`./topgun.redb`
//! by default). Selected as the default backend for `pnpm start:server` so a
//! developer can boot the server with no Postgres container, no Docker, and
//! no environment variables -- writes survive process restart.
//!
//! # Storage layout
//!
//! One `redb::TableDefinition<&str, &[u8]>` per `(map_name, is_backup)` tuple.
//! The table name carries the map name verbatim behind a prefix that depends on
//! the name's class (TG-NAME-001):
//!
//! | Map name | Primary table | Backup table |
//! |---|---|---|
//! | identifier class (`^[a-zA-Z_][a-zA-Z0-9_]*$`) | `map__{name}` | `map__{name}__backup` |
//! | any other storable name | `mapr__{name}` | `maprb__{name}` |
//!
//! The character class is NOT a gate: it only picks the prefix. Identifier-class
//! names keep the tables they have always had, so nothing on disk is renamed;
//! every other name gets a prefix no identifier-class table can carry (byte 3 is
//! `r`, not `_`), so the two classes never share a table.
//!
//! What the store does refuse is decided by the shared rule
//! [`check_map_name`]: an empty name, a name containing U+0000, and a name ending
//! in the **reserved** `__backup` suffix. The suffix is reserved because the
//! identifier-class backup table is spelled with it: a primary map literally
//! named `foo__backup` would encode to table `map__foo__backup` — byte-identical
//! to the backup partition of map `foo` — so `list_maps` (which derives the
//! durable map set from the table catalog and skips backup tables) would
//! silently drop it, leaving its post-restart Merkle root at 0. The rule's
//! length bound is an ingress concern and is deliberately NOT applied here: the
//! store must keep reading every table that already exists.
//!
//! Values are msgpack-serialized [`RecordValue`] (via
//! [`rmp_serde::to_vec_named`]), matching the on-disk format the Postgres
//! backend uses for its `value BYTEA` column. This preserves byte-level
//! cross-backend wire compatibility for future migration tooling.
//!
//! # Durability semantics
//!
//! `redb` commits its write-ahead log on every `WriteTransaction::commit()`
//! by default -- writes are durable on commit, matching the Postgres
//! write-through guarantee. Do not call `Database::set_durability(None)` on
//! the inner handle; that would silently downgrade the embedded backend
//! below the documented HN-demo "data survives restart" promise.
//!
//! # `is_null` contract
//!
//! [`RedbDataStore`] does NOT override [`MapDataStore::is_null`]. The trait's
//! default returns `false`, which is correct for any real backing store. Only
//! [`NullDataStore`](super::NullDataStore) returns `true`; that is the canary
//! signal used elsewhere in the server to detect the ephemeral test path.
//! Copy-pasting the `NullDataStore` implementation of `is_null` into this
//! file would silently break that detection.
//!
//! # Concurrency
//!
//! `redb` is single-writer / multi-reader. Concurrent readers do not block
//! each other, but writes serialize on the database-level write lock. This is
//! acceptable for the embedded Drop-in tier (single-user dev, HN demo); for
//! multi-client production workloads, prefer the Postgres backend.
//!
//! # Version pin
//!
//! Pinned to `redb = "2"` (current stable major as of 2026-05-04). `redb` has
//! had on-disk format breaking changes across major versions; before bumping
//! to `3.x`, either ship a migration step or document a "rebuild from
//! Postgres" path. See follow-up TODO-335 for the migration plan.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::storage::map_data_store::{
    check_map_name, merkle_leaf_hash, LeafSink, MapDataStore, MerkleLeaf, ScanBatch, ScanCursor,
};
use crate::storage::record::RecordValue;
use anyhow::bail;
use async_trait::async_trait;
use redb::{ReadableTable, TableDefinition, TableHandle};

/// Number of leaves accumulated before invoking the sink once.
///
/// Bounds the producer's peak memory during enumeration: only this many
/// `(key, u32)` pairs are held at a time, never the whole map. Values are
/// never materialized — each row is decoded only far enough to derive its
/// leaf hash, then dropped.
const LEAF_BATCH_SIZE: usize = 1024;

/// Default per-batch resident byte budget for value-streamed scans when the
/// caller passes `max_batch_cost == 0`.
///
/// Conservative fraction of the `TOPGUN_MAX_RAM_MB` ceiling (default 1 GiB)
/// so a single batch never dominates the record cache. A scan resumes via its
/// `ScanCursor` until the map is exhausted, so this only caps one batch, not
/// the total scanned volume.
const DEFAULT_SCAN_BATCH_COST: u64 = 8 * 1024 * 1024;

/// Whether a map name is in the identifier class `^[a-zA-Z_][a-zA-Z0-9_]*$`.
///
/// This validates nothing. It is the class predicate of TG-NAME-001, used only
/// by [`table_name_for`] to pick the table prefix: names in the class keep the
/// `map__` tables they have always had, every other storable name goes under
/// `mapr__` / `maprb__`. Which names the store refuses is decided separately,
/// by [`refuse_unstorable`].
fn is_identifier_class(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Refuse a map name the store cannot hold, per the shared rule
/// [`check_map_name`]: empty, ending in the reserved `__backup` suffix, or
/// containing U+0000.
///
/// The rule's length bound is not a store refusal (`refused_by_store()` is
/// false for it): a boot path lists and scans every table that already exists,
/// and a name that is merely long must stay readable.
fn refuse_unstorable(map: &str) -> anyhow::Result<()> {
    match check_map_name(map) {
        Err(violation) if violation.refused_by_store() => {
            bail!("Invalid map name {map:?}: {violation}")
        }
        _ => Ok(()),
    }
}

/// Build the redb table name for a `(map, is_backup)` tuple (TG-NAME-001).
///
/// Identifier-class names map to `map__{map}` / `map__{map}__backup`, exactly
/// the tables earlier versions created; every other name maps to
/// `mapr__{map}` / `maprb__{map}`. The mapping is injective over storable
/// names: byte 3 separates the two classes (`_` vs `r`), byte 4 separates the
/// raw-name primary from its backup (`_` vs `b`), and an identifier-class
/// primary never ends in `__backup` because no storable name does.
///
/// Caller must have already passed `map` through [`refuse_unstorable`]; the
/// injectivity argument does not hold for a name ending in `__backup`.
fn table_name_for(map: &str, is_backup: bool) -> String {
    match (is_identifier_class(map), is_backup) {
        (true, false) => format!("map__{map}"),
        (true, true) => format!("map__{map}__backup"),
        (false, false) => format!("mapr__{map}"),
        (false, true) => format!("maprb__{map}"),
    }
}

/// Module-level intern cache: maps a table name string to a leaked `&'static str`.
///
/// redb 2.x requires `&'static str` for `TableDefinition::new`. We satisfy this by
/// leaking one `Box<str>` per *distinct* table name encountered at runtime and caching
/// the resulting `&'static str`. Repeat calls for the same name return the cached
/// pointer without a new allocation, bounding leak growth to the number of distinct
/// `(map, is_backup)` combinations ever seen — a small, schema-fixed set in practice.
static LEAKED_TABLE_NAMES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

/// Return a `TableDefinition` whose name pointer is stable for the process lifetime.
///
/// On the first call for a given `name`, a single `Box<str>` is leaked and the
/// resulting `&'static str` is cached in `LEAKED_TABLE_NAMES`. Subsequent calls for
/// the same `name` return the cached pointer — no allocation, no additional leak.
/// Total leaked memory is bounded by the number of distinct table names (one per
/// `(map, is_backup)` combination), which is small and fixed by the application schema.
fn table_def(name: &str) -> TableDefinition<'_, &'static str, &'static [u8]> {
    let cache = LEAKED_TABLE_NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("LEAKED_TABLE_NAMES mutex poisoned");
    let static_name = guard
        .entry(name.to_string())
        .or_insert_with(|| Box::leak(name.to_string().into_boxed_str()));
    TableDefinition::new(static_name)
}

/// Embedded `redb`-backed `MapDataStore`.
///
/// Wraps a single [`redb::Database`] handle behind an [`Arc`] so the same
/// store can be cloned cheaply into the partition dispatcher and per-task
/// futures.
///
/// # Construction
///
/// ```ignore
/// let store = RedbDataStore::new("./topgun.redb")?;
/// store.initialize().await?;
/// // hand to RecordStoreFactory::new(...)
/// ```
pub struct RedbDataStore {
    db: Arc<redb::Database>,
}

impl RedbDataStore {
    /// Open (or create) a `redb` database at the given path.
    ///
    /// The database file is created on first call; subsequent calls reuse
    /// the existing file. The parent directory must already exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened (permissions, corrupt
    /// header, or another process holds the `redb` lockfile).
    pub fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let db = redb::Database::create(path.as_ref())?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Initialize the data store.
    ///
    /// `redb` opens its tables lazily inside the first `WriteTransaction`,
    /// so this is currently a no-op. The async signature mirrors
    /// `PostgresDataStore::initialize` to keep the bootstrap call site
    /// uniform across backends.
    ///
    /// # Errors
    ///
    /// Reserved for future schema migrations; never errors today.
    // Async signature is intentional: `select_datastore()` in `topgun_server.rs` calls
    // `store.initialize().await` uniformly across backends. `PostgresDataStore::initialize`
    // is genuinely async (runs DDL migrations); keeping the same signature here lets the
    // bootstrap code branch on the backend without conditional `.await`.
    #[allow(clippy::unused_async)]
    pub async fn initialize(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Drop this handle's reference to the inner database.
    ///
    /// `redb` flushes and releases the lockfile when the last `Arc`
    /// reference is dropped. Calling `close` explicitly is mostly a
    /// readability aid for shutdown paths; the same effect happens
    /// automatically when the store goes out of scope.
    ///
    /// # Errors
    ///
    /// Reserved for future explicit-flush behavior; never errors today.
    // Async signature is intentional: matches the `PostgresDataStore::close` signature for
    // interface parity, so callers can call `.close().await` uniformly without branching.
    #[allow(clippy::unused_async)]
    pub async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Load one bounded batch of `(key, value)` pairs, optionally resuming
    /// strictly after `start_after_key`, returning a cursor for the next batch.
    ///
    /// Accumulates rows until the summed serialized byte cost reaches
    /// `max_batch_cost` (or `DEFAULT_SCAN_BATCH_COST` when `0`), then stops so a
    /// single batch never exceeds the resident-memory budget. The returned
    /// `next_cursor` is `None` once the table is fully drained.
    fn scan_from(
        &self,
        map: &str,
        is_backup: bool,
        start_after_key: Option<&str>,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        refuse_unstorable(map)?;
        let budget = if max_batch_cost == 0 {
            DEFAULT_SCAN_BATCH_COST
        } else {
            max_batch_cost
        };
        let table_name = table_name_for(map, is_backup);
        let def = table_def(&table_name);
        let txn = self.db.begin_read()?;
        let table = match txn.open_table(def) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(ScanBatch::default()),
            Err(e) => return Err(e.into()),
        };

        // redb ranges are inclusive of the lower bound, so resume by skipping a
        // leading row whose key equals the cursor marker.
        let range_iter = match start_after_key {
            Some(k) => table.range(k..)?,
            None => table.iter()?,
        };

        let mut records: Vec<(String, RecordValue)> = Vec::new();
        let mut accumulated: u64 = 0;
        let mut more_rows_pending = false;

        for entry in range_iter {
            let (key_guard, val_guard) = entry?;
            let key = key_guard.value().to_string();
            if let Some(after) = start_after_key {
                if key == after {
                    // Inclusive lower bound: skip the cursor row itself.
                    continue;
                }
            }
            let bytes = val_guard.value();
            let row_cost = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            // Strict check-before-push: account for the prospective row's byte
            // cost BEFORE admitting it, so a batch never includes the row that
            // would cross the budget (peak resident batch cost stays <= budget).
            // The empty-batch carve-out admits a single oversized record whole
            // because one record cannot be split — for that degenerate case the
            // bound is <= budget + one max-record.
            if !records.is_empty() && accumulated.saturating_add(row_cost) > budget {
                more_rows_pending = true;
                break;
            }
            accumulated += row_cost;
            let value: RecordValue = rmp_serde::from_slice(bytes)?;
            records.push((key, value));
        }

        let next_cursor = if more_rows_pending {
            records
                .last()
                .map(|(k, _)| ScanCursor(k.clone().into_bytes()))
        } else {
            None
        };
        Ok(ScanBatch {
            records,
            next_cursor,
        })
    }
}

/// Insert (or overwrite) a single record under the given `(map, key, is_backup)`
/// tuple: serializes the value via msgpack ([`encode_record`]) and stores the
/// bytes through [`write_encoded`], so a value and its pre-encoded bytes always
/// land as the same row.
///
/// This is a deliberately CRDT-agnostic blind insert: it does NOT merge by
/// timestamp. Last-write-wins ordering is upheld before the store is ever
/// reached — by the CRDT service layer on the live path, and by the
/// `RecordValue::Lww` read-compare gate in `WalRecovery::replay_entry` on the
/// recovery path (TG-WAL-006). Keeping merge semantics out of the storage
/// primitive avoids a layering inversion and a per-write read on the flush hot
/// path.
fn write_one(
    db: &redb::Database,
    map: &str,
    key: &str,
    value: &RecordValue,
    is_backup: bool,
) -> anyhow::Result<()> {
    let bytes = encode_record(value)?;
    write_encoded(db, map, key, &bytes, is_backup)
}

/// The one msgpack encoding of a stored record. Callers that hand
/// [`RedbDataStore`] pre-encoded bytes (`add_encoded`) must produce exactly
/// this encoding.
fn encode_record(value: &RecordValue) -> anyhow::Result<Vec<u8>> {
    Ok(rmp_serde::to_vec_named(value)?)
}

/// Store already-encoded record bytes under `(map, key, is_backup)`. Refuses a
/// name the store cannot hold, opens (or creates) the per-(map, `is_backup`) table inside one
/// `WriteTransaction`, inserts the bytes as given, and commits. The bytes are
/// not decoded or checked here.
fn write_encoded(
    db: &redb::Database,
    map: &str,
    key: &str,
    bytes: &[u8],
    is_backup: bool,
) -> anyhow::Result<()> {
    refuse_unstorable(map)?;
    let table_name = table_name_for(map, is_backup);
    let def = table_def(&table_name);
    let txn = db.begin_write()?;
    {
        let mut table = txn.open_table(def)?;
        table.insert(key, bytes)?;
    }
    txn.commit()?;
    Ok(())
}

/// Delete a single record under the given `(map, key, is_backup)` tuple.
/// No-op if the table or key does not exist.
fn delete_one(db: &redb::Database, map: &str, key: &str, is_backup: bool) -> anyhow::Result<()> {
    refuse_unstorable(map)?;
    let table_name = table_name_for(map, is_backup);
    let def = table_def(&table_name);

    // Cheap pre-check: is this table even known to the database? If not,
    // the key cannot exist; skip the WriteTransaction round trip entirely.
    {
        let read_txn = db.begin_read()?;
        if !table_exists(&read_txn, &table_name) {
            return Ok(());
        }
    }

    let txn = db.begin_write()?;
    {
        let mut table = txn.open_table(def)?;
        table.remove(key)?;
    }
    txn.commit()?;
    Ok(())
}

/// Check whether a table with the given name has ever been created in the
/// database. Used by the delete and remove-all paths to short-circuit when
/// the table has never been written to.
fn table_exists(read_txn: &redb::ReadTransaction, table_name: &str) -> bool {
    match read_txn.list_tables() {
        Ok(iter) => iter.into_iter().any(|h| h.name() == table_name),
        Err(_) => false,
    }
}

#[async_trait]
impl MapDataStore for RedbDataStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        write_one(&self.db, map, key, value, false)
    }

    fn accepts_encoded(&self) -> bool {
        true
    }

    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        write_encoded(&self.db, map, key, bytes, false)
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        write_one(&self.db, map, key, value, true)
    }

    async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        delete_one(&self.db, map, key, false)
    }

    async fn remove_backup(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        delete_one(&self.db, map, key, true)
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        refuse_unstorable(map)?;
        let table_name = table_name_for(map, false);
        let def = table_def(&table_name);
        let txn = self.db.begin_read()?;
        let table = match txn.open_table(def) {
            Ok(t) => t,
            // If the table has never been written to, `open_table` returns
            // a `TableDoesNotExist` error -- treat as "no value".
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let bytes = match table.get(key)? {
            Some(b) => b.value().to_vec(),
            None => return Ok(None),
        };
        let value: RecordValue = rmp_serde::from_slice(&bytes)?;
        Ok(Some(value))
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        refuse_unstorable(map)?;
        let table_name = table_name_for(map, false);
        let def = table_def(&table_name);
        let txn = self.db.begin_read()?;
        let table = match txn.open_table(def) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(b) = table.get(key.as_str())? {
                let value: RecordValue = rmp_serde::from_slice(b.value())?;
                results.push((key.clone(), value));
            }
        }
        Ok(results)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        refuse_unstorable(map)?;
        if keys.is_empty() {
            return Ok(());
        }
        let table_name = table_name_for(map, false);
        let def = table_def(&table_name);

        // Skip the WriteTransaction if the table has never been created.
        {
            let read_txn = self.db.begin_read()?;
            if !table_exists(&read_txn, &table_name) {
                return Ok(());
            }
        }

        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(def)?;
            for key in keys {
                table.remove(key.as_str())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        // Derive the durable map set from the redb table catalog rather than
        // any in-memory cache, so a map that was persisted before a restart but
        // is not yet resident is still discovered. Only primary tables feed the
        // SYNC_INIT root; backup tables are deliberately skipped.
        //
        // This is the inverse of `table_name_for` on primaries (TG-NAME-001).
        // The three prefixes are mutually exclusive — none is a prefix of
        // another — so a catalog name takes at most one branch whatever the
        // order. Only the identifier-class branch has suffix logic: a raw name
        // is returned as the bytes after its prefix, untouched, so a raw name
        // that merely contains `__backup` or looks like a table prefix decodes
        // to itself.
        let read_txn = self.db.begin_read()?;
        let mut names: Vec<String> = Vec::new();
        for handle in read_txn.list_tables()? {
            let table = handle.name();
            if let Some(raw) = table.strip_prefix("mapr__") {
                names.push(raw.to_string());
            } else if table.starts_with("maprb__") {
                // Backup partition of a raw-name map.
            } else if let Some(rest) = table.strip_prefix("map__") {
                if !rest.ends_with("__backup") {
                    names.push(rest.to_string());
                }
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        refuse_unstorable(map)?;
        let table_name = table_name_for(map, is_backup);
        let def = table_def(&table_name);
        let txn = self.db.begin_read()?;
        let table = match txn.open_table(def) {
            Ok(t) => t,
            // Never-written table contributes no leaves.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        // Range-scan the whole table in key order, decoding each row only far
        // enough to derive its leaf hash, then flushing fixed-size batches to
        // the sink so peak memory stays bounded regardless of map size.
        let mut batch: Vec<MerkleLeaf> = Vec::with_capacity(LEAF_BATCH_SIZE);
        for entry in table.iter()? {
            let (key_guard, val_guard) = entry?;
            let value: RecordValue = rmp_serde::from_slice(val_guard.value())?;
            let key = key_guard.value().to_string();
            if let Some((kind, leaf_hash)) = merkle_leaf_hash(&key, &value) {
                batch.push(MerkleLeaf {
                    key,
                    kind,
                    leaf_hash,
                });
            }
            if batch.len() >= LEAF_BATCH_SIZE {
                sink.consume(std::mem::take(&mut batch)).await?;
                batch = Vec::with_capacity(LEAF_BATCH_SIZE);
            }
        }
        if !batch.is_empty() {
            sink.consume(batch).await?;
        }
        Ok(())
    }

    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // First batch starts from the table's beginning: an empty resume key.
        self.scan_from(map, is_backup, None, max_batch_cost)
    }

    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // The cursor carries the last key returned by the previous batch (UTF-8
        // bytes). Resume strictly after it.
        let last_key = String::from_utf8(cursor.0)
            .map_err(|e| anyhow::anyhow!("scan cursor is not valid UTF-8: {e}"))?;
        self.scan_from(map, is_backup, Some(&last_key), max_batch_cost)
    }

    fn is_loadable(&self, _key: &str) -> bool {
        // Write-through: every successful add/remove has already committed
        // to redb before the call returned, so every key is loadable.
        true
    }

    fn pending_operation_count(&self) -> u64 {
        // Write-through: nothing is queued.
        0
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        // Nothing to flush -- writes are already durable.
        Ok(0)
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        // Nothing to flush -- writes are already durable on commit.
        Ok(())
    }

    async fn flush_key(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        is_backup: bool,
    ) -> anyhow::Result<()> {
        // For write-through, "flush this key" is equivalent to "write it
        // through right now". Mirrors the Postgres flush_key semantics.
        write_one(&self.db, map, key, value, is_backup)
    }

    fn reset(&self) {
        // Nothing to reset -- redb's data lives on disk and has no
        // in-memory queue. (Matches PostgresDataStore::reset.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::record::OrMapEntry;
    use tempfile::tempdir;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    /// Build a `RedbDataStore` over a fresh tempdir-backed file.
    fn fresh_store() -> (RedbDataStore, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test.redb");
        let store = RedbDataStore::new(&path).expect("redb open");
        (store, dir)
    }

    fn dummy_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    #[tokio::test]
    async fn add_then_load_returns_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v1"));
    }

    #[tokio::test]
    async fn add_overwrites_previous_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        store
            .add("users", "alice", &dummy_value("v2"), 0, 1001)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v2"));
    }

    #[tokio::test]
    async fn add_backup_is_isolated_from_primary() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("primary"), 0, 1000)
            .await
            .unwrap();
        store
            .add_backup("users", "alice", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load("users", "alice")
            .await
            .unwrap()
            .expect("primary");
        assert!(
            matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "primary")
        );
    }

    #[tokio::test]
    async fn remove_deletes_value() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("v1"), 0, 1000)
            .await
            .unwrap();
        store.remove("users", "alice", 1001).await.unwrap();
        assert!(store.load("users", "alice").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_missing_key_is_ok() {
        let (store, _dir) = fresh_store();
        // Both nonexistent table and nonexistent key paths must be no-op.
        store.remove("never_written", "k", 1000).await.unwrap();
        store
            .add("users", "bob", &dummy_value("v"), 0, 1000)
            .await
            .unwrap();
        store
            .remove("users", "alice_not_present", 1000)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn remove_backup_only_affects_backup() {
        let (store, _dir) = fresh_store();
        store
            .add("users", "alice", &dummy_value("primary"), 0, 1000)
            .await
            .unwrap();
        store
            .add_backup("users", "alice", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        store.remove_backup("users", "alice", 1001).await.unwrap();
        // Primary survives.
        assert!(store.load("users", "alice").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let (store, _dir) = fresh_store();
        // Table never created path:
        assert!(store.load("never", "k").await.unwrap().is_none());
        // Table exists but key missing path:
        store
            .add("users", "alice", &dummy_value("v"), 0, 1000)
            .await
            .unwrap();
        assert!(store.load("users", "ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_all_returns_subset() {
        let (store, _dir) = fresh_store();
        store
            .add("m", "a", &dummy_value("a"), 0, 1000)
            .await
            .unwrap();
        store
            .add("m", "b", &dummy_value("b"), 0, 1000)
            .await
            .unwrap();
        let got = store
            .load_all(
                "m",
                &["a".to_string(), "b".to_string(), "missing".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(got.len(), 2, "missing key silently absent");
    }

    #[tokio::test]
    async fn load_all_on_missing_table_returns_empty() {
        let (store, _dir) = fresh_store();
        let got = store.load_all("never", &["a".to_string()]).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn remove_all_atomic_per_map() {
        let (store, _dir) = fresh_store();
        for k in ["a", "b", "c"] {
            store.add("m", k, &dummy_value(k), 0, 1000).await.unwrap();
        }
        store
            .remove_all("m", &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert!(store.load("m", "a").await.unwrap().is_none());
        assert!(store.load("m", "b").await.unwrap().is_none());
        assert!(store.load("m", "c").await.unwrap().is_some());
    }

    #[test]
    fn is_loadable_returns_true() {
        let (store, _dir) = fresh_store();
        assert!(store.is_loadable("any-key"));
    }

    #[test]
    fn pending_operation_count_returns_zero() {
        let (store, _dir) = fresh_store();
        assert_eq!(store.pending_operation_count(), 0);
    }

    #[tokio::test]
    async fn soft_flush_returns_zero() {
        let (store, _dir) = fresh_store();
        assert_eq!(store.soft_flush().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn hard_flush_returns_ok() {
        let (store, _dir) = fresh_store();
        store.hard_flush().await.unwrap();
    }

    #[tokio::test]
    async fn flush_key_writes_through() {
        let (store, _dir) = fresh_store();
        store
            .flush_key("m", "k", &dummy_value("v"), false)
            .await
            .unwrap();
        let got = store.load("m", "k").await.unwrap().expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v"));
    }

    #[test]
    fn reset_is_noop() {
        let (store, _dir) = fresh_store();
        store.reset();
    }

    #[test]
    fn is_null_returns_false_via_trait_default() {
        let (store, _dir) = fresh_store();
        // Per is_null contract: RedbDataStore must NOT override the default.
        // Only NullDataStore returns true.
        assert!(!store.is_null());
    }

    #[tokio::test]
    async fn durability_across_reopen() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("durable.redb");
        {
            let store = RedbDataStore::new(&path).unwrap();
            store
                .add("m", "k", &dummy_value("persisted"), 0, 1000)
                .await
                .unwrap();
            store.close().await.unwrap();
            // Drop releases the redb lockfile.
        }
        // Reopen on the same path -- value must survive.
        let store = RedbDataStore::new(&path).unwrap();
        let got = store
            .load("m", "k")
            .await
            .unwrap()
            .expect("survives reopen");
        assert!(
            matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "persisted")
        );
    }

    /// A name full of metacharacters is data, not syntax: redb table names are
    /// opaque strings, so the name is stored verbatim under the raw-name prefix
    /// and comes back from the catalog byte-identical.
    #[tokio::test]
    async fn metacharacter_map_name_is_stored_verbatim() {
        let (store, _dir) = fresh_store();
        let name = "foo; DROP TABLE x";
        store
            .add(name, "k", &dummy_value("v"), 0, 1000)
            .await
            .expect("a metacharacter map name is storable");
        assert_eq!(
            store.load(name, "k").await.unwrap(),
            Some(dummy_value("v")),
            "the value reads back under the same name"
        );
        assert_eq!(store.list_maps().await.unwrap(), vec![name.to_string()]);
        assert_eq!(catalog(&store), vec![format!("mapr__{name}")]);
    }

    #[tokio::test]
    async fn empty_map_name_rejected() {
        let (store, _dir) = fresh_store();
        let err = store.add("", "k", &dummy_value("v"), 0, 1000).await;
        assert!(err.is_err());
    }

    /// Every table name in the store's catalog, sorted.
    fn catalog(store: &RedbDataStore) -> Vec<String> {
        let txn = store.db.begin_read().expect("read txn");
        let mut tables: Vec<String> = txn
            .list_tables()
            .expect("list_tables")
            .map(|handle| handle.name().to_string())
            .collect();
        tables.sort();
        tables
    }

    /// The raw bytes stored for `(map, key)` in the primary table.
    fn raw_row(store: &RedbDataStore, map: &str, key: &str) -> Option<Vec<u8>> {
        let table_name = table_name_for(map, false);
        let txn = store.db.begin_read().expect("read txn");
        let table = match txn.open_table(table_def(&table_name)) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return None,
            Err(e) => panic!("open table: {e}"),
        };
        table
            .get(key)
            .expect("get row")
            .map(|guard| guard.value().to_vec())
    }

    fn or_value_with_tombstones() -> RecordValue {
        let ts = |millis| Timestamp {
            millis,
            counter: 3,
            node_id: "node-a".to_string(),
        };
        RecordValue::OrMap {
            records: vec![
                OrMapEntry {
                    value: Value::String("first".to_string()),
                    tag: "t-1".to_string(),
                    timestamp: ts(10),
                },
                OrMapEntry {
                    value: Value::Int(42),
                    tag: "t-2".to_string(),
                    timestamp: ts(11),
                },
            ],
            tombstones: vec!["t-0".to_string(), "t-9".to_string()],
        }
    }

    #[test]
    fn redb_accepts_encoded_records() {
        let (store, _dir) = fresh_store();
        assert!(store.accepts_encoded());
    }

    /// `add_encoded` of a value's msgpack encoding stores exactly the row `add`
    /// stores for that value — the write-behind flush relies on this to hand
    /// redb bytes encoded under a cell lock instead of a copied record. Checked
    /// on a fresh row and on an overwrite, for an OR value with tombstones and
    /// for an LWW value.
    #[tokio::test]
    async fn add_encoded_stores_byte_identical_rows_to_add() {
        let (store, _dir) = fresh_store();
        for (i, value) in [or_value_with_tombstones(), dummy_value("lww")]
            .into_iter()
            .enumerate()
        {
            let by_value = format!("by_value_{i}");
            let by_bytes = format!("by_bytes_{i}");
            store
                .add("golden", &by_value, &value, 0, 1000)
                .await
                .unwrap();
            let bytes = rmp_serde::to_vec_named(&value).unwrap();
            store
                .add_encoded("golden", &by_bytes, &bytes, 0, 1000)
                .await
                .unwrap();

            let row_value = raw_row(&store, "golden", &by_value).expect("add row");
            let row_bytes = raw_row(&store, "golden", &by_bytes).expect("add_encoded row");
            assert_eq!(row_bytes, row_value, "fresh row, value #{i}");
            assert_eq!(row_bytes, bytes, "the row is the bytes as handed over");
            assert_eq!(
                store.load("golden", &by_bytes).await.unwrap(),
                Some(value.clone()),
                "an encoded row loads back as the value"
            );

            // Overwrite both rows with a different value through the other path.
            let next = dummy_value(&format!("next-{i}"));
            store
                .add_encoded(
                    "golden",
                    &by_value,
                    &rmp_serde::to_vec_named(&next).unwrap(),
                    0,
                    1001,
                )
                .await
                .unwrap();
            store
                .add("golden", &by_bytes, &next, 0, 1001)
                .await
                .unwrap();
            assert_eq!(
                raw_row(&store, "golden", &by_value),
                raw_row(&store, "golden", &by_bytes),
                "overwrite, value #{i}"
            );
        }
    }

    /// `add_encoded` refuses a name the store cannot hold exactly as `add` does:
    /// the same error, and no table created. And a name outside the identifier
    /// class, which both paths take, lands in the same table through either.
    #[tokio::test]
    async fn add_encoded_refuses_a_store_refused_map_name_exactly_as_add() {
        let (store, _dir) = fresh_store();
        let value = dummy_value("v");
        let bytes = rmp_serde::to_vec_named(&value).unwrap();
        for bad in ["", "users__backup", "has-dash__backup", "a\0b"] {
            let via_add = store
                .add(bad, "k", &value, 0, 1000)
                .await
                .expect_err("add refuses");
            let via_encoded = store
                .add_encoded(bad, "k", &bytes, 0, 1000)
                .await
                .expect_err("add_encoded refuses");
            assert_eq!(via_encoded.to_string(), via_add.to_string(), "map {bad:?}");
            assert!(via_encoded.to_string().contains("Invalid map name"));
        }
        assert!(
            catalog(&store).is_empty(),
            "a refused write creates no table"
        );

        for name in ["foo; DROP TABLE x", "9starts_with_digit", "has-dash"] {
            store
                .add(name, "by_value", &value, 0, 1000)
                .await
                .expect("add stores a name outside the identifier class");
            store
                .add_encoded(name, "by_bytes", &bytes, 0, 1000)
                .await
                .expect("add_encoded stores it too");
            assert_eq!(
                raw_row(&store, name, "by_bytes"),
                raw_row(&store, name, "by_value"),
                "map {name:?}"
            );
            assert_eq!(raw_row(&store, name, "by_bytes"), Some(bytes.clone()));
        }
    }

    #[tokio::test]
    async fn concurrent_writers_serialize_cleanly() {
        // redb is single-writer; concurrent writes serialize on the write
        // lock. Spawn two write tasks against the same store and verify both
        // succeed and both values are observable. This exercises the
        // serialization path without asserting any particular ordering.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("concurrent.redb");
        let store = Arc::new(RedbDataStore::new(&path).unwrap());

        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);
        let h1 = tokio::spawn(async move {
            for i in 0..10 {
                s1.add("m", &format!("a{i}"), &dummy_value("A"), 0, 1000)
                    .await
                    .unwrap();
            }
        });
        let h2 = tokio::spawn(async move {
            for i in 0..10 {
                s2.add("m", &format!("b{i}"), &dummy_value("B"), 0, 1000)
                    .await
                    .unwrap();
            }
        });
        h1.await.unwrap();
        h2.await.unwrap();

        for i in 0..10 {
            assert!(store.load("m", &format!("a{i}")).await.unwrap().is_some());
            assert!(store.load("m", &format!("b{i}")).await.unwrap().is_some());
        }
    }

    #[test]
    fn is_identifier_class_holds_for_identifiers() {
        assert!(is_identifier_class("users"));
        assert!(is_identifier_class("_internal"));
        assert!(is_identifier_class("Users123"));
        // Text that looks like a table prefix or the backup suffix is still an
        // identifier: the predicate reads characters, not meaning.
        assert!(is_identifier_class("r__x"));
        assert!(is_identifier_class("mapr__x"));
        assert!(is_identifier_class("__backup_data"));
        assert!(is_identifier_class("backup"));
    }

    #[test]
    fn is_identifier_class_excludes_everything_else() {
        assert!(!is_identifier_class(""));
        assert!(!is_identifier_class("123leading_digit"));
        assert!(!is_identifier_class("foo bar"));
        assert!(!is_identifier_class("foo;DROP"));
        assert!(!is_identifier_class("foo--backup"));
        assert!(!is_identifier_class("user-profiles"));
        assert!(!is_identifier_class("users/profiles"));
        assert!(!is_identifier_class("notes:abc"));
        assert!(!is_identifier_class("é"));
    }

    /// The class predicate decides the prefix and nothing else: being outside
    /// the class is not a refusal, and the refusals do not depend on the class.
    #[test]
    fn the_class_predicate_is_not_the_store_gate() {
        // Outside the class, storable.
        for name in ["123leading_digit", "foo bar", "foo;DROP", "foo--backup"] {
            assert!(!is_identifier_class(name), "{name:?}");
            assert!(refuse_unstorable(name).is_ok(), "{name:?}");
        }
        // A primary map named `foo__backup` would encode to table
        // `map__foo__backup`, indistinguishable from the backup partition of
        // map `foo`, so `list_maps` would silently drop it and serve Merkle
        // root 0 for durable data. The suffix is refused in either class.
        for name in ["foo__backup", "__backup", "a-b__backup"] {
            let err = refuse_unstorable(name).expect_err("reserved suffix");
            assert!(err.to_string().contains("__backup"), "{err}");
        }
        // But `__backup` elsewhere in the name is fine — only the suffix is reserved.
        assert!(refuse_unstorable("__backup_data").is_ok());
        assert!(refuse_unstorable("backup").is_ok());
        // The length bound belongs to the ingress, not to the store.
        assert!(refuse_unstorable(&"a".repeat(600)).is_ok());
        assert!(refuse_unstorable(&format!("-{}", "a".repeat(599))).is_ok());
    }

    #[test]
    fn table_def_cache_returns_pointer_identical_static_str() {
        // Two calls for the same name must return the exact same &'static str pointer,
        // confirming the intern cache hits on the second call and does not leak a second
        // allocation for the same distinct name.
        let def1 = table_def("map__cache_test");
        let def2 = table_def("map__cache_test");
        // TableDefinition::name() returns the &'static str used at construction.
        assert!(
            std::ptr::eq(def1.name(), def2.name()),
            "second call must reuse cached pointer, not allocate a new &'static str"
        );
        // A different name must produce a different pointer (distinct intern entries).
        let def3 = table_def("map__cache_test__backup");
        assert!(
            !std::ptr::eq(def1.name(), def3.name()),
            "distinct names must have distinct pointers"
        );
    }

    /// Test sink that collects every leaf the enumeration produces.
    struct CollectingSink {
        leaves: Vec<MerkleLeaf>,
    }

    #[async_trait]
    impl LeafSink for CollectingSink {
        async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            self.leaves.extend(batch);
            Ok(())
        }
    }

    #[tokio::test]
    async fn enumerate_leaves_hash_matches_write_path_formula() {
        let (store, _dir) = fresh_store();
        // A non-zero HLC so millis/counter/node_id all participate in the hash.
        let ts = Timestamp {
            millis: 123_456,
            counter: 7,
            node_id: "node-A".to_string(),
        };
        let value = RecordValue::Lww {
            value: Value::String("payload".to_string()),
            timestamp: ts.clone(),
        };
        store.add("users", "alice", &value, 0, 1000).await.unwrap();

        let mut sink = CollectingSink { leaves: Vec::new() };
        store
            .enumerate_leaves("users", false, &mut sink)
            .await
            .unwrap();

        assert_eq!(sink.leaves.len(), 1, "exactly one durable leaf");
        let expected = topgun_core::hash::fnv1a_hash(&format!(
            "{}:{}:{}:{}",
            "alice", ts.millis, ts.counter, ts.node_id
        ));
        assert_eq!(
            sink.leaves[0].leaf_hash, expected,
            "redb LWW leaf hash must equal the write-path fnv1a(key:millis:counter:node_id)"
        );
        assert_eq!(sink.leaves[0].key, "alice");
    }

    #[tokio::test]
    async fn enumerate_leaves_empty_for_never_written_map() {
        let (store, _dir) = fresh_store();
        let mut sink = CollectingSink { leaves: Vec::new() };
        store
            .enumerate_leaves("never", false, &mut sink)
            .await
            .unwrap();
        assert!(sink.leaves.is_empty());
    }

    #[tokio::test]
    async fn scan_values_resumes_via_cursor_and_covers_all_keys() {
        let (store, _dir) = fresh_store();
        for i in 0..50 {
            store
                .add("m", &format!("k{i:03}"), &dummy_value("v"), 0, 1000)
                .await
                .unwrap();
        }

        // Tiny budget forces multiple batches so the cursor-resume path is exercised.
        let mut seen: Vec<String> = Vec::new();
        let mut batch = store.scan_values("m", false, 1).await.unwrap();
        loop {
            for (k, _) in &batch.records {
                seen.push(k.clone());
            }
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            50,
            "every key surfaces exactly once across batches"
        );
        assert_eq!(seen.first().map(String::as_str), Some("k000"));
        assert_eq!(seen.last().map(String::as_str), Some("k049"));
    }

    #[tokio::test]
    async fn scan_values_empty_map_is_exhausted() {
        let (store, _dir) = fresh_store();
        let batch = store.scan_values("never", false, 0).await.unwrap();
        assert!(batch.records.is_empty());
        assert!(
            batch.next_cursor.is_none(),
            "empty scan is immediately exhausted"
        );
    }

    /// Serialized byte cost of a single record, matching what `scan_from`
    /// accounts (`rmp_serde::to_vec_named` is the redb on-disk encoding).
    fn record_byte_cost(value: &RecordValue) -> u64 {
        let bytes = rmp_serde::to_vec_named(value).expect("encode");
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    }

    /// AC5: strict check-before-push — a batch never includes the row that
    /// would cross the budget, so peak resident batch byte cost <= `max_batch_cost`.
    #[tokio::test]
    async fn scan_from_strict_byte_budget_never_overshoots() {
        let (store, _dir) = fresh_store();
        for i in 0..20 {
            store
                .add("m", &format!("k{i:03}"), &dummy_value("payload"), 0, 1000)
                .await
                .unwrap();
        }
        let one_cost = record_byte_cost(&dummy_value("payload"));
        // Budget for ~3 records; strict check-before-push must stop BEFORE the
        // 4th, so no batch exceeds the budget.
        let budget = one_cost * 3;

        let mut batch = store.scan_values("m", false, budget).await.unwrap();
        loop {
            let batch_cost: u64 = batch.records.iter().map(|(_, v)| record_byte_cost(v)).sum();
            assert!(
                batch_cost <= budget,
                "batch byte cost {batch_cost} must not exceed budget {budget}"
            );
            assert!(!batch.records.is_empty(), "each batch makes progress");
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, budget)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
    }

    /// AC5 degenerate carve-out: a single record larger than the whole budget is
    /// admitted whole (one record cannot be split), so the bound for that case is
    /// <= `max_batch_cost` + one max-record. WHY: progress must be guaranteed; an
    /// empty batch with a remaining cursor would livelock the scan.
    #[tokio::test]
    async fn scan_from_admits_single_oversized_record_whole() {
        let (store, _dir) = fresh_store();
        let big = "x".repeat(4096);
        store
            .add("m", "only", &dummy_value(&big), 0, 1000)
            .await
            .unwrap();
        let budget = 1; // far smaller than the record's byte cost
        let batch = store.scan_values("m", false, budget).await.unwrap();
        assert_eq!(batch.records.len(), 1, "oversized record admitted whole");
        assert_eq!(batch.records[0].0, "only");
        assert!(
            batch.next_cursor.is_none(),
            "single record drains the map in one batch"
        );
    }

    /// AC5 negative control: with the pre-fix check-AFTER-push accounting a batch
    /// overshoots by one row. This reconstructs the pre-fix loop body locally and
    /// asserts it overshoots, proving the production strict check is load-bearing.
    #[tokio::test]
    async fn scan_byte_budget_negative_control_pre_fix_overshoots() {
        let costs = [10u64, 10, 10, 10];
        let budget = 25u64;

        // Pre-fix behavior: accumulate, push, THEN check >= budget.
        let mut pre_fix_batch: Vec<u64> = Vec::new();
        let mut acc = 0u64;
        for &c in &costs {
            acc += c;
            pre_fix_batch.push(c);
            if acc >= budget {
                break;
            }
        }
        let pre_fix_cost: u64 = pre_fix_batch.iter().sum();
        assert!(
            pre_fix_cost > budget,
            "pre-fix accounting overshoots the budget ({pre_fix_cost} > {budget})"
        );

        // Post-fix behavior (current production): check-before-push.
        let mut post_fix_batch: Vec<u64> = Vec::new();
        let mut acc = 0u64;
        for &c in &costs {
            if !post_fix_batch.is_empty() && acc.saturating_add(c) > budget {
                break;
            }
            acc += c;
            post_fix_batch.push(c);
        }
        let post_fix_cost: u64 = post_fix_batch.iter().sum();
        assert!(
            post_fix_cost <= budget,
            "post-fix accounting respects the budget ({post_fix_cost} <= {budget})"
        );
    }

    /// A write the store refuses on every attempt must not disappear quietly:
    /// once the write-behind layer gives up, the operator has to be able to see
    /// WHICH write was dropped, and within a time short enough to act on.
    ///
    /// The store underneath is the real embedded one and the retry settings are
    /// the defaults a server boots with, so the wait below is the wall-clock an
    /// operator would actually sit through. A map name carrying the reserved
    /// backup suffix is used because the store refuses it unconditionally.
    ///
    /// Current-thread runtime on purpose: the line is emitted by the spawned
    /// flush task, and a thread-local subscriber only reaches that task when it
    /// runs on this test's own thread.
    #[tokio::test]
    async fn a_store_rejected_write_is_reported_within_a_bounded_time() {
        use crate::storage::datastores::{WriteBehindConfig, WriteBehindDataStore};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        const REFUSED_MAP: &str = "probe__backup";
        const KEY: &str = "k1";
        const BOUND: Duration = Duration::from_secs(15);

        #[derive(Clone)]
        struct CapturedLog(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for CapturedLog {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = CapturedLog(Arc::clone(&captured));
        // Scoped, never a global install: this test binary is shared and runs in
        // parallel, so a global subscriber would leak into every other test.
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let (redb, _dir) = fresh_store();
        let redb = Arc::new(redb);
        let value = dummy_value("v");
        #[allow(clippy::cast_possible_truncation)]
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as i64;

        // The premise: the store itself refuses this name outright.
        let direct = redb
            .add(REFUSED_MAP, KEY, &value, 0, now)
            .await
            .expect_err("the embedded store refuses the reserved suffix");
        assert!(direct.to_string().contains("Invalid map name"), "{direct}");

        let inner: Arc<dyn MapDataStore> = redb;
        let store = WriteBehindDataStore::new(inner, WriteBehindConfig::default());

        // The write is accepted: the buffering layer does not look at the name.
        store
            .add(REFUSED_MAP, KEY, &value, 0, now)
            .await
            .expect("write-behind accepts the write");
        let accepted_at = Instant::now();

        let snapshot = |buf: &Arc<Mutex<Vec<u8>>>| {
            String::from_utf8_lossy(
                &buf.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
            .into_owned()
        };
        let names_the_write = |line: &str| {
            (line.contains(" WARN ") || line.contains("ERROR"))
                && line.contains(REFUSED_MAP)
                && line.contains(KEY)
        };

        // Measured from acceptance, which is earlier than the first refused store
        // call, so passing here is the stricter claim.
        let mut reported_after = None;
        while accepted_at.elapsed() < BOUND {
            if snapshot(&captured).lines().any(names_the_write) {
                reported_after = Some(accepted_at.elapsed());
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let log = snapshot(&captured);
        eprintln!(
            "--- captured log ({} lines, reported_after={reported_after:?}, waited={:?}) ---\n{log}--- end of captured log ---",
            log.lines().count(),
            accepted_at.elapsed(),
        );
        assert!(
            reported_after.is_some(),
            "no WARN-or-higher event naming map {REFUSED_MAP:?} and key {KEY:?} within {BOUND:?} of the write being accepted"
        );
    }

    /// Names the identifier-class rule turns away, chosen to cover each way a
    /// raw name can go wrong as a table name: separators, whitespace, a leading
    /// digit, text that looks like one of the store's own prefixes or its backup
    /// suffix, non-ASCII, and the two longest names an ingress bound of 512
    /// bytes admits (one ASCII, one multi-byte).
    fn probe_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "user-profiles",
            "users/profiles",
            "users.profiles",
            "$sys/stats",
            "foo bar",
            " ",
            "notes:abc",
            "123leading",
            "foo;DROP",
            "foo--backup",
            "map__a-b",
            "mapr__a-b",
            "maprb__a-b",
            "a-b__backup_data",
            "заметки-ユーザー-📝",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let long_ascii = format!("-{}", "a".repeat(511));
        let long_multibyte = "é".repeat(256);
        assert_eq!(long_ascii.len(), 512);
        assert_eq!(long_multibyte.len(), 512);
        names.push(long_ascii);
        names.push(long_multibyte);
        names
    }

    /// Short, log-friendly rendering of a probe name: the 512-byte ones would
    /// otherwise drown the per-name report.
    fn shown(name: &str) -> String {
        if name.len() > 40 {
            let head: String = name.chars().take(8).collect();
            format!("{head:?}… ({} bytes)", name.len())
        } else {
            format!("{name:?}")
        }
    }

    /// One table, through redb alone: a row goes in, the transaction commits,
    /// the row reads back, and the catalog lists the table under the very bytes
    /// it was created with, exactly once.
    fn raw_table_round_trips(db: &redb::Database, table: &str) -> Result<(), String> {
        const KEY: &str = "k";
        let payload = table.as_bytes();

        let txn = db.begin_write().map_err(|e| format!("begin_write: {e}"))?;
        {
            let mut t = txn
                .open_table(table_def(table))
                .map_err(|e| format!("open_table for write: {e}"))?;
            t.insert(KEY, payload).map_err(|e| format!("insert: {e}"))?;
        }
        txn.commit().map_err(|e| format!("commit: {e}"))?;

        let txn = db.begin_read().map_err(|e| format!("begin_read: {e}"))?;
        let t = txn
            .open_table(table_def(table))
            .map_err(|e| format!("open_table for read: {e}"))?;
        let got = t
            .get(KEY)
            .map_err(|e| format!("get: {e}"))?
            .map(|guard| guard.value().to_vec());
        if got.as_deref() != Some(payload) {
            return Err("the row did not read back as written".to_string());
        }
        let listed = txn
            .list_tables()
            .map_err(|e| format!("list_tables: {e}"))?
            .filter(|handle| handle.name().as_bytes() == table.as_bytes())
            .count();
        if listed != 1 {
            return Err(format!(
                "the catalog lists the table {listed} times, expected exactly once"
            ));
        }
        Ok(())
    }

    /// The premise the raw-name table mapping stands on, checked against redb
    /// itself and not through the store: a table may be named with a fixed
    /// prefix followed by ANY of the probe names, verbatim, and the catalog
    /// hands that name back unchanged. If redb normalised, truncated or refused
    /// any of them, deriving the map set from the catalog would be unsound and
    /// the names would have to be encoded instead.
    ///
    /// Every name is tried and reported before the verdict, so a failure names
    /// the whole failing set and not just its first member.
    #[test]
    fn redb_accepts_the_probe_names_under_both_new_prefixes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("premise.redb");
        let names = probe_names();
        let mut failing: Vec<String> = Vec::new();

        {
            let db = redb::Database::create(&path).expect("redb create");
            for name in &names {
                let mut verdict = Ok(());
                for prefix in ["mapr__", "maprb__"] {
                    let table = format!("{prefix}{name}");
                    if let Err(why) = raw_table_round_trips(&db, &table) {
                        verdict = Err(format!("under {prefix:?}: {why}"));
                        break;
                    }
                }
                match verdict {
                    Ok(()) => eprintln!("probe {}: accepted, round-tripped", shown(name)),
                    Err(why) => {
                        eprintln!("probe {}: FAILED — {why}", shown(name));
                        failing.push(shown(name));
                    }
                }
            }
        }

        // A catalog that only agrees with itself inside one process would not
        // help a restarted server, so the names are read again from the file.
        let db = redb::Database::create(&path).expect("redb reopen");
        let txn = db.begin_read().expect("begin_read");
        let mut on_disk: Vec<String> = txn
            .list_tables()
            .expect("list_tables")
            .map(|handle| handle.name().to_string())
            .collect();
        on_disk.sort();
        let mut expected: Vec<String> = names
            .iter()
            .flat_map(|n| [format!("mapr__{n}"), format!("maprb__{n}")])
            .collect();
        expected.sort();
        eprintln!(
            "after reopen: {} tables in the catalog, {} expected, failing set = {failing:?}",
            on_disk.len(),
            expected.len()
        );

        assert!(
            failing.is_empty(),
            "redb did not hold these probe names verbatim: {failing:?}"
        );
        assert_eq!(
            on_disk, expected,
            "the reopened catalog must list every probe table byte-identically"
        );
    }

    /// Everything a map needs from the store, for one name: a value written is
    /// the value read, the map shows up in the durable map set under its own
    /// bytes exactly once, a backup write does not disturb that set, and a
    /// removed key is gone.
    async fn store_round_trips(store: &RedbDataStore, name: &str, tag: &str) -> Result<(), String> {
        const KEY: &str = "k1";
        let value = dummy_value(tag);

        store
            .add(name, KEY, &value, 0, 1000)
            .await
            .map_err(|e| format!("add: {e}"))?;
        let loaded = store
            .load(name, KEY)
            .await
            .map_err(|e| format!("load: {e}"))?;
        if loaded.as_ref() != Some(&value) {
            return Err(format!("load returned {loaded:?}, not the value written"));
        }

        let listed = store
            .list_maps()
            .await
            .map_err(|e| format!("list_maps: {e}"))?;
        let occurrences = listed
            .iter()
            .filter(|m| m.as_bytes() == name.as_bytes())
            .count();
        if occurrences != 1 {
            return Err(format!(
                "list_maps names the map {occurrences} times, expected exactly once"
            ));
        }

        store
            .add_backup(name, KEY, &dummy_value("backup"), 0, 1000)
            .await
            .map_err(|e| format!("add_backup: {e}"))?;
        let listed_after_backup = store
            .list_maps()
            .await
            .map_err(|e| format!("list_maps after add_backup: {e}"))?;
        if listed_after_backup != listed {
            return Err("a backup write changed the durable map set".to_string());
        }

        store
            .remove(name, KEY, 1001)
            .await
            .map_err(|e| format!("remove: {e}"))?;
        match store.load(name, KEY).await {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err("the key is still readable after remove".to_string()),
            Err(e) => Err(format!("load after remove: {e}")),
        }
    }

    /// TG-NAME-001, the round trip: a map whose name is outside the identifier
    /// class is as durable and as visible as one inside it. The server acks a
    /// write before the store sees the name, so a name the store turns away is
    /// an acknowledged write that is gone after a restart.
    ///
    /// All probe names share one store, so a name that aliased another's table
    /// would show up as a wrong value or a wrong map set, not pass by luck.
    #[tokio::test]
    async fn map_names_outside_the_identifier_class_round_trip() {
        let (store, _dir) = fresh_store();
        let names = probe_names();
        let mut failing: Vec<String> = Vec::new();

        for (i, name) in names.iter().enumerate() {
            match store_round_trips(&store, name, &format!("v{i}")).await {
                Ok(()) => eprintln!("probe {}: round-tripped through the store", shown(name)),
                Err(why) => {
                    eprintln!("probe {}: FAILED — {why}", shown(name));
                    failing.push(format!("{}: {why}", shown(name)));
                }
            }
        }

        assert!(
            failing.is_empty(),
            "{} of {} names did not round-trip through the store:\n{}",
            failing.len(),
            names.len(),
            failing.join("\n")
        );

        let mut expected = names;
        expected.sort();
        assert_eq!(
            store.list_maps().await.expect("list_maps"),
            expected,
            "the durable map set must be exactly the probe names, each once"
        );
    }

    /// A WAL written before the store accepted a name must not stay stuck once
    /// it does. An earlier server acked writes to a map the store then refused;
    /// their frames were never applied and never dropped — recovery keeps a
    /// frame it could not replay and tries again on every boot. So the first
    /// boot whose store takes the name has to replay them, in the order they
    /// were written, and let the watermark move past them.
    ///
    /// The fixture is such a log: two stores and then a remove of the first
    /// key, with no applied watermark. The remove is what makes the order
    /// observable — replayed before its store, or skipped, it would leave the
    /// first key readable (TG-WAL-011).
    #[tokio::test]
    async fn recovery_replays_frames_abandoned_for_a_previously_refused_map_name() {
        use crate::storage::wal::{
            Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery, WalStorePayload, WalWriter,
        };

        const MAP: &str = "user-profiles";
        const PARTITION: u32 = 7;

        let stamped = |tag: &str, millis: u64| RecordValue::Lww {
            value: Value::String(tag.to_string()),
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: "writer".to_string(),
            },
        };
        let store_frame = |key: &str, value: &RecordValue, sequence: u64| WalEntry {
            map: MAP.to_string(),
            key: key.to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Record(value.clone()),
                expiration_time: None,
            },
            timestamp: match value {
                RecordValue::Lww { timestamp, .. } => Some(timestamp.clone()),
                _ => None,
            },
            sequence,
        };

        let k1_value = stamped("first", 100);
        let k2_value = stamped("second", 200);
        let frames = [
            store_frame("k1", &k1_value, 1),
            store_frame("k2", &k2_value, 2),
            WalEntry {
                map: MAP.to_string(),
                key: "k1".to_string(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: 3,
            },
        ];
        let highest_sequence = 3;

        let dir = tempdir().expect("tempdir");
        let wal_dir = dir.path().join("wal");

        // The log as the earlier process left it: frames durable, nothing
        // marked applied. The writer is dropped so recovery reads the files
        // the way a new process would.
        {
            let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).expect("wal open");
            for frame in &frames {
                wal.append(PARTITION, frame).await.expect("wal append");
            }
        }

        let wal = WalWriter::new(wal_dir, WalFsyncPolicy::PerOp).expect("wal reopen");
        let before = wal.unapplied(PARTITION).await.expect("unapplied");
        assert_eq!(
            before.as_slice(),
            frames.as_slice(),
            "fixture: the reopened log holds exactly the three frames, in order, all un-applied"
        );

        let store = Arc::new(RedbDataStore::new(dir.path().join("store.redb")).expect("redb open"));
        let inner: Arc<dyn MapDataStore> = store.clone();
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(inner)
            .await
            .expect("recovery itself never refuses to boot over un-replayable frames");

        let unreplayed: Vec<(String, u64)> = wal
            .unapplied(PARTITION)
            .await
            .expect("unapplied after recovery")
            .into_iter()
            .map(|e| (e.key, e.sequence))
            .collect();
        let watermark = wal.test_applied_watermark(PARTITION);
        let k1 = store.load(MAP, "k1").await.map_err(|e| e.to_string());
        let k2 = store.load(MAP, "k2").await.map_err(|e| e.to_string());
        eprintln!(
            "after recovery: unreplayed (key, sequence) = {unreplayed:?}, applied watermark = \
             {watermark} (highest frame sequence {highest_sequence}), load k1 = {k1:?}, \
             load k2 = {k2:?}"
        );

        assert!(
            unreplayed.is_empty(),
            "recovery left {} of {} frames for map {MAP:?} unreplayed: {unreplayed:?}; \
             load k2 = {k2:?}",
            unreplayed.len(),
            frames.len()
        );
        assert_eq!(
            watermark, highest_sequence,
            "the applied watermark must reach the highest frame sequence written"
        );
        assert_eq!(
            k2,
            Ok(Some(k2_value)),
            "the stored value of the key that was never removed"
        );
        assert_eq!(
            k1,
            Ok(None),
            "the remove must replay after the store it follows"
        );
    }

    /// Identifier-class names every earlier version could store, including the
    /// server's own fixed maps and names shaped like the raw-name prefixes.
    const IDENTIFIER_NAMES: [&str; 9] = [
        "users",
        "_internal",
        "Users123",
        "r__x",
        "rb__x",
        "x__backup_data",
        "_topgun_tombstone_cursors_v2",
        "_topgun_device_credentials",
        "__topgun_policies",
    ];

    /// TG-NAME-001: an identifier-class name keeps the exact table names it has
    /// always had, every other name goes under the raw-name prefixes, the four
    /// ranges never meet, and the catalog decodes back to the primary names.
    ///
    /// All names share one store and both tables of each are written, so a
    /// name that aliased another's table, or a backup read as a primary, shows
    /// up as a wrong catalog or a wrong map set.
    #[tokio::test]
    async fn table_names_keep_the_old_class_and_prefix_the_new_class() {
        let raw_names = probe_names();

        for n in IDENTIFIER_NAMES {
            assert_eq!(table_name_for(n, false), format!("map__{n}"));
            assert_eq!(table_name_for(n, true), format!("map__{n}__backup"));
        }
        for n in &raw_names {
            assert_eq!(
                table_name_for(n, false),
                format!("mapr__{n}"),
                "{}",
                shown(n)
            );
            assert_eq!(
                table_name_for(n, true),
                format!("maprb__{n}"),
                "{}",
                shown(n)
            );
        }

        // What keeps the ranges apart: byte 3 tells the classes apart, byte 4
        // tells a raw-name primary from its backup, and an identifier-class
        // primary never carries the backup suffix.
        for n in IDENTIFIER_NAMES {
            for backup in [false, true] {
                assert_eq!(table_name_for(n, backup).as_bytes()[3], b'_', "{n}");
            }
            assert!(!table_name_for(n, false).ends_with("__backup"), "{n}");
        }
        for n in &raw_names {
            assert_eq!(table_name_for(n, false).as_bytes()[3], b'r');
            assert_eq!(table_name_for(n, true).as_bytes()[3], b'r');
            assert_eq!(table_name_for(n, false).as_bytes()[4], b'_');
            assert_eq!(table_name_for(n, true).as_bytes()[4], b'b');
        }

        let all_names: Vec<String> = IDENTIFIER_NAMES
            .iter()
            .map(ToString::to_string)
            .chain(raw_names.iter().cloned())
            .collect();

        let (store, _dir) = fresh_store();
        for (i, n) in all_names.iter().enumerate() {
            store
                .add(n, "k", &dummy_value(&format!("primary-{i}")), 0, 1000)
                .await
                .unwrap_or_else(|e| panic!("add {}: {e}", shown(n)));
            store
                .add_backup(n, "k", &dummy_value(&format!("backup-{i}")), 0, 1000)
                .await
                .unwrap_or_else(|e| panic!("add_backup {}: {e}", shown(n)));
        }

        let mut expected_tables: Vec<String> = all_names
            .iter()
            .flat_map(|n| [table_name_for(n, false), table_name_for(n, true)])
            .collect();
        expected_tables.sort();
        let distinct = expected_tables
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(
            distinct,
            all_names.len() * 2,
            "the mapping is injective over (name, is_backup)"
        );
        assert_eq!(catalog(&store), expected_tables);

        let mut expected_maps = all_names.clone();
        expected_maps.sort();
        assert_eq!(
            store.list_maps().await.expect("list_maps"),
            expected_maps,
            "the durable map set is exactly the primary names, each once"
        );

        // No table was shared: every primary still holds its own value.
        for (i, n) in all_names.iter().enumerate() {
            assert_eq!(
                store.load(n, "k").await.expect("load"),
                Some(dummy_value(&format!("primary-{i}"))),
                "{}",
                shown(n)
            );
        }
    }

    /// TG-NAME-001, primary vs backup: a name the store refuses reaches no
    /// table through any entry point, so nothing can be written where a backup
    /// partition lives, and backup tables never show up as maps. The length
    /// bound is not one of the store's refusals — except that a long name which
    /// is ALSO refused must still be refused.
    #[tokio::test]
    async fn store_refused_names_reach_no_table_and_backups_stay_unlisted() {
        let (store, _dir) = fresh_store();
        let value = dummy_value("v");

        store.add("foo", "k", &value, 0, 1000).await.unwrap();
        store
            .add_backup("foo", "k", &dummy_value("backup"), 0, 1000)
            .await
            .unwrap();
        assert_eq!(store.list_maps().await.unwrap(), vec!["foo".to_string()]);
        let tables_before = catalog(&store);
        assert_eq!(tables_before, vec!["map__foo", "map__foo__backup"]);

        for refused in ["foo__backup", "a-b__backup", "", "a\0b"] {
            assert!(
                store.add(refused, "k", &value, 0, 1000).await.is_err(),
                "add {refused:?}"
            );
            assert!(store.load(refused, "k").await.is_err(), "load {refused:?}");
            for is_backup in [false, true] {
                assert!(
                    store.scan_values(refused, is_backup, 0).await.is_err(),
                    "scan_values {refused:?} backup={is_backup}"
                );
                let mut sink = CollectingSink { leaves: Vec::new() };
                assert!(
                    store
                        .enumerate_leaves(refused, is_backup, &mut sink)
                        .await
                        .is_err(),
                    "enumerate_leaves {refused:?} backup={is_backup}"
                );
                assert!(sink.leaves.is_empty(), "no leaf for {refused:?}");
            }
            // The remaining entry points take the same check.
            assert!(store
                .add_backup(refused, "k", &value, 0, 1000)
                .await
                .is_err());
            assert!(store.remove(refused, "k", 1000).await.is_err());
            assert!(store.remove_backup(refused, "k", 1000).await.is_err());
            assert!(store.load_all(refused, &["k".to_string()]).await.is_err());
            assert!(store.remove_all(refused, &["k".to_string()]).await.is_err());
        }

        assert_eq!(
            catalog(&store),
            tables_before,
            "a refused name creates no table"
        );
        // The backup partition of `foo` was not read or overwritten as a map.
        assert_eq!(store.load("foo", "k").await.unwrap(), Some(value.clone()));
        assert_eq!(store.list_maps().await.unwrap(), vec!["foo".to_string()]);

        // Longer than the ingress bound, in either class: the store takes it.
        let long_identifier = "a".repeat(600);
        let long_raw = format!("-{}", "a".repeat(599));
        for name in [&long_identifier, &long_raw] {
            assert_eq!(name.len(), 600);
            store
                .add(name, "k", &value, 0, 1000)
                .await
                .unwrap_or_else(|e| panic!("add {}: {e}", shown(name)));
            assert_eq!(
                store.load(name, "k").await.expect("load"),
                Some(value.clone()),
                "{}",
                shown(name)
            );
        }

        // Too long AND reserved: the reserved suffix decides. Were the length
        // reported instead, the store would let the name through and it would
        // land on a backup table.
        let long_reserved = format!("{}__backup", "a".repeat(592));
        assert_eq!(long_reserved.len(), 600);
        let err = store
            .add(&long_reserved, "k", &value, 0, 1000)
            .await
            .expect_err("a long name with the reserved suffix is refused");
        assert!(err.to_string().contains("__backup"), "{err}");
        assert!(store.load(&long_reserved, "k").await.is_err());
        assert_eq!(
            catalog(&store),
            vec![
                table_name_for(&long_identifier, false),
                "map__foo".to_string(),
                "map__foo__backup".to_string(),
                table_name_for(&long_raw, false),
            ],
            "only the two storable long names gained a table"
        );
    }

    /// What the store spends, per call, turning a map name into a table name —
    /// the name check plus the mapping. Every store call pays it, so it is
    /// measured here on its own, where a change to either function shows up
    /// undiluted by I/O.
    ///
    /// Both functions run for every name, as they do in the store: none of the
    /// three names is one the store refuses. `users` is the identifier-class
    /// case; the other two take the raw-name branch, on a typical and on the
    /// longest name the ingress admits.
    ///
    /// A measurement, not a verdict: it asserts nothing about speed and is kept
    /// out of the default run.
    #[test]
    #[ignore = "measurement: run with --ignored --nocapture"]
    fn map_name_cost_table_mapping() {
        use std::hint::black_box;
        use std::time::Instant;

        const CALLS: u32 = 1_000_000;
        const REPEATS: usize = 5;

        let long_name = format!("-{}", "a".repeat(511));
        let cases = [
            ("users", "users"),
            ("todo/<uuid>", "todo/123e4567-e89b-12d3-a456-426614174000"),
            ("512-byte name", long_name.as_str()),
        ];

        for (label, name) in cases {
            let mut ns_per_call: Vec<f64> = (0..REPEATS)
                .map(|_| {
                    let started = Instant::now();
                    for _ in 0..CALLS {
                        let name = black_box(name);
                        black_box(check_map_name(name).is_ok());
                        black_box(table_name_for(name, false));
                    }
                    started.elapsed().as_secs_f64() * 1e9 / f64::from(CALLS)
                })
                .collect();
            ns_per_call.sort_by(f64::total_cmp);
            let median = ns_per_call[REPEATS / 2];
            eprintln!(
                "map_name_cost_table_mapping {label} ({} bytes): median {median:.1} ns/call \
                 over {CALLS} calls x {REPEATS} repeats; repeats = {ns_per_call:.1?}",
                name.len()
            );
        }
    }
}
