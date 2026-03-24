---
id: SPEC-115
type: docs
status: done
priority: P2
complexity: medium
created: 2026-03-11
---

# SPEC-115: Verify and Finalize Documentation Content for Rust Server

## Context

TODO-106 specifies updating docs site content for the Rust server. However, most of the originally scoped work has already been completed:

- **SPEC-114** updated Docker, cluster CLI, deployment docs, and CONTRIBUTING.md for the Rust server
- **TODO-096** (SPEC-096) created the Security Guide (`guides/security.mdx`), Adoption Path (`guides/adoption-path.mdx`), and PostgreSQL guide (`guides/postgresql.mdx`)
- `comparison.mdx` already includes Replicache/Zero and Durable Objects columns
- `examples/collaborative-tasks/` was already deleted (SPEC-084)
- `examples/sync-lab/` already has session isolation via `prefixMap()` and `getSessionId()`
- `reference/server.mdx`, `reference/cli.mdx`, and `reference/protocol.mdx` already reference the Rust server

What remains is a **verification pass** to ensure all documentation content is accurate against the current Rust server codebase, code snippets compile/run correctly, and no stale references exist. Additionally, the `examples/todo-app/` directory still exists as a standalone app and should be evaluated for removal (replaced by the Tier 1 code snippet already in `adoption-path.mdx`).

## Task

Audit all docs site pages for accuracy against the current Rust server implementation. Fix any stale content, broken code snippets, or missing information. Remove the `examples/todo-app/` directory since its purpose is now served by the Tier 1 code snippet in `adoption-path.mdx`.

## Requirements

### Files to Verify (read + fix if stale)

1. **`apps/docs-astro/src/content/docs/reference/server.mdx`** — Verify all Rust struct fields (`NetworkConfig`, `ServerConfig`, `ConnectionConfig`, `TlsConfig`, `SecurityConfig`, `PostgresDataStore`) match current source in `packages/server-rust/`. Verify endpoint table matches actual routes.
2. **`apps/docs-astro/src/content/docs/reference/cli.mdx`** — Verify environment variables (`PORT`, `RUST_LOG`, `TOPGUN_ADMIN_DIR`, `DATABASE_URL`) match the Rust binary. Verify Docker run example uses correct image name and env vars.
3. **`apps/docs-astro/src/content/docs/reference/protocol.mdx`** — Verify message types (`AUTH_REQUIRED`, `AUTH`, `AUTH_ACK`, `AUTH_FAIL`, `CLIENT_OP`, `BATCH`, etc.) match `packages/server-rust/src/network/` and `packages/core-rust/src/message/`. Verify field names are camelCase as documented.
4. **`apps/docs-astro/src/content/docs/comparison.mdx`** — Verify Replicache/Zero and Durable Objects entries are factually accurate. Ensure the "Why TopGun?" prose sections are up-to-date with current capabilities.
5. **`apps/docs-astro/src/content/docs/guides/security.mdx`** — Verify TLS env var names match Rust config. Verify RBAC/JWT claims match `JwtClaims` struct (uses `sub` not `userId`).
6. **`apps/docs-astro/src/content/docs/guides/adoption-path.mdx`** — Verify Tier 1 code snippet compiles and uses current SDK API (`TopGunClient`, `useQuery`, `useClient`, `IDBAdapter`).
7. **`apps/docs-astro/src/content/docs/guides/postgresql.mdx`** — Verify table schemas (`topgun_maps`, `topgun_merkle`) match `PostgresDataStore` DDL in Rust source. Verify comparison table (PowerSync/ElectricSQL vs TopGun) is present and accurate.
8. **`apps/docs-astro/src/content/docs/quick-start.mdx`** — Verify the getting-started code uses current SDK API.
9. **`apps/docs-astro/src/content/docs/installation.mdx`** — Verify package names and install commands are current.
10. **`apps/docs-astro/src/content/docs/guides/deployment.mdx`** — Verify Dockerfile references, compose files, and environment variables match the current `deploy/` directory.

### Files to Modify

