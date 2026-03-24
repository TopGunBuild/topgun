---
id: SPEC-140
type: docs
status: running
priority: P2
complexity: medium
created: 2026-03-24
source: TODO-172
delta: true
---

# Docs Audit: Verify All Remaining Guides Against Rust Implementation

## Context

Three of ~25 guide pages (`authentication.mdx`, `security.mdx`, `rbac.mdx`) were audited in SPEC-139 and found to contain critical mismatches: referencing nonexistent env vars, documenting unimplemented features as available, and showing TS-server-era API patterns. The remaining ~22 pages have not been verified against the actual Rust server and TS client code. These pages are published documentation — inaccurate guides will erode trust and waste developer time.

Priority targets (most likely to have TS-to-Rust migration gaps): `deployment.mdx`, `cluster-replication.mdx`, `cluster-client.mdx`, `postgresql.mdx`, `performance.mdx`, `observability.mdx`.

## Task

Systematically audit every remaining guide page in `apps/docs-astro/src/content/docs/guides/` against actual source code in `packages/server-rust/`, `packages/client/`, `packages/core/`, and `packages/core-rust/`. For each page, produce a verdict (accurate / minor issues / major rewrite needed) and a list of specific issues. For features found to be unimplemented, cross-reference `.specflow/todos/TODO.md` to check if work is already planned. If not planned, create a new TODO entry referencing the old TS server as implementation reference. The output is a structured audit report plus any new TODO entries for coverage gaps.

## Delta

### ADDED
- `.specflow/reference/DOCS_AUDIT_TRIAGE.md` — Pass 1 output: quick triage table (feature exists / not exists / partial) for all 22 pages
- `.specflow/reference/DOCS_AUDIT_REPORT.md` — final audit report combining triage + deep audit findings

### MODIFIED
- `.specflow/todos/TODO.md` — new TODO entries added for unimplemented features not yet tracked

