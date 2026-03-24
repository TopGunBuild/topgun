---
id: SPEC-076a
type: feature
status: done
priority: P1
complexity: small
created: 2026-03-04
todo: TODO-093
parent: SPEC-076
depends_on: []
---

# Admin Dashboard v1.0 -- Rust Admin Types & Auth Middleware

## Context

TopGun is building an admin dashboard backed by a Rust axum server. This sub-spec creates the foundational types and auth middleware that the admin API handlers (SPEC-076b) and React dashboard (SPEC-076c) depend on.

The Rust server (`packages/server-rust/`) currently has axum HTTP routes for health, WebSocket, sync, and metrics -- but zero admin API endpoints. The existing `JwtClaims` struct in `auth.rs` is private and lacks a `roles` field, preventing role-based admin access control.

**Parent spec:** SPEC-076 (Admin Dashboard v1.0 -- Rust Server Adaptation)
**Sibling specs:** SPEC-076b (handlers + wiring), SPEC-076c (React dashboard)

### Inherited Decisions

- Admin credentials are configured via `TOPGUN_ADMIN_USERNAME` (default: `"admin"`) and `TOPGUN_ADMIN_PASSWORD` (required) environment variables
- Constant-time comparison via `subtle::ConstantTimeEq` for credential validation
- Same-port admin API (no separate admin port)
- `ArcSwap<ServerConfig>` introduced for admin hot-reload; existing `Arc<ServerConfig>` usages unchanged
- All Rust admin types use `#[serde(rename_all = "camelCase")]`, `ToSchema`, proper integer types (no `f64` for integer semantics)
- Bootstrap mode deferred to v1.1; `GET /api/status` always returns `mode: Normal`

## Task

Define all admin API request/response types with `utoipa::ToSchema` derives, make `JwtClaims` public with a `roles` field, implement the admin auth middleware extractor, and add required dependencies to `Cargo.toml`.

## Requirements

### New files

1. `packages/server-rust/src/network/handlers/admin_types.rs` -- Admin response/request types with `Serialize`, `Deserialize`, `utoipa::ToSchema`:

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

2. `packages/server-rust/src/network/handlers/admin_auth.rs` -- Admin-specific auth middleware:
   - Axum extractor (`AdminClaims`) that validates JWT Bearer token and checks for `"admin"` in the `roles` array
   - Imports `JwtClaims` from `auth.rs` (made `pub` -- see item 4 below)
   - Returns 401 for missing/invalid token, 403 for non-admin role

### Modified files

3. `packages/server-rust/Cargo.toml` -- Add dependencies:
   - `utoipa = { version = "5", features = ["axum_extras"] }`
   - `utoipa-axum = "0.2"`
   - `utoipa-swagger-ui = { version = "9", features = ["axum"] }`
   - `subtle = "2"`
   - Add `"fs"` feature to the **existing** `tower-http` entry (currently has `trace`, `cors`, `timeout`, `request-id`, `compression-gzip`); do NOT add a separate `tower-http` line -- append `"fs"` to the existing features list

4. `packages/server-rust/src/network/handlers/auth.rs` -- Make `JwtClaims` `pub` (currently private); add `pub roles: Option<Vec<String>>` field with `#[serde(skip_serializing_if = "Option::is_none", default)]`; update `handle_auth` to propagate `token_data.claims.roles` into `Principal.roles` (replacing the current hardcoded `roles: vec![]`)

5. `packages/server-rust/src/network/handlers/mod.rs` -- Add `pub mod admin_types;` and `pub mod admin_auth;` declarations

## Acceptance Criteria