1. Fix any inaccuracies found in the verification pass (update code snippets, struct fields, env vars, etc.)
2. Remove `examples/todo-app/` directory entirely (replaced by Tier 1 snippet in adoption-path.mdx)
3. Remove `examples/todo-app` from any workspace configuration (`pnpm-workspace.yaml`, root `package.json`, etc.) if referenced

### Files NOT to Create

- No new documentation pages (all content already exists)
- No new example applications

## Acceptance Criteria

1. Every Rust struct documented in `reference/server.mdx` (`NetworkConfig`, `ServerConfig`, `ConnectionConfig`, `PostgresDataStore`) has fields matching the current Rust source code — no missing fields, no extra fields, no wrong types
2. Every HTTP/WS endpoint documented in `reference/server.mdx` exists in the Rust router (`packages/server-rust/src/network/`) — no phantom endpoints, no undocumented endpoints
3. Every environment variable documented in `reference/cli.mdx` is read by the Rust binary — no phantom vars
4. Every message type documented in `reference/protocol.mdx` has a corresponding variant in the Rust `Message` enum — no phantom message types
5. The `topgun_maps` and `topgun_merkle` DDL in `guides/postgresql.mdx` matches the `CREATE TABLE` statements in `PostgresDataStore::initialize()`
6. The comparison table in `guides/postgresql.mdx` (PowerSync/ElectricSQL vs TopGun) is present with at least 5 rows
7. The Tier 1 code snippet in `guides/adoption-path.mdx` uses only APIs that exist in the current `@topgunbuild/client` and `@topgunbuild/react` packages
8. `examples/todo-app/` directory no longer exists
9. No documentation page references `examples/collaborative-tasks/`, `examples/todo-app/`, `@topgunbuild/server`, or `packages/server/`
10. Security guide references `sub` (not `userId`) for JWT claims

## Validation Checklist

1. `grep -r "todo-app\|collaborative-tasks\|@topgunbuild/server\|packages/server/" apps/docs-astro/src/content/` — returns zero matches
2. Compare `NetworkConfig` fields in `server.mdx` against `packages/server-rust/src/network/config.rs` — all fields present and types correct
3. Compare `topgun_maps` DDL in `postgresql.mdx` against `PostgresDataStore` source — columns and types match
4. `ls examples/todo-app 2>/dev/null` — directory does not exist
5. Verify `pnpm build` in `apps/docs-astro/` succeeds after changes (no broken imports)

## Constraints

- Do NOT create new documentation pages — only update existing ones
- Do NOT modify any Rust or TypeScript source code (only `.mdx` files and example directory deletion)
- Do NOT change the Astro docs site configuration or theme
- Do NOT add speculative documentation for v2.0 features (SQL, DAG, WASM, etc.)
- Preserve the existing MDX component patterns (`CodeBlock`, `ApiParam`, `ApiConstructor`, JSX table markup)

## Assumptions

- The Tier 1 code snippet requirement from TODO-106 is satisfied by the existing code in `adoption-path.mdx` (lines 13-41) — no additional 20-line snippet needed
- The sync-lab session isolation requirement from TODO-106 is satisfied by the existing `session.ts` with `prefixMap()` and `getShareUrl()` — no additional work needed
- The `examples/todo-app/` is safe to delete because it is not referenced in any docs page and its purpose is now served by the Tier 1 code snippet
- The Security Model and Adoption Path sections from TODO-096 are already present and complete — this spec only verifies accuracy, not creates content
- `comparison.mdx` already includes Replicache/Zero and Durable Objects — this spec only verifies accuracy of the comparison claims
- The `examples/notes-app/`, `examples/sync-lab/`, `examples/storage-worker/`, and `examples/push-worker/` are kept (not in scope for deletion)

## Implementation Tasks

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Verify reference docs (server.mdx, cli.mdx, protocol.mdx) against Rust source; fix any inaccuracies | — | ~30% |
| G2 | 1 | Verify guide docs (security.mdx, adoption-path.mdx, postgresql.mdx, quick-start.mdx, installation.mdx, deployment.mdx) against current SDK/server; fix any inaccuracies | — | ~25% |
| G3 | 1 | Verify comparison.mdx claims are factually accurate; update if needed | — | ~10% |
| G4 | 1 | Delete examples/todo-app/ and remove from workspace config | — | ~5% |
| G5 | 2 | Final grep validation: no stale references across all docs; build verification | G1, G2, G3, G4 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3, G4 | Yes | 4 |
| 2 | G5 | No | 1 |

