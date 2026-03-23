---
id: SPEC-139
type: bugfix
status: approved
priority: P2
complexity: medium
created: 2026-03-23
source: TODO-170
delta: true
---

# Fix Auth & Security Documentation (Outdated Claims, Missing Guidance)

## Context

Auth and security documentation contains errors and undocumented gaps introduced during the Rust migration. Three pages are affected: `authentication.mdx`, `security.mdx`, and `rbac.mdx`. These docs cause real integration failures (e.g., developer sends JWT with only `userId` claim, gets `AUTH_FAIL` with no explanation) and create false security expectations (e.g., TLS env vars documented but not implemented).

Key background:
- TODO-107 resolved: JWT uses only standard `sub` claim. No `userId` field in Rust `JwtClaims`.
- SPEC-138 completed: RS256 auto-detection via `decode_jwt_key()` and `normalize_pem()` is implemented and working.
- SPEC-137 completed: JWT `exp` validation re-enabled, `JWT_SECRET` read from env, CORS defaults tightened.
- No production binary exists. Only `test-server` binary with hardcoded `tls: None`.
- Cluster TLS (mTLS) is not implemented. `ClusterConfig` has no TLS fields.
- RBAC role evaluation for data access is not implemented. Only basic map-level `read`/`write` booleans per connection work.

## Delta

### MODIFIED
- `apps/docs-astro/src/content/docs/guides/authentication.mdx` — Fix incorrect JWT claims, replace static token pattern, add token lifecycle section, verify RS256 docs, complete Better Auth bridge
  - Lines 149-155: Remove `userId` claim from JWT payload example; document only `sub` (RFC 7519)
  - Lines 157-171: Remove `userId` from `generateToken()` function
  - Lines 188-210: Replace `setAuthToken()` + `localStorage` with `setAuthTokenProvider()` pattern including token refresh callback
  - Lines 139-147: Update `serverConfigCode` export — remove direct reference to `topgun-server` binary; replace with `test-server` / programmatic embedding guidance consistent with AC10
  - Add new "Token Lifecycle" section explaining expiry behavior
  - Lines 424-425: Verify RS256 auto-detection claim matches SPEC-138 implementation (it does — keep as-is)
  - Better Auth section: Add bridge example showing how to get JWT from BetterAuth session and pass to TopGun via `setAuthTokenProvider()`
- `apps/docs-astro/src/content/docs/guides/security.mdx` — Fix phantom TLS env vars, remove unimplemented mTLS section, document working env vars, document trusted-origin bypass
  - TLS env vars table: Mark all `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` vars as "Planned" with clear callout that they do not exist in current server
  - mTLS section: Add prominent "Planned for v3.0" banner; keep content as forward-looking but clearly mark as not yet implemented
  - Add "Working Environment Variables" section documenting: `TOPGUN_ADMIN_PASSWORD` (required), `TOPGUN_ADMIN_USERNAME` (default "admin"), `TOPGUN_ADMIN_DIR` (admin SPA path), `TOPGUN_LOG_FORMAT` ("json" for structured logging), `JWT_SECRET`, `DATABASE_URL`, `PORT`
  - Add "Security Pipeline Details" subsection documenting the trusted-origin bypass (`Forwarded`, `Backup`, `Wan`, `System` sources skip `WriteValidator` checks)
  - Clarify server deployment model: library crate requiring programmatic embedding; `test-server` is for development only
- `apps/docs-astro/src/content/docs/guides/rbac.mdx` — Rewrite to document only what exists, mark unimplemented features as planned
  - Add prominent "Current Status" banner at top explaining what works vs what is planned
  - Keep role extraction from JWT as documented (this works)
  - Mark map pattern matching (`users:*`), field-level security (`allowedFields`), and role-based policy evaluation as "Planned — see TODO-171"
  - Document what actually works: basic per-connection map-level `read`/`write` booleans

## Requirements

### File 1: `apps/docs-astro/src/content/docs/guides/authentication.mdx`

**R1: Fix JWT payload example (lines 149-155)**
Replace the `jwtPayloadCode` export. Remove `"userId"` claim entirely. The payload must show only:
- `sub` (required) — user identifier per RFC 7519
- `roles` (optional) — array of role strings
- `iat` — issued at
- `exp` — expiration

