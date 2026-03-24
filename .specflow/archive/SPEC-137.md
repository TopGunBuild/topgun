---
id: SPEC-137
type: bugfix
status: done
priority: P0
complexity: medium
created: 2026-03-22
source: TODO-163
---

# SPEC-137: Fix Critical Auth Vulnerabilities (JWT Expiry, Secret Loading, CORS, Sub Claim, TLS Warning)

## Context

A security audit on 2026-03-22 identified five vulnerabilities in the Rust server authentication and network layers. Two are P0 (credential permanence, auth bypass in production), three are P1 (CORS wildcard, anonymous identity, plaintext credentials). All must be fixed before any public-facing deployment, including demo.topgun.build.

## Goal Statement

After this spec is complete, the Rust server rejects expired JWT tokens, refuses to start without a JWT secret when auth is required, restricts CORS to an explicit allowlist, rejects tokens missing the `sub` claim, and warns when JWT auth is configured without TLS.

### Observable Truths

1. A JWT token with `exp` in the past is rejected by both WebSocket auth and admin auth extractors.
2. A JWT token with `exp` 30 seconds in the past (within clock skew leeway of 60s) is accepted.
3. `NetworkModule` reads `JWT_SECRET` from the environment and passes it to `AppState`.
4. The server refuses to start (panics or returns error) when `server_config` is `Some`, `require_auth: true`, and no JWT secret is available.
5. Default `cors_origins` is an empty `Vec`, not `["*"]`. An empty allowlist rejects cross-origin requests.
6. A valid JWT token without a `sub` claim is rejected with an auth error (not silently assigned "anonymous").
7. When `jwt_secret` is `Some(_)` and `tls` is `None`, a `warn!` log message is emitted at startup.

### Required Artifacts

| File | Change |
|------|--------|
| `packages/server-rust/src/network/handlers/auth.rs` | Re-enable `exp` validation, add `leeway: u64` parameter to `handle_auth()`, reject missing `sub` with `AUTH_FAIL` send, add `exp` field to `JwtClaims` |
| `packages/server-rust/src/network/handlers/admin_auth.rs` | Re-enable `exp` validation, add leeway read from `AppState.config`, reject missing `sub` |
| `packages/server-rust/src/network/config.rs` | Change `cors_origins` default to empty `Vec`, add `jwt_clock_skew_secs` field |
| `packages/server-rust/src/network/module.rs` | Read `JWT_SECRET` env var, refuse to start when `server_config` is `Some` and `require_auth: true` and secret absent, emit TLS warning |
| `packages/server-rust/src/network/handlers/websocket.rs` | Pass `state.config.jwt_clock_skew_secs` as the `leeway` argument at the `handle_auth()` call site (one-line change, line 130) |

## Task

Fix five security vulnerabilities in the Rust server's authentication and network configuration.

## Requirements

### R1: Re-enable JWT `exp` Claim Validation (P0)

**Files:** `auth.rs`, `admin_auth.rs`

In both `auth.rs` (line 102) and `admin_auth.rs` (line 93), replace `validation.required_spec_claims.clear()` with:
- Keep `validate_aud = false` (TopGun tokens do not use audience claims).
- Set `validation.leeway` to the clock skew tolerance.
- Do NOT clear `required_spec_claims` -- the `jsonwebtoken` crate defaults to requiring `exp`, which is the desired behavior.

**In `auth.rs`:** Add a `leeway: u64` parameter to `AuthHandler::handle_auth()`. The caller in `websocket.rs` (line 130) passes `state.config.jwt_clock_skew_secs` as this argument. This keeps `AuthHandler::new()` signature unchanged and limits `websocket.rs` to a one-line call-site update.

**In `admin_auth.rs`:** The `AdminClaims` extractor reads clock skew directly from `state.config.jwt_clock_skew_secs` (the extractor already has access to `AppState`).

### R2: Load JWT Secret from Environment in NetworkModule (P0)

**File:** `module.rs`

In `build_app()` (line 239), replace `jwt_secret: None` with:
- Read `std::env::var("JWT_SECRET")`.
- If the env var is set and non-empty, use `Some(secret)`.
- If the env var is unset/empty, use `None`.

Add a startup validation check (called during `start()` or `build_router()`):
- Only execute the check when `server_config` is `Some`.
- Load `server_config` via `.load()` and read `security.require_auth`.
- If `require_auth` is `true` AND `jwt_secret` is `None`, panic with: `"JWT_SECRET environment variable is required when require_auth is true"`.
- When `server_config` is `None` (e.g., in unit tests, load harness, `test_server.rs`), skip the check entirely — do not panic.

