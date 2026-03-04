//! Admin API endpoint handlers for the `TopGun` admin dashboard.
//!
//! Endpoints:
//! - `GET /api/status` -- public server status
//! - `GET /api/admin/cluster/status` -- cluster topology (admin only)
//! - `GET /api/admin/maps` -- map enumeration with entry counts (admin only)
//! - `GET /api/admin/settings` -- current server configuration (admin only)
//! - `PUT /api/admin/settings` -- update hot-reloadable settings (admin only)
//! - `POST /api/auth/login` -- admin login (returns JWT)

use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{EncodingKey, Header};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tracing::info;

use super::admin_auth::AdminClaims;
use super::admin_types::{
    self, ClusterStatusResponse, ErrorResponse, LoginRequest, LoginResponse, MapInfo,
    MapsListResponse, NodeStatus, PartitionInfo, ServerMode, ServerStatusResponse,
    SettingsResponse, SettingsUpdateRequest,
};
use super::AppState;

use crate::cluster::types::NodeState;

/// JWT claims for token generation (encoding).
///
/// Separate from `auth::JwtClaims` which only derives `Deserialize`.
/// This struct adds `exp` for token expiry and derives `Serialize` for
/// `jsonwebtoken::encode`.
#[derive(Serialize)]
struct AdminJwtClaims {
    sub: String,
    roles: Vec<String>,
    exp: u64,
}

// ── Public endpoints (no auth required) ──────────────────────────────

/// Returns server status information.
///
/// Always returns `mode: Normal` for v1.0 (bootstrap mode deferred to v1.1).
/// Version is sourced from `CARGO_PKG_VERSION` at compile time.
pub async fn server_status() -> impl IntoResponse {
    Json(ServerStatusResponse {
        configured: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode: ServerMode::Normal,
    })
}

/// Authenticates an admin user and returns a JWT.
///
/// Validates credentials against `TOPGUN_ADMIN_USERNAME` and `TOPGUN_ADMIN_PASSWORD`
/// environment variables using constant-time comparison to prevent timing attacks.
/// Returns a JWT with `sub` set to the admin username, `roles: ["admin"]`,
/// and 24-hour expiry.
///
/// # Errors
///
/// Returns 401 on invalid credentials, 500 if admin password or JWT secret is not configured.
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let expected_username =
        std::env::var("TOPGUN_ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string());
    let expected_password = std::env::var("TOPGUN_ADMIN_PASSWORD").ok().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "admin password not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let jwt_secret = state.jwt_secret.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "authentication not configured".to_string(),
                field: None,
            }),
        )
    })?;

    // Constant-time comparison to prevent timing attacks.
    let username_match =
        req.username.as_bytes().ct_eq(expected_username.as_bytes());
    let password_match =
        req.password.as_bytes().ct_eq(expected_password.as_bytes());

    if username_match.unwrap_u8() != 1 || password_match.unwrap_u8() != 1 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "invalid credentials".to_string(),
                field: None,
            }),
        ));
    }

    // Generate JWT with 24-hour expiry.
    let exp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 86_400; // 24 hours

    let claims = AdminJwtClaims {
        sub: expected_username,
        roles: vec!["admin".to_string()],
        exp,
    };

    let token = jsonwebtoken::encode(
        &Header::default(), // HS256
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("token generation failed: {e}"),
                field: None,
            }),
        )
    })?;

    info!("Admin login successful");

    Ok(Json(LoginResponse { token }))
}

// ── Admin-only endpoints (require AdminClaims) ──────────────────────

/// Returns cluster topology: nodes, partitions, rebalancing state.
///
/// Maps cluster `NodeState` to admin `NodeStatus`:
/// - `Active` / `Joining` / `Leaving` -> `Healthy`
/// - `Suspect` -> `Suspect`
/// - `Dead` / `Removed` -> `Dead`
#[allow(clippy::cast_possible_truncation)]
pub async fn cluster_status(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(cluster_state) = &state.cluster_state else {
        // Single-node mode: return minimal cluster info with local node.
        let uptime = state.start_time.elapsed().as_secs();
        let local_node = admin_types::NodeInfo {
            node_id: "local".to_string(),
            address: format!("{}:{}", state.config.host, state.config.port),
            status: NodeStatus::Healthy,
            partition_count: 0,
            connections: state.registry.count() as u32,
            memory: 0,
            uptime,
        };
        return Json(ClusterStatusResponse {
            nodes: vec![local_node],
            partitions: Vec::new(),
            total_partitions: 0,
            is_rebalancing: false,
        });
    };

    let view = cluster_state.current_view();
    let uptime = state.start_time.elapsed().as_secs();

    let nodes: Vec<admin_types::NodeInfo> = view
        .members
        .iter()
        .map(|member| {
            let status = match member.state {
                NodeState::Active | NodeState::Joining | NodeState::Leaving => NodeStatus::Healthy,
                NodeState::Suspect => NodeStatus::Suspect,
                NodeState::Dead | NodeState::Removed => NodeStatus::Dead,
            };

            admin_types::NodeInfo {
                node_id: member.node_id.clone(),
                address: format!("{}:{}", member.host, member.client_port),
                status,
                partition_count: 0, // Partition count per node not tracked in MemberInfo
                connections: if member.node_id == cluster_state.local_node_id {
                    state.registry.count() as u32
                } else {
                    0
                },
                memory: 0, // Memory tracking not yet implemented
                uptime,
            }
        })
        .collect();

    // Collect partition info from the partition table.
    let partition_table = &cluster_state.partition_table;
    let total_partitions = partition_table.partition_count();
    let mut partitions = Vec::new();
    for id in 0..total_partitions {
        if let Some(meta) = partition_table.get_partition(id) {
            partitions.push(PartitionInfo {
                id,
                owner_node_id: meta.owner.clone(),
            });
        }
    }
    let is_rebalancing = !cluster_state
        .active_migrations
        .try_read()
        .map(|m| m.is_empty())
        .unwrap_or(true);

    Json(ClusterStatusResponse {
        nodes,
        partitions,
        total_partitions,
        is_rebalancing,
    })
}

