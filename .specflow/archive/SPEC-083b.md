---
id: SPEC-083b
type: docs
status: done
priority: P2
complexity: medium
created: 2026-03-08
parent: SPEC-083
depends_on: [SPEC-083a]
---

# New Guides, Comparison Update, and Code Snippet Audit

## Context

The Rust server migration is complete. New documentation pages are needed for adoption guidance, PostgreSQL positioning, and security model. The comparison page is missing key competitors. All existing code snippets must be audited for correctness against the current API. The quick-start page needs updating to reference the Rust server.

**Parent:** SPEC-083 (Update Documentation Content for Rust Server)
**Source TODO:** TODO-106
**Depends on:** SPEC-083a (reference docs must be settled before quick-start and snippet audit)

**Note:** Requirements R1-R3 and R9 are defined in SPEC-083a. This spec continues from R4.

## Task

Create new guide pages (adoption-path, postgresql), extend security and comparison pages, update quick-start, and audit all code snippets across docs.

## Requirements

### R4: Add Security Model Section to `guides/security.mdx`

**File:** `apps/docs-astro/src/content/docs/guides/security.mdx`

- Add a "Security Model" section at the top explaining the overall threat model
- Cover: JWT authentication (standard `sub` claim), TLS for client connections, mTLS for cluster, RBAC with roles
- Reference content from TODO-096 outputs (SPEC-077 adoption docs)
- Replace the existing `ServerFactory.create({...})` TS snippet with Rust server configuration: use environment variables for simple cases (e.g., `PORT=8080 DATABASE_URL=... topgun-server`) and link to `reference/server.mdx` (from SPEC-083a) for the full Rust embed API (`NetworkConfig`, `TlsConfig`). Do not use TS `ServerFactory` code.

### R5: Create Adoption Path Guide

**File:** `apps/docs-astro/src/content/docs/guides/adoption-path.mdx` (new)

- Document the 3-tier adoption model from PRODUCT_CAPABILITIES.md:
  - **Tier 1:** Add TopGun alongside existing app (real-time features only)
  - **Tier 2:** TopGun as primary data layer with PostgreSQL persistence
  - **Tier 3:** Full platform (cluster, search, processing)
- Include a 20-line Tier 1 code snippet covering both client-side React code (`client.getMap()`, `useQuery()`) and a brief server start command using environment variables (e.g., `PORT=8080 DATABASE_URL=... topgun-server`). The snippet should show users need both a running server and client-side integration.
- Each tier gets: description, code example, when to use, what you gain

### R6: Create "TopGun + Your PostgreSQL" Guide

**File:** `apps/docs-astro/src/content/docs/guides/postgresql.mdx` (new)

- Explain that TopGun is NOT a sync layer over existing SQL schemas
- TopGun uses PostgreSQL as a durability backend with its own storage format (MsgPack BYTEA)
- Coexistence = same PostgreSQL instance, separate `topgun_*` tables
- Include comparison table:

| Aspect | PowerSync / ElectricSQL | TopGun |
|--------|------------------------|--------|
| Storage model | Syncs your existing schema | Owns its storage, lives alongside |
| Schema changes | Must match source DB | Independent |
| Query language | SQL on your tables | TopGun queries on TopGun data |
| Use case | Offline-first SQL apps | Real-time reactive features |

- Show configuration: `DATABASE_URL` pointing to user's existing PostgreSQL instance

### R7: Update `comparison.mdx` with New Competitors

**File:** `apps/docs-astro/src/content/docs/comparison.mdx`

- Add columns for **Replicache/Zero** and **Cloudflare Durable Objects**
- The existing `ComparisonRow` component only supports 4 columns (`tg`, `es`, `fb`, `rx`) and must NOT be modified. Replace the `ComparisonRow`-based table with a raw HTML/MDX table using standard `<table>`, `<thead>`, `<tbody>`, `<tr>`, `<td>` markup to support all 6 columns: TopGun, ElectricSQL, Firebase, RxDB, Replicache/Zero, Durable Objects
- Remove the `ComparisonRow` import line (it will be unused after replacing with raw HTML table markup)
- Replicache/Zero: client-side sync, no server cluster, SaaS pricing
- Cloudflare Durable Objects: edge compute, proprietary platform, per-object pricing
- Add "vs Replicache/Zero" and "vs Durable Objects" entries in the "Why TopGun?" section

