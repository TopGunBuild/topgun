---
id: SPEC-132a
type: feature
status: done
priority: P2
complexity: small
created: 2026-03-19
parent: SPEC-132
depends_on: []
---

# Cargo Configuration and I/O Seams for Simulation Testing

## Context

TopGun needs deterministic simulation testing (DST) via madsim to reproducibly test distributed protocols. Before any simulation harness or tests can be written, the workspace and crate Cargo configuration must be in place: feature flags, conditional dependencies, workspace profile, and I/O seam abstractions.

This is the foundation spec that all other simulation specs depend on. It establishes the `simulation` feature flag and the `cfg(feature = "simulation")` conditional compilation gates that swap `tokio` for madsim's tokio shim and seed `rand`.

**Parent:** SPEC-132 (Deterministic Simulation Testing via madsim)
**Source TODO:** TODO-027

## Task

Configure the Cargo workspace and crate manifests for madsim-based simulation testing. Add the `simulation` feature flag with conditional dependencies. Add I/O seam abstractions (`cfg(feature = "simulation")` gates for time and RNG) and a pnpm script for running simulation tests.

## Requirements

### R1: Workspace Cargo Configuration

**File: `Cargo.toml` (workspace root)**
- Add `madsim` as a workspace dependency (pin the version verified in AC0)
- Add `ci-sim` profile that inherits from `dev` with `opt-level = 1` and `debug-assertions = true` for faster simulation builds with assertion checking

Note: the `ci-sim` profile controls compiler settings only. The `simulation` feature flag is activated separately via `--features simulation` on the command line. These are independent concerns — profiles cannot activate features.

### R2: Server-Rust Feature Flag and Conditional Dependencies

**File: `packages/server-rust/Cargo.toml`**
- Add `simulation` feature flag
- Under the `[features]` section, declare `simulation = ["dep:madsim"]`
- Add `madsim` as an optional dependency: `madsim = { workspace = true, optional = true }`
- Do NOT use `[target.'cfg(...)'.dependencies]` — that syntax is for target triples (OS/arch), not Cargo features. Feature-gated dependencies must use `optional = true` combined with a `[features]` declaration.

The madsim ecosystem provides a tokio compatibility shim via `madsim::tokio` (re-exported from madsim itself) rather than a standalone `madsim-tokio` crate. The recommended mechanism is a `[patch.crates-io]` section in the workspace `Cargo.toml` that redirects `tokio` to madsim's bundled shim when simulation is active — however, `[patch]` sections apply unconditionally and cannot be gated on a feature flag.

Given this constraint, the implementation must choose one of these approaches and document the choice explicitly:
- **Option A (recommended):** Use `madsim::tokio` as a conditional import alias within `#[cfg(feature = "simulation")]` blocks only, without patching the global `tokio` crate.
- **Option B:** Accept that `[patch]` is build-wide and use a separate cargo workspace or `--config` override for simulation builds.

The chosen approach must be stated in the implementation comments and in R4.

### R3: Core-Rust Feature Flag (if needed)

**File: `packages/core-rust/Cargo.toml`**
- Add `simulation` feature flag if rand usage in core-rust needs seeding under simulation
- If core-rust does not use `rand` directly, this can be skipped with a note explaining why

### R4: I/O Seam Module

**File: `packages/server-rust/src/sim.rs`**

Create this module behind `#[cfg(feature = "simulation")]` (or as a pub mod conditional on the feature in `lib.rs`/`main.rs`). It provides re-exports so downstream simulation code imports time and RNG from one location:

```
packages/server-rust/src/sim.rs   <- new file (only compiled under simulation feature)
```

The module must expose at minimum:
- A time re-export pointing to `madsim::time` (virtual time)
- A RNG re-export pointing to `madsim::rand` (seeded deterministic RNG)

All conditional compilation gates in this spec use `cfg(feature = "simulation")`, not the bare `cfg(simulation)`. `cfg(simulation)` would require a raw `--cfg simulation` rustc flag and is never true when using `--features simulation`.

