---
id: SPEC-114
type: refactor
status: done
priority: high
complexity: medium
created: 2026-03-10
---

# SPEC-114: Update Docker, Cluster CLI, and Docs for Rust Server

## Context

SPEC-084 removed the legacy TypeScript server package (`packages/server/`) and native package (`packages/native/`), but several deployment and tooling artifacts still reference the deleted TS server:

- `deploy/Dockerfile.server` builds and runs `packages/server` (Node.js)
- `deploy/Dockerfile.notes-app` copies `packages/server/package.json` (line 29)
- `docker-compose.yml` services use the TS-based Dockerfile
- `deploy/docker-compose.cluster.yml` uses the TS-based Dockerfile
- `tests/k6/docker-compose.cluster.yml` uses the TS-based Dockerfile
- `bin/commands/cluster/start.js` spawns `examples/simple-server.ts` via `npx tsx` (deleted file)
- `CONTRIBUTING.md` references `@topgunbuild/server` package and `packages/server/` directory
- `README.md` lists `@topgunbuild/server` as a package
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` describes TS server Docker build
- `.gitignore` has a stale `packages/server/test/fixtures/` entry and a stale `packages/native/build/` entry
- `packages/node_modules/` contains a 12K stale artifact (only `.pnpm` symlinks to deleted `node-addon-api`)

This is a continuation of SPEC-084 (legacy TS server removal).

## Goal Statement

After this spec is complete, all Docker images, compose files, CLI commands, and documentation correctly target the Rust server binary. No references to the deleted TypeScript server remain in operational files.

### Observable Truths

1. `docker build -f deploy/Dockerfile.server .` produces a container running the Rust `test-server` binary
2. `docker-compose up server` starts a Rust server container connected to PostgreSQL
3. `npx topgun cluster:start` spawns Rust server binaries (not `npx tsx simple-server.ts`)
4. `deploy/Dockerfile.notes-app` builds without referencing `packages/server/package.json`
5. `grep -r "packages/server" .` returns zero hits outside of `specifications/` (historical docs) and `.specflow/` (archived specs)
6. `packages/node_modules/` directory does not exist

## Task

Rewrite Docker infrastructure, cluster CLI, and documentation to use the Rust server. Remove all remaining references to the deleted TypeScript server.

## Requirements

### Files to Rewrite

#### 1. `deploy/Dockerfile.server` (rewrite)

Rewrite as a multi-stage Rust build:

- **Stage 1 (builder):** Use `rust:1.85-bookworm` (or latest stable). Copy `Cargo.toml`, `Cargo.lock`, `packages/core-rust/`, `packages/server-rust/`. Run `cargo build --release --bin test-server`.
- **Stage 2 (runtime):** Use `debian:bookworm-slim`. Install `ca-certificates`, `libssl3`, and `wget` (for health check). Copy the compiled binary from the builder stage and rename it: `COPY --from=builder /app/target/release/test-server /usr/local/bin/topgun-server`. Run as non-root user. Health check on `${TOPGUN_PORT:-8080}/health`. Expose ports 8080, 9080, 9090. `CMD ["/usr/local/bin/topgun-server"]`.
- Remove all Node.js, pnpm, and `packages/server` references.
- Remove `NODE_ENV` from environment — Rust server uses `RUST_LOG`.

#### 2. `deploy/Dockerfile.notes-app` (modify)

- Remove line 29: `COPY packages/server/package.json packages/server/package.json`
- The notes-app is a client-only Vite app; it does not need the server package.

#### 3. `docker-compose.yml` (modify)

- **`server` service:** Remove `NODE_ENV` env var. Add `RUST_LOG: topgun_server=info`. Keep `DATABASE_URL`, `TOPGUN_PORT`, `TOPGUN_CLUSTER_PORT`.
- **`server-auto` service:** Same treatment — replace `NODE_ENV` with `RUST_LOG`. Remove `TOPGUN_AUTO_SETUP`, `TOPGUN_STORAGE_TYPE`, `TOPGUN_DEPLOYMENT_MODE`, `TOPGUN_MCP_ENABLED`, `TOPGUN_VECTOR_ENABLED`, `TOPGUN_ADMIN_*` env vars (these are TS server concepts not implemented in Rust server). Remove the `secrets` reference from the `server-auto` service. Remove the root-level `secrets:` block (lines 271-273: `admin_password` file reference) — after the `server-auto` simplification no service references this secret and the block would be orphaned. Simplify to match `server` service with different profile.
- **`node-1`, `node-2`, `node-3` (cluster profile):** Remove `NODE_ENV` references if present. Env vars `NODE_ID`, `TOPGUN_PORT`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_PEERS`, `DATABASE_URL` remain (Rust server reads these).