### R3: Change CORS Default to Empty Allowlist (P1)

**File:** `config.rs`

Change `NetworkConfig::default()` so `cors_origins` defaults to `vec![]` (empty), not `vec!["*".to_string()]`.

The existing `build_cors_layer()` logic in `middleware.rs` already handles an empty list correctly — `AllowOrigin::list()` with an empty iterator produces no allowed origins, so no change to `middleware.rs` is needed.

Update the existing test `default_cors_origins_is_wildcard` (or equivalent) to assert the new empty default.

### R4: Reject Tokens Without `sub` Claim (P1)

**Files:** `auth.rs`, `admin_auth.rs`

Replace `.unwrap_or_else(|| "anonymous".to_string())` with an explicit check:
- If `token_data.claims.sub` is `None`, send an `AUTH_FAIL` message to the client channel before returning an error, then return the error. This matches the pattern used in the `Err(e)` decode branch.
- In `auth.rs`: send `AUTH_FAIL` to the client, then return `AuthError::InvalidToken { reason: "missing sub claim in JWT".to_string() }`.
- In `admin_auth.rs`: return `AdminAuthError::InvalidToken("missing sub claim in JWT".to_string())` (no channel send — admin extractor is HTTP-based, not WebSocket).

### R5: Warn When JWT Configured Without TLS (P1)

**File:** `module.rs`

During `build_router()` or `start()`, after determining `jwt_secret`:
- If `jwt_secret.is_some()` and `self.config.tls.is_none()`, emit: `warn!("JWT authentication is enabled but TLS is not configured. Credentials will be sent in plaintext over ws://. Configure TLS for production deployments.")`.
- This is a warning only, not a hard error (development use case).

### R6: Add `jwt_clock_skew_secs` to NetworkConfig

**File:** `config.rs`

Add field `pub jwt_clock_skew_secs: u64` to `NetworkConfig` with a default of `60`. Both `auth.rs` and `admin_auth.rs` read this value from `state.config` at validation time.

### R7: Add `exp` Field to `JwtClaims`

**File:** `auth.rs`

Add `#[serde(default)] pub exp: Option<u64>` to the `JwtClaims` struct. The `jsonwebtoken` crate validates `exp` from raw JSON even without a struct field, but making it explicit enables future use and improves clarity. The `#[serde(default)]` attribute matches the existing pattern on `roles` and ensures consistent serde behavior across all `Option<T>` fields in `JwtClaims`, preventing deserialization errors if the struct is used outside the `jsonwebtoken` decode path.

## Acceptance Criteria

1. **AC1:** JWT with `exp` set to 1 hour ago is rejected by `AuthHandler::handle_auth()` with `AuthError::InvalidToken`.
2. **AC2:** JWT with `exp` set to 1 hour from now is accepted by `AuthHandler::handle_auth()`.
3. **AC3:** JWT with `exp` set to 30 seconds ago is accepted when `jwt_clock_skew_secs` is 60 (within leeway).
4. **AC4:** JWT with `exp` set to 90 seconds ago is rejected when `jwt_clock_skew_secs` is 60 (beyond leeway).
5. **AC5:** `NetworkModule::build_router()` reads `JWT_SECRET` env var and populates `AppState.jwt_secret`.
6. **AC6:** When `server_config` is `Some`, `require_auth: true`, and `JWT_SECRET` is unset, `build_router()` panics with descriptive message. When `server_config` is `None`, no panic occurs.
7. **AC7:** `NetworkConfig::default().cors_origins` is an empty `Vec`.
8. **AC8:** JWT with valid signature but no `sub` claim is rejected in both `auth.rs` and `admin_auth.rs`.
9. **AC9:** Admin auth extractor rejects tokens without `sub` with HTTP 401.
10. **AC10:** When `jwt_secret` is `Some` and `tls` is `None`, a `warn!` level log is emitted during startup.
11. **AC11:** Existing integration tests still pass (TS token generators already include `exp` via `expiresIn: '1h'` and `sub` — no changes needed to TS helpers).
12. **AC12:** In `auth.rs`, when `sub` is `None`, an `AUTH_FAIL` message is sent to the client channel before the error is returned.

## Validation Checklist

1. Run `SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server` -- all tests pass, including new auth tests.
2. Run `pnpm test:integration-rust` -- all 61 integration tests pass. Note: TS test helpers in `tests/integration-rust/` already include `expiresIn: '1h'` (sets `exp`) and `sub` — no changes to TS helpers are needed.
3. Verify `cargo clippy -p topgun-server -- -D warnings` produces no warnings.
4. Manually confirm: start server without `JWT_SECRET` when `require_auth: true` -- server refuses to start with clear error.
5. Manually confirm: start server with `JWT_SECRET` but without TLS config -- warning log appears.

