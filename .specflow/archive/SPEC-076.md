---
id: SPEC-076
type: feature
status: archived
priority: P1
complexity: large
created: 2026-03-03
todo: TODO-093
---

> **SPLIT:** This specification was decomposed into:
> - SPEC-076a: Admin Dashboard v1.0 -- Rust Admin Types & Auth Middleware
> - SPEC-076b: Admin Dashboard v1.0 -- Rust Admin Handlers & Wiring
> - SPEC-076c: Admin Dashboard v1.0 -- React Dashboard Adaptation
>
> See child specifications for implementation.

# Admin Dashboard v1.0 -- Rust Server Adaptation

## Context

TopGun has an existing React 19 + Vite admin dashboard (`apps/admin-dashboard/`) that is ~85% functional. It currently expects a TS server admin API on port 9091 that does not exist in the Rust server. The Rust server (`packages/server-rust/`) has axum HTTP routes for health, WebSocket, sync, and metrics -- but zero admin API endpoints.

The dashboard is a **v1.0 product differentiator** vs Hazelcast Management Center: first-class admin UI from day one, not a paid add-on.

**Current state of the dashboard:**
- Pages: Dashboard (system overview via `$sys/stats` + `$sys/cluster`), Maps list, Data Explorer (CRUD + Monaco JSON editor), Query Playground, Cluster Topology (SVG partition ring), Settings (tabbed config editor), Login
- Data fetching: manual `fetch` + `useState` + `setInterval` polling via `adminFetch()` helper to `http://localhost:9091`
- TopGun client: `@topgunbuild/react` hooks (`useQuery`, `useMap`) for live data via `$sys/*` system maps
- Auth: JWT token in localStorage, `Authorization: Bearer` header on admin API calls
- UI: Tailwind CSS, Radix UI primitives, lucide-react icons, cmdk command palette, Monaco editor, dark mode

**What needs to happen:**
1. Rust server must expose admin API endpoints (cluster status, maps list, settings, auth)
2. Dashboard must be adapted to talk to the Rust server (same port as WS, not separate port 9091)
3. OpenAPI spec generation via `utoipa` for documentation and Swagger UI
4. SWR migration for live data fetching (replace manual fetch+useState+polling)
5. CRDT Debug tab in Data Explorer (placeholder UI in v1.0; populated when system maps are wired in a follow-up spec)

### Goal-Backward Analysis

**Goal Statement:** Operators can monitor, configure, and debug a TopGun Rust server through a browser-based admin dashboard with live-updating metrics, cluster topology visualization, data exploration with CRDT debugging, and environment-variable-based admin authentication.

**Observable Truths:**
1. Visiting `http://<server>/admin/` serves the React dashboard SPA via `tower-http::services::ServeDir`
2. Dashboard shows live cluster topology, node health, and partition ring
3. Data Explorer lists all maps with entry counts and supports CRUD operations
4. CRDT Debug tab UI is present in Data Explorer with placeholder/empty state (data population deferred until system maps are implemented in a follow-up spec)
5. Settings page shows current Rust server configuration and allows hot-reloadable changes
6. All admin API calls are authenticated via JWT with admin role checking

**Required Artifacts:**
- Rust: admin API handler module with axum endpoints + OpenAPI via utoipa
- Rust: admin auth middleware (JWT + admin role gate)
- Rust: admin response types (serde + utoipa structs)
- Rust: static SPA serving via `tower-http::services::ServeDir` for `/admin/*` route
- Rust: system map population deferred to a follow-up spec (Dashboard page will show placeholder data for v1.0; admin REST API endpoints provide equivalent data)
- React: API client layer updated (same-port base URL, SWR hooks, hand-typed response interfaces)
- React: CRDT Debug tab component in Data Explorer (placeholder UI; shows empty state until system maps are populated)
- Vite: proxy config for dev mode, static asset serving in production

**Key Links (fragile/critical):**
- Rust `AppState` must carry new shared data (ClusterState, RecordStoreFactory, ServerConfig) to admin handlers
- System maps (`$sys/*`) are NOT populated in this spec; Dashboard page falls back to admin REST API data
- Admin auth must work with the same JWT secret as WebSocket auth
- `JwtClaims` must be made `pub` and extended with `roles` field for admin auth middleware to access

## Task

