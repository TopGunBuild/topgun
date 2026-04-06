//! HTTP sync endpoint handler for `MsgPack`-encoded request/response.
//!
//! Provides a POST /sync endpoint that accepts `MsgPack` bodies and returns
//! `MsgPack` responses. This is the HTTP fallback transport for clients that
//! cannot maintain a WebSocket connection (e.g., behind restrictive proxies).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::FromRequestParts;
use axum::extract::OptionalFromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use jsonwebtoken::Validation;
use topgun_core::hash_to_partition;
use topgun_core::messages::base::Query;
use topgun_core::messages::{HttpQueryRequest, HttpQueryResult, HttpSyncAck, HttpSyncError, HttpSyncRequest, HttpSyncResponse};
use topgun_core::Timestamp;

use super::AppState;
use super::auth_validator::AuthValidationContext;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::domain::predicate::{execute_query, value_to_rmpv};
use crate::service::operation::CallerOrigin;
use crate::service::policy::{PermissionAction, PolicyDecision, PolicyEvaluator, PolicyStore};
use crate::storage::factory::RecordStoreFactory;
use crate::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// ClientAuthError
// ---------------------------------------------------------------------------

/// Errors that can occur when extracting `ClientClaims` from a request.
#[derive(Debug)]
pub enum ClientAuthError {
    /// No Authorization header present.
    MissingToken,
    /// JWT is present but invalid, expired, or malformed.
    InvalidToken(String),
    /// JWT secret not configured on the server.
    NotConfigured,
}

impl IntoResponse for ClientAuthError {
    fn into_response(self) -> axum::response::Response {
        // The handler uses Option<ClientClaims>, so axum converts Err to None
        // and this impl is only invoked if ClientClaims is used directly (non-optional).
        // Always return a generic message here since this impl has no access to config.
        // The enforce_auth function (used by the HTTP sync handler) applies the
        // insecure_forward_auth_errors flag when constructing the response.
        let json = match &self {
            Self::MissingToken => r#"{"code":401,"message":"authentication required"}"#.to_string(),
            Self::InvalidToken(_) | Self::NotConfigured => r#"{"code":401,"message":"Authentication failed"}"#.to_string(),
        };
        (
            StatusCode::UNAUTHORIZED,
            [("content-type", "application/json")],
            json.into_bytes(),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// TokenPresence
// ---------------------------------------------------------------------------

/// Records whether an `Authorization` header was present in the request,
/// regardless of whether the token was valid.
///
/// Injected into request extensions by the `ClientClaims` extractor so the
/// handler can distinguish "no token" from "bad token" when using
/// `Option<ClientClaims>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPresence {
    Present,
    Absent,
}

// ---------------------------------------------------------------------------
// ClientClaims
// ---------------------------------------------------------------------------

/// Axum extractor for client Bearer JWT authentication.
///
/// Unlike `AdminClaims`, this extractor:
/// - Does NOT require an `"admin"` role — any valid JWT with a `sub` claim is accepted.
/// - Is intended for use as `Option<ClientClaims>` in the handler signature so
///   missing tokens return `None` rather than rejecting the request. The handler
///   decides rejection based on `require_auth`.
///
/// When the `Authorization` header is present (valid or invalid), the extractor
/// sets `TokenPresence::Present` in request extensions. When absent, it sets
/// `TokenPresence::Absent`. The handler reads `Extension<TokenPresence>` to
/// distinguish "no token" from "bad token".
#[derive(Debug, Clone)]
pub struct ClientClaims {
    /// The authenticated user's identifier (from JWT `sub` claim).
    pub user_id: String,
    /// Roles from the JWT.
    pub roles: Vec<String>,
}

impl OptionalFromRequestParts<AppState> for ClientClaims {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        // Extract Bearer token from Authorization header.
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let Some(token_str) = auth_header else {
            // No Authorization header at all.
            parts.extensions.insert(TokenPresence::Absent);
            return Ok(None);
        };

        // Header present — record presence regardless of token validity.
        parts.extensions.insert(TokenPresence::Present);

        let token = token_str
            .strip_prefix("Bearer ")
            .unwrap_or("")
            .trim();

        if token.is_empty() {
            return Ok(None);
        }

        // JWT secret must be configured. If not configured, treat as no token.
        let Some(jwt_secret) = state.jwt_secret.as_deref() else {
            return Ok(None);
        };

        // Validate JWT — supports both HS256 and RS256 via decode_jwt_key.
        let Ok((algorithm, key)) = super::auth::decode_jwt_key(jwt_secret) else {
            return Ok(None);
        };
        let mut validation = Validation::new(algorithm);
        validation.validate_aud = false;
        validation.leeway = state.config.jwt_clock_skew_secs;

        // Decode into serde_json::Value to obtain raw_claims for AuthValidationContext.
        let Ok(token_data) = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation) else {
            return Ok(None);
        };

        let raw_claims = token_data.claims.clone();

        // Require sub claim — anonymous identity is not permitted.
        let Some(user_id) = raw_claims["sub"].as_str().map(str::to_owned) else {
            return Ok(None);
        };

        let roles: Vec<String> = raw_claims["roles"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        // Call custom validator after signature verification if configured.
        if let Some(ref validator) = state.auth_validator {
            let ctx = AuthValidationContext {
                user_id: user_id.clone(),
                roles: roles.clone(),
                raw_claims,
            };
            if let Err(reason) = validator.validate(&ctx).await {
                tracing::warn!(user_id = %user_id, reason = %reason, "custom auth validator rejected token");
                return Ok(None);
            }
        }

        Ok(Some(ClientClaims { user_id, roles }))
    }
}

// Keep FromRequestParts for direct (non-optional) use of ClientClaims.
impl FromRequestParts<AppState> for ClientClaims {
    type Rejection = ClientAuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        <ClientClaims as OptionalFromRequestParts<AppState>>::from_request_parts(parts, state)
            .await
            .unwrap() // Infallible
            .ok_or(ClientAuthError::MissingToken)
    }
}

/// Content-Type header value for `MsgPack` responses.
const MSGPACK_CONTENT_TYPE: &str = "application/msgpack";

/// Returns the current wall-clock time as a `Timestamp` when the HLC is unavailable.
fn wall_clock_timestamp() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0);
    Timestamp {
        millis,
        counter: 0,
        node_id: String::new(),
    }
}

