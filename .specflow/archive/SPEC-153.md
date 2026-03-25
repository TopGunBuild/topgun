---
id: SPEC-153
type: docs
status: done
priority: P1
complexity: small
created: 2026-03-25
source: TODO-188
delta: true
---

# Fix performance.mdx — Replace TS Server Config with Actual Rust Config

## Context

`performance.mdx` documents non-existent TS server configuration knobs (`eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs`, etc.). None of these exist in the Rust server. The binary name `topgun-server` is incorrect (the actual binary is `test-server`). The monitoring metrics section references fabricated metric names (`topgun_event_queue_size`, `topgun_event_queue_rejected_total`, `topgun_backpressure_timeouts_total`, `topgun_backpressure_pending_ops`, `topgun_connections_rejected_total`). Only 4 actual Prometheus metrics exist in the Rust server:

- `topgun_active_connections` (gauge)
- `topgun_operations_total` (counter, labels: service, outcome)
- `topgun_operation_duration_seconds` (histogram, labels: service)
- `topgun_operation_errors_total` (counter, labels: service, error)

The actual tunable configs are:

- `ServerConfig`: `max_concurrent_operations` (default 1000), `default_operation_timeout_ms` (30000), `gc_interval_ms` (60000), `max_query_records` (10000)
- `ConnectionConfig`: `outbound_channel_capacity` (256), `send_timeout` (5s), `idle_timeout` (60s), `ws_write_buffer_size` (128KB), `ws_max_write_buffer_size` (512KB)
- `NetworkConfig`: `request_timeout` (30s)

These are compile-time Rust defaults, not env-var configurable. The page must be rewritten to reflect this reality.

## Delta

### MODIFIED
- `apps/docs-astro/src/content/docs/guides/performance.mdx` — Full rewrite to replace TS fiction with Rust reality
  - Remove all `eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs` references
  - Fix binary name from `topgun-server` to `test-server`
  - Replace fake metrics with actual 4 Prometheus metrics
  - Replace "When to Adjust" table with actual Rust config fields
  - Replace Quick Reference cards with actual Rust defaults
  - Update Grafana/PromQL queries to use real metric names
  - Update alerts table to use real metric names
  - Keep OS-Level Tuning section (Linux sysctl, file descriptors) — this is valid
  - Keep Performance Benchmarks section — numbers come from actual load harness

## Requirements

### File: `apps/docs-astro/src/content/docs/guides/performance.mdx`

1. **Remove warning banner** — The "Partially Implemented" AlertBox at line 11 must be removed since the page will now document only real config.

2. **Fix binary name** — Replace all occurrences of `topgun-server` with `test-server` in code blocks.

3. **Replace config code blocks** — Remove the three separate `highThroughputConfig`, `lowLatencyConfig`, and `productionConfig` code blocks and replace them with a single "Server Startup" code block. Since server-level tuning (concurrency, timeouts, buffer sizes) is compile-time only, there are no meaningful differences between profiles at the env-var level. The single block should show only real env vars (`PORT`, `DATABASE_URL`, `RUST_LOG`, `JWT_SECRET`) and the correct binary name. Add a comment noting that server-level tuning is controlled via Rust `ServerConfig`/`ConnectionConfig` structs at compile time, not env vars.

4. **Replace "Understanding the Optimizations" cards** — Remove "Bounded Event Queue", "Backpressure Regulation", "Write Coalescing" cards (these TS concepts do not exist). Replace with actual Rust server optimizations:
   - **Subscription-Based Routing** (keep — this is real)
   - **Tower LoadShed Middleware** — Rejects requests when `max_concurrent_operations` is exceeded
   - **Bounded Outbound Channels** — Per-connection `outbound_channel_capacity` prevents slow clients from causing OOM
   - **Async I/O via tokio** — Non-blocking runtime eliminates thread-per-connection overhead

