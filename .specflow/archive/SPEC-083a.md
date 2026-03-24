---
id: SPEC-083a
type: docs
status: done
priority: P2
complexity: medium
created: 2026-03-08
parent: SPEC-083
depends_on: []
---

# Reference Documentation Rewrite for Rust Server

## Context

The Rust server migration is functionally complete (Phase 3 done, all integration tests passing). The reference docs (`reference/server.mdx`, `reference/cli.mdx`, `reference/protocol.mdx`) still describe the TypeScript server API. These pages must be rewritten to document the Rust server, its configuration, endpoints, and wire protocol.

**Parent:** SPEC-083 (Update Documentation Content for Rust Server)
**Source TODO:** TODO-106

## Task

Rewrite the three reference documentation pages to accurately describe the Rust server, its configuration surfaces, and the MsgPack wire protocol.

## Requirements

### R1: Rewrite `reference/server.mdx` for Rust Server

**File:** `apps/docs-astro/src/content/docs/reference/server.mdx`

- Replace all TypeScript `ServerFactory.create()` examples with Rust server binary startup
- Document Rust server configuration: environment variables (`PORT`, `TOPGUN_ADMIN_DIR`, `RUST_LOG`) and programmatic config via `NetworkConfig`/`ServerConfig` structs
- Document all Rust server endpoints:
  - `GET /health` — basic health check
  - `GET /health/live` — liveness probe
  - `GET /health/ready` — readiness probe
  - `GET /ws` — WebSocket upgrade
  - `POST /sync` — HTTP sync transport
  - `GET /metrics` — Prometheus metrics
  - `GET /api/status` — server status
  - `POST /api/auth/login` — authentication
  - `GET /api/admin/cluster/status` — cluster status
  - `GET /api/admin/maps` — map listing
  - `GET|PUT /api/admin/settings` — server settings
  - `GET /api/docs` — Swagger UI
  - `/admin/*` — SPA admin dashboard
- Document PostgresDataStore configuration (connection string, pool settings)
- Remove all references to `@topgunbuild/server` npm package as the primary server
- Keep client-side TypeScript examples (those remain valid)
- For each TS-specific configuration section (Connection Scaling Options, Event Queue Options, Backpressure Options, Write Coalescing Options, Rate Limiting Options, TLS Configuration), verify if a Rust equivalent exists in the server-rust source; remove sections with no Rust counterpart

### R2: Rewrite `reference/cli.mdx` for Rust Server Configuration

**File:** `apps/docs-astro/src/content/docs/reference/cli.mdx`

The Rust server does not currently have a production binary with CLI argument parsing (no `clap` dependency). The only binary is `test-server`, used for integration tests. R2 must document only what exists:

- Document the `test-server` binary and its purpose (integration testing)
- Document the existing environment variables: `PORT` (server port), `TOPGUN_ADMIN_DIR` (admin dashboard static files), `RUST_LOG` (tracing filter, e.g., `topgun_server=debug`)
- Document programmatic configuration via `NetworkConfig` and `ServerConfig` structs (the primary configuration surface for embedding the server)
- Include a Docker run example using environment variables (not CLI flags)
- Remove all `npx topgun` commands and audit all existing CLI commands (`topgun doctor`, `topgun setup`, `topgun dev`, `topgun test`, `topgun config`, `topgun cluster:*`, `topgun debug:*`, `topgun search:explain`) -- remove any that do not exist in the Rust binary
- Note that a production binary with CLI flags may be added in a future spec

### R3: Verify and Update `reference/protocol.mdx` for Rust Wire Protocol

**File:** `apps/docs-astro/src/content/docs/reference/protocol.mdx`

- Verify all message type examples match the current Rust `Message` enum variants
- Update the envelope format if the Rust server uses different field names or structure
- Document the binary MsgPack framing (4-byte BE u32 length-prefix for BATCH messages)
- Confirm AUTH flow matches Rust implementation (two-phase: AUTH_REQUIRED on connect, AUTH with token, AUTH_ACK/AUTH_FAIL response)

