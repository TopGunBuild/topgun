---
id: SPEC-138
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-03-23
source: TODO-169
delta: true
---

# SPEC-138: Add RSA JWT Algorithm Auto-Detection (RS256/RS384/RS512)

## Context

The Rust server hardcodes `Algorithm::HS256` + `DecodingKey::from_secret()` in both `auth.rs` (WebSocket auth) and `admin_auth.rs` (HTTP admin auth). The deleted TS server had auto-detection: if the JWT secret started with `-----BEGIN`, it used RS256 with PEM decoding; otherwise HS256 with raw secret bytes.

Without RSA support, all Clerk, Auth0, and Firebase integrations are broken. The documentation at `apps/docs-astro/src/content/docs/guides/authentication.mdx` already promises RS256 works. Helper scripts (`scripts/get-clerk-key.js`, `start-clerk-server.sh`) and `examples/notes-app` (Clerk) exist but cannot function until this is fixed.

The `jsonwebtoken` v9 crate already supports `DecodingKey::from_rsa_pem()` and `Algorithm::RS256/RS384/RS512`. No new dependencies are needed.

## Delta

### MODIFIED
- `packages/server-rust/src/network/handlers/auth.rs` — Extract shared key-detection helper, use RSA decoding when PEM detected
  - Add `normalize_pem()` function: replace escaped `\\n` with real `\n` in PEM strings from Docker env vars
  - Add `decode_jwt_key()` function: return `(Algorithm, DecodingKey)` based on key format detection (`-----BEGIN` prefix = RS256, else HS256)
  - `AuthHandler::handle_auth()`: call `decode_jwt_key()` instead of hardcoded HS256
  - Add unit tests for RS256 acceptance, RS256 rejection, auto-detection, and PEM newline normalization
- `packages/server-rust/src/network/handlers/admin_auth.rs` — Use same key-detection logic for admin JWT validation
  - `AdminClaims::from_request_parts()`: call `decode_jwt_key()` from `auth.rs` instead of hardcoded HS256
  - Add unit test for RS256 admin token acceptance

## Requirements

### R1: PEM Normalization Helper (`auth.rs`)

Add a `pub(crate) fn normalize_pem(input: &str) -> String` function:
- Replace literal `\\n` (two-char sequence backslash + n) with actual newline `\n`
- This handles Docker/k8s environment variables where PEM newlines are escaped
- Return the normalized string (owned, since replacement may occur)

### R2: JWT Key Detection Helper (`auth.rs`)

Add a `pub(crate) fn decode_jwt_key(secret: &str) -> Result<(Algorithm, DecodingKey), String>` function:
- Call `normalize_pem()` on the input first
- If normalized string starts with `"-----BEGIN"`:
  - Call `DecodingKey::from_rsa_pem(normalized.as_bytes())`
  - On success, return `(Algorithm::RS256, key)`
  - On failure, return `Err` with the PEM parsing error message
- Otherwise:
  - Return `(Algorithm::HS256, DecodingKey::from_secret(secret.as_bytes()))`

Note: RS256 is the default RSA algorithm because Clerk, Auth0, and Firebase all use RS256. The `jsonwebtoken` crate's `Validation` with `Algorithm::RS256` also accepts RS384/RS512 if `validation.algorithms` is extended, but for auto-detection RS256 is the correct default. Users needing RS384/RS512 can be supported in a future iteration via explicit algorithm config.

### R3: Update `AuthHandler::handle_auth()` (`auth.rs`)

Replace:
```rust
let mut validation = Validation::new(Algorithm::HS256);
// ...
let key = DecodingKey::from_secret(self.jwt_secret.as_bytes());
```

With:
```rust
let (algorithm, key) = decode_jwt_key(&self.jwt_secret)
    .map_err(|reason| AuthError::InvalidToken { reason })?;
let mut validation = Validation::new(algorithm);
```

All other validation settings (leeway, validate_aud, required_spec_claims) remain unchanged.

### R4: Update `AdminClaims::from_request_parts()` (`admin_auth.rs`)

Replace:
```rust
let mut validation = Validation::new(Algorithm::HS256);
// ...
let key = DecodingKey::from_secret(jwt_secret.as_bytes());
```

With:
```rust
use super::auth::decode_jwt_key;

let (algorithm, key) = decode_jwt_key(jwt_secret)
    .map_err(|e| AdminAuthError::InvalidToken(e))?;
let mut validation = Validation::new(algorithm);
```

### R5: Unit Tests (`auth.rs`)

Add the following tests to the existing `#[cfg(test)] mod tests` block:

