//! Admin API request and response types for the TopGun admin dashboard.
//!
//! All types derive `utoipa::ToSchema` for OpenAPI documentation and use
//! `#[serde(rename_all = "camelCase")]` for consistent JSON field naming.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    /// Current RUST_LOG / tracing EnvFilter value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub log_level: Option<String>,
}

/// Partial update for hot-reloadable settings only.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdateRequest {
    /// Update tracing EnvFilter at runtime.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub log_level: Option<String>,
    /// Update GC interval (takes effect on next GC cycle).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gc_interval_ms: Option<u64>,
    /// Update max concurrent operations (takes effect immediately via ArcSwap).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_concurrent_operations: Option<u32>,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub field: Option<String>,
}