## Constraints

- Do NOT add new crate dependencies. The `jsonwebtoken` crate already supports `exp` validation and `leeway`.
- Do NOT change the JWT algorithm (HS256 stays).
- Do NOT make TLS mandatory -- the warning is informational only.
- Do NOT change the `SecurityConfig` struct or `WriteValidator` -- those are not affected.
- Do NOT add `iss` or `aud` validation -- TopGun tokens do not use those claims.
- Do NOT add a `clock_skew_secs` field to `AuthHandler` or change `AuthHandler::new()` -- read clock skew via the `leeway` parameter passed to `handle_auth()` instead.

## Assumptions

- Clock skew tolerance of 60 seconds is reasonable for production deployments.
- `JWT_SECRET` as an environment variable name follows the existing convention (no config file support needed yet).
- The `jsonwebtoken` crate's default `required_spec_claims` includes `exp` when not cleared -- verified by crate documentation.
- TS integration test token generators in `tests/integration-rust/` already include `expiresIn: '1h'` (which sets `exp`) and `sub` — no updates needed.
- Empty CORS allowlist is acceptable as default because developers must explicitly configure origins for their deployment.
- `test_server.rs` and the load harness `main.rs` construct `AppState` directly (bypassing `NetworkModule`), hardcoding `jwt_secret: Some("test-e2e-secret")`. This bypasses the R2 env-var reading path intentionally. These paths also set `server_config: None`, so the R2 startup validation check is skipped. This is correct test infrastructure behavior and requires no code change.

## Task Groups

### Implementation Tasks

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `jwt_clock_skew_secs` to `NetworkConfig`, add `exp` field to `JwtClaims`, change `cors_origins` default to empty vec | -- | ~15% |
| G2 | 2 | Fix `auth.rs`: remove `required_spec_claims.clear()`, add `leeway: u64` param to `handle_auth()`, reject missing `sub` with `AUTH_FAIL` send; update `websocket.rs` call site to pass `state.config.jwt_clock_skew_secs` | G1 | ~20% |
| G3 | 2 | Fix `admin_auth.rs`: remove `required_spec_claims.clear()`, read leeway from `state.config`, reject missing `sub` | G1 | ~20% |
| G4 | 2 | Fix `module.rs`: read `JWT_SECRET` env var, conditional startup validation (only when `server_config` is `Some`), TLS warning | G1 | ~25% |
| G5 | 3 | Add `#[cfg(test)]` modules to `auth.rs` and `admin_auth.rs` with unit tests for expired tokens, leeway behavior, and missing-sub rejection (AC1-AC4, AC8, AC12) | G2, G3 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3, G4 | Yes | 3 |
| 3 | G5 | No | 1 |

**Total workers needed:** 3 (max in any wave)

**Note:** G1 is config/types only (trait-first). G2 and G3 are independent auth handler fixes. G4 is independent of G2/G3 — it only needs `NetworkConfig` to exist (G1). G4 touches `module.rs` only; G2 touches `auth.rs` and `websocket.rs`; G3 touches `admin_auth.rs`. G5 adds test modules to `auth.rs` and `admin_auth.rs` after both handlers are complete.

## Audit History

### Audit v1 (2026-03-23)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total

**Critical:**
1. **Missing file: `websocket.rs` not listed as artifact or in any task group.** R1 adds a `clock_skew_secs: u64` parameter to `AuthHandler::new()`, but `AuthHandler::new()` is called in `websocket.rs` (lines 83 and 129), not in `auth.rs` itself. After changing the `AuthHandler::new()` signature, `websocket.rs` will fail to compile. Either (a) add `websocket.rs` to Required Artifacts and to G2's task list, or (b) redesign so `AuthHandler` reads clock skew from `AppState.config` instead of storing it as a field (this avoids touching `websocket.rs` entirely -- the `ws_handler` already has access to `state.config`).
2. **R2 references `server_config.security.require_auth` but `server_config` may be `None`.** The spec says to check `require_auth: true` in `build_router()` or `start()`, but `server_config` is `Option<Arc<ArcSwap<ServerConfig>>>` and is only set via `set_server_config()`. When `server_config` is `None` (e.g., in existing tests, load harness), the startup validation must not panic. The spec must clarify: skip the check when `server_config` is `None`, or require `server_config` to always be set. Recommended: only panic when `server_config` is `Some` AND `require_auth` is `true` AND `jwt_secret` is `None`.
3. **R4 in `auth.rs`: missing `AUTH_FAIL` send for sub-claim rejection.** The spec says to "return `AuthError::InvalidToken`" when `sub` is `None`, but this code path is inside the `Ok(token_data)` branch of the JWT decode match. Unlike the `Err(e)` branch (which sends `AUTH_FAIL` before returning error), the `Ok` branch has no channel send. The spec must explicitly state that an `AUTH_FAIL` message should be sent before returning the error, matching the pattern in the `Err` branch. Otherwise the client receives no feedback -- the connection just drops.

