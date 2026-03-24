---
id: SPEC-134
type: docs
status: done
priority: medium
complexity: small
created: 2026-03-20
---

# Document Load Harness and Performance Testing in CLAUDE.md

## Context

The Rust load harness (`packages/server-rust/benches/load_harness/`) is fully implemented with fire-and-wait/fire-and-forget modes, HDR histograms, baseline assertions, and JSON output. A perf-gate CI job already runs both modes on every push. Flamegraph profiling documentation exists at `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md`. However, none of this is documented in CLAUDE.md, which is the primary developer onboarding file. Developers working on hot-path code have no guidance to verify performance impact before merge.

## Task

Add documentation to CLAUDE.md covering the load harness, performance testing workflow, and a developer guideline for hot-path changes. Three additions:

1. Add `cargo bench --bench load_harness` to the Common Commands section.
2. Add a new "Performance Testing" section (after "Simulation Testing") describing the load harness architecture, run commands, CI perf-gate, and flamegraph workflow.
3. Include a developer guideline that changes to hot-path code (service routing, CRDT merge, WebSocket handling) should be verified with the load harness before merge.

## Requirements

### File: `CLAUDE.md`

**Addition 1 -- Common Commands section (line ~47, after CRDT micro-benchmarks):**

Add these entries to the existing bash code block:

```bash
# Rust load harness (in-process perf test)
cargo bench --bench load_harness

# Load harness: fire-and-forget mode
cargo bench --bench load_harness -- --fire-and-forget --interval 0
```

**Addition 2 -- New "Performance Testing" section** (insert after the "Simulation Testing" section, before end of file):

Section titled `## Performance Testing` containing:

- **Architecture overview:** The load harness (`packages/server-rust/benches/load_harness/`) boots a full server instance (all 7 domain services, partition dispatcher, WebSocket handler) in-process, opens N WebSocket connections, and runs configurable scenarios while recording latency with HDR histograms. Results are printed as ASCII tables and optionally written as JSON for CI.
- **Modes:** Two execution modes:
  - Fire-and-wait (default): sends OpBatch, waits for OP_ACK, records round-trip latency
  - Fire-and-forget (`--fire-and-forget`): sends batches without waiting, measures raw push throughput
- **Key CLI flags table:** `--connections` (default 200), `--duration` (default 30s), `--interval` (default 50ms), `--fire-and-forget`, `--json-output`
- **Running locally:** Example commands for quick smoke test (`--connections 50 --duration 10`), full run, and fire-and-forget mode
- **Baseline assertions:** The harness enforces two pass/fail checks: acked ratio >= 80% and p99 latency < 500ms. Both must pass for exit code 0. Baseline thresholds for CI are defined in `packages/server-rust/benches/load_harness/baseline.json`.
- **CI perf-gate:** The `perf-gate` job in `.github/workflows/rust.yml` runs both fire-and-wait and fire-and-forget scenarios (200 connections, 15s each), compares results against baseline.json thresholds using `jq`, and is currently informational (`continue-on-error: true`).
- **Flamegraph profiling:** Reference to `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md` for flamegraph methodology, baseline numbers, and hot-path analysis. Mention the `release-with-debug` profile and `cargo flamegraph` with macOS Instruments.

**Addition 3 -- Developer guideline** (within the Performance Testing section):

A subsection titled `### When to Run the Load Harness` stating: Changes to hot-path code -- service routing (`service/dispatch/`, `service/middleware/`), CRDT merge (`service/domain/crdt/`), WebSocket handling (`network/handlers/`), or serialization -- should be verified with the load harness before merge. Run at minimum a fire-and-wait scenario and compare ops/sec against the baseline. Regressions over 20% require investigation or justification in the PR.

## Acceptance Criteria

1. CLAUDE.md Common Commands section contains `cargo bench --bench load_harness` and `cargo bench --bench load_harness -- --fire-and-forget --interval 0`.
2. CLAUDE.md contains a "Performance Testing" section with architecture overview, modes description, CLI flags, example commands, baseline assertions explanation, CI perf-gate description, and flamegraph reference.
3. CLAUDE.md contains a "When to Run the Load Harness" subsection naming the specific hot-path directories and the 20% regression threshold.
4. No changes to any file other than CLAUDE.md.
5. Existing CLAUDE.md content (Project Overview, Common Commands, Architecture, Build System, Commit Message Format, Code Comments Convention, Test Notes, Simulation Testing) is preserved unchanged except for the additions specified above.

## Constraints

- Do NOT modify any Rust source files, CI workflows, or benchmark code.
- Do NOT duplicate the full load harness README content -- summarize and reference it.
- Do NOT add new pnpm scripts -- `cargo bench` is the canonical command.
- Follow the established documentation style from the Simulation Testing section (SPEC-133) as a template for structure and tone.

