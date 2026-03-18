---
id: SPEC-123
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-18
source: TODO-115
---

# Add Performance Regression CI Gate Using Rust Load Harness

## Context

SPEC-116 through SPEC-122 revealed how a single architectural flaw can drop throughput 2000x (from 200,000 ops/sec down to 100 ops/sec). The load harness built in SPEC-121a/b/c already exists at `packages/server-rust/benches/load_harness/` and can assert throughput thresholds. However, it is not integrated into CI, so performance regressions can silently merge.

This spec adds the load harness as a CI gate on every server-touching PR, with baseline assertions and JSON result output for trend tracking.

## Goal Statement

Every PR that touches `packages/server-rust/` runs the load harness automatically. If throughput drops >20% from baseline or absolute thresholds are missed, the CI job fails.

### Observable Truths

1. A GitHub Actions job runs the load harness on every PR touching server-rust code
2. The harness produces machine-readable JSON output alongside the human-readable ASCII report
3. A baseline file in the repo defines minimum acceptable thresholds
4. CI fails (exit code 1) when throughput drops below baseline thresholds
5. JSON results are uploaded as CI artifacts for trend tracking
6. Two modes are tested: fire-and-wait and fire-and-forget, each with independent thresholds

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `.github/workflows/rust.yml` (modified) | Add perf-gate job to existing Rust CI workflow |
| `packages/server-rust/benches/load_harness/baseline.json` (new) | Stores baseline thresholds for CI assertion |
| `packages/server-rust/benches/load_harness/main.rs` (modified) | Add `--json-output <path>` CLI flag to emit machine-readable results |
| `packages/server-rust/benches/load_harness/traits.rs` (modified) | Add `JsonReport` struct for serializable output |

## Task

### 1. Add JSON output to load harness

Add a `--json-output <path>` CLI argument to `main.rs`. When provided, after the scenario run completes, serialize a JSON report to the given path containing:

```json
{
  "scenario": "throughput",
  "mode": "fire-and-wait",
  "connections": 200,
  "duration_secs": 30,
  "total_ops": 120000,
  "ops_per_sec": 4000,
  "latency": {
    "p50_us": 45000,
    "p95_us": 120000,
    "p99_us": 250000,
    "p999_us": 400000
  },
  "assertions": [
    { "name": "throughput_assertion", "passed": true, "message": null }
  ],
  "timestamp": "2026-03-18T12:00:00Z"
}
```

Use `serde_json` for serialization. `serde_json` is already a regular dependency in `packages/server-rust/Cargo.toml` — no Cargo.toml change is needed. Define a `JsonReport` struct in `traits.rs` with `#[derive(Serialize)]`.

**Timestamp formatting note:** Neither `chrono` nor the `time` crate is available, and no new Cargo dependencies may be added. The `timestamp` field must be formatted manually using `std::time::SystemTime` and `UNIX_EPOCH` arithmetic. Convert the elapsed seconds to a UTC string in the format `YYYY-MM-DDTHH:MM:SSZ` using integer division on the Unix epoch value. No external crate is needed for this basic UTC formatting.

### 2. Create baseline threshold file

Create `packages/server-rust/benches/load_harness/baseline.json`:

```json
{
  "fire_and_wait": {
    "min_ops_per_sec": 1000,
    "max_p50_us": 1000000
  },
  "fire_and_forget": {
    "min_ops_per_sec": 50000,
    "max_p50_us": 1000000
  },
  "regression_tolerance_pct": 20
}
```

These thresholds are intentionally conservative (well below the 200k peak) to avoid CI flake on noisy CI runners. The `regression_tolerance_pct` field is documented but not used for dynamic comparison in this spec (reserved for future baseline-tracking).

### 3. Add CI job to rust.yml

Add a `perf-gate` job to `.github/workflows/rust.yml` that:

1. **Depends on** the `check` job (only run perf if tests pass)
2. **Runs on** `ubuntu-latest` with `timeout-minutes: 15` and `continue-on-error: true` (job is informational only and must not block merges)
3. **Setup steps:** Replicate the same checkout, Rust toolchain installation, and cargo cache steps used in the `check` job — the `perf-gate` job runs on a separate runner and needs its own toolchain/cache configuration
4. **Builds** the load harness in release mode: `cargo build --release --bench load_harness` (this explicit build step serves as an early-fail check — if the bench binary fails to compile it surfaces immediately, before the longer 15-second scenario runs begin)
5. **Runs fire-and-wait mode:**
   ```bash
   cargo bench --bench load_harness -- \
     --scenario throughput \
     --connections 200 \
     --duration 15 \
     --interval 50 \
     --json-output results-faw.json
   ```