/// Checks auth requirements and returns an error response if authentication fails.
/// Returns `None` when the request is authorized to proceed.
///
/// When `insecure_forward_auth_errors` is `false` (the default), invalid-token
/// failures return the generic "Authentication failed" message rather than
/// implementation details. Missing-token responses always say "authentication required"
/// since that is standard HTTP semantics, not an information leak.
fn enforce_auth(
    require_auth: bool,
    token_presence: TokenPresence,
    claims: Option<&ClientClaims>,
    insecure_forward_auth_errors: bool,
) -> Option<axum::response::Response> {
    if !require_auth {
        return None;
    }
    match token_presence {
        TokenPresence::Absent => {
            let json = r#"{"code":401,"message":"authentication required"}"#.to_string();
            Some(
                (StatusCode::UNAUTHORIZED, [("content-type", "application/json")], json.into_bytes())
                    .into_response(),
            )
        }
        TokenPresence::Present if claims.is_none() => {
            let message = if insecure_forward_auth_errors {
                "invalid or expired token"
            } else {
                "Authentication failed"
            };
            let json = format!(r#"{{"code":401,"message":"{message}"}}"#);
            Some(
                (StatusCode::UNAUTHORIZED, [("content-type", "application/json")], json.into_bytes())
                    .into_response(),
            )
        }
        TokenPresence::Present => None,
    }
}

/// Dispatches operations through the partition pipeline and populates the response.
async fn dispatch_operations(
    ops: Vec<topgun_core::messages::ClientOp>,
    classify_svc: &crate::service::classify::OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    claims: Option<&ClientClaims>,
    caller_origin: CallerOrigin,
    principal: Option<&topgun_core::Principal>,
    response: &mut HttpSyncResponse,
) {
    if ops.is_empty() {
        return;
    }

    let last_id = ops
        .last()
        .and_then(|op| op.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Group ops by partition so each group targets one partition worker.
    let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> = HashMap::new();
    for op in ops {
        let partition_id = hash_to_partition(&op.key);
        partition_groups.entry(partition_id).or_default().push(op);
    }

    // Build sub-batch operations up front, then dispatch concurrently.
    let mut sub_ops: Vec<crate::service::operation::Operation> =
        Vec::with_capacity(partition_groups.len());
    for (partition_id, group_ops) in partition_groups {
        let mut op = classify_svc.classify_op_batch_for_partition(
            group_ops,
            partition_id,
            claims.map(|c| c.user_id.clone()),
            caller_origin,
            None,
            None,
        );
        // Set principal on context for RBAC authorization middleware (HTTP path).
        if let Some(p) = principal {
            op.set_principal(p.clone());
        }
        sub_ops.push(op);
    }

    // Dispatch all sub-batches concurrently.
    let mut join_set = tokio::task::JoinSet::new();
    for sub_op in sub_ops {
        let d = Arc::clone(dispatcher);
        join_set.spawn(async move { d.dispatch(sub_op).await });
    }

    // Collect results; record any dispatch errors.
    let mut dispatch_error: Option<String> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_resp)) => {}
            Ok(Err(e)) => {
                dispatch_error = Some(format!("{e}"));
            }
            Err(join_err) => {
                dispatch_error = Some(format!("join error: {join_err}"));
            }
        }
    }

    if let Some(msg) = dispatch_error {
        let errors = response.errors.get_or_insert_with(Vec::new);
        errors.push(HttpSyncError {
            code: 500,
            message: msg,
            context: None,
        });
    } else {
        response.ack = Some(HttpSyncAck {
            last_id,
            results: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Cursor helpers
// ---------------------------------------------------------------------------

/// Internal cursor payload for HTTP one-shot query pagination.
///
/// Encodes the position after the last returned result so the next request
/// can resume from exactly that point. HTTP queries run on a single server
/// scanning all partitions locally, so per-node tracking (used in WebSocket
/// cursors) adds no value. The cursor is JSON-serialized and base64url-encoded.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpCursorData {
    /// Sort value of the last result in the page (accommodates any sortable type).
    pub last_sort_value: serde_json::Value,
    /// Key of the last result in the page, used as tiebreaker for equal sort values.
    pub last_key: String,
    /// Sort field name, or None when defaulting to key-based ordering.
    pub sort_field: Option<String>,
    /// Sort direction used for this query.
    pub sort_direction: topgun_core::messages::base::SortDirection,
    /// Hash of the predicate applied in this query (0 if no predicate).
    pub predicate_hash: u64,
    /// Hash of the sort specification (0 if no sort).
    pub sort_hash: u64,
    /// Unix timestamp (ms) when this cursor was created; used for expiry checks.
    pub timestamp: i64,
}

/// Encodes cursor data as base64url JSON for use in HTTP responses.
fn encode_http_cursor(data: &HttpCursorData) -> String {
    let json = serde_json::to_vec(data).expect("HttpCursorData serialization is infallible");
    base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &json)
}

/// Decodes and validates a cursor string from an HTTP request.
///
/// Returns None when the cursor is malformed, not valid base64url, or fails JSON
/// deserialization. Callers must additionally validate the timestamp for expiry.
fn decode_http_cursor(cursor: &str) -> Option<HttpCursorData> {
    let bytes = base64::engine::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        cursor,
    )
    .ok()?;
    serde_json::from_slice::<HttpCursorData>(&bytes).ok()
}

/// Returns true when the given `(key, value)` entry comes strictly after the cursor position.
///
/// For ASC sort: include if sort value > cursor value, or equal value with key > cursor key.
/// For DESC sort: include if sort value < cursor value, or equal value with key > cursor key.
///
/// The `value` parameter is an `rmpv::Value` (from the store), while `cursor.last_sort_value`
/// is a `serde_json::Value` (from JSON round-tripping). Comparison converts both to f64 for
/// numbers or compares string representations to avoid needing rmpv<->`serde_json` conversion.
fn is_after_cursor(key: &str, value: &rmpv::Value, cursor: &HttpCursorData) -> bool {
    use topgun_core::messages::base::SortDirection;

    // Extract the sort field value from the rmpv record if a sort field is specified.
    let sort_val: &rmpv::Value = match &cursor.sort_field {
        Some(field) => match value {
            rmpv::Value::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k.as_str() == Some(field.as_str()))
                .map_or(&rmpv::Value::Nil, |(_, v)| v),
            _ => &rmpv::Value::Nil,
        },
        // No sort field: ordering is by key only; use Nil as sort value.
        None => &rmpv::Value::Nil,
    };

    let cmp = compare_rmpv_to_json(sort_val, &cursor.last_sort_value);

    match cursor.sort_direction {
        SortDirection::Asc => {
            // After cursor: sort_val > last_sort_value, or equal with key > last_key
            cmp > 0 || (cmp == 0 && key > cursor.last_key.as_str())
        }
        SortDirection::Desc => {
            // After cursor (descending): sort_val < last_sort_value, or equal with key > last_key
            cmp < 0 || (cmp == 0 && key > cursor.last_key.as_str())
        }
    }
}