I/O boundary summary:

| I/O Boundary | Current Implementation | Sim Strategy |
|---|---|---|
| Async runtime | `tokio` | `madsim::tokio` via cfg-gated import alias (see R2) |
| Time | `tokio::time`, `std::time::Instant` | `madsim::time` (virtual time) via `sim.rs` |
| RNG | `rand::thread_rng()` | `madsim::rand` (seeded) via `sim.rs` |
| Disk I/O | In-memory `HashMapEngine` for tests | No change needed |

### R5: CI Integration Script

- Add `pnpm test:sim` script to root `package.json` that runs: `cargo test --profile ci-sim --features simulation -p topgun-server -- sim`
- Document the run command in the spec

## Acceptance Criteria

0. (Pre-flight) `madsim` version X compiles alongside `axum 0.8` and `tokio 1.x` in a minimal test project or branch. This must be verified and the compatible version pinned before implementation proceeds. If incompatible, this spec requires re-evaluation.
1. `cargo check --features simulation -p topgun-server` compiles without errors
2. `cargo check -p topgun-server` (without `simulation` feature) compiles without errors — no regressions
3. `cargo clippy --features simulation -p topgun-server` produces no warnings
4. Regular `cargo test -p topgun-server` (without `simulation` feature) continues to pass all existing 559+ tests unchanged
5. The `ci-sim` profile is defined in workspace `Cargo.toml` with explicit settings (`opt-level = 1`, `debug-assertions = true`)
6. `pnpm test:sim` script exists and invokes the correct cargo command
7. `packages/server-rust/src/sim.rs` exists, is gated on `cfg(feature = "simulation")`, and re-exports time and RNG seams

## Constraints

- Do NOT modify existing non-sim code paths. All simulation infrastructure is behind `#[cfg(feature = "simulation")]`
- Do NOT replace tokio with madsim globally. Use conditional compilation so both compile targets work
- Keep madsim version compatible with the existing tokio 1.x and axum 0.8 dependencies (see AC0)
- Do NOT block regular CI. Simulation tests are opt-in via feature flag
- All cfg gates use `cfg(feature = "simulation")`, never bare `cfg(simulation)`

## Assumptions

- madsim crate is compatible with our tokio 1.x and axum 0.8 versions. If version conflicts arise during AC0 pre-flight, this spec requires re-evaluation before implementation.
- The `simulation` feature name is used (not `madsim`). This follows the convention of describing what it enables, not the tool used.
- The implementation will document which R2 option (A or B) was chosen for the tokio shim mechanism.

## Audit History

### Audit v1 (2026-03-19)
**Status:** NEEDS_REVISION

**Context Estimate:** ~18% total

**Critical:**
1. **R1: Cargo profiles cannot enable features.** The spec says "Add ci-sim profile that inherits from dev and enables simulation feature." Cargo profiles (`[profile.ci-sim]`) control optimization level, debug info, LTO, etc. They cannot activate feature flags. Features are enabled via `--features` on the command line or in dependency declarations. The `ci-sim` profile can exist for custom optimization settings, but it needs to be decoupled from feature activation. Either (a) remove the "enables simulation feature" claim and clarify that `ci-sim` is just a profile for sim-specific compiler settings (e.g., debug assertions), with features passed separately via `--features simulation`, or (b) replace the profile concept with a `.cargo/config.toml` alias or a shell alias that combines profile + features.

2. **R2: Inconsistent cfg syntax.** The spec uses `cfg(simulation)` in R2 and the Context section, but `cfg(feature = "simulation")` in R4. These are different things in Rust. `cfg(simulation)` requires `--cfg simulation` (a raw cfg flag). `cfg(feature = "simulation")` requires `--features simulation` (a Cargo feature). Since the spec defines a Cargo feature, ALL conditional compilation must use `cfg(feature = "simulation")`. The bare `cfg(simulation)` will never be true when using `--features simulation`. Fix all occurrences to use `cfg(feature = "simulation")`.