## Acceptance Criteria

1. `reference/server.mdx` contains zero references to `ServerFactory.create()` or `@topgunbuild/server` as primary server setup -- all examples show Rust server
2. `reference/cli.mdx` documents the `test-server` binary, the three environment variables (`PORT`, `TOPGUN_ADMIN_DIR`, `RUST_LOG`), programmatic `NetworkConfig`/`ServerConfig` configuration, and includes a Docker example
3. `reference/protocol.mdx` message examples match the Rust `Message` enum (spot-check at least AUTH, CLIENT_OP, OP_BATCH, MERKLE_DIFF)
4. `pnpm start:docs` builds without errors from these three files

## Constraints

- Do NOT remove client-side TypeScript documentation (`@topgunbuild/client`, `@topgunbuild/react` remain TS packages)
- Do NOT change the docs site framework (Astro + MDX + React components)
- Follow existing MDX conventions (frontmatter `order` field, breadcrumb pattern, component imports)

## Assumptions

- The only Rust server binary is `test-server` (defined in Cargo.toml `[[bin]]` section). No production binary with CLI flags exists yet.
- The Rust server reads three environment variables: `PORT` (in `test_server.rs`), `TOPGUN_ADMIN_DIR` (in `module.rs`), and `RUST_LOG` (via `tracing_subscriber::EnvFilter`). No other env vars are read.
- Programmatic configuration is done via `NetworkConfig` and `ServerConfig` structs. These are the primary configuration surface.

## Implementation Notes

Read the Rust server source to extract accurate endpoints and configuration:
- `packages/server-rust/src/bin/test_server.rs` (binary entry point)
- `packages/server-rust/src/config.rs` (or equivalent configuration structs)
- `packages/server-rust/src/network/module.rs` (endpoint routes)
- `packages/core-rust/src/message/` (Message enum variants for protocol docs)

## Audit History

### Audit v1 (2026-03-08)
**Status:** APPROVED

**Context Estimate:** ~25% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest file | ~10% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**

1. **Clarity:** PASS -- Context explains WHY (TS docs are stale after Rust migration), Task explains WHAT (rewrite 3 reference pages), Requirements are specific per file.
2. **Completeness:** PASS -- All 3 target files identified with absolute paths. Requirements list specific content to add and remove. Implementation Notes point to Rust source files for accuracy.
3. **Testability:** PASS -- AC1 is a negative check (zero references to old API). AC2 requires specific content (flags, env vars, Docker). AC3 requires spot-check against Rust source. AC4 is a build verification.
4. **Scope:** PASS -- Clearly bounded to 3 MDX files. Constraints explicitly exclude client-side docs, framework changes. No scope creep.
5. **Feasibility:** PASS -- Straightforward documentation rewrite. Assumptions acknowledge binary name and flags need verification from source. Implementation Notes provide the right source files to read.
6. **Architecture fit:** PASS -- Follows existing MDX conventions (frontmatter, component imports, breadcrumb pattern). Docs site framework unchanged.
7. **Non-duplication:** PASS -- Rewriting existing files, not creating parallel documentation.
8. **Cognitive load:** PASS -- Simple documentation task. Three files, each with clear purpose. No abstractions or indirection.
9. **Strategic fit:** PASS -- Aligned with project goals. Rust migration is functionally complete; docs must match reality. P2 priority is appropriate for post-migration cleanup.
10. **Project compliance:** PASS -- Honors PROJECT.md decisions. Language Profile does not apply (docs files, not Rust source in core-rust/server-rust). No violations or deviations.

**Rust Auditor Checklist:** N/A -- this is a docs spec, not a Rust code spec. No structs, enums, or serde attributes involved.

**Strategic fit:** Aligned with project goals -- documentation must reflect the completed Rust migration.