6. **Runs fire-and-forget mode:**
   ```bash
   cargo bench --bench load_harness -- \
     --scenario throughput \
     --connections 200 \
     --duration 15 \
     --interval 0 \
     --fire-and-forget \
     --json-output results-faf.json
   ```
7. **Validates results** against baseline using inline shell:
   - Parse `results-faw.json` with `jq`
   - Assert `ops_per_sec >= baseline.fire_and_wait.min_ops_per_sec`
   - Assert `latency.p50_us <= baseline.fire_and_wait.max_p50_us`
   - Repeat for fire-and-forget results against `baseline.fire_and_forget`
   - Exit 1 on any failure
8. **Uploads** both JSON result files as GitHub Actions artifacts (retention: 90 days)

**CI duration:** 15s per mode + build time. Total job should be under 10 minutes.

**Trigger filter:** Same as existing `check` job (server-rust path changes).

### 4. Update ThroughputAssertion for mode-aware thresholds

The existing `ThroughputAssertion` uses hardcoded thresholds (80% ack ratio, p99 < 500ms). These remain as the harness-level assertions. The CI-level baseline comparison (from `baseline.json`) is an additional layer enforced by the shell script in the workflow, not by the Rust code. This avoids coupling the harness binary to the baseline file format.

## Requirements

### Files to Create

| File | Purpose |
|------|---------|
| `packages/server-rust/benches/load_harness/baseline.json` | Baseline thresholds for CI gate |

### Files to Modify

| File | Change |
|------|--------|
| `.github/workflows/rust.yml` | Add `perf-gate` job |
| `packages/server-rust/benches/load_harness/main.rs` | Add `--json-output` CLI flag, serialize report after run |
| `packages/server-rust/benches/load_harness/traits.rs` | Add `JsonReport`, `JsonLatency`, `JsonAssertionResult` structs with `#[derive(Serialize)]` |

### Files NOT Modified

- `scenarios/throughput.rs` -- no changes needed, assertion logic stays as-is
- `metrics.rs` -- snapshot API already provides all needed data
- `connection_pool.rs` -- no changes needed
- `Cargo.toml` -- `serde_json` is already a regular dependency, no change needed

## Acceptance Criteria

1. Running `cargo bench --bench load_harness -- --json-output /tmp/out.json` produces a valid JSON file at the specified path
2. The JSON file contains `scenario`, `mode`, `connections`, `duration_secs`, `total_ops`, `ops_per_sec`, `latency` (with `p50_us`, `p95_us`, `p99_us`, `p999_us`), `assertions` array, and `timestamp` fields
3. Without `--json-output`, behavior is unchanged (ASCII report only, no file written)
4. `baseline.json` exists and contains thresholds for both `fire_and_wait` and `fire_and_forget` modes
5. `.github/workflows/rust.yml` has a `perf-gate` job that depends on `check`
6. The `perf-gate` job runs both fire-and-wait and fire-and-forget scenarios
7. The `perf-gate` job fails if ops/sec falls below baseline thresholds
8. JSON result files are uploaded as GitHub Actions artifacts with 90-day retention
9. `cargo clippy --all-targets` passes with no new warnings
10. All existing tests continue to pass

## Validation Checklist

1. Run `cargo bench --bench load_harness -- --connections 10 --duration 5 --json-output /tmp/test.json` locally -- verify JSON file is written and parseable with `jq`
2. Run `cargo bench --bench load_harness -- --connections 10 --duration 5 --fire-and-forget --json-output /tmp/test-faf.json` -- verify fire-and-forget JSON output
3. Run `cargo clippy --all-targets --all-features -- -D warnings` -- no new warnings
4. Validate `baseline.json` is valid JSON with `jq . packages/server-rust/benches/load_harness/baseline.json`
5. Inspect `.github/workflows/rust.yml` -- confirm `perf-gate` job has `needs: [check]`, `continue-on-error: true`, checkout/toolchain/cache setup steps, correct cargo bench commands, jq validation steps, and artifact upload

## Constraints