### R8: Update `quick-start.mdx` with Tier 1 Snippet

**File:** `apps/docs-astro/src/content/docs/quick-start.mdx`

- Ensure the quick-start shows the Rust server binary (not TS server)
- Include the 20-line Tier 1 snippet or link to the adoption-path guide
- Update install instructions if package names changed

### R10: Verify All Code Snippets

- Audit every `.mdx` file for code examples referencing `@topgunbuild/server` TypeScript API
- Update any that reference removed/changed APIs
- Verify client-side snippets (`@topgunbuild/client`, `@topgunbuild/react`) still work
- Key files to audit: `installation.mdx`, `quick-start.mdx`, all `guides/*.mdx`, all `reference/*.mdx`
- **Universal replacement pattern:** For each `ServerFactory.create({...})` snippet, replace the server-side configuration portion with environment variable examples (same pattern as R4: e.g., `PORT=8080 DATABASE_URL=... topgun-server`) and add a link to `reference/server.mdx` for the full Rust embed API. For `deployment.mdx` serverless snippets (`HttpSyncHandler` from `@topgunbuild/server/coordinator`), replace with a note that the Rust server does not support serverless edge deployment and link to `reference/server.mdx` for supported deployment modes.
- Known files containing `@topgunbuild/server` references (prioritize these):
  - `guides/security.mdx` (ServerFactory TLS config)
  - `guides/authentication.mdx` (ServerFactory JWT config)
  - `guides/interceptors.mdx` (ServerFactory interceptors)
  - `guides/full-text-search.mdx` (ServerFactory search setup)
  - `guides/cluster-replication.mdx` (ServerFactory cluster setup)
  - `guides/deployment.mdx` (ServerFactory production config, HttpSyncHandler serverless snippets)
  - `guides/rbac.mdx` (ServerFactory RBAC config)
  - `guides/performance.mdx` (ServerFactory performance configs)
  - `guides/event-journal.mdx` (ServerFactory event journal)
  - `reference/adapter.mdx` (PostgresAdapter imports)
  - `reference/index.mdx` (description text referencing `@topgunbuild/server` -- update to reference Rust server)
  - `blog/full-text-search-offline-first.mdx` (npm install @topgunbuild/server)
  - `blog/distributed-live-subscriptions.mdx` (npm install @topgunbuild/server)

## Acceptance Criteria

1. `guides/security.mdx` has a "Security Model" section covering JWT (`sub` claim), TLS, mTLS, RBAC
2. `guides/adoption-path.mdx` exists with 3 tiers, each with code example, and a 20-line Tier 1 snippet covering both client React code and server start command
3. `guides/postgresql.mdx` exists with PowerSync/ElectricSQL vs TopGun comparison table and `DATABASE_URL` configuration example
4. `comparison.mdx` table has 6 columns (TopGun, ElectricSQL, Firebase, RxDB, Replicache/Zero, Durable Objects) using raw HTML/MDX table markup (not `ComparisonRow` component, which only supports 4 columns)
5. `quick-start.mdx` references the Rust server binary, not `@topgunbuild/server`
6. No docs page contains `@topgunbuild/server` code snippets as primary server setup (client-side usage of `@topgunbuild/client` is fine)
7. `pnpm start:docs` builds without errors

## Constraints

- Do NOT remove client-side TypeScript documentation (`@topgunbuild/client`, `@topgunbuild/react` remain TS packages)
- Do NOT change the docs site framework (Astro + MDX + React components)
- Do NOT modify the `ComparisonRow` component -- it only supports 4 columns (`tg`, `es`, `fb`, `rx`). Use raw HTML/MDX table markup for the 6-column comparison table.
- Follow existing MDX conventions (frontmatter `order` field, breadcrumb pattern, component imports)

## Assumptions

