---
id: SPEC-152
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-03-25
source: TODO-187
delta: true
---

# Rewrite observability.mdx with Actual Rust Server Metrics and Tracing

## Context

The `observability.mdx` documentation page lists approximately 30 metric names from the old Node.js/TypeScript server architecture. None of these metrics exist in the current Rust server. The page also claims Pino JSON logging, whereas the Rust server uses `tracing-subscriber` with `TOPGUN_LOG_FORMAT=json`. Entire metric table sections (Event Queue, Backpressure, Connection Rate Limiting) describe Node.js event-loop concepts that have no equivalent in the tokio-based Rust server.

The actual Rust server exposes exactly four metrics (verified from `metrics_endpoint.rs` and `metrics.rs`):

| Metric | Type | Source |
|--------|------|--------|
| `topgun_active_connections` | Gauge | `metrics_endpoint.rs` line 35 |
| `topgun_operations_total` | Counter (labels: `service`, `outcome`) | `metrics.rs` line 96 |
| `topgun_operation_duration_seconds` | Histogram (label: `service`) | `metrics.rs` line 97 |
| `topgun_operation_errors_total` | Counter (labels: `service`, `error`) | `metrics.rs` line 111 |

Users following the current docs will create dashboards and alerts referencing non-existent metrics.

## Delta

### MODIFIED
- `apps/docs-astro/src/content/docs/guides/observability.mdx` — Full rewrite of metric tables and logging section
  - Remove existing "Partially Implemented" AlertBox warning banner
  - Replace "Key Metrics" table (5 fake metrics) with actual 4 Rust metrics table
  - Remove "Event Routing Metrics" section entirely (TS server artifact)
  - Remove "Event Queue Metrics" section entirely (TS server artifact)
  - Remove "Backpressure Metrics" section entirely (TS server artifact)
  - Remove "Connection Rate Limiting Metrics" section entirely (TS server artifact)
  - Rewrite "Alert Recommendations" to reference actual Rust metrics
  - Replace Pino logging section with Rust `tracing-subscriber` section
  - Replace JSON log example with Rust tracing JSON output format
  - Add "Planned Metrics" info box listing future metrics (map size, cluster members) with reference to TODO-137
  - Add `topgun_operation_errors_total` label values table documenting error kinds: `timeout`, `overloaded`, `wrong_service`, `internal`, `unknown_service`, `unauthorized`, `forbidden`, `value_too_large`, `schema_invalid`

## Requirements

### File: `apps/docs-astro/src/content/docs/guides/observability.mdx`

**Prometheus Metrics section:**

1. Replace the "Key Metrics" table with a single table containing exactly these four rows:
   - `topgun_active_connections` | Gauge | Number of currently connected WebSocket clients
   - `topgun_operations_total` | Counter | Total operations processed. Labels: `service` (crdt, sync, query, search, messaging, persistence, coordination), `outcome` (ok, error)
   - `topgun_operation_duration_seconds` | Histogram | Operation latency in seconds. Label: `service`
   - `topgun_operation_errors_total` | Counter | Total operation errors. Labels: `service`, `error` (timeout, overloaded, wrong_service, internal, unknown_service, unauthorized, forbidden, value_too_large, schema_invalid)

2. Remove the sentence "Standard Node.js metrics (CPU, Event Loop, GC) are also exported with the `topgun_` prefix."

3. Remove these entire sections (tables and headings):
   - "Event Routing Metrics"
   - "Event Queue Metrics"
   - "Backpressure Metrics"
   - "Connection Rate Limiting Metrics"

4. Add a "Metric Labels" subsection after the metrics table documenting:
   - `service` label values: crdt, sync, query, search, messaging, persistence, coordination
   - `error` label values: timeout, overloaded, wrong_service, internal, unknown_service, unauthorized, forbidden, value_too_large, schema_invalid

**Alert Recommendations section:**

5. Replace the four alert items with:
   - `topgun_operation_errors_total` increasing — operations are failing
   - `topgun_operation_duration_seconds` p99 above threshold — latency degradation
   - `topgun_active_connections` dropping to zero — server may be unreachable

**Structured Logging section:**

6. Replace "Pino" reference with "Rust `tracing-subscriber`"
7. State that JSON output is enabled via `TOPGUN_LOG_FORMAT=json` environment variable
8. State that log level filtering uses `RUST_LOG` environment variable (defaults to `info`)
9. Replace the `logExample` export with a Rust tracing JSON log example containing fields: `timestamp`, `level`, `target`, `span`, `fields`, `message`
10. Remove mention of "Pino" from the description text and the `<FileText>` intro paragraph

**Planned Metrics section:**

11. Add a "Planned Metrics" section after Alert Recommendations with an info-style AlertBox stating that additional metrics are planned for future releases, including: map item counts, cluster member counts, sync delta sizes, and persistence write latency. Reference TODO-137.

**Cleanup:**