#### 4. `deploy/docker-compose.cluster.yml` (modify)

- Remove all TS-specific env vars from all 3 nodes: `TOPGUN_GRADUAL_REBALANCING`, `TOPGUN_REPLICATION_ENABLED`, `TOPGUN_MIGRATION_BATCH_SIZE`, `TOPGUN_MIGRATION_PARALLEL_TRANSFERS`, `TOPGUN_REPLICATION_CONSISTENCY`, `TOPGUN_REPLICATION_QUEUE_SIZE` (these were TS server config, not read by Rust server).
- Add `RUST_LOG: topgun_server=info` to each node.
- Keep `NODE_ID`, `TOPGUN_HOST`, `TOPGUN_PORT`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_METRICS_PORT`, `TOPGUN_DISCOVERY`, `TOPGUN_PEERS`.

#### 5. `tests/k6/docker-compose.cluster.yml` (modify)

- No TS-specific env vars to remove (already clean), but all nodes reference `deploy/Dockerfile.server` which will now build Rust. Verify env vars are compatible. Add `RUST_LOG: topgun_server=info`.

#### 6. `bin/commands/cluster/start.js` (rewrite)

- Rewrite to spawn Rust server binaries instead of `npx tsx simple-server.ts`.
- Follow the pattern established in `bin/commands/dev.js`: check for `target/release/test-server` binary, error if not found with build instructions.
- For each node, spawn the Rust binary with env vars: `PORT`, `NODE_ID`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_PEERS`.
- Remove `examples/simple-server.ts` reference.

### Files to Modify (documentation cleanup)

#### 7. `CONTRIBUTING.md` (modify)

- Replace `packages/server/` directory listing with `packages/server-rust/` and note it is Rust.
- Replace `@topgunbuild/server` with `packages/server-rust` in package hierarchy.
- Remove `pnpm --filter @topgunbuild/server test` — replace with `cargo test --release -p topgun-server`.

#### 8. `README.md` (modify)

- Replace `@topgunbuild/server | WebSocket server, clustering, storage adapters` row with `server-rust | Rust WebSocket server (axum), clustering, PostgreSQL` or similar.

#### 9. `apps/docs-astro/src/content/docs/guides/deployment.mdx` (modify)

- Update `dockerBuildCode` to show Rust Dockerfile build.
- Update `dockerComposeCode` to remove `NODE_ENV`, use Rust-appropriate env vars.
- Update `dockerComposeTlsCode` similarly.
- Update the prose around line 286-292: remove "Building all packages (core, server)" and "Starting the server via `node packages/server/dist/start-server.js`". Replace with Rust binary description.

### Files to Delete

#### 10. `packages/node_modules/` (delete directory)

- Contains only stale `.pnpm` symlinks to `node-addon-api` (from deleted `packages/native/`).
- Safe to delete entirely. Add `packages/node_modules/` to `.gitignore` if not already covered by existing patterns.

### Files to Clean Up

#### 11. `.gitignore` (modify)

- Remove line 72: `# packages/server/test/fixtures/` (stale comment referencing deleted directory).
- Remove line 75: `packages/native/build/` (stale entry — `packages/native/` was deleted in SPEC-084).

## Acceptance Criteria

1. `deploy/Dockerfile.server` builds a Rust binary container (no Node.js layers)
2. `deploy/Dockerfile.notes-app` does not reference `packages/server/`
3. `docker-compose.yml` server services use Rust-compatible env vars (no `NODE_ENV`)
4. `deploy/docker-compose.cluster.yml` nodes use Rust-compatible env vars (no TS-specific cluster config)
5. `tests/k6/docker-compose.cluster.yml` nodes have `RUST_LOG` env var
6. `bin/commands/cluster/start.js` spawns Rust binaries, not `npx tsx`
7. `CONTRIBUTING.md` references `server-rust` package, not `@topgunbuild/server`
8. `README.md` references `server-rust` package, not `@topgunbuild/server`
9. `apps/docs-astro/.../deployment.mdx` describes Rust server Docker deployment
10. `packages/node_modules/` directory is deleted
11. `.gitignore` does not contain `packages/server/test/fixtures/` reference or `packages/native/build/` reference
12. `grep -rn "packages/server[^-]" . --include='*.{yml,yaml,js,mdx,md}' | grep -v specifications/ | grep -v .specflow/` returns zero results (excluding historical spec docs)

