---
id: SPEC-125
type: feature
status: done
priority: P2
complexity: medium
created: 2026-03-18
source: TODO-118
---

# Flamegraph Profiling and Data-Driven Optimization Plan

## Context

The Rust server achieved 200k ops/sec (fire-and-forget) and 2.8k ops/sec (fire-and-wait) after v1.0 performance work (SPEC-116 through SPEC-122). TODO-117 proposed 5 hypothetical optimizations, but all were eliminated as incorrect, already implemented, or non-bottlenecks. This proved that optimization without profiling data wastes effort.

This spec delivers the profiling tooling (Cargo profile, capture script, analysis document template) needed to identify the actual hot paths. The profiling execution and analysis are inherently manual and are documented as follow-up steps in the script and template; they are not automatable by a code agent.

## Task

1. Add a `[profile.release-with-debug]` Cargo profile that preserves debug symbols for flamegraph readability while retaining release-level optimizations.
2. Create a profiling runner script that captures flamegraphs for both fire-and-forget and fire-and-wait workloads using `cargo flamegraph` with the load harness bench target.
3. Create an analysis document template with the structure for identifying the top-5 hot functions by cumulative sample time, classifying each as CPU-bound, IO-bound, lock contention, or allocation overhead.
4. The analysis document template includes an optimization plan table and proposed TODOs section to be filled in by the human after running the script.

**Note:** Steps 1-3 are delivered by this spec (automatable). Step 4 (filling in the data) is a manual follow-up performed by the developer after running `flamegraph-capture.sh`.

## Goal Analysis

**Goal Statement:** Deliver the tooling required to identify where the Rust server actually spends time under load, so that future optimization work targets real bottlenecks, not hypothesized ones.

**Observable Truths:**
1. Running the profiling script produces two flamegraph SVGs (fire-and-forget, fire-and-wait) with readable function names
2. The analysis document template lists the top-5 hot functions table structure with placeholders for measured percentages
3. Each hot function row has a classification column (CPU / IO / lock / alloc)
4. An optimization plan table and proposed TODOs section exist as placeholders ready for measured data
5. The Cargo profile compiles with release optimizations + debug symbols in a single command

**Required Artifacts:**
- `Cargo.toml` (workspace) -- `[profile.release-with-debug]` section
- `packages/server-rust/scripts/flamegraph-capture.sh` -- profiling runner
- `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md` -- analysis template (placeholders; filled manually after profiling runs)

**Manual Follow-up Artifacts (produced by developer, not by /sf:run):**
- `packages/server-rust/docs/profiling/flamegraph-fire-and-forget.svg` -- captured SVG
- `packages/server-rust/docs/profiling/flamegraph-fire-and-wait.svg` -- captured SVG

## Requirements

### 1. Cargo Profile: `release-with-debug` (Cargo.toml, workspace root)

Add to workspace `Cargo.toml`:

```toml
[profile.release-with-debug]
inherits = "release"
debug = true
```

This produces optimized binaries with DWARF debug info so flamegraph stack traces show function names instead of addresses.

### 2. Profiling Script: `packages/server-rust/scripts/flamegraph-capture.sh`

Executable shell script that:

- Prints a prerequisite check: exits with an error message if `cargo flamegraph` is not installed, with installation instructions (`cargo install flamegraph --version 0.6.0` or later)
- Documents that `sudo` is required on macOS for dtrace (the script must be run as root or via `sudo`)
- Sets `SDKROOT` for macOS compatibility: `export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)`
- Creates `packages/server-rust/docs/profiling/` if it does not exist
- Runs `cargo flamegraph` for fire-and-forget mode using the exact invocation:
  ```
  cargo flamegraph --bench load_harness --profile release-with-debug \
    -o packages/server-rust/docs/profiling/flamegraph-fire-and-forget.svg \
    -- --connections 200 --interval 0 --duration 60 --fire-and-forget
  ```