5. **Replace "When to Adjust" table** — Remove the TS config table. Replace with a table of actual `ServerConfig`, `ConnectionConfig`, and `NetworkConfig` fields, their defaults, and when to change them:

   | Field | Default | Increase When | Decrease When |
   |-------|---------|---------------|---------------|
   | `max_concurrent_operations` | 1000 | High `topgun_operation_errors_total` with load-shed errors | Memory constrained |
   | `default_operation_timeout_ms` | 30000 | Complex queries timing out before completion | Want faster failure for hung operations |
   | `max_query_records` | 10000 | Large dataset queries returning truncated results | Memory constrained or query abuse concerns |
   | `outbound_channel_capacity` | 256 | Slow consumers causing send timeouts | Memory per connection too high |
   | `send_timeout` | 5s | Clients on high-latency networks | Want to drop slow clients faster |
   | `idle_timeout` | 60s | Clients reconnect frequently | Want to reclaim idle connections sooner |
   | `ws_write_buffer_size` | 128KB | Large messages being fragmented | Memory constrained |
   | `ws_max_write_buffer_size` | 512KB | Very large payloads needed | Memory per connection too high |
   | `request_timeout` | 30s | Complex queries timing out | Want faster failure for hung requests |
   | `gc_interval_ms` | 60000 | GC overhead too frequent | Stale data accumulating |

6. **Add "Configuration Note" info box** — After the config table, add an info box explaining these are Rust struct defaults, not env-var configurable. To change them, modify the config structs in `packages/server-rust/src/service/config.rs` and `packages/server-rust/src/network/config.rs` and rebuild. Future releases may expose env-var overrides.

7. **Remove "Latency vs Throughput Trade-offs" table and Trade-off Warning box** — This table documents `writeCoalescingMaxDelayMs` which does not exist. Also remove the "Trade-off Warning" AlertBox that accompanies it and references write coalescing, which does not exist in the Rust server.

8. **Replace "Critical Metrics" table** — Replace all 5 fake metrics with the 4 real ones:

   | Metric | Type | Labels | Healthy Range | Action if Exceeded |
   |--------|------|--------|---------------|-------------------|
   | `topgun_active_connections` | Gauge | — | Depends on capacity | Add nodes or increase file descriptors |
   | `topgun_operations_total` | Counter | service, outcome | Steady rate | Investigate spikes by service label |
   | `topgun_operation_duration_seconds` | Histogram | service | p99 < 500ms | Profile slow service, check DB |
   | `topgun_operation_errors_total` | Counter | service, error | Near 0 | Investigate by error label |

9. **Replace Grafana/PromQL queries** — Replace the `grafanaQueries` export with queries using real metrics:
   - `rate(topgun_operations_total[5m])` — Operation throughput
   - `histogram_quantile(0.99, rate(topgun_operation_duration_seconds_bucket[5m]))` — p99 latency
   - `rate(topgun_operation_errors_total[5m])` — Error rate
   - `topgun_active_connections` — Active connections

10. **Replace alerts table** — Replace all 4 alert rows with alerts based on real metrics:
    - **HighErrorRate**: `rate(topgun_operation_errors_total[5m]) > 10` — Critical
    - **HighLatency**: `histogram_quantile(0.99, ...) > 0.5` — Warning
    - **ConnectionSpike**: `topgun_active_connections > 1000` — Warning

11. **Replace Quick Reference cards** — Remove the three cards with TS config values. Replace with a single "Server Defaults" reference card listing actual `ServerConfig` and `ConnectionConfig` defaults from source code.

12. **Keep OS-Level Tuning section unchanged** — The Linux sysctl settings, file descriptor limits, and Docker/Kubernetes note are valid and infrastructure-level.

13. **Keep Performance Benchmarks section** — The 33,000+ ops/sec, 100 clients, <15ms p50 numbers are from the actual load harness. Keep as-is.

## Acceptance Criteria

