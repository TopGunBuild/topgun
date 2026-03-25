## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-140 audited all 22 remaining guide pages. Found 11 major-rewrite (7 unimplemented/stub + deployment/observability/performance/write-concern with critical mismatches), 6 minor-issues, 5 accurate. Created TODO-174 through TODO-180 for untracked missing features. Key finding: observability.mdx and performance.mdx use metric names and config knobs from the removed TS server.
- SPEC-141 created Shapes (Partial Replication) guide page at apps/docs-astro/src/content/docs/guides/shapes.mdx. All 9 sections present. Code examples use actual SyncEngine + subscribeShape() API. order: 12.5 (no collision).
- SPEC-142 extended QUERY_SUB/QUERY_RESP with Shape capabilities: optional fields projection on QuerySubPayload, optional merkle_root_hash on QueryRespPayload, new QUERY_SYNC_INIT message + Operation::QuerySyncInit variant routed to query service. 483 core + 610 server tests pass, TS build clean.
- SPEC-143 merged Shape capabilities into QueryService: field projection on QUERY_RESP/QUERY_UPDATE, per-query Merkle trees via QueryMerkleSyncManager, QUERY_SYNC_INIT handler, max_query_records clamping (default 10,000) with has_more, writer exclusion via CrdtService::broadcast_query_updates() (QueryMutationObserver removed from observer chain). 617 server tests pass, clippy-clean.
- SPEC-144 merged ShapeHandle/ShapeManager capabilities into QueryHandle/QueryManager: added fields to QueryFilter, added fields/merkleRootHash to QueryHandle, updated QueryManager to include fields in QUERY_SUB and send QUERY_SYNC_INIT on reconnect, deprecated SyncEngine.subscribeShape(), passed merkleRootHash through QUERY_RESP handler. 461 client tests + 63 integration tests pass, build clean.
- SPEC-145 removed all deprecated Shape code (Query+Shape merge step 4/4). Deleted 9 files, modified 20. Inlined project() as project_fields() into query.rs. Updated live-queries.mdx with fields/limit/Merkle reconnect docs. 589 server + 461 client + 57 integration tests pass. Zero shape references remain (SyncShape intentionally retained in schema module).
- SPEC-146 added 8 missing documentation pages for launch readiness: troubleshooting, tutorials (index + todo app), FAQ, schema guide, changelog, community, benchmarks. Updated DocsSidebar.tsx with Tutorials and Resources sections. 8 commits, 2 review cycles, all fixes applied.