**Recommendations:**
4. [Strategic] The TS integration test helper (`tests/integration-rust/helpers/test-client.ts` line 36) already includes `expiresIn: '1h'` which sets `exp`. Assumption 4 says tests "may need `exp` added" -- this is already done. The `sub` claim is also already present. Validation checklist item 2 note about "TS test token generators must be updated" is misleading and may confuse the implementer. Recommend updating this note.
5. The `JwtClaims` struct in `auth.rs` should add an `exp` field (`pub exp: Option<u64>`) so that the `jsonwebtoken` crate can deserialize and validate it. Currently the struct only has `sub` and `roles`. The `jsonwebtoken` crate validates `exp` from the raw JSON even without a struct field, but making it explicit improves clarity and enables future use.
6. G2 context estimate of ~25% may be tight given it requires understanding the `AuthHandler` construction chain, adding a field, updating `new()`, changing validation logic, AND adding the sub-rejection with AUTH_FAIL send. Consider whether G2 needs the `websocket.rs` change folded in or the alternative design (read from config) to stay within budget.
7. [Compliance] `test_server.rs` and load harness `main.rs` both construct `AppState` directly (bypassing `NetworkModule`), hardcoding `jwt_secret: Some("test-e2e-secret")`. The R2 env-var reading in `build_app()` will not affect these paths. This is acceptable for test infrastructure but should be documented as intentional in the spec to avoid confusion.

### Response v1 (2026-03-23)
**Applied:** All critical issues and all recommendations.

**Changes:**
1. [✓] Missing `websocket.rs` / `AuthHandler` field — Redesigned per option (b): removed `clock_skew_secs` field from `AuthHandler`, updated R1 to read `state.config.jwt_clock_skew_secs` at validation time. `AuthHandler::new()` signature is unchanged; `websocket.rs` is not touched. Constraints section updated to explicitly prohibit the field approach.
2. [✓] R2 `server_config` may be `None` — Updated R2 startup validation to only panic when `server_config` is `Some` AND `require_auth: true` AND `jwt_secret` is `None`. Observable Truth #4 and AC6 updated to reflect the conditional guard. Existing tests and load harness (which use `server_config: None`) are unaffected.
3. [✓] R4 missing `AUTH_FAIL` send for sub-claim rejection — Updated R4 to explicitly require sending `AUTH_FAIL` to the client channel before returning the error in `auth.rs`. Added AC12 to the acceptance criteria. Clarified that `admin_auth.rs` has no channel (HTTP-based extractor) so no send is needed there.
4. [✓] Validation checklist note about TS generators misleading — Updated checklist item 2 to state that TS helpers already include `expiresIn: '1h'` and `sub`, and that no changes to TS helpers are needed. Updated Assumption 4 to remove the "may need `exp` added" hedge.
5. [✓] Add `exp` field to `JwtClaims` — Added new R7 requiring `pub exp: Option<u64>` on `JwtClaims`. Added to Required Artifacts table. Added to G1 task group.
6. [✓] G2 context estimate tight — Reduced G2 estimate from ~25% to ~20% (alternative design eliminates `AuthHandler::new()` and construction chain work). G5 reduced from ~15% to ~10% (TS helpers need no changes, only Rust unit test helpers). Total remains within ~90%.
7. [✓] `test_server.rs` and load harness bypass `NetworkModule` — Documented as intentional in Assumptions section with explanation of why startup validation is safely skipped.

### Audit v2 (2026-03-23)
**Status:** NEEDS_REVISION

**Context Estimate:** ~45% total (per-worker max ~25%)

**Critical:**
1. **R1 `handle_auth()` has no access to `state.config`.** The spec says `AuthHandler::handle_auth()` "reads `state.config.jwt_clock_skew_secs` directly from the `AppState` reference it already holds at call time." This is incorrect. The current signature is `pub async fn handle_auth(&self, auth_msg: &AuthMessage, tx: &mpsc::Sender<OutboundMessage>)` -- it receives only `&self` (which holds `jwt_secret`), the auth message, and the tx channel. There is no `AppState` parameter. The caller in `websocket.rs` (line 130) does have access to `state`, but `handle_auth` itself does not. Fix: add a `leeway: u64` parameter to `handle_auth()` and pass `state.config.jwt_clock_skew_secs` from the `websocket.rs` call site. This changes `handle_auth`'s signature (not `new()`), requiring a one-line update in `websocket.rs` (line 130). Add `websocket.rs` to Required Artifacts with a note that only the `handle_auth` call site changes (adding one argument). Alternatively, pass `&NetworkConfig` or `&AppState` as a parameter. The constraint prohibiting changes to `AuthHandler::new()` is still honored.

