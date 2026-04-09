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
    CreatePolicyRequest, ErrorResponse, IndexInfoResponse, IndexListResponse, IndexTypeParam,
    LoginRequest, LoginResponse, MapInfo, MapsListResponse, NodeStatus, PartitionInfo,
    PolicyListResponse, PolicyResponse, ServerMode, ServerStatusResponse, SettingsResponse,
    SettingsUpdateRequest,
};
use super::AppState;

use crate::cluster::types::NodeState;
use crate::service::domain::index::IndexType;
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
    let username_match =
        req.username.as_bytes().ct_eq(expected_username.as_bytes());
    let password_match =
        req.password.as_bytes().ct_eq(expected_password.as_bytes());

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
        log_level: state.observability.as_ref().and_then(|o| o.current_log_level()),
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

    let id = req
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

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
                message: format!(
                    "no index found for map '{map_name}' attribute '{attribute}'"
                ),
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
    if let Some(progress) = state.backfill_progress.get(&(map_name.clone(), attribute.clone())) {
        return Ok(Json(BackfillStatusResponse {
            map_name,
            attribute,
            total: progress.total.load(Ordering::Relaxed),
            processed: progress.processed.load(Ordering::Relaxed),
            done: progress.done.load(Ordering::Relaxed),
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
        }));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: 404,
            message: format!(
                "no index found for map '{map_name}' attribute '{attribute}'"
            ),
            field: None,
        }),
    ))
}