## Assumptions

- The load harness README at `packages/server-rust/benches/load_harness/README.md` is the authoritative detailed reference; CLAUDE.md provides a concise summary with pointers.
- The `release-with-debug` Cargo profile already exists for flamegraph profiling.
- The 20% regression threshold matches the `regression_tolerance_pct` value in baseline.json.

## Audit History

### Audit v1 (2026-03-20)
**Status:** APPROVED

**Context Estimate:** ~8% total

**Comment:** Well-structured documentation spec with precise placement instructions, verified technical claims, and clear acceptance criteria. All three assumptions confirmed against source code: `regression_tolerance_pct` is 20 in baseline.json, the acked ratio >= 80% and p99 < 500ms assertions exist in ThroughputAssertion, and the `release-with-debug` profile is referenced in FLAMEGRAPH_ANALYSIS.md. The spec correctly follows the Simulation Testing section as a structural template. Single-file scope is appropriate for the complexity.

Strategic fit: Aligned with project goals -- fills a real developer onboarding gap for performance workflows.
Project compliance: Honors PROJECT.md decisions (docs-only, no new dependencies, no code changes).

**Recommendations:**
1. The spec mentions `--scenario` flag exists in the CLI parser (with "throughput" as the only current option) but does not include it in the CLI flags table. Consider adding it for completeness, or explicitly noting it is omitted because only one scenario exists.

---

## Execution Summary

**Executed:** 2026-03-20
**Commits:** 1

### Files Modified
- `CLAUDE.md` — Added load harness commands to Common Commands, added Performance Testing section with architecture, modes, CLI flags, baseline assertions, CI perf-gate, flamegraph reference, and When to Run the Load Harness guideline

### Acceptance Criteria Status
- [x] AC1: CLAUDE.md Common Commands contains `cargo bench --bench load_harness` and fire-and-forget variant
- [x] AC2: CLAUDE.md contains Performance Testing section with architecture, modes, CLI flags, example commands, baseline assertions, CI perf-gate, and flamegraph reference
- [x] AC3: CLAUDE.md contains When to Run the Load Harness subsection naming hot-path directories and 20% threshold
- [x] AC4: No changes to any file other than CLAUDE.md
- [x] AC5: Existing CLAUDE.md content preserved unchanged

### Deviations
None.

### Notes
Audit recommendation 1 (add `--scenario` flag to CLI table) was not applied — the spec explicitly did not include it, and only one scenario exists currently. Can be added when more scenarios are implemented.

---

## Review History

### Review v1 (2026-03-20)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: Both `cargo bench --bench load_harness` and `cargo bench --bench load_harness -- --fire-and-forget --interval 0` present in Common Commands bash block, correctly placed after CRDT micro-benchmarks
- [✓] AC2: `## Performance Testing` section present at end of file with all required subsections: architecture overview, Modes, CLI Flags table (5 flags), Running Locally code block (3 example commands), Baseline Assertions, CI Perf-Gate, and Flamegraph Profiling
- [✓] AC3: `### When to Run the Load Harness` subsection names all four hot-path directories (`service/dispatch/`, `service/middleware/`, `service/domain/crdt/`, `network/handlers/`) and states the 20% regression threshold
- [✓] AC4: Commit 3a22263 modifies only `CLAUDE.md` and `.specflow/STATE.md` — no Rust source files, CI workflows, or benchmark code touched
- [✓] AC5: Diff confirms all pre-existing sections (Project Overview, Common Commands, Architecture, Build System, Commit Message Format, Code Comments Convention, Test Notes, Simulation Testing) are preserved verbatim — only additive changes
- [✓] Placement: Performance Testing section inserted after Simulation Testing (after Proptest Async Bridge subsection), before end of file — matches spec instruction
- [✓] Style consistency: Structure and tone match the Simulation Testing section (subsection headers, bullet descriptions, fenced bash blocks) as required by constraints
- [✓] No duplication: Content is a concise summary with a pointer to FLAMEGRAPH_ANALYSIS.md — does not reproduce full load harness README
- [✓] No pnpm scripts added — `cargo bench` used as canonical command throughout

**Summary:** Implementation exactly matches the specification. All five acceptance criteria are satisfied, the single-file constraint is honored, and the content follows the established documentation style. No issues found.

---

## Completion

**Completed:** 2026-03-20
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Added comprehensive performance testing documentation to CLAUDE.md, covering the load harness architecture, CLI flags, baseline assertions, CI perf-gate, flamegraph profiling, and developer guidelines for hot-path changes.

### Key Files

- `CLAUDE.md` — Developer onboarding file, now includes Performance Testing section

### Patterns Established

None — followed existing patterns from the Simulation Testing section.

### Deviations

None — implemented as specified.
