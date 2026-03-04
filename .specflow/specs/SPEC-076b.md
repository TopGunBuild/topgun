---
id: SPEC-076b
type: feature
status: audited
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
   - `GET /api/status` -- Server status (version, mode). Version sourced from `env!("CARGO_PKG_VERSION")`. Always returns `mode: Normal` for v1.0. Does NOT require admin auth (public endpoint)
   - `GET /api/admin/cluster/status` -- Cluster topology: nodes, partitions, rebalancing state. Requires `AdminClaims`. Maps cluster `NodeState` to admin `NodeStatus` as follows: `Active`/`Joining`/`Leaving` -> `Healthy`, `Suspect` -> `Suspect`, `Dead`/`Removed` -> `Dead`. **Import alias:** use `admin_types::NodeInfo as AdminNodeInfo` to avoid collision with `topgun_core::messages::cluster::NodeInfo`
   - `GET /api/admin/maps` -- List all maps with entry counts. Requires `AdminClaims`. Calls `RecordStoreFactory::map_names()` to enumerate all map names, then `get_all_for_map()` for each to sum entry counts across partitions. Excludes maps with names starting with `$sys/`
   - `GET /api/admin/settings` -- Current server configuration (SettingsResponse). Requires `AdminClaims`. Reads from `ArcSwap<ServerConfig>`, `NetworkConfig`, `SecurityConfig`
   - `PUT /api/admin/settings` -- Update hot-reloadable settings. Requires `AdminClaims`. Accepts `SettingsUpdateRequest` with optional fields `logLevel`, `gcIntervalMs`, `maxConcurrentOperations`. **Read-only field rejection strategy:** Parse the raw request body as `serde_json::Value` first and check for the presence of read-only keys (`nodeId`, `host`, `port`, `partitionCount`, `requireAuth`, `maxValueBytes`, `defaultOperationTimeoutMs`). If any are present, return 400 with `ErrorResponse` listing the forbidden field. Only then deserialize the `Value` into `SettingsUpdateRequest`. This avoids modifying `admin_types.rs` (SPEC-076a constraint)
   - `POST /api/auth/login` -- Admin login. Does NOT require admin auth. Validates `{ username, password }` against `TOPGUN_ADMIN_USERNAME` / `TOPGUN_ADMIN_PASSWORD` env vars using constant-time comparison (`subtle::ConstantTimeEq`); returns `{ token: "..." }` with JWT signed using HS256 (matching `admin_auth.rs` validation), `sub` claim set to the admin username, `roles: ["admin"]`, expiry of 24 hours (`exp` claim); returns 401 on invalid credentials. Signs with `EncodingKey::from_secret(jwt_secret.as_bytes())`

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

3. `packages/server-rust/src/network/handlers/mod.rs` -- Add `pub mod admin;` declaration (note: `pub mod admin_types;` and `pub mod admin_auth;` already exist from SPEC-076a, do NOT re-declare them); add `cluster_state: Option<Arc<ClusterState>>`, `store_factory: Option<Arc<RecordStoreFactory>>`, `server_config: Option<Arc<ArcSwap<ServerConfig>>>` fields to `AppState`. **Note:** The `ArcSwap<ServerConfig>` here is a NEW wrapper introduced specifically for admin hot-reload. Existing services continue using their current `Arc<ServerConfig>` references unchanged

4. `packages/server-rust/src/network/module.rs` -- Update `build_app()` to accept new `AppState` fields. **Approach:** Add a `NetworkModuleOptions` struct with optional fields (`cluster_state`, `store_factory`, `server_config`) and change `build_app()` signature to accept it as an additional parameter. `NetworkModule` gains corresponding setter methods (e.g., `set_cluster_state()`, `set_store_factory()`, `set_server_config()`) that store the values, and `build_router()`/`serve()` pass them through to `build_app()`. This preserves backward compatibility: existing callers that don't set these fields get `None` defaults. Add admin routes to the router:
   - `/api/status` (no auth)
   - `/api/auth/login` (no auth)
   - `/api/admin/*` routes (admin auth required)
   - `/api/openapi.json` and `/api/docs` routes
   - `tower_http::services::ServeDir` for `/admin/` static SPA serving with fallback to `index.html` for SPA client-side routing