- PRODUCT_CAPABILITIES.md and STRATEGIC_RECOMMENDATIONS.md in `.specflow/reference/` contain the source content for adoption tiers and PostgreSQL positioning.
- The `ComparisonRow` component only accepts 4 fixed props (`tg`, `es`, `fb`, `rx`) and cannot support 6 columns. Raw HTML/MDX table markup is required.
- The security guide already has substantial TLS/mTLS content; the new "Security Model" section is an addition at the top, not a rewrite.
- SPEC-083a is complete, so `reference/server.mdx` and `reference/cli.mdx` already show Rust server content.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add Security Model section to `guides/security.mdx` | -- | ~15% |
| G2 | 1 | Create `guides/adoption-path.mdx` with 3-tier model | -- | ~20% |
| G3 | 1 | Create `guides/postgresql.mdx` with comparison table | -- | ~15% |
| G4 | 1 | Update `comparison.mdx` with Replicache/Zero and Durable Objects (raw HTML table, remove ComparisonRow import) | -- | ~15% |
| G5 | 2 | Update `quick-start.mdx` for Rust server | G2 | ~10% |
| G6a | 2 | Audit guide files for stale `@topgunbuild/server` snippets: `authentication.mdx`, `interceptors.mdx`, `full-text-search.mdx`, `cluster-replication.mdx`, `deployment.mdx`, `rbac.mdx`, `performance.mdx`, `event-journal.mdx`, `reference/adapter.mdx` (9 files, ServerFactory replacements using universal pattern from R10) | G1, G2, G3, G4 | ~18% |
| G6b | 2 | Audit blog + reference files for stale `@topgunbuild/server` references: `blog/full-text-search-offline-first.mdx`, `blog/distributed-live-subscriptions.mdx`, `reference/index.mdx` (3 files, install line and description text updates) | G1, G2, G3, G4 | ~7% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3, G4 | Yes | 4 |
| 2 | G5, G6a, G6b | Yes | 3 |

**Total workers needed:** 4 (max in any wave)

## Audit History

### Audit v1 (2026-03-09)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% cumulative, ~25% max per worker (parallel execution)

**Critical:**
1. R7 instructs "Add `ComparisonRow` entries using the existing component pattern" but `ComparisonRow` only accepts 4 fixed props (`tg`, `es`, `fb`, `rx`) -- it cannot accept additional columns. The constraint says "Do NOT modify the `ComparisonRow` component" then says "extend it or use raw table markup." R7 must be rewritten to explicitly state: use raw HTML table markup for the 6-column comparison (since the component cannot be modified). Remove the instruction to "Add `ComparisonRow` entries" for the new columns.
2. R4 says "Update code snippets to use Rust server configuration" but does not specify what the Rust server configuration format looks like. The existing `security.mdx` has a `ServerFactory.create({...})` TypeScript snippet (lines 10-34). The spec must clarify what replaces it -- CLI flags (`topgun-server --tls-cert /path`), TOML config file, environment variables only, or a reference to `reference/server.mdx` (from SPEC-083a). Without this, the implementer will guess.

**Recommendations:**
3. Requirement numbering skips R1-R3 and R9 (presumably in SPEC-083a). Add a brief note like "R1-R3 and R9 are in SPEC-083a" for clarity.
4. R5 mentions "Rust server with PostgresDataStore" in the 20-line Tier 1 snippet, but the Tier 1 description says "Add TopGun alongside existing app (real-time features only)." The server-side code in a Tier 1 snippet may confuse users who only want to add client-side real-time features. Clarify whether Tier 1 snippet is client-only or includes server setup.
5. AC4 should note that the 6-column table uses raw HTML markup (not ComparisonRow), so the implementer knows the expected approach upfront.
6. G6 scope ("audit all .mdx files") covers 41 files. Consider listing which files are known to contain `@topgunbuild/server` references to reduce audit scope and context usage. A quick grep shows `security.mdx` (ServerFactory on line 10) is the primary offender; listing known files would help the implementer prioritize.

### Response v1 (2026-03-09)
**Applied:** All critical issues (1-2) and all recommendations (3-6)