12. Remove the existing "Partially Implemented" AlertBox at the top of the file (the page will be accurate after this rewrite)
13. Verify the `Gauge` import from `lucide-react` is still used after rewrite; remove it if no longer referenced to avoid lint warnings

## Acceptance Criteria

1. The file `observability.mdx` contains exactly four metric names: `topgun_active_connections`, `topgun_operations_total`, `topgun_operation_duration_seconds`, `topgun_operation_errors_total`
2. Zero occurrences of these removed metric names: `topgun_connected_clients`, `topgun_map_size_items`, `topgun_ops_total`, `topgun_memory_usage_bytes`, `topgun_cluster_members`, `topgun_events_routed_total`, `topgun_events_filtered_by_subscription`, `topgun_subscribers_per_event`, `topgun_event_queue_size`, `topgun_event_queue_enqueued_total`, `topgun_event_queue_dequeued_total`, `topgun_event_queue_rejected_total`, `topgun_backpressure_sync_forced_total`, `topgun_backpressure_pending_ops`, `topgun_backpressure_waits_total`, `topgun_backpressure_timeouts_total`, `topgun_connections_accepted_total`, `topgun_connections_rejected_total`, `topgun_connections_pending`, `topgun_connection_rate_per_second`
3. Zero occurrences of the word "Pino" in the file
4. Zero occurrences of "Node.js" in the file
5. The strings `TOPGUN_LOG_FORMAT=json` and `RUST_LOG` both appear in the Structured Logging section
6. A "Planned Metrics" section exists referencing TODO-137
7. The "Partially Implemented" AlertBox is removed
8. The file renders without MDX compilation errors (verified by `pnpm --filter docs-astro build` or dev server)
9. Navigation links (Previous: Cluster Replication, Next: Performance Tuning) remain intact at bottom of page

## Constraints

- Do not modify any Rust server code; this is documentation-only
- Do not invent metrics that do not exist in the Rust server source
- Do not remove the page's frontmatter, imports, or breadcrumb navigation structure
- Preserve the existing visual styling patterns (table classes, AlertBox components, icon usage)
- Do not add metric names from TODO-137 to the main metrics table; they belong only in the "Planned" info box

## Assumptions

- The four metrics identified in `metrics_endpoint.rs` and `metrics.rs` are the complete set of Prometheus metrics currently emitted by the Rust server
- The `service` label values correspond to the seven domain services listed in PROJECT.md (crdt, sync, query, search, messaging, persistence, coordination)
- The `error` label values are the nine variants of `OperationError` enum visible in `metrics.rs`
- The existing AlertBox component supports an "info" variant for the Planned Metrics box (consistent with usage in other docs pages)
- The Rust tracing JSON format includes standard fields: `timestamp`, `level`, `target`, `span`, `fields` (based on `tracing-subscriber` JSON layer defaults)

## Audit History

### Audit v1 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~10% total

**Source Verification:**
- 4 metrics confirmed in `metrics_endpoint.rs` (line 35) and `metrics.rs` (lines 96-111)
- 9 error kinds confirmed matching `OperationError` variants in `metrics.rs` (lines 100-109)
- AlertBox "info" variant confirmed in `AlertBox.tsx` (line 2)

**Delta validation:** 1/1 entries valid

**Dimensions:**
- Clarity: Excellent -- every requirement specifies exact content
- Completeness: All changes covered, single file scope
- Testability: All 9 acceptance criteria are concrete and verifiable
- Scope: Small, appropriate complexity rating
- Feasibility: All assumptions verified against source code
- Architecture fit: Follows existing MDX patterns
- Non-duplication: N/A (documentation fix)
- Cognitive load: Low
- Strategic fit: Aligned -- fixing misleading docs for completed Rust migration
- Project compliance: Honors PROJECT.md decisions, documentation-only change

**Recommendations:**
1. The `Gauge` import from `lucide-react` (line 8 of current file) may be unused after removing the Node.js metrics icon usage. Verify during implementation and remove if unused to avoid lint warnings.

**Comment:** Exceptionally well-specified documentation bugfix. Source-verified metrics, exhaustive negative acceptance criteria for removed content, and clear structural guidance. Ready for implementation.

### Response v1 (2026-03-25)
**Applied:** All recommendations from Audit v1

**Changes:**
1. [✓] Gauge import cleanup -- Added requirement #13 to verify `Gauge` lucide-react import is still used after rewrite and remove if unused

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~10% total

**Delta validation:** 1/1 entries valid

**Re-audit after revision:** Verified that Audit v1 recommendation was incorporated as requirement #13. All previous findings remain valid. No new issues found.

**Source re-verification:**
- 4 metrics confirmed: `metrics_endpoint.rs` line 35, `metrics.rs` lines 96-97, 111
- 9 `OperationError` variants confirmed in `operation.rs` lines 458-477, mapping in `metrics.rs` lines 100-109
- AlertBox "info" variant confirmed in `AlertBox.tsx` line 2
- `Gauge` icon (lucide-react) is used on line 37 of current file in the Prometheus section intro -- will remain used after rewrite since that section is updated, not removed

