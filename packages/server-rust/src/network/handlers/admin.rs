//! Admin API endpoint handlers for the `TopGun` admin dashboard.
//!
//! Endpoints:
//! - `GET /api/status` -- public server status
//! - `GET /api/admin/cluster/status` -- cluster topology (admin only)
//! - `GET /api/admin/maps` -- map enumeration with entry counts (admin only)
//! - `GET /api/admin/settings` -- current server configuration (admin only)
//! - `PUT /api/admin/settings` -- update hot-reloadable settings (admin only)
//! - `GET /api/auth/status` -- auth posture probe (public, no token required)
//! - `POST /api/auth/login` -- admin login (returns JWT)
//! - `POST /api/admin/indexes` -- create a secondary index (admin only)
//! - `GET /api/admin/indexes` -- list all indexes across all maps (admin only)
//! - `DELETE /api/admin/indexes/:map/:attr` -- remove an index (admin only)
//! - `GET /api/admin/indexes/:map/:attr/status` -- backfill progress (admin only)
//! - `DELETE /api/admin/tenants/:tenant` -- delete a tenant's maps (admin only)

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{EncodingKey, Header};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tracing::info;
use tracing_subscriber::EnvFilter;
use utoipa::ToSchema;

use super::admin_auth::AdminClaims;
use super::admin_types::{
    self, BackfillProgress, BackfillStatusResponse, ClusterStatusResponse, CreateIndexRequest,
    CreatePolicyRequest, CreateVectorIndexRequest, ErrorResponse, IndexInfoResponse,
    IndexListResponse, IndexTypeParam, LoginRequest, LoginResponse, MapInfo, MapsListResponse,
    NodeStatus, OptimizeResponse, PartitionInfo, PolicyListResponse, PolicyResponse,
    ScalarIndexDescriptor, ServerMode, ServerStatusResponse, SettingsResponse,
    SettingsUpdateRequest, VectorIndexDescriptor, VectorIndexInfoResponse,
    VectorIndexStatusResponse,
};
use super::AppState;

use crate::cluster::types::NodeState;
use crate::service::domain::index::vector_index::format_iso8601;
use crate::service::domain::index::IndexType;
use crate::service::middleware::authorization::is_reserved_map_name;
use crate::service::middleware::metrics::total_operations;
use crate::service::policy::{
    expr_parser::parse_permission_expr, validate_map_pattern, PermissionAction, PermissionPolicy,
    PolicyEffect, PolicyStore,
};
use crate::storage::{check_tenant, DropOutcome, MapDataStore};
use topgun_core::types::tenant_owns;

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

/// Response body for `GET /api/auth/status`.
///
/// Tells the admin frontend whether authentication is required so it can
/// skip the Login screen when the server runs without a JWT secret.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatusResponse {
    pub auth_required: bool,
}

// ── Public endpoints (no auth required) ──────────────────────────────

/// Returns server status information.
///
/// Always returns `mode: Normal` for v1.0 (bootstrap mode deferred to v1.1).
/// Version is sourced from `CARGO_PKG_VERSION` at compile time.
#[utoipa::path(
    get,
    path = "/api/status",
    responses(
        (status = 200, description = "Server status", body = ServerStatusResponse)
    ),
    tag = "Server"
)]
#[allow(clippy::cast_possible_truncation)]
pub async fn server_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(ServerStatusResponse {
        configured: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode: ServerMode::Normal,
        connections: state.registry.count() as u32,
        uptime_seconds: state.start_time.elapsed().as_secs(),
        total_operations: total_operations(),
    })
}

/// Reports whether authentication is configured on this server.
///
/// The admin frontend calls this on load to decide whether to show the Login
/// screen. When no JWT secret is configured (`TOPGUN_NO_AUTH=1`), auth is not
/// required and the Dashboard is accessible without a token. Always returns
/// HTTP 200 — never 500 — so the frontend can safely fail-open to showing
/// the Login form on any unexpected error.
#[utoipa::path(
    get,
    path = "/api/auth/status",
    responses(
        (status = 200, description = "Auth posture", body = AuthStatusResponse)
    ),
    tag = "Auth"
)]
pub async fn auth_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(AuthStatusResponse {
        auth_required: state.jwt_secret.is_some(),
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
#[utoipa::path(
    post,
    path = "/api/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 500, description = "Server error", body = ErrorResponse),
    ),
    tag = "Auth"
)]
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
                code: 500,
                message: "admin password not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let jwt_secret = state.jwt_secret.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: 500,
                message: "authentication not configured".to_string(),
                field: None,
            }),
        )
    })?;

    // Constant-time comparison to prevent timing attacks.
    let username_match = req.username.as_bytes().ct_eq(expected_username.as_bytes());
    let password_match = req.password.as_bytes().ct_eq(expected_password.as_bytes());

    if username_match.unwrap_u8() != 1 || password_match.unwrap_u8() != 1 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                code: 401,
                message: "invalid credentials".to_string(),
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
                code: 500,
                message: format!("token generation failed: {e}"),
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
#[utoipa::path(
    get,
    path = "/api/admin/cluster/status",
    responses(
        (status = 200, description = "Cluster topology", body = ClusterStatusResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Cluster"
)]
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
/// Lists the maps the store's registry holds, so a map not touched since the
/// last start is listed too. Excludes `$sys/` maps and reserved internal maps
/// (`_topgun_*`, `__topgun_*`, ASCII case-insensitively). The entry count is
/// the resident count, summed across partitions: `0` for a map not loaded
/// into memory, whatever the store holds.
///
/// # Errors
///
/// Returns 503 if the storage layer is not configured or the registry cannot
/// be read.
#[utoipa::path(
    get,
    path = "/api/admin/maps",
    responses(
        (status = 200, description = "List of maps", body = MapsListResponse),
        (status = 503, description = "Storage not configured or unreadable", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Maps"
)]
pub async fn list_maps(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<MapsListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store_factory = state.store_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "storage not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let map_names = store_factory.data_store().list_maps().await.map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: format!("failed to list maps: {e:#}"),
                field: None,
            }),
        )
    })?;
    let maps: Vec<MapInfo> = map_names
        .into_iter()
        .filter(|name| !name.starts_with("$sys/") && !is_reserved_map_name(name))
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
#[utoipa::path(
    get,
    path = "/api/admin/settings",
    responses(
        (status = 200, description = "Current server settings", body = SettingsResponse),
        (status = 503, description = "Config not available", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Settings"
)]
pub async fn get_settings(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let server_config = state.server_config.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "server config not available".to_string(),
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
        log_level: state
            .observability
            .as_ref()
            .and_then(|o| o.current_log_level()),
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
#[utoipa::path(
    put,
    path = "/api/admin/settings",
    request_body = SettingsUpdateRequest,
    responses(
        (status = 200, description = "Updated server settings", body = SettingsResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 503, description = "Config not available", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Settings"
)]
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
                code: 400,
                message: format!("invalid JSON: {e}"),
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
                        code: 400,
                        message: format!("field '{field}' is read-only and cannot be updated"),
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
                code: 400,
                message: format!("invalid settings: {e}"),
                field: None,
            }),
        )
    })?;

    let server_config = state.server_config.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "server config not available".to_string(),
                field: None,
            }),
        )
    })?;

    // Apply hot-reloadable settings via ArcSwap.
    let mut new_config = (**server_config.load()).clone();
    let mut changed = Vec::new();

    if let Some(ref log_level) = req.log_level {
        // Obtain the reload handle from the observability stack.
        // Returns 503 when observability is not configured (no handle) and 400
        // when the provided filter directive is invalid.
        let handle = state
            .observability
            .as_ref()
            .and_then(|o| o.log_level_handle())
            .ok_or_else(|| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(ErrorResponse {
                        code: 503,
                        message: "observability not configured".to_string(),
                        field: None,
                    }),
                )
            })?;

        let new_filter = EnvFilter::try_new(log_level).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: 400,
                    message: format!("invalid log level filter: {e}"),
                    field: Some("logLevel".to_string()),
                }),
            )
        })?;

        handle.reload(new_filter).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: 400,
                    message: format!("failed to reload log level: {e}"),
                    field: None,
                }),
            )
        })?;

        info!(log_level = %log_level, "log level reloaded");
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

// ── Policy admin endpoints ────────────────────────────────────────────

/// Lists all permission policies.
///
/// # Errors
///
/// Returns 503 if the policy store is not configured.
#[utoipa::path(
    get,
    path = "/api/admin/policies",
    responses(
        (status = 200, description = "List of policies", body = PolicyListResponse),
        (status = 503, description = "Policy store not configured", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Policies"
)]
pub async fn list_policies(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<PolicyListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let policy_store = state.policy_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "policy store not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let policies = policy_store
        .list_policies()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: 500,
                    message: format!("failed to list policies: {e}"),
                    field: None,
                }),
            )
        })?
        .into_iter()
        .map(|p| PolicyResponse {
            id: p.id,
            map_pattern: p.map_pattern,
            action: p.action,
            effect: p.effect,
            condition: p.condition,
        })
        .collect();

    Ok(Json(PolicyListResponse { policies }))
}