1. All admin response structs derive `utoipa::ToSchema` and `serde::Serialize` with `#[serde(rename_all = "camelCase")]`; all `Option<T>` fields have `#[serde(skip_serializing_if = "Option::is_none", default)]`; payload structs with 2+ optional fields derive `Default`
2. `JwtClaims` in `auth.rs` is `pub` with `pub roles: Option<Vec<String>>` field
3. Admin auth middleware (`AdminClaims` extractor) returns 401 for missing/invalid JWT Bearer token
4. Admin auth middleware returns 403 when JWT lacks `"admin"` in `roles` array
5. `Cargo.toml` includes `utoipa`, `utoipa-axum`, `utoipa-swagger-ui`, `subtle` dependencies; `tower-http` has `"fs"` feature added to existing entry (no duplicate line)
6. `ServerMode` and `NodeStatus` are Rust enums (not strings), serialized via `#[serde(rename_all = "camelCase")]`
7. All integer-semantic fields use proper types (`u32`, `u64`, `u16`) -- no `f64` for counts, sizes, or byte values
8. Existing endpoints (`/health`, `/ws`, `/sync`, `/metrics`) continue to function without regression after `JwtClaims` modification
9. `handle_auth` in `auth.rs` propagates decoded `roles` from JWT claims into `Principal.roles` (no longer hardcoded to `vec![]`)
10. `mod.rs` declares `pub mod admin_types;` and `pub mod admin_auth;`

## Constraints

- Do NOT implement admin API endpoint handlers (those are in SPEC-076b)
- Do NOT wire routes into the axum router (that is in SPEC-076b)
- Do NOT modify React dashboard files (those are in SPEC-076c)
- Do NOT modify existing WebSocket protocol or message schema
- 5 Rust files maximum (complies with Language Profile limit of 5)

## Assumptions

1. The existing `JwtClaims` struct in `auth.rs` can be made `pub` without breaking existing code paths, since it is only used internally in `auth.rs` for token verification.
2. Adding `roles: Option<Vec<String>>` to `JwtClaims` with `#[serde(default)]` is backward-compatible: existing JWTs without a `roles` field will deserialize with `roles: None`.
3. The `subtle` crate is used for constant-time credential comparison in the login handler (SPEC-076b), but is added to `Cargo.toml` here to keep dependency management in one place.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Define all admin response/request types with `ToSchema` derives (`admin_types.rs`) including `ServerMode` and `NodeStatus` enums; make `JwtClaims` `pub` and add `roles` field in `auth.rs`; update `handle_auth` to propagate roles; add `pub mod` declarations to `mod.rs` | -- | ~15% |
| G2 | 1 | Add `utoipa` + `utoipa-axum` + `utoipa-swagger-ui` + `subtle` to `Cargo.toml`; add `"fs"` feature to existing `tower-http` entry | -- | ~5% |
| G3 | 2 | Implement admin auth extractor (`admin_auth.rs`) using `pub JwtClaims` from `auth.rs` | G1 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-04)
**Status:** APPROVED

**Context Estimate:** ~30% total

**Dimensions:**
- Clarity: Pass -- requirements are specific with full code examples
- Completeness: Pass (after fixes applied inline during audit)
- Testability: Pass -- all 10 acceptance criteria are measurable
- Scope: Pass -- 5 files, well-bounded
- Feasibility: Pass -- assumptions verified against source code
- Architecture fit: Pass -- follows existing axum extractor and serde patterns
- Non-duplication: Pass -- no reinventing existing functionality
- Cognitive load: Pass -- straightforward types + extractor pattern
- Strategic fit: Aligned with project goals
- Project compliance: Honors PROJECT.md decisions (Rust type mapping, serde conventions, Language Profile)
- Language profile: Compliant with Rust profile (5 files <= 5 max, trait-first G1 is types-only)

**Fixes applied during audit (integrated into spec above):**
1. Added requirement 4 update: `handle_auth` must propagate `claims.roles` to `Principal.roles` (was hardcoded as `roles: vec![]` -- without this, `AdminClaims` extractor would never see admin roles from JWT)
2. Added requirement 5: `mod.rs` must declare `pub mod admin_types;` and `pub mod admin_auth;` (without this, new files would not compile)
3. Added acceptance criteria 9 and 10 for the above
4. Updated file count from 4 to 5 in Constraints section