**Changes:**
1. [✓] R7 ComparisonRow limitation — Rewrote R7 to explicitly require raw HTML/MDX table markup for the 6-column comparison. Removed "Add ComparisonRow entries" instruction. Updated Constraints and Assumptions to state ComparisonRow only supports 4 columns.
2. [✓] R4 Rust server config format — Expanded R4 bullet to specify: replace ServerFactory TS snippet with env var config for simple cases and link to reference/server.mdx for full Rust embed API.
3. [✓] R1-R3/R9 note — Added note after Context section: "Requirements R1-R3 and R9 are defined in SPEC-083a."
4. [✓] Tier 1 snippet scope — Rewrote R5 snippet description to clarify it covers both client-side React code and a brief server start command (env vars), not just client-only. Updated AC2 accordingly.
5. [✓] AC4 raw HTML note — Updated AC4 to explicitly state "using raw HTML/MDX table markup (not ComparisonRow component, which only supports 4 columns)."
6. [✓] R10 file list — Added prioritized list of 12 known files containing @topgunbuild/server references to R10. Updated G6 description to reference the file list.

### Audit v2 (2026-03-09)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% cumulative, ~25% max per worker (parallel execution)

**Critical:**
1. R10 lists 12 files with `@topgunbuild/server` references but provides no replacement strategy for files outside R4's scope. R4 specifies what replaces `ServerFactory` in `security.mdx` (env vars + link to `reference/server.mdx`). However, 9 other guide files (`authentication.mdx`, `interceptors.mdx`, `full-text-search.mdx`, `cluster-replication.mdx`, `deployment.mdx`, `rbac.mdx`, `performance.mdx`, `event-journal.mdx`, `reference/adapter.mdx`) each contain substantial `ServerFactory.create({...})` or `@topgunbuild/server` import blocks with no guidance on what replaces them. The implementer of G6 needs a universal replacement pattern. Add to R10: "For each `ServerFactory.create({...})` snippet, replace the server-side configuration portion with environment variable examples (same pattern as R4) and add a link to `reference/server.mdx` for the full Rust embed API. For `deployment.mdx` serverless snippets (`HttpSyncHandler` from `@topgunbuild/server/coordinator`), replace with a note that the Rust server does not support serverless edge deployment and link to `reference/server.mdx` for supported deployment modes."

**Recommendations:**
2. The `ComparisonRow` import on line 7 of `comparison.mdx` will become unused after R7 replaces all `ComparisonRow` usages with raw HTML. R7 should explicitly state: remove the `ComparisonRow` import line. Leaving an unused import may cause build warnings or linter errors (AC7: `pnpm start:docs` builds without errors).
3. `reference/index.mdx` also contains a `@topgunbuild/server` reference (description text: "Configuration options and middleware for @topgunbuild/server"). It is not in R10's known file list. While it is just description text (not a code snippet), it should be noted for completeness -- the description should reference the Rust server, not the TS package.
4. [Strategic] G6 at ~25% estimated context covers 12 files of varying complexity. The `deployment.mdx` file alone has 4 separate `@topgunbuild/server` import blocks (ServerFactory, HttpSyncHandler in 3 serverless examples). Consider splitting G6 into two segments: G6a for guide files (9 files, straightforward ServerFactory replacements) and G6b for blog + reference files (3 files, simpler install line updates). This would improve quality by keeping each worker under 20%.

**Assumptions Extracted:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | PRODUCT_CAPABILITIES.md contains 3-tier adoption model content | R5 cannot be implemented without source content |
| A2 | SPEC-083a is complete and reference/server.mdx exists with Rust embed API | R4, R10 cannot link to server reference |
| A3 | `pnpm start:docs` tolerates unused component imports | AC7 could fail if linter catches unused ComparisonRow import |
| A4 | The Rust server binary is named `topgun-server` | Code examples in R4, R5, R8 would use wrong binary name |