/// Creates a new permission policy.
///
/// Generates a UUID for the policy id if not provided in the request.
///
/// # Errors
///
/// Returns 503 if the policy store is not configured.
#[utoipa::path(
    post,
    path = "/api/admin/policies",
    request_body = CreatePolicyRequest,
    responses(
        (status = 201, description = "Policy created", body = PolicyResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 503, description = "Policy store not configured", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Policies"
)]
pub async fn create_policy(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Json(req): Json<CreatePolicyRequest>,
) -> Result<(StatusCode, Json<PolicyResponse>), (StatusCode, Json<ErrorResponse>)> {
    let policy_store = state.policy_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "policy store not configured".to_string(),
                field: None,
            }),
        )
    })?;

    if let Err(segment) = validate_map_pattern(&req.map_pattern) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: 400,
                message: format!(
                    "mapPattern segment {segment:?} contains `*` but is neither `*` nor `**`; \
                     it would match only a name spelled with that star"
                ),
                field: Some("mapPattern".to_string()),
            }),
        ));
    }

    let id = req.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Resolve condition: JSON `condition` takes precedence over `condition_expr` string.
    let condition = if req.condition.is_some() {
        req.condition.clone()
    } else if let Some(ref expr) = req.condition_expr {
        let node = parse_permission_expr(expr).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: 400,
                    message: e.to_string(),
                    field: Some("conditionExpr".to_string()),
                }),
            )
        })?;
        Some(node)
    } else {
        None
    };

    let policy = PermissionPolicy {
        id: id.clone(),
        map_pattern: req.map_pattern.clone(),
        action: req.action,
        effect: req.effect,
        condition: condition.clone(),
    };

    policy_store.upsert_policy(policy).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: 500,
                message: format!("failed to create policy: {e}"),
                field: None,
            }),
        )
    })?;

    let response = PolicyResponse {
        id,
        map_pattern: req.map_pattern,
        action: req.action,
        effect: req.effect,
        condition,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Deletes a permission policy by id.
///
/// Returns 204 even if the policy did not exist.
///
/// # Errors
///
/// Returns 503 if the policy store is not configured.
#[utoipa::path(
    delete,
    path = "/api/admin/policies/{id}",
    params(
        ("id" = String, Path, description = "Policy ID to delete")
    ),
    responses(
        (status = 204, description = "Policy deleted"),
        (status = 503, description = "Policy store not configured", body = ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Policies"
)]
pub async fn delete_policy(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let policy_store = state.policy_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "policy store not configured".to_string(),
                field: None,
            }),
        )
    })?;

    policy_store.delete_policy(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: 500,
                message: format!("failed to delete policy: {e}"),
                field: None,
            }),
        )
    })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Tenant deletion ───────────────────────────────────────────────────

/// One map a tenant delete did not drop, with the error it met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantDeleteFailedMap {
    /// The map that is still stored.
    pub map: String,
    /// Why the store did not drop it.
    pub error: String,
}

/// Response body for `DELETE /api/admin/tenants/{tenant}` once the store ran
/// the drop: 200 when every owned map is gone and its index descriptors are
/// removed, else 503 with what is left. Either way the tenant stays denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantDeleteResponse {
    /// The maps this request removed, in name order.
    pub dropped: Vec<String>,
    /// The maps still stored; a re-run drops them.
    pub failed: Vec<TenantDeleteFailedMap>,
    /// What happened and what to do next.
    pub message: String,
}

/// How a tenant delete that passed the checks ended. Built by the task that
/// runs it, which logs it whether or not the caller is still waiting.
enum TenantDeleteEnd {
    /// The deny policy could not be written; nothing was closed or deleted.
    UpsertFailed(String),
    /// The store answered `Busy` or `Unsupported`; this request deleted nothing.
    NotRun(DropOutcome),
    /// The store refused the drop or failed it.
    DropFailed(String),
    /// The drop ran; `descriptors` is the scalar descriptor removal's error.
    Done {
        dropped: Vec<String>,
        failed: Vec<(String, String)>,
        descriptors: Option<String>,
    },
}

fn tenant_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(ErrorResponse {
            code: u32::from(status.as_u16()),
            message,
            field: None,
        }),
    )
        .into_response()
}

/// The id of the deny policy a delete of `tenant` writes.
fn tenant_deny_policy_id(tenant: &str) -> String {
    format!("tenant-deleted:{tenant}")
}

/// Deletes every map a tenant owns (`TG-TEN-002`), on the operator plane only.
///
/// Before anything is written it refuses: 409 with tenancy off, 409 without a
/// configured policy store (a deny policy written into an unconfigured store
/// would turn the whole deployment default-deny), 400 for a tenant the
/// tenant-id rule refuses. Then, in one task the request does not own: (a)
/// upsert the deny policy `tenant-deleted:{tenant}` on `{tenant}.**`, (b) drop
/// the tenant's maps in the store, (c) when the drop ran, remove the scalar and
/// vector index descriptors of every owned map except those the store failed
/// to drop. Every answer after (a) leaves the deny policy in place. A client
/// that disconnects does not stop the task: an accepted delete runs to the
/// end, and its outcome is logged (`tenant delete finished`).
///
/// # Errors
///
/// 409 or 400 before (a), as above; after (a), 409 when another delete is
/// running or the store cannot drop a tenant, 503 when the upsert, the drop or
/// the descriptor removal failed or a map is left.
#[utoipa::path(
    delete,
    path = "/api/admin/tenants/{tenant}",
    params(
        ("tenant" = String, Path, description = "Tenant whose maps (`{tenant}.*`) are deleted")
    ),
    responses(
        (status = 200, description = "Every owned map dropped; the tenant stays denied", body = TenantDeleteResponse),
        (status = 400, description = "Invalid tenant; nothing was changed", body = ErrorResponse),
        (status = 409, description = "Tenancy off or policy store not configured (nothing was changed), or a delete already running / unsupported store (the tenant is denied)", body = ErrorResponse),
        (status = 503, description = "Some maps are left, or the drop failed; the tenant is denied unless the policy upsert itself failed", body = TenantDeleteResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "Tenants"
)]
pub async fn delete_tenant(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path(tenant): Path<String>,
) -> Response {
    delete_tenant_at(state, tenant, scalar_descriptor_path()).await
}

/// [`delete_tenant`] with the scalar descriptor file named by the caller, so
/// tests need not set `TOPGUN_INDEX_PATH` in the process environment.
async fn delete_tenant_at(
    state: AppState,
    tenant: String,
    scalar_path: std::path::PathBuf,
) -> Response {
    if state.config.tenant_claim.is_none() {
        return tenant_error(
            StatusCode::CONFLICT,
            "tenancy is off (TOPGUN_TENANT_CLAIM is not set); nothing was changed".to_string(),
        );
    }
    let policy_store = match &state.policy_store {
        Some(store) if matches!(store.is_configured().await, Ok(true)) => Arc::clone(store),
        _ => {
            return tenant_error(
                StatusCode::CONFLICT,
                "the policy store is not configured: deleting a tenant writes a deny policy, \
                 which would turn every client default-deny; configure RBAC first. \
                 Nothing was changed"
                    .to_string(),
            )
        }
    };
    if let Err(e) = check_tenant(&tenant) {
        return tenant_error(
            StatusCode::BAD_REQUEST,
            format!("{e:#}; nothing was changed"),
        );
    }
    let Some(store_factory) = state.store_factory.as_ref() else {
        return tenant_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage not configured; nothing was changed".to_string(),
        );
    };
    let data_store = store_factory.data_store();
    let vector_path = descriptor_path(&state);

    // Once the deny policy is written, the drop must run to the end: the store
    // keeps the tenant closed after its inner drop starts, so a drop cut short
    // by a dropped request future would leave some maps deleted and the rest
    // closed until a re-run. The task is detached from the request for that.
    let task = {
        use tracing::instrument::WithSubscriber;
        let tenant = tenant.clone();
        tokio::spawn(
            async move {
                let end = run_tenant_delete(
                    &*policy_store,
                    &*data_store,
                    &tenant,
                    &scalar_path,
                    &vector_path,
                )
                .await;
                log_tenant_delete(&tenant, &end);
                end
            }
            .with_current_subscriber(),
        )
    };
    match task.await {
        Ok(end) => tenant_delete_response(&tenant, end),
        Err(e) => tenant_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "the delete of tenant {tenant} failed ({e}); the deny policy {} may have been \
                 written and data may be partly deleted; re-run to finish",
                tenant_deny_policy_id(&tenant)
            ),
        ),
    }
}

/// Steps (a)–(c) of a tenant delete.
async fn run_tenant_delete(
    policy_store: &dyn PolicyStore,
    data_store: &dyn MapDataStore,
    tenant: &str,
    scalar_path: &std::path::Path,
    vector_path: &std::path::Path,
) -> TenantDeleteEnd {
    let deny = PermissionPolicy {
        id: tenant_deny_policy_id(tenant),
        map_pattern: format!("{tenant}.**"),
        action: PermissionAction::All,
        effect: PolicyEffect::Deny,
        condition: None,
    };
    if let Err(e) = policy_store.upsert_policy(deny).await {
        return TenantDeleteEnd::UpsertFailed(e.to_string());
    }
    let (dropped, failed) = match data_store.drop_tenant(tenant).await {
        Ok(DropOutcome::Done { dropped, failed }) => (dropped, failed),
        Ok(outcome) => return TenantDeleteEnd::NotRun(outcome),
        Err(e) => return TenantDeleteEnd::DropFailed(format!("{e:#}")),
    };
    // Every owned descriptor, not only those of `dropped`: a re-run after a
    // crash between a map's drop and its descriptor removal finds the map gone
    // and must still remove the leftover. Read after the drop, each file in
    // one read-filter-write.
    let left = |map: &str| failed.iter().any(|(name, _)| name == map);
    let removable = |map: &str| tenant_owns(tenant, map) && !left(map);
    let descriptors = remove_scalar_descriptors(scalar_path, |d| removable(&d.map_name))
        .err()
        .map(|e| e.to_string());
    remove_vector_descriptors(vector_path, |d| removable(&d.map_name));
    TenantDeleteEnd::Done {
        dropped,
        failed,
        descriptors,
    }
}