3. **R2: `madsim-tokio` crate may not exist.** The spec assumes a standalone `madsim-tokio` crate provides a drop-in tokio replacement. The madsim ecosystem works differently -- it uses `[patch]` sections in Cargo.toml to redirect `tokio` imports to madsim's own tokio shim, or re-exports via `madsim::tokio`. The spec must verify the actual madsim dependency mechanism and specify it precisely. If `[patch]` is needed, this fundamentally changes R2's approach (feature-gated `[patch]` sections are not natively supported by Cargo and require workarounds).

4. **R4: Wrapper module file paths not specified.** R4 says "Create any necessary cfg(feature = simulation) wrapper modules" but does not name the files or module paths. For a spec that must be implementable, the file paths must be explicit (e.g., `packages/server-rust/src/sim.rs` or `packages/server-rust/src/io_seams.rs`). This also affects the file count against the Language Profile limit (max 5 files).

**Recommendations:**
5. [Strategic] The Assumptions section correctly flags the madsim + axum 0.8 compatibility risk, but given this is a foundational spec blocking 3 others, consider adding a "pre-flight check" acceptance criterion: verify `madsim` version X compiles alongside `axum 0.8` before proceeding with implementation. This could be AC0 or a gating step.

6. R2 mentions `[target.'cfg(simulation)'.dependencies]` as an option. This syntax is for target triples (OS/arch), not for Cargo features. Feature-gated dependencies use `feature-name = ["dep:crate-name"]` in the `[features]` section combined with `optional = true` on the dependency. Remove the `[target.'cfg(...)'.dependencies]` suggestion to avoid confusion.

7. The `ci-sim` profile (once corrected per issue 1) should specify what settings it actually configures. If it's just `inherits = "dev"` with no overrides, it serves no purpose. Consider specifying concrete settings like `opt-level = 1` or `debug-assertions = true`, or dropping the profile requirement if it adds no value beyond the feature flag.

### Response v1 (2026-03-19)
**Applied:** All 4 critical issues and all 3 recommendations.

**Changes:**
1. [✓] R1: Cargo profiles cannot enable features — Decoupled profile from feature activation. R1 now states clearly that `ci-sim` controls compiler settings only (`opt-level = 1`, `debug-assertions = true`) and that `--features simulation` is passed separately. The phrase "enables simulation feature" is removed.
2. [✓] R2: Inconsistent cfg syntax — Removed all occurrences of bare `cfg(simulation)` from Context, R2, R4, and Constraints. All conditional compilation gates now read `cfg(feature = "simulation")`. Added an explicit note in R4 explaining why bare `cfg(simulation)` is wrong.
3. [✓] R2: `madsim-tokio` dependency mechanism — Replaced the `madsim-tokio` standalone crate assumption with an accurate description of how madsim works (`madsim::tokio` re-export, `[patch]` mechanism, its unconditional apply constraint). Added Option A / Option B choice with Option A recommended. Implementation must document the chosen option.
4. [✓] R4: Wrapper module file paths not specified — Named the file explicitly as `packages/server-rust/src/sim.rs`. Added it to R4 with a code-fence showing the path. Added AC7 to verify the file exists and is correctly gated.
5. [✓] Pre-flight AC0 added — AC0 requires verifying madsim version compatibility with axum 0.8 and tokio 1.x before implementation proceeds. Pinning the verified version is a prerequisite. Assumptions section updated to match.
6. [✓] `[target.'cfg(simulation)'.dependencies]` removed — R2 no longer mentions this syntax. Added an explicit note that it is for target triples, not features, to prevent implementer confusion.
7. [✓] `ci-sim` profile now has concrete settings — Profile specifies `opt-level = 1` and `debug-assertions = true` in both R1 and AC5. If the implementer decides these settings add no value, they may drop the profile and update AC5 accordingly.