**Verification of assumptions:**
- A1: Confirmed -- `.specflow/reference/PRODUCT_CAPABILITIES.md` exists
- A2: Confirmed -- `reference/server.mdx` exists with Rust content (NetworkConfig, TlsConfig, env vars)
- A3: Uncertain -- should remove unused import to be safe (see Recommendation 2)
- A4: Not verified -- R4 and R5 use `topgun-server` but `reference/server.mdx` shows `cargo run --bin test-server`. The binary name may differ. However, this is a docs content concern, not a spec structure issue.

**Project compliance:** Compliant. This is a docs spec; Language Profile (Rust-specific) does not apply to MDX files. No project constraints violated.

**Strategic fit:** Aligned with project goals. Post-migration docs update is necessary for adoption.

### Response v2 (2026-03-09)
**Applied:** Critical issue (1) and all recommendations (2, 3, 4)

**Changes:**
1. [✓] R10 universal replacement strategy — Added "Universal replacement pattern" paragraph to R10 specifying: replace ServerFactory snippets with env var examples + link to reference/server.mdx; replace deployment.mdx serverless snippets (HttpSyncHandler) with a note that Rust server does not support serverless edge deployment. Also added specific note about HttpSyncHandler to deployment.mdx entry in the known file list.
2. [✓] ComparisonRow import removal — Added explicit bullet to R7: "Remove the ComparisonRow import line." Updated G4 description to mention import removal.
3. [✓] reference/index.mdx added — Added `reference/index.mdx` as 13th entry in R10's known file list with note about description text referencing @topgunbuild/server.
4. [✓] G6 split into G6a/G6b — Split G6 into G6a (9 guide + reference/adapter files, ~18% context) and G6b (2 blog + reference/index files, ~7% context). Updated Execution Plan to show 3 workers in Wave 2.

### Audit v3 (2026-03-09)
**Status:** APPROVED

**Context Estimate:** ~100% cumulative, ~20% max per worker (parallel execution across 7 groups)

**Quality Projection:** PEAK per worker (each group ~7-20%, well within 30% target)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~100% | <=50% | Needs parallel execution |
| Largest task group | ~20% (G2) | <=30% | OK |
| Worker overhead | ~5% per worker | <=10% | OK |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Status |
|-------|------|-------|--------------|--------|
| G1 | 1 | Security Model section | ~15% | OK |
| G2 | 1 | Adoption path guide | ~20% | OK |
| G3 | 1 | PostgreSQL guide | ~15% | OK |
| G4 | 1 | Comparison table rewrite | ~15% | OK |
| G5 | 2 | Quick-start update | ~10% | OK |
| G6a | 2 | Guide snippet audit (9 files) | ~18% | OK |
| G6b | 2 | Blog/reference audit (3 files) | ~7% | OK |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | PRODUCT_CAPABILITIES.md contains 3-tier adoption model | R5 cannot be written |
| A2 | SPEC-083a complete, reference/server.mdx exists | R4, R10 links broken |
| A3 | ComparisonRow import removal prevents build errors | AC7 failure |
| A4 | Rust binary named `topgun-server` | Code examples incorrect |

**Project compliance:** Compliant. Docs spec; Language Profile does not apply to MDX files.

**Strategic fit:** Aligned with project goals.

**Comment:** Spec is well-structured after two revision cycles. All prior critical issues have been resolved. Requirements are clear with explicit file targets, replacement patterns, and acceptance criteria. The parallel execution plan keeps each worker well within context bounds. The universal replacement pattern in R10 gives G6a/G6b workers a consistent strategy. Ready for implementation.

## Execution Summary

**Executed:** 2026-03-09
**Mode:** orchestrated (sequential fallback -- subagent CLI not available)
**Commits:** 7

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2, G3, G4 | complete |
| 2 | G5, G6a, G6b | complete |

### Files Created
- `apps/docs-astro/src/content/docs/guides/adoption-path.mdx`
- `apps/docs-astro/src/content/docs/guides/postgresql.mdx`