Remove the comment "or use userId" and "Alternative to sub".

**R2: Fix custom JWT generation (lines 157-171)**
In `customJwtCode`, remove `userId: user.id` from the `jwt.sign()` payload. Only `sub`, `roles`, and standard claims remain.

**R3: Replace static token pattern with token provider (lines 188-210)**
Replace `clientAuthCode` export. The new code must:
1. Use `tgClient.setAuthTokenProvider(async () => { ... })` instead of `setAuthToken()`
2. The provider callback calls `/api/login` or returns a cached token, refreshing when needed
3. The cache must use an in-memory variable (e.g., `let cachedToken: string | null = null`) — not `localStorage` or any browser storage API. Include a comment in the example making this explicit (e.g., `// In-memory cache — not persisted to storage`)
4. Remove `localStorage.setItem/getItem` pattern for raw token storage
5. Add a comment explaining the provider is called on every `AUTH_REQUIRED` message

**R4: Add "Token Lifecycle" section**
Insert a new section after "Custom JWT Provider" and before "Production Deployment with Clerk". Content must explain:
1. Active WebSocket connections are NOT terminated when a token expires
2. Token expiry causes failure only on reconnect (network drop, page reload)
3. `setAuthTokenProvider()` is called on every `AUTH_REQUIRED` message from the server
4. If the provider returns `null`, the connection remains unauthenticated
5. Recommendation: token provider should call the app's refresh endpoint

**R5: RS256 auto-detection (lines 424-425)**
Verify the callout matches implementation. SPEC-138 confirmed: `decode_jwt_key()` detects `-----BEGIN` prefix and selects RS256. The current documentation text is accurate. No change needed unless wording is misleading.

**R6: Better Auth bridge**
Add a subsection after "3. Use in Your App" in the Better Auth section. Show:
1. How to obtain a JWT from a BetterAuth session and pass it to TopGun via `setAuthTokenProvider()`
2. A note that BetterAuth issues session tokens (cookies/opaque tokens) by default — not JWTs. To use TopGun with BetterAuth, developers must create a custom server endpoint (e.g., `GET /api/topgun-token`) that verifies the BetterAuth session and mints a signed JWT for TopGun consumption. The bridge example must reflect this two-step pattern: session → custom endpoint → JWT → `setAuthTokenProvider()`
3. Brief note that BetterAuth manages sessions/users, but TopGun needs a JWT for sync auth

**R11 (extended): Clarify server deployment model — also covers `authentication.mdx`**
In addition to `security.mdx` (original R11 scope), also update the `serverConfigCode` export at lines 139-147 of `authentication.mdx`. This code block must not imply a working `topgun-server` production binary. Replace with `test-server` usage for development or a programmatic embedding example for production. Add a comment or callout that the standalone `topgun-server` production binary is planned but does not yet exist.

### File 2: `apps/docs-astro/src/content/docs/guides/security.mdx`

**R7: Mark TLS env vars as planned**
The env vars table (lines 424-437) and the server TLS code block (lines 11-23) document `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` variables that do not exist in the Rust server. Add a prominent yellow callout box before the table stating: "The TLS environment variables below are planned for a future production binary. Currently, TLS is configured programmatically via the `TlsConfig` struct when embedding the server." (Note: a partial version of this callout already exists at lines 418-419 — ensure it is clear and prominent, not easily missed.)

**R8: Mark mTLS cluster section as planned**
The "mTLS for Cluster Communication" section (lines 372-409) describes features that do not exist. `ClusterConfig` has no TLS fields; cluster traffic is plaintext TCP. Add a prominent banner at the top of this section: "Planned for v3.0. Cluster communication currently uses plaintext TCP. See the roadmap for cluster TLS progress." Keep the content as forward-looking design documentation.

**R9: Add working environment variables section**
Add a new section "Server Environment Variables" (or rename the existing "Environment Variables Reference"). Document all env vars that actually work:

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `PORT` | No | `8090` | HTTP/WebSocket listen port |
| `DATABASE_URL` | Yes | - | PostgreSQL connection string |
| `JWT_SECRET` | Yes | - | HMAC secret or RSA public key (PEM) for JWT verification |
| `TOPGUN_ADMIN_PASSWORD` | Yes (for admin) | - | Password for admin API login |
| `TOPGUN_ADMIN_USERNAME` | No | `admin` | Username for admin API login |
| `TOPGUN_ADMIN_DIR` | No | - | Path to admin SPA static files |
| `TOPGUN_LOG_FORMAT` | No | human-readable | Set to `json` for structured JSON logging |