**Project compliance:** Honors PROJECT.md decisions. No violations.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Binary is named topgun-server | Docs show wrong binary name (low risk -- spec says "verify from source") |
| A2 | Env vars follow PORT/DATABASE_URL convention | Wrong env var names in docs (low risk -- spec says "verify from source") |
| A3 | POST /sync endpoint exists in Rust server | server.mdx documents nonexistent endpoint (medium risk -- verify during implementation) |
| A4 | Existing MDX components (CodeBlock, ApiParam, etc.) work with new content | Build failure (low risk -- AC4 catches this) |

**Comment:** Well-structured documentation spec with clear scope, specific file targets, and practical acceptance criteria. The "verify from Rust source" approach in Assumptions and Implementation Notes is the right pattern for docs specs -- it avoids encoding potentially stale details in the spec while ensuring accuracy during implementation.

**Recommendations:**

1. R3 says "Update protocol.mdx IF wire format changed" -- the conditional framing ("if") may lead an implementer to skip verification. Since the spec already lists specific checks (BATCH framing, AUTH flow), the "if" is misleading. Consider removing the conditional and treating R3 as a required verification pass.

2. The existing `server.mdx` documents TS-specific features that may not exist in Rust (Connection Scaling Options, Event Queue Options, Backpressure Options, Write Coalescing Options, Rate Limiting Options, TLS Configuration). The spec does not explicitly state whether these sections should be removed, replaced with Rust equivalents, or preserved. The implementer will need to check the Rust server source for each. Consider adding a note: "For each TS-specific configuration section, verify if a Rust equivalent exists; remove sections with no Rust counterpart."

3. The existing `cli.mdx` documents commands like `topgun doctor`, `topgun setup`, `topgun dev`, `topgun test`, `topgun config`, cluster commands, and debug commands. Most of these are Node.js CLI tooling that likely does not exist in the Rust binary. R2 says "Remove references to `npx topgun doctor`, `npx topgun setup` if those no longer exist" but does not cover `topgun dev`, `topgun test`, `topgun config`, `topgun cluster:*`, `topgun debug:*`, `topgun search:explain`. The implementer should audit all commands, not just the two mentioned.

### Response v1 (2026-03-08)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [x] R3 conditional "if" removed -- Renamed R3 heading from "Update `reference/protocol.mdx` if Wire Format Changed" to "Verify and Update `reference/protocol.mdx` for Rust Wire Protocol", making it a required verification pass rather than conditional
2. [x] TS-specific config section guidance added to R1 -- Added bullet: "For each TS-specific configuration section (Connection Scaling Options, Event Queue Options, Backpressure Options, Write Coalescing Options, Rate Limiting Options, TLS Configuration), verify if a Rust equivalent exists in the server-rust source; remove sections with no Rust counterpart"
3. [x] CLI command audit scope expanded in R2 -- Replaced narrow "Remove references to `npx topgun doctor`, `npx topgun setup` if those no longer exist" with comprehensive: "Audit all existing CLI commands (`topgun doctor`, `topgun setup`, `topgun dev`, `topgun test`, `topgun config`, `topgun cluster:*`, `topgun debug:*`, `topgun search:explain`) and remove any that do not exist in the Rust binary"

### Audit v2 (2026-03-08 fresh-eyes)
**Status:** NEEDS_REVISION

**Context Estimate:** ~30% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~30% | <=50% | OK |
| Largest file (server.mdx ~517 lines + Rust source reading) | ~12% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**