- Do NOT increase CI time by more than 10 minutes total (use 15s duration, not 30s)
- Do NOT modify the existing `check` or `cross-lang` jobs in `rust.yml`
- Do NOT make the perf-gate job required for merge initially (add as informational/optional) -- it can be promoted to required after baseline stability is confirmed
- Do NOT store results in `logs/` for CI (it is gitignored) -- use GitHub Actions artifacts instead
- Do NOT add new Cargo dependencies
- Keep harness changes backward-compatible: `--json-output` is optional

## Assumptions

- **CI runner performance:** Ubuntu-latest GitHub Actions runners can sustain >1000 ops/sec fire-and-wait with 200 connections. The baseline thresholds are intentionally conservative to account for CI runner variability. If CI proves too noisy, thresholds can be lowered via `baseline.json` without code changes.
- **jq available on CI:** `jq` is pre-installed on `ubuntu-latest` GitHub Actions runners (confirmed: it is in the default image).
- **15-second runs are sufficient:** Short runs may have higher variance, but the conservative thresholds (1000 ops/sec vs 200k peak) provide >100x margin.
- **Perf gate starts as optional:** The job runs and reports but is not a required check for merge. This avoids blocking PRs while baseline stability is validated.

## Goal Analysis

### Key Links

| From | To | Risk |
|------|-----|------|
| `main.rs` --json-output flag | JSON file on disk | Low -- straightforward serde_json::to_writer |
| `traits.rs` JsonReport struct | `main.rs` serialization | Low -- simple data struct |
| `rust.yml` perf-gate job | cargo bench binary | Medium -- CI runner performance variance |
| `rust.yml` jq validation | baseline.json thresholds | Low -- static file, simple jq expressions |
| CI artifacts | JSON result files | Low -- standard GitHub Actions upload |

## Implementation Tasks

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `JsonReport`/`JsonLatency`/`JsonAssertionResult` structs to `traits.rs` | -- | ~15% |
| G2 | 2 | Add `--json-output` CLI flag and JSON serialization to `main.rs` | G1 | ~25% |
| G3 | 2 | Create `baseline.json` with conservative thresholds | -- | ~5% |
| G4 | 3 | Add `perf-gate` job to `.github/workflows/rust.yml` with both modes, jq validation, artifact upload | G2, G3 | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-18 12:00)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Evaluation:**
- Clarity: Excellent. Tasks are specific with exact CLI flags, JSON schemas, and shell commands.
- Completeness: Good. All files listed, "Files NOT Modified" section prevents scope creep.
- Testability: Strong. All 10 acceptance criteria are concrete and verifiable.
- Scope: Well-bounded by explicit constraints.
- Feasibility: Sound. Builds on existing harness, conservative thresholds.
- Architecture fit: Aligns with existing CI workflow structure and Rust bench conventions.
- Non-duplication: Reuses existing harness, metrics snapshot API, and CI patterns.
- Cognitive load: Low. Straightforward serialization + CI plumbing.
- Strategic fit: Aligned with project goals -- prevents regression of hard-won perf improvements.
- Project compliance: Honors PROJECT.md decisions and constraints.
- Language profile: Compliant with Rust profile (5 files, trait-first G1 with types only).

**Rust Auditor Checklist:**
- No f64 for integer-semantic fields: OK (JSON report uses u64 for ops/latency)
- No r#type on message structs: OK (not message structs, bench-only data)
- Default derived on payload structs: N/A (report structs are fully populated on construction)
- serde_json already a regular dependency: See recommendation 1 below
- #[serde(rename_all = "camelCase")]: N/A -- JSON output uses snake_case intentionally (matching jq field names in CI script)

**Goal-Backward Validation:**
- All 6 observable truths have corresponding artifacts: OK
- All artifacts map to at least one truth: OK
- Key links identified with risk levels: OK
- No orphan artifacts: OK

**Strategic Sanity Check:**
- Strategic fit: Aligned with project goals. Directly prevents the class of regression that motivated SPEC-116 through SPEC-122.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | CI runners sustain >1000 ops/sec | Flaky CI -- mitigated by 100x margin |
| A2 | jq available on ubuntu-latest | CI step fails -- easy to add install step |
| A3 | 15s runs are sufficient | Higher variance -- mitigated by conservative thresholds |
| A4 | serde_json not already a dependency | See recommendation 1 |

**Project Compliance:** Honors PROJECT.md decisions. No violations or out-of-scope intrusions.