fn log_tenant_delete(tenant: &str, end: &TenantDeleteEnd) {
    let (dropped, failed, error): (&[String], &[(String, String)], Option<String>) = match end {
        TenantDeleteEnd::Done {
            dropped,
            failed,
            descriptors,
        } => (
            dropped,
            failed,
            descriptors
                .clone()
                .or_else(|| (!failed.is_empty()).then(|| "some maps were not dropped".to_string())),
        ),
        TenantDeleteEnd::UpsertFailed(e) => (&[], &[], Some(format!("deny policy upsert: {e}"))),
        TenantDeleteEnd::DropFailed(e) => (&[], &[], Some(e.clone())),
        TenantDeleteEnd::NotRun(outcome) => (&[], &[], Some(format!("{outcome:?}"))),
    };
    match error {
        None => info!(tenant, ?dropped, ?failed, "tenant delete finished"),
        Some(error) => tracing::warn!(tenant, ?dropped, ?failed, %error, "tenant delete finished"),
    }
}

fn tenant_delete_response(tenant: &str, end: TenantDeleteEnd) -> Response {
    let policy = tenant_deny_policy_id(tenant);
    match end {
        TenantDeleteEnd::UpsertFailed(e) => tenant_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("failed to write the deny policy {policy}: {e}; nothing was closed or deleted"),
        ),
        TenantDeleteEnd::NotRun(outcome) => {
            let why = if outcome == DropOutcome::Busy {
                "another tenant delete is running on this node"
            } else {
                "this storage backend cannot delete a tenant"
            };
            tenant_error(
                StatusCode::CONFLICT,
                format!(
                    "{why}; this request deleted nothing, but tenant {tenant} is denied \
                     (policy {policy}); re-run to finish"
                ),
            )
        }
        TenantDeleteEnd::DropFailed(e) => tenant_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "the store did not drop tenant {tenant}: {e}; tenant {tenant} is denied \
                 (policy {policy}); data may be partly deleted; re-run to finish"
            ),
        ),
        TenantDeleteEnd::Done {
            dropped,
            failed,
            descriptors,
        } => {
            let ok = failed.is_empty() && descriptors.is_none();
            let message = if ok {
                format!(
                    "tenant {tenant} deleted; it stays denied by policy {policy} — delete that \
                     policy and restart the server to onboard it again"
                )
            } else {
                let what = descriptors.map_or_else(
                    || "some maps were not dropped".to_string(),
                    |e| format!("failed to remove its scalar index descriptors: {e}"),
                );
                format!(
                    "{what}; tenant {tenant} is denied (policy {policy}); data may be partly \
                     deleted; re-run to finish"
                )
            };
            let body = TenantDeleteResponse {
                dropped,
                failed: failed
                    .into_iter()
                    .map(|(map, error)| TenantDeleteFailedMap { map, error })
                    .collect(),
                message,
            };
            let status = if ok {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            (status, Json(body)).into_response()
        }
    }
}

// ── Index admin endpoints ─────────────────────────────────────────────

/// Creates a secondary index on a map attribute and starts async backfill.
///
/// If an index already exists for the (map, attribute) pair, returns 409 Conflict.
/// On success, spawns a background task to populate the index from existing records
/// and returns 201 with the index info.
///
/// # Errors
///
/// Returns 409 if the index already exists, 503 if the index observer factory
/// is not configured.
#[allow(clippy::too_many_lines)]
pub async fn create_index(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Json(req): Json<CreateIndexRequest>,
) -> Result<(StatusCode, Json<IndexInfoResponse>), (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.register_map(&req.map_name);

    if registry.has_index(&req.attribute) {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                code: 409,
                message: format!(
                    "index already exists for map '{}' attribute '{}'",
                    req.map_name, req.attribute
                ),
                field: None,
            }),
        ));
    }

    match req.index_type {
        IndexTypeParam::Hash => registry.add_hash_index(&req.attribute),
        IndexTypeParam::Navigable => registry.add_navigable_index(&req.attribute),
        IndexTypeParam::Inverted => registry.add_inverted_index(&req.attribute),
        // Vector index creation via the admin API is handled by a later spec;
        // this arm prevents a non-exhaustive match compile error.
        IndexTypeParam::Vector => {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(admin_types::ErrorResponse {
                    code: 400,
                    message: "Vector index creation via this endpoint is not yet supported. Use add_vector_index directly.".to_string(),
                    field: None,
                }),
            ));
        }
    }

    // Persist the descriptor so a subsequent restart can rebuild this index.
    // This MUST succeed before we return 201 — the durability-before-ack contract.
    let path = scalar_descriptor_path();
    let mut descriptors = load_scalar_descriptors(&path);
    let now_iso = {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        crate::service::domain::index::vector_index::format_iso8601(secs)
    };
    descriptors.push(ScalarIndexDescriptor {
        map_name: req.map_name.clone(),
        attribute: req.attribute.clone(),
        index_type: req.index_type.clone(),
        created_at: now_iso,
    });
    if let Err(e) = save_scalar_descriptors(&path, &descriptors) {
        // Roll back the in-memory registration so the persisted state matches
        // the state visible to clients. Using the registry handle we just
        // acquired.
        let _ = registry.remove_index(&req.attribute);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: 500,
                message: format!("failed to persist index descriptor: {e}"),
                field: None,
            }),
        ));
    }

    // Spawn a background backfill task to populate the new index with existing records.
    if let Some(store_factory) = state.store_factory.clone() {
        let backfill_progress = Arc::clone(&state.backfill_progress);
        let map_name = req.map_name.clone();
        let attribute = req.attribute.clone();
        let registry_clone = Arc::clone(&registry);

        let progress = Arc::new(BackfillProgress {
            total: std::sync::atomic::AtomicU64::new(0),
            processed: std::sync::atomic::AtomicU64::new(0),
            done: std::sync::atomic::AtomicBool::new(false),
            rebuild_type: crate::network::handlers::admin_types::RebuildType::Backfill,
        });
        backfill_progress.insert((map_name.clone(), attribute.clone()), Arc::clone(&progress));

        tokio::spawn(async move {
            match crate::service::domain::index::scalar_rebuild::backfill_scalar_index(
                &store_factory,
                &map_name,
                &attribute,
                &registry_clone,
                &progress,
            )
            .await
            {
                Ok(total) => {
                    info!(map = %map_name, attribute = %attribute, total = total, "index backfill complete");
                }
                Err(error) => tracing::error!(
                    map = %map_name,
                    attribute = %attribute,
                    error = %error,
                    "index backfill failed: the index holds only the records scanned before the error"
                ),
            }
        });
    }

    let response = IndexInfoResponse {
        map_name: req.map_name,
        attribute: req.attribute,
        index_type: req.index_type,
        entry_count: 0,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Lists all secondary indexes across all maps with their entry counts and types.
///
/// Returns 200 with an `IndexListResponse`.
///
/// # Errors
///
/// Returns 503 if the index observer factory is not configured.
pub async fn list_indexes(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<IndexListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let all_stats = factory.all_index_stats();
    let indexes: Vec<IndexInfoResponse> = all_stats
        .into_iter()
        .flat_map(|(map_name, stats)| {
            stats.into_iter().map(move |s| {
                let index_type = match s.index_type {
                    IndexType::Hash => IndexTypeParam::Hash,
                    IndexType::Navigable => IndexTypeParam::Navigable,
                    IndexType::Inverted => IndexTypeParam::Inverted,
                    IndexType::Vector => IndexTypeParam::Vector,
                };
                IndexInfoResponse {
                    map_name: map_name.clone(),
                    attribute: s.attribute,
                    index_type,
                    entry_count: s.entry_count,
                }
            })
        })
        .collect();

    Ok(Json(IndexListResponse { indexes }))
}

/// Removes a secondary index for the specified map and attribute.
///
/// Also removes the corresponding backfill progress entry to prevent stale state.
/// Returns 404 if no index exists for the (map, attribute) pair.
///
/// # Errors
///
/// Returns 404 if the index does not exist, 503 if the index observer factory
/// is not configured.
pub async fn remove_index_handler(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, attribute)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.get_registry(&map_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no indexes registered for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    if !registry.has_index(&attribute) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no index found for map '{map_name}' attribute '{attribute}'"),
                field: None,
            }),
        ));
    }

    let _ = registry.remove_index(&attribute);

    // Remove stale backfill progress entry to avoid indefinitely accumulating entries.
    state
        .backfill_progress
        .remove(&(map_name.clone(), attribute.clone()));

    // Drop the descriptor entry from disk so a subsequent restart does not
    // re-register a removed index. Disk write failures are logged but do not
    // fail the request — the in-memory removal succeeded and that is the
    // user-observable contract for DELETE.
    if let Err(e) = remove_scalar_descriptors(&scalar_descriptor_path(), |d| {
        d.map_name == map_name && d.attribute == attribute
    }) {
        tracing::warn!(
            target: "topgun_server::index_persistence",
            map = %map_name,
            attribute = %attribute,
            error = %e,
            "failed to persist scalar index descriptor removal; in-memory state already removed"
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Returns the backfill progress for a specific (map, attribute) index.
///
/// If no backfill entry exists but the index does exist, the index was created
/// without a backfill (or backfill was not tracked) — returns `done: true` with
/// zero counts. Returns 404 if neither a backfill entry nor the index exist.
///
/// # Errors
///
/// Returns 404 if the index does not exist and no backfill entry is present,
/// 503 if the index observer factory is not configured.
pub async fn index_backfill_status(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, attribute)): Path<(String, String)>,
) -> Result<Json<BackfillStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Check for an in-progress or completed backfill entry first.
    if let Some(progress) = state
        .backfill_progress
        .get(&(map_name.clone(), attribute.clone()))
    {
        return Ok(Json(BackfillStatusResponse {
            map_name,
            attribute,
            total: progress.total.load(Ordering::Relaxed),
            processed: progress.processed.load(Ordering::Relaxed),
            done: progress.done.load(Ordering::Relaxed),
            rebuild_type: progress.rebuild_type,
        }));
    }

    // No backfill entry: check if the index exists.
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let index_exists = factory
        .get_registry(&map_name)
        .is_some_and(|r| r.has_index(&attribute));

    if index_exists {
        // Index exists but has no backfill record: treat as already done.
        return Ok(Json(BackfillStatusResponse {
            map_name,
            attribute,
            total: 0,
            processed: 0,
            done: true,
            rebuild_type: crate::network::handlers::admin_types::RebuildType::Backfill,
        }));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: 404,
            message: format!("no index found for map '{map_name}' attribute '{attribute}'"),
            field: None,
        }),
    ))
}