**Strategic fit:** Aligned with project goals
**Project compliance:** Honors PROJECT.md decisions

**Comment:** Revision successfully addressed the Gauge import concern. Spec is clean, complete, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25 18:36
**Commits:** 1

### Files Created
None.

### Files Modified
- `apps/docs-astro/src/content/docs/guides/observability.mdx` — Full rewrite: replaced fake Node.js metrics with 4 actual Rust metrics, removed 4 obsolete sections (Event Routing, Event Queue, Backpressure, Connection Rate Limiting), added Metric Labels subsection, updated alert recommendations, replaced Pino logging with Rust tracing-subscriber documentation, added Planned Metrics info box, removed Partially Implemented warning banner.

### Files Deleted
None.

### Acceptance Criteria Status
- [x] Exactly four metric names present: topgun_active_connections, topgun_operations_total, topgun_operation_duration_seconds, topgun_operation_errors_total
- [x] Zero occurrences of all 20 removed metric names (verified with grep)
- [x] Zero occurrences of "Pino"
- [x] Zero occurrences of "Node.js"
- [x] TOPGUN_LOG_FORMAT=json and RUST_LOG both present in Structured Logging section
- [x] "Planned Metrics" section with TODO-137 reference added
- [x] "Partially Implemented" AlertBox removed
- [x] MDX compiles without errors (pnpm --filter apps-docs-astro build: 65 pages built, Complete!)
- [x] Navigation links (Previous: Cluster Replication, Next: Performance Tuning) preserved

### Deviations
None.

### Notes
- The `Gauge` lucide-react import was retained as confirmed by Audit v2 — it remains used in the Prometheus section intro icon (line 36 of new file).
- The `Planned Metrics` subsection was placed under `## Prometheus Metrics` as a `###` heading with an AlertBox, matching the spec requirement to add it after Alert Recommendations.

---

## Review History

### Review v1 (2026-03-25 19:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: Exactly four metric names present — `topgun_active_connections`, `topgun_operations_total`, `topgun_operation_duration_seconds`, `topgun_operation_errors_total` (lines 54, 59, 64, 69)
- [✓] AC2: Zero occurrences of all 20 removed metric names — confirmed by grep, no matches
- [✓] AC3: Zero occurrences of "Pino" — confirmed by grep, no matches
- [✓] AC4: Zero occurrences of "Node.js" — confirmed by grep, no matches
- [✓] AC5: `TOPGUN_LOG_FORMAT=json` and `RUST_LOG` both present in Structured Logging section (line 127)
- [✓] AC6: "Planned Metrics" section exists with TODO-137 reference (lines 116-118)
- [✓] AC7: "Partially Implemented" AlertBox removed — not present anywhere in file
- [✓] AC8: Build verification — relied on execution summary (65 pages, Complete!); file structure is syntactically sound with well-formed JSX/MDX
- [✓] AC9: Navigation links intact — Previous: Cluster Replication (`/docs/guides/cluster-replication`), Next: Performance Tuning (`/docs/guides/performance`) at lines 140-151
- [✓] Gauge import still used — `<Gauge className="w-6 h-6 text-brand" />` at line 35
- [✓] Alert Recommendations — exactly three items referencing actual Rust metrics (lines 110-112)
- [✓] Metric Labels subsection — documents `service` and `error` label values (lines 77-103)
- [✓] Structured Logging section — uses `tracing-subscriber`, both env vars documented (lines 122-137)
- [✓] logExample uses Rust tracing JSON format with `timestamp`, `level`, `target`, `span`, `fields`, `message` fields (lines 11-18)
- [✓] No Rust server code modified — documentation-only change
- [✓] Planned metrics (map item counts, cluster member counts, sync delta sizes, persistence write latency) mentioned only in the info box, not added to main table
- [✓] Frontmatter, imports, and breadcrumb navigation structure preserved

**Summary:** Implementation fully satisfies all 9 acceptance criteria and all 13 requirements. The rewrite is accurate, clean, and consistent with the existing MDX patterns. No issues found.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Rewrote the observability.mdx documentation page to replace ~30 non-existent Node.js/TypeScript metric names with the 4 actual Rust server Prometheus metrics, replaced Pino logging references with Rust tracing-subscriber documentation, and added a Planned Metrics info box.

### Key Files

- `apps/docs-astro/src/content/docs/guides/observability.mdx` — Complete rewrite with accurate Rust server metrics, labels, alerts, and structured logging documentation

### Changes Applied

**Modified:**
- `apps/docs-astro/src/content/docs/guides/observability.mdx` — Full rewrite: replaced fake metrics table with 4 actual Rust metrics, removed 4 obsolete TS server sections, added Metric Labels subsection, updated alert recommendations, replaced Pino with tracing-subscriber, added Planned Metrics info box

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified.