**R10: Document trusted-origin bypass**
Add a subsection under "Security Pipeline" explaining that operations from trusted origins (`Forwarded`, `Backup`, `Wan`, `System` caller origins) bypass `WriteValidator` checks entirely. This is by design for internal server-to-server operations (replication, backup restore, cluster forwarding). External client connections always have `CallerOrigin::Client` and pass through all security checks.

**R11: Clarify server deployment model**
Update references to `topgun-server` binary in `security.mdx`. The server is a library crate (`topgun-server`). The only binary is `test-server` (development/testing). For production, developers embed the server programmatically in their Rust application. The `topgun-server` production binary with env-var configuration is planned but does not exist yet.

### File 3: `apps/docs-astro/src/content/docs/guides/rbac.mdx`

**R12: Add implementation status banner**
Add a prominent callout at the top of the page (after the title) clearly stating:

"Current implementation status: Roles are extracted from JWT `roles` claim and attached to the connection principal. Basic per-connection, per-map `read`/`write` permissions work. Role-based policy evaluation, map pattern matching (`users:*`), and field-level security (`allowedFields`) are planned for a future release (TODO-171)."

**R13: Mark unimplemented features**
Add "Planned" badges or callout boxes to:
- The "Configuration" section showing policy structures (map pattern matching is not implemented)
- The "Field-Level Security" section (`allowedFields` is not implemented)
- Any references to `topgun-server` binary

**R14: Document what works**
Add or update content to describe the current working RBAC:
- JWT `roles` claim is extracted and stored on the connection principal
- Per-connection map permissions (`read: true/false`, `write: true/false`) are enforced
- Default-deny: connections without explicit permissions cannot access maps
- `WriteValidator` enforces these permissions in the security pipeline

## Acceptance Criteria

1. **No `userId` in JWT docs:** The string `userId` does not appear as a JWT claim anywhere in `authentication.mdx`
2. **Token provider pattern:** `authentication.mdx` shows `setAuthTokenProvider()` (not `setAuthToken()`) in the custom JWT section
3. **Token lifecycle documented:** A "Token Lifecycle" section exists explaining expiry-on-reconnect behavior
4. **Better Auth bridge:** The Better Auth section shows how to pass a session token to `setAuthTokenProvider()`, and includes a note that BetterAuth uses session tokens by default requiring a custom JWT-minting endpoint
5. **TLS env vars marked planned:** Every `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` reference has a visible "planned" or "not yet implemented" indicator
6. **mTLS section marked planned:** The cluster mTLS section has a prominent v3.0 planned banner
7. **Working env vars documented:** `TOPGUN_ADMIN_PASSWORD`, `TOPGUN_ADMIN_USERNAME`, `TOPGUN_ADMIN_DIR`, `TOPGUN_LOG_FORMAT` appear in a documented table in `security.mdx`
8. **Trusted-origin bypass documented:** Security pipeline section mentions the `Forwarded`/`Backup`/`Wan`/`System` bypass
9. **RBAC status clear:** `rbac.mdx` has a visible banner distinguishing implemented vs planned features
10. **No `topgun-server` binary claims:** No documentation in any of the three files implies a working `topgun-server` production binary without a "planned" qualifier — including the `serverConfigCode` export in `authentication.mdx`
11. **In-memory token cache:** The token provider example in `authentication.mdx` uses an explicit in-memory variable for caching (not `localStorage`), with a comment making the in-memory nature clear

## Validation Checklist

1. Search all three files for `"userId"` as a JWT claim — zero matches expected
2. Search `authentication.mdx` for `setAuthToken(` without `Provider` — zero matches expected (only `setAuthTokenProvider` should appear in examples)
3. Search `security.mdx` for `TOPGUN_ADMIN_PASSWORD` — at least one match confirming documentation
4. Visually inspect `rbac.mdx` — planned features have visible callout boxes, not just inline text
5. Search `authentication.mdx` `serverConfigCode` block for `topgun-server` binary reference — zero unqualified matches expected (any remaining reference must be accompanied by "planned" or "not yet available" qualifier)
6. Search the token provider example in `authentication.mdx` for `localStorage` — zero matches expected
7. Build docs site (`pnpm start:docs`) — all three pages render without errors