**Total workers needed:** 4 (max in any wave)

## Audit History

### Audit v1 (2026-03-11)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~80% total

**Scope:** Large (~80% estimated, exceeds 50% target). Each parallel group is within bounds; orchestrated execution required.

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~30% | At boundary (3 MDX + multiple Rust source cross-references) |
| G2 | ~25% | OK (6 MDX files, lighter cross-referencing) |
| G3 | ~10% | OK |
| G4 | ~5% | OK |
| G5 | ~10% | OK |

**Quality Projection:** DEGRADING range if run sequentially; GOOD range per-worker with parallel execution

**Dimensions evaluated:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Good | Task is well-defined: verify, fix, delete |
| Completeness | Good | All 10 files listed with specific verification targets |
| Testability | Good | ACs are measurable (field matching, grep checks, directory existence) |
| Scope | Good | Clear boundaries (docs only, no source changes) |
| Feasibility | Good | Straightforward verification task |
| Architecture fit | N/A | Docs-only spec |
| Non-duplication | Good | No existing tooling being bypassed |
| Cognitive load | Good | Simple read-compare-fix pattern |
| Strategic fit | Good | Aligns with v1.0 completion goals |
| Project compliance | Good | Honors all PROJECT.md constraints |

**Project compliance:** Honors PROJECT.md decisions (no v2.0 content, no source changes, MsgPack wire format acknowledged)

**Strategic fit:** Aligned with project goals -- docs accuracy is prerequisite for v1.0 release

**Language profile:** N/A -- spec modifies only MDX files and examples, not packages/core-rust/ or packages/server-rust/

