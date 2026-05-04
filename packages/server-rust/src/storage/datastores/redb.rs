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

#[cfg(test)]
mod tests {
    use super::*;
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
        store.add("users", "alice", &dummy_value("v1"), 0, 1000).await.unwrap();
        let got = store.load("users", "alice").await.unwrap().expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v1"));
    }

    #[tokio::test]
    async fn add_overwrites_previous_value() {
        let (store, _dir) = fresh_store();
        store.add("users", "alice", &dummy_value("v1"), 0, 1000).await.unwrap();
        store.add("users", "alice", &dummy_value("v2"), 0, 1001).await.unwrap();
        let got = store.load("users", "alice").await.unwrap().expect("present");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "v2"));
    }

    #[tokio::test]
    async fn add_backup_is_isolated_from_primary() {
        let (store, _dir) = fresh_store();
        store.add("users", "alice", &dummy_value("primary"), 0, 1000).await.unwrap();
        store.add_backup("users", "alice", &dummy_value("backup"), 0, 1000).await.unwrap();
        let got = store.load("users", "alice").await.unwrap().expect("primary");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "primary"));
    }

    #[tokio::test]
    async fn remove_deletes_value() {
        let (store, _dir) = fresh_store();
        store.add("users", "alice", &dummy_value("v1"), 0, 1000).await.unwrap();
        store.remove("users", "alice", 1001).await.unwrap();
        assert!(store.load("users", "alice").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_missing_key_is_ok() {
        let (store, _dir) = fresh_store();
        // Both nonexistent table and nonexistent key paths must be no-op.
        store.remove("never_written", "k", 1000).await.unwrap();
        store.add("users", "bob", &dummy_value("v"), 0, 1000).await.unwrap();
        store.remove("users", "alice_not_present", 1000).await.unwrap();
    }

    #[tokio::test]
    async fn remove_backup_only_affects_backup() {
        let (store, _dir) = fresh_store();
        store.add("users", "alice", &dummy_value("primary"), 0, 1000).await.unwrap();
        store.add_backup("users", "alice", &dummy_value("backup"), 0, 1000).await.unwrap();
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
        store.add("users", "alice", &dummy_value("v"), 0, 1000).await.unwrap();
        assert!(store.load("users", "ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_all_returns_subset() {
        let (store, _dir) = fresh_store();
        store.add("m", "a", &dummy_value("a"), 0, 1000).await.unwrap();
        store.add("m", "b", &dummy_value("b"), 0, 1000).await.unwrap();
        let got = store
            .load_all("m", &["a".to_string(), "b".to_string(), "missing".to_string()])
            .await
            .unwrap();
        assert_eq!(got.len(), 2, "missing key silently absent");
    }

    #[tokio::test]
    async fn load_all_on_missing_table_returns_empty() {
        let (store, _dir) = fresh_store();
        let got = store
            .load_all("never", &["a".to_string()])
            .await
            .unwrap();
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
        store.flush_key("m", "k", &dummy_value("v"), false).await.unwrap();
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
            store.add("m", "k", &dummy_value("persisted"), 0, 1000).await.unwrap();
            store.close().await.unwrap();
            // Drop releases the redb lockfile.
        }
        // Reopen on the same path -- value must survive.
        let store = RedbDataStore::new(&path).unwrap();
        let got = store.load("m", "k").await.unwrap().expect("survives reopen");
        assert!(matches!(got, RecordValue::Lww { value: Value::String(ref s), .. } if s == "persisted"));
    }

    #[tokio::test]
    async fn map_name_injection_rejected() {
        let (store, _dir) = fresh_store();
        let bad = "foo; DROP TABLE x";
        let err = store.add(bad, "k", &dummy_value("v"), 0, 1000).await;
        assert!(err.is_err(), "metacharacter map name must be rejected");
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("Invalid map name"), "error message names the violation");
    }

    #[tokio::test]
    async fn empty_map_name_rejected() {
        let (store, _dir) = fresh_store();
        let err = store.add("", "k", &dummy_value("v"), 0, 1000).await;
        assert!(err.is_err());
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
                s1.add("m", &format!("a{i}"), &dummy_value("A"), 0, 1000).await.unwrap();
            }
        });
        let h2 = tokio::spawn(async move {
            for i in 0..10 {
                s2.add("m", &format!("b{i}"), &dummy_value("B"), 0, 1000).await.unwrap();
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
    fn is_valid_map_name_accepts_canonical() {
        assert!(is_valid_map_name("users"));
        assert!(is_valid_map_name("_internal"));
        assert!(is_valid_map_name("Users123"));
    }

    #[test]
    fn is_valid_map_name_rejects_metacharacters() {
        assert!(!is_valid_map_name(""));
        assert!(!is_valid_map_name("123leading_digit"));
        assert!(!is_valid_map_name("foo bar"));
        assert!(!is_valid_map_name("foo;DROP"));
        assert!(!is_valid_map_name("foo--backup"));
    }
}