## Constraints

- Do NOT change any Rust server code. This is a documentation-only spec.
- Do NOT remove planned/future content entirely. Mark it clearly as planned with version targets where known.
- Do NOT invent new API surface. Document only what exists or is confirmed planned.
- Maintain existing MDX component patterns (CodeBlock, FeatureCard, callout boxes). Do not introduce new components.
- Keep the existing page structure and navigation links intact.

## Assumptions

- `setAuthTokenProvider()` exists on `TopGunClient` and accepts an `async () => string | null` callback. (Based on the Clerk integration example already in the docs at line 33.)
- BetterAuth issues session tokens (cookies or opaque tokens) by default, not JWTs. A custom server endpoint is required to mint a TopGun-compatible JWT from a BetterAuth session. The bridge example will be illustrative, showing this two-step pattern.
- The "Planned for v3.0" label for cluster mTLS aligns with TODO-164.
- The "Planned — TODO-171" label for RBAC policy evaluation is the correct tracking item.
- `PORT` defaults to `8090` based on the test-server binary and existing docs.

## Audit History

### Audit v1 (2026-03-23 12:00)
**Status:** APPROVED

**Context Estimate:** ~28% total

**Quality Projection:** PEAK range (0-30%)

Delta validation: 3/3 entries valid

Strategic fit: Aligned with project goals — fixing documentation that causes real integration failures is high-value cleanup work after Rust migration.

Project compliance: Honors PROJECT.md decisions. Documentation-only spec, no code changes, no new dependencies, no constraint violations. Language Profile does not apply (files are in `apps/docs-astro/`, not `packages/core-rust/` or `packages/server-rust/`).

**Recommendations:**
1. The `serverConfigCode` export at lines 139-147 of `authentication.mdx` references `topgun-server` binary directly. R11 covers this for `security.mdx` but does not explicitly call out this code block in `authentication.mdx`. The implementer should also update this code block to mark the binary as planned or replace with `test-server` / programmatic embedding guidance, consistent with AC10.
2. R3 describes the new token provider as calling `/api/login` or returning a cached token. Consider clarifying where the cache lives (e.g., in-memory variable) since the spec explicitly removes `localStorage` usage. The example code should make the caching mechanism obvious to avoid confusion.
3. R6 (Better Auth bridge) should include a note that BetterAuth issues session tokens by default, and developers may need a custom server endpoint to mint a JWT from a BetterAuth session for TopGun consumption. The spec's Assumptions section acknowledges this, but the requirement text should make it explicit so the implementer includes this caveat in the documentation.

**Comment:** Well-structured documentation bugfix spec. Requirements are specific with line number references, acceptance criteria are measurable and automatable, and the delta section accurately reflects the current state of all three files. The scope is appropriate for a single execution pass.

### Response v1 (2026-03-23)
**Applied:** All three recommendations from Audit v1

**Changes:**
1. [✓] Rec 1 — `serverConfigCode` in `authentication.mdx` not covered by R11: Extended the Delta section to call out lines 139-147 of `authentication.mdx` explicitly. Added a new requirement block "R11 (extended)" under File 1 requirements that covers the `serverConfigCode` export. Updated AC10 to mention `authentication.mdx` explicitly. Added Validation Checklist item 5 to verify no unqualified `topgun-server` binary references in `serverConfigCode`.
2. [✓] Rec 2 — Token cache location ambiguous in R3: Added requirement 3 in R3 to use an explicit in-memory variable (`let cachedToken: string | null = null`) with a comment in the example code making the in-memory nature clear. Added AC11 ("In-memory token cache") to ensure this is verifiable. Added Validation Checklist item 6 to confirm no `localStorage` in the token provider example.
3. [✓] Rec 3 — R6 does not make BetterAuth JWT-minting requirement explicit: Updated R6 requirement text to state that BetterAuth issues session tokens by default, explain the two-step pattern (session → custom endpoint → JWT → `setAuthTokenProvider()`), and require the bridge example to reflect this. Updated AC4 to include the BetterAuth JWT-minting note. Updated Assumptions section to clarify the BetterAuth default behavior and the custom endpoint requirement.