1. Zero occurrences of `eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs`, `writeCoalescingEnabled` in the file
2. Zero occurrences of `topgun_event_queue_size`, `topgun_event_queue_rejected_total`, `topgun_backpressure_timeouts_total`, `topgun_backpressure_pending_ops`, `topgun_connections_rejected_total` in the file
3. Binary name `topgun-server` does not appear; `test-server` is used in code blocks
4. All 4 actual Prometheus metrics (`topgun_active_connections`, `topgun_operations_total`, `topgun_operation_duration_seconds`, `topgun_operation_errors_total`) are documented
5. Config fields documented match actual Rust source: `ServerConfig` in `packages/server-rust/src/service/config.rs` and `ConnectionConfig` in `packages/server-rust/src/network/config.rs`
6. Default values in documentation match actual `Default` impl values in Rust source
7. OS-Level Tuning section (sysctl, limits.conf, Docker note) is unchanged
8. Performance Benchmarks section is unchanged
9. Page renders without build errors (`pnpm build` in docs-astro)

## Constraints

- Do not invent config knobs that do not exist in source code
- Do not change the page's position in sidebar (order: 17)
- Do not modify navigation links (previous: Observability, next: MCP Server)
- Do not remove lucide-react imports that are still used
- Keep the existing MDX component patterns (CodeBlock, AlertBox, etc.)

## Assumptions

- `test-server` is the correct binary name based on `Cargo.toml` `[[bin]]` section; there is no separate production binary name
- The 4 Prometheus metrics found via `counter!`/`histogram!`/`gauge!` calls are the complete set of server metrics
- Config fields are not currently env-var configurable (they are Rust struct defaults only)
- The Performance Benchmarks numbers (33k ops/sec, 100 clients, <15ms p50) are accurate from the load harness and should be kept

## Audit History

### Audit v1 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~10% total (single MDX file rewrite, docs-only)

**Delta validation:** 1/1 entries valid
- MODIFIED `apps/docs-astro/src/content/docs/guides/performance.mdx`: File exists, change descriptions present. Pass.

**Strategic fit:** Aligned with project goals -- fixing documentation to match reality before launch.

**Project compliance:** Honors PROJECT.md decisions. This is a docs spec modifying an MDX file, not Rust packages, so Language Profile does not apply.

**Recommendations:**
1. The "When to Adjust" table (Requirement 5) omits 3 real config fields that are listed in the Context section: `default_operation_timeout_ms`, `max_query_records`, and `ws_max_write_buffer_size`. Consider adding them for completeness, or explicitly noting they are intentionally excluded (e.g., `max_query_records` may not be relevant to performance tuning).
2. The "Trade-off Warning" box (lines 211-217) about write coalescing should be explicitly called out for removal in the Requirements, since it references a non-existent feature. Requirement 7 removes the table below it but does not mention this warning box. The implementer should remove it, but it is not stated.
3. Requirement 3 says to keep three separate config code blocks (`highThroughputConfig`, `lowLatencyConfig`, `productionConfig`) but since server tuning is now compile-time only, the three profiles are nearly identical (just env vars). Consider consolidating into a single "Server Startup" code block to avoid the appearance of meaningfully different configurations.

**Comment:** Well-structured spec with precise, source-verified requirements. All config defaults and metric names verified against actual Rust source code. The 13 requirements are specific and actionable, acceptance criteria are measurable, and constraints are clear. Minor gaps noted in recommendations above.

### Response v1 (2026-03-25)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] Add missing config fields to "When to Adjust" table — Added `default_operation_timeout_ms` (30000), `max_query_records` (10000), and `ws_max_write_buffer_size` (512KB) rows to the table in Requirement 5, with appropriate "Increase When" and "Decrease When" guidance.
2. [✓] Explicitly call out Trade-off Warning box for removal — Updated Requirement 7 title and body to explicitly state that the "Trade-off Warning" AlertBox referencing write coalescing must also be removed, not just the table.
3. [✓] Consolidate three config code blocks into one — Revised Requirement 3 to replace the three separate `highThroughputConfig`, `lowLatencyConfig`, `productionConfig` code blocks with a single "Server Startup" code block, with an explanation that no meaningful profile differences exist at the env-var level when server tuning is compile-time only.

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~10% total (single MDX file rewrite, docs-only)

