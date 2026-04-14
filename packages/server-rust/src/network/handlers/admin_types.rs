//! Admin API request and response types for the `TopGun` admin dashboard.
//!
//! All types derive [`ToSchema`] for `OpenAPI` documentation and use
//! `#[serde(rename_all = "camelCase")]` for consistent JSON field naming.

use std::sync::atomic::{AtomicBool, AtomicU64};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::service::policy::{PermissionAction, PolicyEffect};
use topgun_core::messages::base::PredicateNode;
use topgun_core::vector::DistanceMetric;

/// Server operational mode.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ServerMode {
    Normal,
    Bootstrap,
}

/// Node health status.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum NodeStatus {
    Healthy,
    Suspect,
    Dead,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub configured: bool,
    pub version: String,
    pub mode: ServerMode,
    /// Active WebSocket connections.
    pub connections: u32,
    /// Seconds since server start.
    pub uptime_seconds: u64,
    /// Cumulative operations processed through the Tower pipeline.
    pub total_operations: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub node_id: String,
    /// Node listen address (e.g., "127.0.0.1:8080").
    pub address: String,
    pub status: NodeStatus,
    pub partition_count: u32,
    pub connections: u32,
    /// Memory usage in bytes.
    pub memory: u64,
    /// Uptime in seconds.
    pub uptime: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartitionInfo {
    pub id: u32,
    pub owner_node_id: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClusterStatusResponse {
    pub nodes: Vec<NodeInfo>,
    pub partitions: Vec<PartitionInfo>,
    pub total_partitions: u32,
    pub is_rebalancing: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    pub name: String,
    pub entry_count: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MapsListResponse {
    pub maps: Vec<MapInfo>,
}

/// Aggregated server settings for the admin dashboard.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    // General (from ServerConfig)
    pub node_id: String,
    pub default_operation_timeout_ms: u64,
    pub max_concurrent_operations: u32,
    pub gc_interval_ms: u64,
    pub partition_count: u32,

    // Network (from NetworkConfig)
    pub host: String,
    pub port: u16,

    // Security (from SecurityConfig)
    pub require_auth: bool,
    pub max_value_bytes: u64,

    // Runtime (not from config structs)
    /// Current `RUST_LOG` / tracing `EnvFilter` value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub log_level: Option<String>,
}

/// Partial update for hot-reloadable settings only.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdateRequest {
    /// Update tracing `EnvFilter` at runtime.
    #[serde(default)]
    pub log_level: Option<String>,
    /// Update GC interval (takes effect on next GC cycle).
    #[serde(default)]
    pub gc_interval_ms: Option<u64>,
    /// Update max concurrent operations (takes effect immediately via `ArcSwap`).
    #[serde(default)]
    pub max_concurrent_operations: Option<u32>,
}

#[derive(Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub code: u32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub field: Option<String>,
}

// ---------------------------------------------------------------------------
// Policy admin types
// ---------------------------------------------------------------------------

/// Request body for creating a new permission policy.
#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreatePolicyRequest {
    /// Optional; server generates UUID if omitted.
    #[serde(default)]
    pub id: Option<String>,
    pub map_pattern: String,
    pub action: PermissionAction,
    pub effect: PolicyEffect,
    /// Optional predicate condition as a JSON object.
    /// Takes precedence over `condition_expr` when both are provided.
    #[serde(default)]
    #[schema(value_type = Object, nullable = true)]
    pub condition: Option<PredicateNode>,
    /// CEL-like expression string, parsed to `PredicateNode`.
    /// Ignored when `condition` is provided (JSON takes precedence).
    #[serde(default)]
    #[schema(nullable = true)]
    pub condition_expr: Option<String>,
}

/// A single policy as returned by the admin API.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PolicyResponse {
    pub id: String,
    pub map_pattern: String,
    pub action: PermissionAction,
    pub effect: PolicyEffect,
    /// Optional predicate condition as a JSON object.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schema(value_type = Object, nullable = true)]
    pub condition: Option<PredicateNode>,
}