1. **`rs256_valid_token_accepted`**: Generate an RSA key pair with `jsonwebtoken::EncodingKey::from_rsa_pem()`, create a token with RS256 header, verify `handle_auth` accepts it and returns correct Principal
2. **`rs256_invalid_signature_rejected`**: Sign a token with one RSA key, verify with a different RSA public key -- must return `AuthError::InvalidToken`
3. **`pem_key_detected_as_rsa`**: Call `decode_jwt_key` with a PEM string, assert the returned algorithm is `Algorithm::RS256`
4. **`non_pem_key_detected_as_hmac`**: Call `decode_jwt_key` with a plain string like `"my-secret"`, assert the returned algorithm is `Algorithm::HS256`
5. **`normalize_pem_replaces_escaped_newlines`**: Verify `normalize_pem("-----BEGIN PUBLIC KEY-----\\nABC\\n-----END PUBLIC KEY-----")` contains real newlines, not escaped ones

### R6: Unit Test (`admin_auth.rs`)

Add one test to the existing `#[cfg(test)] mod tests` block:

1. **`rs256_admin_token_accepted`**: Generate RSA key pair, create admin token with RS256 + `sub` + `roles: ["admin"]`, set `jwt_secret` in `test_state` to the PEM public key, verify `AdminClaims::from_request_parts` returns `Ok` with correct `user_id`

## Acceptance Criteria

1. A JWT signed with RS256 and a valid RSA private key is accepted by `AuthHandler::handle_auth()` when the server's `jwt_secret` is the corresponding RSA public key in PEM format
2. A JWT signed with RS256 is rejected when verified against a different RSA public key
3. A JWT signed with HS256 continues to work exactly as before (no regression)
4. PEM keys with escaped newlines (`\\n`) from Docker env vars are correctly normalized before use
5. `admin_auth.rs` uses the same auto-detection logic as `auth.rs` (no code duplication of detection logic)
6. All existing auth tests continue to pass unchanged
7. New RS256 tests pass: at least 5 new tests in `auth.rs`, 1 in `admin_auth.rs`

## Constraints

- Do NOT add a new configuration field for algorithm selection; auto-detection from key format is sufficient
- Do NOT add new crate dependencies; `jsonwebtoken` v9 already supports RSA
- Do NOT modify `JwtClaims` struct or any message types
- Do NOT change the wire protocol or any public API
- Keep `decode_jwt_key` as `pub(crate)` -- not public API, only shared between auth modules
- RS256 is the auto-detected RSA algorithm; do NOT attempt to detect RS384/RS512 from PEM format (PEM keys are algorithm-agnostic)

## Assumptions

- RSA test keys can be generated at compile time or embedded as test constants (the `jsonwebtoken` crate examples show this pattern)
- The `jsonwebtoken` v9 crate's `DecodingKey::from_rsa_pem()` accepts standard PKCS#1 and PKCS#8 PEM formats
- No ECDSA (ES256) support is needed at this time; only RSA and HMAC
- The `examples/notes-app` Clerk integration test is manual/out-of-scope for this spec's automated tests

## Audit History

### Audit v1 (2026-03-23)
**Status:** APPROVED

**Context Estimate:** ~16% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~16% | <=50% | OK |
| Largest file (auth.rs) | ~6% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Delta validation:** 2/2 entries valid

**Strategic fit:** Aligned with project goals -- unblocks documented Clerk/Auth0/Firebase integrations

**Project compliance:** Honors PROJECT.md decisions (no new deps, no message changes, Rust patterns followed)

**Language profile:** Compliant with Rust profile (2 files, within max 5)

**Recommendations:**
1. Consider using a dedicated error type instead of `String` in `decode_jwt_key`'s return type for better error handling ergonomics. Not blocking since the function is `pub(crate)` and has only two call sites.

**Comment:** Well-structured spec with clear before/after code snippets, specific test requirements, and well-scoped constraints. The delta is minimal and precisely targeted. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-23
**Commits:** 2

### Files Created
None.

### Files Modified
- `packages/server-rust/src/network/handlers/auth.rs` — Added `normalize_pem()`, `decode_jwt_key()` helpers; updated `handle_auth()` to use auto-detection; added 5 RSA unit tests with real PKCS#8 key constants
- `packages/server-rust/src/network/handlers/admin_auth.rs` — Replaced hardcoded HS256 with `super::auth::decode_jwt_key()`; removed unused `Algorithm`/`DecodingKey` imports; added `rs256_admin_token_accepted` test

### Files Deleted
None.

### Acceptance Criteria Status
- [x] A JWT signed with RS256 and valid RSA private key is accepted when `jwt_secret` is the corresponding RSA public key PEM
- [x] A JWT signed with RS256 is rejected when verified against a different RSA public key
- [x] HS256 tokens continue to work exactly as before (all prior tests pass)
- [x] PEM keys with escaped newlines (`\\n`) from Docker env vars are correctly normalized
- [x] `admin_auth.rs` uses the same auto-detection via `super::auth::decode_jwt_key()` — no code duplication
- [x] All existing auth tests continue to pass (603 → 609 tests, 0 failures)
- [x] 5 new tests in `auth.rs` + 1 in `admin_auth.rs`