1. **Clarity:** PASS -- Title, context, and task are clear. Requirements are specific per file.
2. **Completeness:** FAIL -- See Critical 1 and 2 below.
3. **Testability:** FAIL -- AC2 requires documenting "flags, env vars, and Docker example" but the flags and env vars do not exist in the codebase. See Critical 1.
4. **Scope:** PASS -- Bounded to 3 MDX files with clear constraints.
5. **Feasibility:** FAIL -- R2 asks to document CLI flags and env vars that do not exist. See Critical 1.
6. **Architecture fit:** PASS -- Follows existing MDX conventions.
7. **Non-duplication:** PASS -- Rewriting existing files, not creating duplicates.
8. **Cognitive load:** PASS -- Straightforward documentation task.
9. **Strategic fit:** PASS -- Documentation must reflect the completed Rust migration.
10. **Project compliance:** PASS -- Language Profile does not apply (MDX files, not Rust source). No violations.

**Language Profile:** N/A -- this spec modifies MDX files in `apps/docs-astro/`, not Rust source files in `packages/core-rust/` or `packages/server-rust/`.

**Assumptions Verified Against Source:**

| # | Assumption | Verified | Finding |
|---|------------|----------|---------|
| A1 | Binary is named `topgun-server` | WRONG | Only binary is `test-server` (Cargo.toml `[[bin]]` section). No `main.rs` exists. No production binary. |
| A2 | Env vars: `PORT`, `DATABASE_URL`, `CLUSTER_PORT`, `NODE_ID` | MOSTLY WRONG | Only `PORT` (in test_server.rs) and `TOPGUN_ADMIN_DIR` (in module.rs) are read. No `DATABASE_URL`, `CLUSTER_PORT`, `NODE_ID` env vars exist. |
| A3 | CLI flags: `--port`, `--cluster-port`, `--db-url`, `--node-id` | WRONG | No `clap` dependency. No CLI argument parsing exists anywhere in server-rust. |
| A4 | POST /sync endpoint exists | CORRECT | `POST /sync` route exists in `network/module.rs` line 263. |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | A production binary with CLI flags exists or will exist before docs ship | R2 and AC2 are unmeetable; cli.mdx would document fictional features |
| A2 | Programmatic config (NetworkConfig, ServerConfig) is equivalent to CLI flags | Documenting struct fields as if they are CLI flags misleads users |

Strategic fit: Aligned with project goals, but R2 scope needs adjustment to match reality.

Project compliance: Honors PROJECT.md decisions. No violations.

**Critical:**

1. **R2 and AC2 assume CLI flags and env vars that do not exist.** The Rust server has no `clap` dependency, no argument parsing, and no production binary. The only binary is `test-server` (for integration tests), which hardcodes its config and reads only `PORT` from the environment. R2 says "Document `topgun-server` flags: `--port`, `--cluster-port`, `--db-url`, `--node-id`" but none of these exist. AC2 requires "flags, env vars, and Docker example" but there are no flags to document. The spec must either: (a) scope R2 to document only what exists (the `test-server` binary, `PORT` env var, `TOPGUN_ADMIN_DIR` env var, and programmatic config via `NetworkConfig`/`ServerConfig` structs), or (b) explicitly state that R2 includes creating CLI argument parsing as a prerequisite (which changes this from a docs spec to a code+docs spec and affects complexity/scope).

2. **R1 endpoint list is incomplete.** The Rust server (in `network/module.rs`) exposes more endpoints than R1 lists. The full set is: `GET /health`, `GET /health/live`, `GET /health/ready`, `GET /ws`, `POST /sync`, `GET /metrics`, `GET /api/status`, `POST /api/auth/login`, `GET /api/admin/cluster/status`, `GET /api/admin/maps`, `GET|PUT /api/admin/settings`, `GET /api/docs` (Swagger UI), `/admin/*` (SPA dashboard). R1 should either document all endpoints or explicitly state which are excluded and why.

**Recommendations:**

1. Consider splitting this spec: R1 (server.mdx) and R3 (protocol.mdx) are implementable as-is with minor fixes, but R2 (cli.mdx) requires either a code change (add clap + production binary) or a fundamental rewrite of its requirements. A separate spec for the production binary could unblock R1 and R3.

