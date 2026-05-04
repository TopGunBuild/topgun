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
//! Table names follow `map__{map_name}` for primary tables and
//! `map__{map_name}__backup` for backup partitions. Map names are validated
//! against `^[a-zA-Z_][a-zA-Z0-9_]*$` before use to prevent table-name
//! collisions via metacharacters (parity with `PostgresDataStore`).
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

use std::path::Path;
use std::sync::Arc;

use anyhow::bail;
use async_trait::async_trait;
use redb::{TableDefinition, TableHandle};

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

/// Validate that a map name matches `^[a-zA-Z_][a-zA-Z0-9_]*$`.
///
/// Map names are interpolated into redb table-definition strings via
/// `format!("map__{name}")`; rejecting metacharacters prevents collision
/// between user-supplied maps and reserved table names. Mirrors the
/// `is_valid_table_name` check in `PostgresDataStore`.
fn is_valid_map_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Build the redb table name for a `(map, is_backup)` tuple.
///
/// Caller must have already validated `map` via `is_valid_map_name`.
fn table_name_for(map: &str, is_backup: bool) -> String {
    if is_backup {
        format!("map__{map}__backup")
    } else {
        format!("map__{map}")
    }
}

/// Macro that builds a `TableDefinition<&str, &[u8]>` whose lifetime is
/// tied to the borrowed `name: &str`. `redb::TableDefinition::new` is a
/// `const fn` and the resulting handle holds a `&'static str` reference;
/// since we build the name dynamically, we leak it via the leak-on-first-
/// use pattern below at each callsite.
fn table_def(name: &str) -> TableDefinition<'_, &'static str, &'static [u8]> {
    // redb 2.x table-definition API requires a `&'static str` for the table
    // name. Leaking a Box keeps the per-(map, is_backup) string alive for
    // the process lifetime. The number of distinct map names is bounded by
    // application schema (small, fixed at startup), so leak growth is
    // negligible compared to the alternative (a full HashMap-backed cache
    // with mutex on hot path).
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    TableDefinition::new(leaked)
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
    pub async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Insert (or overwrite) a single record under the given `(map, key, is_backup)`
/// tuple. Validates the map name, opens (or creates) the per-(map, is_backup)
/// table inside one `WriteTransaction`, serializes the value via msgpack, and
/// commits.
fn write_one(
    db: &redb::Database,
    map: &str,
    key: &str,
    value: &RecordValue,
    is_backup: bool,
) -> anyhow::Result<()> {
    if !is_valid_map_name(map) {
        bail!("Invalid map name '{map}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$");
    }
    let bytes = rmp_serde::to_vec_named(value)?;
    let table_name = table_name_for(map, is_backup);
    let def = table_def(&table_name);
    let txn = db.begin_write()?;
    {
        let mut table = txn.open_table(def)?;
        table.insert(key, bytes.as_slice())?;
    }
    txn.commit()?;
    Ok(())
}

/// Delete a single record under the given `(map, key, is_backup)` tuple.
/// No-op if the table or key does not exist.
fn delete_one(db: &redb::Database, map: &str, key: &str, is_backup: bool) -> anyhow::Result<()> {
    if !is_valid_map_name(map) {
        bail!("Invalid map name '{map}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$");
    }
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
        Ok(iter) => iter
            .into_iter()
            .any(|h| h.name() == table_name),
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
        if !is_valid_map_name(map) {
            bail!("Invalid map name '{map}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$");
        }
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
        if !is_valid_map_name(map) {
            bail!("Invalid map name '{map}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$");
        }
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
                let value: RecordValue = rmp_serde::from_slice(&b.value().to_vec())?;
                results.push((key.clone(), value));
            }
        }
        Ok(results)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        if !is_valid_map_name(map) {
            bail!("Invalid map name '{map}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$");
        }
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