### AUDITED (read-only inputs — do NOT modify)
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — Verify env vars, Docker config, port references, binary name against actual Rust server
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` — Verify partition count (271), replication factor, rebalancing behavior against code
- `apps/docs-astro/src/content/docs/guides/cluster-client.mdx` — Verify client cluster connection API against TS client SDK
- `apps/docs-astro/src/content/docs/guides/postgresql.mdx` — Verify PostgreSQL adapter config, connection options, schema against `PostgresDataStore`
- `apps/docs-astro/src/content/docs/guides/performance.mdx` — Verify benchmark numbers, tuning knobs, config options against actual implementation
- `apps/docs-astro/src/content/docs/guides/observability.mdx` — Verify tracing/metrics config, env vars against `tracing` setup in server
- `apps/docs-astro/src/content/docs/guides/live-queries.mdx` — Verify query subscription API against TS client `QueryHandle`
- `apps/docs-astro/src/content/docs/guides/pub-sub.mdx` — Verify topic API against TS client and Rust `MessagingService`
- `apps/docs-astro/src/content/docs/guides/full-text-search.mdx` — Verify search API against Rust `SearchService` and tantivy integration
- `apps/docs-astro/src/content/docs/guides/entry-processor.mdx` — Verify entry processor API exists in Rust or mark as planned
- `apps/docs-astro/src/content/docs/guides/conflict-resolvers.mdx` — Verify CRDT merge strategies match `core-rust` implementation
- `apps/docs-astro/src/content/docs/guides/interceptors.mdx` — Verify middleware/interceptor API against Tower middleware pipeline
- `apps/docs-astro/src/content/docs/guides/distributed-locks.mdx` — Verify lock API exists in Rust or mark as planned
- `apps/docs-astro/src/content/docs/guides/ttl.mdx` — Verify TTL support exists in Rust or mark as planned
- `apps/docs-astro/src/content/docs/guides/write-concern.mdx` — Verify write concern options against actual acknowledgement protocol
- `apps/docs-astro/src/content/docs/guides/pn-counter.mdx` — Verify PN-Counter CRDT exists in core-rust or mark as planned
- `apps/docs-astro/src/content/docs/guides/adaptive-indexing.mdx` — Verify adaptive indexing exists in Rust or mark as planned
- `apps/docs-astro/src/content/docs/guides/indexing.mdx` — Verify index types (Hash, Navigable, Inverted) against Rust implementation
- `apps/docs-astro/src/content/docs/guides/event-journal.mdx` — Verify event journal/oplog API against actual persistence layer
- `apps/docs-astro/src/content/docs/guides/adoption-path.mdx` — Verify migration/adoption steps reference correct APIs
- `apps/docs-astro/src/content/docs/guides/mcp-server.mdx` — Verify MCP server package exists and API matches docs
- `apps/docs-astro/src/content/docs/guides/index.mdx` — Verify guide index descriptions match actual page content after audit

Note: This spec produces an audit report + TODO entries for untracked features. Actual doc fixes will be tracked as TODO items, not separate specs.

## Requirements

### R1: Audit Report Structure

For each guide page, produce:

1. **Page name** (filename)
2. **Verdict**: one of `accurate`, `minor-issues`, `major-rewrite`
3. **Issues list** with:
   - Line reference or section name
   - What the docs say
   - What the code actually does (with file path reference)
   - Severity: `critical` (completely wrong/nonexistent feature), `major` (wrong API/config), `minor` (outdated wording, cosmetic)

### R2: Verification Checklist Per Page

Each page audit verifies at minimum:

1. **API references**: Do the function/method names, parameters, and return types exist in the referenced package?
2. **Config options / env vars**: Does the server actually read these? Check `ServerConfig`, `AppState`, env var parsing in `main.rs` and config modules.
3. **Code examples**: Would the shown code compile (Rust) or pass type checking (TS)?
4. **Feature existence**: Is the described feature implemented or is it planned/aspirational? Mark planned features explicitly.
5. **Architecture claims**: Do statements about partitions, replication, sync protocol match the actual Rust implementation?

### R3: Priority Classification

Classify each page into fix priority:

- **P1 — Block launch**: Page describes nonexistent features as available, or shows code that will fail
- **P2 — Fix soon**: Page has wrong config values, outdated API signatures, or misleading architecture claims
- **P3 — Polish**: Minor wording issues, slightly outdated but not harmful

### R4: Output Location

Write the audit report to `.specflow/reference/DOCS_AUDIT_REPORT.md` as a structured markdown document with one section per guide page.

### R5: Missing Functionality Registry

For each feature documented but not implemented in Rust:

1. **Identify** the feature name and which doc page describes it
2. **Check TS server reference**: Confirm the feature existed in the old TS server (check `packages/server/` in git history, commit `926e856` removed TS server — use `git show 926e856^:packages/server/...`)
3. **Cross-reference TODO.md**: Search `.specflow/todos/TODO.md` for an existing TODO covering this feature
4. **Record** in the audit report: feature name, doc page, TS server file paths (if found), existing TODO ID (if any)

### R6: Gap-Filling TODO Creation

If a documented feature has **no existing TODO** in `.specflow/todos/TODO.md`:

1. Create a new TODO entry in TODO.md with:
   - Priority: P3 (unless the feature is core to product positioning, then P2)
   - Complexity estimate based on TS implementation scope
   - Summary describing the feature gap
   - **TS Reference** section with `git show` commands to recover old implementation files
   - Note: "Documented in `{page}.mdx` — currently presented as available but not yet ported to Rust"
2. Use the existing TODO format and numbering convention in TODO.md

### R7: Planned-Feature Doc Handling

For doc pages describing unimplemented features:

- Do **NOT** recommend deleting or removing the page
- Instead, recommend adding a **planned-feature banner** at the top (same pattern used in SPEC-139 for security.mdx): `> **Status: Planned** — This feature is on the roadmap but not yet implemented in the current Rust server.`
- The verdict for such pages should be `major-rewrite` (to add the banner and remove "how to use" code examples that won't work) but the fix approach is "mark as planned + simplify to feature overview", not "delete"

## Acceptance Criteria

1. Every guide page in `apps/docs-astro/src/content/docs/guides/` (excluding the 3 already audited in SPEC-139) has a verdict and issue list in the audit report
2. Each issue references the specific source file that contradicts the documentation claim
3. Pages describing unimplemented features are flagged as `major-rewrite` with recommendation "add planned banner", not "delete page"
4. The report includes a summary table: page name, verdict, issue count, fix priority
5a. Every unimplemented feature has a cross-reference to either an existing TODO ID or a newly created TODO entry
5b. Newly created TODOs include TS server file references recoverable via `git show`
6. At least the 6 priority targets (deployment, cluster-replication, cluster-client, postgresql, performance, observability) have thorough line-by-line verification

## Validation Checklist

- Count guide pages in `guides/` directory minus 3 already audited — report covers all remaining pages
- For each `critical` issue, open the referenced source file and confirm the documented feature/API does not exist
- For `deployment.mdx`, verify every env var shown in Docker Compose against `ServerConfig` fields
- Summary table totals match individual page sections
- For each new TODO created, verify the TS server file path is recoverable via `git show 926e856^:{path}`
- Cross-check: no duplicate TODOs created for features that already have TODO entries

## Constraints

- Do NOT modify any guide pages in this spec — output is the audit report only
- Do NOT re-audit `authentication.mdx`, `security.mdx`, or `rbac.mdx` (already done in SPEC-139)
- Reference actual file paths in `packages/server-rust/src/` and `packages/client/src/` — do not guess from memory
- Mark uncertainty explicitly: if a feature's implementation status is unclear, say "UNCLEAR — needs manual verification" rather than guessing

## Assumptions

- The 3 pages audited in SPEC-139 do not need re-audit (their fixes are already tracked)
- `index.mdx` (the guide index page) only needs description accuracy check, not deep code verification
- The audit is text-based research output, not code changes — complexity is driven by number of pages to read and cross-reference
- Follow-up work is tracked as TODO entries (not separate specs) — more context-efficient
- Many "missing" features likely existed in the old TS server (removed in commit `926e856`) and need porting, not designing from scratch
- Pages documenting future/planned features are acceptable IF they are clearly marked as planned (with banners or badges) — the issue is when planned features are presented as currently available
- Doc pages for unimplemented features should NOT be deleted — they describe real product goals and will be updated when features are ported

### Two-Pass Execution Strategy

The audit uses two passes to maximize quality while staying within context budgets. Pass 1 is a fast triage that classifies all 22 pages. Pass 2 does deep line-by-line audits only where needed.

### Pass 1: Quick Sweep (feature existence triage)

**Goal:** For each of 22 pages, determine: feature exists in Rust / partially exists / does not exist. Also cross-reference TODO.md and locate TS server references for missing features.

**Method:** Read each doc page header + key sections, `grep` server-rust for feature keywords (service names, struct names, config fields). Read TODO.md once. No deep code reading.

**Output:** Triage table written to `.specflow/reference/DOCS_AUDIT_TRIAGE.md`:

| Page | Feature Status | Existing TODO | TS Server Reference | Pass 2 Needed? |
|------|---------------|---------------|---------------------|----------------|

Pages classified as "does not exist" get their verdict immediately (`major-rewrite`, recommend planned banner) and do NOT enter Pass 2. New TODOs are created for untracked missing features (R6).

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G0 | 1 | Quick sweep: read all 22 doc pages, grep server-rust, read TODO.md, produce triage table, create missing TODOs | — | ~20% |

### Pass 2: Deep Audit (implemented features only)

**Goal:** Line-by-line verification of pages where the feature actually exists (estimated 8-12 pages based on known Rust server capabilities: deployment, cluster-replication, cluster-client, postgresql, performance, observability, live-queries, pub-sub, full-text-search, conflict-resolvers, adoption-path, mcp-server).

**Method:** For each page, read the full doc + read the corresponding source files. Verify API references, env vars, config options, code examples, architecture claims (R2 checklist).

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 2 | Deep audit infra pages: deployment, postgresql, performance, observability | G0 | ~20% |
| G2 | 2 | Deep audit cluster + sync pages: cluster-replication, cluster-client, live-queries, pub-sub | G0 | ~20% |
| G3 | 2 | Deep audit remaining implemented pages: full-text-search, conflict-resolvers, adoption-path, mcp-server, index + any extras from G0 triage | G0 | ~15% |

Note: Exact page assignment for G1-G3 will be adjusted based on G0 triage results. Pages where feature does not exist skip Pass 2 entirely.

### Pass 2 Finalization

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G4 | 3 | Merge G0 triage + G1-G3 deep audits into final DOCS_AUDIT_REPORT.md, compile summary table, classify fix priorities | G1, G2, G3 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G0 | No | 1 |
| 2 | G1, G2, G3 | Yes | 3 |
| 3 | G4 | No | 1 |

**Total workers needed:** 3 (max in Wave 2)

### Why Two Passes

- **Pass 1 alone** handles ~50% of pages (features that don't exist) — cheap, PEAK quality
- **Pass 2** concentrates context budget on pages that need real code cross-referencing
- Each Pass 2 worker gets 3-5 pages instead of 5-11, staying in PEAK zone (~15-20%)
- Total effective context: ~85% across all workers, but no single worker exceeds ~20%

## Audit History

### Audit v1 (2026-03-24)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (across 4 worker invocations)

**Delta validation:** 22/23 entries valid

**Critical:**
1. Delta MODIFIED file `apps/docs-astro/src/content/docs/guides/clustering.mdx` not found in codebase. Only `cluster-replication.mdx` and `cluster-client.mdx` exist. Remove this entry from Delta, update the priority targets list in Context (line 18), and update AC5 to reference 6 priority targets instead of 7.
2. Delta missing ADDED entry: The output file `.specflow/reference/DOCS_AUDIT_REPORT.md` is a new file to be created but is not listed in the Delta section. Add it under an ADDED subsection.

**Recommendations:**
3. [Strategic] G1 is estimated at ~40% context, which exceeds the 30% per-group target. Consider splitting G1 into two sub-groups (e.g., G1a: deployment, postgresql, performance, observability; G1b: cluster-replication, cluster-client) to keep each worker in the PEAK quality range.
4. Delta semantic mismatch: All 22 guide pages are listed as MODIFIED, but the Constraints section says "Do NOT modify any guide pages." These are audit inputs, not modification targets. Consider moving them out of Delta or using a custom "AUDITED" category to avoid confusion during execution.
5. Context section says "remaining ~21 pages" but actual count is 22 (25 total minus 3 already audited). Minor accuracy issue.

**Strategic fit:** Aligned with project goals -- auditing published docs against actual implementation prevents user trust erosion.

**Project compliance:** Honors PROJECT.md decisions. No violations detected.

### Response v1 (2026-03-24)
**Applied:** all critical issues and all recommendations

**Changes:**
1. [✓] Remove `clustering.mdx` from Delta and Context priority targets — removed entry from Delta AUDITED list and removed `clustering.mdx` from the priority targets sentence in Context
2. [✓] Add ADDED entry for `.specflow/reference/DOCS_AUDIT_REPORT.md` — added ADDED subsection to Delta with the output file
3. [✓] Split G1 into G1a and G1b sub-groups — G1a covers deployment/postgresql/performance/observability (~20%), G1b covers cluster-replication/cluster-client (~20%); Wave 1 now runs 3 workers in parallel; total workers updated from 2 to 3
4. [✓] Relabel guide pages in Delta from MODIFIED to AUDITED — replaced "MODIFIED" heading with "AUDITED (read-only inputs — do NOT modify)" to match the Constraints intent
5. [✓] Fix page count from ~21 to ~22 — updated Context sentence to "remaining ~22 pages"

### Audit v2 (2026-03-24 18:30)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (across 5 worker invocations)

**Delta validation:** 23/23 entries valid (1 ADDED, 22 AUDITED)

**Scope:** Large (~100% estimated across all groups, exceeds 50% target)

**Per-Group Breakdown:**

| Group | Wave | Est. Context | Status |
|-------|------|--------------|--------|
| G1a | 1 | ~20% | OK |
| G1b | 1 | ~20% | OK |
| G2 | 1 | ~25% | OK |
| G3 | 2 | ~25% | OK |
| G4 | 3 | ~10% | OK |

**Quality Projection:** GOOD range per worker (each group <=25%), but total requires orchestrated parallel execution.

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- per-worker target |
| 30-50% | GOOD | -- |
| 50-70% | DEGRADING | -- |
| 70%+ | POOR | -- |

Each worker stays in PEAK-to-GOOD range individually. Orchestrated execution required.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-139 pages do not need re-audit | Audit report has coverage gaps |
| A2 | All guide pages are in the guides/ directory | Missing pages from audit |
| A3 | Source code reflects current implementation | Audit conclusions could be wrong |
| A4 | Follow-up fix specs will be created from report | Report sits unused |
| A5 | Planned features with banners are acceptable | Inconsistent verdicts |

All assumptions are reasonable and low-risk.

**Strategic fit:** Aligned with project goals -- auditing published docs against actual implementation prevents user trust erosion.

**Project compliance:** Honors PROJECT.md decisions. No violations detected. Language profile not applicable (docs-type spec, output file is in .specflow/reference/, not in Rust packages).

**Recommendation:** Use `/sf:run --parallel` -- spec is already well-decomposed into 5 groups across 3 waves with max 3 parallel workers.

### Response v2 (2026-03-24)
**Applied:** Strategic revision — user-directed workflow change

**Changes:**
1. [✓] Added R5 (Missing Functionality Registry) — cross-reference unimplemented features with TODO.md
2. [✓] Added R6 (Gap-Filling TODO Creation) — create TODOs for untracked features with TS server references via `git show 926e856^:`
3. [✓] Added R7 (Planned-Feature Doc Handling) — recommend "add planned banner" instead of deleting pages
4. [✓] Updated Delta — added MODIFIED entry for TODO.md
5. [✓] Updated Task description — output now includes TODO entries for coverage gaps
6. [✓] Updated AC3 — "add planned banner" instead of "P1 major-rewrite"
7. [✓] Added AC5a/5b — TODO cross-references and TS server file references
8. [✓] Updated Assumptions — TODO items instead of specs, TS server as reference, no page deletion
9. [✓] Added Validation items — verify `git show` paths, check for duplicate TODOs

### Response v3 (2026-03-24)
**Applied:** Strategic revision — two-pass execution strategy for quality optimization

**Changes:**
1. [✓] Replaced single-pass 5-group plan with two-pass strategy: Pass 1 (quick sweep triage) → Pass 2 (deep audit implemented features only)
2. [✓] Added G0 (Wave 1) — quick sweep reads all 22 pages + greps server, produces triage table, creates missing TODOs (~20% context)
3. [✓] Restructured G1-G3 (Wave 2) — deep audit only pages where feature exists (~8-12 pages), 3-5 pages per worker (~15-20% each)
4. [✓] G4 (Wave 3) — merges triage + deep audits into final report
5. [✓] Added DOCS_AUDIT_TRIAGE.md to Delta ADDED — intermediate Pass 1 output
6. [✓] Each worker stays in PEAK zone (≤20%), no worker exceeds 20%
7. [✓] Pages with nonexistent features handled entirely in Pass 1 — no wasted deep-audit context

### Audit v3 (2026-03-24 19:00)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~85% total (across 5 worker invocations, no single worker exceeds ~20%)

**Delta validation:** 25/25 entries valid (2 ADDED, 1 MODIFIED, 22 AUDITED)

**Scope:** Large (~85% estimated across all groups, exceeds 50% target)

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Status |
|-------|------|-------|--------------|--------|
| G0 | 1 | Quick sweep triage (22 pages + grep + TODO.md) | ~20% | OK |
| G1 | 2 | Deep audit infra (deployment, postgresql, performance, observability) | ~20% | OK |
| G2 | 2 | Deep audit cluster+sync (cluster-replication, cluster-client, live-queries, pub-sub) | ~20% | OK |
| G3 | 2 | Deep audit remaining (full-text-search, conflict-resolvers, adoption-path, mcp-server, index) | ~15% | OK |
| G4 | 3 | Merge into final report | ~10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- per-worker target (all groups) |
| 30-50% | GOOD | -- |
| 50-70% | DEGRADING | -- |
| 70%+ | POOR | -- |

Each worker stays in PEAK range individually. Orchestrated parallel execution required.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | SPEC-139 pages do not need re-audit | Minor coverage gap |
| A2 | All guide pages are in guides/ directory | Would miss pages |
| A3 | Source code reflects current implementation | Wrong audit conclusions |
| A4 | Follow-up work tracked as TODOs | Report sits unused |
| A5 | git show 926e856^ recovers TS server files | Cannot create TS references in new TODOs |

All assumptions are reasonable and low-risk.

**Strategic fit:** Aligned with project goals -- auditing published docs against actual implementation prevents user trust erosion.

**Project compliance:** Honors PROJECT.md decisions. No violations detected. Language profile not applicable (docs-type spec, not Rust code).

**Goal-Backward:** No Goal Analysis section present. Acceptable for docs-type medium spec -- requirements R1-R7 provide sufficient structure.

**Comment:** Spec is well-structured after three revision rounds. Two-pass strategy is sound: Pass 1 eliminates ~50% of pages cheaply, Pass 2 concentrates deep verification on implemented features only. All 22 guide pages verified to exist. Delta entries are accurate. Requirements are thorough and testable. Acceptance criteria are concrete and measurable.

**Recommendation:** Use `/sf:run --parallel` -- spec is well-decomposed into 5 groups across 3 waves with max 3 parallel workers.

## Execution Summary

**Executed:** 2026-03-24
**Mode:** orchestrated
**Commits:** 2 (1fc5260, abcc706)

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G0 (Quick sweep triage) | complete |
| 2 | G1, G2, G3 (Deep audits) | complete |
| 3 | G4 (Final report merge) | complete |

### Files Created

- `.specflow/reference/DOCS_AUDIT_TRIAGE.md` — Pass 1 triage table for all 22 pages
- `.specflow/reference/DOCS_AUDIT_REPORT.md` — Final audit report with verdicts, issues, and fix priorities

### Files Modified

- `.specflow/todos/TODO.md` — Added TODO-174 through TODO-180 for unimplemented features

### Acceptance Criteria Status

- [x] Every guide page (22/22) has a verdict and issue list
- [x] Each issue references the specific source file
- [x] Pages with unimplemented features flagged as major-rewrite with "add planned banner"
- [x] Report includes summary table with page name, verdict, issue count, fix priority
- [x] Every unimplemented feature cross-references an existing or new TODO ID
- [x] New TODOs (TODO-174 through TODO-180) include TS server file references via `git show 926e856^:`
- [x] All 6 priority targets (deployment, cluster-replication, cluster-client, postgresql, performance, observability) thoroughly audited

### Key Findings

- **11 pages need major-rewrite** (7 with unimplemented/stub features + deployment, observability, performance, write-concern with critical mismatches)
- **6 pages have minor issues**
- **5 pages are accurate** (live-queries, pub-sub, postgresql, pn-counter, mcp-server)
- **Critical priority:** observability.mdx metric names are from the old TS server (Pino/Node.js), not the Rust server. performance.mdx config knobs don't exist in Rust. deployment.mdx shows planned TOPGUN_TLS_* env vars as current. cluster-replication.mdx shows non-existent consistency modes (QUORUM/STRONG).
- **7 new TODOs created** for features documented but not yet tracked in TODO.md

### Deviations

None — spec executed as designed with two-pass strategy.

---

## Review History

### Review v1 (2026-03-24)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**
1. **Summary totals mismatch — report footer contradicts its own table**
   - File: `.specflow/reference/DOCS_AUDIT_REPORT.md:38`
   - Issue: The "Totals" footer line reads "9 major-rewrite, 5 minor-issues, 3 accurate" but the summary table immediately above it contains 11 major-rewrite, 6 minor-issues, and 5 accurate pages (22 pages total). This violates Validation Checklist item "Summary table totals match individual page sections" and AC4 ("report includes a summary table"). The incorrect totals also propagate into the Execution Summary in SPEC-140 ("9 pages need major-rewrite", "5 pages have minor issues", "3 pages are accurate (live-queries, pub-sub, postgresql, pn-counter, mcp-server)" — listing 5 pages while calling it 3).
   - Fix: Correct the Totals footer line to "11 major-rewrite, 6 minor-issues, 5 accurate". Update the Execution Summary Key Findings in SPEC-140 to match.

**Passed:**
- [✓] Both output files exist: `.specflow/reference/DOCS_AUDIT_TRIAGE.md` and `.specflow/reference/DOCS_AUDIT_REPORT.md`
- [✓] TODO.md modified with 7 new entries (TODO-174 through TODO-180) — all present and properly structured
- [✓] No guide pages modified — constraint honored (verified via git diff, zero changes to `apps/docs-astro/src/content/docs/guides/`)
- [✓] All 22 remaining pages covered (25 total minus 3 from SPEC-139 = 22, confirmed by directory count)
- [✓] All 6 priority targets (deployment, cluster-replication, cluster-client, postgresql, performance, observability) thoroughly audited with line-by-line verification
- [✓] Each issue references a specific source file with line numbers — verified against actual source code
- [✓] Critical claims confirmed against real code: metric names (`topgun_active_connections`, `topgun_operations_total`) verified in `metrics_endpoint.rs`/`metrics.rs`; absent doc metrics confirmed absent; `achieved_level: None` confirmed at `crdt.rs:197,267`; `setWithAck`/`batchSet` confirmed absent from TS client; stub status of LockRequest/LockRelease confirmed in `coordination.rs`; WASM sandbox stubs confirmed in `persistence.rs`
- [✓] Env var claims verified: only `PORT`, `JWT_SECRET`, `DATABASE_URL`, `RUST_LOG`, `TOPGUN_LOG_FORMAT`, `TOPGUN_ADMIN_*` are actually read; `TOPGUN_PORT`, `TOPGUN_TLS_*`, `TOPGUN_CLUSTER_*` confirmed absent
- [✓] Pages with unimplemented features correctly flagged as major-rewrite with "add planned banner" recommendation (not "delete page")
- [✓] Every unimplemented feature cross-references an existing TODO or a newly created one (R5/R6 compliance)
- [✓] New TODOs include TS server file references via `git show 926e856^:` format (R6 compliance)
- [✓] Two-pass strategy executed correctly — Pass 1 triage eliminated non-existent features cheaply, Pass 2 concentrated on implemented features

**Summary:** The audit is substantive and thorough — all source code claims were verified against actual implementation. One major issue: the report's own summary totals footer (line 38) says "9 major-rewrite, 5 minor-issues, 3 accurate" while the table shows 11/6/5. This factual error propagates to the Execution Summary. Fix is a one-line correction in the report and two lines in the spec.
