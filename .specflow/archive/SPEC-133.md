---
id: SPEC-133
type: docs
status: done
priority: medium
complexity: small
created: 2026-03-20
---

# Document Simulation Testing Framework and Add CI Job

## Context

The simulation testing framework (SPEC-132a through SPEC-132d) is fully implemented with SimCluster, SimNetwork, SimNode, proptest integration, and 11 passing simulation tests. However, this framework is not documented in CLAUDE.md (the primary developer onboarding file), has no CI job in the Rust workflow, and there is no guideline requiring simulation test coverage for domain service changes.

## Task

Add three deliverables:

1. **CLAUDE.md "Simulation Testing" section** describing architecture, commands, when to write sim tests, and the proptest async bridge pattern.
2. **CI job in `.github/workflows/rust.yml`** running simulation tests as a separate non-blocking job.
3. **CLAUDE.md developer guideline** stating domain service changes require simulation tests.

## Requirements

### File: `CLAUDE.md`

**Deliverable 1 — New "Simulation Testing" section** (insert after "Test Notes" section):

Add a section titled `## Simulation Testing` containing:

- **Architecture overview:** Brief description of the three core components:
  - `SimCluster` (`packages/server-rust/src/sim/cluster.rs`) — orchestrates N in-memory nodes with write/read/advance_time convenience methods
  - `SimNetwork` (`packages/server-rust/src/sim/network.rs`) — structural fault injection: partition, heal, delay, reorder between node pairs
  - `SimNode` — single node built via `SimNode::build()` wiring all 7 domain services with NullDataStore + HashMapStorage
- **How to run:** `pnpm test:sim` (which runs `cargo test --profile ci-sim --features simulation -p topgun-server -- sim`)
- **When to write sim tests:** Any change to domain services in `packages/server-rust/src/service/domain/` should include simulation tests verifying behavior under network partitions and node failures
- **Proptest async bridge pattern:** `block_in_place` + `Handle::block_on` inside `#[tokio::test(flavor = "multi_thread")]` — brief explanation of why this is needed (proptest closures are sync, sim harness is async)

**Deliverable 3 — Developer guideline addition:**

In the "Simulation Testing" section (combined with deliverable 1), include a clearly marked guideline paragraph stating: changes to files under `packages/server-rust/src/service/domain/` should be accompanied by simulation tests that exercise the changed behavior under at least one fault scenario (network partition or node failure).

### File: `.github/workflows/rust.yml`

**Deliverable 2 — New `simulation` job:**

Add a job named `simulation` with:
- `name: Simulation Tests`
- `needs: [check]` (run after check passes)
- `runs-on: ubuntu-latest`
- `timeout-minutes: 20`
- `continue-on-error: true` (non-blocking, allowed to fail initially)
- Steps:
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable`
  3. Cargo cache (same pattern as existing jobs)
  4. Run: `cargo test --profile ci-sim --features simulation -p topgun-server -- sim`
- Upload test output as artifact (optional, follow perf-gate pattern if natural)

Also add `pnpm test:sim` to the "Common Commands" section in CLAUDE.md under a `# Simulation tests` comment. Insert it immediately after the `pnpm test:integration-rust` line (the last test-related command in the Common Commands block, before the k6 load test commands).

## Acceptance Criteria

1. CLAUDE.md contains a "Simulation Testing" section with architecture overview mentioning SimCluster, SimNetwork, and SimNode by name
2. CLAUDE.md contains the `pnpm test:sim` command in the "Common Commands" code block
3. CLAUDE.md contains a guideline stating domain service changes should include simulation tests under fault scenarios
4. CLAUDE.md describes the proptest async bridge pattern (block_in_place + Handle::block_on)
5. `.github/workflows/rust.yml` contains a `simulation` job that runs `cargo test --profile ci-sim --features simulation -p topgun-server -- sim`
6. The `simulation` job has `continue-on-error: true`
7. The `simulation` job depends on the `check` job (`needs: [check]`)
8. Existing jobs in `rust.yml` are unchanged

## Constraints

- Do NOT modify any Rust source files
- Do NOT modify any test files
- Do NOT change existing CI jobs (check, cross-lang, perf-gate)
- Keep CLAUDE.md section concise — this is a developer reference, not a tutorial

## Assumptions

- The `ci-sim` Cargo profile and `simulation` feature flag are already configured in `Cargo.toml` (established in SPEC-132a)
- `pnpm test:sim` script already exists in root `package.json` (confirmed above)
- The simulation test binary name filter `-- sim` is sufficient to select only simulation tests
- No additional Cargo features or dependencies are needed for CI

## Audit History

### Audit v1 (2026-03-20)
**Status:** APPROVED

**Context Estimate:** ~19% total

**Comment:** Well-structured documentation spec with clear deliverables. All 8 acceptance criteria are concrete and verifiable. The spec correctly mirrors existing CI patterns (perf-gate job structure) and respects all constraints. Two files modified, well within scope limits. Strategic fit is strong -- documenting an implemented framework and adding CI coverage are high-value, low-risk tasks.