2. The `Implementation Notes` section references `packages/server-rust/src/main.rs` which does not exist. The actual binary entry point is `packages/server-rust/src/bin/test_server.rs`. Update this reference.

3. The `RUST_LOG` env var (`tracing_subscriber::EnvFilter`) is a documented configuration surface that users will need. Consider adding it to the server configuration documentation in R1.

### Response v2 (2026-03-08)
**Applied:** All 2 critical issues and all 3 recommendations from Audit v2

**Changes:**
1. [x] Critical 1: R2 and AC2 rewritten to document only what exists -- R2 heading changed from "Rewrite `reference/cli.mdx` for Rust CLI" to "Rewrite `reference/cli.mdx` for Rust Server Configuration"; removed all references to CLI flags (`--port`, `--cluster-port`, `--db-url`, `--node-id`); replaced with `test-server` binary, three env vars (`PORT`, `TOPGUN_ADMIN_DIR`, `RUST_LOG`), and programmatic `NetworkConfig`/`ServerConfig` config; AC2 rewritten to match (no flags, documents env vars and struct config); Docker example uses env vars not CLI flags; added note about future production binary
2. [x] Critical 2: R1 endpoint list expanded to full set -- replaced partial list with all 13 endpoints from `network/module.rs`: `/health`, `/health/live`, `/health/ready`, `/ws`, `/sync`, `/metrics`, `/api/status`, `/api/auth/login`, `/api/admin/cluster/status`, `/api/admin/maps`, `/api/admin/settings`, `/api/docs`, `/admin/*`
3. [x] Recommendation 1 (splitting): Not applied as a split, but R2 is now self-contained by documenting only what exists. No code prerequisite needed. Future production binary can be a separate spec.
4. [x] Recommendation 2: Implementation Notes updated -- replaced `packages/server-rust/src/main.rs` with `packages/server-rust/src/bin/test_server.rs`; added `packages/server-rust/src/network/module.rs` for endpoint routes
5. [x] Recommendation 3: `RUST_LOG` added to R1 server configuration env vars and to R2 env var list; also added to Assumptions

**Skipped:** None. Recommendation 1 (splitting) was addressed by rewriting R2 to be feasible without a code prerequisite, making the split unnecessary.

### Audit v3 (2026-03-09)
**Status:** APPROVED

**Context Estimate:** ~29% total

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~29% | <=50% | OK |
| Largest file (server.mdx rewrite + Rust source reading) | ~12% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**

1. **Clarity:** PASS -- Context explains WHY (TS docs stale after Rust migration), Task explains WHAT (rewrite 3 reference pages), Requirements are specific per file with explicit bullet lists.
2. **Completeness:** PASS -- All 3 target files identified. R1 lists all 13 endpoints. R2 documents only what exists (test-server binary, 3 env vars, programmatic config). R3 has specific verification targets. Implementation Notes point to correct Rust source files.
3. **Testability:** PASS -- AC1 is a negative check (zero old references). AC2 requires specific documented items matching reality. AC3 requires spot-check against Rust source. AC4 is build verification.
4. **Scope:** PASS -- Bounded to 3 MDX files. Constraints explicitly exclude client-side docs and framework changes.
5. **Feasibility:** PASS -- All requirements document things that actually exist. R2 was corrected in v2 to match codebase reality.
6. **Architecture fit:** PASS -- Follows existing MDX conventions (frontmatter, component imports, breadcrumb pattern).
7. **Non-duplication:** PASS -- Rewriting existing files, not creating parallel documentation.
8. **Cognitive load:** PASS -- Straightforward documentation task. Three files, each with clear purpose.
9. **Strategic fit:** PASS -- Aligned with project goals. Rust migration is functionally complete; docs must match reality.
10. **Project compliance:** PASS -- Honors PROJECT.md decisions. No violations.

**Language Profile:** N/A -- this spec modifies MDX files in `apps/docs-astro/`, not Rust source files in `packages/core-rust/` or `packages/server-rust/`.