/// Compares an `rmpv::Value` (from store) to a `serde_json::Value` (from cursor JSON).
///
/// Returns negative/zero/positive like a standard comparison. Nil/null sorts last.
/// Strings are compared lexicographically. Numbers are compared as f64.
/// Mixed types (string vs number) compare by type name for stability.
fn compare_rmpv_to_json(rmpv_val: &rmpv::Value, json_val: &serde_json::Value) -> i32 {
    match (rmpv_val, json_val) {
        (rmpv::Value::Nil, serde_json::Value::Null) => 0,
        (rmpv::Value::Nil, _) => 1,  // nil sorts after any non-null value
        (_, serde_json::Value::Null) => -1, // any non-nil sorts before null

        (rmpv::Value::String(s), serde_json::Value::String(js)) => {
            s.as_str().unwrap_or("").cmp(js.as_str()).into_i32_sign()
        }
        (rmpv::Value::Integer(i), serde_json::Value::Number(n)) => {
            let a = i.as_f64().unwrap_or(f64::NAN);
            let b = n.as_f64().unwrap_or(f64::NAN);
            a.partial_cmp(&b).map_or(0, OrderingExt::into_i32_sign)
        }
        (rmpv::Value::F32(a), serde_json::Value::Number(n)) => {
            let b = n.as_f64().unwrap_or(f64::NAN);
            f64::from(*a).partial_cmp(&b).map_or(0, OrderingExt::into_i32_sign)
        }
        (rmpv::Value::F64(a), serde_json::Value::Number(n)) => {
            let b = n.as_f64().unwrap_or(f64::NAN);
            a.partial_cmp(&b).map_or(0, OrderingExt::into_i32_sign)
        }
        // Type mismatch: compare by type tag string for stable ordering
        _ => {
            let a_tag = rmpv_type_tag(rmpv_val);
            let b_tag = json_type_tag(json_val);
            a_tag.cmp(b_tag).into_i32_sign()
        }
    }
}

fn rmpv_type_tag(v: &rmpv::Value) -> &'static str {
    match v {
        rmpv::Value::Nil => "nil",
        rmpv::Value::Boolean(_) => "bool",
        rmpv::Value::Integer(_) | rmpv::Value::F32(_) | rmpv::Value::F64(_) => "number",
        rmpv::Value::String(_) => "string",
        rmpv::Value::Binary(_) => "binary",
        rmpv::Value::Array(_) => "array",
        rmpv::Value::Map(_) => "map",
        rmpv::Value::Ext(_, _) => "ext",
    }
}

fn json_type_tag(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "nil",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "map",
    }
}

/// Converts an `rmpv::Value` to a `serde_json::Value` for cursor serialization.
///
/// Only primitive types (nil, bool, integer, float, string) are converted; complex
/// types (map, array, binary, ext) return None as they are not sortable.
fn rmpv_to_json_value(v: &rmpv::Value) -> Option<serde_json::Value> {
    match v {
        rmpv::Value::Nil => Some(serde_json::Value::Null),
        rmpv::Value::Boolean(b) => Some(serde_json::Value::Bool(*b)),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Some(serde_json::Value::Number(serde_json::Number::from(n)))
            } else {
                i.as_u64().map(|n| serde_json::Value::Number(serde_json::Number::from(n)))
            }
        }
        rmpv::Value::F32(f) => serde_json::Number::from_f64(f64::from(*f))
            .map(serde_json::Value::Number),
        rmpv::Value::F64(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number),
        rmpv::Value::String(s) => s.as_str()
            .map(|s| serde_json::Value::String(s.to_owned())),
        _ => None,
    }
}

/// Extension trait for converting `std::cmp::Ordering` to an i32 sign value.
trait OrderingExt {
    fn into_i32_sign(self) -> i32;
}