Expose admin API endpoints on the Rust axum server and adapt the existing React dashboard to consume them. Generate an OpenAPI spec from Rust types using `utoipa` and serve Swagger UI at `/api/docs`.

**Note:** This spec exceeds the Language Profile limit (8 Rust files vs 5-file max). It **MUST be split** via `/sf:split` before implementation. Recommended split:
- **(A) SPEC-076a:** Rust admin types + auth middleware (`admin_types.rs`, `admin_auth.rs`, `auth.rs` modification, `Cargo.toml` -- 4 Rust files)
- **(B) SPEC-076b:** Rust admin handlers + wiring (`admin.rs`, `openapi.rs`, `mod.rs`, `module.rs` -- 4 Rust files)
- **(C) SPEC-076c:** React dashboard adaptation (14 TS/TSX files, Language Profile limit does not apply to TypeScript)

## Requirements

### Rust Server (packages/server-rust/)

**New files:**

1. `src/network/handlers/admin.rs` -- Admin API endpoint handlers:
   - `GET /api/status` -- Server status (version, mode). Always returns `mode: Normal` for v1.0
   - `GET /api/admin/cluster/status` -- Cluster topology: nodes, partitions, rebalancing state
   - `GET /api/admin/maps` -- List all maps with entry counts
   - `GET /api/admin/settings` -- Current server configuration (see SettingsResponse below)
   - `PUT /api/admin/settings` -- Update hot-reloadable settings (log level, GC interval, max concurrent ops)
   - `POST /api/auth/login` -- Admin login (username/password validated against env vars, returns JWT)

2. `src/network/handlers/admin_types.rs` -- Response/request types with `Serialize`, `Deserialize`, `utoipa::ToSchema`:

   ```rust
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
   ```

   **Hot-reloadable settings** (can be changed via `PUT /api/admin/settings` without restart):
   - `log_level` -- updates the `tracing` EnvFilter at runtime
   - `gc_interval_ms` -- takes effect on the next GC cycle
   - `max_concurrent_operations` -- updated atomically via `ArcSwap<ServerConfig>`

   **Read-only settings** (returned by `GET` but rejected if included in `PUT`):
   - `node_id`, `host`, `port`, `partition_count`, `require_auth`, `max_value_bytes`, `default_operation_timeout_ms`

3. `src/network/handlers/admin_auth.rs` -- Admin-specific auth middleware:
   - Axum extractor (`AdminClaims`) that validates JWT Bearer token and checks for `"admin"` in the `roles` array
   - Imports `JwtClaims` from `auth.rs` (made `pub` -- see item 7 below)
   - Returns 401 for missing/invalid token, 403 for non-admin role

4. `src/network/openapi.rs` -- OpenAPI spec generation:
   - `utoipa::OpenApi` derive macro aggregating all admin endpoint schemas
   - `GET /api/openapi.json` endpoint serving the generated spec
   - `GET /api/docs` serves Swagger UI via `utoipa-swagger-ui`
   - All admin types annotated with `ToSchema`

**Modified files:**

5. `src/network/handlers/mod.rs` -- Add `pub mod admin;`, `pub mod admin_types;`, `pub mod admin_auth;` declarations and re-exports; add `cluster_state: Option<Arc<ClusterState>>`, `store_factory: Option<Arc<RecordStoreFactory>>`, `server_config: Option<Arc<ArcSwap<ServerConfig>>>` fields to `AppState`. **Note:** The `ArcSwap<ServerConfig>` here is a NEW wrapper introduced specifically for admin hot-reload. Existing services (`ServiceRegistry`, `OperationService`, `OperationPipeline`) continue using their current `Arc<ServerConfig>` references unchanged. Admin handlers read from `ArcSwap` for settings GET/PUT; existing services are not refactored
6. `src/network/module.rs` -- Add admin routes to `build_app()` router; add `/api/openapi.json` and `/api/docs` routes; add `tower_http::services::ServeDir` for `/admin/` static SPA serving with fallback to `index.html` for SPA client-side routing
7. `src/network/handlers/auth.rs` -- Make `JwtClaims` `pub` (currently private); add `pub roles: Option<Vec<String>>` field with `#[serde(skip_serializing_if = "Option::is_none", default)]`
8. `Cargo.toml` -- Add `utoipa = { version = "5", features = ["axum_extras"] }`, `utoipa-axum = "0.2"`, `utoipa-swagger-ui = { version = "9", features = ["axum"] }`, `subtle = "2"` as new dependencies. Add `"fs"` feature to the **existing** `tower-http` entry (currently has `trace`, `cors`, `timeout`, `request-id`, `compression-gzip`); do NOT add a separate `tower-http` line -- append `"fs"` to the existing features list