**Recommendations:**
2. **G4 dependency on G2, G3 is unnecessary.** G4 modifies `module.rs` (env var reading, startup validation, TLS warning). None of these changes depend on the auth handler fixes in G2/G3. G4 only needs G1 (for `jwt_clock_skew_secs` to exist in `NetworkConfig`, though G4 does not even read that field). Moving G4 to Wave 2 (depending only on G1) would allow three parallel workers in Wave 2 and reduce total execution time.
3. **G5 is vague about target files.** "Update Rust unit test token generators to include `exp` claims" does not specify which files contain these generators. Investigation shows that `admin.rs` (login handler) and `load_harness/connection_pool.rs` already include `exp` in their token generation. G5 likely means writing NEW unit tests for AC1-AC4, AC8, AC12 inside `auth.rs` and `admin_auth.rs` (which currently have no `#[cfg(test)]` modules). The task description should explicitly state: "Add `#[cfg(test)]` modules to `auth.rs` and `admin_auth.rs` with tests for expired tokens, leeway behavior, and missing-sub rejection."
4. **R3 references `middleware.rs` but it is not listed in Required Artifacts.** R3 says "In `build_cors_layer()` in `middleware.rs`: when `origins` is empty, return a `CorsLayer` that rejects all cross-origin requests." Then it says "The existing logic already handles this correctly." If no change is needed in `middleware.rs`, remove the mention to avoid confusion. If a change IS needed, add `middleware.rs` to Required Artifacts and count it toward the 5-file Language Profile limit.
5. **Observable Truth #2 says "30 seconds in the future" but should say "in the past."** Truth #2 reads: "A JWT token with `exp` 30 seconds in the future (within clock skew) is accepted." A token with `exp` 30 seconds in the future is not expired at all -- it does not test clock skew. AC3 correctly says "30 seconds ago" (in the past). Truth #2 should match: "A JWT token with `exp` 30 seconds in the past (within clock skew leeway of 60s) is accepted."

### Response v2 (2026-03-23)
**Applied:** All critical issues and all recommendations.

**Changes:**
1. [✓] R1 `handle_auth()` lacks `state.config` access — Added `leeway: u64` parameter to `handle_auth()`. R1 updated to specify this signature change. `websocket.rs` added to Required Artifacts noting only the one-line call site change (passing `state.config.jwt_clock_skew_secs`). G2 task description updated to include the `websocket.rs` call site update. Constraints updated to reflect that `AuthHandler::new()` is unchanged but `handle_auth()` gains the `leeway` parameter.
2. [✓] G4 dependency on G2/G3 unnecessary — Moved G4 to Wave 2 (depends only on G1). Execution Plan updated to show 3 parallel workers in Wave 2 (G2, G3, G4). Wave 3 now contains only G5.
3. [✓] G5 vague about target files — Rewrote G5 task description to explicitly state: "Add `#[cfg(test)]` modules to `auth.rs` and `admin_auth.rs` with unit tests for expired tokens, leeway behavior, and missing-sub rejection (AC1-AC4, AC8, AC12)."
4. [✓] R3 references `middleware.rs` unnecessarily — Removed the `build_cors_layer()` paragraph from R3 and replaced with a single clarifying sentence that no change to `middleware.rs` is needed. `middleware.rs` is not listed in Required Artifacts.
5. [✓] Observable Truth #2 wrong direction — Corrected from "30 seconds in the future" to "30 seconds in the past (within clock skew leeway of 60s)".

### Audit v3 (2026-03-23)
**Status:** APPROVED

**Context Estimate:** ~45% total (per-worker max ~25%)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group | ~25% (G4) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Rust Type Mapping Checklist:**
- [x] No `f64` for integer-semantic fields (`jwt_clock_skew_secs: u64`, `exp: Option<u64>`)
- [x] No `r#type: String` on message structs (not applicable)
- [x] `Default` derived on payload structs with 2+ optional fields (not applicable)
- [x] Enums used for known value sets (not applicable)
- [x] Wire compatibility (not applicable -- no new serialization paths)
- [x] `JwtClaims` is deserialization-only with standard JWT field names -- no rename needed
- [x] `Option<T>` fields have appropriate serde attributes