## Validation Checklist

1. Run `docker build -f deploy/Dockerfile.server .` (or verify Dockerfile syntax is valid) -- builds without error
2. Run `grep -rn "@topgunbuild/server" . --include='*.{yml,yaml,js,jsx,ts,tsx,md,mdx,json}' | grep -v node_modules | grep -v .specflow | grep -v specifications | grep -v CHANGELOG` -- zero results
3. Run `grep -rn "simple-server.ts" .` -- zero results
4. Run `grep -rn "start-server.js" . | grep -v .specflow | grep -v specifications` -- zero results
5. Verify `packages/node_modules/` does not exist

## Constraints

- Do NOT modify any Rust server code (`packages/server-rust/`). This spec is purely Docker/config/docs.
- Do NOT modify `packages/core-rust/` or any other Rust crate.
- Do NOT change the Rust binary name (`test-server`) or its Cargo configuration.
- Do NOT remove `specifications/` directory files (historical technical docs, not operational).
- Do NOT remove `CHANGELOG.md` references (historical records).
- The `server-auto` service simplification removes env vars that the Rust server does not implement; do not invent new Rust server config to replace them.

## Assumptions

- The Rust server binary is `test-server` (built via `cargo build --release --bin test-server`), matching the existing `Cargo.toml` `[[bin]]` section and `bin/commands/dev.js`. The Dockerfile COPY renames the binary to `topgun-server` for a cleaner container image name (`COPY --from=builder /app/target/release/test-server /usr/local/bin/topgun-server`).
- The Rust server reads env vars `PORT`/`TOPGUN_PORT`, `NODE_ID`, `TOPGUN_CLUSTER_PORT`, `TOPGUN_PEERS`, `DATABASE_URL`, `RUST_LOG`, and `JWT_SECRET` based on existing integration test harness and dev.js patterns.
- `rust:1.85-bookworm` is an appropriate builder image; the exact Rust version can be adjusted during implementation.
- The `server-auto` service in docker-compose.yml can be simplified to a basic Rust server with a different profile, since the auto-setup features (admin user creation, MCP toggle, vector toggle) were TS server features not ported to Rust.
- `specifications/TECHNICAL_SUMMARY.md` and `specifications/08_FULLTEXT_SEARCH.md` contain historical `packages/server/` references that are acceptable to leave in place (not operational files).

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Rewrite `deploy/Dockerfile.server` for Rust multi-stage build | -- | ~20% |
| G2 | 1 | Rewrite `bin/commands/cluster/start.js` to spawn Rust binaries | -- | ~15% |
| G3 | 2 | Update `docker-compose.yml`, `deploy/docker-compose.cluster.yml`, `tests/k6/docker-compose.cluster.yml` for Rust env vars | G1 | ~20% |
| G4 | 2 | Fix `deploy/Dockerfile.notes-app`, delete `packages/node_modules/`, clean `.gitignore` | G1 | ~10% |
| G5 | 3 | Update `CONTRIBUTING.md`, `README.md`, `deployment.mdx` docs | G3 | ~20% |
| G6 | 3 | Comprehensive sweep: verify zero remaining TS server references in operational files | G4, G5 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5, G6 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-10)
**Status:** NEEDS_REVISION

**Context Estimate:** ~55% total (6 groups, 11 files, straightforward 1.0x multiplier)

**Critical:**

1. **Orphaned `secrets` block in `docker-compose.yml`:** The spec instructs removing the `secrets` reference from the `server-auto` service, but does not mention removing the root-level `secrets:` block (lines 271-273: `admin_password` file reference). After the `server-auto` simplification, no service references this secret, leaving an orphaned block that references `./secrets/admin_password.txt` (which may not exist). The spec should explicitly state: remove the root-level `secrets:` block from `docker-compose.yml`.