- Runs `cargo flamegraph` for fire-and-wait mode using the exact invocation:
  ```
  cargo flamegraph --bench load_harness --profile release-with-debug \
    -o packages/server-rust/docs/profiling/flamegraph-fire-and-wait.svg \
    -- --connections 200 --interval 0 --duration 60
  ```
- After each `cargo flamegraph` run, post-processes the collapsed stacks to produce a server-only view filtered to `server-rt` threads:
  - On macOS, `cargo flamegraph` produces `flamegraph.stacks` in the current working directory. Check for this filename first; if absent, fall back to globbing for `*.stacks`.
  - Run: `grep 'server-rt' <stacks-file> | inferno-flamegraph > <output>-server-only.svg`
  - If `inferno-flamegraph` is not installed, print a warning and skip this step (the primary SVG is still produced)
- Prints completion paths for both SVGs

**Minimum required tool version:** `cargo-flamegraph` v0.6.0 or later (supports `--profile` flag and bench targets).

**Note on `--bench` vs `--bin`:** The load harness is declared as `[[bench]] name = "load_harness"` in `Cargo.toml`. The correct `cargo flamegraph` flag is `--bench load_harness`, not `--bin`. Using `--bin` will produce a "binary not found" error.

### 3. Analysis Document: `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md`

Structure (delivered as template with placeholders; filled in manually after profiling runs):

```
# Flamegraph Analysis

## Environment
- Date: [FILL IN]
- Hardware: [FILL IN]
- OS: [FILL IN]
- Rust version: [FILL IN]
- Commit hash: [FILL IN]

## Baselines
- Fire-and-forget: [measured] ops/sec
- Fire-and-wait: [measured] ops/sec

## Fire-and-Forget Hot Path Analysis

Use `flamegraph-fire-and-forget-server-only.svg` (filtered to `server-rt` threads) to identify
server-side hot paths separately from client-side load harness overhead.

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 2    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 3    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 4    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 5    | [name]   | [X]%        | CPU/IO/lock/alloc |

### Observations
[Analysis of what the hot path reveals]

## Fire-and-Wait Hot Path Analysis

Use `flamegraph-fire-and-wait-server-only.svg` (filtered to `server-rt` threads).

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 2    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 3    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 4    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 5    | [name]   | [X]%        | CPU/IO/lock/alloc |

### Observations
[Analysis of what the hot path reveals]

## Optimization Plan
| Priority | Target Function | Current % | Expected Improvement | Approach |
|----------|----------------|-----------|---------------------|----------|
| 1        | [name]         | [X]%      | [estimate]          | [brief]  |
| 2        | [name]         | [X]%      | [estimate]          | [brief]  |

## Proposed TODOs
- TODO-NNN: [optimization based on finding 1]
- TODO-NNN: [optimization based on finding 2]
```

The placeholders (`[FILL IN]`, `[name]`, `[X]%`) are intentional: the developer fills them in after running `flamegraph-capture.sh`.

### 4. `.gitattributes` Entry for SVG Files

Add to the repository `.gitattributes` (or create if absent):

```
packages/server-rust/docs/profiling/*.svg binary
```

This prevents git from treating large SVGs as text diffs and makes the profiling directory safe to accumulate runs over time.

## Acceptance Criteria

1. `cargo build --profile release-with-debug -p topgun-server` compiles without error and produces a binary with debug symbols (verified by `file` or `dsymutil` showing DWARF info)
2. `flamegraph-capture.sh` is executable, passes a prerequisite check for `cargo flamegraph` v0.6.0+, and prints correct usage if the tool is missing
3. The script uses `--bench load_harness` (not `--bin`) and includes `--profile release-with-debug` in both `cargo flamegraph` invocations
4. The script includes a post-processing step that checks for `flamegraph.stacks` (falling back to `*.stacks` glob), filters collapsed stacks to `server-rt` threads, and pipes to `inferno-flamegraph` for a server-only SVG
5. `FLAMEGRAPH_ANALYSIS.md` exists with the template structure: Environment, Baselines, two Hot Path Analysis tables (5 rows each), Observations sections, Optimization Plan table, and Proposed TODOs section
6. `.gitattributes` contains `packages/server-rust/docs/profiling/*.svg binary`
7. No code changes to the server or load harness beyond the Cargo profile addition and `.gitattributes` entry

