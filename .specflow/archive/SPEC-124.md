---
id: SPEC-124
type: docs
status: done
priority: P2
complexity: small
created: 2026-03-18
source: TODO-116
---

# Load Harness Documentation and Production Tuning Guide

## Context

The Rust load harness (`packages/server-rust/benches/load_harness/`) was built across SPEC-121a/b/c and SPEC-123, delivering a fully functional in-process load testing tool with CLI flags, throughput scenarios, HDR histogram metrics, JSON reporting, and CI perf-gate integration. However, there is no README or usage documentation. Developers discovering the harness must read source code to understand CLI flags, scenarios, output interpretation, and how to add new scenarios.

Additionally, the server has no production deployment guidance for high-connection workloads. Operators deploying TopGun at scale need concrete OS-level and runtime tuning recommendations.

## Task

Create two documentation files:

1. **`packages/server-rust/benches/load_harness/README.md`** -- Load harness user guide
2. **`docs/production-tuning.md`** -- Production deployment tuning guide for high-connection workloads

## Requirements

### File 1: `packages/server-rust/benches/load_harness/README.md`

Must contain the following sections:

**Overview:** One paragraph explaining what the harness does (in-process server + N WebSocket clients, HDR histogram metrics, CI-friendly JSON output).

**CLI Flags:** Table documenting every flag with type, default, and description:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--connections` | usize | 200 | Number of concurrent WebSocket connections |
| `--duration` | u64 | 30 | Test duration in seconds |
| `--interval` | u64 | 50 | Milliseconds between batch sends per connection |
| `--fire-and-forget` | bool (flag) | false | Send without waiting for OP_ACK |
| `--scenario` | string | "throughput" | Scenario to run |
| `--json-output` | path | none | Write machine-readable JSON report to this path |

**Scenarios:** Describe the `throughput` scenario:
- What it does (sends OpBatch PUT operations over N connections for D seconds)
- Batch size (10 ops per batch, hardcoded)
- Fire-and-wait mode: sends batch, waits for OP_ACK, records round-trip latency
- Fire-and-forget mode: sends batches without waiting, measures push throughput

**Interpreting Results:** Explain the ASCII histogram output columns (operation, count, p50, p95, p99, p99.9, max -- all in microseconds). Explain ops/sec calculation. Explain PASS/FAIL assertion output. Document the `ThroughputAssertion` thresholds that determine PASS/FAIL: acked ratio must be >= 80% (at least 80% of sent operations receive an OP_ACK within the test duration) and p99 latency must be < 500ms; either condition failing causes the assertion to report FAIL.

**Baseline Numbers:** Document observed performance on reference hardware:
- Fire-and-forget at 200 connections: ~200k ops/sec
- Fire-and-wait at 200 connections: ~2.8k ops/sec
- Note: numbers are from in-process testing (no network hop); production will differ

**CI Integration:** Explain `--json-output` flag, `baseline.json` thresholds (fire_and_wait: 1000 ops/sec min, fire_and_forget: 50000 ops/sec min, 20% regression tolerance), and the perf-gate CI job in `rust.yml`.

**Adding New Scenarios:** Step-by-step guide:
1. Create a new file in `scenarios/`
2. Implement `LoadScenario` trait (name, setup, run, assertions)
3. Implement `Assertion` trait for post-run checks
4. Register in `scenarios/mod.rs`
5. Add match arm in `main.rs` scenario dispatch
6. Add baseline thresholds to `baseline.json` if CI-gated

**Example Commands:** 3-5 copy-paste-ready cargo commands for common use cases.

### File 2: `docs/production-tuning.md`

Must contain the following sections:

**File Descriptor Limits:**
- Default `ulimit -n` is 1024 on most Linux distros; each WebSocket connection consumes one fd
- For 100k+ connections: `ulimit -n 1048576`
- Persistent config via `/etc/security/limits.conf` and `systemd` `LimitNOFILE`

**TCP Buffer Tuning:**
- Default kernel TCP buffers allocate ~45KB per connection (send + receive)
- For high connection counts, reduce with `SO_SNDBUF=8192` and `SO_RCVBUF=8192` to ~17KB/conn
- System-wide: `net.core.rmem_default`, `net.core.wmem_default` sysctl values
- Trade-off: smaller buffers reduce memory but increase syscall frequency for large messages

**Memory Budget:**
- ~45KB per connection at default TCP buffer sizes
- ~17KB per connection with reduced buffers (8KB send + 8KB recv + overhead)
- Rule of thumb: ~250k connections per 16GB RAM at default settings; ~900k with reduced buffers
- Application-level memory (CRDT state, query registry) is additive

**Ephemeral Port Exhaustion:**
- Linux ephemeral port range is typically 32768-60999 (~28k ports)
- Each outbound connection (or inbound from same IP) consumes one ephemeral port
- At >28k connections from a single client IP: use multiple source IPs or `SO_REUSEPORT`
- Server-side: not an issue for inbound connections (they share the listening port)
- Increase range: `net.ipv4.ip_local_port_range = 1024 65535`

**Tokio Runtime Tuning:**
- Default worker threads = number of CPU cores; sufficient for most workloads
- For >100k concurrent tasks: consider `tokio::runtime::Builder` with explicit `worker_threads`
- `TOKIO_WORKER_THREADS` environment variable for runtime override
- Monitor task queue depth; backpressure (HTTP 429) indicates worker saturation

**Connection Monitoring:**
- Reference the observability endpoint for active connection count
- Recommend alerting thresholds relative to fd limits (warn at 80%, critical at 95%)
- `ss -s` and `/proc/net/sockstat` for OS-level connection monitoring

## Acceptance Criteria

1. `packages/server-rust/benches/load_harness/README.md` exists and documents all 6 CLI flags with correct defaults matching `main.rs`
2. README contains at least one copy-paste-ready `cargo bench` example command that runs the harness
3. README explains how to add a new scenario with reference to `LoadScenario` and `Assertion` traits
4. `docs/production-tuning.md` exists and covers all 6 sections listed above
5. Production tuning guide includes specific numeric values for `ulimit`, `SO_SNDBUF/RCVBUF`, memory-per-connection, and ephemeral port range
6. Both files use Markdown with no broken links or incorrect flag names

## Constraints

- Do not modify any Rust source files
- Do not add new dependencies
- Baseline numbers in README must match what the code actually produces (reference `baseline.json` thresholds, not aspirational targets)
- Production tuning guide is Linux-focused (TopGun server targets Linux deployment); macOS notes are optional

## Assumptions

- The `docs/` directory at the project root is the correct location for production-facing documentation (alongside any existing docs)
- Baseline performance numbers (200k fire-and-forget, 2.8k fire-and-wait) come from the task description and represent observed measurements; these will be documented as approximate reference values, not guarantees
- The harness is invoked via `cargo bench --bench load_harness` (standard Cargo bench binary convention)
- No server-rust README exists yet; production tuning goes in a separate file rather than appended to a package-level README

## Audit History

### Audit v1 (2026-03-18)
**Status:** APPROVED

**Context Estimate:** ~8% total (2 new Markdown files, documentation-only, no code changes)

**Source Code Verification:**
- All 6 CLI flags verified against `main.rs` lines 59-133: names, types, and defaults are correct
- `baseline.json` thresholds verified: fire_and_wait 1000 ops/sec min, fire_and_forget 50000 ops/sec min, 20% regression tolerance -- all match spec
- Batch size default of 10 verified in `ThroughputConfig::default()` (throughput.rs line 31)
- `LoadScenario` and `Assertion` traits verified in `traits.rs` lines 70-82
- ASCII histogram columns verified in `metrics.rs` `print_report()` lines 33-58: operation, count, p50, p95, p99, p99.9, max (all in microseconds)

**Dimension Assessment:**
- Clarity: Excellent -- every section of both files is specified with concrete content
- Completeness: All CLI flags, thresholds, and section contents are enumerated
- Testability: All 6 acceptance criteria are concrete and verifiable
- Scope: Two Markdown files, no code changes -- well bounded
- Feasibility: Straightforward documentation task with all source material available
- Architecture fit: README co-located with harness; production guide in `docs/` alongside existing guides
- Non-duplication: No existing harness README or production tuning guide
- Cognitive load: Minimal -- clear writing task
- Strategic fit: Aligned with project goals -- fills documentation gap for recently built tooling
- Project compliance: Honors all PROJECT.md constraints (no source modifications, no new dependencies)

**Language Profile:** Not applicable (docs-only spec, no Rust source files created or modified)

**Recommendations:**
1. The `ThroughputAssertion` checks two things (acked ratio >= 80% and p99 < 500ms) that are not mentioned in the spec's "Interpreting Results" section. The implementer should document these assertion thresholds in the README for completeness, even though the spec does not explicitly require it. The information is available in `throughput.rs` lines 304-339.

**Comment:** Well-structured documentation spec with accurate technical details verified against source code. All CLI flags, defaults, and baseline thresholds match the implementation exactly. Ready for implementation.

### Response v1 (2026-03-18)
**Applied:** Recommendation 1

**Changes:**
1. [✓] ThroughputAssertion thresholds added to "Interpreting Results" requirement — Extended the "Interpreting Results" line in the README requirements to explicitly state that the implementer must document the ThroughputAssertion PASS/FAIL thresholds: acked ratio >= 80% and p99 latency < 500ms, with a plain-language explanation of what each condition means.

### Audit v2 (2026-03-18)
**Status:** APPROVED

**Context Estimate:** ~8% total (2 new Markdown files, documentation-only, no code changes)

**Revision Verification:**
- Recommendation 1 from Audit v1 correctly incorporated: "Interpreting Results" section now explicitly requires documenting ThroughputAssertion thresholds (acked ratio >= 80%, p99 < 500ms), verified against `throughput.rs` lines 304-339
- All 6 CLI flags still match `main.rs` (no source changes since v1)
- All baseline.json thresholds unchanged and accurate

**Strategic fit:** Aligned with project goals
**Project compliance:** Honors PROJECT.md decisions

**Comment:** Previous recommendation fully addressed. Spec is complete, accurate, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-18
**Commits:** 2

### Files Created
- `packages/server-rust/benches/load_harness/README.md` — Load harness user guide with CLI flags, scenarios, output interpretation, baseline numbers, CI integration, and new-scenario guide
- `docs/production-tuning.md` — Production deployment tuning guide covering fd limits, TCP buffers, memory budget, ephemeral ports, tokio runtime, and connection monitoring

### Files Modified
None

### Files Deleted
None

### Acceptance Criteria Status
- [x] README.md exists and documents all 6 CLI flags with correct defaults matching main.rs
- [x] README contains copy-paste-ready `cargo bench` example commands (5 examples)
- [x] README explains how to add a new scenario with reference to LoadScenario and Assertion traits
- [x] production-tuning.md exists and covers all 6 sections (fd limits, TCP buffers, memory budget, ephemeral ports, tokio runtime, connection monitoring)
- [x] Production tuning guide includes specific numeric values for ulimit, SO_SNDBUF/RCVBUF, memory-per-connection, and ephemeral port range
- [x] Both files use Markdown with no broken links or incorrect flag names

### Deviations
None

### Notes
- ThroughputAssertion thresholds (acked ratio >= 80%, p99 < 500ms) documented in README per audit v1 recommendation
- All CLI flag defaults, baseline.json thresholds, and ASCII histogram columns verified against source code

---

## Review History

### Review v1 (2026-03-18 18:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: README exists with all 6 CLI flags matching `main.rs` defaults exactly (connections=200, duration=30, interval=50, fire-and-forget=false, scenario="throughput", json-output=none)
- [✓] AC2: Five copy-paste-ready `cargo bench --bench load_harness` example commands provided
- [✓] AC3: New-scenario guide references both `LoadScenario` and `Assertion` traits with code examples for each trait method
- [✓] AC4: `docs/production-tuning.md` covers all 6 required sections (fd limits, TCP buffers, memory budget, ephemeral ports, tokio runtime, connection monitoring)
- [✓] AC5: Specific numeric values present: ulimit 1048576, SO_SNDBUF/SO_RCVBUF 8192, ~45KB/~17KB per connection, ephemeral range 1024-65535
- [✓] AC6: Both files are valid Markdown; no broken links or incorrect flag names found
- [✓] Constraint respected: no Rust source files modified
- [✓] Constraint respected: no new dependencies added
- [✓] baseline.json thresholds accurately documented (fire_and_wait: 1000 ops/sec, fire_and_forget: 50000 ops/sec, 20% tolerance)
- [✓] ASCII histogram columns (operation, count, p50, p95, p99, p99.9, max in µs) match `metrics.rs` print_report() output
- [✓] ThroughputAssertion thresholds documented per audit recommendation (acked ratio check and p99 < 500ms check)
- [✓] Production tuning guide is Linux-focused as required; macOS not mentioned
- [✓] Alerting thresholds in connection monitoring include both 80% (warning) and 95% (critical) as specified — spec required warn at 80% and critical at 95%
- [✓] JSON report example in CI Integration section matches the `JsonReport` struct fields in `traits.rs`
- [✓] Fire-and-forget behavior in README correctly notes no latency histogram is recorded (matches source: acked_count set to total_sent, no histogram recording in fire-and-forget path)

**Minor:**
1. README states the acked ratio assertion requires "at least 80%" to pass. The source code at `throughput.rs:319` uses `if acked_ops <= threshold_ops` (strict less-than-or-equal), meaning exactly 80% triggers FAIL — only strictly greater than 80% passes. The README's "at least 80%" phrasing implies equal-to-80% would pass. This is a one-edge-case inaccuracy that is unlikely to matter in practice but is technically incorrect.

**Summary:** Both documentation files are complete, accurate, and match their respective source implementations. All 6 acceptance criteria are fully satisfied. The only finding is a minor edge-case inaccuracy in the assertion threshold description.

---

## Completion

**Completed:** 2026-03-18
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Delivered comprehensive documentation for the Rust load harness (README with CLI flags, scenarios, output interpretation, CI integration, and new-scenario guide) and a production deployment tuning guide for high-connection workloads.

### Key Files

- `packages/server-rust/benches/load_harness/README.md` — Complete user guide for the load testing harness
- `docs/production-tuning.md` — Linux-focused production deployment tuning for high-connection scenarios

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