**Recommendations:**
1. `SettingsUpdateRequest` has `#[serde(skip_serializing_if = ...)]` on `Option` fields but only derives `Deserialize` (not `Serialize`). Harmless but unnecessary -- consider removing the `skip_serializing_if` attributes from request-only structs for clarity.
2. `LoginRequest` derives `Clone, Debug` which means the `password` field could appear in debug log output. Consider implementing a manual `Debug` that redacts the password, or adding a doc comment warning against debug-logging this struct.

**Comment:** Well-structured spec with clear scope boundaries, comprehensive type definitions, and proper separation from sibling specs. The two critical gaps (roles propagation and mod.rs registration) have been fixed inline.

## Execution Summary

**Executed:** 2026-03-04
**Mode:** orchestrated
**Commits:** 1

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/server-rust/src/network/handlers/admin_types.rs` -- All admin API request/response types with ToSchema derives
- `packages/server-rust/src/network/handlers/admin_auth.rs` -- AdminClaims axum extractor for admin role-based auth

### Files Modified
- `packages/server-rust/Cargo.toml` -- Added utoipa, utoipa-axum, utoipa-swagger-ui, subtle; added "fs" to tower-http features
- `packages/server-rust/src/network/handlers/auth.rs` -- Made JwtClaims pub, added roles field, propagated roles to Principal
- `packages/server-rust/src/network/handlers/mod.rs` -- Added pub mod admin_types and admin_auth declarations

### Acceptance Criteria Status
- [x] AC1: All admin response structs derive ToSchema and Serialize with camelCase; Option fields have skip_serializing_if; Default on multi-optional payloads
- [x] AC2: JwtClaims in auth.rs is pub with pub roles: Option<Vec<String>>
- [x] AC3: AdminClaims extractor returns 401 for missing/invalid JWT Bearer token
- [x] AC4: AdminClaims extractor returns 403 when JWT lacks "admin" in roles
- [x] AC5: Cargo.toml includes utoipa, utoipa-axum, utoipa-swagger-ui, subtle; tower-http has "fs" feature (no duplicate line)
- [x] AC6: ServerMode and NodeStatus are Rust enums serialized via camelCase
- [x] AC7: All integer-semantic fields use proper types (u32, u64, u16)
- [x] AC8: Existing endpoints continue to function -- 502 tests pass, 0 failures
- [x] AC9: handle_auth propagates decoded roles from JWT claims into Principal.roles
- [x] AC10: mod.rs declares pub mod admin_types and pub mod admin_auth

### Deviations
None.

## Review History

### Review v1 (2026-03-04)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**

1. **Clippy lint failures: 6 `doc_markdown` warnings in `admin_types.rs`**
   - File: `packages/server-rust/src/network/handlers/admin_types.rs:1,3,98,107,113`
   - Issue: Doc comments reference type/crate names without backticks. `cargo clippy -- -D warnings` fails with 6 errors: module-level doc mentions `TopGun` and `ToSchema` without backticks (lines 1, 3); `SettingsUpdateRequest` doc comments mention `EnvFilter` (line 107), `ArcSwap` (line 113), and `RUST_LOG` / `EnvFilter` on `SettingsResponse` (line 98, two violations on same line).
   - Fix: Wrap technical identifiers in backticks: `` `TopGun` ``, `` `ToSchema` ``, `` `RUST_LOG` ``, `` `EnvFilter` ``, `` `ArcSwap` ``. Run `cargo clippy -- -D warnings` to confirm zero warnings.

**Minor:**

2. `SettingsUpdateRequest` has `#[serde(skip_serializing_if = "Option::is_none")]` on 3 fields but only derives `Deserialize` (not `Serialize`). The attribute has no effect on deserialization. Harmless but misleading -- removing would improve clarity.

3. `LoginRequest` derives `Debug` with a plaintext `password: String` field. In production, accidental `{:?}` formatting could leak credentials to logs. Consider a manual `Debug` impl that redacts the password field.