## Validation Checklist

1. Run `cargo build --profile release-with-debug -p topgun-server` -- compiles successfully
2. Run `file target/release-with-debug/deps/load_harness-*` -- shows debug info present (note: bench targets are placed at `target/release-with-debug/deps/load_harness-<hash>`, not at a top-level path)
3. Review `flamegraph-capture.sh` -- contains `--bench load_harness`, `--profile release-with-debug`, `flamegraph.stacks` filename lookup, and `grep 'server-rt'` post-processing step
4. Review `FLAMEGRAPH_ANALYSIS.md` -- contains placeholder tables for both workloads and optimization plan section

**Manual validation (after running the script):**
5. Run `sudo ./packages/server-rust/scripts/flamegraph-capture.sh` -- produces SVG files in `docs/profiling/`
6. Open each SVG in browser -- function names like `topgun_server::service::*` are visible, not hex

## Constraints

- Do NOT modify any server source code (this spec is analysis tooling only; optimizations come from follow-up TODOs)
- Do NOT modify the load harness behavior (only the Cargo profile and `.gitattributes` are new artifacts)
- Do NOT propose optimizations in this spec without measured data backing them (the template has placeholders, not invented data)
- Flamegraph capture requires `sudo` on macOS for dtrace -- the script must document this prominently
- The analysis document template must include instructions to use the `server-rt`-filtered SVGs for server-side analysis, not the merged all-threads SVG

## Assumptions

- `cargo-flamegraph` v0.6.0+ will be installed via `cargo install flamegraph` (not vendored)
- macOS dtrace backend is used (no Linux perf_events support needed for this run)
- 60-second duration per capture provides sufficient sample depth for meaningful percentages
- `inferno-flamegraph` may not be installed; the server-only filtering step is best-effort with a graceful warning on failure
- SVG files may be large (1-5 MB) but are acceptable to commit for reference, mitigated by the `.gitattributes` binary marker
- The existing load harness CLI (`--connections`, `--interval`, `--fire-and-forget`, `--duration`) is sufficient; no new flags needed

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `[profile.release-with-debug]` to workspace Cargo.toml and `.gitattributes` entry | -- | ~5% |
| G2 | 1 | Create `flamegraph-capture.sh` profiling script | -- | ~8% |
| G3 | 1 | Create `docs/profiling/` directory structure and `FLAMEGRAPH_ANALYSIS.md` template | -- | ~8% |

**Manual follow-up (not executed by /sf:run):**
- After G1-G3 are complete, the developer runs `sudo ./packages/server-rust/scripts/flamegraph-capture.sh`, inspects the SVGs, and fills in `FLAMEGRAPH_ANALYSIS.md`.

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |

**Total workers needed:** 3 (max in any wave)

## Audit History

### Audit v1 (2026-03-18)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (but misleading -- see critical issues)

**Critical:**

1. **Load harness is a bench target, not a bin.** The spec says "targeting the load harness binary" but `Cargo.toml` declares it as `[[bench]] name = "load_harness"`. The script must use `cargo flamegraph --bench load_harness`, not `--bin`. Additionally, `cargo flamegraph` with bench targets uses `cargo bench` under the hood, which may conflict with the `--profile release-with-debug` flag (benches default to release). The spec must clarify the exact `cargo flamegraph` invocation, e.g., `cargo flamegraph --bench load_harness --profile release-with-debug -- --connections 200 ...`.

2. **Thread filtering is unspecified.** The constraint says "distinguish between server-thread and client-thread samples (filter by thread name `server-rt`)" but provides no mechanism. `cargo flamegraph` produces a single SVG with all threads merged. Filtering by thread name requires either: (a) post-processing the collapsed stacks with `grep server-rt` before feeding to `inferno-flamegraph`, or (b) using `dtrace` directly with a thread-name predicate, or (c) using `--flamechart` mode and manually inspecting. The script must specify which approach is used, or the constraint is unimplementable.