**Recommendations:**
1. `serde_json` is already a regular dependency in `packages/server-rust/Cargo.toml` (line 30: `serde_json = "1"`). The spec says to add it as a dev-dependency, but no Cargo.toml change is needed. Remove `Cargo.toml` from the "Files to Modify" table and update Task 1 text to note serde_json is already available. This also reduces the file count from 5 to 4.
2. The Goal Statement says "the PR is blocked" but the Constraints section says "Do NOT make the perf-gate job required for merge initially." Clarify the Goal Statement to say "the CI job fails" rather than "the PR is blocked" to avoid confusion.
3. G2 original estimate was ~30% (included Cargo.toml change). With serde_json already present, G2 drops to ~25% (main.rs only). Updated in Implementation Tasks above.
4. The `perf-gate` job should include Rust toolchain installation and cargo cache steps (same as `check` job) since it runs on a separate runner. The spec's Task 3 description lists build and run steps but does not explicitly mention toolchain/cache setup steps. The implementer should replicate the checkout + toolchain + cache steps from the `check` job.
5. [Compliance] Consider adding `continue-on-error: true` to the perf-gate job to make its "optional/informational" nature explicit in the workflow YAML, matching the constraint that it should not block merges initially.

**Comment:** Well-structured spec with clear separation between harness-level assertions and CI-level baseline checks. Conservative thresholds and the optional-first approach are pragmatic choices. The existing code is clean and the additions are straightforward.

---

### Response v1 (2026-03-18)
**Applied:** All 5 recommendations from Audit v1

**Changes:**
1. [✓] Remove `Cargo.toml` from "Files to Modify" — removed from the Files to Modify table; added a "Files NOT Modified" note for `Cargo.toml`; updated Task 1 text to state `serde_json` is already a regular dependency and no Cargo.toml change is needed; removed assumption that `serde_json` is not already a dependency from the Assumptions section.
2. [✓] Clarify Goal Statement — changed "the PR is blocked" to "the CI job fails" in the Goal Statement paragraph.
3. [✓] Verify G2 estimate — confirmed ~25% in the Implementation Tasks table (already correct per audit note).
4. [✓] Add explicit setup steps to Task 3 — added step 3 to the perf-gate job description listing checkout, Rust toolchain installation, and cargo cache as required setup steps matching the `check` job; also updated Validation Checklist item 5 to include setup steps.
5. [✓] Add `continue-on-error: true` to Task 3 — added `continue-on-error: true` to the `perf-gate` job description in step 2 with an explanatory note that the job is informational only and must not block merges.

---

### Audit v2 (2026-03-18 14:30)
**Status:** APPROVED

**Context Estimate:** ~42% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~42% | <=50% | OK |
| Largest task group | ~25% (G2) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Evaluation:**
- Clarity: Excellent. All tasks specify exact CLI flags, JSON schemas, shell commands, and workflow YAML structure. No vague terms.
- Completeness: Good. Files to create, modify, and NOT modify are all explicitly listed. The "Files NOT Modified" section with Cargo.toml note is clear.
- Testability: Strong. All 10 acceptance criteria are concrete and verifiable. Validation checklist provides exact commands.
- Scope: Well-bounded. 4 files total (1 new, 3 modified), within the Rust language profile limit of 5.
- Feasibility: Sound. Builds on existing harness with conservative thresholds providing 100x safety margin.
- Architecture fit: Aligns with existing CI workflow structure (same trigger filters, cache patterns). Bench binary conventions match Cargo.toml `[[bench]]` setup.
- Non-duplication: Reuses existing harness, MetricsSnapshot API, and CI workflow patterns. No reinvention.
- Cognitive load: Low. Straightforward serde serialization + CI plumbing. Clear separation between harness-level and CI-level assertions.
- Strategic fit: Directly prevents the class of regression that motivated SPEC-116 through SPEC-122. Aligned with project goals.
- Project compliance: Honors PROJECT.md decisions. No violations or out-of-scope intrusions.

**Rust Auditor Checklist:**
- No f64 for integer-semantic fields: OK (JSON report uses u64 for ops/latency counts)
- No r#type on message structs: OK (bench-only data structs, not message structs)
- Default derived on payload structs: N/A (report structs are fully populated on construction)
- serde_json available: OK (already a regular dependency, confirmed at Cargo.toml line 30)
- #[serde(rename_all = "camelCase")]: N/A -- JSON output uses snake_case intentionally (matching jq field names in CI script)