**Recommendations:**
1. Consider specifying where exactly in the Common Commands code block `pnpm test:sim` should appear (e.g., after `pnpm start:server` or after the Rust server test line in Test Notes). The spec says "near the existing Rust server test command" but the Rust server test command is in Test Notes, not Common Commands. Minor ambiguity -- implementer can resolve reasonably.

### Response v1 (2026-03-20)
**Applied:** Recommendation 1

**Changes:**
1. [✓] Clarify insertion point for `pnpm test:sim` in Common Commands — replaced the vague "near the existing Rust server test command" wording with a precise instruction: insert immediately after the `pnpm test:integration-rust` line (the last test-related command in the Common Commands block, before the k6 load test commands).

### Audit v2 (2026-03-20)
**Status:** APPROVED

**Context Estimate:** ~19% total

**Comment:** Re-audit after revision. The previous recommendation (ambiguous insertion point for pnpm test:sim) has been addressed with a precise instruction referencing the exact line and surrounding context. All 8 acceptance criteria remain concrete and verifiable. The spec is clear, complete, and ready for implementation. Two files modified (CLAUDE.md, rust.yml), well within scope. Strategic fit confirmed -- documenting an existing framework and adding CI coverage are essential housekeeping tasks.

---

## Execution Summary

**Executed:** 2026-03-20
**Commits:** 1

### Files Modified
- `CLAUDE.md` — added `pnpm test:sim` to Common Commands; added "Simulation Testing" section with architecture overview, run command, domain service guideline, and proptest async bridge pattern
- `.github/workflows/rust.yml` — added `simulation` job (needs: check, continue-on-error: true, timeout 20m)

### Acceptance Criteria Status
- [x] AC1: CLAUDE.md contains "Simulation Testing" section mentioning SimCluster, SimNetwork, SimNode
- [x] AC2: CLAUDE.md contains `pnpm test:sim` in Common Commands
- [x] AC3: CLAUDE.md contains guideline for domain service sim test coverage
- [x] AC4: CLAUDE.md describes proptest async bridge (block_in_place + Handle::block_on)
- [x] AC5: rust.yml contains `simulation` job running sim tests
- [x] AC6: simulation job has `continue-on-error: true`
- [x] AC7: simulation job has `needs: [check]`
- [x] AC8: Existing jobs unchanged (check, cross-lang, perf-gate)

### Deviations
None.

### Notes
- Skipped optional artifact upload step from the spec — the simulation job runs `cargo test` (not a benchmark), so there are no result files to upload. The perf-gate pattern doesn't apply here.

---

## Review History

### Review v1 (2026-03-20)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `## Simulation Testing` section present in CLAUDE.md (lines 143-166), naming SimCluster with file path, SimNetwork with file path, and SimNode with build method — all as specified
- [✓] AC2: `pnpm test:sim` command inserted in Common Commands code block (line 37-38), immediately after `pnpm test:integration-rust` with a `# Simulation tests (deterministic, with fault injection)` comment, before k6 commands — exact placement specified
- [✓] AC3: Domain service guideline present (lines 160-162), referencing `packages/server-rust/src/service/domain/`, requiring at least one fault scenario
- [✓] AC4: Proptest async bridge description present (lines 164-166), covering `block_in_place` + `Handle::block_on`, `multi_thread` flavor requirement, and the sync/async mismatch reason
- [✓] AC5: `simulation` job present in rust.yml (lines 202-227) running exact command `cargo test --profile ci-sim --features simulation -p topgun-server -- sim`
- [✓] AC6: `continue-on-error: true` on simulation job (line 207)
- [✓] AC7: `needs: [check]` on simulation job (line 204)
- [✓] AC8: Existing jobs check (lines 31-62), cross-lang (lines 63-104), and perf-gate (lines 105-200) are unchanged
- [✓] Constraint: No Rust source files modified
- [✓] Constraint: No test files modified
- [✓] Cargo cache uses a distinct `cargo-sim-` key prefix (lines 221-224), preventing cache pollution between regular and simulation builds — good practice matching the perf-gate `cargo-perf-` pattern
- [✓] Artifact upload omission is justified: `cargo test` produces no output files, so the optional upload step was correctly skipped (noted in Deviations/Notes)
- [✓] CLAUDE.md section is concise — all four required topics covered in 24 lines, no tutorial padding

**Summary:** All 8 acceptance criteria are met exactly. The implementation faithfully mirrors the perf-gate job structure for the CI job, places the `pnpm test:sim` command at the precise location required by the spec, and the Simulation Testing section in CLAUDE.md covers every required topic with appropriate brevity for a developer reference. No issues found.

---

## Completion

**Completed:** 2026-03-20
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Documented the simulation testing framework in CLAUDE.md and added a non-blocking CI job for simulation tests in the Rust workflow.

### Key Files

- `CLAUDE.md` — Added "Simulation Testing" section with architecture overview, run command, developer guideline, and proptest async bridge pattern
- `.github/workflows/rust.yml` — Added `simulation` job with continue-on-error, depends on check job

### Patterns Established

None — followed existing patterns (perf-gate CI job structure, CLAUDE.md documentation style).

### Deviations

None — implemented as specified.