3. **G4 and G5 are not automatable by a code agent.** These groups require running `sudo cargo flamegraph` interactively, waiting 60+ seconds per run, inspecting SVG output visually, and writing analytical prose based on observed data. A SpecFlow `/sf:run` executor cannot perform these tasks. The spec should be scoped to G1-G3 only (automatable: Cargo profile, script, document template), with G4-G5 documented as manual follow-up steps.

4. **G5 depends on G4 but both are listed as Wave 3.** If G5 depends on G4's output data, they cannot be in the same wave. G5 should be Wave 4. The execution plan shows "G4, G5" in Wave 3 with "Parallel? No" but the wave computation algorithm places groups with the same dependency set in the same wave. Since G5 depends on G4, it must be wave = max(G4 wave) + 1 = 4.

**Recommendations:**

5. **[Strategic] Scope the spec to deliverable artifacts only.** The automatable deliverables are: Cargo profile (2 lines of TOML), shell script, and analysis document template. The profiling execution and analysis are inherently manual. Consider splitting: SPEC-125 delivers the tooling (G1-G3), and the human runs the tooling and fills in the analysis document manually (or in a separate spec that documents the results).

6. **Clarify `cargo flamegraph` invocation syntax.** The `cargo flamegraph` CLI syntax differs between versions. For cargo-flamegraph v0.6+, the pattern is: `cargo flamegraph --bench load_harness --profile release-with-debug -o output.svg -- <bench-args>`. Document the minimum required version.

7. **SVG size in git.** 1-5 MB SVGs in git are fine for a one-time commit, but consider adding to `.gitattributes` as binary/LFS if the profiling directory will accumulate SVGs over time.

8. **Validation checklist item 2 path.** `file target/release-with-debug/load_harness` -- bench targets may be placed at a different path than bin targets. The actual path for a bench built with `--profile release-with-debug` would be `target/release-with-debug/deps/load_harness-*` (with a hash suffix). Verify and correct.

### Response v1 (2026-03-18)
**Applied:** all critical issues and all recommendations

**Changes:**
1. [✓] Load harness is a bench target, not a bin — Updated all references from "targeting the load harness binary" to use `--bench load_harness` in the exact `cargo flamegraph` invocations shown in Requirements §2. Added a note explaining why `--bin` is incorrect and what error it produces.
2. [✓] Thread filtering is unspecified — Added a concrete post-processing step to the script specification: after each `cargo flamegraph` run, grep collapsed stacks for `server-rt` and pipe to `inferno-flamegraph` to produce a `-server-only.svg`. Added graceful warning if `inferno-flamegraph` is absent. Updated analysis document template instructions to reference these server-only SVGs.
3. [✓] G4 and G5 are not automatable — Scoped the spec to G1-G3 only. Removed G4 and G5 from the Task Groups table and Execution Plan. Added clear "Manual follow-up" note in Task Groups, in the Task section, and in the Validation Checklist. Updated Context, Goal Analysis, and Required Artifacts to distinguish between deliverable artifacts (G1-G3) and manual follow-up artifacts (SVGs).
4. [✓] G5 depends on G4 but both are Wave 3 — Resolved by removal of G4/G5 from the automatable wave plan; manual follow-up is not wave-scheduled.
5. [✓] Scope the spec to deliverable artifacts only — Applied as part of critical item 3. The spec now explicitly frames itself as tooling delivery, with profiling execution and analysis as human-performed follow-up.
6. [✓] Clarify `cargo flamegraph` invocation syntax — Added minimum required version (v0.6.0) to Requirements §2 and to Assumptions. The exact invocation pattern with `--bench`, `--profile`, `-o`, and `--` separator is now shown verbatim in the script specification.
7. [✓] SVG size in git — Added Requirements §4 specifying a `.gitattributes` entry `packages/server-rust/docs/profiling/*.svg binary`. Added corresponding Acceptance Criterion 6.
8. [✓] Validation checklist item 2 path — Corrected to `target/release-with-debug/deps/load_harness-*` with an explanatory note about bench target placement.