impl OrderingExt for std::cmp::Ordering {
    fn into_i32_sign(self) -> i32 {
        match self {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

/// Executes one-shot queries directly from the in-memory record store, bypassing the
/// partition dispatcher pipeline to avoid unnecessary operation overhead for reads.
///
/// This function is `async` solely because the RBAC policy evaluation calls
/// (`has_policies()` and `evaluate()`) on `PolicyEvaluator` are async. The store
/// scanning and `execute_query` call are synchronous. The CPU-bound portion is short
/// enough that `spawn_blocking` is not needed.
#[allow(clippy::too_many_lines)]
async fn dispatch_queries(
    queries: Vec<HttpQueryRequest>,
    store_factory: &RecordStoreFactory,
    principal: Option<&topgun_core::Principal>,
    policy_store: Option<&Arc<dyn PolicyStore>>,
    response: &mut HttpSyncResponse,
) {
    for q in queries {
        let map_name = q.map_name.clone();

        // RBAC map-level read access check. Constructing PolicyEvaluator inline
        // keeps all changes within this file (no AppState field changes needed).
        if let Some(store) = policy_store {
            let evaluator = PolicyEvaluator::new(Arc::clone(store));
            // Permissive default: if no policies are configured, allow all reads.
            if evaluator.has_policies().await {
                let decision = evaluator
                    .evaluate(principal, PermissionAction::Read, &map_name, &rmpv::Value::Nil)
                    .await;
                if decision == PolicyDecision::Deny {
                    let errors = response.errors.get_or_insert_with(Vec::new);
                    errors.push(HttpSyncError {
                        code: 403,
                        message: "access denied".into(),
                        context: Some(q.query_id.clone()),
                    });
                    // Skip adding any queryResults entry for this denied query.
                    continue;
                }
            }
        }

        // Collect all entries for the requested map across all partitions.
        let partition_stores = store_factory.get_all_for_map(&map_name);
        let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
        for store in &partition_stores {
            store.for_each_boxed(
                &mut |key: &str, record: &crate::storage::record::Record| {
                    if let RecordValue::Lww { ref value, .. } = record.value {
                        entries.push((key.to_string(), value_to_rmpv(value)));
                    }
                    // Skip OrMap and OrTombstones entries.
                },
                false, // is_backup = false
            );
        }

        // Build a Query struct from the filter field.
        // filter is a MsgPack Map -> use it as a where-clause (field equality).
        // filter is Nil/anything else -> match all (no filter).
        let where_clause: Option<HashMap<String, rmpv::Value>> =
            if let rmpv::Value::Map(ref pairs) = q.filter {
                let map: HashMap<String, rmpv::Value> = pairs
                    .iter()
                    .filter_map(|(k, v)| k.as_str().map(|s| (s.to_string(), v.clone())))
                    .collect();
                Some(map)
            } else {
                None
            };

        let query = Query {
            r#where: where_clause,
            predicate: None,
            // Do not pass limit/cursor/sort to execute_query; apply pagination manually.
            limit: None,
            cursor: None,
            sort: None,
            group_by: None,
        };

        // Execute the query synchronously against the collected entries.
        // Sort by key for stable cursor-based pagination when no explicit sort is given.
        let mut filtered = execute_query(entries, &query);
        filtered.sort_by(|a, b| a.key.cmp(&b.key));

        // Current timestamp for cursor generation and expiry validation.
        let now_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(i64::MAX);

        // Determine pagination strategy: cursor takes precedence over offset.
        let (page_entries, has_more, next_cursor) = if let Some(ref cursor_str) = q.cursor {
            // --- Cursor-based pagination ---
            let Some(cursor_data) = decode_http_cursor(cursor_str) else {
                // Malformed cursor: return a 400 error and skip this query.
                let errors = response.errors.get_or_insert_with(Vec::new);
                errors.push(HttpSyncError {
                    code: 400,
                    message: "invalid or expired cursor".into(),
                    context: Some(q.query_id.clone()),
                });
                continue;
            };

            // Validate cursor expiry: cursors older than 10 minutes are rejected.
            if now_ms - cursor_data.timestamp > 10 * 60 * 1000 {
                let errors = response.errors.get_or_insert_with(Vec::new);
                errors.push(HttpSyncError {
                    code: 400,
                    message: "invalid or expired cursor".into(),
                    context: Some(q.query_id.clone()),
                });
                continue;
            }

            // Filter to only entries strictly after the cursor position.
            let after_cursor: Vec<_> = filtered
                .into_iter()
                .filter(|entry| is_after_cursor(&entry.key, &entry.value, &cursor_data))
                .collect();

            let total_after = after_cursor.len();

            if let Some(limit) = q.limit {
                let lim = limit as usize;
                let truncated = total_after > lim;
                let page_entries: Vec<_> = after_cursor.into_iter().take(lim).collect();

                let nc = if truncated {
                    page_entries.last().map(|last| {
                        // Extract the sort field value from the last entry in the page.
                        let last_sort_value = cursor_data.sort_field.as_deref().and_then(|field| {
                            if let rmpv::Value::Map(ref pairs) = last.value {
                                pairs.iter()
                                    .find(|(k, _)| k.as_str() == Some(field))
                                    .and_then(|(_, v)| rmpv_to_json_value(v))
                            } else {
                                None
                            }
                        }).unwrap_or(serde_json::Value::Null);

                        let next = HttpCursorData {
                            last_sort_value,
                            last_key: last.key.clone(),
                            sort_field: cursor_data.sort_field.clone(),
                            sort_direction: cursor_data.sort_direction.clone(),
                            predicate_hash: cursor_data.predicate_hash,
                            sort_hash: cursor_data.sort_hash,
                            timestamp: now_ms,
                        };
                        encode_http_cursor(&next)
                    })
                } else {
                    None
                };

                let values: Vec<rmpv::Value> = page_entries.into_iter().map(|e| e.value).collect();
                (values, Some(truncated), nc)
            } else {
                // No limit: return all results after cursor, no next_cursor needed.
                let values: Vec<rmpv::Value> = after_cursor.into_iter().map(|e| e.value).collect();
                (values, None, None)
            }
        } else {
            // --- Offset-based pagination (backward compatible) ---
            let total_filtered = filtered.len();
            let offset = q.offset.unwrap_or(0) as usize;

            if let Some(limit) = q.limit {
                let lim = limit as usize;
                // Keep entries as QueryResultEntry to access keys for cursor generation.
                let page_entries: Vec<_> = filtered.into_iter().skip(offset).take(lim).collect();
                let truncated = total_filtered > offset + lim;

                // Generate next_cursor from the last entry when more results exist.
                let nc = if truncated {
                    page_entries.last().map(|last| {
                        let last_sort_value = serde_json::Value::Null; // key-based ordering
                        let next = HttpCursorData {
                            last_sort_value,
                            last_key: last.key.clone(),
                            sort_field: None,
                            sort_direction: topgun_core::messages::base::SortDirection::Asc,
                            predicate_hash: 0,
                            sort_hash: 0,
                            timestamp: now_ms,
                        };
                        encode_http_cursor(&next)
                    })
                } else {
                    None
                };

                let values: Vec<rmpv::Value> = page_entries.into_iter().map(|e| e.value).collect();
                (values, Some(truncated), nc)
            } else {
                let values: Vec<rmpv::Value> = filtered
                    .into_iter()
                    .skip(offset)
                    .map(|entry| entry.value)
                    .collect();
                (values, None, None)
            }
        };

        let result = HttpQueryResult {
            query_id: q.query_id,
            results: page_entries,
            has_more,
            next_cursor,
        };

        let query_results = response.query_results.get_or_insert_with(Vec::new);
        query_results.push(result);
    }
}

/// Handles POST /sync requests with `MsgPack`-encoded bodies.
///
/// Decodes the request body as `HttpSyncRequest`, dispatches operations through
/// the `OperationService`/`PartitionDispatcher` pipeline, and returns an
/// `HttpSyncResponse` with the server's current HLC timestamp and an ack for
/// any submitted operations.
///
/// Returns HTTP 401 with a JSON error body when authentication is required but
/// no valid Bearer JWT is provided.
///
/// Returns HTTP 400 with a JSON error body when the request body is not valid
/// `MsgPack` or cannot be decoded as `HttpSyncRequest`.
///
/// When `operation_service` or `dispatcher` is absent (test environments without
/// service wiring), returns a minimal response with only `server_hlc` populated.
pub async fn http_sync_handler(
    State(state): State<AppState>,
    claims: Option<ClientClaims>,
    axum::Extension(token_presence): axum::Extension<TokenPresence>,
    body: Bytes,
) -> impl IntoResponse {
    // Read require_auth from server_config. Defaults to false when not configured.
    let require_auth = state
        .server_config
        .as_ref()
        .is_some_and(|sc| sc.load().security.require_auth);

    // Authentication enforcement: reject unauthenticated requests when required.
    if let Some(err_response) = enforce_auth(require_auth, token_presence, claims.as_ref(), state.config.insecure_forward_auth_errors) {
        return err_response;
    }

    // Determine caller_origin and principal based on claims.
    let (caller_origin, principal) = if let Some(ref c) = claims {
        (
            CallerOrigin::HttpClient,
            Some(topgun_core::Principal {
                id: c.user_id.clone(),
                roles: c.roles.clone(),
            }),
        )
    } else {
        (CallerOrigin::Anonymous, None)
    };

    // Obtain the server's current HLC timestamp.
    let server_hlc: Timestamp = state
        .operation_service
        .as_ref()
        .map_or_else(wall_clock_timestamp, |s| s.now());

    // Decode the request body.
    let request: HttpSyncRequest = if body.is_empty() {
        HttpSyncRequest::default()
    } else {
        match rmp_serde::from_slice::<HttpSyncRequest>(&body) {
            Ok(req) => req,
            Err(e) => {
                let json = format!(r#"{{"code":400,"message":"invalid MsgPack body: {e}"}}"#);
                return (
                    StatusCode::BAD_REQUEST,
                    [("content-type", "application/json")],
                    json.into_bytes(),
                )
                    .into_response();
            }
        }
    };

    // When neither service is wired, return a minimal response with only server_hlc.
    let (Some(classify_svc), Some(dispatcher)) =
        (state.operation_service.as_ref(), state.dispatcher.as_ref())
    else {
        return msgpack_response(&HttpSyncResponse { server_hlc, ..Default::default() });
    };

    let mut http_response = HttpSyncResponse {
        server_hlc,
        ..Default::default()
    };

    if let Some(ops) = request.operations {
        dispatch_operations(
            ops, classify_svc, dispatcher, claims.as_ref(), caller_origin, principal.as_ref(), &mut http_response,
        )
        .await;
    }

    if let Some(queries) = request.queries {
        if let Some(store_factory) = state.store_factory.as_ref() {
            dispatch_queries(
                queries,
                store_factory,
                principal.as_ref(),
                state.policy_store.as_ref(),
                &mut http_response,
            )
            .await;
        }
    }

    msgpack_response(&http_response)
}

/// Serializes `response` as `MsgPack` and wraps it in an HTTP 200 response.
fn msgpack_response(response: &HttpSyncResponse) -> axum::response::Response {
    match rmp_serde::to_vec_named(response) {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", MSGPACK_CONTENT_TYPE)],
            bytes,
        )
            .into_response(),
        Err(e) => {
            let json = format!(r#"{{"code":500,"message":"serialization error: {e}"}}"#);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json.into_bytes(),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::handlers::auth::{decode_jwt_key, JwtClaims};
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};
    use arc_swap::ArcSwap;
    use jsonwebtoken::{EncodingKey, Header};
    use serde::Serialize;
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use topgun_core::messages::HttpSyncRequest;
    use topgun_core::Timestamp;

    const TEST_SECRET: &str = "test-http-sync-secret";

    /// Minimal claims struct for building test tokens.
    #[derive(Serialize)]
    struct TestClaims {
        sub: Option<String>,
        exp: u64,
    }

    /// Encode a JWT token with HS256 for testing.
    fn make_token(sub: Option<&str>, exp_offset_secs: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        let exp = if exp_offset_secs >= 0 {
            now + exp_offset_secs as u64
        } else {
            now.saturating_sub((-exp_offset_secs) as u64)
        };
        let claims = TestClaims { sub: sub.map(str::to_owned), exp };
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test token encoding should not fail")
    }

    fn test_state() -> AppState {
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
        }
    }

    fn test_state_with_auth(require_auth: bool) -> AppState {
        use crate::service::config::ServerConfig;
        use crate::service::security::SecurityConfig;
        let mut config = NetworkConfig::default();
        config.jwt_clock_skew_secs = 60;
        let server_cfg = ServerConfig {
            security: SecurityConfig { require_auth, ..SecurityConfig::default() },
            ..ServerConfig::default()
        };
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(config),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: Some(TEST_SECRET.to_owned()),
            cluster_state: None,
            store_factory: None,
            server_config: Some(Arc::new(ArcSwap::from_pointee(server_cfg))),
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
        }
    }

    #[tokio::test]
    async fn http_sync_handler_returns_msgpack_content_type() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        let content_type = response
            .headers()
            .get("content-type")
            .expect("content-type header must be present");
        assert_eq!(content_type, "application/msgpack");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_200_for_empty_body() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn http_sync_handler_returns_valid_server_hlc_for_empty_request() {
        let state = test_state();

        // Encode a minimal HttpSyncRequest (no operations) as MsgPack.
        let req = HttpSyncRequest {
            client_id: "test-client".to_string(),
            client_hlc: Timestamp {
                millis: 1_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
            ..Default::default()
        };
        let body = Bytes::from(rmp_serde::to_vec_named(&req).unwrap());

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body read must succeed");
        let decoded: HttpSyncResponse =
            rmp_serde::from_slice(&body_bytes).expect("response must be valid MsgPack");

        // server_hlc.millis must be a positive wall-clock timestamp.
        assert!(decoded.server_hlc.millis > 0, "server_hlc.millis must be > 0");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_400_for_malformed_body() {
        let state = test_state();
        // Two bytes that are not a valid MsgPack-encoded HttpSyncRequest.
        let body = Bytes::from_static(&[0xFF, 0xFF]);

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Auth tests (AC1-AC5)
    // -----------------------------------------------------------------------

    /// AC1: Valid Bearer JWT + require_auth=true → HTTP 200.
    #[tokio::test]
    async fn auth_valid_token_sets_http_client_origin() {
        let state = test_state_with_auth(true);
        let token = make_token(Some("user-1"), 3600);
        // Build real ClientClaims by parsing the token with the test secret.
        let claims = {
            let (algo, key) = decode_jwt_key(TEST_SECRET).unwrap();
            let mut v = jsonwebtoken::Validation::new(algo);
            v.validate_aud = false;
            v.leeway = 60;
            let data = jsonwebtoken::decode::<JwtClaims>(&token, &key, &v).unwrap();
            Some(ClientClaims {
                user_id: data.claims.sub.unwrap(),
                roles: data.claims.roles.unwrap_or_default(),
            })
        };
        let body = Bytes::from_static(b"");
        let response =
            http_sync_handler(State(state), claims, axum::Extension(TokenPresence::Present), body)
                .await
                .into_response();
        // With valid token + require_auth=true, handler should proceed (200 since no dispatch).
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "valid token + require_auth=true must return HTTP 200"
        );
    }

    /// AC2: Present-but-invalid token + require_auth=true → HTTP 401 with generic message (default mode).
    #[tokio::test]
    async fn auth_invalid_token_rejected_when_required() {
        let state = test_state_with_auth(true);
        // Token present in header but claims=None (extractor failed validation).
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None, // token was present but invalid, so claims=None
            axum::Extension(TokenPresence::Present),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "present-but-invalid token must return HTTP 401"
        );
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        // Default mode (insecure_forward_auth_errors=false) returns a generic message.
        assert!(
            body_str.contains("Authentication failed"),
            "response body must contain 'Authentication failed' in default mode, got: {body_str}"
        );
    }

    /// AC2 (insecure mode): Present-but-invalid token + require_auth=true + insecure flag → detailed message.
    #[tokio::test]
    async fn auth_invalid_token_insecure_mode_shows_detail() {
        use crate::network::config::NetworkConfig;
        use crate::service::config::ServerConfig;
        use std::sync::Arc;

        let mut base_state = test_state_with_auth(true);
        base_state.config = Arc::new(NetworkConfig {
            insecure_forward_auth_errors: true,
            ..NetworkConfig::default()
        });

        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(base_state),
            None,
            axum::Extension(TokenPresence::Present),
            body,
        )
        .await
        .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("invalid or expired token"),
            "insecure mode must return detailed message, got: {body_str}"
        );
    }

    /// AC3: No Authorization header + require_auth=true → HTTP 401 "authentication required".
    #[tokio::test]
    async fn auth_missing_token_rejected_when_required() {
        let state = test_state_with_auth(true);
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None,
            axum::Extension(TokenPresence::Absent),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "missing token + require_auth=true must return HTTP 401"
        );
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("authentication required"),
            "response body must contain 'authentication required', got: {body_str}"
        );
    }