/// Lists all non-system maps with entry counts.
///
/// Excludes maps with names starting with `$sys/`.
/// Entry count is summed across all partitions for each map.
///
/// # Errors
///
/// Returns 503 if the storage layer is not configured.
pub async fn list_maps(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<MapsListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store_factory = state.store_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "storage not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let map_names = store_factory.map_names();
    let maps: Vec<MapInfo> = map_names
        .into_iter()
        .filter(|name| !name.starts_with("$sys/"))
        .map(|name| {
            let stores = store_factory.get_all_for_map(&name);
            let entry_count: u64 = stores.iter().map(|s| s.size() as u64).sum();
            MapInfo { name, entry_count }
        })
        .collect();

    Ok(Json(MapsListResponse { maps }))
}

/// Returns the current server configuration.
///
/// Reads from `ArcSwap<ServerConfig>` for general settings, `NetworkConfig`
/// for host/port, and `SecurityConfig` for auth/size limits.
///
/// # Errors
///
/// Returns 503 if the server config is not available.
pub async fn get_settings(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let server_config = state.server_config.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "server config not available".to_string(),
                field: None,
            }),
        )
    })?;

    let config = server_config.load();

    Ok(Json(SettingsResponse {
        node_id: config.node_id.clone(),
        default_operation_timeout_ms: config.default_operation_timeout_ms,
        max_concurrent_operations: config.max_concurrent_operations,
        gc_interval_ms: config.gc_interval_ms,
        partition_count: config.partition_count,
        host: state.config.host.clone(),
        port: state.config.port,
        require_auth: config.security.require_auth,
        max_value_bytes: config.security.max_value_bytes,
        log_level: None, // Runtime log level not yet tracked
    }))
}

/// Read-only field names that cannot be updated via `PUT /api/admin/settings`.
const READONLY_FIELDS: &[&str] = &[
    "nodeId",
    "host",
    "port",
    "partitionCount",
    "requireAuth",
    "maxValueBytes",
    "defaultOperationTimeoutMs",
];

/// Updates hot-reloadable server settings.
///
/// Pre-parses the request body as `serde_json::Value` to detect read-only
/// fields before deserialization. This avoids modifying `admin_types.rs`.
///
/// # Errors
///
/// Returns 400 if JSON is invalid, contains read-only fields, or fails
/// deserialization. Returns 503 if the server config is not available.
pub async fn update_settings(
    claims: AdminClaims,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse raw JSON to check for read-only fields.
    let raw: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid JSON: {e}"),
                field: None,
            }),
        )
    })?;

    if let Some(obj) = raw.as_object() {
        for &field in READONLY_FIELDS {
            if obj.contains_key(field) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("field '{field}' is read-only and cannot be updated"),
                        field: Some(field.to_string()),
                    }),
                ));
            }
        }
    }

    // Deserialize into the typed request.
    let req: SettingsUpdateRequest = serde_json::from_value(raw).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid settings: {e}"),
                field: None,
            }),
        )
    })?;

    let server_config = state.server_config.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "server config not available".to_string(),
                field: None,
            }),
        )
    })?;

    // Apply hot-reloadable settings via ArcSwap.
    let mut new_config = (**server_config.load()).clone();
    let mut changed = Vec::new();

    if let Some(ref log_level) = req.log_level {
        // Log level update: attempt to reload tracing EnvFilter.
        // For now, just log the request. Full EnvFilter reload requires
        // a reload handle which is not yet threaded through AppState.
        info!(log_level = %log_level, "log level update requested");
        changed.push("logLevel");
    }

    if let Some(gc_interval_ms) = req.gc_interval_ms {
        new_config.gc_interval_ms = gc_interval_ms;
        changed.push("gcIntervalMs");
    }

    if let Some(max_concurrent_operations) = req.max_concurrent_operations {
        new_config.max_concurrent_operations = max_concurrent_operations;
        changed.push("maxConcurrentOperations");
    }

    if !changed.is_empty() {
        server_config.store(std::sync::Arc::new(new_config));
        info!(fields = ?changed, "admin settings updated");
    }

    // Return current settings after update.
    get_settings(claims, State(state)).await
}