### Files Modified
- `apps/docs-astro/src/content/docs/guides/security.mdx`
- `apps/docs-astro/src/content/docs/comparison.mdx`
- `apps/docs-astro/src/content/docs/quick-start.mdx`
- `apps/docs-astro/src/content/docs/guides/authentication.mdx`
- `apps/docs-astro/src/content/docs/guides/interceptors.mdx`
- `apps/docs-astro/src/content/docs/guides/full-text-search.mdx`
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx`
- `apps/docs-astro/src/content/docs/guides/deployment.mdx`
- `apps/docs-astro/src/content/docs/guides/rbac.mdx`
- `apps/docs-astro/src/content/docs/guides/performance.mdx`
- `apps/docs-astro/src/content/docs/guides/event-journal.mdx`
- `apps/docs-astro/src/content/docs/reference/adapter.mdx`
- `apps/docs-astro/src/content/docs/reference/index.mdx`
- `apps/docs-astro/src/content/blog/full-text-search-offline-first.mdx`
- `apps/docs-astro/src/content/blog/distributed-live-subscriptions.mdx`

### Acceptance Criteria Status
- [x] AC1: `guides/security.mdx` has a "Security Model" section covering JWT (`sub` claim), TLS, mTLS, RBAC
- [x] AC2: `guides/adoption-path.mdx` exists with 3 tiers, each with code example, and Tier 1 snippet covering client React code and server start command
- [x] AC3: `guides/postgresql.mdx` exists with PowerSync/ElectricSQL vs TopGun comparison table and `DATABASE_URL` configuration example
- [x] AC4: `comparison.mdx` table has 6 columns using raw HTML/MDX table markup (not ComparisonRow component)
- [x] AC5: `quick-start.mdx` references the Rust server binary, not `@topgunbuild/server`
- [x] AC6: No docs page contains `@topgunbuild/server` code snippets as primary server setup -- FIXED (see Review v2)
- [x] AC7: `pnpm start:docs` builds without errors (verified by reviewer)

### Deviations
- Executed sequentially (not parallel) because subagent CLI spawning was not available in the environment. All task groups completed successfully regardless.

---

## Review History

### Review v1 (2026-03-09 18:45)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**
1. **Incomplete ServerFactory removal in cluster-replication.mdx**
   - File: `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx:54,76,93,113`
   - Issue: The G6a audit replaced only the first two code blocks (`clusterSetupCode`, `consistencyLevelsCode`) but left 4 exported code blocks still using `ServerFactory.create({...})`: `antiEntropyCode` (line 54), `gossipProtocolCode` (line 76), `failoverCode` (line 93), and `readReplicaCode` (line 113). These are primary server configuration snippets showing how to configure anti-entropy, gossip, failover, and read replicas. This violates AC6 and R10's universal replacement pattern.
   - Fix: Replace each `ServerFactory.create({...})` block with the env var pattern (e.g., `TOPGUN_REPLICATION_ENABLED=true topgun-server`) or convert them to conceptual pseudocode/comments that explain behavior without showing TS API calls. Add a link to `reference/server.mdx` for configuration details.

2. **Incomplete ServerFactory removal in full-text-search.mdx**
   - File: `apps/docs-astro/src/content/docs/guides/full-text-search.mdx:582`
   - Issue: The `serverSearchPermissionsCode` export still contains `ServerFactory.create({...})` with full-text search and security config. This is a primary server setup snippet showing how to configure search permissions, violating AC6.
   - Fix: Replace with env var pattern or a conceptual snippet showing the RBAC policy structure without `ServerFactory`. Link to `reference/server.mdx`.

3. **Stale ServerFactory text reference in rbac.mdx**
   - File: `apps/docs-astro/src/content/docs/guides/rbac.mdx:79`
   - Issue: Line 79 reads "Policies are passed to `ServerFactory.create()`." while the code block directly below it (line 82) already shows env var configuration. The text contradicts the updated code block and references the removed TS API.
   - Fix: Replace line 79 with text like "The server is configured with RBAC policies via environment variables and configuration:" or similar wording consistent with the env var pattern.

**Major:**
4. **Remaining ServerFactory in blog post**
   - File: `apps/docs-astro/src/content/blog/full-text-search-offline-first.mdx:74`
   - Issue: The blog post still contains a `ServerFactory.create({...})` code block showing server-side search configuration. While blog posts are historical, AC6 states "No docs page contains `@topgunbuild/server` code snippets as primary server setup" without exempting blog posts. The G6b task was supposed to audit this file but only changed the `npm install` line (commit `9b8019c`), missing this code block.
   - Fix: Either replace with env var pattern (preferred for consistency) or add a callout note above the code block indicating this shows the legacy TS API and linking to the current Rust server docs.

**Passed:**
- [v] AC1: `guides/security.mdx` has comprehensive "Security Model" section with Trust Boundary, Security Pipeline table covering JWT `sub` claim, Map-level ACL, HLC Sanitization, Value Size Limits, RBAC, and Transport Security (TLS + mTLS). Well-structured with callout linking to `reference/server.mdx`.
- [v] AC2: `guides/adoption-path.mdx` exists with 3 tiers, each with description, code example, "when to use", and "what you gain" sections. Tier 1 snippet is ~30 lines covering both server start command and client React code with `getMap()` and state updates. Exceeds 20-line target but appropriately so.
- [v] AC3: `guides/postgresql.mdx` exists with 6-row comparison table (PowerSync/ElectricSQL vs TopGun), clear "NOT a sync layer" callout, `DATABASE_URL` configuration, and SQL schema showing `topgun_*` tables.
- [v] AC4: `comparison.mdx` uses raw HTML table with all 6 columns (TopGun, ElectricSQL, Firebase, RxDB, Replicache/Zero, Durable Objects). `ComparisonRow` import removed. "Why TopGun?" section includes vs Replicache/Zero and vs Durable Objects entries.
- [v] AC5: `quick-start.mdx` references Rust server binary (`topgun-server`) with env var startup, links to Server API reference and Adoption Path guide. No `@topgunbuild/server` references.
- [v] AC7: `pnpm start:docs` builds and starts without errors (verified).
- [v] All created files follow MDX conventions: frontmatter with `order` field, breadcrumb navigation, component imports, CodeBlock usage, prev/next navigation links.
- [v] `reference/index.mdx` updated to reference "Rust server embed API" instead of `@topgunbuild/server`.
- [v] Blog `npm install` lines updated to remove `@topgunbuild/server`.

**Summary:** The new guide pages (G1-G4) and quick-start update (G5) are well-implemented with high quality content, proper styling, and correct MDX conventions. However, the G6a snippet audit was incomplete -- it replaced the primary setup snippets at the top of each file but missed secondary `ServerFactory.create({...})` code blocks deeper in `cluster-replication.mdx` (4 blocks), `full-text-search.mdx` (1 block), and a stale text reference in `rbac.mdx`. The G6b blog audit also missed a `ServerFactory` code block in `full-text-search-offline-first.mdx`. These remaining references violate AC6.

### Fix Response v1 (2026-03-09)
**Applied:** All issues (3 critical + 1 major)

**Fixes:**
1. [✓] Incomplete ServerFactory removal in cluster-replication.mdx — Replaced all 4 `ServerFactory.create()` blocks (`antiEntropyCode`, `gossipProtocolCode`, `failoverCode`, `readReplicaCode`) with env var / conceptual bash snippets. Updated CodeBlock language to bash.
   - Commit: `9d54a89`
2. [✓] Incomplete ServerFactory removal in full-text-search.mdx — Replaced `serverSearchPermissionsCode` ServerFactory block with env var pattern and RBAC policy comments. Updated CodeBlock language to bash.
   - Commit: `ee09ecc`
3. [✓] Stale ServerFactory text reference in rbac.mdx — Replaced "Policies are passed to `ServerFactory.create()`" with "RBAC policies are configured on the Rust server via security settings and environment variables."
   - Commit: `5a84bf4`
4. [✓] Remaining ServerFactory in blog post — Replaced `ServerFactory.create()` code block in `full-text-search-offline-first.mdx` with Rust server env var comment.
   - Commit: `0c61a02`

**Verification:** `grep -r "ServerFactory" apps/docs-astro/src/content/` returns zero matches.

### Review v2 (2026-03-09 20:15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [v] AC1: `guides/security.mdx` has comprehensive "Security Model" section with Trust Boundary diagram, Security Pipeline table covering JWT `sub` claim, Map-level ACL, HLC Sanitization, Value Size Limits, RBAC, and Transport Security (TLS + mTLS). Callout links to `reference/server.mdx`.
- [v] AC2: `guides/adoption-path.mdx` exists with 3 tiers (Real-Time Layer, Cache + Sync, Full Platform), each with description, code example, "when to use", and "what you gain" sections. Tier 1 snippet covers both `topgun-server` env var start command and client React code with `getMap()`, `useQuery()`, and state updates.
- [v] AC3: `guides/postgresql.mdx` exists with 6-row comparison table (PowerSync/ElectricSQL vs TopGun), clear "NOT a sync layer" callout, `DATABASE_URL` configuration, SQL schema showing `topgun_*` tables, and guidance on when to use a separate database.
- [v] AC4: `comparison.mdx` uses raw HTML `<table>` with all 6 columns (TopGun, ElectricSQL, Firebase, RxDB, Replicache/Zero, Durable Objects). `ComparisonRow` import removed. 7 comparison rows. "Why TopGun?" section includes vs Replicache/Zero and vs Durable Objects entries.
- [v] AC5: `quick-start.mdx` references Rust server binary (`topgun-server`) with env var startup. Links to Server API reference and Adoption Path guide. No `@topgunbuild/server` references.
- [v] AC6: Zero `ServerFactory` references across all docs content. Zero `@topgunbuild/server` references across all docs content. Verified via `grep -r` across `apps/docs-astro/src/content/`. All v1 review fixes confirmed: `cluster-replication.mdx` (4 blocks replaced), `full-text-search.mdx` (1 block replaced), `rbac.mdx` (text reference fixed), `full-text-search-offline-first.mdx` blog (1 block replaced).
- [v] AC7: `pnpm start:docs` builds and starts without errors (HTTP 200 verified).
- [v] All created files follow MDX conventions: frontmatter with `order` field, breadcrumb navigation, component imports, CodeBlock usage, prev/next navigation links.
- [v] `reference/index.mdx` updated to reference "Rust server embed API" instead of `@topgunbuild/server`.
- [v] Blog `npm install` lines updated to remove `@topgunbuild/server`.
- [v] Replacement snippets in cluster-replication.mdx use conceptual bash comments explaining behavior (anti-entropy, gossip, failover, read replicas) with links to `/docs/reference/server`.
- [v] No security issues: no hardcoded secrets in examples, password placeholders used appropriately.
- [v] No architectural concerns: docs changes only, no code changes.

**Summary:** All 7 acceptance criteria now pass. The v1 review found 3 critical and 1 major issue (incomplete ServerFactory removal in 4 files). Fix Response v1 addressed all issues across 4 commits. This v2 review confirms zero `ServerFactory` and zero `@topgunbuild/server` references remain in docs content. New guide pages are well-structured with consistent styling, proper MDX conventions, and useful content. The comparison table successfully uses raw HTML markup for 6-column support. Implementation is complete.

---

## Completion

**Completed:** 2026-03-09
**Total Commits:** 11 (7 implementation + 4 fix)
**Review Cycles:** 2

### Outcome

Created 2 new guide pages (adoption-path, postgresql), rewrote the comparison table for 6 competitors, updated quick-start for Rust server, and audited all 15 docs/blog files to remove every `@topgunbuild/server` / `ServerFactory` reference.

### Key Files

- `guides/adoption-path.mdx` — 3-tier adoption model for new users
- `guides/postgresql.mdx` — PostgreSQL coexistence guide with competitor comparison
- `comparison.mdx` — 6-column raw HTML comparison table (TopGun vs 5 competitors)
- `guides/security.mdx` — Security Model section with trust boundary and pipeline

### Patterns Established

None — followed existing MDX conventions.

### Deviations

None — implemented as specified (sequential execution instead of parallel due to environment constraint).
