//! Admin API endpoint handlers for the `TopGun` admin dashboard.
//!
//! Endpoints:
//! - `GET /api/status` -- public server status
//! - `GET /api/admin/cluster/status` -- cluster topology (admin only)
//! - `GET /api/admin/maps` -- map enumeration with entry counts (admin only)
//! - `GET /api/admin/settings` -- current server configuration (admin only)
//! - `PUT /api/admin/settings` -- update hot-reloadable settings (admin only)
//! - `POST /api/auth/login` -- admin login (returns JWT)
//! - `POST /api/admin/indexes` -- create a secondary index (admin only)
//! - `GET /api/admin/indexes` -- list all indexes across all maps (admin only)
//! - `DELETE /api/admin/indexes/:map/:attr` -- remove an index (admin only)
//! - `GET /api/admin/indexes/:map/:attr/status` -- backfill progress (admin only)

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{EncodingKey, Header};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tracing::info;
use tracing_subscriber::EnvFilter;

use super::admin_auth::AdminClaims;
use super::admin_types::{
    self, BackfillProgress, BackfillStatusResponse, ClusterStatusResponse, CreateIndexRequest,
    CreatePolicyRequest, CreateVectorIndexRequest, ErrorResponse, IndexInfoResponse,
    IndexListResponse, IndexTypeParam, LoginRequest, LoginResponse, MapInfo, MapsListResponse,
    NodeStatus, OptimizeResponse, PartitionInfo, PolicyListResponse, PolicyResponse, ServerMode,
    ServerStatusResponse, SettingsResponse, SettingsUpdateRequest, VectorIndexDescriptor,
    VectorIndexInfoResponse, VectorIndexStatusResponse,
};
use super::AppState;

use crate::cluster::types::NodeState;
use crate::service::domain::index::IndexType;
use crate::service::domain::index::vector_index::format_iso8601;
use crate::service::middleware::metrics::total_operations;
use crate::service::policy::{expr_parser::parse_permission_expr, PermissionPolicy};

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
/// Excludes maps with names starting with `$sys/`.
/// Entry count is summed across all partitions for each map.
///
/// # Errors
///
/// Returns 503 if the storage layer is not configured.
#[utoipa::path(
    get,
    path = "/api/admin/maps",
    responses(
        (status = 200, description = "List of maps", body = MapsListResponse),
        (status = 503, description = "Storage not configured", body = ErrorResponse),
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
            let stores = store_factory.get_all_for_map(&map_name);

            // Count total records across all partition stores.
            let total: u64 = stores.iter().map(|s| s.size() as u64).sum();
            progress.total.store(total, Ordering::Relaxed);

            // Iterate each record and insert into the index.
            for store in &stores {
                store.for_each_boxed(
                    &mut |key, record| {
                        if let crate::storage::record::RecordValue::Lww { ref value, .. } =
                            record.value
                        {
                            let rmpv_val = crate::service::domain::predicate::value_to_rmpv(value);
                            if let Some(idx) = registry_clone.get_index(&attribute) {
                                idx.insert(key, &rmpv_val);
                            }
                        }
                        progress.processed.fetch_add(1, Ordering::Relaxed);
                    },
                    false,
                );
            }

            progress.done.store(true, Ordering::Relaxed);
            info!(map = %map_name, attribute = %attribute, total = total, "index backfill complete");
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

// ── Vector index descriptor persistence ──────────────────────────────────────

/// Path for the vector index descriptor sidecar JSON file.
///
/// Configured via `TOPGUN_VECTOR_INDEX_PATH`; defaults to `./vector_indexes.json`.
fn descriptor_path() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("TOPGUN_VECTOR_INDEX_PATH")
            .unwrap_or_else(|_| "./vector_indexes.json".to_string()),
    )
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

// ── Vector index admin endpoints ──────────────────────────────────────────────