4. The pre-existing flaky test `websocket_upgrade_and_registry_tracking` fails intermittently (race condition on cleanup timeout). Not caused by this spec but worth noting for overall project health.

**Passed:**

- [pass] AC1: All admin response structs derive `ToSchema` and `Serialize` with `camelCase`; `Option` fields have `skip_serializing_if`; `Default` on multi-optional payloads (`SettingsResponse`, `SettingsUpdateRequest`)
- [pass] AC2: `JwtClaims` in `auth.rs` is `pub` with `pub roles: Option<Vec<String>>` (line 36, 41)
- [pass] AC3: `AdminClaims` extractor returns 401 for missing/invalid JWT Bearer token (`MissingToken` and `InvalidToken` variants)
- [pass] AC4: `AdminClaims` extractor returns 403 when JWT lacks `"admin"` in roles (`Forbidden` variant, line 108)
- [pass] AC5: `Cargo.toml` includes `utoipa` (line 45), `utoipa-axum` (line 46), `utoipa-swagger-ui` (line 47), `subtle` (line 44); `tower-http` has `"fs"` in single entry (line 23)
- [pass] AC6: `ServerMode` and `NodeStatus` are Rust enums with `#[serde(rename_all = "camelCase")]`
- [pass] AC7: All integer fields use proper types (`u32`, `u64`, `u16`) -- no `f64` anywhere
- [pass] AC8: Build succeeds (`cargo check` passes); 497 tests pass, 1 pre-existing flaky failure unrelated to this spec
- [pass] AC9: `handle_auth` propagates `token_data.claims.roles.unwrap_or_default()` into `Principal { roles }` (auth.rs lines 113-118)
- [pass] AC10: `mod.rs` declares `pub mod admin_auth;` and `pub mod admin_types;` (lines 7-8)

**Language Profile:**

- Build check: PASSED (`cargo check -p topgun-server` exits 0)
- Lint check: FAILED (6 `doc_markdown` clippy errors in `admin_types.rs`)
- Test check: 497 passed, 1 pre-existing flaky failure (unrelated to this spec)
- Rust idioms: PASSED (proper `?` usage, no `.unwrap()`, no unnecessary `.clone()`, no `unsafe`)

**Summary:** All 10 acceptance criteria are functionally met. The implementation is clean, well-structured, and follows established project patterns. However, `cargo clippy -- -D warnings` fails due to 6 missing-backtick doc comment violations in `admin_types.rs`. Since the Language Profile mandates clippy-clean code, this is a major issue that must be fixed before approval.

### Fix Response v1 (2026-03-04)
**Commit:** a6a6d27

**Fixes applied:**

1. **MAJOR: Clippy doc_markdown warnings** -- Wrapped all technical identifiers in backticks in doc comments across `admin_types.rs`: `TopGun`, `ToSchema`, `OpenAPI`, `RUST_LOG`, `EnvFilter`, `ArcSwap`. Also fixed a pre-existing `QUERY_UPDATE` backtick issue in `test_server.rs` that was blocking clippy. `cargo clippy -- -D warnings` now exits 0.

2. **MINOR: skip_serializing_if on Deserialize-only struct** -- Removed `#[serde(skip_serializing_if = "Option::is_none")]` from all 3 fields on `SettingsUpdateRequest`, keeping only `#[serde(default)]`. The struct only derives `Deserialize`, so skip_serializing_if had no effect.

3. **MINOR: LoginRequest Debug leaks password** -- Replaced derived `Debug` with manual `impl Debug` that redacts the password field as `"[REDACTED]"`.

**Deviations:**
- [Rule 1 - Bug] Fixed pre-existing `doc_markdown` clippy warning in `test_server.rs` (`QUERY_UPDATE` -> `` `QUERY_UPDATE` ``). Not introduced by this spec but was blocking the clippy gate.

**Verification:**
- `cargo clippy --release -p topgun-server -- -D warnings`: 0 warnings, exits 0
- `cargo test --release -p topgun-server`: 502 passed, 0 failed