// ── Scalar index descriptor persistence ──────────────────────────────────────

/// Path for the scalar index descriptor sidecar JSON file.
///
/// Configured via `TOPGUN_INDEX_PATH`; defaults to `./scalar_indexes.json`.
/// Distinct from `TOPGUN_VECTOR_INDEX_PATH` so the two file rewrites never
/// race each other.
fn scalar_descriptor_path() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("TOPGUN_INDEX_PATH").unwrap_or_else(|_| "./scalar_indexes.json".to_string()),
    )
}

/// Loads all persisted `ScalarIndexDescriptor` entries from the sidecar JSON file.
///
/// Returns an empty `Vec` if the file does not exist (first start). Any parse
/// error is logged and treated as empty to avoid a crash loop on corrupt data.
/// Mirrors `load_vector_descriptors` semantics 1:1.
pub fn load_scalar_descriptors(path: &std::path::Path) -> Vec<ScalarIndexDescriptor> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!(
                target: "topgun_server::index_persistence",
                path = %path.display(),
                error = %e,
                "failed to parse scalar index descriptor file; treating as empty"
            );
            vec![]
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => {
            tracing::warn!(
                target: "topgun_server::index_persistence",
                path = %path.display(),
                error = %e,
                "failed to read scalar index descriptor file; treating as empty"
            );
            vec![]
        }
    }
}

/// Persists the full list of `ScalarIndexDescriptor` entries to the sidecar JSON file.
///
/// Returns `Ok(())` on success or the underlying `std::io::Error` on failure.
/// Unlike `save_vector_descriptors` (which is intentionally infallible), this
/// helper propagates I/O errors so the `create_index` handler can implement
/// durability-before-ack: if the disk write fails, the in-memory registration
/// is rolled back and the client receives 500.
///
/// # Errors
///
/// Returns the underlying `std::io::Error` if serialization fails (mapped via
/// `std::io::Error::other`) or if the disk write fails.
pub fn save_scalar_descriptors(
    path: &std::path::Path,
    descriptors: &[ScalarIndexDescriptor],
) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(descriptors).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// Removes every scalar index descriptor `remove` matches from the sidecar
/// file at `path`, in one read-filter-write with no await in between; the file
/// is written only when a descriptor matched.
///
/// # Errors
///
/// The error of [`save_scalar_descriptors`].
fn remove_scalar_descriptors(
    path: &std::path::Path,
    remove: impl Fn(&ScalarIndexDescriptor) -> bool,
) -> std::io::Result<()> {
    let mut descriptors = load_scalar_descriptors(path);
    let before = descriptors.len();
    descriptors.retain(|d| !remove(d));
    if descriptors.len() == before {
        return Ok(());
    }
    save_scalar_descriptors(path, &descriptors)
}

// ── Vector index descriptor persistence ──────────────────────────────────────

/// Default vector-index descriptor sidecar path used when none is configured.
const DEFAULT_VECTOR_INDEX_PATH: &str = "./vector_indexes.json";

/// Resolves the vector-index descriptor sidecar path from the environment at the
/// construction boundary.
///
/// Production `AppState` wiring calls this once (`module.rs`, `topgun_server.rs`,
/// the load harness) and stores the result in `AppState::vector_index_path`, so
/// the request handlers themselves never touch process-global env — that keeps
/// them testable by direct field injection rather than `set_var`.
///
/// Configured via `TOPGUN_VECTOR_INDEX_PATH`; defaults to `./vector_indexes.json`.
#[must_use]
pub fn vector_index_path_from_env() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("TOPGUN_VECTOR_INDEX_PATH")
            .unwrap_or_else(|_| DEFAULT_VECTOR_INDEX_PATH.to_string()),
    )
}

/// Path for the vector index descriptor sidecar JSON file for this request.
///
/// Reads the injected `AppState::vector_index_path` (resolved from env at the
/// construction boundary), falling back to the default when unset. No env read
/// happens here so the path is fully determined by the injected state.
fn descriptor_path(state: &AppState) -> std::path::PathBuf {
    state
        .vector_index_path
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_VECTOR_INDEX_PATH))
}

/// Loads all persisted `VectorIndexDescriptor` entries from the sidecar JSON file.
///
/// Returns an empty `Vec` if the file does not exist (first start). Any parse
/// error is logged and treated as empty to avoid a crash loop on corrupt data.
pub fn load_vector_descriptors(path: &std::path::Path) -> Vec<VectorIndexDescriptor> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to parse vector index descriptor file; treating as empty"
            );
            vec![]
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to read vector index descriptor file; treating as empty"
            );
            vec![]
        }
    }
}

/// Persists the full list of `VectorIndexDescriptor` entries to the sidecar JSON file.
///
/// Creates the file on first write. Any write error is logged; the function
/// is intentionally infallible to avoid propagating I/O errors into the HTTP path.
pub fn save_vector_descriptors(path: &std::path::Path, descriptors: &[VectorIndexDescriptor]) {
    match serde_json::to_string_pretty(descriptors) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to write vector index descriptor file"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize vector index descriptors");
        }
    }
}

/// Removes every vector index descriptor `remove` matches from the sidecar
/// file at `path`, in one read-filter-write with no await in between; the file
/// is written only when a descriptor matched. Infallible like
/// [`save_vector_descriptors`], which logs a write error.
fn remove_vector_descriptors(
    path: &std::path::Path,
    remove: impl Fn(&VectorIndexDescriptor) -> bool,
) {
    let mut descriptors = load_vector_descriptors(path);
    let before = descriptors.len();
    descriptors.retain(|d| !remove(d));
    if descriptors.len() != before {
        save_vector_descriptors(path, &descriptors);
    }
}

// ── Vector index admin endpoints ──────────────────────────────────────────────

/// Converts a `VectorIndexStats` (domain type) to a `VectorIndexInfoResponse` (wire type).
fn stats_to_response(
    stats: crate::service::domain::index::registry::VectorIndexStats,
) -> VectorIndexInfoResponse {
    VectorIndexInfoResponse {
        attribute: stats.attribute,
        index_name: stats.index_name,
        dimension: stats.dimension,
        distance_metric: stats.distance_metric,
        vector_count: stats.vector_count,
        memory_bytes: stats.memory_bytes,
        graph_layers: stats.graph_layers,
        pending_updates: stats.pending_updates,
        last_optimized: stats.last_optimized,
    }
}

/// Creates a new vector index on a map attribute.
///
/// Returns 201 with the registered index descriptor on success.
///
/// # Errors
///
/// Returns 409 if an index with the same name already exists for (map, attribute).
/// Returns 422 if dimension is 0 or > 4096.
/// Returns 503 if the index observer factory is not configured.
pub async fn create_vector_index(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Json(req): Json<CreateVectorIndexRequest>,
) -> Result<(StatusCode, Json<VectorIndexInfoResponse>), (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    if req.dimension == 0 || req.dimension > 4096 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                code: 422,
                message: format!("dimension must be in range 1..=4096, got {}", req.dimension),
                field: Some("dimension".to_string()),
            }),
        ));
    }

    let registry = factory.register_map(&req.map_name);

    // Check for an existing vector index with the same index_name on this attribute.
    if let Some(existing) = registry.get_vector_index(&req.attribute) {
        if existing.index_name() == req.index_name {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    code: 409,
                    message: format!(
                        "vector index '{}' already exists for map '{}' attribute '{}'",
                        req.index_name, req.map_name, req.attribute
                    ),
                    field: None,
                }),
            ));
        }
    }

    let hnsw_m = req.hnsw_params.as_ref().and_then(|p| p.m).unwrap_or(16);
    let hnsw_ef = req
        .hnsw_params
        .as_ref()
        .and_then(|p| p.ef_construction)
        .unwrap_or(200);
    let dedup_enabled = req.dedup_enabled.unwrap_or(true);

    let vi = registry.add_vector_index_with_params(
        req.attribute.clone(),
        req.index_name.clone(),
        req.dimension,
        req.distance_metric,
        hnsw_m,
        hnsw_ef,
        dedup_enabled,
    );

    // Persist the descriptor so startup rebuild can re-register this index.
    let path = descriptor_path(&state);
    let mut descriptors = load_vector_descriptors(&path);
    let now_iso = {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format_iso8601(secs)
    };
    descriptors.push(VectorIndexDescriptor {
        map_name: req.map_name.clone(),
        attribute: req.attribute.clone(),
        index_name: req.index_name.clone(),
        dimension: req.dimension,
        distance_metric: req.distance_metric,
        hnsw_m,
        hnsw_ef_construction: hnsw_ef,
        dedup_enabled,
        created_at: now_iso,
    });
    save_vector_descriptors(&path, &descriptors);

    info!(
        map = %req.map_name,
        attribute = %req.attribute,
        index_name = %req.index_name,
        dimension = req.dimension,
        "vector index created"
    );

    Ok((StatusCode::CREATED, Json(stats_to_response(vi.stats()))))
}