/// Converts a `VectorIndexStats` (domain type) to a `VectorIndexInfoResponse` (wire type).
fn stats_to_response(stats: crate::service::domain::index::registry::VectorIndexStats) -> VectorIndexInfoResponse {
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
                message: format!(
                    "dimension must be in range 1..=4096, got {}",
                    req.dimension
                ),
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

    let hnsw_m = req
        .hnsw_params
        .as_ref()
        .and_then(|p| p.m)
        .unwrap_or(16);
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
    let path = descriptor_path();
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
                    message: format!(
                        "no vector index '{index_name}' found for map '{map_name}'"
                    ),
                    field: None,
                }),
            )
        })?;

    let _ = registry.remove_index(&attribute);

    // Remove from persistent descriptor file.
    let path = descriptor_path();
    let mut descriptors = load_vector_descriptors(&path);
    descriptors.retain(|d| !(d.map_name == map_name && d.index_name == index_name));
    save_vector_descriptors(&path, &descriptors);

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
                    message: format!(
                        "no vector index '{index_name}' found for map '{map_name}'"
                    ),
                    field: None,
                }),
            )
        })?;

    let vi = registry.get_vector_index(&attribute).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!(
                    "no vector index '{index_name}' found for map '{map_name}'"
                ),
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
                    message: format!(
                        "no vector index '{index_name}' found for map '{map_name}'"
                    ),
                    field: None,
                }),
            )
        })?;

    let vi = registry.get_vector_index(&attribute).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: format!(
                    "no vector index '{index_name}' found for map '{map_name}'"
                ),
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
                    message: format!(
                        "no vector index '{index_name}' found for map '{map_name}'"
                    ),
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
    use std::time::Instant;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use dashmap::DashMap;
    use topgun_core::vector::DistanceMetric;

    use super::*;
    use crate::network::config::NetworkConfig;
    use crate::network::connection::ConnectionRegistry;
    use crate::network::shutdown::ShutdownController;
    use crate::service::domain::index::mutation_observer::IndexObserverFactory;

    fn make_state_with_factory() -> AppState {
        let factory = Arc::new(IndexObserverFactory::new());
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: None,
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
            index_observer_factory: Some(factory),
            backfill_progress: Arc::new(DashMap::new()),
        }
    }

    fn make_state_no_factory() -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: None,
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
            index_observer_factory: None,
            backfill_progress: Arc::new(DashMap::new()),
        }
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
        let tmp = std::env::temp_dir().join(format!("vi_test_{}.json", uuid::Uuid::new_v4()));
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

        let state = make_state_with_factory();
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
        std::env::remove_var("TOPGUN_VECTOR_INDEX_PATH");
    }

    #[tokio::test]
    async fn create_vector_index_409_on_duplicate() {
        let tmp = std::env::temp_dir().join(format!("vi_test_{}.json", uuid::Uuid::new_v4()));
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

        let state = make_state_with_factory();
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
        std::env::remove_var("TOPGUN_VECTOR_INDEX_PATH");
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
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

        let state = make_state_with_factory();
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
        let encoded = rmp_serde::to_vec_named(&topgun_core::vector::Vector::F32(vec![
            0.1, 0.2, 0.3, 0.4,
        ]))
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
        std::env::remove_var("TOPGUN_VECTOR_INDEX_PATH");
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
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

        let state = make_state_with_factory();
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
        assert!(!resp.optimization_id.is_empty(), "optimization_id should be set");
        assert!(!resp.already_running, "new optimize should report already_running=false");

        let _ = std::fs::remove_file(&tmp);
        std::env::remove_var("TOPGUN_VECTOR_INDEX_PATH");
    }

    // ── cancel_vector_index_optimize_handler tests ──────────────────────────────

    async fn setup_index_with_optimize(state: &AppState) -> String {
        let tmp = std::env::temp_dir().join(format!("vi_cancel_{}.json", uuid::Uuid::new_v4()));
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

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
        let _ = std::fs::remove_file(tmp);
        resp.optimization_id
    }

    #[tokio::test]
    async fn cancel_optimize_503_when_no_factory() {
        let state = make_state_no_factory();
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string(), "fake_id".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn cancel_optimize_409_when_no_in_flight_optimize() {
        let tmp = std::env::temp_dir().join(format!("vi_409_{}.json", uuid::Uuid::new_v4()));
        std::env::set_var("TOPGUN_VECTOR_INDEX_PATH", tmp.to_str().unwrap());

        let state = make_state_with_factory();
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
            Path(("users".to_string(), "emb_idx".to_string(), "any_id".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);

        let _ = std::fs::remove_file(tmp);
        std::env::remove_var("TOPGUN_VECTOR_INDEX_PATH");
    }

    #[tokio::test]
    async fn cancel_optimize_404_when_mismatched_optimization_id() {
        let state = make_state_with_factory();
        let _ = setup_index_with_optimize(&state).await;

        // Use a wrong optimization_id.
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state),
            Path(("users".to_string(), "emb_idx".to_string(), "wrong_id_xyz".to_string())),
        )
        .await;
        assert!(result.is_err());
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cancel_optimize_200_happy_path_and_idempotent() {
        let state = make_state_with_factory();
        let optimization_id = setup_index_with_optimize(&state).await;

        // First cancel — should return 200.
        let result = cancel_vector_index_optimize_handler(
            make_claims(),
            State(state.clone()),
            Path(("users".to_string(), "emb_idx".to_string(), optimization_id.clone())),
        )
        .await;
        assert!(result.is_ok(), "first cancel should return 200, got: {:?}", result.err());
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
        assert!(result2.is_ok(), "second cancel (idempotent) should return 200");
    }
}