**Rust Auditor Checklist:** N/A -- docs spec, no Rust structs or serde attributes.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Only binary is test-server | Low -- spec explicitly states this and instructs implementer to verify from source |
| A2 | Three env vars (PORT, TOPGUN_ADMIN_DIR, RUST_LOG) | Low -- spec says to verify from source |
| A3 | NetworkConfig/ServerConfig are the primary config surface | Low -- implementer reads actual structs during implementation |

**Strategic fit:** Aligned with project goals -- documentation must reflect the completed Rust migration.

**Project compliance:** Honors PROJECT.md decisions. No violations.

**Comment:** The spec has matured well through two revision cycles. All critical issues from Audit v2 (fictional CLI flags, incomplete endpoint list) have been resolved. Requirements now accurately reflect the Rust server's actual configuration surfaces. The "document only what exists" approach for R2 is pragmatic and avoids documenting fictional features. The spec is clear, implementable, and well-scoped.

---

## Execution Summary

**Executed:** 2026-03-09
**Commits:** 3

### Files Modified
- `apps/docs-astro/src/content/docs/reference/server.mdx` — Complete rewrite for Rust server: replaced all TS ServerFactory.create() examples with Rust binary startup, documented all 13 endpoints, NetworkConfig/ServerConfig structs, env vars, PostgresDataStore, graceful shutdown. Removed TS-specific sections (Connection Scaling, Event Queue, Backpressure, Write Coalescing, Rate Limiting, TLS) with no Rust equivalent.
- `apps/docs-astro/src/content/docs/reference/cli.mdx` — Complete rewrite: removed all npx topgun commands (doctor, setup, dev, test, config, cluster:*, debug:*, search:explain). Documented test-server binary, 3 env vars (PORT, TOPGUN_ADMIN_DIR, RUST_LOG), programmatic NetworkConfig/ServerConfig, Docker example.
- `apps/docs-astro/src/content/docs/reference/protocol.mdx` — Updated for Rust wire protocol: binary MsgPack transport (not JSON default), internally-tagged union format, added AUTH_REQUIRED/AUTH_FAIL examples, BATCH binary framing (4-byte BE u32 length-prefix), SYNC_RESP_BUCKETS/SYNC_RESP_LEAF examples, OP_REJECTED, complete 77-type reference table.

### Acceptance Criteria Status
- [x] AC1: server.mdx has zero references to ServerFactory.create() or @topgunbuild/server as primary server setup
- [x] AC2: cli.mdx documents test-server binary, 3 env vars, programmatic NetworkConfig/ServerConfig, Docker example
- [x] AC3: protocol.mdx message examples match Rust Message enum (verified AUTH, CLIENT_OP, OP_BATCH, MERKLE_REQ_BUCKET)
- [x] AC4: pnpm start:docs builds without errors (53 pages built successfully)

### Deviations
None. All requirements implemented as specified.

### Notes
- The TS-specific configuration sections removed from server.mdx (Connection Scaling, Event Queue, Backpressure, Write Coalescing, Rate Limiting, TLS) were verified against the Rust server-rust source. The Rust server has TLS support (TlsConfig struct and serve_tls function), but the configuration surface is fundamentally different (Rust struct vs TS interface with many more options). TLS is documented within the NetworkConfig struct section rather than as a separate section.
- Protocol.mdx now includes a complete reference table of all 77 message types across 7 domains, derived from the Rust Message enum in core-rust/src/messages/mod.rs.
- The MERKLE_DIFF type referenced in AC3 does not exist as a single message type. The Merkle diff protocol uses MERKLE_REQ_BUCKET + SYNC_RESP_BUCKETS + SYNC_RESP_LEAF as a multi-step tree walk. All three are documented.

---

## Review History