**Language Profile Check:**
- File count: 4 files (limit 5): OK
- Trait-first: G1 contains only type definitions (JsonReport, JsonLatency, JsonAssertionResult): OK
- Compilation gate: No group exceeds 3 files: OK

**Goal-Backward Validation:**
- All 6 observable truths have corresponding artifacts: OK
- All artifacts map to at least one truth: OK
- Key links identified with risk levels: OK
- No orphan artifacts: OK

**Strategic Sanity Check:**
- Strategic fit: Aligned with project goals.
- Project compliance: Honors PROJECT.md decisions.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | CI runners sustain >1000 ops/sec with 200 connections | Flaky CI -- mitigated by 100x margin from peak |
| A2 | jq pre-installed on ubuntu-latest | CI step fails -- easy to add install step |
| A3 | 15s runs produce stable enough results | Higher variance -- mitigated by conservative thresholds |

**Recommendations:**
1. The Constraints section contains a stale constraint: "Do NOT add `serde_json` as a non-dev dependency -- it is only needed by the bench binary." In reality, `serde_json` is already a regular (non-dev) dependency and no change is being made. This constraint is harmless (no action results from it) but reads as confusing since it implies serde_json might need to be added. Consider removing or rewording to "Do NOT add new Cargo dependencies."

**Comment:** All v1 recommendations were properly applied. The spec is clear, complete, and ready for implementation. The only remaining note is a cosmetic constraint inconsistency that does not affect implementability.

---

### Response v2 (2026-03-18)
**Applied:** Recommendation 1 from Audit v2

**Changes:**
1. [✓] Reword stale constraint — replaced "Do NOT add `serde_json` as a non-dev dependency -- it is only needed by the bench binary." with "Do NOT add new Cargo dependencies." in the Constraints section. The old wording was misleading because `serde_json` is already a regular dependency and no Cargo.toml change is being made; the new wording is accurate and still prevents scope creep.

---

### Audit v3 (2026-03-18 16:00)
**Status:** APPROVED

**Context Estimate:** ~42% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~42% | <=50% | OK |
| Largest task group | ~25% (G2) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Evaluation:**
- Clarity: Excellent. All four tasks specify exact CLI flags, JSON schemas, shell commands, and workflow YAML structure. No vague terms found.
- Completeness: Good. Files to create, modify, and NOT modify are all explicitly listed. The separation between harness-level assertions (Rust code) and CI-level assertions (jq shell script) is clearly documented in Task 4.
- Testability: Strong. All 10 acceptance criteria are concrete and verifiable. The validation checklist provides exact commands to run locally.
- Scope: Well-bounded. 4 files total (1 new, 3 modified), within the Rust language profile limit of 5. Constraints section clearly defines boundaries.
- Feasibility: Sound. Verified that both `serde` (with derive feature) and `serde_json` are regular dependencies in Cargo.toml. The `MetricsSnapshot` API in `metrics.rs` provides all needed latency data via `snapshot()`. The existing CLI arg parsing pattern in `main.rs` is straightforward to extend.
- Architecture fit: The `perf-gate` job replicates the same checkout/toolchain/cache pattern used by the existing `check` and `cross-lang` jobs. The bench binary convention matches the `[[bench]]` config in Cargo.toml (`harness = false`).
- Non-duplication: Reuses existing harness, MetricsSnapshot API, and CI workflow patterns. No reinvention.
- Cognitive load: Low. Straightforward serde serialization plus CI plumbing. The two-layer assertion design (harness-level vs CI-level) is clean and well-explained.
- Strategic fit: Directly prevents the class of regression that motivated SPEC-116 through SPEC-122. Proportional effort for the value delivered.
- Project compliance: Honors PROJECT.md decisions. No new dependencies added. No out-of-scope intrusions.

**Rust Auditor Checklist:**
- No f64 for integer-semantic fields: OK (JSON report uses u64 for ops/latency counts per the example schema)
- No r#type on message structs: OK (bench-only data structs, not wire-protocol message structs)
- Default derived on payload structs: N/A (report structs are fully populated on construction, not deserialized)
- serde/serde_json available: OK (both are regular dependencies -- serde with derive feature at Cargo.toml line 29, serde_json at line 30)
- #[serde(rename_all = "camelCase")]: N/A -- JSON output uses snake_case intentionally to match jq field access in CI script
- Wire compatibility: N/A -- not a wire protocol struct, bench-only JSON output
- Option skip_serializing_if: N/A -- `message` in JsonAssertionResult is the only nullable field; spec shows `"message": null` which is the serde default for `Option<String>` without skip annotation, consistent with jq parsing expectations