**Admin credential storage (v1.0):**

Admin credentials are configured via environment variables:
- `TOPGUN_ADMIN_USERNAME` (default: `"admin"`)
- `TOPGUN_ADMIN_PASSWORD` (required; server refuses to start admin API if unset)

The `POST /api/auth/login` handler reads these env vars and compares using constant-time comparison (`subtle::ConstantTimeEq`). This is intentionally simple for v1.0. A proper user management system with hashed passwords in PostgreSQL is deferred to v1.1.

### React Dashboard (apps/admin-dashboard/)

**Modified files:**

9. `src/lib/api.ts` -- Change `API_BASE` default from `http://localhost:9091` to same-origin (`''` or `window.location.origin`); add SWR configuration; unify localStorage token key to `topgun_admin_token` (see item 20 below). Update the `login()` return type from `Promise<{ token: string; user: { id: string; username: string; role: string } }>` to `Promise<{ token: string }>` to match the Rust `LoginResponse` shape (no `user` object)
10. `src/hooks/useServerStatus.ts` -- Replace manual fetch+polling with SWR hook. **Note:** The existing file has its own `API_BASE` constant (`import.meta.env.VITE_API_URL || 'http://localhost:9091'`) duplicating the one in `api.ts`. When migrating to SWR, consolidate this to use the shared `API_BASE` from `api.ts` (or the SWR fetcher in `swr-config.ts`) rather than maintaining a second hardcoded URL
11. `src/features/explorer/DataExplorer.tsx` -- Add CRDT Debug tab (placeholder UI showing empty state with explanatory message; populated when system maps are wired in a follow-up spec)
12. `src/features/cluster/ClusterTopology.tsx` -- Replace manual polling with SWR; adapt component to match Rust `NodeInfo` and `PartitionInfo` response shapes. **Note:** The existing `ClusterNode` interface has `partitions: number[]` and `memory: { used, total }` which differ from the Rust `NodeInfo` struct (`partitionCount: u32`, `memory: u64` bytes, `address: String`). The implementer must update the component's type expectations and rendering logic (e.g., remove per-partition array rendering, display memory as a single byte value instead of used/total). **Additionally:** The existing `PartitionInfo` interface has `owner: string` and `replicas: string[]`, while the Rust `PartitionInfo` struct has `ownerNodeId: string` (from `owner_node_id` via camelCase) and no `replicas` field. The partition ring visualization uses `p.owner` to color-code partitions -- this must be updated to `p.ownerNodeId`, and any rendering logic that depends on `replicas` must be removed or stubbed
13. `src/features/settings/Settings.tsx` -- Replace manual polling with SWR; align settings structure with Rust `SettingsResponse`; remove `// Phase 14D-3: Settings Page` comment (violates code comment convention); preserve existing vector search toggle as-is (read-only display, no backend wiring). **Note:** The existing code calls `PATCH /api/admin/settings` and `POST /api/admin/settings/validate` -- neither of these endpoints exist in this spec (spec defines `PUT /api/admin/settings` only). The implementer must rework the save flow: remove the validation endpoint call, change `PATCH` to `PUT`, and restructure error/success handling to match the `PUT` response
14. `src/features/setup/SetupWizard.tsx` -- **Defer to v1.1.** Remove Setup Wizard from navigation/routing for v1.0 (the setup endpoint and bootstrap mode are deferred). The file itself is not deleted, only hidden from the UI
15. `src/App.tsx` -- Remove the `status?.mode === 'bootstrap'` conditional redirect block and the `/setup` route entry. Update the `ServerUnavailable` component text to remove the hardcoded "port 9091" reference (replace with a generic message referencing the server's configured address). Update `ProtectedRoute` to use `topgun_admin_token` as the localStorage key (unifying with `api.ts`)
16. `src/pages/Login.tsx` -- Adapt to the simpler Rust `LoginResponse` shape (`{ token }` only, no `user` object). The existing `Login.tsx` does NOT use the `user` object from the login response -- it calls `await login(username, password)` and navigates on success. The primary change is ensuring the component works with the updated `login()` return type in `api.ts` (which drops the `user` field). No JWT decoding is needed for v1.0 unless a future feature requires displaying the admin username in the UI
17. `src/components/Layout.tsx` -- Change `localStorage.removeItem('topgun_token')` in the logout handler to `localStorage.removeItem('topgun_admin_token')`, unifying with the token key used by `api.ts` and `App.tsx`. Without this change, the logout button would remove the wrong key and leave the actual session token intact
18. `src/lib/client.ts` -- Change `localStorage.getItem('topgun_token')` to `localStorage.getItem('topgun_admin_token')` so the TopGun WebSocket client reads the admin JWT from the same key used by `api.ts`, `App.tsx`, and `Layout.tsx`. This ensures the WebSocket client authenticates with the token stored during login

**New files:**

19. `src/lib/swr-config.ts` -- SWR global configuration (refreshInterval, fetcher, error retry)
20. `src/features/explorer/CrdtDebug.tsx` -- CRDT Debug tab component (placeholder UI for v1.0):
    - Renders an empty state with explanatory text: "CRDT debugging data will be available when system maps are implemented"
    - UI layout prepared for future data: HLC clock state per map, recent merge history, Merkle tree summary, conflict inspector
    - No data fetching in v1.0 (system maps are not populated)
21. `src/lib/admin-api-types.ts` -- Hand-typed TypeScript interfaces matching Rust admin response structs (ServerStatusResponse, ClusterStatusResponse, MapsListResponse, SettingsResponse, SettingsUpdateRequest, LoginRequest, LoginResponse, ErrorResponse, ServerMode, NodeStatus, NodeInfo, PartitionInfo, MapInfo)

**Build configuration:**

22. `package.json` -- Add `swr` dependency
23. `vite.config.ts` -- Add dev proxy for `/api/*` to Rust server; configure static asset serving path

### Deletions

None -- all changes are additions or modifications. (SetupWizard is hidden from navigation, not deleted.)

## Acceptance Criteria

### Rust Server
1. `GET /api/status` returns `{ configured: true, version: "...", mode: "normal" }` as JSON with correct Content-Type. `mode` is serialized from the `ServerMode` enum
2. `GET /api/admin/cluster/status` returns node list where each `NodeInfo` has `nodeId: String`, `address: String`, `status: NodeStatus` (serialized as `"healthy"`, `"suspect"`, or `"dead"`), `partitionCount: u32`, `connections: u32`, `memory: u64` (bytes), `uptime: u64` (seconds)
3. `GET /api/admin/maps` returns `{ maps: [{ name: "...", entryCount: N }] }` where `entryCount` is `u64`, listing all non-system maps from `RecordStoreFactory`
4. `GET /api/admin/settings` returns `SettingsResponse` with fields: `nodeId`, `defaultOperationTimeoutMs`, `maxConcurrentOperations`, `gcIntervalMs`, `partitionCount` (from `ServerConfig`); `host`, `port` (from `NetworkConfig`); `requireAuth`, `maxValueBytes` (from `SecurityConfig`); `logLevel` (optional, from runtime)
5. `PUT /api/admin/settings` accepts `SettingsUpdateRequest` with optional fields `logLevel`, `gcIntervalMs`, `maxConcurrentOperations` and applies them without restart. Returns 400 if request includes read-only fields
6. `POST /api/auth/login` validates `{ username, password }` against `TOPGUN_ADMIN_USERNAME` / `TOPGUN_ADMIN_PASSWORD` env vars using constant-time comparison (`subtle::ConstantTimeEq`); returns `{ token: "..." }` with JWT containing `roles: ["admin"]`; returns 401 on invalid credentials
7. Admin endpoints (except `/api/status`) return 401 without valid JWT Bearer token
8. Admin endpoints return 403 when JWT lacks `"admin"` in `roles` array
9. `GET /api/openapi.json` returns valid OpenAPI 3.0+ spec containing all admin endpoint definitions
10. `GET /api/docs` serves Swagger UI rendering the OpenAPI spec
11. All admin response structs derive `utoipa::ToSchema` and `serde::Serialize` with `#[serde(rename_all = "camelCase")]`; all `Option<T>` fields have `#[serde(skip_serializing_if = "Option::is_none", default)]`; payload structs with 2+ optional fields derive `Default`
12. Existing endpoints (`/health`, `/ws`, `/sync`, `/metrics`) continue to function without regression
13. `JwtClaims` in `auth.rs` is `pub` with `pub roles: Option<Vec<String>>` field
14. `GET /admin/` serves the Vite-built SPA via `tower-http::services::ServeDir` with fallback to `index.html`; returns 404 if the configured static directory does not exist

### React Dashboard
15. Dashboard loads from Rust server (same origin, no port 9091 dependency)
16. Cluster Topology page renders nodes and partition ring from Rust `/api/admin/cluster/status` endpoint, adapted to the `NodeInfo` response shape (`partitionCount` instead of `partitions[]`, `memory` as `u64` bytes instead of `{ used, total }`, `address` as `String`) and the `PartitionInfo` response shape (`ownerNodeId` instead of `owner`, no `replicas` field)
17. Data Explorer lists maps from Rust `/api/admin/maps` endpoint with entry counts
18. CRDT Debug tab renders a placeholder UI with empty state message indicating system map data is not yet available; UI layout is prepared for future HLC, merge history, and Merkle tree data
19. Settings page loads configuration from Rust `/api/admin/settings` and saves changes via `PUT` (not `PATCH`); existing vector search toggle is preserved as read-only display
20. Login page authenticates via Rust `/api/auth/login`, handles the `{ token }` response (no `user` object), and stores JWT in localStorage under the key `topgun_admin_token`
21. SWR replaces manual `fetch`+`useState`+`setInterval` in at least ClusterTopology, Settings, and useServerStatus
22. All existing dashboard pages render without JavaScript errors when connected to Rust server
23. `Settings.tsx` does not contain phase/spec/bug reference comments
24. Setup Wizard is hidden from navigation (not accessible in v1.0); file is preserved for v1.1
25. `App.tsx` does not contain hardcoded "port 9091" references; `ServerUnavailable` component uses a generic server address message
26. `App.tsx`, `api.ts`, `Layout.tsx`, and `client.ts` all use the same localStorage token key (`topgun_admin_token`). Specifically, `Layout.tsx` logout handler removes `topgun_admin_token` (not `topgun_token`), and `client.ts` reads auth token from `topgun_admin_token`
27. `useServerStatus.ts` does not have its own `API_BASE` constant; it uses the shared base URL from `api.ts` or the SWR fetcher

## Constraints

- Do NOT add v2.0 features (pipeline visualization, connector wizard, Arroyo-style DAG editor)
- Do NOT add v3.0 features (tenant admin, S3 storage config, vector search config beyond toggle)
- Do NOT modify existing WebSocket protocol or message schema
- Do NOT remove the TS server admin API compatibility (it will be removed in TODO-103)
- Admin endpoints run on the SAME port as the main server (no separate admin port)
- OpenAPI spec must be generated from Rust code (not hand-written YAML)
- CRDT Debug tab renders placeholder UI in v1.0; it will read live data from system maps once they are populated in a follow-up spec (system maps are NOT populated in this spec)
- This spec MUST be split via `/sf:split` before implementation (8 Rust files exceeds 5-file Language Profile limit)

## Assumptions

1. **Same-port admin API:** Admin endpoints are served on the same axum server as WebSocket/health/metrics (no separate admin port). The existing dashboard's port 9091 reference will be updated to same-origin.
2. **Admin role in JWT:** The admin role check uses a `roles` array in JWT claims (e.g., `{ sub: "admin", roles: ["admin"] }`). The existing `JwtClaims` struct in `auth.rs` will be made `pub` and extended with `pub roles: Option<Vec<String>>`.
3. **Bootstrap mode deferred:** Setup Wizard, `POST /api/setup`, and bootstrap mode detection are deferred to v1.1. For v1.0, `GET /api/status` always returns `mode: "normal"`. The Setup Wizard page is hidden from navigation but the source file is preserved.
4. **Settings storage:** Hot-reloadable settings are stored in a NEW `ArcSwap<ServerConfig>` wrapper introduced specifically for admin hot-reload and updated atomically. Existing services (`ServiceRegistry`, `OperationService`, `OperationPipeline`) continue using their current `Arc<ServerConfig>` references unchanged -- this spec does NOT refactor existing `Arc<ServerConfig>` usages. Admin handlers read from the `ArcSwap` for settings GET/PUT. Non-hot-reloadable settings require a restart. See `SettingsUpdateRequest` for the exact hot-reloadable field list.
5. **Admin credentials via env vars:** For v1.0, admin login credentials come from `TOPGUN_ADMIN_USERNAME` (default `"admin"`) and `TOPGUN_ADMIN_PASSWORD` (required). Credentials are validated with constant-time comparison (`subtle::ConstantTimeEq`). A proper user management system is deferred to v1.1.
6. **SWR replaces polling:** SWR with `refreshInterval: 5000` (cluster) and `refreshInterval: 1000` (dashboard stats) replaces manual `setInterval` polling.
7. **Static asset serving:** In production, Vite-built dashboard assets are served by the Rust server via `tower-http::services::ServeDir` from a configurable directory (default: `./admin-dashboard/dist/`), with SPA fallback to `index.html`. In development, Vite dev server proxies `/api/*` to the Rust server.
8. **System maps deferred:** The `$sys/stats`, `$sys/cluster`, `$sys/maps` system map population is NOT implemented in this spec. The Dashboard page will show placeholder/empty data for system-map-backed widgets in v1.0. Admin REST API endpoints (`/api/admin/cluster/status`, `/api/admin/maps`) provide equivalent data and are the primary data source for v1.0. The CRDT Debug tab shows placeholder UI only; it will be wired to system maps in a follow-up spec. **Note:** `pages/Dashboard.tsx` and `pages/Cluster.tsx` (which use `useQuery('$sys/stats')` and `useQuery('$sys/cluster')` via the TopGun WebSocket client) will show placeholder/empty states or "No Cluster Data" messages in v1.0 since system maps are not populated. These pages are separate from the admin-API-backed `features/cluster/ClusterTopology.tsx` and will render without JavaScript errors (satisfying AC 22) but with degraded UX until system maps are implemented in a follow-up spec.
9. **Hand-typed API interfaces:** For v1.0, TypeScript response interfaces are hand-typed in `admin-api-types.ts` to match Rust structs. OpenAPI codegen (`openapi-typescript` + `openapi-fetch`) is deferred until the API surface grows beyond ~8 endpoints.
10. **Vector search toggle preserved:** The existing vector search toggle in `Settings.tsx` is preserved as a read-only display element. It has no backend wiring in v1.0 (consistent with the v3.0 deferral constraint).

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define admin response/request types with `ToSchema` derives (`admin_types.rs`) including `NodeStatus` enum; make `JwtClaims` `pub` and add `roles` field in `auth.rs` | -- | ~15% |
| G2 | 1 | Add `utoipa` + `utoipa-axum` + `utoipa-swagger-ui` + `subtle` to `Cargo.toml`; add `"fs"` feature to existing `tower-http` entry; create `openapi.rs` with `OpenApi` derive | -- | ~5% |
| G3 | 2 | Implement admin auth extractor (`admin_auth.rs`) using `pub JwtClaims` from `auth.rs` | G1 | ~10% |
| G4 | 2 | Implement admin API handlers (`admin.rs`): status, cluster, maps, settings, login (no setup) | G1, G2 | ~25% |
| G5 | 3 | Wire admin routes into `build_app()` in `module.rs`; update `AppState` with new fields; update `mod.rs`; add `ServeDir` for `/admin/` | G3, G4 | ~10% |
| G6 | 3 | React: SWR migration (`swr-config.ts`, update `api.ts`, `useServerStatus.ts`, `ClusterTopology.tsx`, `Settings.tsx`); create `admin-api-types.ts` with hand-typed interfaces (including `NodeStatus`); update `Login.tsx` for simpler response shape; update `App.tsx` (remove bootstrap redirect, `/setup` route, port 9091 reference, unify token key); update `Layout.tsx` (unify logout token key to `topgun_admin_token`); update `client.ts` (unify auth token key to `topgun_admin_token`) | -- | ~20% |
| G7 | 3 | React: CRDT Debug tab placeholder UI (`CrdtDebug.tsx`, integrate into `DataExplorer.tsx`) | -- | ~5% |
| G8 | 4 | React: Hide Setup Wizard from navigation; update `package.json` (add `swr`); update `vite.config.ts` proxy | G5 | ~5% |
| G9 | 4 | Integration verification: all dashboard pages work against Rust server | G5, G6, G7 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5, G6, G7 | Yes | 3 |
| 4 | G8, G9 | Yes | 2 |

**Total workers needed:** 3 (max in any wave)

### File Count Analysis

**Rust files (new + modified):** 8 files
- New: `admin.rs`, `admin_types.rs`, `admin_auth.rs`, `openapi.rs` (4 new)
- Modified: `mod.rs`, `module.rs`, `Cargo.toml`, `auth.rs` (4 modified)

**React files (new + modified):** 14 files
- New: `swr-config.ts`, `CrdtDebug.tsx`, `admin-api-types.ts` (3 new)
- Modified: `api.ts`, `useServerStatus.ts`, `DataExplorer.tsx`, `ClusterTopology.tsx`, `Settings.tsx`, `SetupWizard.tsx`, `App.tsx`, `Login.tsx`, `Layout.tsx`, `client.ts`, `package.json`, `vite.config.ts` (11 modified)

**Total: 22 files (8 Rust + 14 React)**

> **Split required:** This exceeds the Language Profile limit of 5 Rust files per spec. This spec MUST be split via `/sf:split` into SPEC-076a (4 Rust files: types + auth), SPEC-076b (4 Rust files: handlers + wiring), and SPEC-076c (14 TS/TSX files: React dashboard). See the Task section above for the recommended split boundaries.

## Audit History

### Audit v1 (2026-03-03)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (far exceeds 50% target)

**Critical:**

1. **Language Profile violation: 8 Rust files exceed 5-file limit.** PROJECT.md sets `Max files per spec: 5` for Rust. This spec has 8 Rust files (4 new + 4 modified). The spec itself acknowledges this at the bottom but does not resolve it. This spec MUST be split via `/sf:split` before implementation. Recommended split: (A) Rust admin types + auth middleware (admin_types.rs, admin_auth.rs, auth.rs modification, Cargo.toml -- 4 files), (B) Rust admin handlers + wiring (admin.rs, openapi.rs, mod.rs, module.rs -- 4 files), (C) React dashboard adaptation (10 TS files, no Rust file limit applies).

2. **Contradictory requirements: bootstrap mode vs setup endpoint.** Assumption 3 states bootstrap detection always returns `mode: "normal"` for v1.0 ("full bootstrap wizard deferred to v1.1"). Yet AC 6 requires `POST /api/setup` to accept a bootstrap config payload and return `{ success: true, token: "..." }`, and AC 18 requires the Setup Wizard to "complete bootstrap flow." These are contradictory -- if bootstrap is always "normal," the setup endpoint has no purpose and the Setup Wizard will never be triggered. Either remove the setup endpoint/wizard from this spec (defer to v1.1) or implement real bootstrap detection.

3. **Missing artifact: admin credential storage.** AC 7 requires `POST /api/auth/login` to "validate credentials and return JWT token with admin role." The spec never defines WHERE or HOW admin credentials are stored. There is no admin user table, no config file with credentials, no in-memory credential store specified. The login handler cannot validate credentials without a backing store. Specify the credential storage mechanism (e.g., hardcoded env var for v1.0, or a `users` table in PostgreSQL).

4. **Missing artifact: static SPA serving from Rust.** Observable Truth 1 states "Visiting `http://<server>/admin/` serves the React dashboard SPA." Assumption 7 mentions static asset serving. However, there is NO file, task group, or acceptance criterion for implementing static file serving in the Rust server. No `tower-http::services::ServeDir` or equivalent is specified. Without this, the dashboard cannot be served in production.

5. **Missing artifact: system map population.** The Goal Analysis lists "Rust: system map data population (for `$sys/stats`, `$sys/cluster`, `$sys/maps`)" as a Required Artifact, and a Key Link states "System maps must be populated by Rust server for Dashboard page to work." However, there is NO task group, file, or acceptance criterion that implements this background task. Without system map population, the Dashboard page (which uses `useQuery`/`useMap` hooks via WebSocket for `$sys/*` maps) will show empty data.

6. **Rust type mapping violations.** Per PROJECT.md Auditor Checklist: (a) `mode` field in `ServerStatusResponse` is described as `"bootstrap" | "normal"` string literal but must be a Rust enum per the "Enums over strings for known value sets" rule. (b) `entryCount` in `MapInfo`, `connections` and `uptime` in `NodeInfo` are described without explicit integer types -- they must be `u32` or `u64`, not left ambiguous. (c) No mention of `#[serde(skip_serializing_if = "Option::is_none", default)]` on `Option<T>` fields. (d) No mention of `Default` derive on payload structs with 2+ optional fields (e.g., `SetupRequest`, `LoginRequest`, `SettingsResponse`).

7. **`JwtClaims` is private and needs refactoring.** The spec says G1 extends `JwtClaims` in `auth.rs` with a `roles` field, and G3 creates `admin_auth.rs` that "reuses existing `AuthHandler` JWT verification." However, `JwtClaims` is currently a private `struct` (not `pub`) in `auth.rs`. The admin auth middleware cannot access it without either making it public or extracting it to a shared module. The spec must specify this refactoring explicitly.

8. **`SettingsResponse` is underspecified.** Requirement item 2 describes it as "General, storage, security, cluster, rate limit sections" without defining any specific fields. The existing `Settings.tsx` expects specific fields like `port`, `metricsPort`, `logLevel`, `connectionString`, `jwtAlgorithm`, `sessionTimeout`. The existing `ServerConfig` in Rust has `node_id`, `default_operation_timeout_ms`, and likely other fields. AC 4 says it returns "ServerConfig + NetworkConfig + SecurityConfig" but never defines what a `SecurityConfig` is. Which fields are hot-reloadable (AC 5)? The implementer cannot build this without guessing.

**Recommendations:**

9. [Strategic] Consider whether OpenAPI codegen (`openapi-typescript` + `openapi-fetch`) is worth the complexity for v1.0. The existing `adminFetch()` pattern works, and the admin API has only ~8 endpoints. The codegen adds a build step, a dependency on the Rust server being running during build, and type generation complexity. For v1.0, hand-typing the 8 response interfaces in TypeScript (which already partially exist in the dashboard code) may be simpler. OpenAPI codegen becomes valuable when the API surface grows.

10. [Compliance] The existing `Settings.tsx` has a line `// Phase 14D-3: Settings Page` which violates the "No phase/spec/bug references in code comments" rule from PROJECT.md. When modifying this file, remove that comment.

11. The `SettingsData` interface in `Settings.tsx` includes `integrations.mcp` and `integrations.vectorSearch` fields. The spec constraints say "Do NOT add v3.0 features (vector search config beyond toggle)." Clarify whether the existing vector search toggle in Settings should be preserved or removed.

12. Consider adding an acceptance criterion for Swagger UI accessibility (since `utoipa-swagger-ui` is listed as a dependency). If Swagger UI is included, specify the route (e.g., `GET /api/docs`).

### Response v1 (2026-03-03)
**Applied:** All 8 critical issues and all 4 recommendations.

### Audit v2 (2026-03-03)
**Status:** NEEDS_REVISION

### Response v2 (2026-03-03)
**Applied:** All 4 critical issues and all 5 recommendations.

### Audit v3 (2026-03-03)
**Status:** NEEDS_DECOMPOSITION

### Response v3 (2026-03-03)
**Applied:** All 2 critical issues and all 4 recommendations.

### Audit v4 (2026-03-03)
**Status:** NEEDS_REVISION

### Response v4 (2026-03-03)
**Applied:** All 1 critical issue and all 3 recommendations.

### Audit v5 (2026-03-03)
**Status:** NEEDS_DECOMPOSITION

**Comment:** After 4 audit/revision cycles, this specification is comprehensive and well-structured. All critical issues from previous audits have been resolved. The spec correctly self-identifies its Language Profile violation and mandates splitting before implementation. No remaining critical issues. Ready for decomposition via `/sf:split`.

**Recommendation:** Split via `/sf:split` into SPEC-076a (Rust types + auth), SPEC-076b (Rust handlers + wiring), SPEC-076c (React dashboard), then use `/sf:run` on each sub-spec independently.
