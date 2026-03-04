---
id: SPEC-076b
type: feature
status: draft
priority: P1
complexity: medium
created: 2026-03-04
todo: TODO-093
parent: SPEC-076
depends_on: [SPEC-076a]
---

# Admin Dashboard v1.0 -- Rust Admin Handlers & Wiring

## Context

TopGun is building an admin dashboard backed by a Rust axum server. SPEC-076a created the foundational types (`admin_types.rs`), auth middleware (`admin_auth.rs`), made `JwtClaims` public, and added dependencies. This sub-spec implements the actual admin API endpoint handlers, OpenAPI spec generation, and wires everything into the axum router.

The Rust server (`packages/server-rust/`) currently has axum HTTP routes for health, WebSocket, sync, and metrics -- but zero admin API endpoints. After SPEC-076a, the types and auth extractor exist but are not yet used by any handler or route.

**Parent spec:** SPEC-076 (Admin Dashboard v1.0 -- Rust Server Adaptation)
**Depends on:** SPEC-076a (types + auth middleware)
**Sibling spec:** SPEC-076c (React dashboard)

### Inherited Decisions

- Admin credentials via `TOPGUN_ADMIN_USERNAME` (default: `"admin"`) and `TOPGUN_ADMIN_PASSWORD` (required) env vars
- Constant-time comparison via `subtle::ConstantTimeEq` for credential validation in login handler
- Same-port admin API (no separate admin port)
- `ArcSwap<ServerConfig>` is a NEW wrapper introduced specifically for admin hot-reload. Existing services (`ServiceRegistry`, `OperationService`, `OperationPipeline`) continue using their current `Arc<ServerConfig>` references unchanged. Admin handlers read from `ArcSwap` for settings GET/PUT; existing services are not refactored
- Bootstrap mode deferred to v1.1; `GET /api/status` always returns `mode: Normal`
- System maps (`$sys/*`) NOT populated in this spec; admin REST API endpoints provide data directly
- OpenAPI spec generated from Rust code via `utoipa` (not hand-written YAML)
- Static SPA serving via `tower-http::services::ServeDir` from a configurable directory (default: `./admin-dashboard/dist/`) with SPA fallback to `index.html`

### Available from SPEC-076a

- `admin_types.rs`: `ServerStatusResponse`, `ClusterStatusResponse`, `NodeInfo`, `NodeStatus`, `PartitionInfo`, `MapInfo`, `MapsListResponse`, `SettingsResponse`, `SettingsUpdateRequest`, `LoginRequest`, `LoginResponse`, `ErrorResponse`, `ServerMode`
- `admin_auth.rs`: `AdminClaims` extractor (validates JWT + admin role)
- `auth.rs`: `pub JwtClaims` with `pub roles: Option<Vec<String>>`
- `Cargo.toml`: `utoipa`, `utoipa-axum`, `utoipa-swagger-ui`, `subtle`, `tower-http` with `"fs"` feature

## Task

Implement the admin API endpoint handlers, create OpenAPI spec generation with Swagger UI, wire all admin routes into the axum router, update `AppState` with required shared data, and add static SPA serving for the admin dashboard.

## Requirements

### New files

1. `packages/server-rust/src/network/handlers/admin.rs` -- Admin API endpoint handlers:
   - `GET /api/status` -- Server status (version, mode). Always returns `mode: Normal` for v1.0. Does NOT require admin auth (public endpoint)
   - `GET /api/admin/cluster/status` -- Cluster topology: nodes, partitions, rebalancing state. Requires `AdminClaims`
   - `GET /api/admin/maps` -- List all maps with entry counts. Requires `AdminClaims`. Lists all non-system maps from `RecordStoreFactory`
   - `GET /api/admin/settings` -- Current server configuration (SettingsResponse). Requires `AdminClaims`. Reads from `ArcSwap<ServerConfig>`, `NetworkConfig`, `SecurityConfig`
   - `PUT /api/admin/settings` -- Update hot-reloadable settings. Requires `AdminClaims`. Accepts `SettingsUpdateRequest` with optional fields `logLevel`, `gcIntervalMs`, `maxConcurrentOperations`. Returns 400 if request includes read-only fields
   - `POST /api/auth/login` -- Admin login. Does NOT require admin auth. Validates `{ username, password }` against `TOPGUN_ADMIN_USERNAME` / `TOPGUN_ADMIN_PASSWORD` env vars using constant-time comparison (`subtle::ConstantTimeEq`); returns `{ token: "..." }` with JWT containing `roles: ["admin"]`; returns 401 on invalid credentials

   **Hot-reloadable settings** (can be changed via `PUT /api/admin/settings` without restart):
   - `log_level` -- updates the `tracing` EnvFilter at runtime
   - `gc_interval_ms` -- takes effect on the next GC cycle
   - `max_concurrent_operations` -- updated atomically via `ArcSwap<ServerConfig>`

   **Read-only settings** (returned by `GET` but rejected if included in `PUT`):
   - `node_id`, `host`, `port`, `partition_count`, `require_auth`, `max_value_bytes`, `default_operation_timeout_ms`

2. `packages/server-rust/src/network/openapi.rs` -- OpenAPI spec generation:
   - `utoipa::OpenApi` derive macro aggregating all admin endpoint schemas
   - `GET /api/openapi.json` endpoint serving the generated spec
   - `GET /api/docs` serves Swagger UI via `utoipa-swagger-ui`
   - All admin types annotated with `ToSchema` (from SPEC-076a)