### Review v2 (2026-03-04)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Review v1 Fix Verification:**

1. **MAJOR (fixed): Clippy doc_markdown warnings** -- All 6 warnings resolved. Technical identifiers in `admin_types.rs` doc comments now wrapped in backticks: `` `TopGun` `` (line 1), `` `ToSchema` `` (line 3), `` `OpenAPI` `` (line 3), `` `RUST_LOG` `` (line 98), `` `EnvFilter` `` (lines 98, 107), `` `ArcSwap` `` (line 113). `cargo clippy --release -p topgun-server -- -D warnings` exits 0 with zero warnings. VERIFIED.

2. **MINOR (fixed): skip_serializing_if on Deserialize-only struct** -- `SettingsUpdateRequest` (lines 106-116) now has only `#[serde(default)]` on its 3 `Option` fields. The `skip_serializing_if` attributes have been correctly removed. VERIFIED.

3. **MINOR (fixed): LoginRequest Debug leaks password** -- `LoginRequest` (line 118) no longer derives `Debug`. Manual `impl Debug` (lines 125-132) redacts the password field as `"[REDACTED]"`. VERIFIED.

**Passed:**

- [pass] AC1: All admin response structs derive `ToSchema` and `Serialize` with `#[serde(rename_all = "camelCase")]`; `Option<T>` fields on serializable structs have `skip_serializing_if`; `SettingsResponse` and `SettingsUpdateRequest` derive `Default`
- [pass] AC2: `JwtClaims` in `auth.rs` is `pub struct` (line 36) with `pub roles: Option<Vec<String>>` (line 41) and `#[serde(skip_serializing_if = "Option::is_none", default)]`
- [pass] AC3: `AdminClaims` extractor returns 401 via `AdminAuthError::MissingToken` (missing/malformed header) and `AdminAuthError::InvalidToken` (bad JWT)
- [pass] AC4: `AdminClaims` extractor returns 403 via `AdminAuthError::Forbidden` when `roles` does not contain `"admin"` (line 108)
- [pass] AC5: `Cargo.toml` has `utoipa` (line 45), `utoipa-axum` (line 46), `utoipa-swagger-ui` (line 47), `subtle` (line 44); `tower-http` single entry with `"fs"` feature (line 23, no duplicate)
- [pass] AC6: `ServerMode` and `NodeStatus` are Rust enums with `#[serde(rename_all = "camelCase")]` (lines 10-15, 18-24 of `admin_types.rs`)
- [pass] AC7: All integer fields use `u32`, `u64`, or `u16` -- no `f64` anywhere in admin types or auth structs
- [pass] AC8: 502 tests pass (498 unit + 4 integration), 0 failures -- no regression from `JwtClaims` modification
- [pass] AC9: `handle_auth` in `auth.rs` propagates `token_data.claims.roles.unwrap_or_default()` into `Principal { id: user_id, roles }` (lines 113-119)
- [pass] AC10: `mod.rs` declares `pub mod admin_auth;` (line 7) and `pub mod admin_types;` (line 8)

**Language Profile:**

- Build check: PASSED (`cargo clippy` implicitly builds; exits 0)
- Lint check: PASSED (`cargo clippy --release -p topgun-server -- -D warnings` exits 0, zero warnings)
- Test check: PASSED (502 passed, 0 failed, 0 ignored in main test suites)
- Rust idioms: PASSED
  - Proper `?` operator usage throughout (`admin_auth.rs` lines 73, 80, 84, 98)
  - No `.unwrap()` in production code (only `unwrap_or_default()` / `unwrap_or_else()`)
  - No unnecessary `.clone()` calls
  - No `unsafe` blocks
  - `Send + Sync` compliance via standard types

**Summary:** All 3 issues from Review v1 have been correctly fixed. All 10 acceptance criteria pass. The implementation is clean, well-structured, follows established project patterns, and passes all language profile gates (clippy-clean, 502 tests passing). Approved for finalization.