**Delta validation:** 1/1 entries valid
- MODIFIED `apps/docs-astro/src/content/docs/guides/performance.mdx`: File exists, change descriptions present. Pass.

**Source verification:**
- All 4 Prometheus metrics confirmed in Rust source (`metrics_endpoint.rs`, `metrics.rs`)
- All `ServerConfig` defaults confirmed: `max_concurrent_operations=1000`, `default_operation_timeout_ms=30000`, `gc_interval_ms=60000`, `max_query_records=10000` (in `service/config.rs`)
- All `ConnectionConfig` defaults confirmed: `outbound_channel_capacity=256`, `send_timeout=5s`, `idle_timeout=60s`, `ws_write_buffer_size=128KB`, `ws_max_write_buffer_size=512KB` (in `network/config.rs`)
- `NetworkConfig.request_timeout=30s` confirmed (in `network/config.rs`)
- Binary name `test-server` confirmed as the only `[[bin]]` entry in `Cargo.toml`

**Strategic fit:** Aligned with project goals -- fixing documentation to match reality before launch.

**Project compliance:** Honors PROJECT.md decisions. Docs-only spec, Language Profile does not apply.

**Comment:** All three recommendations from Audit v1 have been incorporated cleanly. The spec now covers all 10 config fields with verified defaults, explicitly removes the Trade-off Warning box, and consolidates config blocks into a single startup example. Requirements are precise and source-verified. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25
**Commits:** 1

### Files Created
None.

### Files Modified
- `apps/docs-astro/src/content/docs/guides/performance.mdx` — Full rewrite: removed fake TS config, replaced with actual Rust ServerConfig/ConnectionConfig/NetworkConfig defaults, replaced fake metrics with 4 real Prometheus metrics, removed Trade-off Warning box, replaced 3 Quick Reference cards with single Server Defaults card.

### Files Deleted
None.

### Acceptance Criteria Status
- [x] Zero occurrences of `eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs`, `writeCoalescingEnabled`
- [x] Zero occurrences of fake metric names (`topgun_event_queue_size`, etc.)
- [x] Binary name `topgun-server` removed; `test-server` used in code block
- [x] All 4 actual Prometheus metrics documented
- [x] Config fields match actual Rust source (`service/config.rs`, `network/config.rs`)
- [x] Default values match actual `Default` impl values in Rust source
- [x] OS-Level Tuning section (sysctl, limits.conf, Docker note) unchanged
- [x] Performance Benchmarks section unchanged
- [x] Page renders without build errors (build completed in 17.46s)

### Deviations
None.

### Notes
- The `AlertTriangle` lucide icon import was removed since the Trade-off Warning box was deleted and no other usage remained.
- The Quick Reference section now uses a single full-width card instead of three cards, as the three-profile distinction only made sense for the fake TS configs.
- `AlertBox variant="info"` was used for the Compile-Time Configuration note, consistent with the pattern in observability.mdx.

---

## Review History