2. **Dockerfile CMD vs binary name mismatch:** The spec says `CMD ["/usr/local/bin/topgun-server"]` in the runtime stage but the binary is built as `test-server` (`cargo build --release --bin test-server`). The implementer must either rename the binary during COPY (e.g., `COPY --from=builder /app/target/release/test-server /usr/local/bin/topgun-server`) or use the actual name. This ambiguity should be resolved explicitly in the spec -- state whether to rename during COPY or use `/usr/local/bin/test-server` as the CMD.

3. **Missing `packages/native/build/` cleanup in `.gitignore`:** The spec removes the stale `# packages/server/test/fixtures/` comment (line 72) but does not address line 75: `packages/native/build/` which is equally stale -- `packages/native/` was deleted in SPEC-084. This should be removed in the same cleanup pass.

**Recommendations:**

4. **`deploy/docker-compose.cluster.yml` has stale "Phase 4" comments:** Lines 1-11 contain TS-era comments referencing "Phase 4: Clustering Improvements", "Gradual rebalancing enabled", "Async replication pipeline". These should be updated to describe the Rust server cluster when the TS-specific env vars are removed.

5. **`tests/k6/docker-compose.cluster.yml` has stale "Phase 4.5" comments:** Lines 1-12 reference "Phase 4.5 Task 07" which is a TS-era planning artifact. Consider updating when adding `RUST_LOG`.

6. **`CONTRIBUTING.md` has additional stale references:** Lines 71-72 reference `tests/e2e/` (deleted) and `tests/load/` (actual directory is `tests/k6/`). While not directly about the TS server, if CONTRIBUTING.md is being updated anyway, these could be corrected in the same pass. This is optional -- out of strict scope but cheap to fix.

7. **Healthcheck tool in Dockerfile runtime stage:** The spec uses `wget` for the health check (`wget -q --spider http://localhost:${TOPGUN_PORT:-8080}/health`), but `debian:bookworm-slim` does not include `wget` by default. Either install `wget` alongside `ca-certificates`, use `curl` (also not installed by default), or consider adding a `/health` check via the binary itself. Alternatively, install `wget` in the `RUN apt-get` step.