### Modified files

3. `packages/server-rust/src/network/handlers/mod.rs` -- Add `pub mod admin;`, `pub mod admin_types;`, `pub mod admin_auth;` declarations and re-exports; add `cluster_state: Option<Arc<ClusterState>>`, `store_factory: Option<Arc<RecordStoreFactory>>`, `server_config: Option<Arc<ArcSwap<ServerConfig>>>` fields to `AppState`. **Note:** The `ArcSwap<ServerConfig>` here is a NEW wrapper introduced specifically for admin hot-reload. Existing services continue using their current `Arc<ServerConfig>` references unchanged

4. `packages/server-rust/src/network/module.rs` -- Add admin routes to `build_app()` router:
   - `/api/status` (no auth)
   - `/api/auth/login` (no auth)
   - `/api/admin/*` routes (admin auth required)
   - `/api/openapi.json` and `/api/docs` routes
   - `tower_http::services::ServeDir` for `/admin/` static SPA serving with fallback to `index.html` for SPA client-side routing

## Acceptance Criteria

1. `GET /api/status` returns `{ configured: true, version: "...", mode: "normal" }` as JSON with correct Content-Type. `mode` is serialized from the `ServerMode` enum
2. `GET /api/admin/cluster/status` returns node list where each `NodeInfo` has `nodeId: String`, `address: String`, `status: NodeStatus` (serialized as `"healthy"`, `"suspect"`, or `"dead"`), `partitionCount: u32`, `connections: u32`, `memory: u64` (bytes), `uptime: u64` (seconds)
3. `GET /api/admin/maps` returns `{ maps: [{ name: "...", entryCount: N }] }` where `entryCount` is `u64`, listing all non-system maps from `RecordStoreFactory`
4. `GET /api/admin/settings` returns `SettingsResponse` with fields: `nodeId`, `defaultOperationTimeoutMs`, `maxConcurrentOperations`, `gcIntervalMs`, `partitionCount` (from `ServerConfig`); `host`, `port` (from `NetworkConfig`); `requireAuth`, `maxValueBytes` (from `SecurityConfig`); `logLevel` (optional, from runtime)
5. `PUT /api/admin/settings` accepts `SettingsUpdateRequest` with optional fields `logLevel`, `gcIntervalMs`, `maxConcurrentOperations` and applies them without restart. Returns 400 if request includes read-only fields
6. `POST /api/auth/login` validates `{ username, password }` against `TOPGUN_ADMIN_USERNAME` / `TOPGUN_ADMIN_PASSWORD` env vars using constant-time comparison (`subtle::ConstantTimeEq`); returns `{ token: "..." }` with JWT containing `roles: ["admin"]`; returns 401 on invalid credentials
7. Admin endpoints (except `/api/status` and `/api/auth/login`) return 401 without valid JWT Bearer token
8. Admin endpoints return 403 when JWT lacks `"admin"` in `roles` array
9. `GET /api/openapi.json` returns valid OpenAPI 3.0+ spec containing all admin endpoint definitions
10. `GET /api/docs` serves Swagger UI rendering the OpenAPI spec
11. `GET /admin/` serves the Vite-built SPA via `tower-http::services::ServeDir` with fallback to `index.html`; returns 404 if the configured static directory does not exist
12. Existing endpoints (`/health`, `/ws`, `/sync`, `/metrics`) continue to function without regression

## Constraints

- Do NOT modify admin types or auth middleware (those are in SPEC-076a)
- Do NOT modify React dashboard files (those are in SPEC-076c)
- Do NOT modify existing WebSocket protocol or message schema
- Do NOT add v2.0 features (pipeline visualization, connector wizard, DAG editor)
- Do NOT add v3.0 features (tenant admin, S3 storage config, vector search config beyond toggle)
- Admin endpoints run on the SAME port as the main server (no separate admin port)
- OpenAPI spec must be generated from Rust code (not hand-written YAML)
- 4 Rust files maximum (complies with Language Profile limit of 5)

## Assumptions

1. `RecordStoreFactory` has a method (or can be queried) to list all map names and their entry counts across partitions. If not, the `/api/admin/maps` handler will iterate `get_all_for_map()` across known maps.
2. `ClusterState` (or equivalent) provides current node list and partition assignments. For v1.0 single-node mode, the handler returns a single-node list with local server info.
3. The `ArcSwap<ServerConfig>` is instantiated during server startup and threaded through `AppState`. Existing `Arc<ServerConfig>` references in `ServiceRegistry`, `OperationService`, and `OperationPipeline` are NOT changed.
4. Static asset directory for the admin dashboard is configurable (default: `./admin-dashboard/dist/`). If the directory does not exist, `/admin/` returns 404.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create OpenAPI spec generation (`openapi.rs`) with `OpenApi` derive aggregating all admin schemas | -- | ~5% |
| G2 | 1 | Implement admin API handlers (`admin.rs`): status, cluster, maps, settings GET/PUT, login | -- | ~25% |
| G3 | 2 | Wire admin routes into `build_app()` in `module.rs`; update `AppState` with new fields in `mod.rs`; add `pub mod` declarations; add `ServeDir` for `/admin/` | G1, G2 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)