### Audit v2 (2026-03-23 14:30)
**Status:** APPROVED

**Context Estimate:** ~25% total (3 MDX files, documentation-only edits)

**Quality Projection:** PEAK range (0-30%)

Delta validation: 3/3 entries valid

Strategic fit: Aligned with project goals -- fixing documentation that causes real integration failures (userId claim, phantom TLS env vars) is high-value work that directly unblocks developer adoption.

Project compliance: Honors PROJECT.md decisions. Documentation-only spec, no code changes, no new dependencies. Language Profile does not apply (MDX files in `apps/docs-astro/`, not Rust packages).

**Comment:** The spec is thorough and implementation-ready after Response v1 incorporated all Audit v1 recommendations. Key strengths: (1) all claims verified against actual codebase state -- `setAuthTokenProvider` confirmed in client SDK, `userId` references confirmed at documented line numbers, TLS env vars confirmed as phantom; (2) 11 acceptance criteria are concrete and automatable; (3) 7-item validation checklist provides grep-based verification; (4) clear separation of "what works" vs "planned" across all three files. No critical issues or new recommendations.

---

## Execution Summary

**Executed:** 2026-03-23
**Commits:** 3

### Files Created
None.

### Files Modified
- `apps/docs-astro/src/content/docs/guides/authentication.mdx` — Removed userId from JWT examples, replaced setAuthToken/localStorage with setAuthTokenProvider + in-memory cache, added Token Lifecycle section, added Better Auth bridge with JWT-minting note, updated serverConfigCode to use test-server and mark production binary as planned
- `apps/docs-astro/src/content/docs/guides/security.mdx` — Added Server Deployment Model section, added mTLS v3.0 planned banner, split env vars into working vars table and planned TLS vars table, added Trusted-Origin Bypass subsection, updated serverTlsCode and production env example to mark unimplemented vars as planned
- `apps/docs-astro/src/content/docs/guides/rbac.mdx` — Added Current Implementation Status banner, added What Works Today section, added Planned banners to Configuration and Field-Level Security sections, replaced userId with sub in Principal card, updated configCode to mark pattern matching as planned

### Files Deleted
None.

### Acceptance Criteria Status
- [x] AC1: No `userId` in JWT docs — zero matches in authentication.mdx
- [x] AC2: Token provider pattern — `setAuthTokenProvider()` used throughout, no bare `setAuthToken()`
- [x] AC3: Token lifecycle documented — "Token Lifecycle" section added explaining expiry-on-reconnect
- [x] AC4: Better Auth bridge — bridge section added with JWT-minting note and two-step pattern
- [x] AC5: TLS env vars marked planned — all TOPGUN_TLS_* and TOPGUN_CLUSTER_* vars have "Planned" markers
- [x] AC6: mTLS section marked planned — v3.0 banner added at top of mTLS section
- [x] AC7: Working env vars documented — TOPGUN_ADMIN_PASSWORD, TOPGUN_ADMIN_USERNAME, TOPGUN_ADMIN_DIR, TOPGUN_LOG_FORMAT in working vars table
- [x] AC8: Trusted-origin bypass documented — Forwarded/Backup/Wan/System bypass table added in Security Pipeline
- [x] AC9: RBAC status clear — prominent banner with Current Implementation Status and What Works Today section
- [x] AC10: No unqualified topgun-server binary claims — all three files use "planned" qualifiers for any topgun-server production binary reference
- [x] AC11: In-memory token cache — explicit `let cachedToken: string | null = null` with in-memory comment in clientAuthCode and betterAuthBridgeClientCode

### Deviations
None. All requirements implemented as specified.

### Notes
- The `betterAuthBridgeServerCode` and `betterAuthBridgeClientCode` exports were added to authentication.mdx alongside the bridge section — no new MDX components were introduced, consistent with spec constraints.
- The `envVarsTable` export in security.mdx was not used inline in the page body (it was an export but the page rendered the table directly). The replacement maintains the same pattern.