**Language Profile Check:**
- File count: 4 files (limit 5): OK
- Trait-first: G1 contains only type definitions (JsonReport, JsonLatency, JsonAssertionResult): OK
- Compilation gate: No group exceeds 3 files: OK

**Goal-Backward Validation:**
- Truth 1 (CI job runs on PRs) -> rust.yml perf-gate job: OK
- Truth 2 (machine-readable JSON) -> main.rs --json-output + traits.rs JsonReport: OK
- Truth 3 (baseline file) -> baseline.json: OK
- Truth 4 (CI fails on threshold miss) -> rust.yml jq validation step: OK
- Truth 5 (artifacts uploaded) -> rust.yml upload-artifact step: OK
- Truth 6 (two modes tested) -> rust.yml runs both fire-and-wait and fire-and-forget: OK
- All artifacts map to at least one truth: OK
- No orphan artifacts: OK
- Key links identified with appropriate risk levels: OK

**Strategic Sanity Check:**
- Strategic fit: Aligned with project goals.
- Project compliance: Honors PROJECT.md decisions.
- No simpler alternative ignored: The approach of conservative thresholds + optional job is the standard industry pattern for perf CI gates.
- Root cause addressed: Yes -- this prevents silent regression of the performance work done in SPEC-116 through SPEC-122.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | CI runners sustain >1000 ops/sec with 200 connections | Flaky CI -- mitigated by 100x margin from peak |
| A2 | jq pre-installed on ubuntu-latest | CI step fails -- easy to add install step |
| A3 | 15s runs produce stable enough results | Higher variance -- mitigated by conservative thresholds |
| A4 | No chrono/time crate needed for ISO 8601 timestamp | See recommendation 1 |

**Recommendations:**
1. The JSON schema includes a `timestamp` field in ISO 8601 format (`"2026-03-18T12:00:00Z"`), but neither `chrono` nor the `time` crate is available as a dependency (and the constraint prohibits adding new dependencies). The implementer will need to format the timestamp manually using `std::time::SystemTime` and `UNIX_EPOCH` arithmetic. This is feasible but awkward. Consider either: (a) documenting that a manual UTC formatter is expected, or (b) changing the `timestamp` field to a Unix epoch integer (e.g., `"timestamp_unix": 1710763200`) which is trivially generated with `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()`. Either approach works; the current spec just leaves the implementation detail ambiguous.
2. The `perf-gate` job step 4 builds with `cargo build --release --bench load_harness`, but step 5 runs with `cargo bench --bench load_harness`. The `cargo bench` command will rebuild if needed, making step 4 redundant. Consider removing the explicit build step to simplify the workflow and save CI time, or note that it serves as an early-fail check before the longer scenario runs.

**Comment:** The spec is well-refined after two revision cycles. All previous recommendations were properly addressed. The two new recommendations are minor implementation details that do not block execution. The spec is clear, complete, and ready for implementation.

---

### Response v3 (2026-03-18)
**Applied:** Both recommendations from Audit v3

**Changes:**
1. [✓] Document `timestamp` formatting with `std::time::SystemTime` — added a "Timestamp formatting note" paragraph at the end of Task 1, after the `serde_json` sentence. The note explains that neither `chrono` nor `time` is available, that the timestamp must be formatted using `std::time::SystemTime` and `UNIX_EPOCH` arithmetic, and that basic UTC formatting (`YYYY-MM-DDTHH:MM:SSZ`) is achievable with integer division on the Unix epoch value without any external crate. Option (a) from the audit recommendation was chosen — documenting the manual approach while keeping the ISO 8601 string format, which the CI jq script can consume without any changes.
2. [✓] Note early-fail purpose of explicit build step — expanded Task 3 step 4 from the bare command to include a parenthetical explanation: the explicit `cargo build --release --bench load_harness` step serves as an early-fail check so that a compilation failure surfaces immediately, before the longer 15-second scenario runs begin.

---

### Audit v4 (2026-03-18 17:30)
**Status:** APPROVED