### Review v1 (2026-03-09)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. server.mdx and cli.mdx both contain identical Docker run examples and overlapping env var tables. This is intentional cross-referencing between the two pages, but the duplication means a future change to env vars requires updating both files. Consider linking from one to the other instead of duplicating.

2. protocol.mdx BATCH example shows `"data": <binary>` which is not valid JSON. While the surrounding comment explains the binary framing, the example might confuse readers who try to parse it. A hex dump or diagram might be clearer for this particular message type.

3. cli.mdx breadcrumb uses manual `<div>` markup (lines 70-76) while server.mdx does not include a breadcrumb. Inconsistent breadcrumb usage across the three reference pages.

**Passed:**
- [x] AC1: server.mdx contains zero references to `ServerFactory.create()` or `@topgunbuild/server` -- verified via grep, no matches
- [x] AC2: cli.mdx documents `test-server` binary (line 87+), three env vars `PORT`/`TOPGUN_ADMIN_DIR`/`RUST_LOG` (table at lines 113-140), programmatic `NetworkConfig` (line 146+) and `ServerConfig` (line 157+), Docker example (line 168+)
- [x] AC3: protocol.mdx includes AUTH (lines 19-39), AUTH_REQUIRED, AUTH_FAIL, CLIENT_OP (line 42), OP_BATCH (line 61), MERKLE_REQ_BUCKET (line 151), SYNC_RESP_BUCKETS (line 139), SYNC_RESP_LEAF (line 159) -- all match Rust Message enum variants
- [x] AC4: Build verified per execution summary (53 pages built successfully)
- [x] R1 endpoints: All 13 endpoints documented in server.mdx (health, health/live, health/ready, ws, sync, metrics, api/status, api/auth/login, api/admin/cluster/status, api/admin/maps, api/admin/settings, api/docs, admin/*) -- matches `network/module.rs` routes exactly
- [x] R1 TS sections removed: Connection Scaling, Event Queue, Backpressure, Write Coalescing, Rate Limiting removed; TLS folded into NetworkConfig
- [x] R2 old CLI commands removed: No `npx topgun`, `topgun doctor`, `topgun setup`, or any other old CLI commands remain -- verified via grep
- [x] R2 future production binary note present (line 82-85 yellow callout)
- [x] R3 binary MsgPack framing documented: BATCH 4-byte BE u32 length-prefix (lines 69-78)
- [x] R3 AUTH flow documented: two-phase AUTH_REQUIRED -> AUTH -> AUTH_ACK/AUTH_FAIL (connection lifecycle section)
- [x] NetworkConfig fields match Rust source (`config.rs`): host, port, tls, connection, cors_origins, request_timeout -- all correct with accurate types and defaults
- [x] ServerConfig fields match Rust source (`service/config.rs`): node_id, default_operation_timeout_ms, max_concurrent_operations, gc_interval_ms, partition_count, security -- all correct
- [x] Constraints respected: no client-side TS docs removed, no framework changes, MDX conventions followed (frontmatter order field, component imports)
- [x] No security issues: hardcoded JWT secret is correctly documented as test-only (server.mdx line 148, cli.mdx line 101)

**Summary:** All four acceptance criteria are met. The three reference docs accurately reflect the Rust server's actual configuration surfaces, endpoints, and wire protocol. Content was verified against the Rust source files (`network/config.rs`, `service/config.rs`, `network/module.rs`). The documentation is well-structured, follows existing MDX conventions, and provides clear examples. Three minor issues identified (content duplication between pages, BATCH binary example formatting, inconsistent breadcrumbs) -- none affect correctness.

### Fix Response v1 (2026-03-09)
**Applied:** All 3 minor issues from Review v1

**Fixes:**
1. [✓] Duplicated Docker/env var content — Replaced inline Docker section and env var table in server.mdx with links to cli.mdx. Single source of truth now in cli.mdx.
   - Commit: e2eb354
2. [✓] BATCH binary example — Replaced invalid `"data": <binary>` JSON with ASCII wire diagram showing 4-byte BE u32 length-prefix framing and hex example.
   - Commit: e2eb354
3. [✓] Inconsistent breadcrumbs — Removed manual `<div>` breadcrumb from cli.mdx to match server.mdx and protocol.mdx (no manual breadcrumbs).
   - Commit: e2eb354

### Review v2 (2026-03-09)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verification (all 3 from Review v1):**
1. [VERIFIED] Duplication removed -- server.mdx now links to cli.mdx for env vars (line 150) and Docker (line 364) instead of duplicating content. Single source of truth in cli.mdx.
2. [VERIFIED] BATCH binary example -- protocol.mdx lines 69-90 now use an ASCII wire diagram with hex example instead of invalid `"data": <binary>` JSON.
3. [VERIFIED] Breadcrumb consistency -- No manual breadcrumb `<div>` markup in any of the three files. All three pages are consistent (no manual breadcrumbs).

**Passed:**
- [x] AC1: server.mdx contains zero references to `ServerFactory.create()` or `@topgunbuild/server` -- verified via grep, no matches
- [x] AC2: cli.mdx documents `test-server` binary (line 79+), three env vars in table (lines 106-131), programmatic `NetworkConfig` (line 138+) and `ServerConfig` (line 149+), Docker example (line 160+)
- [x] AC3: protocol.mdx message types match Rust `Message` enum -- AUTH_REQUIRED, AUTH, AUTH_ACK, AUTH_FAIL, CLIENT_OP, OP_BATCH, MERKLE_REQ_BUCKET, SYNC_RESP_BUCKETS, SYNC_RESP_LEAF all confirmed in `core-rust/src/messages/mod.rs`
- [x] AC4: Build verified per execution summary (53 pages)
- [x] All 13 endpoints documented in server.mdx endpoint tables
- [x] No old CLI commands remain (npx topgun, topgun doctor, etc.) -- verified via grep
- [x] Future production binary callout present in cli.mdx (yellow box, lines 74-77)
- [x] BATCH binary framing documented with wire diagram (protocol.mdx lines 69-90)
- [x] Two-phase AUTH flow documented in connection lifecycle (protocol.mdx lines 296-325)
- [x] server.mdx env vars section links to cli.mdx (no duplication)
- [x] server.mdx Docker section links to cli.mdx (no duplication)
- [x] MDX conventions followed: frontmatter with order field, component imports, no framework changes
- [x] Constraints respected: client-side TS docs untouched
- [x] No security issues: hardcoded JWT secret documented as test-only
- [x] 77-type message reference table in protocol.mdx (lines 534-577)

**Summary:** All four acceptance criteria remain met after fixes. The three minor issues from Review v1 have been correctly resolved: duplication eliminated via cross-page links, BATCH example uses a clear wire diagram, and breadcrumb markup is consistent across all three pages. The documentation is complete, accurate, and ready for finalization.

---

## Completion

**Completed:** 2026-03-09
**Total Commits:** 4 (3 implementation + 1 fix)
**Review Cycles:** 2

### Outcome

Rewrote all three reference documentation pages (server.mdx, cli.mdx, protocol.mdx) to accurately reflect the Rust server's endpoints, configuration surfaces, and MsgPack wire protocol.

### Key Files

- `apps/docs-astro/src/content/docs/reference/server.mdx` — Rust server reference: 13 endpoints, NetworkConfig/ServerConfig structs, PostgresDataStore
- `apps/docs-astro/src/content/docs/reference/cli.mdx` — Server configuration: test-server binary, env vars, Docker example, programmatic config
- `apps/docs-astro/src/content/docs/reference/protocol.mdx` — Wire protocol: MsgPack framing, AUTH flow, BATCH format, 77-type reference table

### Patterns Established

None — followed existing MDX documentation patterns.

### Deviations

None — implemented as specified.