5. `packages/server-rust/src/network/mod.rs` -- Add `pub mod openapi;` declaration (since `openapi.rs` lives at the `network/` module level alongside `module.rs`, NOT under `handlers/`)

6. `packages/server-rust/src/storage/factory.rs` -- Add `pub fn map_names(&self) -> Vec<String>` method to `RecordStoreFactory`. Implementation: iterate `store_cache` keys, collect unique map names (the first element of the `(String, u32)` tuple), deduplicate, and return sorted. This enables AC3 (map enumeration for `/api/admin/maps`)

## Acceptance Criteria

1. `GET /api/status` returns `{ configured: true, version: "...", mode: "normal" }` as JSON with correct Content-Type. `version` is sourced from `env!("CARGO_PKG_VERSION")`. `mode` is serialized from the `ServerMode` enum
2. `GET /api/admin/cluster/status` returns node list where each `NodeInfo` has `nodeId: String`, `address: String`, `status: NodeStatus` (serialized as `"healthy"`, `"suspect"`, or `"dead"`), `partitionCount: u32`, `connections: u32`, `memory: u64` (bytes), `uptime: u64` (seconds). Cluster `NodeState` maps to admin `NodeStatus`: `Active`/`Joining`/`Leaving` -> `Healthy`, `Suspect` -> `Suspect`, `Dead`/`Removed` -> `Dead`
3. `GET /api/admin/maps` returns `{ maps: [{ name: "...", entryCount: N }] }` where `entryCount` is `u64`, listing all non-system maps from `RecordStoreFactory` via the new `map_names()` method
4. `GET /api/admin/settings` returns `SettingsResponse` with fields: `nodeId`, `defaultOperationTimeoutMs`, `maxConcurrentOperations`, `gcIntervalMs`, `partitionCount` (from `ServerConfig`); `host`, `port` (from `NetworkConfig`); `requireAuth`, `maxValueBytes` (from `SecurityConfig`); `logLevel` (optional, from runtime)
5. `PUT /api/admin/settings` accepts `SettingsUpdateRequest` with optional fields `logLevel`, `gcIntervalMs`, `maxConcurrentOperations` and applies them without restart. Returns 400 with `ErrorResponse` if request JSON includes any read-only field keys (detected via `serde_json::Value` pre-parse before deserialization into `SettingsUpdateRequest`)
6. `POST /api/auth/login` validates `{ username, password }` against `TOPGUN_ADMIN_USERNAME` / `TOPGUN_ADMIN_PASSWORD` env vars using constant-time comparison (`subtle::ConstantTimeEq`); returns `{ token: "..." }` with JWT signed using HS256 (`EncodingKey::from_secret`), `sub` set to admin username, `roles: ["admin"]`, 24-hour expiry; returns 401 on invalid credentials
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
- 6 Rust files maximum (2 new + 4 modified, complies with Language Profile limit)

## Assumptions

1. `ClusterState` (or equivalent) provides current node list via `current_view()` -> `MembersView` with `members: Vec<MemberInfo>`, and partition assignments via `partition_table`. For v1.0 single-node mode, the handler returns a single-node list with local server info.
2. The `ArcSwap<ServerConfig>` is instantiated during server startup and threaded through `AppState`. Existing `Arc<ServerConfig>` references in `ServiceRegistry`, `OperationService`, and `OperationPipeline` are NOT changed.
3. Static asset directory for the admin dashboard is configurable (default: `./admin-dashboard/dist/`). If the directory does not exist, `/admin/` returns 404.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `map_names()` method to `RecordStoreFactory` in `factory.rs` | -- | ~3% |
| G2 | 1 | Create OpenAPI spec generation (`openapi.rs`) with `OpenApi` derive aggregating all admin schemas; add `pub mod openapi;` to `network/mod.rs` | -- | ~5% |
| G3 | 1 | Implement admin API handlers (`admin.rs`): status, cluster, maps, settings GET/PUT, login | G1 | ~25% |
| G4 | 2 | Wire admin routes into `build_app()` in `module.rs`; update `AppState` with new fields in `handlers/mod.rs`; add `pub mod admin;` declaration; add `ServeDir` for `/admin/` | G2, G3 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 1 | G3 | No (after G1) | 1 |
| 2 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-04)
**Status:** NEEDS_REVISION