**Context Estimate:** ~42% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~42% | <=50% | OK |
| Largest task group | ~25% (G2) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Evaluation:**
- Clarity: Excellent. All four tasks are fully specified with exact CLI flags, JSON schema, shell commands, workflow structure, and timestamp formatting guidance. No vague terms.
- Completeness: Complete. Files to create, modify, and NOT modify are explicit. The two-layer assertion design (harness-level Rust assertions vs CI-level jq assertions) is clearly separated in Task 4.
- Testability: All 10 acceptance criteria are concrete and independently verifiable. Validation checklist provides exact local commands.
- Scope: Well-bounded at 4 files (1 new, 3 modified), within the Rust language profile limit of 5. Six explicit constraints prevent scope creep.
- Feasibility: Verified against source code. `serde` (with derive) and `serde_json` are regular dependencies. `MetricsSnapshot` provides all needed latency percentiles via `snapshot()`. The CLI arg parsing pattern in `main.rs` (manual `args[i]` matching) is straightforward to extend with `--json-output`.
- Architecture fit: The `perf-gate` job mirrors the existing `check` and `cross-lang` jobs in checkout/toolchain/cache structure. The `[[bench]]` with `harness = false` convention is already established.
- Non-duplication: Builds on existing harness infrastructure. No reinvention.
- Cognitive load: Low. Data struct serialization plus CI YAML -- no complex logic, no state management.
- Strategic fit: Directly prevents the class of silent performance regression that motivated SPEC-116 through SPEC-122.
- Project compliance: Honors all PROJECT.md decisions. No new dependencies. No out-of-scope intrusions.

**Rust Auditor Checklist:**
- No f64 for integer-semantic fields: OK (ops_per_sec, latency percentiles, total_ops are all integer-semantic and should use u64)
- No r#type on message structs: N/A (bench-only data structs)
- Default derived on payload structs: N/A (report structs are fully populated on construction)
- serde/serde_json available: OK (regular dependencies at Cargo.toml lines 29-30)
- #[serde(rename_all = "camelCase")]: N/A -- snake_case intentional for jq field access in CI script
- Wire compatibility: N/A -- bench-only JSON output, not wire protocol
- Option skip_serializing_if: N/A -- `"message": null` output is correct for jq parsing

**Language Profile Check:**
- File count: 4 files (limit 5): OK
- Trait-first: G1 contains only type definitions: OK
- Compilation gate: No group exceeds 3 files: OK

**Goal-Backward Validation:**
- All 6 observable truths mapped to artifacts: OK
- All artifacts serve at least one truth: OK
- Key links with risk levels: OK
- No orphan artifacts: OK

**Strategic Sanity Check:**
- Strategic fit: Aligned with project goals.
- Project compliance: Honors PROJECT.md decisions.
- Alternatives: Conservative thresholds + optional job is standard industry practice. No simpler alternative ignored.

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | CI runners sustain >1000 ops/sec with 200 connections | Flaky CI -- mitigated by 100x margin from peak |
| A2 | jq pre-installed on ubuntu-latest | CI step fails -- easy to add install step |
| A3 | 15s runs produce stable enough results | Higher variance -- mitigated by conservative thresholds |

**Comment:** The spec has been refined through three revision cycles, with all previous recommendations addressed. All technical claims verified against source code. No critical issues, no remaining recommendations. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-18
**Mode:** orchestrated
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |
| 3 | G4 | complete |

### Files Created

- `packages/server-rust/benches/load_harness/baseline.json`

### Files Modified

- `packages/server-rust/benches/load_harness/traits.rs` — added JsonReport/JsonLatency/JsonAssertionResult structs with #[derive(Serialize)]
- `packages/server-rust/benches/load_harness/main.rs` — added --json-output CLI flag, JSON report serialization, utc_timestamp_now() helper
- `.github/workflows/rust.yml` — added perf-gate job with both modes, jq validation, artifact upload

### Acceptance Criteria Status

- [x] Running `cargo bench --bench load_harness -- --json-output /tmp/out.json` produces a valid JSON file
- [x] JSON contains scenario, mode, connections, duration_secs, total_ops, ops_per_sec, latency (p50_us/p95_us/p99_us/p999_us), assertions, timestamp
- [x] Without `--json-output`, behavior is unchanged (ASCII report only)
- [x] `baseline.json` exists with thresholds for both fire_and_wait and fire_and_forget modes
- [x] `.github/workflows/rust.yml` has perf-gate job with `needs: [check]`
- [x] perf-gate runs both fire-and-wait and fire-and-forget scenarios
- [x] perf-gate fails (exit 1) if ops/sec falls below baseline thresholds
- [x] JSON result files uploaded as GitHub Actions artifacts with 90-day retention
- [x] No new clippy warnings introduced (3 pre-existing cast errors in throughput.rs unchanged)
- [x] 540 existing tests continue to pass

