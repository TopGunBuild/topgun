use std::sync::Arc;

use async_trait::async_trait;
use topgun_core::{
    CrdtMap, MapSchema, MapType, RequestContext, StorageValue, SyncShape, ValidationResult, Value,
};

/// Pluggable persistence backend for the server.
/// Implementations: `PostgreSQL`, `SQLite`, S3 (future), memory (tests).
#[async_trait]
pub trait ServerStorage: Send + Sync {
    /// Load a single record by map name and key.
    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<StorageValue>>;

    /// Load multiple records by map name and a set of keys.
    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, StorageValue)>>;

    /// Load all keys present in the given map.
    async fn load_all_keys(&self, map: &str) -> anyhow::Result<Vec<String>>;

    /// Store a single record.
    async fn store(&self, map: &str, key: &str, value: &StorageValue) -> anyhow::Result<()>;

    /// Store multiple records in a single batch.
    async fn store_all(
        &self,
        map: &str,
        records: &[(String, StorageValue)],
    ) -> anyhow::Result<()>;

    /// Delete a single record by map name and key.
    async fn delete(&self, map: &str, key: &str) -> anyhow::Result<()>;

    /// Delete multiple records by map name and a set of keys.
    async fn delete_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()>;

    /// One-time initialization (e.g., create tables, run migrations).
    async fn initialize(&self) -> anyhow::Result<()>;

    /// Release resources and close connections.
    async fn close(&self) -> anyhow::Result<()>;
}

/// Async map access with tiered storage awareness.
/// Abstracts whether a map is in memory, being loaded from disk, or evicted.
#[async_trait]
pub trait MapProvider: Send + Sync {
    /// Get a map if it is currently loaded in memory.
    async fn get_map(&self, name: &str) -> Option<Arc<CrdtMap>>;

    /// Get a map, loading it from storage if necessary.
    async fn get_or_load_map(
        &self,
        name: &str,
        type_hint: MapType,
    ) -> anyhow::Result<Arc<CrdtMap>>;

    /// Check whether a map is currently loaded in memory (non-blocking).
    fn has_map(&self, name: &str) -> bool;
}

/// Schema validation and partial replication shape computation.
/// Controls what data clients receive and validates writes against schemas.
#[async_trait]
pub trait SchemaProvider: Send + Sync {
    /// Retrieve the schema for a given map, if one has been registered.
    async fn get_schema(&self, map_name: &str) -> Option<MapSchema>;

    /// Register a schema for a map. Overwrites any existing schema.
    async fn register_schema(&self, map_name: &str, schema: MapSchema) -> anyhow::Result<()>;

    /// Synchronously validate a value against the map's schema.
    fn validate(&self, map_name: &str, value: &Value) -> ValidationResult;

    /// Compute the sync shape for a client given their request context.
    /// Returns `None` if the client has no access or no shape is configured.
    async fn get_shape(
        &self,
        map_name: &str,
        client_ctx: &RequestContext,
    ) -> Option<SyncShape>;
}