/// Response body for the list policies endpoint.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PolicyListResponse {
    pub policies: Vec<PolicyResponse>,
}

// ---------------------------------------------------------------------------
// Index admin types
// ---------------------------------------------------------------------------

/// Index strategy discriminant for index admin endpoints.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IndexTypeParam {
    Hash,
    Navigable,
    Inverted,
    Vector,
}

/// Request body for creating a new secondary index on a map attribute.
#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexRequest {
    /// Map name to create the index on (e.g., "users").
    pub map_name: String,
    /// Attribute name to index (e.g., "email").
    pub attribute: String,
    /// Index type: "hash", "navigable", or "inverted".
    pub index_type: IndexTypeParam,
}

/// A single index entry as returned by the admin list indexes endpoint.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfoResponse {
    pub map_name: String,
    pub attribute: String,
    pub index_type: IndexTypeParam,
    pub entry_count: u64,
}

/// Response body for the list indexes endpoint.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IndexListResponse {
    pub indexes: Vec<IndexInfoResponse>,
}

/// Response body for the backfill status endpoint.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatusResponse {
    pub map_name: String,
    pub attribute: String,
    /// Total records to backfill.
    pub total: u64,
    /// Records processed so far.
    pub processed: u64,
    /// Whether backfill is complete.
    pub done: bool,
}

/// Tracks async backfill progress for a single (map, attribute) index.
///
/// Stored in `AppState.backfill_progress` keyed by `(map_name, attribute)`.
/// All fields use atomics so the background task and HTTP handler can access
/// progress concurrently without a mutex.
pub struct BackfillProgress {
    pub total: AtomicU64,
    pub processed: AtomicU64,
    pub done: AtomicBool,
}

// ---------------------------------------------------------------------------
// Vector index admin types
// ---------------------------------------------------------------------------

/// HNSW graph-construction parameters for vector index creation.
///
/// All fields are optional; defaults match HNSW community best-practice values.
/// `m` controls graph connectivity (higher = better recall, more memory).
/// `ef_construction` controls search depth during graph build (higher = better
/// recall, slower build).
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HnswParams {
    /// Number of bidirectional links per node (default: 16).
    /// Stored as `u16` (range 2–256); `u32` is NOT used here — this field lives
    /// inside the request body and maps to the HNSW graph parameter directly.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub m: Option<u16>,
    /// Search depth during graph construction (default: 200).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ef_construction: Option<u32>,
}

/// Request body for `POST /api/admin/indexes/vector`.
#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateVectorIndexRequest {
    /// Map name to create the vector index on (e.g., "users").
    pub map_name: String,
    /// Attribute name to index (e.g., "_embedding").
    pub attribute: String,
    /// User-visible name for this index.
    pub index_name: String,
    /// Vector dimensionality; must be in range 1–4096.
    pub dimension: u16,
    /// Distance metric for ANN queries: "cosine", "euclidean", "dotProduct", "manhattan".
    #[schema(value_type = String)]
    pub distance_metric: DistanceMetric,
    /// HNSW graph-construction parameters. Defaults applied when `None`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hnsw_params: Option<HnswParams>,
    /// Enable BLAKE3-based duplicate vector suppression (default: `true`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dedup_enabled: Option<bool>,
}

/// Wire representation of a vector index for admin API list/create responses.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexInfoResponse {
    /// Attribute name this index covers (e.g., `_embedding`).
    pub attribute: String,
    /// User-visible index name.
    pub index_name: String,
    /// Vector dimensionality.
    pub dimension: u16,
    /// Distance metric used for ANN queries: "cosine", "euclidean", "dotProduct", "manhattan".
    #[schema(value_type = String)]
    pub distance_metric: DistanceMetric,
    /// Committed vector count (does not include pending writes).
    pub vector_count: u64,
    /// Estimated HNSW graph memory usage in bytes.
    pub memory_bytes: u64,
    /// Number of HNSW graph layers. `u32` for Hazelcast codec parity.
    pub graph_layers: u32,
    /// Number of pending (uncommitted) mutations in the write buffer.
    pub pending_updates: u64,
    /// ISO-8601 UTC timestamp of the last completed optimize, or `null`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_optimized: Option<String>,
}