**Goal-Backward Validation:**
- Truth 1 (expired JWT rejected) -> covered by R1, AC1, AC4
- Truth 2 (leeway acceptance) -> covered by R1/R6, AC3
- Truth 3 (env var loading) -> covered by R2, AC5
- Truth 4 (startup validation) -> covered by R2, AC6
- Truth 5 (CORS default) -> covered by R3, AC7
- Truth 6 (sub rejection) -> covered by R4, AC8, AC9, AC12
- Truth 7 (TLS warning) -> covered by R5, AC10
- All truths have artifacts. All artifacts have purposes. No orphans.

**Strategic fit:** Aligned with project goals -- P0 security bugfix required before public deployment.

**Project compliance:** Honors PROJECT.md decisions -- no new deps, Rust type mapping rules followed, Language Profile compliant (5 files exactly at limit).

**Language Profile:** Compliant with Rust profile.
- File count: 5 (at limit of 5)
- Trait-first: G1 contains only types/config changes
- Compilation gate: all task groups modify <=3 files

**Assumptions Validated:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | `jsonwebtoken` crate defaults `required_spec_claims` to include `exp` | Tokens without `exp` would still be accepted -- P0 vuln remains |
| A2 | TS integration helpers already include `exp` and `sub` | Integration tests would fail after enabling exp validation |
| A3 | `AllowOrigin::list()` with empty iterator rejects all origins | CORS would silently allow all origins -- P1 vuln remains |
| A4 | Clock skew of 60s is reasonable | Too tight = false rejections; too loose = larger attack window |

All assumptions are reasonable and verifiable at implementation time.

**Comment:** Well-structured security bugfix spec after two revision rounds. All previous critical issues have been resolved. Requirements are precise with exact line numbers and code patterns. Acceptance criteria are measurable and comprehensive (12 ACs covering all 7 observable truths). Task groups follow trait-first ordering with correct dependency graph and parallel execution plan. The `websocket.rs` path has been corrected from `network/websocket.rs` to `network/handlers/websocket.rs` in the Required Artifacts table (fix applied during this audit). Ready for implementation.

**Recommendations:**
1. R7 should specify adding `#[serde(default)]` on the `exp: Option<u64>` field to match the existing pattern on `roles`. While the `jsonwebtoken` crate validates `exp` from raw JSON before struct deserialization, adding `default` ensures consistent serde behavior across all `Option<T>` fields in `JwtClaims` and prevents deserialization errors if the struct is ever used outside the `jsonwebtoken` decode path.

### Response v3 (2026-03-23)
**Applied:** Recommendation R7 (serde default on exp field).

**Changes:**
1. [✓] R7 `#[serde(default)]` on `exp` field — Updated R7 requirement text to specify `#[serde(default)] pub exp: Option<u64>` with explanation that this matches the existing pattern on `roles` and ensures consistent serde behavior across all `Option<T>` fields in `JwtClaims`.

### Audit v4 (2026-03-23)
**Status:** APPROVED

**Context Estimate:** ~45% total (per-worker max ~25%)

**Source code verified:** All spec claims cross-checked against current source files. Line numbers, function signatures, field names, and code patterns are accurate.

**Verified against source:**
- `auth.rs:102` -- `validation.required_spec_claims.clear()` confirmed
- `auth.rs:111` -- `.unwrap_or_else(|| "anonymous".to_string())` confirmed
- `admin_auth.rs:93` -- `validation.required_spec_claims.clear()` confirmed
- `admin_auth.rs:103` -- `.unwrap_or_else(|| "anonymous".to_string())` confirmed
- `config.rs:30` -- `cors_origins: vec!["*".to_string()]` confirmed
- `module.rs:239` -- `jwt_secret: None` confirmed
- `websocket.rs:130` -- `auth_handler.handle_auth(auth_msg, &handle.tx).await` confirmed as one-line call site
- `AdminClaims::from_request_parts` receives `state: &AppState` -- can read `state.config.jwt_clock_skew_secs`
- `AppState.config` is `Arc<NetworkConfig>` -- adding `jwt_clock_skew_secs` makes it accessible everywhere
- `build_cors_layer` with empty vec hits the `else` branch producing `AllowOrigin::list([])` -- correctly rejects all origins
- TS helpers at `tests/integration-rust/helpers/test-client.ts:33-36` already include `sub` and `expiresIn: '1h'`

**Comment:** Spec is clean and ready for implementation. All previous audit findings have been resolved. The v3 recommendation about `#[serde(default)]` has been applied to R7. No new issues found.

## Execution Summary

**Executed:** 2026-03-23
**Mode:** orchestrated
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3, G4 | complete |
| 3 | G5 | complete |

