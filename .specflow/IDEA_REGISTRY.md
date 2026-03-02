# TopGun Idea Registry

**Purpose:** Centralized tracking of all ideas extracted from PROMPTS directory
**Created:** 2026-02-03
**Last Verified:** 2026-02-03
**Migrated to SpecFlow:** 2026-02-05
**Status:** Migrated with context preservation

---

## Migration Status (2026-02-05)

**Context files preserved in:** `.specflow/reference/`

| File | Source | TODO Coverage |
|------|--------|---------------|
| PHASE_4.5_CLIENT_CLUSTER_SPEC.md | PROMPTS | TODO-023 |
| HAZELCAST_DAG_EXECUTOR_SPEC.md | PROMPTS | TODO-025 |
| HAZELCAST_QUICK_WINS.md | PROMPTS | TODO-028, 029, 037, 038 |
| TURSO_INSIGHTS.md | PROMPTS | TODO-026, 027, 036, 042, 043, 044 |
| RUST_WASM_ANALYSIS.md | PROMPTS | TODO-034 |
| PHASE_15_VECTOR_SEARCH_SPEC.md | PROMPTS | TODO-039 |
| PHASE_5_MULTI_TENANCY_SPEC.md | PROMPTS | TODO-041 |
| topgun-rocksdb.md | Downloads | TODO-033, 040 |

**All 17 TODO items now have context links in:** `.specflow/todos/TODO.md`

---

## Verification Summary

| Status | Count | Notes |
|--------|-------|-------|
| ✅ Implemented | ~60 items | Confirmed in codebase |
| 📋 In TODO.md | 17 items | Migrated with context links |
| ❌ Rejected | 3 items | Not applicable to TopGun |

---

## Session 1: Research Documents (VERIFIED)

### GRAPHITI_INSIGHTS.md
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| GRAPH-01 | MCP Server Integration | ✅ Implemented | `packages/mcp-server/` |
| GRAPH-02 | Hybrid Search (BM25 + RRF) | ✅ Implemented | `ReciprocalRankFusion.ts` |
| GRAPH-03 | Bi-Temporal Data Model | 📋 TODO-044 | Low priority |

### HAZELCAST_*.md
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| HZ-01 | DAG Executor | 📋 TODO-025 | High priority |
| HZ-02 | Point Lookup Optimization | 📋 TODO-028 | Medium priority |
| HZ-03 | Network-aware Cost Model | 📋 TODO-037 | Low priority |
| HZ-04 | Partition Pruning | 📋 TODO-029 | Medium priority |
| HZ-05 | Index Hints | 📋 TODO-038 | Low priority |
| HZ-06 | Query Explain API | ✅ Implemented | MCP `explainTool` |
| HZ-07 | 3-tier Processor Model | 📋 Part of TODO-025 | |
| HZ-08 | Cooperative Threading | ❌ Not applicable | Node.js model |

### TURSO_INSIGHTS.md
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| TURSO-01 | HTTP Sync Fallback | 📋 TODO-026 | High priority |
| TURSO-02 | Deterministic Simulation Testing | 📋 TODO-027 | High priority |
| TURSO-03 | SQLite Cold Storage Adapter | ✅ Implemented | `BetterSqlite3Adapter.ts` |
| TURSO-04 | DBSP Incremental Views | 📋 TODO-042 | Low, high risk |
| TURSO-05 | Pluggable Extension System | 📋 TODO-036 | Medium priority |
| TURSO-06 | S3 Bottomless Storage | 📋 TODO-043 | Low priority |
| TURSO-07 | Time-Travel Queries | 📋 TODO-044 | Low priority |
| TURSO-08 | SQL Query Backend | ❌ Rejected | Conflicts with architecture |

### RUST_WASM_ANALYSIS.md
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| RUST-01 | MerkleTree WASM migration | 📋 TODO-034 | Medium, deferred |
| RUST-02 | CRDT Merge WASM migration | 📋 TODO-034 | |
| RUST-03 | SQL Parser (sqlparser-rs) | 📋 TODO-034 | |
| RUST-04 | DAG Executor WASM | 📋 TODO-034 | |

### topgun-rocksdb.md
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| ROCKS-01 | AsyncStorageWrapper (Write-Behind) | 📋 TODO-033 | Medium priority |
| ROCKS-02 | Staging Area (Read-Your-Writes) | 📋 TODO-033 | |
| ROCKS-03 | Write Coalescing | 📋 TODO-033 | |
| ROCKS-04 | Tiered Storage (Hot/Cold) | 📋 TODO-040 | Low priority |

---

## Session 2-3: Phase 11-14 (VERIFIED)

### Phase 11: Full-Text Search
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P11-01 | FTS Core (BM25, Tokenizer) | ✅ Implemented | `packages/core/src/search/` |
| P11-02 | FTS Cluster Support | ✅ Implemented | `ClusterSearchCoordinator.ts` |
| P11-11 | scoreSingleDocument O(1) | ✅ Implemented | |
| P11-12 | Notification Batching | ✅ Implemented | |