### Deviations
- [Rule 1 - Bug] Embedded RSA key pair constants initially used invented/fictional key material that failed `InvalidKeyFormat`. Replaced with two real RSA 2048-bit key pairs generated via `openssl genrsa` + `pkcs8 -nocrypt`. The unused `TEST_RSA2_PRIVATE_PEM` constant was removed after discovery that only the public key is needed for the mismatch rejection test.

### Notes
- Real RSA test keys are embedded as `const &str` in test modules (PKCS#8 PEM format). They are test-only, 2048-bit, generated fresh, and have no security significance.
- The `decode_jwt_key` function signature uses `String` for the error type as specified (not a dedicated error type). The audit recommendation for a dedicated error type is noted but deferred.
- 609 server tests pass total (up from 603).

---

## Review History

### Review v1 (2026-03-23)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] R1: `normalize_pem()` implemented as `pub(crate)`, replaces `\\n` with real newlines — matches spec exactly
- [✓] R2: `decode_jwt_key()` implemented as `pub(crate) fn decode_jwt_key(secret: &str) -> Result<(Algorithm, DecodingKey), String>` — calls `normalize_pem()` first, branches on `-----BEGIN` prefix, returns RS256/HS256 as required
- [✓] R3: `handle_auth()` uses `decode_jwt_key()` — all other validation settings (leeway, validate_aud, required_spec_claims) unchanged
- [✓] R4: `admin_auth.rs` calls `super::auth::decode_jwt_key()` at line 92 — no logic duplication, unused `Algorithm`/`DecodingKey` imports removed
- [✓] R5: All 5 required tests present in `auth.rs`: `rs256_valid_token_accepted`, `rs256_invalid_signature_rejected`, `pem_key_detected_as_rsa`, `non_pem_key_detected_as_hmac`, `normalize_pem_replaces_escaped_newlines`
- [✓] R6: `rs256_admin_token_accepted` test present in `admin_auth.rs`, constructs `test_state_rsa` with PEM public key and verifies correct `user_id`
- [✓] All 7 acceptance criteria met (confirmed by 609/0 test result)
- [✓] No new crate dependencies added
- [✓] `JwtClaims` struct not modified
- [✓] Wire protocol unchanged
- [✓] `decode_jwt_key` visibility is `pub(crate)` — not public API
- [✓] Build check: passed (`cargo check -p topgun-server` exits 0)
- [✓] Lint check: passed (`cargo clippy -- -D warnings` exits 0, 0 warnings)
- [✓] Test check: passed (609 tests, 0 failures)
- [✓] Rust idioms: `?` operator used throughout, no `.unwrap()` in production code, no unnecessary clones, error propagation via `map_err`
- [✓] RSA test key constants are real PKCS#8 2048-bit keys (not fictional), verified by successful test execution

**Minor:**
1. Stale doc comment on `AuthHandler` struct at `auth.rs:81` still reads "JWT verification uses HS256 algorithm with the configured secret." — should say "auto-detects RS256 or HS256 based on key format." This is cosmetic only.
2. RSA test key constants (`TEST_RSA_PRIVATE_PEM`, `TEST_RSA_PUBLIC_PEM`) are duplicated verbatim between `auth.rs` and `admin_auth.rs` test modules. Since both are `#[cfg(test)]` and in different files, this is acceptable Rust practice (test helpers are not shared across modules), but worth noting.

**Summary:** Implementation fully satisfies all requirements and acceptance criteria. Both modified files are clean, the shared helper eliminates code duplication in production paths, and all 609 tests pass with clippy-clean code. Two cosmetic minor observations, neither requiring action before `/sf:done`.

---

## Completion

**Completed:** 2026-03-23
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Added RSA JWT auto-detection to the Rust server, enabling Clerk, Auth0, and Firebase integrations that were broken since the TS server removal. The server now automatically detects PEM-formatted RSA public keys and uses RS256 validation, with HS256 as the default fallback.

### Key Files

- `packages/server-rust/src/network/handlers/auth.rs` — Shared `normalize_pem()` and `decode_jwt_key()` helpers; updated WebSocket auth handler
- `packages/server-rust/src/network/handlers/admin_auth.rs` — Updated admin HTTP auth to reuse shared key detection

### Changes Applied

**Modified:**
- `packages/server-rust/src/network/handlers/auth.rs` — Added `normalize_pem()`, `decode_jwt_key()` pub(crate) helpers; `handle_auth()` uses auto-detection instead of hardcoded HS256; 5 new RSA unit tests
- `packages/server-rust/src/network/handlers/admin_auth.rs` — `AdminClaims::from_request_parts()` calls shared `decode_jwt_key()` instead of hardcoded HS256; 1 new RSA unit test

### Patterns Established

None — followed existing patterns.

### Spec Deviations

- Embedded RSA key pair constants initially used fictional key material; replaced with real 2048-bit PKCS#8 keys generated via openssl (Rule 1 - Bug).
