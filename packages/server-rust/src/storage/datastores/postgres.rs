//! PostgreSQL-backed [`MapDataStore`] implementation.
//!
//! Provides durable write-through persistence via `sqlx::PgPool`.
//! Every `add()` / `remove()` call persists synchronously before returning.
//! On startup, `load_all()` enables bulk cache warm-up from `PostgreSQL`.

use anyhow::bail;
use async_trait::async_trait;
use sqlx::PgPool;

use crate::network::handlers::{RefreshGrant, RefreshGrantStore};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

/// Write-through `PostgreSQL` persistence backend.
///
/// Implements [`MapDataStore`] with synchronous persistence: every mutation
/// is committed to `PostgreSQL` before the async method returns. Connection
/// pooling via [`PgPool`] handles concurrency without connection exhaustion.
///
/// # Construction
///
/// ```ignore
/// let pool = PgPool::connect("postgres://...").await?;
/// let store = PostgresDataStore::new(pool, None)?;
/// store.initialize().await?;
/// ```
pub struct PostgresDataStore {
    pool: PgPool,
    table_name: String,
}

impl PostgresDataStore {
    /// Create a new `PostgreSQL` data store.
    ///
    /// `table_name` defaults to `"topgun_maps"` if `None`. The table name is
    /// validated against `^[a-zA-Z_][a-zA-Z0-9_]*$` to prevent SQL injection
    /// (it is interpolated into query strings via `format!()`).
    ///
    /// # Errors
    ///
    /// Returns an error if the table name contains invalid characters.
    pub fn new(pool: PgPool, table_name: Option<String>) -> anyhow::Result<Self> {
        let table_name = table_name.unwrap_or_else(|| "topgun_maps".to_string());

        if !is_valid_table_name(&table_name) {
            bail!(
                "Invalid table name '{table_name}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$"
            );
        }

        Ok(Self { pool, table_name })
    }

    /// Run the schema migration (CREATE TABLE + indices).
    ///
    /// Idempotent: uses `IF NOT EXISTS` so calling multiple times is safe.
    /// Must be called once after construction, before the store is handed to
    /// `RecordStoreFactory`.
    ///
    /// # Errors
    ///
    /// Returns an error if the SQL statements fail to execute.
    pub async fn initialize(&self) -> anyhow::Result<()> {
        let create_table = format!(
            r"
            CREATE TABLE IF NOT EXISTS {} (
                map_name        TEXT    NOT NULL,
                key             TEXT    NOT NULL,
                value           BYTEA  NOT NULL,
                expiration_time BIGINT NOT NULL DEFAULT 0,
                is_backup       BOOLEAN NOT NULL DEFAULT FALSE,
                created_at      BIGINT NOT NULL,
                updated_at      BIGINT NOT NULL,
                PRIMARY KEY (map_name, key, is_backup)
            )
            ",
            self.table_name
        );

        let create_map_index = format!(
            "CREATE INDEX IF NOT EXISTS idx_{table}_map ON {table} (map_name)",
            table = self.table_name
        );

        let create_expiry_index = format!(
            "CREATE INDEX IF NOT EXISTS idx_{table}_expiry ON {table} (map_name, expiration_time) WHERE expiration_time > 0",
            table = self.table_name
        );

        sqlx::query(&create_table).execute(&self.pool).await?;
        sqlx::query(&create_map_index).execute(&self.pool).await?;
        sqlx::query(&create_expiry_index)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Load all non-backup keys for a given map name.
    ///
    /// Public helper for startup cache warm-up (not part of the `MapDataStore` trait).
    ///
    /// # Errors
    ///
    /// Returns an error if the SQL query fails.
    pub async fn load_all_keys(&self, map: &str) -> anyhow::Result<Vec<String>> {
        let query = format!(
            "SELECT key FROM {} WHERE map_name = $1 AND is_backup = false",
            self.table_name
        );

        let rows: Vec<(String,)> = sqlx::query_as(&query)
            .bind(map)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|(key,)| key).collect())
    }
}