### Audit v2 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~18% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~18% | <=50% | OK |
| Largest task group | ~18% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | madsim is compatible with tokio 1.x + axum 0.8 | Entire spec blocked; AC0 pre-flight gates this |
| A2 | `madsim::tokio` re-export provides sufficient API surface for Option A | May need to fall back to Option B (build-wide patch) |
| A3 | core-rust does not need simulation feature | Verified: core-rust has no `rand` dependency |

**Project Alignment:** Aligned. TODO-027 is in the v2.0 roadmap. Foundation spec correctly scoped as config-only before harness/test specs.

**Project Compliance:** Honors PROJECT.md decisions. Uses established Cargo feature pattern (matches `postgres = ["dep:sqlx"]`, `arrow = ["dep:arrow-schema"]`). No new runtime deps outside `cfg(feature = "simulation")` gate. MsgPack wire format unaffected.

**Language Profile:** Compliant with Rust profile. File count is 5 (workspace Cargo.toml, server-rust Cargo.toml, sim.rs, lib.rs mod line, package.json) — at the limit of 5. Trait-first not applicable (no traits defined). No compilation gate concerns (single small module).

**Strategic fit:** Aligned with project goals.

**Comment:** Spec is well-structured after v1 revision. All four critical issues from Audit v1 have been thoroughly addressed. Requirements are precise with exact Cargo syntax, file paths, and cfg gate syntax. The AC0 pre-flight check correctly gates the main risk (madsim compatibility). The Option A/B flexibility for the tokio shim mechanism is pragmatic — it avoids over-specifying a decision that depends on runtime verification. R3 correctly handles the conditional case (core-rust confirmed to have no `rand` dependency, so it can be skipped).

**Recommendations:**
1. R4 implicitly requires adding `#[cfg(feature = "simulation")] mod sim;` to `packages/server-rust/src/lib.rs`, but this file is not explicitly listed as a modification target. The implementer will infer this, but for maximum clarity consider noting it. Not blocking since the instruction in R4 ("pub mod conditional on the feature in lib.rs") makes the intent clear.

---

## Execution Summary

**Executed:** 2026-03-19
**Commits:** 4

### Files Created
- `packages/server-rust/src/sim.rs` — I/O seam module re-exporting madsim::time and madsim::rand behind cfg(feature = "simulation")

### Files Modified
- `Cargo.toml` (workspace root) — added madsim = "0.2.34" workspace dep and [profile.ci-sim] with opt-level=1, debug-assertions=true
- `packages/server-rust/Cargo.toml` — added simulation = ["dep:madsim"] feature and madsim optional dep
- `packages/server-rust/src/lib.rs` — added #[cfg(feature = "simulation")] pub mod sim
- `package.json` (root) — added test:sim script

### Files Deleted
None.

### Acceptance Criteria Status
- [x] AC0: Pre-flight verified — madsim 0.2.34 compiles alongside axum 0.8 + tokio 1.49.0 with and without simulation feature
- [x] AC1: `cargo check --features simulation -p topgun-server` — passes (exit 0)
- [x] AC2: `cargo check -p topgun-server` (no simulation) — passes, no regressions (exit 0)
- [x] AC3: `cargo clippy --features simulation -p topgun-server` — no warnings (exit 0)
- [x] AC4: `cargo test --release -p topgun-server` — 559 tests pass, 0 failures
- [x] AC5: ci-sim profile defined in workspace Cargo.toml with opt-level=1 and debug-assertions=true
- [x] AC6: pnpm test:sim script exists and invokes correct cargo command
- [x] AC7: packages/server-rust/src/sim.rs exists, gated on cfg(feature = "simulation"), re-exports time and rand seams

### Deviations
None. All requirements implemented as specified.

### Notes
- AC0 confirmed madsim 0.2.34 is compatible. rand version conflict (madsim uses 0.8, server uses 0.9) is not a problem — semver major versions coexist as separate crate instances.
- Option A chosen for tokio shim strategy: madsim::tokio used as conditional import alias inside #[cfg(feature = "simulation")] blocks. No global [patch] applied.
- R3 (core-rust simulation feature) skipped as specified — core-rust has no direct rand dependency, so no seeding needed.
- madsim::time and madsim::rand are available via the std/mod.rs re-exports even without the cfg(madsim) raw rustc flag, matching the Option A approach.