### Phase 12: Hybrid Search
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P12-01 | Reciprocal Rank Fusion | ✅ Implemented | `ReciprocalRankFusion.ts` |
| P12-07 | Unified Predicates API | ✅ Implemented | `predicate.ts` |
| P12-08 | matchPhrase predicate | ✅ Implemented | `Predicates.matchPhrase()` |
| P12-09 | matchPrefix predicate | ✅ Implemented | `Predicates.matchPrefix()` |
| P12-10 | multiMatch predicate | ✅ Implemented | `Predicates.multiMatch()` |

### Phase 13: MCP Server
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P13-01 | MCP Server Integration | ✅ Implemented | `packages/mcp-server/` |
| P13-02 | Stdio Transport | ✅ Implemented | Default transport |
| P13-03 | HTTP/SSE Transport | ✅ Implemented | `transport/http.ts` |
| P13-04 | topgun_query tool | ✅ Implemented | |
| P13-05 | topgun_mutate tool | ✅ Implemented | |
| P13-06 | topgun_search tool | ✅ Implemented | |
| P13-07 | topgun_subscribe tool | ✅ Implemented | |
| P13-08 | topgun_schema tool | ✅ Implemented | |
| P13-09 | topgun_explain tool | ✅ Implemented | |
| P13-10 | topgun_stats tool | ✅ Implemented | |

### Phase 14A: CLI + SQLite
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14-01 | CLI (doctor/setup/dev/test) | ✅ Implemented | `bin/` commands |
| P14-02 | SQLite Adapter | ✅ Implemented | `BetterSqlite3Adapter.ts` |
| P14A-04 | cluster:start/stop/status | ✅ Implemented | |

### Phase 14B: DevContainer + Docker
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14B-01 | DevContainer | ✅ Implemented | `.devcontainer/` |
| P14B-02 | Docker Compose profiles | ✅ Implemented | `docker-compose.yml` |
| P14B-09 | Dockerfile.server | ✅ Implemented | Multi-stage build |

### Phase 14C: Observability
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14-03 | Prometheus Metrics | ✅ Implemented | `PrometheusExporter.ts` |
| P14C-01 | CRDTDebugger class | ✅ Implemented | `debug/CRDTDebugger.ts` (510 lines) |
| P14C-06 | SearchDebugger class | ✅ Implemented | `debug/SearchDebugger.ts` (398 lines) |
| P14C-09 | Grafana Dashboard | ✅ Implemented | `deploy/grafana/dashboards/` |
| P14C-10 | MetricsServer endpoints | ✅ Implemented | `/metrics`, `/health`, `/ready` |

### Phase 14D: Admin UI
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14-04 | Admin UI MVP | ✅ Implemented | `apps/admin-dashboard/` |
| P14D-01 | Bootstrap Mode lifecycle | ✅ Implemented | |
| P14D-02 | Setup Wizard | ✅ Implemented | `SetupWizard.tsx` (531 lines) |
| P14D-03 | Data Explorer | ✅ Implemented | `DataExplorer.tsx` |
| P14D-04 | Query Playground | ✅ Implemented | `QueryPlayground.tsx` |
| P14D-05 | Cluster Topology | ✅ Implemented | `ClusterTopology.tsx` |
| P14D-07 | Command Palette | ✅ Implemented | `CommandPalette.tsx` |
| P14D-10 | shadcn/ui components | ✅ Implemented | |
| P14D-11 | Monaco Editor | ✅ Implemented | In QueryPlayground |

### Phase 14D: Settings
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14S-01 | Hot-reloadable settings | ✅ Implemented | `Settings.tsx` |

### Phase 14D: Auth Security
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P14Auth-01 | Login API | ✅ Implemented | `BootstrapController.ts` |
| P14Auth-02 | JWT generation | ✅ Implemented | |
| P14Auth-03 | Protected admin routes | ✅ Implemented | |
| P14Auth-07 | adminFetch wrapper | ✅ Implemented | `lib/api.ts` |

### Phase 14 Addendums: Distributed Search
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| DSEARCH-01 | ClusterSearchCoordinator | ✅ Implemented | Full scatter-gather |
| DSEARCH-02 | CLUSTER_SEARCH_REQ/RESP | ✅ Implemented | `cluster-schemas.ts` |
| DSEARCH-04 | SearchCursor | ✅ Implemented | `SearchCursor.ts` |

### Phase 14 Addendums: Cursor Pagination
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| CURSOR-01 | QueryCursor class | ✅ Implemented | `QueryCursor.ts` (403 lines) |
| CURSOR-02 | base64url.ts | ✅ Implemented | `utils/base64url.ts` |
| CURSOR-03 | hashObject utility | ✅ Implemented | `utils/hash.ts` |

