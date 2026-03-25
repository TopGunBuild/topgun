## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-143 merged Shape capabilities into QueryService: field projection on QUERY_RESP/QUERY_UPDATE, per-query Merkle trees via QueryMerkleSyncManager, QUERY_SYNC_INIT handler, max_query_records clamping (default 10,000) with has_more, writer exclusion via CrdtService::broadcast_query_updates() (QueryMutationObserver removed from observer chain). 617 server tests pass, clippy-clean.
- SPEC-144 merged ShapeHandle/ShapeManager capabilities into QueryHandle/QueryManager: added fields to QueryFilter, added fields/merkleRootHash to QueryHandle, updated QueryManager to include fields in QUERY_SUB and send QUERY_SYNC_INIT on reconnect, deprecated SyncEngine.subscribeShape(), passed merkleRootHash through QUERY_RESP handler. 461 client tests + 63 integration tests pass, build clean.
- SPEC-145 removed all deprecated Shape code (Query+Shape merge step 4/4). Deleted 9 files, modified 20. Inlined project() as project_fields() into query.rs. Updated live-queries.mdx with fields/limit/Merkle reconnect docs. 589 server + 461 client + 57 integration tests pass. Zero shape references remain (SyncShape intentionally retained in schema module).
- SPEC-146 added 8 missing documentation pages for launch readiness: troubleshooting, tutorials (index + todo app), FAQ, schema guide, changelog, community, benchmarks. Updated DocsSidebar.tsx with Tutorials and Resources sections. 8 commits, 2 review cycles, all fixes applied.
- SPEC-147 replaced ~195 hardcoded blue Tailwind classes with brand CSS token system. Added --brand/--brand-hover/--brand-subtle/--brand-muted to global.css :root/.dark/@theme. Zero blue- class residue across all TSX/Astro/MDX files. og-image.ts and SVG chart hex values retained as documented exceptions (satori/recharts cannot use CSS custom properties).
- SPEC-148 added DataFusion SQL query documentation page (sql-queries.mdx) with all 7 required sections, and added SQL Queries SubItem to DocsSidebar.tsx between Live Queries and Schema & Type Safety. 2 commits, all 9 acceptance criteria met.
