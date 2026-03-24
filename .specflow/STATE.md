## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-139 fixed auth/security docs: removed userId JWT claim, replaced setAuthToken/localStorage with setAuthTokenProvider + in-memory cache, added Token Lifecycle section, added Better Auth JWT-minting bridge, marked TLS env vars and mTLS as planned, documented working env vars, documented trusted-origin bypass, rewrote RBAC to show current status vs planned, replaced all unqualified topgun-server binary references with planned qualifiers.
- SPEC-140 audited all 22 remaining guide pages. Found 11 major-rewrite (7 unimplemented/stub + deployment/observability/performance/write-concern with critical mismatches), 6 minor-issues, 5 accurate. Created TODO-174 through TODO-180 for untracked missing features. Key finding: observability.mdx and performance.mdx use metric names and config knobs from the removed TS server.
- SPEC-141 created Shapes (Partial Replication) guide page at apps/docs-astro/src/content/docs/guides/shapes.mdx. All 9 sections present. Code examples use actual SyncEngine + subscribeShape() API. order: 12.5 (no collision).
- SPEC-142 extended QUERY_SUB/QUERY_RESP with Shape capabilities: optional fields projection on QuerySubPayload, optional merkle_root_hash on QueryRespPayload, new QUERY_SYNC_INIT message + Operation::QuerySyncInit variant routed to query service. 483 core + 610 server tests pass, TS build clean.
- SPEC-143 merged Shape capabilities into QueryService: field projection on QUERY_RESP/QUERY_UPDATE, per-query Merkle trees via QueryMerkleSyncManager, QUERY_SYNC_INIT handler, max_query_records clamping (default 10,000) with has_more, writer exclusion via CrdtService::broadcast_query_updates() (QueryMutationObserver removed from observer chain). 617 server tests pass, clippy-clean.