### Files Modified

- `packages/server-rust/src/network/config.rs` — added `jwt_clock_skew_secs: u64` field, changed `cors_origins` default to `vec![]`, updated test
- `packages/server-rust/src/network/handlers/auth.rs` — added `exp: Option<u64>` to `JwtClaims`, re-enabled exp validation, added `leeway` param to `handle_auth()`, reject missing sub with AUTH_FAIL, added 5 unit tests
- `packages/server-rust/src/network/handlers/admin_auth.rs` — removed `required_spec_claims.clear()`, added leeway from config, reject missing sub, added 5 unit tests
- `packages/server-rust/src/network/module.rs` — read `JWT_SECRET` env var, conditional startup assertion, TLS warning
- `packages/server-rust/src/network/handlers/websocket.rs` — pass `state.config.jwt_clock_skew_secs` to `handle_auth()`

### Acceptance Criteria Status

- [x] AC1: JWT with exp 1 hour ago rejected
- [x] AC2: JWT with exp 1 hour from now accepted
- [x] AC3: JWT with exp 30 seconds ago accepted at 60s leeway
- [x] AC4: JWT with exp 90 seconds ago rejected at 60s leeway
- [x] AC5: NetworkModule reads JWT_SECRET env var
- [x] AC6: Panics when require_auth=true and JWT_SECRET absent; skips when server_config=None
- [x] AC7: NetworkConfig::default().cors_origins is empty Vec
- [x] AC8: JWT without sub rejected in auth.rs and admin_auth.rs
- [x] AC9: Admin auth rejects missing sub with HTTP 401
- [x] AC10: TLS warning emitted when jwt_secret=Some and tls=None
- [x] AC11: All 603 server tests pass (up from 593; 10 new unit tests added)
- [x] AC12: AUTH_FAIL sent before returning sub-missing error

### Deviations

None.

---

## Review History

### Review v1 (2026-03-23 17:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1 — `expired_token_rejected` test in `auth.rs` verifies 1-hour-old token is rejected; exp validation enabled by removing `required_spec_claims.clear()`
- [✓] AC2 — `valid_token_accepted` test verifies token with exp 1 hour from now is accepted
- [✓] AC3 — `token_within_leeway_accepted` test verifies 30s-expired token accepted at 60s leeway
- [✓] AC4 — `token_beyond_leeway_rejected` test verifies 90s-expired token rejected at 60s leeway
- [✓] AC5 — `build_app()` in `module.rs:233-235` reads `JWT_SECRET` env var with `filter(|s| !s.is_empty())`
- [✓] AC6 — Conditional assert at `module.rs:241-247`: only panics when `server_config` is `Some` AND `require_auth` AND `jwt_secret` is `None`
- [✓] AC7 — `config.rs:35` sets `cors_origins: vec![]`; test at line 94 asserts `is_empty()`
- [✓] AC8 — Both `auth.rs:123-134` and `admin_auth.rs:103-106` reject `None` sub with explicit error
- [✓] AC9 — `admin_auth.rs` returns `AdminAuthError::InvalidToken` which maps to HTTP 401 via `IntoResponse`; test `missing_sub_rejected` confirms
- [✓] AC10 — TLS warning at `module.rs:251-257` fires when `jwt_secret.is_some() && config.tls.is_none()`
- [✓] AC11 — 603 server tests pass (verified by running cargo test)
- [✓] AC12 — `auth.rs:125-130` sends `AUTH_FAIL` via channel before returning error when sub is `None`
- [✓] R7 `exp` field — `#[serde(default)] pub exp: Option<u64>` added to `JwtClaims` at line 46-47
- [✓] Build check — `cargo check -p topgun-server` exits 0
- [✓] Lint check — `cargo clippy -p topgun-server -- -D warnings` exits 0 with no warnings
- [✓] Test check — 603 tests pass, 0 failures
- [✓] No new crate dependencies added
- [✓] HS256 algorithm unchanged
- [✓] `AuthHandler::new()` signature unchanged; only `handle_auth()` gained `leeway: u64` parameter
- [✓] `websocket.rs:130` passes `state.config.jwt_clock_skew_secs` at call site — one-line change as specified
- [✓] `admin_auth.rs:94` reads `state.config.jwt_clock_skew_secs` directly — no channel send on sub rejection
- [✓] No spec/bug references in code comments — WHY-comments used throughout
- [✓] File count: 5 files modified (exactly at Language Profile limit)
- [✓] Trait-first ordering honored (G1 was types/config only)
- [✓] `anonymous` fallback completely removed from both auth handlers

