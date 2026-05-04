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

use async_trait::async_trait;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

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

#[async_trait]
impl MapDataStore for RedbDataStore {
    async fn add(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::add — implemented in G2")
    }

    async fn add_backup(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::add_backup — implemented in G2")
    }

    async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::remove — implemented in G2")
    }

    async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::remove_backup — implemented in G2")
    }

    async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
        unimplemented!("RedbDataStore::load — implemented in G2")
    }

    async fn load_all(
        &self,
        _map: &str,
        _keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        unimplemented!("RedbDataStore::load_all — implemented in G2")
    }

    async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::remove_all — implemented in G2")
    }

    fn is_loadable(&self, _key: &str) -> bool {
        unimplemented!("RedbDataStore::is_loadable — implemented in G2")
    }

    fn pending_operation_count(&self) -> u64 {
        unimplemented!("RedbDataStore::pending_operation_count — implemented in G2")
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        unimplemented!("RedbDataStore::soft_flush — implemented in G2")
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::hard_flush — implemented in G2")
    }

    async fn flush_key(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _is_backup: bool,
    ) -> anyhow::Result<()> {
        unimplemented!("RedbDataStore::flush_key — implemented in G2")
    }

    fn reset(&self) {
        unimplemented!("RedbDataStore::reset — implemented in G2")
    }
}