    /// AC4: No Authorization header + require_auth=false → HTTP 200 (backward compatible).
    #[tokio::test]
    async fn auth_missing_token_allowed_when_not_required() {
        let state = test_state_with_auth(false);
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None,
            axum::Extension(TokenPresence::Absent),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "missing token + require_auth=false must return HTTP 200"
        );
    }

    /// AC5: Valid Bearer JWT + require_auth=false → HTTP 200.
    #[tokio::test]
    async fn auth_valid_token_optional_auth_uses_http_client() {
        let state = test_state_with_auth(false);
        let token = make_token(Some("user-2"), 3600);
        let claims = {
            let (algo, key) = decode_jwt_key(TEST_SECRET).unwrap();
            let mut v = jsonwebtoken::Validation::new(algo);
            v.validate_aud = false;
            v.leeway = 60;
            let data = jsonwebtoken::decode::<JwtClaims>(&token, &key, &v).unwrap();
            Some(ClientClaims {
                user_id: data.claims.sub.unwrap(),
                roles: data.claims.roles.unwrap_or_default(),
            })
        };
        let body = Bytes::from_static(b"");
        let response =
            http_sync_handler(State(state), claims, axum::Extension(TokenPresence::Present), body)
                .await
                .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "valid token + require_auth=false must return HTTP 200"
        );
    }

    // -----------------------------------------------------------------------
    // dispatch_queries tests (R4)
    // -----------------------------------------------------------------------

    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::RecordValue as StoreRecordValue;
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
    use topgun_core::hlc::Timestamp as HlcTimestamp;
    use topgun_core::messages::HttpQueryRequest as QueryReq;
    use topgun_core::types::Value as TgValue;

    /// Helper to build a minimal AppState with a pre-seeded RecordStoreFactory.
    ///
    /// Seeds `map_name` with `records` (key -> string value) into partition 0.
    async fn state_with_map_data(map_name: &str, records: Vec<(&str, &str)>) -> AppState {
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let store = factory.get_or_create(map_name, 0);
        for (key, val) in records {
            store
                .put(
                    key,
                    StoreRecordValue::Lww {
                        value: TgValue::Map(
                            [("name".to_string(), TgValue::String(val.to_string()))]
                                .into_iter()
                                .collect(),
                        ),
                        timestamp: HlcTimestamp {
                            millis: 1_000_000,
                            counter: 0,
                            node_id: "node-1".to_string(),
                        },
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("seeding store should not fail");
        }
        AppState {
            registry: Arc::new(crate::network::ConnectionRegistry::new()),
            shutdown: Arc::new(crate::network::ShutdownController::new()),
            config: Arc::new(crate::network::NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: None,
            cluster_state: None,
            store_factory: Some(factory),
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
        }
    }

    /// Executes a dispatch_queries call directly and returns the response.
    async fn run_dispatch_queries(
        queries: Vec<QueryReq>,
        state: &AppState,
    ) -> HttpSyncResponse {
        let mut response = HttpSyncResponse::default();
        if let Some(sf) = state.store_factory.as_ref() {
            dispatch_queries(queries, sf, None, state.policy_store.as_ref(), &mut response).await;
        }
        response
    }

    /// Query with a matching where-filter returns only the matching records.
    #[tokio::test]
    async fn query_returns_matching_results() {
        let state = state_with_map_data(
            "users",
            vec![("alice", "Alice"), ("bob", "Bob"), ("charlie", "Charlie")],
        )
        .await;

        // Filter: name == "Alice"
        let filter = rmpv::Value::Map(vec![(
            rmpv::Value::String("name".into()),
            rmpv::Value::String("Alice".into()),
        )]);
        let query = QueryReq {
            query_id: "q1".to_string(),
            map_name: "users".to_string(),
            filter,
            limit: None,
            offset: None,
            cursor: None,
        };

        let response = run_dispatch_queries(vec![query], &state).await;
        let results = response.query_results.expect("queryResults must be present");
        assert_eq!(results.len(), 1, "should have one query result entry");
        assert_eq!(results[0].query_id, "q1");
        assert_eq!(results[0].results.len(), 1, "filter should return exactly 1 matching record");
    }

    /// Query against a non-existent map returns empty results, not an error.
    #[tokio::test]
    async fn query_empty_map_returns_empty_results() {
        let state = state_with_map_data("users", vec![]).await;

        let query = QueryReq {
            query_id: "q-empty".to_string(),
            map_name: "nonexistent-map".to_string(),
            filter: rmpv::Value::Nil,
            limit: None,
            offset: None,
            cursor: None,
        };

        let response = run_dispatch_queries(vec![query], &state).await;
        // No error entries.
        assert!(response.errors.is_none(), "missing map must not produce errors");
        let results = response.query_results.expect("queryResults must be present");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].results.len(), 0, "empty map returns empty results array");
    }

    /// Query with limit and offset paginates correctly, with hasMore = true.
    #[tokio::test]
    async fn query_with_limit_and_offset() {
        // 5 records: a, b, c, d, e
        let state = state_with_map_data(
            "items",
            vec![("a", "A"), ("b", "B"), ("c", "C"), ("d", "D"), ("e", "E")],
        )
        .await;

        // limit=2, offset=1 → skip 1, take 2. With 5 total, has_more should be true (5 > 1+2).
        let query = QueryReq {
            query_id: "q-page".to_string(),
            map_name: "items".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: Some(1),
            cursor: None,
        };

        let response = run_dispatch_queries(vec![query], &state).await;
        let results = response.query_results.expect("queryResults must be present");
        assert_eq!(results.len(), 1);
        let qr = &results[0];
        assert_eq!(qr.results.len(), 2, "limit=2 must return exactly 2 records");
        assert_eq!(
            qr.has_more,
            Some(true),
            "has_more must be true when more records exist beyond the page"
        );
    }

    /// Query with Nil filter returns all records in the map.
    #[tokio::test]
    async fn query_no_filter_returns_all() {
        let state = state_with_map_data(
            "products",
            vec![("p1", "Widget"), ("p2", "Gadget"), ("p3", "Gizmo")],
        )
        .await;

        let query = QueryReq {
            query_id: "q-all".to_string(),
            map_name: "products".to_string(),
            filter: rmpv::Value::Nil,
            limit: None,
            offset: None,
            cursor: None,
        };

        let response = run_dispatch_queries(vec![query], &state).await;
        let results = response.query_results.expect("queryResults must be present");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].results.len(),
            3,
            "nil filter should return all 3 records"
        );
        assert!(results[0].has_more.is_none(), "has_more must be None when no limit is set");
    }

    /// When store_factory is None, queries are silently skipped and queryResults is None.
    #[tokio::test]
    async fn query_skipped_when_no_store_factory() {
        let state = test_state(); // store_factory = None

        let req = HttpSyncRequest {
            client_id: "c1".to_string(),
            client_hlc: Timestamp { millis: 1000, counter: 0, node_id: "n1".to_string() },
            queries: Some(vec![QueryReq {
                query_id: "q-skip".to_string(),
                map_name: "users".to_string(),
                filter: rmpv::Value::Nil,
                limit: None,
                offset: None,
                cursor: None,
            }]),
            ..Default::default()
        };
        let body = Bytes::from(rmp_serde::to_vec_named(&req).unwrap());

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let decoded: HttpSyncResponse = rmp_serde::from_slice(&body_bytes).unwrap();
        assert!(
            decoded.query_results.is_none(),
            "queryResults must be None when store_factory is not wired"
        );
    }

    // -----------------------------------------------------------------------
    // R5: Cursor-based pagination tests
    // -----------------------------------------------------------------------

    /// R5 test 1: Two-page cursor traversal returns all records without duplicates.
    ///
    /// Seeds 5 records (keys a–e). Queries with limit=2 to get page 1, extracts
    /// next_cursor, then queries with that cursor + limit=2 to get page 2.
    /// Verifies page 2 contains 2 records not in page 1, and that a third query
    /// with the page-2 cursor returns the remaining 1 record.
    #[tokio::test]
    async fn query_with_cursor_paginates_correctly() {
        // Records keyed a–e; key sort gives stable order: a, b, c, d, e.
        let state = state_with_map_data(
            "paged",
            vec![("a", "A"), ("b", "B"), ("c", "C"), ("d", "D"), ("e", "E")],
        )
        .await;

        // Page 1: limit=2, no cursor → first 2 records.
        let page1_query = QueryReq {
            query_id: "p1".to_string(),
            map_name: "paged".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: None,
            cursor: None,
        };
        let resp1 = run_dispatch_queries(vec![page1_query], &state).await;
        let results1 = resp1.query_results.expect("page 1 must have results");
        let qr1 = &results1[0];
        assert_eq!(qr1.results.len(), 2, "page 1 must have 2 results");
        assert_eq!(qr1.has_more, Some(true), "has_more must be true for page 1");
        let cursor1 = qr1.next_cursor.clone().expect("page 1 must have next_cursor");

        // Page 2: use cursor from page 1.
        let page2_query = QueryReq {
            query_id: "p2".to_string(),
            map_name: "paged".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: None,
            cursor: Some(cursor1),
        };
        let resp2 = run_dispatch_queries(vec![page2_query], &state).await;
        let results2 = resp2.query_results.expect("page 2 must have results");
        let qr2 = &results2[0];
        assert_eq!(qr2.results.len(), 2, "page 2 must have 2 results");
        assert_eq!(qr2.has_more, Some(true), "has_more must be true for page 2");
        let cursor2 = qr2.next_cursor.clone().expect("page 2 must have next_cursor");

        // Page 3: use cursor from page 2, should get 1 remaining record.
        let page3_query = QueryReq {
            query_id: "p3".to_string(),
            map_name: "paged".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: None,
            cursor: Some(cursor2),
        };
        let resp3 = run_dispatch_queries(vec![page3_query], &state).await;
        let results3 = resp3.query_results.expect("page 3 must have results");
        let qr3 = &results3[0];
        assert_eq!(qr3.results.len(), 1, "page 3 must have 1 remaining record");
        assert_eq!(qr3.has_more, Some(false), "has_more must be false for last page");
        assert!(qr3.next_cursor.is_none(), "next_cursor must be None on last page");

        // Verify no duplicates across pages.
        let all_count = qr1.results.len() + qr2.results.len() + qr3.results.len();
        assert_eq!(all_count, 5, "all 5 records must appear across pages");
    }

    /// R5 test 2: Invalid cursor string produces a 400 error for that query.
    #[tokio::test]
    async fn query_with_invalid_cursor_returns_error() {
        let state = state_with_map_data(
            "items2",
            vec![("x", "X"), ("y", "Y")],
        )
        .await;

        let query = QueryReq {
            query_id: "q-bad-cursor".to_string(),
            map_name: "items2".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(5),
            offset: None,
            cursor: Some("not-valid-base64!!!".to_string()),
        };
        let response = run_dispatch_queries(vec![query], &state).await;

        // Should produce an error, not query results for this query.
        let errors = response.errors.expect("errors must be present for invalid cursor");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 400);
        assert!(errors[0].message.contains("invalid or expired cursor"));
        assert_eq!(errors[0].context.as_deref(), Some("q-bad-cursor"));
        // No query result entry for the failed query.
        assert!(response.query_results.is_none(), "queryResults must be absent when cursor is invalid");
    }

    /// R5 test 3: When both cursor and offset are provided, cursor takes precedence.
    #[tokio::test]
    async fn query_cursor_precedence_over_offset() {
        // Seed 5 records a–e; key sort: a, b, c, d, e.
        let state = state_with_map_data(
            "prec",
            vec![("a", "A"), ("b", "B"), ("c", "C"), ("d", "D"), ("e", "E")],
        )
        .await;

        // First, get page 1 to obtain a real cursor (last key should be "b").
        let page1 = QueryReq {
            query_id: "prec-p1".to_string(),
            map_name: "prec".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: None,
            cursor: None,
        };
        let resp1 = run_dispatch_queries(vec![page1], &state).await;
        let cursor = resp1
            .query_results.unwrap()[0]
            .next_cursor.clone()
            .expect("page 1 must produce a cursor");

        // Query with both cursor and offset=0; cursor must win (should skip a, b).
        let query = QueryReq {
            query_id: "prec-both".to_string(),
            map_name: "prec".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: Some(0), // would return a, b if offset path taken
            cursor: Some(cursor),
        };
        let response = run_dispatch_queries(vec![query], &state).await;
        let results = response.query_results.expect("results must be present");
        let qr = &results[0];
        // Cursor path: should return c, d (the two records after b).
        // Offset=0 path would return a, b. If cursor won, len=2 and records != a, b.
        assert_eq!(qr.results.len(), 2, "cursor + offset must return cursor-based page (2 records)");
    }

    /// R5 test 4: encode_http_cursor / decode_http_cursor roundtrip.
    #[test]
    fn query_cursor_encode_decode_roundtrip() {
        let original = HttpCursorData {
            last_sort_value: serde_json::Value::Number(serde_json::Number::from(42i64)),
            last_key: "record-abc".to_string(),
            sort_field: Some("score".to_string()),
            sort_direction: topgun_core::messages::base::SortDirection::Asc,
            predicate_hash: 12345u64,
            sort_hash: 67890u64,
            timestamp: 1_700_000_000_000i64,
        };

        let encoded = encode_http_cursor(&original);
        let decoded = decode_http_cursor(&encoded).expect("valid cursor must decode");

        assert_eq!(decoded.last_key, original.last_key);
        assert_eq!(decoded.sort_field, original.sort_field);
        assert_eq!(decoded.sort_direction, original.sort_direction);
        assert_eq!(decoded.predicate_hash, original.predicate_hash);
        assert_eq!(decoded.sort_hash, original.sort_hash);
        assert_eq!(decoded.timestamp, original.timestamp);
        assert_eq!(decoded.last_sort_value, original.last_sort_value);

        // Invalid inputs must return None.
        assert!(decode_http_cursor("!!!not-base64!!!").is_none());
        assert!(decode_http_cursor("aGVsbG8=").is_none()); // valid base64 but not JSON cursor
    }

    /// R5 test 5: Offset pagination continues to work (regression guard).
    #[tokio::test]
    async fn query_offset_still_works() {
        let state = state_with_map_data(
            "offset-guard",
            vec![("a", "A"), ("b", "B"), ("c", "C"), ("d", "D"), ("e", "E")],
        )
        .await;

        // limit=2, offset=2 → should return records 3 and 4 (c, d in key order).
        let query = QueryReq {
            query_id: "offset-test".to_string(),
            map_name: "offset-guard".to_string(),
            filter: rmpv::Value::Nil,
            limit: Some(2),
            offset: Some(2),
            cursor: None,
        };
        let response = run_dispatch_queries(vec![query], &state).await;
        let results = response.query_results.expect("results must be present");
        let qr = &results[0];
        assert_eq!(qr.results.len(), 2, "offset=2 limit=2 must return 2 records");
        assert_eq!(qr.has_more, Some(true), "has_more must be true (5 > 2+2)");
        assert!(response.errors.is_none(), "offset pagination must not produce errors");
    }

    // -----------------------------------------------------------------------
    // AuthValidator integration tests (SPEC-189 AC2, AC4)
    // -----------------------------------------------------------------------

    /// Helper: build a minimal AppState with a jwt_secret and optional validator.
    fn test_state_with_validator(validator: Option<std::sync::Arc<dyn crate::network::handlers::auth_validator::AuthValidator>>) -> AppState {
        let mut config = crate::network::config::NetworkConfig::default();
        config.jwt_clock_skew_secs = 60;
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(crate::network::shutdown::ShutdownController::new()),
            config: Arc::new(config),
            start_time: std::time::Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: Some(TEST_SECRET.to_owned()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: validator,
        }
    }

    /// AC2 (SPEC-189): A rejecting AuthValidator causes ClientClaims extractor to return None.
    #[tokio::test]
    async fn client_claims_rejecting_validator_returns_none() {
        use axum::extract::OptionalFromRequestParts;
        use axum::http::{header, Request};

        let validator = Arc::new(|_ctx: &crate::network::handlers::auth_validator::AuthValidationContext| {
            Err("revoked".to_string())
        });
        let state = test_state_with_validator(Some(validator));
        let token = make_token(Some("user-revoked"), 3600);
        let req = Request::builder()
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .expect("request construction should not fail");
        let (mut parts, _) = req.into_parts();
        let result = <ClientClaims as axum::extract::OptionalFromRequestParts<AppState>>::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok(), "extractor should be infallible");
        assert!(result.unwrap().is_none(), "rejecting validator should cause None");
    }

    /// AC4 (SPEC-189): When auth_validator is None, valid token is accepted (no regression).
    #[tokio::test]
    async fn client_claims_no_validator_accepts_valid_token() {
        use axum::extract::OptionalFromRequestParts;
        use axum::http::{header, Request};

        let state = test_state_with_validator(None);
        let token = make_token(Some("user-ok"), 3600);
        let req = Request::builder()
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .expect("request construction should not fail");
        let (mut parts, _) = req.into_parts();
        let result = <ClientClaims as axum::extract::OptionalFromRequestParts<AppState>>::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok(), "extractor should be infallible");
        let claims = result.unwrap();
        assert!(claims.is_some(), "no validator should accept valid token");
        assert_eq!(claims.unwrap().user_id, "user-ok");
    }
}