**Minor:**
1. `JwtClaims.sub: Option<String>` (line 38) has no `#[serde(default)]` attribute while `roles` and `exp` both have it. Since `JwtClaims` only derives `Deserialize` (not `Serialize`), `skip_serializing_if` is irrelevant, and serde implicitly treats `Option<T>` fields as optional during deserialization — so this is functionally correct. Consistency with `roles` and `exp` is slightly off but causes no issue in practice.

**Summary:** All 12 acceptance criteria are met. Build, lint, and test checks pass. The implementation is clean, well-commented with WHY-reasoning, and follows established patterns. Security vulnerabilities are correctly addressed. The one minor inconsistency (missing `#[serde(default)]` on `sub`) is cosmetic and functionally harmless.

### Fix Response v1 (2026-03-23)
**Applied:** Minor issue from Review v1

**Fixes:**
1. [✓] Added `#[serde(default)]` to `JwtClaims.sub: Option<String>` for consistency with `roles` and `exp` fields
   - Commit: bb07139

---

### Review v2 (2026-03-23)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix applied — `#[serde(default)]` now present on `JwtClaims.sub` at `auth.rs:38`, matching the pattern on `roles` (line 41) and `exp` (line 47). All three `Option<T>` fields in `JwtClaims` are now consistent.
- [✓] AC1 — `expired_token_rejected` test passes; exp validation active (`required_spec_claims` not cleared)
- [✓] AC2 — `valid_token_accepted` test passes
- [✓] AC3 — `token_within_leeway_accepted` test passes (30s-expired token with 60s leeway)
- [✓] AC4 — `token_beyond_leeway_rejected` test passes (90s-expired token with 60s leeway)
- [✓] AC5 — `module.rs:233-235` reads `JWT_SECRET` env var with `.filter(|s| !s.is_empty())`
- [✓] AC6 — `module.rs:241-247` asserts only when `server_config` is `Some` AND `require_auth` AND `jwt_secret` is `None`
- [✓] AC7 — `config.rs:35` sets `cors_origins: vec![]`; test asserts `is_empty()`
- [✓] AC8 — `auth.rs:124-135` uses `let Some(user_id) = token_data.claims.sub else { ... }` pattern; `admin_auth.rs:103-106` uses `.ok_or_else(...)` pattern
- [✓] AC9 — `AdminAuthError::InvalidToken` maps to HTTP 401 via `IntoResponse`
- [✓] AC10 — TLS warning at `module.rs:251-257`
- [✓] AC11 — 603 tests pass, 0 failures
- [✓] AC12 — `auth.rs:126-131` sends `AUTH_FAIL` before returning error when `sub` is `None`
- [✓] Build check — `cargo check -p topgun-server` exits 0
- [✓] Lint check — `cargo clippy -p topgun-server -- -D warnings` exits 0 with no warnings
- [✓] Test check — 603 tests pass, 0 failures (all 10 new auth unit tests pass)
- [✓] No residual `required_spec_claims.clear()` or `unwrap_or_else(|| "anonymous"...)` in codebase
- [✓] No residual wildcard `cors_origins: vec!["*"...]` in codebase
- [✓] `jwt_secret: None` references in health/metrics/http_sync handlers are confined to `#[cfg(test)]` modules — correct test infrastructure behavior
- [✓] WHY-comments throughout; no spec/bug references in code

**Summary:** The minor issue from Review v1 (missing `#[serde(default)]` on `JwtClaims.sub`) has been correctly applied in commit bb07139. All 12 acceptance criteria remain satisfied. Build, lint, and all 603 tests pass. The implementation is complete and clean.

---

## Completion

**Completed:** 2026-03-23
**Total Commits:** 4
**Review Cycles:** 2

### Outcome

Fixed five critical auth vulnerabilities in the Rust server: re-enabled JWT expiry validation with configurable clock skew leeway, loaded JWT secret from environment with startup validation, changed CORS default to empty allowlist, rejected tokens missing `sub` claim, and added TLS warning when JWT auth runs without encryption.

### Key Files

- `packages/server-rust/src/network/handlers/auth.rs` — JWT exp validation, leeway parameter, sub rejection with AUTH_FAIL, 5 unit tests
- `packages/server-rust/src/network/handlers/admin_auth.rs` — Matching exp/sub fixes for admin HTTP auth extractor, 5 unit tests
- `packages/server-rust/src/network/module.rs` — JWT_SECRET env var loading, conditional startup assertion, TLS warning
- `packages/server-rust/src/network/config.rs` — `jwt_clock_skew_secs` field, empty CORS default
- `packages/server-rust/src/network/handlers/websocket.rs` — Pass clock skew to handle_auth() call site

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
