//! OpenAPI specification generation and Swagger UI for the admin API.
//!
//! Uses `utoipa` to derive the OpenAPI 3.1 spec from Rust types and handler
//! annotations. Provides two endpoints:
//! - `GET /api/openapi.json` -- raw OpenAPI JSON spec
//! - `GET /api/docs` -- Swagger UI rendering the spec

use axum::response::IntoResponse;
use axum::Json;
use utoipa::OpenApi;

use super::handlers::admin_types::{
    ClusterStatusResponse, ErrorResponse, LoginRequest, LoginResponse, MapInfo, MapsListResponse,
    NodeInfo, NodeStatus, PartitionInfo, ServerMode, ServerStatusResponse, SettingsResponse,
    SettingsUpdateRequest,
};

/// Aggregated OpenAPI spec for all admin endpoints.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "TopGun Admin API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Administration API for TopGun real-time data grid"
    ),
    paths(
        openapi_json,
    ),
    components(schemas(
        ServerStatusResponse,
        ServerMode,
        ClusterStatusResponse,
        NodeInfo,
        NodeStatus,
        PartitionInfo,
        MapInfo,
        MapsListResponse,
        SettingsResponse,
        SettingsUpdateRequest,
        LoginRequest,
        LoginResponse,
        ErrorResponse,
    ))
)]
pub struct AdminApiDoc;

/// Returns the OpenAPI JSON specification.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json")
    ),
    tag = "OpenAPI"
)]
pub async fn openapi_json() -> impl IntoResponse {
    Json(AdminApiDoc::openapi())
}