/// Lists all registered vector indexes across all maps.
///
/// Filters `factory.all_index_stats()` to `IndexType::Vector` and augments with
/// vector-specific stats from `IndexRegistry::vector_index_stats()`.
///
/// # Errors
///
/// Returns 503 if the index observer factory is not configured.
pub async fn list_vector_indexes(
    _claims: AdminClaims,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    // Collect vector indexes from all maps that have at least one Vector index.
    // We use all_index_stats() to enumerate maps, then retrieve vector-specific
    // stats via get_registry() — avoiding a dedicated method on the factory.
    let indexes: Vec<VectorIndexInfoResponse> = factory
        .all_index_stats()
        .into_iter()
        .filter(|(_, stats)| stats.iter().any(|s| s.index_type == IndexType::Vector))
        .flat_map(|(map_name, _)| {
            factory
                .get_registry(&map_name)
                .map(|r| r.vector_index_stats())
                .unwrap_or_default()
        })
        .map(stats_to_response)
        .collect();

    Ok(Json(serde_json::json!({ "indexes": indexes })))
}

/// Removes a vector index for the specified map and index name.
///
/// Returns 204 on success.
///
/// # Errors
///
/// Returns 404 if no vector index exists for (map, name).
/// Returns 503 if the index observer factory is not configured.
pub async fn remove_vector_index_handler(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, index_name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.get_registry(&map_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector indexes registered for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    // Find the attribute for this index_name.
    let attribute = registry
        .find_vector_index_attribute(&index_name)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: 404,
                    message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                    field: None,
                }),
            )
        })?;

    let _ = registry.remove_index(&attribute);

    // Remove from persistent descriptor file.
    remove_vector_descriptors(&descriptor_path(&state), |d| {
        d.map_name == map_name && d.index_name == index_name
    });

    // Clean up backfill progress entry if present.
    state
        .backfill_progress
        .remove(&(map_name.clone(), attribute));

    Ok(StatusCode::NO_CONTENT)
}

/// Triggers an HNSW graph optimize (rebuild) for a vector index.
///
/// Returns 202 with an `OptimizeResponse`. If an optimize is already in progress,
/// returns 202 with `already_running: true` and the existing `optimization_id`.
///
/// # Errors
///
/// Returns 404 if no vector index with the given name exists for the map.
/// Returns 503 if the index observer factory is not configured.
pub async fn optimize_vector_index_handler(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, index_name)): Path<(String, String)>,
) -> Result<(StatusCode, Json<OptimizeResponse>), (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.get_registry(&map_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector indexes registered for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let attribute = registry
        .find_vector_index_attribute(&index_name)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: 404,
                    message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                    field: None,
                }),
            )
        })?;

    let vi = registry.get_vector_index(&attribute).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let (handle, already_running) = vi.optimize();

    Ok((
        StatusCode::ACCEPTED,
        Json(OptimizeResponse {
            optimization_id: handle.id.clone(),
            started_at: handle.started_at.clone(),
            already_running,
        }),
    ))
}

/// Returns the status of a vector index, including any in-progress optimize.
///
/// Returns 200 with a `VectorIndexStatusResponse`.
///
/// # Errors
///
/// Returns 404 if no vector index with the given name exists for the map.
/// Returns 503 if the index observer factory is not configured.
pub async fn vector_index_status(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, index_name)): Path<(String, String)>,
) -> Result<Json<VectorIndexStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.get_registry(&map_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector indexes registered for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let attribute = registry
        .find_vector_index_attribute(&index_name)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: 404,
                    message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                    field: None,
                }),
            )
        })?;

    let vi = registry.get_vector_index(&attribute).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let vi_stats = vi.stats();
    let optimize_handle = vi.current_optimize_handle();
    let (optimize_in_progress, optimize_processed, optimize_total, optimize_cancelled) =
        if let Some(ref h) = optimize_handle {
            let finished = h.finished.load(std::sync::atomic::Ordering::Relaxed);
            let cancelled = h.cancelled.load(std::sync::atomic::Ordering::Relaxed);
            (
                !finished,
                h.processed.load(std::sync::atomic::Ordering::Relaxed),
                h.total.load(std::sync::atomic::Ordering::Relaxed),
                // A run is "cancelled" only when it has both the flag set and is finished.
                cancelled && finished,
            )
        } else {
            (false, 0, 0, false)
        };

    Ok(Json(VectorIndexStatusResponse {
        stats: stats_to_response(vi_stats),
        optimize_in_progress,
        optimize_processed,
        optimize_total,
        optimize_cancelled,
    }))
}

/// Cancels an in-flight HNSW optimize for a vector index.
///
/// Sets `cancelled = true` cooperatively. The rebuild loop checks this flag
/// before each `fresh.insert` call and exits early without swapping the graph.
///
/// # Returns
///
/// - `200 OK` — cancellation flag set (idempotent; calling twice returns 200 both times).
///
/// # Errors
///
/// - `404 NOT FOUND` — no such map/index, or the in-flight optimize's id does not
///   match `optimization_id` (stale cancel request).
/// - `409 CONFLICT` — no in-flight optimize exists for this index (nothing to cancel).
/// - `503 SERVICE UNAVAILABLE` — index observer factory not configured.
pub async fn cancel_vector_index_optimize_handler(
    _claims: AdminClaims,
    State(state): State<AppState>,
    Path((map_name, index_name, optimization_id)): Path<(String, String, String)>,
) -> Result<Json<VectorIndexStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    let factory = state.index_observer_factory.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                code: 503,
                message: "index observer factory not configured".to_string(),
                field: None,
            }),
        )
    })?;

    let registry = factory.get_registry(&map_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let attribute = registry
        .find_vector_index_attribute(&index_name)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: 404,
                    message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                    field: None,
                }),
            )
        })?;

    let vi = registry.get_vector_index(&attribute).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!("no vector index '{index_name}' found for map '{map_name}'"),
                field: None,
            }),
        )
    })?;

    let handle = vi.current_optimize_handle().ok_or_else(|| {
        // No in-flight optimize — nothing to cancel.
        (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                code: 409,
                message: format!(
                    "no in-flight optimize for vector index '{index_name}' on map '{map_name}'"
                ),
                field: None,
            }),
        )
    })?;

    // If the handle has already finished, there is no active optimize to cancel.
    // The optimization_id check below distinguishes "finished with matching id"
    // from "finished with a different id" (stale cancel).
    if handle.id != optimization_id {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!(
                    "optimization '{optimization_id}' not found; current handle id is '{}'",
                    handle.id
                ),
                field: None,
            }),
        ));
    }

    // Set the cancellation flag cooperatively. Idempotent — setting true twice is fine.
    handle.cancelled.store(true, Ordering::Relaxed);

    // Return current status immediately; the background task observes the flag
    // asynchronously and sets finished=true on its next iteration.
    let vi_stats = vi.stats();
    let finished = handle.finished.load(Ordering::Relaxed);
    let cancelled = handle.cancelled.load(Ordering::Relaxed);
    Ok(Json(VectorIndexStatusResponse {
        stats: stats_to_response(vi_stats),
        // The task may still be running at this point; in-progress = !finished.
        optimize_in_progress: !finished,
        optimize_processed: handle.processed.load(Ordering::Relaxed),
        optimize_total: handle.total.load(Ordering::Relaxed),
        // cancelled && finished: may not yet be true if the task hasn't observed
        // the flag yet — callers should poll status until this transitions to true.
        optimize_cancelled: cancelled && finished,
    }))
}

// ── ISO-8601 timestamp helper ─────────────────────────────────────────────────

#[cfg(test)]
mod vector_admin_tests {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use topgun_core::vector::DistanceMetric;

    use super::*;
    use crate::service::domain::index::mutation_observer::IndexObserverFactory;

    fn make_state_with_factory() -> AppState {
        let factory = Arc::new(IndexObserverFactory::new());
        AppState {
            index_observer_factory: Some(factory),
            ..AppState::for_test()
        }
    }

    fn make_state_no_factory() -> AppState {
        AppState::for_test()
    }

    fn make_claims() -> AdminClaims {
        AdminClaims {
            user_id: "test".to_string(),
            roles: vec!["admin".to_string()],
        }
    }