**Context Estimate:** ~40% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~40% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Critical:**

1. **RecordStoreFactory has no public API to enumerate map names.** The `store_cache` field is `DashMap<(String, u32), Arc<dyn RecordStore>>` and is private. There is no `map_names()`, `list_maps()`, or similar method. The `get_all_for_map()` method requires you already know the map name. Assumption #1 acknowledges this gap but the fallback ("iterate `get_all_for_map()` across known maps") is circular -- you need to know map names to call `get_all_for_map()`. The spec must either: (a) add `factory.rs` as a 5th modified file with a new `pub fn map_names(&self) -> Vec<String>` method (still within the 5-file language profile limit), or (b) specify an alternative data source for map enumeration. Without this, AC3 is unimplementable.

2. **Read-only field rejection in PUT /api/admin/settings is underspecified.** AC5 requires "Returns 400 if request includes read-only fields" but `SettingsUpdateRequest` (from SPEC-076a) does not have `#[serde(deny_unknown_fields)]`, so extra JSON keys like `nodeId` or `host` are silently ignored by serde. The constraint says "Do NOT modify admin types" (SPEC-076a). The spec must specify the implementation strategy: parse raw JSON (`serde_json::Value`) first to check for forbidden keys, then deserialize. Without this guidance, the implementer may miss this requirement or attempt to modify `admin_types.rs`.

3. **`pub mod admin_types` and `pub mod admin_auth` already declared in mod.rs.** The spec says to "Add `pub mod admin;`, `pub mod admin_types;`, `pub mod admin_auth;` declarations" to `mod.rs`, but the current file already has `pub mod admin_auth;` and `pub mod admin_types;` (lines 7-8 of `handlers/mod.rs`). Only `pub mod admin;` is actually new. This is misleading and could cause a compilation error if the implementer adds duplicate declarations.

**Recommendations:**

4. **NodeInfo name collision with cluster messages.** The admin `NodeInfo` type (in `admin_types.rs`) has the same name as `topgun_core::messages::cluster::NodeInfo` (used by `ClusterState`). The admin handler will need to import both. The spec should note the need for aliased imports (e.g., `use admin_types::NodeInfo as AdminNodeInfo`) to avoid ambiguity when mapping cluster state to admin response types.

5. **AC2 cluster status mapping mismatch.** AC2 says `NodeStatus` serializes as `"healthy"`, `"suspect"`, or `"dead"`, which matches the admin `NodeStatus` enum (`Healthy`, `Suspect`, `Dead`). However, the cluster `NodeState` has 5 variants (`Active`, `Joining`, `Leaving`, `Suspect`, `Dead/Removed`). The spec should clarify how `Active`, `Joining`, and `Leaving` map to the 3 admin status values (presumably all map to `Healthy`).

6. **Login handler JWT generation details missing.** AC6 says the login handler returns a JWT with `roles: ["admin"]`, but does not specify: JWT expiry time, the `sub` claim value (presumably the admin username), or the signing algorithm (presumably HS256 matching `admin_auth.rs` validation). These should be specified for deterministic implementation.

7. **`build_app()` signature change needed but not described.** The current `build_app()` function takes `(config: NetworkConfig, registry, shutdown, observability)` and constructs `AppState` internally. Adding `cluster_state`, `store_factory`, and `server_config` to `AppState` requires either: (a) passing them as additional parameters to `build_app()`, or (b) restructuring `NetworkModule` to hold these. The spec should specify the approach, since `build_app()` is called from both `build_router()` and `serve()`.