### Audit v2 (2026-03-18)
**Status:** APPROVED

**Context Estimate:** ~21% total

**Dimension Evaluation:**
- Clarity: Clear. Title, context, task, and requirements all well-defined. No vague terms.
- Completeness: All 4 artifacts specified with exact content. Manual vs automated scope clearly separated.
- Testability: All 7 acceptance criteria are concrete and verifiable.
- Scope: Well-bounded by constraints. No scope creep.
- Feasibility: Verified against codebase -- bench target exists as `[[bench]] name = "load_harness"`, CLI flags (`--connections`, `--interval`, `--duration`, `--fire-and-forget`) confirmed in source, `server-rt` thread name confirmed at line 143 of `main.rs`.
- Architecture fit: Tooling/docs only, no architectural impact.
- Non-duplication: No existing profiling tooling in the repository.
- Cognitive load: Low -- 3 simple file creations plus 2 lines of TOML.
- Strategic fit: Aligned with project goals. Addresses the proven need for data-driven optimization after TODO-117 showed hypothetical optimizations were wasteful.
- Project compliance: Honors PROJECT.md decisions. No new runtime dependencies, no server code changes, no constraint violations.

**Language Profile:** Compliant with Rust profile. 4 files total (under 5-file limit). Trait-first not applicable (no traits/types being created -- this is tooling/docs).

**Goal Analysis Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (SVGs with readable names) has artifacts | OK | Cargo profile + script |
| Truth 2 (template tables) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 3 (classification column) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 4 (optimization plan placeholders) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 5 (Cargo profile compiles) has artifacts | OK | Cargo.toml |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | `cargo-flamegraph` v0.6.0+ supports `--profile` and `--bench` together | Script invocations would fail; user would need to adjust flags |
| A2 | macOS dtrace backend produces collapsed stacks file findable by the script | Post-processing step would fail (graceful: primary SVG still produced) |
| A3 | 60-second duration provides sufficient sample depth | Flamegraphs may be sparse; user can re-run with longer duration |
| A4 | `inferno-flamegraph` may not be installed | Handled: graceful warning and skip |

All assumptions have low-to-medium impact and are mitigated by graceful fallbacks or manual adjustment.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~21% | <=50% | OK |
| Largest task group | ~8% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- Current estimate |
| 30-50% | GOOD | |
| 50-70% | DEGRADING | |
| 70%+ | POOR | |

**Recommendations:**

1. **Context estimates in Task Groups are inflated.** G2 was listed at ~40% and G3 at ~50% for creating a single shell script and a single markdown template respectively. Corrected to ~8% each in the approved spec. These are simple file-creation tasks with no complex logic or source code reading required.

2. **Collapsed stacks file location.** The post-processing step says "Locate the `perf.data` or dtrace `.stacks` file" but on macOS, `cargo flamegraph` typically produces `flamegraph.stacks` in the current working directory. The implementer should check for this specific filename and fall back to globbing for `*.stacks` if not found.

3. **G2 and G3 do not strictly depend on G1.** The shell script and markdown template can be created independently of the Cargo profile and `.gitattributes`. Declaring them as Wave 2 is harmless but unnecessary -- all three groups could run in Wave 1. This is cosmetic and does not affect correctness.

**Comment:** All 4 critical issues from Audit v1 were properly addressed. The spec is now well-scoped, technically accurate, and ready for implementation. The separation of automatable tooling delivery from manual profiling execution is clean and appropriate.

### Response v2 (2026-03-18)
**Applied:** all 3 recommendations from Audit v2