    #[tokio::test]
    async fn create_vector_index_503_when_no_factory() {
        let state = make_state_no_factory();
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let result = create_vector_index(make_claims(), State(state), Json(req)).await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn create_vector_index_422_on_zero_dimension() {
        let state = make_state_with_factory();
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 0,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let result = create_vector_index(make_claims(), State(state), Json(req)).await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_vector_index_422_on_dimension_above_4096() {
        let state = make_state_with_factory();
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4097,
            distance_metric: DistanceMetric::Euclidean,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let result = create_vector_index(make_claims(), State(state), Json(req)).await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn create_vector_index_201_on_valid_request() {
        // Use temp dir for descriptor file to avoid polluting the workspace.
        // Inject the path via AppState instead of mutating process env, so this
        // test cannot race a parallel test reading TOPGUN_VECTOR_INDEX_PATH.
        let tmp = std::env::temp_dir().join(format!("vi_test_{}.json", uuid::Uuid::new_v4()));

        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let result = create_vector_index(make_claims(), State(state), Json(req)).await;
        assert!(result.is_ok(), "expected 201, got err: {:?}", result.err());
        let (code, Json(resp)) = result.unwrap();
        assert_eq!(code, StatusCode::CREATED);
        assert_eq!(resp.dimension, 4);
        assert_eq!(resp.index_name, "emb_idx");
        assert_eq!(resp.vector_count, 0);

        // Clean up temp file.
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn create_vector_index_409_on_duplicate() {
        let tmp = std::env::temp_dir().join(format!("vi_test_{}.json", uuid::Uuid::new_v4()));

        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        // First create succeeds.
        let _ = create_vector_index(make_claims(), State(state.clone()), Json(req.clone())).await;
        // Second create conflicts.
        let result = create_vector_index(make_claims(), State(state), Json(req)).await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn list_vector_indexes_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = list_vector_indexes(make_claims(), State(state)).await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn list_vector_indexes_200_empty_when_none_registered() {
        let state = make_state_with_factory();
        let result = list_vector_indexes(make_claims(), State(state)).await;
        assert!(result.is_ok());
        let Json(body) = result.unwrap();
        let indexes = body["indexes"].as_array().unwrap();
        assert!(indexes.is_empty());
    }

    #[tokio::test]
    async fn list_vector_indexes_reports_vector_count_after_insert() {
        use crate::service::domain::index::Index;

        let tmp = std::env::temp_dir().join(format!("vi_list_{}.json", uuid::Uuid::new_v4()));

        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let _ = create_vector_index(make_claims(), State(state.clone()), Json(req)).await;

        // Insert a record directly against the registered index, then commit.
        let factory = state.index_observer_factory.as_ref().unwrap();
        let registry = factory.get_registry("users").expect("registry exists");
        let vi = registry
            .get_vector_index("_embedding")
            .expect("vector index registered");
        let encoded =
            rmp_serde::to_vec_named(&topgun_core::vector::Vector::F32(vec![0.1, 0.2, 0.3, 0.4]))
                .unwrap();
        let record = rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("_embedding")),
            rmpv::Value::Binary(encoded),
        )]);
        vi.insert("k1", &record);
        vi.commit_pending();

        let result = list_vector_indexes(make_claims(), State(state)).await;
        assert!(result.is_ok());
        let Json(body) = result.unwrap();
        let indexes = body["indexes"].as_array().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0]["vectorCount"].as_u64().unwrap(), 1);

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn remove_vector_index_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = remove_vector_index_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn remove_vector_index_404_when_not_found() {
        let state = make_state_with_factory();
        let result = remove_vector_index_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "missing".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn vector_index_status_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = vector_index_status(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn vector_index_status_404_when_not_found() {
        let state = make_state_with_factory();
        let result = vector_index_status(
            make_claims(),
            State(state),
            Path(("users".to_string(), "missing".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn descriptor_persistence_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("vi_desc_{}.json", uuid::Uuid::new_v4()));
        let descriptors = vec![VectorIndexDescriptor {
            map_name: "maps".to_string(),
            attribute: "_vec".to_string(),
            index_name: "vec_idx".to_string(),
            dimension: 3,
            distance_metric: DistanceMetric::Cosine,
            hnsw_m: 16,
            hnsw_ef_construction: 200,
            dedup_enabled: true,
            created_at: "2026-04-14T00:00:00Z".to_string(),
        }];
        save_vector_descriptors(&tmp, &descriptors);
        let loaded = load_vector_descriptors(&tmp);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].index_name, "vec_idx");
        assert_eq!(loaded[0].dimension, 3);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_vector_descriptors_returns_empty_when_file_absent() {
        let path = std::path::Path::new("/tmp/definitely_does_not_exist_vi.json");
        let result = load_vector_descriptors(path);
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn optimize_vector_index_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = optimize_vector_index_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn optimize_vector_index_404_when_not_found() {
        let state = make_state_with_factory();
        let result = optimize_vector_index_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "missing_idx".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn optimize_vector_index_202_happy_path() {
        let tmp = std::env::temp_dir().join(format!("vi_opt_{}.json", uuid::Uuid::new_v4()));

        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        // First create the index.
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let _ = create_vector_index(make_claims(), State(state.clone()), Json(req)).await;

        // Now optimize it.
        let result = optimize_vector_index_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string())),
        )
        .await;
        assert!(result.is_ok(), "expected 202, got err: {:?}", result.err());
        let (code, Json(resp)) = result.unwrap();
        assert_eq!(code, StatusCode::ACCEPTED);
        assert!(
            !resp.optimization_id.is_empty(),
            "optimization_id should be set"
        );
        assert!(
            !resp.already_running,
            "new optimize should report already_running=false"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    // ── cancel_vector_index_optimize_handler tests ──────────────────────────────

    /// Creates an index and starts an optimize against the descriptor path
    /// already injected into `state` (`state.vector_index_path`), returning the
    /// optimization id. Callers inject a temp path so no two parallel tests share
    /// a descriptor file; the temp file is cleaned up here.
    async fn setup_index_with_optimize(state: &AppState) -> String {
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let _ = create_vector_index(make_claims(), State(state.clone()), Json(req)).await;

        let result = optimize_vector_index_handler(
            make_claims(),
            State(state.clone()),
            Path(("users".to_string(), "emb_idx".to_string())),
        )
        .await
        .expect("optimize should succeed");
        let (_, Json(resp)) = result;
        if let Some(path) = &state.vector_index_path {
            let _ = std::fs::remove_file(path);
        }
        resp.optimization_id
    }

    #[tokio::test]
    async fn cancel_optimize_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path((
                "users".to_string(),
                "emb_idx".to_string(),
                "fake_id".to_string(),
            )),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn cancel_optimize_409_when_no_in_flight_optimize() {
        let tmp = std::env::temp_dir().join(format!("vi_409_{}.json", uuid::Uuid::new_v4()));

        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        // Create index but do NOT start an optimize.
        let req = CreateVectorIndexRequest {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_params: None,
            dedup_enabled: None,
        };
        let _ = create_vector_index(make_claims(), State(state.clone()), Json(req)).await;

        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path((
                "users".to_string(),
                "emb_idx".to_string(),
                "any_id".to_string(),
            )),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);

        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn cancel_optimize_404_when_mismatched_optimization_id() {
        let tmp = std::env::temp_dir().join(format!("vi_c404_{}.json", uuid::Uuid::new_v4()));
        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        let _ = setup_index_with_optimize(&state).await;

        // Use a wrong optimization_id.
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path((
                "users".to_string(),
                "emb_idx".to_string(),
                "wrong_id_xyz".to_string(),
            )),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cancel_optimize_200_happy_path_and_idempotent() {
        let tmp = std::env::temp_dir().join(format!("vi_c200_{}.json", uuid::Uuid::new_v4()));
        let state = AppState {
            vector_index_path: Some(tmp.clone()),
            ..make_state_with_factory()
        };
        let optimization_id = setup_index_with_optimize(&state).await;

        // First cancel — should return 200.
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state.clone()),
            Path((
                "users".to_string(),
                "emb_idx".to_string(),
                optimization_id.clone(),
            )),
        )
        .await;
        assert!(
            result.is_ok(),
            "first cancel should return 200, got: {:?}",
            result.err()
        );
        let Json(resp) = result.unwrap();
        // cancelled flag should be set on the response immediately.
        // (optimize_cancelled may be false until the task observes the flag,
        //  but cancelled is cooperative — the HTTP layer set it; task confirms.)
        // At minimum: the response struct must be valid.
        let _ = resp.stats;

        // Second cancel (idempotent) — should also return 200.
        let result2 = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string(), optimization_id)),
        )
        .await;
        assert!(
            result2.is_ok(),
            "second cancel (idempotent) should return 200"
        );
    }
}