8. **`openapi.rs` location should be under `network/` not `network/handlers/`.** The spec says `packages/server-rust/src/network/openapi.rs`, which is at the `network` module level alongside `module.rs`. This is correct architecturally (it's not a handler), but the `mod.rs` for the `network` module will need a `pub mod openapi;` declaration. The spec only mentions modifying `handlers/mod.rs`, not `network/mod.rs`. Clarify which `mod.rs` gets the `pub mod openapi;` declaration.

9. [Strategic] The `version` field in `GET /api/status` response should specify where the version string comes from (e.g., `env!("CARGO_PKG_VERSION")` or a constant). This is minor but avoids ambiguity.

**Project compliance:** Honors PROJECT.md decisions. Rust type mapping rules followed (u32/u64 for integer-semantic fields, enums for known value sets, camelCase serde). No violations detected.

**Language profile:** 4 files within 5-file limit. However, critical issue #1 may require a 5th file (`factory.rs`).

**Strategic fit:** Aligned with project goals (admin dashboard for v1.0).

### Response v1 (2026-03-04)
**Applied:** All 3 critical issues and all 6 recommendations

**Changes:**
1. [+] **RecordStoreFactory map enumeration** -- Added `factory.rs` as modified file #6 with new `pub fn map_names(&self) -> Vec<String>` method. Updated AC3 to reference the new method. Removed circular Assumption #1. Updated file count constraint from "4 files" to "6 files (2 new + 4 modified)". Added G1 task group for the new method.
2. [+] **Read-only field rejection strategy** -- Specified `serde_json::Value` pre-parse approach in both Requirements (PUT endpoint description) and AC5. Explicitly states: parse raw body as `serde_json::Value`, check for forbidden keys, return 400 with `ErrorResponse` if found, then deserialize into `SettingsUpdateRequest`.
3. [+] **Duplicate mod declarations** -- Fixed modified file #3 to state "Add `pub mod admin;` declaration (note: `pub mod admin_types;` and `pub mod admin_auth;` already exist from SPEC-076a, do NOT re-declare them)".
4. [+] **NodeInfo import alias** -- Added import alias guidance to cluster/status endpoint description: `use admin_types::NodeInfo as AdminNodeInfo`.
5. [+] **NodeState to NodeStatus mapping** -- Added explicit mapping in both Requirements and AC2: `Active`/`Joining`/`Leaving` -> `Healthy`, `Suspect` -> `Suspect`, `Dead`/`Removed` -> `Dead` (6 variants, not 5 as audit stated -- `Removed` is a 6th variant).
6. [+] **JWT generation details** -- Specified in both Requirements (login endpoint) and AC6: HS256 algorithm, `sub` = admin username, `roles: ["admin"]`, 24-hour expiry, `EncodingKey::from_secret(jwt_secret.as_bytes())`.
7. [+] **`build_app()` signature change** -- Specified `NetworkModuleOptions` struct approach in modified file #4 description. `NetworkModule` gains setter methods; `build_app()` accepts options as additional parameter. Backward compatible via `None` defaults.
8. [+] **`openapi.rs` module declaration location** -- Added modified file #5: `network/mod.rs` gets `pub mod openapi;` declaration. Clarified that `openapi.rs` lives at network level, not under handlers.
9. [+] **Version source** -- Specified `env!("CARGO_PKG_VERSION")` in both Requirements (status endpoint) and AC1.

### Audit v2 (2026-03-04)
**Status:** APPROVED

**Context Estimate:** ~43% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~43% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Previous Issues Resolution:**
- Critical #1 (RecordStoreFactory map enumeration): RESOLVED -- `factory.rs` added as modified file #6 with `map_names()` method. Implementation correctly iterates `store_cache` DashMap keys (confirmed `DashMap<(String, u32), ...>` at line 47 of factory.rs).
- Critical #2 (Read-only field rejection): RESOLVED -- `serde_json::Value` pre-parse strategy specified. Confirmed `SettingsUpdateRequest` lacks `deny_unknown_fields` (admin_types.rs lines 104-116).
- Critical #3 (Duplicate mod declarations): RESOLVED -- Now correctly states only `pub mod admin;` is new. Confirmed `admin_types` and `admin_auth` already declared at lines 7-8 of handlers/mod.rs.
- Rec #4 (NodeInfo alias): RESOLVED -- Import alias specified.
- Rec #5 (NodeState mapping): RESOLVED -- All 6 variants mapped (confirmed `NodeState` has `Joining`, `Active`, `Suspect`, `Leaving`, `Dead`, `Removed` at cluster/types.rs lines 23-30).
- Rec #6 (JWT details): RESOLVED -- HS256, sub, roles, expiry, signing key all specified.
- Rec #7 (build_app signature): RESOLVED -- `NetworkModuleOptions` struct approach specified.
- Rec #8 (openapi.rs location): RESOLVED -- `network/mod.rs` modification added as file #5.
- Rec #9 (Version source): RESOLVED -- `env!("CARGO_PKG_VERSION")` specified.

**Source Validation:**
- `AppState` (handlers/mod.rs): Confirmed `Option<...>` pattern used for optional fields (lines 49-62). New fields follow same pattern.
- `NetworkModule` (module.rs): Confirmed `build_app()` is called from both `build_router()` (line 96) and `serve()` (line 157). `NetworkModuleOptions` approach is sound.
- `ServerConfig` (service/config.rs): Confirmed fields match spec AC4 (`node_id`, `default_operation_timeout_ms`, `max_concurrent_operations`, `gc_interval_ms`, `partition_count`, `security: SecurityConfig`).
- `SecurityConfig` (service/security.rs): Confirmed `require_auth: bool`, `max_value_bytes: u64` fields exist.
- `NetworkConfig` (network/config.rs): Confirmed `host: String`, `port: u16` fields exist.
- `network/mod.rs`: Currently has 6 `pub mod` declarations (lines 3-8). Adding `pub mod openapi;` is straightforward.

**Project compliance:** Honors PROJECT.md decisions. Rust type mapping rules followed (u32/u64 for integer-semantic fields, enums for known value sets, camelCase serde). No violations detected.

**Strategic fit:** Aligned with project goals (admin dashboard for v1.0).

**Language profile:** 6 files slightly exceeds the 5-file Language Profile limit. However, this is a necessary consequence of addressing Critical #1 from audit v1 (adding `factory.rs`). The 6th file is a minimal single-method addition (~3% context). Acceptable given the trade-off.

**Recommendations:**

1. [Language Profile] File count (6) exceeds the Language Profile limit of 5 by 1. The extra file (`factory.rs`) is a minimal addition (single `map_names()` method, ~3% context). If strict compliance is desired, the `map_names()` method could be added as part of a separate micro-spec, but the overhead of a separate spec likely outweighs the benefit. The constraint line in the spec (line 112) acknowledges this as "complies with Language Profile limit" which is technically inaccurate -- it exceeds the limit by 1 file. Consider updating that constraint line to "6 Rust files (2 new + 4 modified, exceeds Language Profile limit by 1 -- accepted due to minimal factory.rs change)".

2. [Trait-first] The Language Profile requires G1 (Wave 1) to contain only types/traits. This spec has no new types to define (all come from SPEC-076a), so G1 contains the `map_names()` implementation instead. This is a pragmatic deviation -- a vacuous types-only G1 would add no value. No action needed, noted for record.

**Comment:** Well-structured spec with clear separation of concerns from SPEC-076a. All previous critical issues and recommendations have been thoroughly addressed with precise source-level references. Acceptance criteria are measurable and testable. The `NetworkModuleOptions` approach for backward-compatible `build_app()` extension is architecturally sound.