/// Response body for `POST /api/admin/indexes/vector/{map}/{name}/optimize`.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeResponse {
    /// Unique identifier for this optimize run (UUID v4).
    pub optimization_id: String,
    /// ISO-8601 UTC timestamp when the optimize started.
    pub started_at: String,
    /// `true` when this call returned an already-in-progress optimize;
    /// `false` when a new optimize was started. Both cases return HTTP 202.
    pub already_running: bool,
}

/// Response body for `GET /api/admin/indexes/vector/{map}/{name}/status`.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexStatusResponse {
    /// Current index statistics.
    pub stats: VectorIndexInfoResponse,
    /// Whether an optimize is currently in progress.
    pub optimize_in_progress: bool,
    /// Vectors processed so far during the current (or last) optimize.
    pub optimize_processed: u64,
    /// Total vectors to process in the current (or last) optimize.
    pub optimize_total: u64,
}

/// On-disk descriptor for a registered vector index.
///
/// Persisted to `TOPGUN_VECTOR_INDEX_PATH` (default: `./vector_indexes.json`)
/// as a JSON array. Loaded at startup to re-register vector indexes and rebuild
/// their HNSW graphs from persisted records.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexDescriptor {
    /// Map name this index belongs to.
    pub map_name: String,
    /// Attribute name this index covers.
    pub attribute: String,
    /// User-visible index name.
    pub index_name: String,
    /// Vector dimensionality.
    pub dimension: u16,
    /// Distance metric.
    pub distance_metric: DistanceMetric,
    /// HNSW `m` parameter (bidirectional links per node).
    pub hnsw_m: u16,
    /// HNSW `ef_construction` parameter (search depth during graph build).
    pub hnsw_ef_construction: u32,
    /// Whether BLAKE3 dedup is enabled for this index.
    pub dedup_enabled: bool,
    /// ISO-8601 UTC timestamp when this index was originally created.
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_response_serializes_code_and_message() {
        let resp = ErrorResponse {
            code: 400,
            message: "bad request".to_string(),
            field: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 400);
        assert_eq!(json["message"], "bad request");
        assert!(
            json.get("field").is_none(),
            "field should be omitted when None"
        );
        assert!(
            json.get("error").is_none(),
            "old 'error' key must not appear"
        );
    }

    #[test]
    fn error_response_includes_field_when_present() {
        let resp = ErrorResponse {
            code: 400,
            message: "invalid value".to_string(),
            field: Some("nodeId".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 400);
        assert_eq!(json["message"], "invalid value");
        assert_eq!(json["field"], "nodeId");
    }

    #[test]
    fn error_response_code_is_integer_not_float() {
        let resp = ErrorResponse {
            code: 500,
            message: "internal".to_string(),
            field: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["code"].is_u64(), "code must serialize as integer");
    }

    #[test]
    fn server_status_response_includes_metrics_fields() {
        let resp = ServerStatusResponse {
            configured: true,
            version: "0.1.0".to_string(),
            mode: ServerMode::Normal,
            connections: 42,
            uptime_seconds: 3600,
            total_operations: 999,
        };
        let json = serde_json::to_value(&resp).unwrap();

        // Existing fields unchanged
        assert_eq!(json["configured"], true);
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["mode"], "normal");

        // New metrics fields present with camelCase keys
        assert_eq!(json["connections"], 42);
        assert_eq!(json["uptimeSeconds"], 3600);
        assert_eq!(json["totalOperations"], 999);

        // Integer types, not floats
        assert!(json["connections"].is_u64(), "connections must be integer");
        assert!(
            json["uptimeSeconds"].is_u64(),
            "uptimeSeconds must be integer"
        );
        assert!(
            json["totalOperations"].is_u64(),
            "totalOperations must be integer"
        );
    }
}