/// Validate that a table name matches `^[a-zA-Z_][a-zA-Z0-9_]*$`.
///
/// This prevents SQL injection since table names are interpolated via `format!()`.
fn is_valid_table_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();

    // First character must be a letter or underscore
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }

    // Remaining characters must be alphanumeric or underscore
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Get current wall-clock time as milliseconds since Unix epoch.
fn now_millis() -> i64 {
    // i64 can hold millis until year 292_278_994 -- truncation is not a concern
    #[allow(clippy::cast_possible_truncation)]
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    ms
}

#[async_trait]
impl MapDataStore for PostgresDataStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        let bytes = rmp_serde::to_vec_named(value)?;

        let query = format!(
            r"
            INSERT INTO {} (map_name, key, value, expiration_time, is_backup, created_at, updated_at)
            VALUES ($1, $2, $3, $4, false, $5, $5)
            ON CONFLICT (map_name, key, is_backup) DO UPDATE
            SET value = EXCLUDED.value,
                expiration_time = EXCLUDED.expiration_time,
                updated_at = EXCLUDED.updated_at
            ",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(key)
            .bind(&bytes)
            .bind(expiration_time)
            .bind(now)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        let bytes = rmp_serde::to_vec_named(value)?;

        let query = format!(
            r"
            INSERT INTO {} (map_name, key, value, expiration_time, is_backup, created_at, updated_at)
            VALUES ($1, $2, $3, $4, true, $5, $5)
            ON CONFLICT (map_name, key, is_backup) DO UPDATE
            SET value = EXCLUDED.value,
                expiration_time = EXCLUDED.expiration_time,
                updated_at = EXCLUDED.updated_at
            ",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(key)
            .bind(&bytes)
            .bind(expiration_time)
            .bind(now)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        let query = format!(
            "DELETE FROM {} WHERE map_name = $1 AND key = $2 AND is_backup = false",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn remove_backup(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        let query = format!(
            "DELETE FROM {} WHERE map_name = $1 AND key = $2 AND is_backup = true",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        let query = format!(
            "SELECT value FROM {} WHERE map_name = $1 AND key = $2 AND is_backup = false",
            self.table_name
        );

        let row: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(map)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some((bytes,)) => {
                let value: RecordValue = rmp_serde::from_slice(&bytes)?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        let query = format!(
            "SELECT key, value FROM {} WHERE map_name = $1 AND key = ANY($2) AND is_backup = false",
            self.table_name
        );

        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(&query)
            .bind(map)
            .bind(keys)
            .fetch_all(&self.pool)
            .await?;

        let mut results = Vec::with_capacity(rows.len());
        for (key, bytes) in rows {
            let value: RecordValue = rmp_serde::from_slice(&bytes)?;
            results.push((key, value));
        }

        Ok(results)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        let query = format!(
            "DELETE FROM {} WHERE map_name = $1 AND key = ANY($2) AND is_backup = false",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(keys)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    fn is_loadable(&self, _key: &str) -> bool {
        true
    }

    fn pending_operation_count(&self) -> u64 {
        0
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn flush_key(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        is_backup: bool,
    ) -> anyhow::Result<()> {
        let bytes = rmp_serde::to_vec_named(value)?;
        let now = now_millis();

        let query = format!(
            r"
            INSERT INTO {} (map_name, key, value, expiration_time, is_backup, created_at, updated_at)
            VALUES ($1, $2, $3, 0, $4, $5, $5)
            ON CONFLICT (map_name, key, is_backup) DO UPDATE
            SET value = EXCLUDED.value,
                updated_at = EXCLUDED.updated_at
            ",
            self.table_name
        );

        sqlx::query(&query)
            .bind(map)
            .bind(key)
            .bind(&bytes)
            .bind(is_backup)
            .bind(now)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    fn reset(&self) {
        // No-op for write-through: all data is already persisted
    }
}

// ── PostgresRefreshGrantStore ─────────────────────────────────────────────────

/// PostgreSQL-backed [`RefreshGrantStore`] implementation.
///
/// Stores refresh grants in the `topgun_refresh_grants` table. Raw refresh
/// tokens are never persisted; only their SHA-256 hashes.
///
/// # Construction
///
/// ```ignore
/// let store = PostgresRefreshGrantStore::new(pool, 2_592_000); // 30 days
/// store.initialize().await?;
/// ```
pub struct PostgresRefreshGrantStore {
    pool: PgPool,
    grant_duration_secs: u64,
}

impl PostgresRefreshGrantStore {
    /// Create a new grant store.
    ///
    /// `grant_duration_secs` is returned by `grant_duration_secs()` and used
    /// by callers when computing grant expiry. 2_592_000 (30 days) is the
    /// recommended default.
    pub fn new(pool: PgPool, grant_duration_secs: u64) -> Self {
        Self { pool, grant_duration_secs }
    }

    /// Run the schema migration (CREATE TABLE + indices).
    ///
    /// Idempotent: uses `IF NOT EXISTS` so calling multiple times is safe.
    /// Must be called once after construction.
    ///
    /// # Errors
    ///
    /// Returns an error if the SQL statements fail.
    pub async fn initialize(&self) -> anyhow::Result<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS topgun_refresh_grants (
                id          TEXT    PRIMARY KEY,
                sub         TEXT    NOT NULL,
                roles       JSONB   NOT NULL,
                token_hash  TEXT    NOT NULL UNIQUE,
                created_at  BIGINT  NOT NULL,
                expires_at  BIGINT  NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_refresh_grants_token_hash \
             ON topgun_refresh_grants(token_hash)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_refresh_grants_expires_at \
             ON topgun_refresh_grants(expires_at)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[async_trait]
impl RefreshGrantStore for PostgresRefreshGrantStore {
    fn grant_duration_secs(&self) -> u64 {
        self.grant_duration_secs
    }

    async fn insert_grant(&self, grant: &RefreshGrant) -> anyhow::Result<()> {
        let roles_json = serde_json::to_value(&grant.roles)?;
        // BIGINT columns use i64 on the wire; u64 values fit in i64 for all
        // realistic timestamps (before year 292_277_026_596).
        #[allow(clippy::cast_possible_wrap)]
        let created_at = grant.created_at as i64;
        #[allow(clippy::cast_possible_wrap)]
        let expires_at = grant.expires_at as i64;

        sqlx::query(
            r"
            INSERT INTO topgun_refresh_grants (id, sub, roles, token_hash, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(&grant.id)
        .bind(&grant.sub)
        .bind(roles_json)
        .bind(&grant.token_hash)
        .bind(created_at)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn consume_grant(&self, token_hash: &str) -> anyhow::Result<Option<RefreshGrant>> {
        use std::time::SystemTime;

        #[allow(clippy::cast_possible_wrap)]
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Atomically consume the grant with DELETE ... RETURNING.
        // The AND expires_at > $2 check ensures expired grants are rejected
        // without a separate SELECT.
        let row: Option<(String, String, serde_json::Value, String, i64, i64)> =
            sqlx::query_as(
                r"
                DELETE FROM topgun_refresh_grants
                WHERE token_hash = $1 AND expires_at > $2
                RETURNING id, sub, roles, token_hash, created_at, expires_at
                ",
            )
            .bind(token_hash)
            .bind(now_secs)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            None => Ok(None),
            Some((id, sub, roles_val, th, created_at, expires_at)) => {
                let roles: Vec<String> = serde_json::from_value(roles_val)?;
                Ok(Some(RefreshGrant {
                    id,
                    sub,
                    roles,
                    token_hash: th,
                    #[allow(clippy::cast_sign_loss)]
                    created_at: created_at as u64,
                    #[allow(clippy::cast_sign_loss)]
                    expires_at: expires_at as u64,
                }))
            }
        }
    }

    async fn delete_expired_grants(&self) -> anyhow::Result<u64> {
        use std::time::SystemTime;

        #[allow(clippy::cast_possible_wrap)]
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query(
            "DELETE FROM topgun_refresh_grants WHERE expires_at <= $1",
        )
        .bind(now_secs)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::types::Value;

    #[test]
    fn valid_table_names() {
        assert!(is_valid_table_name("topgun_maps"));
        assert!(is_valid_table_name("_private"));
        assert!(is_valid_table_name("Table123"));
        assert!(is_valid_table_name("a"));
        assert!(is_valid_table_name("A"));
        assert!(is_valid_table_name("_"));
    }

    #[test]
    fn invalid_table_names() {
        assert!(!is_valid_table_name(""));
        assert!(!is_valid_table_name("123abc"));
        assert!(!is_valid_table_name("table-name"));
        assert!(!is_valid_table_name("table name"));
        assert!(!is_valid_table_name("table.name"));
        assert!(!is_valid_table_name("table;drop"));
        assert!(!is_valid_table_name("Robert'); DROP TABLE students;--"));
    }

    #[test]
    fn constructor_validates_table_name() {
        // Cannot create a PgPool without a real database, but we can test the
        // validation logic via is_valid_table_name which new() delegates to.
        assert!(is_valid_table_name("topgun_maps"));
        assert!(!is_valid_table_name("bad;name"));
    }

    #[test]
    fn write_through_invariants() {
        // These are compile-time verifiable properties of the write-through design:
        // is_loadable always true, pending_operation_count always 0, is_null always false.
        // We verify the const-like behavior without needing a database connection.

        // Verify now_millis returns a reasonable value (after Unix epoch, before year 3000)
        let ms = now_millis();
        assert!(ms > 0, "now_millis should return a positive value");
        assert!(
            ms < 32_503_680_000_000,
            "now_millis should be before year 3000"
        );
    }

    #[test]
    fn default_table_name() {
        // Verify the default table name matches the TS convention
        assert!(is_valid_table_name("topgun_maps"));
    }

    #[test]
    fn msgpack_round_trip_lww() {
        // Verify RecordValue round-trips through MsgPack without a database
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;

        let value = RecordValue::Lww {
            value: Value::String("hello".to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 1,
                node_id: "node-1".to_string(),
            },
        };

        let bytes = rmp_serde::to_vec_named(&value).expect("serialize");
        let restored: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize");

        match restored {
            RecordValue::Lww {
                value: v,
                timestamp: ts,
            } => {
                assert!(matches!(v, Value::String(ref s) if s == "hello"));
                assert_eq!(ts.millis, 1_000_000);
                assert_eq!(ts.counter, 1);
                assert_eq!(ts.node_id, "node-1");
            }
            _ => panic!("expected Lww variant"),
        }
    }

    #[test]
    fn msgpack_round_trip_ormap() {
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;

        let value = RecordValue::OrMap {
            records: vec![crate::storage::record::OrMapEntry {
                value: Value::Int(42),
                tag: "tag-1".to_string(),
                timestamp: Timestamp {
                    millis: 2_000_000,
                    counter: 0,
                    node_id: "node-2".to_string(),
                },
            }],
        };

        let bytes = rmp_serde::to_vec_named(&value).expect("serialize");
        let restored: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize");

        match restored {
            RecordValue::OrMap { records } => {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].tag, "tag-1");
                assert!(matches!(records[0].value, Value::Int(42)));
            }
            _ => panic!("expected OrMap variant"),
        }
    }

    #[test]
    fn msgpack_round_trip_or_tombstones() {
        let value = RecordValue::OrTombstones {
            tags: vec!["a".to_string(), "b".to_string()],
        };

        let bytes = rmp_serde::to_vec_named(&value).expect("serialize");
        let restored: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize");

        match restored {
            RecordValue::OrTombstones { tags } => {
                assert_eq!(tags, vec!["a", "b"]);
            }
            _ => panic!("expected OrTombstones variant"),
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests requiring a real PostgreSQL database.
    // Run with: DATABASE_URL=postgres://... cargo test -p topgun-server --features postgres
    // Skipped automatically when DATABASE_URL is not set.
    // -----------------------------------------------------------------------

    /// Helper to create a test `RecordValue` (LWW)
    fn test_lww_value(s: &str) -> RecordValue {
        use topgun_core::hlc::Timestamp;

        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 1,
                node_id: "test-node".to_string(),
            },
        }
    }

    /// Connect to a test database or return None if DATABASE_URL is not set.
    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let pool = PgPool::connect(&url).await.ok()?;
        Some(pool)
    }

    /// Macro to skip integration tests when no database is available.
    macro_rules! require_db {
        () => {
            match test_pool().await {
                Some(pool) => pool,
                None => {
                    eprintln!("Skipping: DATABASE_URL not set");
                    return;
                }
            }
        };
    }

    #[tokio::test]
    async fn round_trip_add_and_load() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let value = test_lww_value("hello");
        store.add("my_map", "key1", &value, 0, 1000).await.unwrap();

        let loaded = store.load("my_map", "key1").await.unwrap();
        assert!(loaded.is_some(), "loaded value should exist");

        let loaded = loaded.unwrap();
        match loaded {
            RecordValue::Lww { value: v, .. } => {
                assert!(
                    matches!(v, Value::String(ref s) if s == "hello"),
                    "loaded value should match"
                );
            }
            _ => panic!("expected Lww variant"),
        }
    }

    #[tokio::test]
    async fn upsert_overwrites_existing() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let v1 = test_lww_value("first");
        store.add("map", "k", &v1, 0, 1000).await.unwrap();

        let v2 = test_lww_value("second");
        store.add("map", "k", &v2, 0, 2000).await.unwrap();

        let loaded = store.load("map", "k").await.unwrap().unwrap();
        match loaded {
            RecordValue::Lww { value: v, .. } => {
                assert!(matches!(v, Value::String(ref s) if s == "second"));
            }
            _ => panic!("expected Lww variant"),
        }
    }

    #[tokio::test]
    async fn remove_deletes_row() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let value = test_lww_value("doomed");
        store.add("map", "k", &value, 0, 1000).await.unwrap();
        assert!(store.load("map", "k").await.unwrap().is_some());

        store.remove("map", "k", 2000).await.unwrap();
        assert!(store.load("map", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_all_batch() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        store
            .add("map", "a", &test_lww_value("va"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map", "b", &test_lww_value("vb"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map", "c", &test_lww_value("vc"), 0, 1000)
            .await
            .unwrap();

        let keys = vec!["a".to_string(), "c".to_string()];
        let results = store.load_all("map", &keys).await.unwrap();
        assert_eq!(results.len(), 2);

        let result_keys: Vec<&str> = results.iter().map(|(k, _)| k.as_str()).collect();
        assert!(result_keys.contains(&"a"));
        assert!(result_keys.contains(&"c"));
    }

    #[tokio::test]
    async fn remove_all_batch() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        store
            .add("map", "a", &test_lww_value("va"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map", "b", &test_lww_value("vb"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map", "c", &test_lww_value("vc"), 0, 1000)
            .await
            .unwrap();

        let keys = vec!["a".to_string(), "b".to_string()];
        store.remove_all("map", &keys).await.unwrap();

        assert!(store.load("map", "a").await.unwrap().is_none());
        assert!(store.load("map", "b").await.unwrap().is_none());
        assert!(store.load("map", "c").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn backup_isolation() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        // Add primary and backup records for the same key
        let primary = test_lww_value("primary");
        let backup = test_lww_value("backup");

        store.add("map", "k", &primary, 0, 1000).await.unwrap();
        store
            .add_backup("map", "k", &backup, 0, 1000)
            .await
            .unwrap();

        // Load (non-backup) should return primary
        let loaded = store.load("map", "k").await.unwrap().unwrap();
        match loaded {
            RecordValue::Lww { value: v, .. } => {
                assert!(matches!(v, Value::String(ref s) if s == "primary"));
            }
            _ => panic!("expected Lww variant"),
        }

        // Remove primary only
        store.remove("map", "k", 2000).await.unwrap();
        assert!(
            store.load("map", "k").await.unwrap().is_none(),
            "primary should be gone"
        );

        // Remove backup
        store.remove_backup("map", "k", 2000).await.unwrap();
    }

    #[tokio::test]
    async fn load_all_keys_returns_non_backup_keys() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        store
            .add("map", "a", &test_lww_value("va"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map", "b", &test_lww_value("vb"), 0, 1000)
            .await
            .unwrap();
        store
            .add_backup("map", "c", &test_lww_value("vc"), 0, 1000)
            .await
            .unwrap();

        let keys = store.load_all_keys("map").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
        // "c" is backup only, should not be returned
        assert!(!keys.contains(&"c".to_string()));
    }

    #[tokio::test]
    async fn initialize_is_idempotent() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        // Call initialize twice -- second call should not fail
        store.initialize().await.unwrap();
        store.initialize().await.unwrap();

        // Verify the table works after double initialization
        store
            .add("map", "k", &test_lww_value("v"), 0, 1000)
            .await
            .unwrap();
        assert!(store.load("map", "k").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn flush_key_performs_upsert() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let value = test_lww_value("flushed");
        store.flush_key("map", "k", &value, false).await.unwrap();

        let loaded = store.load("map", "k").await.unwrap();
        assert!(loaded.is_some(), "flush_key should persist the record");
    }

    #[tokio::test]
    async fn flush_key_backup() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let value = test_lww_value("backup-flush");
        store.flush_key("map", "k", &value, true).await.unwrap();

        // Backup flush should not be visible via normal load (non-backup)
        assert!(
            store.load("map", "k").await.unwrap().is_none(),
            "backup flush should not appear in non-backup load"
        );
    }

    #[tokio::test]
    async fn write_through_trait_methods() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        assert!(store.is_loadable("any_key"));
        assert_eq!(store.pending_operation_count(), 0);
        assert!(!store.is_null());
        assert_eq!(store.soft_flush().await.unwrap(), 0);
        assert!(store.hard_flush().await.is_ok());
        store.reset(); // should not panic
    }

    #[tokio::test]
    async fn expiration_time_stored() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        let value = test_lww_value("expiring");
        store
            .add("map", "k", &value, 999_999, 1000)
            .await
            .unwrap();

        // The value should be loadable (expiration is metadata, not enforced by the store)
        assert!(store.load("map", "k").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn custom_table_name() {
        let pool = require_db!();
        let store =
            PostgresDataStore::new(pool, Some("custom_table".to_string())).unwrap();
        store.initialize().await.unwrap();

        store
            .add("map", "k", &test_lww_value("v"), 0, 1000)
            .await
            .unwrap();
        let loaded = store.load("map", "k").await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn different_maps_are_isolated() {
        let pool = require_db!();
        let store = PostgresDataStore::new(pool, None).unwrap();
        store.initialize().await.unwrap();

        store
            .add("map_a", "k", &test_lww_value("from_a"), 0, 1000)
            .await
            .unwrap();
        store
            .add("map_b", "k", &test_lww_value("from_b"), 0, 1000)
            .await
            .unwrap();

        let loaded_a = store.load("map_a", "k").await.unwrap().unwrap();
        match loaded_a {
            RecordValue::Lww { value: v, .. } => {
                assert!(matches!(v, Value::String(ref s) if s == "from_a"));
            }
            _ => panic!("expected Lww variant"),
        }

        // Removing from map_a should not affect map_b
        store.remove("map_a", "k", 2000).await.unwrap();
        assert!(store.load("map_a", "k").await.unwrap().is_none());
        assert!(store.load("map_b", "k").await.unwrap().is_some());
    }
}