### Review v1 (2026-03-25 19:55)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: Zero occurrences of all forbidden TS config names — confirmed by grep
- [✓] AC2: Zero occurrences of all 5 fake metric names — confirmed by grep
- [✓] AC3: Binary name `topgun-server` absent; `test-server` used in the startup code block (line 18) — confirmed against `Cargo.toml [[bin]]`
- [✓] AC4: All 4 real Prometheus metrics documented in Critical Metrics table (lines 181-184) and PromQL queries (lines 48-57)
- [✓] AC5: Config fields match actual Rust source — all 10 fields present in `service/config.rs` and `network/config.rs`
- [✓] AC6: All default values verified against `Default` impl: `max_concurrent_operations=1000`, `default_operation_timeout_ms=30000`, `max_query_records=10000`, `gc_interval_ms=60000`, `outbound_channel_capacity=256`, `send_timeout=5s`, `idle_timeout=60s`, `ws_write_buffer_size=128KB`, `ws_max_write_buffer_size=512KB`, `request_timeout=30s`
- [✓] AC7: OS-Level Tuning section (sysctl, limits.conf, Docker note) intact and unchanged
- [✓] AC8: Performance Benchmarks section (33,000+ ops/sec, 100 clients, <15ms p50) intact and unchanged
- [✓] AC9: Build passes cleanly — `pnpm build` completed in 16.69s with 65 pages built, no errors
- [✓] Req 1: Warning banner removed — no `AlertBox` with "Partially Implemented" text exists
- [✓] Req 3: Three separate config code blocks consolidated into single `serverStartup` export
- [✓] Req 4: Four optimization cards are all real Rust concepts (Subscription-Based Routing, Tower LoadShed, Bounded Outbound Channels, Async I/O via tokio)
- [✓] Req 6: Compile-Time Configuration `AlertBox variant="info"` present at line 168
- [✓] Req 7: Trade-off Warning box and Latency vs Throughput table absent; `AlertTriangle` import removed
- [✓] Req 10: Three alert rows use real metric names (HighErrorRate, HighLatency, ConnectionSpike)
- [✓] Req 11: Single "Server Defaults" quick reference card with all 10 config fields
- [✓] Navigation: `order: 17`, previous link to `/docs/guides/observability`, next link to `/docs/guides/mcp-server` all correct
- [✓] No invented config knobs — every field in the documentation exists in Rust source

**Minor:**
1. `CheckCircle` is imported on line 8 but is not used anywhere in the file body. Since `AlertTriangle` was correctly removed when the Trade-off Warning box was deleted, `CheckCircle` should have been audited similarly. It does not cause a build error (Astro/React tree-shakes unused imports silently) but it is dead code.
   - File: `apps/docs-astro/src/content/docs/guides/performance.mdx:8`

**Summary:** The implementation fully satisfies all 9 acceptance criteria and all 13 requirements. All config defaults match Rust source exactly, all fake content has been removed, and the build passes cleanly. One unused lucide import (`CheckCircle`) remains, which is a minor cleanup item.

### Fix Response v1 (2026-03-25)
**Applied:** Minor issue #1

**Fixes:**
1. [✓] Unused `CheckCircle` import — Removed from lucide-react import line
   - Commit: 6e97e04

---

### Review v2 (2026-03-25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix verified: `CheckCircle` is absent from the lucide-react import line (line 8) — confirmed by grep, zero occurrences
- [✓] AC1: Zero occurrences of all 5 forbidden TS config names — confirmed by grep
- [✓] AC2: Zero occurrences of all 5 fake metric names — confirmed by grep
- [✓] AC3: `topgun-server` absent from file; `test-server` present in startup code block — confirmed by grep
- [✓] All 7 remaining lucide imports (`ChevronRight`, `Zap`, `Gauge`, `Timer`, `Server`, `Activity`, `Terminal`) are used in the file body — no new dead imports introduced
- [✓] No regressions introduced by the fix — only the `CheckCircle` token was removed from the import line, file is otherwise unchanged

**Summary:** The v1 minor issue (unused `CheckCircle` import, commit 6e97e04) was applied correctly. No regressions. The file is clean with no unused imports and all 9 acceptance criteria remain satisfied.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 2
**Review Cycles:** 2

### Outcome

Rewrote performance.mdx to replace all fictional TypeScript server configuration with actual Rust ServerConfig/ConnectionConfig/NetworkConfig defaults, real Prometheus metrics, and correct binary name.

### Key Files

- `apps/docs-astro/src/content/docs/guides/performance.mdx` — Performance tuning documentation now reflecting actual Rust server reality

### Changes Applied

**Modified:**
- `apps/docs-astro/src/content/docs/guides/performance.mdx` — Full rewrite: removed fake TS config knobs, replaced with actual Rust config defaults and real Prometheus metrics, fixed binary name, removed Trade-off Warning box, consolidated config code blocks

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified.