### Deviations

None. All spec requirements implemented as specified.

---

## Review History

### Review v1 (2026-03-18)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `--json-output` flag implemented in `main.rs` lines 119-127; `json_output: Option<String>` captures the path
- [✓] AC2: `JsonReport` struct in `traits.rs` contains all required fields: `scenario`, `mode`, `connections`, `duration_secs`, `total_ops`, `ops_per_sec`, `latency` (with `p50_us`/`p95_us`/`p99_us`/`p999_us`), `assertions`, `timestamp`
- [✓] AC3: JSON writing is gated on `if let Some(path) = json_output` — no file written when flag is absent
- [✓] AC4: `baseline.json` exists with correct structure: `fire_and_wait`, `fire_and_forget`, and `regression_tolerance_pct` fields
- [✓] AC5: `perf-gate` job has `needs: [check]` at line 107
- [✓] AC6: Both fire-and-wait (lines 134-141) and fire-and-forget (lines 143-151) scenarios run with correct flags
- [✓] AC7: jq validation step exits with `exit $FAILED` where `FAILED=1` is set on any threshold miss; integer comparison operators (`-lt`, `-gt`) are correct for `u64` JSON values
- [✓] AC8: `upload-artifact@v4` step uploads `results-faw.json` and `results-faf.json` with `retention-days: 90` and `if: always()` to ensure upload even on failure
- [✓] AC9: No new clippy warnings in `main.rs` or `traits.rs`; 3 pre-existing cast errors in `throughput.rs` are unchanged
- [✓] AC10: 540 server tests pass with no regressions (confirmed via `cargo test --release -p topgun-server`)
- [✓] Build: `cargo build --release --bench load_harness` exits 0
- [✓] `baseline.json` is valid JSON (confirmed with `jq`)
- [✓] `serde_json` is a regular dependency — no Cargo.toml change was made
- [✓] `continue-on-error: true` is set on the `perf-gate` job (line 111)
- [✓] Setup steps (checkout, toolchain, cache) are replicated in `perf-gate` as required by spec Task 3 step 3
- [✓] Timestamp formatter `utc_timestamp_now()` correctly implements Gregorian calendar arithmetic using only `std::time::SystemTime` and integer division — no external crate
- [✓] `#[allow(clippy::struct_field_names)]` on `JsonLatency` is correct: the `_us` suffix is intentional and load-bearing for CI consumers
- [✓] `serde_json::to_writer_pretty` used for human-readable artifact output; errors are printed but do not cause panic (graceful degradation)
- [✓] Existing `check` and `cross-lang` jobs are unmodified
- [✓] Artifact name uses `github.run_id` for uniqueness — no collisions across concurrent runs
- [✓] No files that should have been deleted; no lingering references to removed code
- [✓] No `f64` for integer-semantic fields — all latency/ops fields use `u64`
- [✓] `Option<String>` in `JsonAssertionResult.message` intentionally serializes as `null` (not omitted) — correct for jq field access without `has()` guard

**Summary:** The implementation matches the specification exactly across all 10 acceptance criteria. Code quality is high: clear separation between harness-level and CI-level assertions, well-commented WHY rationale on non-obvious choices (the `_us` suffix comment, the `message: null` comment, the UTC formatter comment), and no new dependencies introduced. The 540-test suite passes cleanly.

---

## Completion

**Completed:** 2026-03-18
**Total Commits:** 4
**Review Cycles:** 1

### Outcome

Added a performance regression CI gate that runs the Rust load harness on every server-touching PR, with machine-readable JSON output, conservative baseline thresholds, and GitHub Actions artifact upload for trend tracking.

### Key Files

- `packages/server-rust/benches/load_harness/traits.rs` — JsonReport/JsonLatency/JsonAssertionResult structs for machine-readable output
- `packages/server-rust/benches/load_harness/main.rs` — --json-output CLI flag and UTC timestamp formatter
- `packages/server-rust/benches/load_harness/baseline.json` — conservative CI thresholds (1k ops/sec FAW, 50k FAF)
- `.github/workflows/rust.yml` — perf-gate job with jq validation and artifact upload

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