#[cfg(test)]
mod tenant_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::extract::State;
    use axum::http::StatusCode;
    use parking_lot::Mutex;
    use serial_test::serial;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;
    use topgun_core::vector::DistanceMetric;

    use super::*;
    use crate::network::config::NetworkConfig;
    use crate::service::policy::{InMemoryPolicyStore, PolicyError};
    use crate::storage::datastores::kv_engine::{
        KeyRange, KvCapabilities, KvEngine, KvError, KvResult, KvSnapshot, MemKvEngine, WriteBatch,
        WriteOptions,
    };
    use crate::storage::impls::StorageConfig;
    use crate::storage::{
        EngineMapDataStore, NullDataStore, RecordStoreFactory, RecordValue, WriteBehindConfig,
        WriteBehindDataStore,
    };

    const TENANT: &str = "b";

    // Every test that runs a delete is `serial(tenant_delete)`: the
    // `tenant delete finished` event is one callsite, and while another test
    // hits it concurrently the subscriber of
    // `an_accepted_delete_runs_to_the_end_after_the_caller_left` does not see
    // it (the test passes alone and serially).

    fn claims() -> AdminClaims {
        AdminClaims {
            user_id: "op".to_string(),
            roles: vec!["admin".to_string()],
        }
    }

    fn lww(value: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(value.to_string()),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n".to_string(),
            },
            ttl_ms: None,
        }
    }

    /// A policy store that has been configured, as the deployment it guards.
    async fn configured_policies() -> Arc<InMemoryPolicyStore> {
        let store = Arc::new(InMemoryPolicyStore::new());
        store
            .upsert_policy(PermissionPolicy {
                id: "allow-all".to_string(),
                map_pattern: "*".to_string(),
                action: PermissionAction::All,
                effect: PolicyEffect::Allow,
                condition: None,
            })
            .await
            .unwrap();
        store
    }

    /// Descriptor files in a temporary directory, kept alive with it.
    struct Files {
        _dir: tempfile::TempDir,
        scalar: std::path::PathBuf,
        vector: std::path::PathBuf,
    }

    impl Files {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            Self {
                scalar: dir.path().join("scalar_indexes.json"),
                vector: dir.path().join("vector_indexes.json"),
                _dir: dir,
            }
        }

        /// One scalar and one vector index descriptor on each of `maps`.
        fn index(&self, maps: &[&str]) {
            let mut scalar = load_scalar_descriptors(&self.scalar);
            let mut vector = load_vector_descriptors(&self.vector);
            for map in maps {
                scalar.push(ScalarIndexDescriptor {
                    map_name: (*map).to_string(),
                    attribute: "a".to_string(),
                    index_type: IndexTypeParam::Hash,
                    created_at: "t".to_string(),
                });
                vector.push(VectorIndexDescriptor {
                    map_name: (*map).to_string(),
                    attribute: "_embedding".to_string(),
                    index_name: "emb".to_string(),
                    dimension: 4,
                    distance_metric: DistanceMetric::Cosine,
                    hnsw_m: 16,
                    hnsw_ef_construction: 200,
                    dedup_enabled: false,
                    created_at: "t".to_string(),
                });
            }
            save_scalar_descriptors(&self.scalar, &scalar).unwrap();
            save_vector_descriptors(&self.vector, &vector);
        }

        /// The maps with a scalar and with a vector descriptor, sorted.
        fn indexed(&self) -> (Vec<String>, Vec<String>) {
            let mut scalar: Vec<String> = load_scalar_descriptors(&self.scalar)
                .into_iter()
                .map(|d| d.map_name)
                .collect();
            let mut vector: Vec<String> = load_vector_descriptors(&self.vector)
                .into_iter()
                .map(|d| d.map_name)
                .collect();
            scalar.sort();
            vector.sort();
            (scalar, vector)
        }
    }

    fn names(maps: &[&str]) -> Vec<String> {
        maps.iter().map(ToString::to_string).collect()
    }

    fn state(
        data_store: Arc<dyn MapDataStore>,
        policies: Option<Arc<dyn PolicyStore>>,
        tenancy: bool,
        files: &Files,
    ) -> AppState {
        AppState {
            config: Arc::new(NetworkConfig {
                tenant_claim: tenancy.then(|| "org".to_string()),
                ..NetworkConfig::default()
            }),
            store_factory: Some(Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                data_store,
                Vec::new(),
            ))),
            policy_store: policies,
            vector_index_path: Some(files.vector.clone()),
            ..AppState::for_test()
        }
    }

    async fn delete(state: &AppState, tenant: &str, files: &Files) -> (StatusCode, String) {
        let response =
            delete_tenant_at(state.clone(), tenant.to_string(), files.scalar.clone()).await;
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    async fn deny_policy(policies: &dyn PolicyStore, tenant: &str) -> Option<PermissionPolicy> {
        policies
            .list_policies()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == format!("tenant-deleted:{tenant}"))
    }

    /// The deny policy is present with exactly the content R8.1a names.
    async fn assert_denied(policies: &dyn PolicyStore, tenant: &str) {
        let policy = deny_policy(policies, tenant)
            .await
            .unwrap_or_else(|| panic!("the deny policy of {tenant} must be present"));
        assert_eq!(policy.map_pattern, format!("{tenant}.**"));
        assert_eq!(policy.action, PermissionAction::All);
        assert_eq!(policy.effect, PolicyEffect::Deny);
        assert!(policy.condition.is_none());
    }

    fn write_behind_config(write_delay_ms: u64) -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms,
            flush_interval_ms: 10,
            shutdown_timeout_ms: 10_000,
            ..WriteBehindConfig::default()
        }
    }

    /// An engine store over a memory engine, with a write-behind store above.
    fn stores(
        write_delay_ms: u64,
    ) -> (
        Arc<EngineMapDataStore<MemKvEngine>>,
        Arc<WriteBehindDataStore>,
    ) {
        let inner = Arc::new(EngineMapDataStore::open(Arc::new(MemKvEngine::new())).unwrap());
        let store = WriteBehindDataStore::new(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            write_behind_config(write_delay_ms),
        );
        (inner, store)
    }

    async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !cond() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    // ── AC9 ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ac9_create_policy_refuses_a_star_inside_a_segment() {
        for (pattern, accepted) in [
            ("users/*", false),
            ("a*", false),
            ("a.b*", false),
            ("*", true),
            ("**", true),
            ("a.**", true),
            ("*.orders", true),
            ("users/x", true),
        ] {
            let state = AppState {
                policy_store: Some(Arc::new(InMemoryPolicyStore::new())),
                ..AppState::for_test()
            };
            let req = CreatePolicyRequest {
                id: None,
                map_pattern: pattern.to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
                condition_expr: None,
            };
            let result = create_policy(claims(), State(state.clone()), Json(req)).await;
            let stored = state.policy_store.unwrap().list_policies().await.unwrap();
            if accepted {
                assert!(result.is_ok(), "{pattern} must be accepted");
                assert_eq!(stored.len(), 1, "{pattern}");
            } else {
                let (code, Json(body)) = result.expect_err(pattern);
                assert_eq!(code, StatusCode::BAD_REQUEST, "{pattern}");
                assert_eq!(body.field.as_deref(), Some("mapPattern"));
                assert!(stored.is_empty(), "{pattern} must not be stored");
            }
        }
    }

    // ── AC12 ─────────────────────────────────────────────────────────────

    /// `b.x` (primary), `b.y` (backup rows only), `b.z` (only buffered, not in
    /// the registry) are dropped, `bb.z` is intact, the deny policy is there,
    /// the owned descriptors are gone — also on `b.never`, a map that never
    /// stored a record — and a re-run drops nothing and removes a leftover
    /// descriptor.
    #[serial(tenant_delete)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ac12_delete_drops_the_owned_maps_and_a_rerun_finishes() {
        let (inner, store) = stores(300);
        inner.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();
        inner.add_backup("b.y", "k", &lww("y"), 0, 1).await.unwrap();
        inner.add("bb.z", "k", &lww("bbz"), 0, 1).await.unwrap();
        store.add("b.z", "k", &lww("z"), 0, 1).await.unwrap();
        assert!(!inner
            .list_maps()
            .await
            .unwrap()
            .contains(&"b.z".to_string()));
        let files = Files::new();
        files.index(&["b.x", "b.never", "bb.z"]);
        let policies = configured_policies().await;
        let state = state(store.clone(), Some(policies.clone()), true, &files);

        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["dropped"], serde_json::json!(["b.x", "b.y", "b.z"]));
        assert_eq!(body["failed"], serde_json::json!([]));
        assert_denied(&*policies, TENANT).await;
        assert_eq!(files.indexed(), (names(&["bb.z"]), names(&["bb.z"])));
        assert!(store.load("bb.z", "k").await.unwrap().is_some());
        for map in ["b.x", "b.z"] {
            assert!(store.load(map, "k").await.unwrap().is_none(), "{map}");
        }
        // A client of `b` replaying its op on `b.x` is refused; `b.x` stays absent.
        assert!(store.add("b.x", "k", &lww("again"), 0, 2).await.is_err());
        assert!(store.load("b.x", "k").await.unwrap().is_none());
        assert_eq!(inner.list_maps().await.unwrap(), names(&["bb.z"]));

        // A descriptor left from an earlier run is removed by a re-run that
        // drops nothing.
        files.index(&["b.x"]);
        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["dropped"], serde_json::json!([]));
        assert_eq!(files.indexed(), (names(&["bb.z"]), names(&["bb.z"])));
    }

    /// The refusals before (a) leave no deny policy and change nothing.
    #[serial(tenant_delete)]
    #[tokio::test]
    async fn ac12_refusals_before_the_upsert_change_nothing() {
        let files = Files::new();
        files.index(&["b.x"]);
        let (inner, store) = stores(0);
        inner.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();

        // Tenancy off.
        let policies = configured_policies().await;
        let off = state(store.clone(), Some(policies.clone()), false, &files);
        assert_eq!(delete(&off, TENANT, &files).await.0, StatusCode::CONFLICT);
        assert!(deny_policy(&*policies, TENANT).await.is_none());

        // A policy store that exists but was never configured, and none.
        let unconfigured = Arc::new(InMemoryPolicyStore::new());
        let never = state(store.clone(), Some(unconfigured.clone()), true, &files);
        assert_eq!(delete(&never, TENANT, &files).await.0, StatusCode::CONFLICT);
        assert!(!unconfigured.is_configured().await.unwrap());
        assert!(unconfigured.list_policies().await.unwrap().is_empty());
        let none = state(store.clone(), None, true, &files);
        assert_eq!(delete(&none, TENANT, &files).await.0, StatusCode::CONFLICT);

        // A tenant the tenant-id rule refuses.
        let on = state(store.clone(), Some(policies.clone()), true, &files);
        for tenant in ["a/b", "a.b", "", "_TOPGUN_x"] {
            let (status, body) = delete(&on, tenant, &files).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{tenant:?}: {body}");
            assert!(
                deny_policy(&*policies, tenant).await.is_none(),
                "{tenant:?}"
            );
        }

        assert!(store.load("b.x", "k").await.unwrap().is_some());
        store.add("b.x", "k2", &lww("x2"), 0, 2).await.unwrap();
        assert_eq!(files.indexed(), (names(&["b.x"]), names(&["b.x"])));
    }

    /// `Unsupported` (the Null backend) and `Busy` (a second delete while one
    /// waits) are 409s answered after (a): the deny policy is present.
    #[serial(tenant_delete)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ac12_busy_and_unsupported_keep_the_deny_policy() {
        let files = Files::new();
        let policies = configured_policies().await;
        let null = state(
            Arc::new(NullDataStore),
            Some(policies.clone()),
            true,
            &files,
        );
        let (status, body) = delete(&null, TENANT, &files).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("is denied"), "{body}");
        assert_denied(&*policies, TENANT).await;

        let (inner, store) = stores(0);
        inner.add("b.x", "k0", &lww("x"), 0, 1).await.unwrap();
        let state = state(store.clone(), Some(policies.clone()), true, &files);
        let mut park = store.test_park_before_queue_insert("b.x", "k1");
        let write = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.add("b.x", "k1", &lww("x1"), 0, 2).await })
        };
        park.wait_parked().await;
        let first = {
            let (state, scalar) = (state.clone(), files.scalar.clone());
            tokio::spawn(async move { delete_tenant_at(state, TENANT.to_string(), scalar).await })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !first.is_finished(),
            "the first delete waits for the parked write"
        );

        let (status, body) = delete(&state, "c", &files).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("is denied"), "{body}");
        assert_denied(&*policies, "c").await;

        park.release();
        write.await.unwrap().unwrap();
        assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    }

    /// A policy store whose upsert fails, configured as production's is.
    struct FailingUpsert;

    #[async_trait]
    impl PolicyStore for FailingUpsert {
        async fn get_policies(&self, _: &str) -> Result<Vec<PermissionPolicy>, PolicyError> {
            Ok(Vec::new())
        }
        async fn list_policies(&self) -> Result<Vec<PermissionPolicy>, PolicyError> {
            Ok(Vec::new())
        }
        async fn upsert_policy(&self, _: PermissionPolicy) -> Result<(), PolicyError> {
            Err(PolicyError::Internal("injected upsert failure".to_string()))
        }
        async fn delete_policy(&self, _: &str) -> Result<(), PolicyError> {
            Ok(())
        }
        async fn is_configured(&self) -> Result<bool, PolicyError> {
            Ok(true)
        }
    }

    #[serial(tenant_delete)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ac12_a_failed_upsert_is_503_and_closes_nothing() {
        let files = Files::new();
        files.index(&["b.x"]);
        let (inner, store) = stores(0);
        inner.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();
        let state = state(store.clone(), Some(Arc::new(FailingUpsert)), true, &files);
        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(body.contains("nothing was closed"), "{body}");
        store.add("b.x", "k2", &lww("x2"), 0, 2).await.unwrap();
        assert_eq!(inner.list_maps().await.unwrap(), names(&["b.x"]));
        assert_eq!(files.indexed(), (names(&["b.x"]), names(&["b.x"])));
    }

    /// (b) returning an error (a store that is shutting down) → 503, the deny
    /// policy is present, the descriptors are kept.
    #[serial(tenant_delete)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ac12_a_drop_error_is_503_keeps_the_policy_and_the_descriptors() {
        let files = Files::new();
        files.index(&["b.x", "b.never"]);
        let (inner, store) = stores(0);
        inner.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();
        store.hard_flush().await.unwrap();
        let policies = configured_policies().await;
        let state = state(store.clone(), Some(policies.clone()), true, &files);
        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(
            body.contains("is denied") && body.contains("re-run"),
            "{body}"
        );
        assert_denied(&*policies, TENANT).await;
        assert_eq!(
            files.indexed(),
            (names(&["b.never", "b.x"]), names(&["b.never", "b.x"]))
        );
        assert_eq!(inner.list_maps().await.unwrap(), names(&["b.x"]));
    }

    /// An engine whose `n`-th write from arming fails, so the store fails
    /// exactly one map's drop.
    struct FailingWrites {
        inner: MemKvEngine,
        fail_in: Mutex<Option<u64>>,
    }

    impl KvEngine for FailingWrites {
        fn capabilities(&self) -> KvCapabilities {
            self.inner.capabilities()
        }
        fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
            self.inner.get(key)
        }
        fn write(&self, batch: WriteBatch, opts: WriteOptions) -> KvResult<()> {
            let mut fail_in = self.fail_in.lock();
            if let Some(n) = fail_in.as_mut() {
                *n -= 1;
                if *n == 0 {
                    *fail_in = None;
                    return Err(KvError::Other(anyhow::anyhow!("injected write failure")));
                }
            }
            drop(fail_in);
            self.inner.write(batch, opts)
        }
        fn delete(&self, key: &[u8], opts: WriteOptions) -> KvResult<()> {
            self.inner.delete(key, opts)
        }
        fn delete_range(&self, start: &[u8], end: &[u8], opts: WriteOptions) -> KvResult<()> {
            self.inner.delete_range(start, end, opts)
        }
        fn snapshot(&self) -> KvResult<Box<dyn KvSnapshot + '_>> {
            self.inner.snapshot()
        }
        fn ingest(
            &self,
            range: KeyRange,
            sorted: Vec<(Vec<u8>, Vec<u8>)>,
            opts: WriteOptions,
        ) -> KvResult<()> {
            self.inner.ingest(range, sorted, opts)
        }
        fn sync(&self) -> KvResult<()> {
            self.inner.sync()
        }
        fn shutdown(&self) -> KvResult<()> {
            self.inner.shutdown()
        }
    }

    /// A map the store failed to drop is answered in `failed` with a 503 and
    /// keeps its descriptors; the maps it dropped lose theirs.
    #[serial(tenant_delete)]
    #[tokio::test]
    async fn a_map_left_by_the_drop_keeps_its_descriptors() {
        let engine = Arc::new(FailingWrites {
            inner: MemKvEngine::new(),
            fail_in: Mutex::new(None),
        });
        let store = Arc::new(EngineMapDataStore::open(Arc::clone(&engine)).unwrap());
        store.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();
        store.add("b.y", "k", &lww("y"), 0, 1).await.unwrap();
        let files = Files::new();
        files.index(&["b.x", "b.y"]);
        let policies = configured_policies().await;
        let state = state(store.clone(), Some(policies.clone()), true, &files);

        // Maps drop in name order, one batch each: the first batch is `b.x`'s.
        *engine.fail_in.lock() = Some(1);
        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["dropped"], serde_json::json!(["b.y"]));
        assert_eq!(json["failed"][0]["map"], "b.x");
        assert!(json["message"].as_str().unwrap().contains("is denied"));
        assert_denied(&*policies, TENANT).await;
        assert_eq!(files.indexed(), (names(&["b.x"]), names(&["b.x"])));

        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(files.indexed(), (Vec::new(), Vec::new()));
    }

    /// A scalar descriptor file the delete cannot rewrite is a 503, not a
    /// logged and ignored error.
    #[cfg(unix)]
    #[serial(tenant_delete)]
    #[tokio::test]
    async fn a_scalar_descriptor_save_error_is_503() {
        use std::os::unix::fs::PermissionsExt;
        let files = Files::new();
        files.index(&["b.x"]);
        std::fs::set_permissions(&files.scalar, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::OpenOptions::new()
            .write(true)
            .open(&files.scalar)
            .is_ok()
        {
            eprintln!("skipped: this user can write a read-only file (root)");
            return;
        }
        let store = Arc::new(EngineMapDataStore::open(Arc::new(MemKvEngine::new())).unwrap());
        store.add("b.x", "k", &lww("x"), 0, 1).await.unwrap();
        let policies = configured_policies().await;
        let state = state(store.clone(), Some(policies.clone()), true, &files);
        let (status, body) = delete(&state, TENANT, &files).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["dropped"], serde_json::json!(["b.x"]));
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("scalar index descriptors"),
            "{body}"
        );
        assert_denied(&*policies, TENANT).await;
    }

    /// A delete whose caller goes away still runs to the end and logs its
    /// outcome: the request future is dropped while the drop waits for a
    /// parked write; once the write is released, the maps are gone.
    #[serial(tenant_delete)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_accepted_delete_runs_to_the_end_after_the_caller_left() {
        use tracing::instrument::WithSubscriber;

        #[derive(Clone, Default)]
        struct Log(Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Log {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let log = Log::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let log = log.clone();
                move || log.clone()
            })
            .with_ansi(false)
            .finish();

        let files = Files::new();
        files.index(&["b.x"]);
        let (inner, store) = stores(0);
        inner.add("b.x", "k0", &lww("x"), 0, 1).await.unwrap();
        let policies = configured_policies().await;
        let state = state(store.clone(), Some(policies.clone()), true, &files);
        let mut park = store.test_park_before_queue_insert("b.x", "k1");
        let write = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.add("b.x", "k1", &lww("x1"), 0, 2).await })
        };
        park.wait_parked().await;

        let request = delete_tenant_at(state, TENANT.to_string(), files.scalar.clone())
            .with_subscriber(subscriber);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), request)
                .await
                .is_err(),
            "the delete waits for the parked write"
        );
        park.release();
        write.await.unwrap().unwrap();

        eventually("the log line of the finished delete", || {
            String::from_utf8_lossy(&log.0.lock().unwrap()).contains("tenant delete finished")
        })
        .await;
        assert!(inner.list_maps().await.unwrap().is_empty());
        assert!(store.load("b.x", "k0").await.unwrap().is_none());
        assert!(store.add("b.x", "k2", &lww("x2"), 0, 3).await.is_err());
        assert_eq!(files.indexed(), (Vec::new(), Vec::new()));
        assert_denied(&*policies, TENANT).await;
        let line = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
        assert!(line.contains("INFO") && line.contains("b.x"), "{line}");
    }

    // ── AC13 ─────────────────────────────────────────────────────────────

    /// The listing comes from the store's registry: a map no record store
    /// holds (as after a restart) is listed, with a resident count of 0; no
    /// reserved map is, whatever its case.
    #[tokio::test]
    async fn ac13_maps_are_listed_from_the_registry_without_reserved_ones() {
        let store = Arc::new(EngineMapDataStore::open(Arc::new(MemKvEngine::new())).unwrap());
        for map in [
            "orders",
            "_TOPGUN_x",
            "_topgun_device_credentials",
            "__topgun_policies",
            "__TopGun_y",
            "$sys/z",
        ] {
            store.add(map, "k", &lww("v"), 0, 1).await.unwrap();
        }
        let files = Files::new();
        let state = state(store, None, false, &files);
        let Json(body) = list_maps(claims(), State(state)).await.unwrap();
        let listed: Vec<(String, u64)> = body
            .maps
            .into_iter()
            .map(|m| (m.name, m.entry_count))
            .collect();
        assert_eq!(listed, vec![("orders".to_string(), 0)]);
    }
}