8. **`docker-compose.yml` cluster nodes missing `RUST_LOG`:** The spec says to remove `NODE_ENV` from cluster nodes if present (they don't have it) and keep the existing env vars, but does not explicitly say to add `RUST_LOG` to the cluster profile nodes (node-1, node-2, node-3). The `server` service gets `RUST_LOG` but the cluster nodes are not mentioned for it. Consider adding `RUST_LOG: topgun_server=info` for consistency.

### Response v1 (2026-03-10)
**Applied:** Critical items 1, 2, 3

**Changes:**
1. [✓] Orphaned `secrets` block in `docker-compose.yml` — Added explicit instruction in Requirement 3 (`server-auto` service) to remove both the `secrets` reference from the service and the root-level `secrets:` block (lines 271-273).
2. [✓] Dockerfile CMD vs binary name mismatch — Resolved in Requirement 1 by specifying the COPY rename strategy: `COPY --from=builder /app/target/release/test-server /usr/local/bin/topgun-server`. Also updated Assumptions section to document this decision. Also added `wget` to the install step (overlapping fix for recommendation 7 since the Requirement 1 rewrite was the correct place to state it).
3. [✓] Missing `packages/native/build/` cleanup in `.gitignore` — Added removal of line 75 (`packages/native/build/`) to Requirement 11 (`.gitignore` modify). Updated Acceptance Criterion 11 to cover both stale entries.

**Skipped:** Recommendations 4, 5, 6, 7, 8 — not in revision scope.

### Audit v2 (2026-03-10)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~55% total (6 groups, 11 files, straightforward 1.0x multiplier)

**Scope:** Large (~55% estimated, exceeds 50% target)

**Per-Group Breakdown:**

| Group | Est. Context | Status |
|-------|--------------|--------|
| G1 | ~20% | OK |
| G2 | ~15% | OK |
| G3 | ~20% | OK |
| G4 | ~10% | OK |
| G5 | ~20% | OK |
| G6 | ~15% | OK |

**Quality Projection:** DEGRADING range (50-70%)

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution across 3 waves with 2 workers max.

**Recommendations:**

1. **Stale comments in compose files (carried from v1 items 4-5):** `deploy/docker-compose.cluster.yml` lines 1-11 reference "Phase 4: Clustering Improvements" and `tests/k6/docker-compose.cluster.yml` lines 1-12 reference "Phase 4.5 Task 07". These TS-era comments should be updated when removing TS-specific env vars. Cheap to fix during G3 execution.

2. **CONTRIBUTING.md stale test directory references (carried from v1 item 6):** Lines 71-72 reference `tests/e2e/` (deleted) and `tests/load/` (should be `tests/k6/`). Cheap to fix during G5 execution.

3. **docker-compose.yml cluster nodes missing RUST_LOG (carried from v1 item 8):** The cluster profile nodes (node-1, node-2, node-3) in `docker-compose.yml` do not get `RUST_LOG` added, unlike the `server` service and nodes in `deploy/docker-compose.cluster.yml`. Consider adding for consistency during G3 execution.

**Comment:** All three critical issues from Audit v1 have been properly addressed in the revision. The spec is clear, complete, and implementable. The remaining recommendations are all low-effort improvements that can be incorporated during execution. The spec exceeds the 50% context threshold due to touching 11 files across 6 task groups, requiring orchestrated execution.

---

## Execution Summary

**Executed:** 2026-03-10
**Commits:** 5

### Files Created
(none — all rewrites/modifications)

### Files Modified
- `deploy/Dockerfile.server` — rewritten as Rust multi-stage build (rust:1.85-bookworm builder, debian:bookworm-slim runtime)
- `bin/commands/cluster/start.js` — rewritten to spawn Rust binary with PORT, NODE_ID, TOPGUN_CLUSTER_PORT, TOPGUN_PEERS env vars
- `docker-compose.yml` — removed NODE_ENV, secrets block, TS-specific env vars from server-auto; added RUST_LOG to server, server-auto, and all cluster nodes
- `deploy/docker-compose.cluster.yml` — removed TS-specific env vars (TOPGUN_GRADUAL_REBALANCING, etc.); added RUST_LOG; updated header comments
- `tests/k6/docker-compose.cluster.yml` — added RUST_LOG to all 4 nodes; updated header comment
- `deploy/Dockerfile.notes-app` — removed `COPY packages/server/package.json` line
- `.gitignore` — removed stale `packages/server/test/fixtures/` comment and `packages/native/build/` entry
- `CONTRIBUTING.md` — replaced packages/server with server-rust, fixed test dirs (e2e→integration-rust, load→k6), updated package hierarchy
- `README.md` — replaced @topgunbuild/server with server-rust in packages table
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — updated Dockerfile description, replaced NODE_ENV with RUST_LOG in compose examples

### Files Deleted
- `packages/node_modules/` — deleted stale .pnpm symlinks to node-addon-api

### Acceptance Criteria Status
- [x] AC1: deploy/Dockerfile.server builds a Rust binary container (no Node.js layers)
- [x] AC2: deploy/Dockerfile.notes-app does not reference packages/server/
- [x] AC3: docker-compose.yml server services use Rust-compatible env vars (no NODE_ENV)
- [x] AC4: deploy/docker-compose.cluster.yml nodes use Rust-compatible env vars (no TS-specific cluster config)
- [x] AC5: tests/k6/docker-compose.cluster.yml nodes have RUST_LOG env var
- [x] AC6: bin/commands/cluster/start.js spawns Rust binaries, not npx tsx
- [x] AC7: CONTRIBUTING.md references server-rust package, not @topgunbuild/server
- [x] AC8: README.md references server-rust package, not @topgunbuild/server
- [x] AC9: apps/docs-astro/.../deployment.mdx describes Rust server Docker deployment
- [x] AC10: packages/node_modules/ directory is deleted
- [x] AC11: .gitignore does not contain packages/server/test/fixtures/ or packages/native/build/ references
- [x] AC12: grep -rn "packages/server[^-]" returns zero results in operational files

### Deviations
1. [Rule 2 - Missing Critical] Added RUST_LOG to docker-compose.yml cluster nodes (node-1, node-2, node-3) — audit recommendation 3 incorporated for consistency
2. [Rule 2 - Missing Critical] Updated stale header comments in deploy/docker-compose.cluster.yml and tests/k6/docker-compose.cluster.yml — audit recommendations 1, 2 incorporated
3. [Rule 2 - Missing Critical] Fixed stale test directory references in CONTRIBUTING.md (tests/e2e/ → tests/integration-rust/, tests/load/ → tests/k6/) — audit recommendation 2 from v2 incorporated

### Notes
- `.claude/settings.local.json` contains historical @topgunbuild/server permission entries — these are auto-generated, not version-controlled, and not operational files
- packages/node_modules/ was tracked in git via .pnpm symlinks; deleted from working tree (was not in git staging area since parent node_modules/ is in .gitignore)

---

## Review History

### Review v1 (2026-03-10)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [v] AC1: `deploy/Dockerfile.server` is a clean multi-stage Rust build (rust:1.85-bookworm builder, debian:bookworm-slim runtime). No Node.js, pnpm, or TS references. Binary correctly renamed via COPY. Non-root user, health check with wget (installed in apt-get step), correct ports exposed.
- [v] AC2: `deploy/Dockerfile.notes-app` has no reference to `packages/server/`. The stale COPY line has been removed.
- [v] AC3: `docker-compose.yml` has no `NODE_ENV`, no `secrets` block, no TS-specific env vars. `RUST_LOG: topgun_server=info` present on server, server-auto, and all 3 cluster nodes.
- [v] AC4: `deploy/docker-compose.cluster.yml` has no TS-specific env vars (TOPGUN_GRADUAL_REBALANCING, etc.). All 3 nodes have `RUST_LOG`. Header comments updated to describe Rust server cluster.
- [v] AC5: `tests/k6/docker-compose.cluster.yml` has `RUST_LOG` on all 4 nodes. Header comment updated.
- [v] AC6: `bin/commands/cluster/start.js` spawns `target/release/test-server` Rust binary with proper env vars (PORT, NODE_ID, TOPGUN_CLUSTER_PORT, TOPGUN_PEERS). No npx tsx or simple-server.ts references. Clean error message if binary not found.
- [v] AC7: `CONTRIBUTING.md` references `server-rust/` and `core-rust/` in project structure. Package hierarchy shows Rust server. Test command uses `cargo test --release -p topgun-server`. Test directories corrected (integration-rust, k6).
- [v] AC8: `README.md` lists `server-rust | Rust WebSocket server (axum), clustering, PostgreSQL` in packages table.
- [v] AC9: `deployment.mdx` describes Rust multi-stage build, uses `RUST_LOG` in compose examples, no NODE_ENV or start-server.js references. TLS example also uses Rust-appropriate env vars.
- [v] AC10: `packages/node_modules/` directory does not exist (verified).
- [v] AC11: `.gitignore` has no `packages/server/test/fixtures/` or `packages/native/build/` references (verified).
- [v] AC12: `grep -rn "packages/server[^-]"` returns zero results in operational files (verified). The only `@topgunbuild/server` references are in `.claude/settings.local.json` which is auto-generated and gitignored.

**Minor:**
1. Validation checklist items 1-4 all return zero results. Validation item 1 (docker build) was syntax-verified rather than actually built, which is acceptable given the spec constraint is Docker/config/docs only.

**Summary:** All 12 acceptance criteria are met. Every modified file correctly targets the Rust server. All TS server references have been eliminated from operational files. The Dockerfile is well-structured with proper security practices (non-root user, minimal runtime image, health check). The cluster CLI correctly spawns Rust binaries. Documentation is accurate and consistent. All audit recommendations (stale comments, RUST_LOG on cluster nodes, test directory fixes) were incorporated as beneficial deviations. No critical or major issues found.

---

## Completion

**Completed:** 2026-03-10
**Total Commits:** 5
**Review Cycles:** 1

### Outcome

All Docker infrastructure, cluster CLI, and documentation updated to target the Rust server binary. Zero remaining references to the deleted TypeScript server in operational files.

### Key Files

- `deploy/Dockerfile.server` — Rust multi-stage build (builder + slim runtime with non-root user)
- `bin/commands/cluster/start.js` — Spawns Rust `test-server` binary for local cluster development
- `docker-compose.yml` — Production compose with Rust-compatible env vars and RUST_LOG

### Patterns Established

None — followed existing patterns.

### Deviations

3 beneficial deviations incorporated from audit recommendations (RUST_LOG on cluster nodes, stale compose comments updated, CONTRIBUTING.md test dirs fixed).