**Changes:**
1. [✓] Context estimates corrected — G2 and G3 already showed ~8% each in the post-v1 spec; no change needed to the values themselves. Confirmed correct.
2. [✓] Collapsed stacks file location — Updated Requirements §2 post-processing step to specify that on macOS `cargo flamegraph` produces `flamegraph.stacks` in the CWD; the script checks for this filename first and falls back to globbing `*.stacks` if absent. Updated Acceptance Criterion 4 and Validation Checklist item 3 to reference `flamegraph.stacks` lookup.
3. [✓] G2 and G3 don't strictly depend on G1 — Removed G1 as a dependency for G2 and G3 in the Task Groups table (both now show `--`). Updated Wave from 2 to 1 for both groups. Updated the Execution Plan to show all three groups running in Wave 1 in parallel with 3 workers.

### Audit v3 (2026-03-18)
**Status:** APPROVED

**Context Estimate:** ~21% total

**Dimension Evaluation:**
- Clarity: Excellent. All deliverables specified with exact content and invocation syntax.
- Completeness: 4 artifacts fully specified. Manual vs automated scope clearly delineated.
- Testability: All 7 acceptance criteria are concrete and verifiable without ambiguity.
- Scope: Well-bounded. Constraints explicitly prohibit server/harness code changes.
- Feasibility: Verified against codebase -- `[[bench]] name = "load_harness"` confirmed in `packages/server-rust/Cargo.toml:59-62`, CLI flags (`--connections`, `--duration`, `--interval`, `--fire-and-forget`) confirmed in `benches/load_harness/main.rs:61-116`, `server-rt` thread name confirmed at `main.rs:143`.
- Architecture fit: Tooling/docs only, no architectural impact.
- Non-duplication: No existing profiling tooling, scripts directory, or `.gitattributes` in repository.
- Cognitive load: Low -- 3 independent file creations plus 2 lines of TOML.
- Strategic fit: Aligned with project goals. Data-driven profiling directly addresses the lesson from TODO-117.
- Project compliance: No new runtime dependencies, no server code changes, no constraint violations.

**Language Profile:** Compliant with Rust profile. 4 files total (under 5-file limit). Trait-first not applicable (tooling/docs, no types created).

**Goal Analysis Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (SVGs with readable names) has artifacts | OK | Cargo profile + script |
| Truth 2 (template tables) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 3 (classification column) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 4 (optimization plan placeholders) has artifacts | OK | FLAMEGRAPH_ANALYSIS.md |
| Truth 5 (Cargo profile compiles) has artifacts | OK | Cargo.toml |

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| No new runtime dependencies | Tooling only, no runtime deps | OK |
| No server code changes | Explicitly constrained | OK |
| Rust profile (max 5 files) | 4 files | OK |
| MsgPack wire protocol | Not applicable (tooling) | OK |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | `cargo-flamegraph` v0.6.0+ supports `--profile` and `--bench` together | Script fails; user adjusts flags manually |
| A2 | macOS dtrace produces findable collapsed stacks file | Post-processing fails gracefully; primary SVG still produced |
| A3 | 60-second duration provides sufficient samples | Sparse flamegraphs; user re-runs with longer duration |
| A4 | `inferno-flamegraph` may not be installed | Handled with graceful warning and skip |

All assumptions low-to-medium impact with graceful fallbacks.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~21% | <=50% | OK |
| Largest task group | ~8% | <=30% | OK |
| Worker overhead | ~15% (3 workers x ~5%) | <=15% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- Current estimate |
| 30-50% | GOOD | |
| 50-70% | DEGRADING | |
| 70%+ | POOR | |

**Comment:** All prior audit issues have been thoroughly addressed across two revision cycles. The spec is clean, technically accurate, well-scoped, and ready for implementation. Codebase verification confirms all referenced targets, CLI flags, and thread names exist as specified.

## Execution Summary

**Executed:** 2026-03-18 18:52 UTC
**Mode:** orchestrated
**Commits:** 1 (ee0559a)

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2, G3 | complete |

### Files Created

- `.gitattributes` -- marks profiling SVGs as binary to prevent text diffs
- `packages/server-rust/scripts/flamegraph-capture.sh` -- flamegraph capture script (executable)
- `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md` -- analysis template

### Files Modified