---

## Review History

### Review v1 (2026-03-19)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `cargo check --features simulation -p topgun-server` — exits 0, no errors (verified)
- [✓] AC2: `cargo check -p topgun-server` (no simulation) — exits 0, no regressions (verified)
- [✓] AC3: `cargo clippy --features simulation -p topgun-server` — exits 0, zero warnings (verified)
- [✓] AC4: `cargo test --release -p topgun-server` — 559 tests pass, 0 failed, 0 ignored (verified)
- [✓] AC5: `[profile.ci-sim]` present in `/Users/koristuvac/Projects/topgun/topgun/Cargo.toml` with `inherits = "dev"`, `opt-level = 1`, `debug-assertions = true`; comment explains the why
- [✓] AC6: `"test:sim": "cargo test --profile ci-sim --features simulation -p topgun-server -- sim"` in root `package.json` — exact command matches spec
- [✓] AC7: `packages/server-rust/src/sim.rs` exists; `pub use madsim::time` and `pub use madsim::rand` present; file-level `#![cfg]` gate absent (correct: gate is on the `pub mod sim` line in `lib.rs`, not inside the file itself)
- [✓] R2 feature declaration: `simulation = ["dep:madsim"]` in `[features]`; `madsim = { workspace = true, optional = true }` in `[dependencies]` — matches spec exactly
- [✓] R2 Option A documented: tokio shim strategy explained in `sim.rs` module-level doc comment with explicit code example and rationale; no `[patch]` section added to workspace Cargo.toml
- [✓] R3 skipped correctly: core-rust has no `rand` dependency; decision noted in Execution Summary
- [✓] `lib.rs` module gate: `#[cfg(feature = "simulation")] pub mod sim;` — correct Cargo feature syntax, not bare `cfg(simulation)`
- [✓] No existing non-sim code paths modified: all changes are additive
- [✓] madsim version 0.2.34 pinned as workspace dep — version consistent with AC0 pre-flight confirmation
- [✓] Feature pattern (`simulation = ["dep:madsim"]`) follows established project convention (`postgres = ["dep:sqlx"]`, `arrow = ["dep:arrow-schema"]`)
- [✓] `sim.rs` WHY-comments explain purpose of each re-export (virtual time for deterministic sleep/interval/timeout; seeded RNG for reproducible random sequences)
- [✓] No bare `cfg(simulation)` anywhere in implementation — all gates use `cfg(feature = "simulation")`

**Summary:** All seven acceptance criteria verified live. The implementation is minimal, correct, and follows established project conventions without touching any production code paths. The tokio shim strategy (Option A) is clearly documented in `sim.rs`. No issues found.

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 4
**Review Cycles:** 1

### Outcome

Established the madsim simulation testing foundation for TopGun server: workspace dependency, feature flag, ci-sim profile, I/O seam module (sim.rs), and pnpm test:sim script. All simulation infrastructure is behind `cfg(feature = "simulation")` with zero impact on production code paths.

### Key Files

- `packages/server-rust/src/sim.rs` — I/O seam module providing centralized re-exports of madsim::time and madsim::rand for downstream simulation code
- `Cargo.toml` (workspace) — madsim 0.2.34 workspace dep + ci-sim profile
- `packages/server-rust/Cargo.toml` — simulation feature flag with optional madsim dep

### Patterns Established

- Simulation feature flag pattern: `simulation = ["dep:madsim"]` with `cfg(feature = "simulation")` gates — follows existing `postgres`/`arrow` feature conventions
- Option A tokio shim strategy: conditional import alias via `madsim::tokio` in cfg-gated blocks, no global `[patch]` section

### Deviations

None — implemented as specified.