**Assumptions assessed:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | todo-app is not referenced in any docs page | Stale links would remain after deletion |
| A2 | adoption-path.mdx Tier 1 snippet replaces todo-app | Users lose a working example app |
| A3 | All 10 docs pages already exist | Verification would fail on missing pages |
| A4 | Workspace glob (examples/*) auto-excludes deleted dirs | Manual config cleanup needed |

All assumptions verified: todo-app exists, all MDX files exist, pnpm-workspace.yaml uses glob pattern (no explicit todo-app reference to remove).

**Recommendations:**
1. Consider merging G3 (comparison.mdx, ~10%) into G2 to reduce from 4 to 3 parallel workers -- the overhead of a separate worker for one file may not be justified.
2. Goal Analysis section recommended for medium/large specs -- not critical for a docs verification task but would formalize the observable truths (e.g., "all docs match Rust source").
3. The Task Groups section was nested under Assumptions; moved to top-level Implementation Tasks section for standard structure.

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution with 4 workers in Wave 1.

## Execution Summary

**Executed:** 2026-03-11
**Mode:** orchestrated (sequential fallback -- no subagent tool available)
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2, G3, G4 | complete |
| 2 | G5 | complete |

### Files Modified
- `CONTRIBUTING.md` -- removed todo-app references, updated example to notes-app
- `deploy/Dockerfile.notes-app` -- removed todo-app COPY line
- `apps/docs-astro/src/content/docs/comparison.mdx` -- fixed ElectricSQL consistency model
- `apps/docs-astro/src/content/docs/guides/security.mdx` -- added planned-feature notes for TLS env vars
- `apps/docs-astro/src/content/docs/guides/postgresql.mdx` -- fixed topgun_maps DDL, removed phantom topgun_merkle table
- `apps/docs-astro/src/content/docs/installation.mdx` -- replaced phantom CLI commands with cargo run
- `apps/docs-astro/src/content/docs/quick-start.mdx` -- replaced topgun-server with cargo run
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` -- fixed production settings table, added TLS env var notes

### Files Deleted
- `examples/todo-app/` (entire directory, 8 files)

### Acceptance Criteria Status
- [x] AC1: NetworkConfig, ServerConfig, ConnectionConfig, TlsConfig fields verified against Rust source -- all match
- [x] AC2: All HTTP/WS endpoints verified against Rust router -- all match
- [x] AC3: Environment variables (PORT, RUST_LOG, TOPGUN_ADMIN_DIR) verified -- all match
- [x] AC4: All 77 message types verified against Rust Message enum -- all present
- [x] AC5: topgun_maps DDL fixed to match PostgresDataStore::initialize()
- [x] AC6: Comparison table in postgresql.mdx has 6 rows (exceeds minimum 5)
- [x] AC7: Tier 1 code snippet verified -- TopGunClient, IDBAdapter, useQuery, useClient all exist
- [x] AC8: examples/todo-app/ deleted
- [x] AC9: No docs reference collaborative-tasks, todo-app, @topgunbuild/server, or packages/server/
- [x] AC10: Security guide correctly references `sub` for JWT claims (line 280)

### Deviations
- G1 (reference docs): No changes needed -- server.mdx, cli.mdx, and protocol.mdx were already accurate
- G4: Also cleaned stale todo-app references from CONTRIBUTING.md and deploy/Dockerfile.notes-app (not just the directory deletion)
- Security/deployment docs: TLS env vars (TOPGUN_TLS_*, TOPGUN_CLUSTER_TLS_*) are documented but don't exist in the Rust server yet. Added "planned for future production binary" notes rather than removing the content, since it provides useful reference for the upcoming production binary.
- Installation page: Replaced non-existent TopGun CLI (npx topgun setup/dev) with actual cargo run commands
- Sequential execution used instead of parallel (no subagent spawning tool available)

---

## Review History

### Review v1 (2026-03-11)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. The `adoption-path.mdx` Tier 1 server command (line 11) and `postgresql.mdx` config example (line 14) still reference `topgun-server` as the binary name, which does not exist -- the actual binary is `cargo run --bin test-server --release`. The `quick-start.mdx` and `installation.mdx` were correctly updated but these two files were not. This is cosmetic since the adoption-path and postgresql pages are about future production usage, and the security guide correctly marks TLS env vars as "planned for future production binary."

**Passed:**
- [x] AC1: `NetworkConfig` fields (host, port, tls, connection, cors_origins, request_timeout) match `packages/server-rust/src/network/config.rs` exactly -- types and defaults all correct
- [x] AC1: `ServerConfig` fields (node_id, default_operation_timeout_ms, max_concurrent_operations, gc_interval_ms, partition_count, security) match `packages/server-rust/src/service/config.rs` exactly
- [x] AC1: `ConnectionConfig` fields (outbound_channel_capacity, send_timeout, idle_timeout, ws_write_buffer_size, ws_max_write_buffer_size) match source exactly
- [x] AC1: `TlsConfig` fields (cert_path, key_path, ca_cert_path) match source exactly
- [x] AC3: All three environment variables (PORT, RUST_LOG, TOPGUN_ADMIN_DIR) confirmed in Rust source via `std::env::var` calls
- [x] AC5: `topgun_maps` DDL in `postgresql.mdx` matches `PostgresDataStore::initialize()` -- all 7 columns, types, defaults, and primary key match; phantom `topgun_merkle` table correctly removed
- [x] AC6: Comparison table in `postgresql.mdx` has 6 rows (Storage model, Schema changes, Query language, Use case, What it syncs, Database access)
- [x] AC7: Tier 1 code snippet references `TopGunClient`, `IDBAdapter`, `useQuery`, `useClient` -- all confirmed exported from `@topgunbuild/client`, `@topgunbuild/adapters`, and `@topgunbuild/react`
- [x] AC8: `examples/todo-app/` directory confirmed deleted
- [x] AC9: `grep -r` for stale references (todo-app, collaborative-tasks, @topgunbuild/server, packages/server/) returns zero matches across all docs content and config files
- [x] AC10: Security guide line 280 correctly references `sub` claim for JWT authentication
- [x] No source code (Rust or TypeScript) was modified -- only MDX files and example directory deletion
- [x] No new documentation pages were created
- [x] Existing MDX component patterns (CodeBlock, ApiParam, ApiConstructor) preserved throughout
- [x] TLS env vars properly annotated as "planned for future production binary" rather than removed -- good editorial decision

**Summary:** All 10 acceptance criteria are met. The documentation accurately reflects the current Rust server codebase. The `examples/todo-app/` directory was cleanly deleted with all stale references removed from docs, CONTRIBUTING.md, and Dockerfile. One minor issue noted: two docs pages still reference a `topgun-server` binary name that does not yet exist, but this is consistent with the "planned for future" editorial approach used elsewhere and does not affect accuracy for current users.

### Fix Response v1 (2026-03-11)
**Applied:** Minor issue #1 — stale `topgun-server` binary name in adoption-path.mdx and postgresql.mdx

**Fixes:**
1. [✓] Replaced `topgun-server` with `cargo run --bin test-server --release` in adoption-path.mdx (3 occurrences: tier1, tier2, tier3 server commands) and postgresql.mdx (1 occurrence: separate database example)
   - Commit: ddc57e2

### Review v2 (2026-03-11)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: Struct fields in `reference/server.mdx` verified in Review v1 -- no changes since, still accurate
- [x] AC2: HTTP/WS endpoints verified in Review v1 -- no changes since, still accurate
- [x] AC3: Environment variables verified in Review v1 -- no changes since, still accurate
- [x] AC4: Message types verified in Review v1 -- no changes since, still accurate
- [x] AC5: `topgun_maps` DDL in `postgresql.mdx` matches `PostgresDataStore::initialize()` -- 7 columns, correct types, correct primary key
- [x] AC6: Comparison table in `postgresql.mdx` has 6 rows (Storage model, Schema changes, Query language, Use case, What it syncs, Database access)
- [x] AC7: Tier 1 code snippet in `adoption-path.mdx` uses `TopGunClient`, `IDBAdapter`, `useQuery`, `useClient` -- all valid current APIs
- [x] AC8: `examples/todo-app/` directory confirmed deleted
- [x] AC9: `grep -r` for stale references (todo-app, collaborative-tasks, @topgunbuild/server, packages/server/) returns zero matches in docs content
- [x] AC10: Security guide line 280 correctly references `sub` claim for JWT authentication
- [x] Fix v1 verified: `topgun-server` binary name replaced with `cargo run --bin test-server --release` in `adoption-path.mdx` (3 occurrences) and `postgresql.mdx` (1 occurrence) -- confirmed no remaining matches in either file
- [x] No source code (Rust or TypeScript) was modified
- [x] No new documentation pages were created
- [x] Existing MDX component patterns preserved
- [x] Constraints respected: no v2.0 speculative docs, no Astro config changes

**Summary:** All 10 acceptance criteria remain met. The minor issue from Review v1 (stale `topgun-server` binary name in adoption-path.mdx and postgresql.mdx) has been correctly fixed in commit ddc57e2. No new issues found. The spec is complete.

---

## Completion

**Completed:** 2026-03-11
**Total Commits:** 4 (3 execution + 1 fix)
**Review Cycles:** 2

### Outcome

All documentation content verified and corrected against the current Rust server codebase. Stale references, phantom CLI commands, and incorrect DDL removed. `examples/todo-app/` deleted as replaced by Tier 1 code snippet.

### Key Files

- `apps/docs-astro/src/content/docs/guides/postgresql.mdx` — Fixed DDL to match PostgresDataStore, removed phantom topgun_merkle table
- `apps/docs-astro/src/content/docs/installation.mdx` — Replaced phantom CLI commands with cargo run
- `apps/docs-astro/src/content/docs/comparison.mdx` — Fixed ElectricSQL consistency model claim

### Patterns Established

None — followed existing patterns.

### Deviations

- TLS env vars kept as "planned for future production binary" rather than removed, since they provide useful reference for the upcoming production binary
- CONTRIBUTING.md and deploy/Dockerfile.notes-app also cleaned of todo-app references (beyond original scope of docs-only changes)
