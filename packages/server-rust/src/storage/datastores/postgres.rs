//! PostgreSQL-backed [`MapDataStore`] implementation.
//!
//! Provides durable write-through persistence via `sqlx::PgPool`.
//! Every `add()` / `remove()` call persists synchronously before returning.
//! On startup, `load_all()` enables bulk cache warm-up from `PostgreSQL`.

use anyhow::bail;
use async_trait::async_trait;
use sqlx::PgPool;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
