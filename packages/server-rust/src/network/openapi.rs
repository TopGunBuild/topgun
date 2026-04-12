//! `OpenAPI` specification generation and Swagger UI for the admin API.
//!
//! Uses `utoipa` to derive the `OpenAPI` 3.1 spec from Rust types and handler
//! annotations. Provides two endpoints:
//! - `GET /api/openapi.json` -- raw `OpenAPI` JSON spec
//! - `GET /api/docs` -- Swagger UI rendering the spec

// The `utoipa::OpenApi` derive macro generates code that triggers this lint.
#![allow(clippy::needless_for_each)]

use axum::response::IntoResponse;
use axum::Json;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use super::handlers::admin;
use super::handlers::admin_types::{
    ClusterStatusResponse, CreatePolicyRequest, ErrorResponse, LoginRequest, LoginResponse,
    MapInfo, MapsListResponse, NodeInfo, NodeStatus, PartitionInfo, PolicyListResponse,
    PolicyResponse, ServerMode, ServerStatusResponse, SettingsResponse, SettingsUpdateRequest,
};
use crate::service::policy::{PermissionAction, PermissionPolicy, PolicyEffect};

/// Adds the `bearer_auth` HTTP security scheme to the `OpenAPI` spec.
struct BearerAuthAddon;

impl Modify for BearerAuthAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

/// Aggregated `OpenAPI` spec for all admin endpoints.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "TopGun Admin API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Administration API for TopGun real-time data grid"
    ),
    paths(
        openapi_json,
        admin::server_status,
        admin::login,
        admin::cluster_status,
        admin::list_maps,
        admin::get_settings,
        admin::update_settings,
        admin::list_policies,
        admin::create_policy,
        admin::delete_policy,
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
        PermissionAction,
        PolicyEffect,
        PermissionPolicy,
        CreatePolicyRequest,
        PolicyListResponse,
        PolicyResponse,
    )),
    modifiers(&BearerAuthAddon)
)]
pub struct AdminApiDoc;

/// Returns the `OpenAPI` JSON specification.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json")
    ),
    tag = "OpenAPI"
)]
#[must_use]
pub fn openapi_json() -> impl IntoResponse {
    Json(AdminApiDoc::openapi())
}
