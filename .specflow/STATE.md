## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-146 added 8 missing documentation pages for launch readiness: troubleshooting, tutorials (index + todo app), FAQ, schema guide, changelog, community, benchmarks. Updated DocsSidebar.tsx with Tutorials and Resources sections. 8 commits, 2 review cycles, all fixes applied.
- SPEC-147 replaced ~195 hardcoded blue Tailwind classes with brand CSS token system. Added --brand/--brand-hover/--brand-subtle/--brand-muted to global.css :root/.dark/@theme. Zero blue- class residue across all TSX/Astro/MDX files. og-image.ts and SVG chart hex values retained as documented exceptions (satori/recharts cannot use CSS custom properties).
- SPEC-148 added DataFusion SQL query documentation page (sql-queries.mdx) with all 7 required sections, and added SQL Queries SubItem to DocsSidebar.tsx between Live Queries and Schema & Type Safety. 2 commits, all 9 acceptance criteria met.
- SPEC-149 fixed misleading auth/security/RBAC documentation: corrected `clientWssCode` in security.mdx (removed non-existent `token` field, added `storage`, used `setAuthToken()`), corrected false "default-deny" claims in rbac.mdac to accurately describe default-allow model. 2 commits.
- SPEC-150 fixed 5 mcp-server bugs: pagination race condition (Promise.race with 500ms timeout), QueryFilter type annotation, fields projection in QueryArgsSchema/toolSchemas.query, removed dead methods from SearchArgsSchema/toolSchemas.search, dynamic version from package.json via createRequire, fixed test mocks (CONNECTED uppercase, subscribe fires callback immediately). 77 tests pass.
- SPEC-151 removed non-functional env vars (`TOPGUN_PORT`) from deployment.mdx, replaced with `PORT`, added yellow "planned" banners above all TLS/cluster config sections. 1 commit, 1 review cycle.
- SPEC-152 rewrote observability.mdx: replaced ~30 fake Node.js metrics with 4 actual Rust server Prometheus metrics, removed 4 obsolete TS sections, added Metric Labels subsection, replaced Pino with tracing-subscriber docs, added Planned Metrics info box. 1 commit, 1 review cycle.
- SPEC-153 rewrote performance.mdx: replaced fake TS config knobs with actual Rust ServerConfig/ConnectionConfig/NetworkConfig defaults, replaced 5 fake Prometheus metrics with 4 real ones, fixed binary name to test-server, removed Trade-off Warning box, consolidated 3 config code blocks into 1. 2 commits, 2 review cycles, all fixes applied.
- SPEC-154 fixed cluster-replication.mdx: added yellow planned banners above all unimplemented env var sections, fixed gossip message names (JoinRequest/JoinResponse), marked QUORUM/STRONG as planned, removed non-existent metrics and health checks, fixed anti-entropy component names, updated AlertBox, Best Practices, and Data Flow. 3 commits, 2 review cycles.