### Phase 14 Addendums: Distributed Live
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| DLIVE-01 | DistributedSubscriptionCoordinator | ✅ Implemented | Full implementation |
| DLIVE-02 | CLUSTER_SUB_REGISTER | ✅ Implemented | `cluster-schemas.ts` |
| DLIVE-03 | CLUSTER_SUB_UPDATE | ✅ Implemented | |

### Query Optimizer
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P8-01 | Adaptive Indexing | ✅ Implemented | `query/adaptive/` (5+ files) |

---

## Session 4-5: Remaining Phases (VERIFIED)

### Phase 4.5: Client Cluster
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P4.5-01 | Client Smart Routing | 📋 TODO-023 | High priority |

### Phase 5: Multi-Tenancy
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P5-01 | Multi-Tenancy | 📋 TODO-041 | Low priority |
| P5-03 | PN Counter | ✅ Implemented | `PNCounter.ts` |
| P5-04 | Entry Processor | ✅ Implemented | |
| P5-05 | Event Journal | ✅ Implemented | `EventJournalService.ts` |
| P5-06 | Custom Conflict Resolvers | ✅ Implemented | |

### Phase 10: Cluster Enhancements
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P10-01 | Cluster E2E Tests | ✅ Implemented | `tests/e2e/cluster/` |
| P10-02 | Anti-Entropy Repair | ✅ Implemented | `RepairScheduler.ts` |
| P10-03 | Automatic Failover | ✅ Implemented | `PartitionReassigner.ts` |

### Phase 15: Vector Search
| ID | Idea | Status | Notes |
|----|------|--------|-------|
| P15-01 | Vector Search | 📋 TODO-039 | Low priority |

---

## Active TODO Items (17 total)

### High Priority (4)
| ID | Feature | Complexity |
|----|---------|------------|
| TODO-023 | Client Cluster Smart Routing | Large |
| TODO-025 | DAG Executor for Distributed Queries | Large |
| TODO-026 | HTTP Sync Fallback (Serverless) | Medium |
| TODO-027 | Deterministic Simulation Testing | Medium |

### Medium Priority (5)
| ID | Feature | Complexity |
|----|---------|------------|
| TODO-028 | Point Lookup Optimization | Low |
| TODO-029 | Partition Pruning | Medium |
| TODO-033 | AsyncStorageWrapper (Write-Behind) | Medium |
| TODO-034 | Rust/WASM Hot Path Migration | Large |
| TODO-036 | Pluggable Extension System | Medium |

### Low Priority (8)
| ID | Feature | Complexity |
|----|---------|------------|
| TODO-037 | Network-aware Cost Model | Low |
| TODO-038 | Index Hints | Low |
| TODO-039 | Vector Search | Large |
| TODO-040 | Tiered Storage | Large |
| TODO-041 | Multi-Tenancy | Large |
| TODO-042 | DBSP Incremental Views | Very Large |
| TODO-043 | S3 Bottomless Storage | Very Large |
| TODO-044 | Bi-Temporal Queries | Large |

---

## Rejected Ideas

| ID | Idea | Reason |
|----|------|--------|
| TURSO-08 | SQL Query Backend | Conflicts with CRDT/real-time architecture |
| HZ-08 | Cooperative Threading | Node.js event loop model incompatible |
| P11-03 | ORMap FTS Support | No clear semantic for FTS on OR-set |

---

## Key Findings

### What Was Already Implemented

The following major features were found to be fully implemented during verification:

1. **Observability Stack** - PrometheusExporter, Grafana dashboards, CRDTDebugger, SearchDebugger
2. **MCP Server** - 8 tools, HTTP/SSE transport, stdio transport
3. **Admin Dashboard** - SetupWizard, DataExplorer, QueryPlayground, ClusterTopology, CommandPalette, Auth
4. **Distributed Search** - ClusterSearchCoordinator, DistributedSubscriptionCoordinator, SearchCursor
5. **Cursor Pagination** - QueryCursor, SearchCursor, base64url utilities
6. **FTS Predicates** - matchPhrase, matchPrefix, multiMatch
7. **Adaptive Indexing** - IndexAdvisor, QueryPatternTracker, AutoIndexManager
8. **SQLite Storage** - BetterSqlite3Adapter
9. **Cluster E2E Tests** - Full test suite in `tests/e2e/cluster/`

### What Remains Unimplemented

Core gaps requiring implementation:

1. **DAG Executor** - No distributed query execution engine
2. **HTTP Sync** - WebSocket-only, no serverless support
3. **DST Testing** - No deterministic simulation framework
4. **Client Smart Routing** - Clients don't route to partition owners
5. **Query Optimizer Quick Wins** - Point lookup, partition pruning

---

*Last verified: 2026-02-03*