- `Cargo.toml` (workspace) -- added `[profile.release-with-debug]` section

### Acceptance Criteria Status

1. [x] `cargo build --profile release-with-debug -p topgun-server` compiles without error (verified: `Finished release-with-debug profile [optimized + debuginfo]`)
2. [x] `flamegraph-capture.sh` is executable and performs prerequisite check for `cargo flamegraph` v0.6.0+
3. [x] Script uses `--bench load_harness` (not `--bin`) with `--profile release-with-debug` in both invocations
4. [x] Script checks for `flamegraph.stacks` first, falls back to `*.stacks` glob, filters with `grep server-rt`, pipes to `inferno-flamegraph` with graceful warning if absent
5. [x] `FLAMEGRAPH_ANALYSIS.md` has Environment, Baselines, two 5-row Hot Path Analysis tables with Observations, Optimization Plan table, and Proposed TODOs section
6. [x] `.gitattributes` contains `packages/server-rust/docs/profiling/*.svg binary`
7. [x] No server source code or load harness behavior modified

### Deviations

None.

---

## Review History

### Review v1 (2026-03-18)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `cargo build --profile release-with-debug -p topgun-server` compiles successfully — confirmed `Finished release-with-debug profile [optimized + debuginfo]`
- [✓] AC2: `flamegraph-capture.sh` is executable (`-rwxr-xr-x`) and exits with error + installation instructions if `cargo flamegraph` is absent
- [✓] AC3: Script uses `--bench load_harness` (not `--bin`) with `--profile release-with-debug` in both `cargo flamegraph` invocations
- [✓] AC4: Script checks for `flamegraph.stacks` first, falls back to `*.stacks` glob, filters with `grep 'server-rt'`, pipes to `inferno-flamegraph`, and gracefully warns and skips if `inferno-flamegraph` is not installed
- [✓] AC5: `FLAMEGRAPH_ANALYSIS.md` contains Environment, Baselines, two 5-row Hot Path Analysis tables with Category columns, two Observations sections, Optimization Plan table, and Proposed TODOs section — all as specified placeholders
- [✓] AC6: `.gitattributes` contains exactly `packages/server-rust/docs/profiling/*.svg binary`
- [✓] AC7: Commit diff shows only 4 files modified — `Cargo.toml`, `.gitattributes`, `flamegraph-capture.sh`, `FLAMEGRAPH_ANALYSIS.md` — no server source or load harness code touched
- [✓] Cargo.toml profile matches spec exactly: `inherits = "release"`, `debug = true`
- [✓] Script uses `set -euo pipefail` — robust error handling for a shell script
- [✓] SDKROOT set via `xcrun --sdk macosx --show-sdk-path` — correct macOS linker compatibility fix
- [✓] `sudo` requirement documented prominently in script header comment and in inline echo during capture
- [✓] `filter_server_only` function uses `${primary_svg%.svg}-server-only.svg` naming — correct derivation from primary output path
- [✓] Script prints completion paths for both primary and server-only SVGs
- [✓] No spec/bug references in code comments — all comments are WHY-style (compliant with CLAUDE.md)
- [✓] Language Profile compliance: 4 files total, under the 5-file limit; trait-first not applicable (tooling/docs)

**Summary:** All 7 acceptance criteria pass. The implementation is a clean, minimal delivery of the profiling tooling. The shell script is robust (`set -euo pipefail`, graceful fallbacks for missing tools), the Cargo profile matches the spec exactly, the analysis template matches the specified structure, and the `.gitattributes` entry is correct. No issues found.

---

## Completion

**Completed:** 2026-03-18
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Delivered flamegraph profiling tooling: a `release-with-debug` Cargo profile, a capture script for fire-and-forget and fire-and-wait workloads, and an analysis document template for recording hot path findings and optimization plans.

### Key Files

- `packages/server-rust/scripts/flamegraph-capture.sh` — captures flamegraphs with server-thread filtering
- `packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md` — template for recording profiling results
- `.gitattributes` — marks profiling SVGs as binary

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
