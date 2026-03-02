# SPEC-049: Rust Project Bootstrap — Cargo Workspace, CI Pipeline, and Language Profile

```yaml
id: SPEC-049
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-13
todo_ref: TODO-059
```

## Context

Phase 0 (TypeScript Completion) is finished. The project is entering Phase 1 (Bridge TS to Rust), and the very first task is bootstrapping the Rust infrastructure so subsequent Rust specs (TODO-060 onward) can execute immediately.

The monorepo currently uses pnpm workspaces for TypeScript packages. This spec adds a Cargo workspace alongside pnpm, a pattern used by Turso, Deno, and SurrealDB. The structure follows the plan in [RUST_SERVER_MIGRATION_RESEARCH.md Section 8](../reference/RUST_SERVER_MIGRATION_RESEARCH.md).

This blocks ALL subsequent Rust work (TODO-060 through TODO-072).

## Goal Analysis

**Goal Statement:** Establish Rust build infrastructure so the first Rust feature spec (TODO-060: Upfront Trait Definitions) can be executed without any toolchain setup.

**Observable Truths:**
1. `cargo check` succeeds from the repository root
2. `cargo test` succeeds (with at least one passing test per crate)
3. `cargo clippy` passes with zero warnings
4. `cargo fmt --check` passes
5. CI pipeline runs fmt, clippy, and test on push/PR
6. `pnpm build` and `pnpm test` still work (no regression)
7. PROJECT.md contains a Rust Language Profile section

**Required Artifacts:**
- `/Cargo.toml` (workspace root)
- `/rust-toolchain.toml`
- `/packages/core-rust/Cargo.toml` + `src/lib.rs`
- `/packages/server-rust/Cargo.toml` + `src/lib.rs`
- `/.github/workflows/rust.yml`
- `/.gitignore` (updated with Rust entries)
- `/.specflow/PROJECT.md` (updated with Language Profile)

## Task

Create the Rust project bootstrap: Cargo workspace configuration, two skeleton crates, CI pipeline, toolchain pinning, and gitignore/PROJECT.md updates.

## Requirements

### Files to Create

#### 1. `/Cargo.toml` (workspace root)

```toml
[workspace]
resolver = "2"
members = [
    "packages/core-rust",
    "packages/server-rust",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "BSL-1.1"
repository = "https://github.com/topgunbuild/topgun"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"

[workspace.lints.rust]
unsafe_code = "forbid"
```

- `resolver = "2"` is required for edition 2021.
- `clippy::pedantic` as warn (not deny) gives strictness without blocking initial development.
- `unsafe_code = "forbid"` enforces safe Rust from the start.

#### 2. `/rust-toolchain.toml`

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- Stable channel per research document recommendation.
- Components ensure `cargo fmt` and `cargo clippy` are available without manual install.

#### 3. `/packages/core-rust/Cargo.toml`

```toml
[package]
name = "topgun-core"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "TopGun core: CRDTs, HLC, MerkleTree, message schemas"

[lints]
workspace = true

[dependencies]

[dev-dependencies]
```

- Crate name `topgun-core` (not `core-rust` -- crate names use the product name).
- No dependencies yet; TODO-060 and TODO-061 will add serde, rmp-serde, etc.

#### 4. `/packages/core-rust/src/lib.rs`

```rust
//! TopGun Core — CRDTs, Hybrid Logical Clock, MerkleTree, and message schemas.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }
}
```

- Minimal skeleton with one test to verify the crate compiles.
- Doc comment describes the crate's purpose.
- Empty test body avoids `clippy::assertions_on_constants` lint (a test that compiles and runs is sufficient proof the crate loads).

#### 5. `/packages/server-rust/Cargo.toml`

```toml
[package]
name = "topgun-server"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "TopGun server: axum, tokio, clustering, storage"

[lints]
workspace = true

[dependencies]
topgun-core = { path = "../core-rust" }

[dev-dependencies]
```

- Depends on `topgun-core` to establish the package hierarchy from day one.

#### 6. `/packages/server-rust/src/lib.rs`

```rust
//! TopGun Server — WebSocket server with clustering, partitioning, and PostgreSQL storage.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }
}
```

#### 7. `/.github/workflows/rust.yml`

```yaml
name: Rust

on:
  push:
    branches: [main, develop]
    paths:
      - 'packages/core-rust/**'
      - 'packages/server-rust/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'rust-toolchain.toml'
      - '.github/workflows/rust.yml'
  pull_request:
    branches: [main, develop]
    paths:
      - 'packages/core-rust/**'
      - 'packages/server-rust/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'rust-toolchain.toml'
      - '.github/workflows/rust.yml'

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: Check & Lint
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache Cargo registry and build
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: cargo fmt --check
        run: cargo fmt --all -- --check

      - name: cargo clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: cargo test
        run: cargo test --all-targets
```

- Path-filtered: only runs when Rust files change (avoids wasting CI on TS-only changes).
- Single job with sequential steps (simpler than matrix for two small crates).
- Cargo caching via `actions/cache@v4`.
- `clippy -D warnings` in CI (deny all warnings) while workspace config uses warn (allows local iteration).
- `dtolnay/rust-toolchain@stable` respects `rust-toolchain.toml`.
- No separate `cargo check` step: clippy already runs the full check pass, so a standalone check would be redundant.

### Files to Modify

#### 8. `/.gitignore` — append Rust entries

Add the following block at the end of the existing `.gitignore`:

```gitignore

# Rust build artifacts (generated by cargo build/test)
/target/
```

- Only `/target/` at root is needed; member crates use the workspace target directory.

#### 9. `/.specflow/PROJECT.md` — add Language Profile section

Insert before the final `---` line, after the `## Constraints` section:

```markdown
## Language Profile

| Setting | Value |
|---------|-------|
| Language | Rust |
| Max files per spec | 5 |
| Trait-first | Yes |

**Notes:**
- Rust specs use trait-first ordering: G1 (Wave 1) defines traits/types only, implementation groups depend on G1
- Max 5 files per spec to limit borrow checker cascade risk
- Applies to `packages/core-rust/` and `packages/server-rust/` only
- TypeScript packages continue using existing conventions (no file limit, no trait-first)
```

## Acceptance Criteria

- **AC-1:** Running `cargo check` from the repository root exits with code 0.
- **AC-2:** Running `cargo test` from the repository root exits with code 0 and reports 2 passing tests (one per crate).
- **AC-3:** Running `cargo clippy --all-targets --all-features -- -D warnings` exits with code 0 (zero warnings).
- **AC-4:** Running `cargo fmt --all -- --check` exits with code 0 (code is formatted).
- **AC-5:** Running `pnpm build` from the repository root still succeeds (no regression to TypeScript builds). If pnpm emits warnings about Rust directories lacking package.json, the constraint about not modifying pnpm-workspace.yaml may need to be relaxed to explicitly list TS packages or exclude Rust directories.
- **AC-6:** Running `pnpm test` from the repository root still succeeds (no regression to TypeScript tests).
- **AC-7:** `/.github/workflows/rust.yml` exists and contains steps for fmt, clippy, and test.
- **AC-8:** `/rust-toolchain.toml` specifies `channel = "stable"` with rustfmt and clippy components.
- **AC-9:** `/.gitignore` contains `/target/` entry.
- **AC-10:** `.specflow/PROJECT.md` contains a `## Language Profile` section with `Max files per spec: 5` and `Trait-first: Yes`.
- **AC-11:** `topgun-server` crate has a dependency on `topgun-core` (verifiable via `Cargo.toml`).

## Constraints

- DO NOT add any runtime dependencies (tokio, serde, axum, etc.) -- those belong in TODO-060 and later specs.
- DO NOT create `src/main.rs` for `server-rust` yet -- it starts as a library crate. Binary entry point comes with TODO-064 (Network Layer).
- DO NOT modify `pnpm-workspace.yaml` or `package.json` -- Cargo and pnpm are independent build systems.
- DO NOT use nightly Rust features -- stable channel only.
- DO NOT add Rust build steps to the existing `benchmark.yml` workflow -- keep Rust CI separate.

## Assumptions

- **Crate naming:** `topgun-core` and `topgun-server` (kebab-case, product name prefix) rather than `core-rust`/`server-rust` which are directory names only.
- **Single CI job:** One job with sequential steps is sufficient for two skeleton crates. Matrix builds can be added when compilation time warrants it.
- **No `Cargo.lock` in `.gitignore`:** Since `server-rust` will produce a binary, `Cargo.lock` should be committed per Cargo best practices.
- **Workspace lints:** `clippy::pedantic` as warn (not deny) for local development; CI uses `-D warnings` for strictness.
- **`unsafe_code = "forbid"`:** Enforced project-wide from the start. If a future crate needs unsafe (e.g., native FFI), it can override locally with justification.
- **No `deny.toml` / `cargo-deny`:** Supply chain auditing can be added later when actual dependencies are introduced (TODO-060+).
- **pnpm workspace glob compatibility:** The `pnpm-workspace.yaml` uses `packages/*` which will match the new `packages/core-rust/` and `packages/server-rust/` directories. Modern pnpm (v10.13.1) silently skips directories without `package.json`. AC-5 and AC-6 will verify no regressions occur. If pnpm emits warnings, the constraint about not modifying pnpm-workspace.yaml may need to be relaxed to explicitly list TS packages or exclude Rust directories.
- **Pedantic lint tuning:** The `clippy::pedantic` lint may trigger noisy warnings on future non-trivial code (e.g., `must_use_candidate`, `missing_errors_doc`). Specific noisy lints can be allowed in workspace Cargo.toml as real code is added (e.g., `allow = ["clippy::must_use_candidate"]`). Not a blocker for skeleton crates.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create workspace root files (`Cargo.toml`, `rust-toolchain.toml`) | -- | ~15% |
| G2 | 1 | Create `packages/core-rust/` skeleton (`Cargo.toml`, `src/lib.rs`) | -- | ~10% |
| G3 | 1 | Create `packages/server-rust/` skeleton (`Cargo.toml`, `src/lib.rs`) | -- | ~10% |
| G4 | 2 | Create CI workflow (`.github/workflows/rust.yml`) | G1, G2, G3 | ~25% |
| G5 | 2 | Update `.gitignore` and `.specflow/PROJECT.md` | G1 | ~15% |
| G6 | 3 | Verify all acceptance criteria (cargo check/test/clippy/fmt, pnpm build/test) | G4, G5 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |
| 2 | G4, G5 | Yes | 2 |
| 3 | G6 | No | 1 |

**Total workers needed:** 3 (max in any wave)

## Audit History

### Audit v1 (2026-02-13)
**Status:** APPROVED

**Context Estimate:** ~25% total (small spec, all files are new skeleton files with exact content provided)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Every file has exact content specified; no ambiguity |
| Completeness | Excellent | All 9 files listed with full contents; modifications clearly described |
| Testability | Excellent | All 11 acceptance criteria are concrete commands with expected exit codes |
| Scope | Excellent | Clear boundaries; 5 explicit constraints prevent scope creep |
| Feasibility | Excellent | Standard Cargo workspace setup; no novel engineering |
| Architecture fit | Excellent | Matches existing benchmark.yml patterns; Cargo alongside pnpm is a proven pattern |
| Non-duplication | Excellent | No existing Rust infrastructure to duplicate |
| Cognitive load | Excellent | Straightforward boilerplate; easy for any Rust-familiar developer |
| Strategic fit | Excellent | Directly unblocks TODO-060 through TODO-072; critical path item |
| Project compliance | Excellent | Honors all PROJECT.md decisions (stable Rust, MsgPack, no nightly) |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (cargo check) has artifacts | OK | Cargo.toml + crate files |
| Truth 2 (cargo test) has artifacts | OK | lib.rs with tests |
| Truth 3 (cargo clippy) has artifacts | OK | workspace lints config |
| Truth 4 (cargo fmt) has artifacts | OK | rust-toolchain.toml with rustfmt |
| Truth 5 (CI pipeline) has artifacts | OK | rust.yml workflow |
| Truth 6 (pnpm regression) has artifacts | OK | Constraint: do not modify pnpm files |
| Truth 7 (Language Profile) has artifacts | OK | PROJECT.md update |
| All artifacts have purpose | OK | No orphan artifacts |
| Wiring completeness | OK | server-rust depends on core-rust |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Rust stable channel is sufficient for all planned features | Would need to change rust-toolchain.toml (low impact) |
| A2 | clippy::pedantic warn + CI deny is the right strictness balance | Could annoy developers with false positives (low impact, tunable) |
| A3 | pnpm silently ignores directories without package.json in workspace globs | Could cause pnpm warnings/errors (see Recommendation 1) |

Strategic fit: OK -- Aligned with project goals. This is the minimal viable bootstrap for Phase 1.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Stable Rust, not nightly | Spec uses channel = "stable" | OK |
| MsgPack wire protocol | No wire protocol added yet (correct for skeleton) | OK |
| 6 upfront traits in TODO-060 | Spec defers to TODO-060 (correct) | OK |
| No phase/spec refs in code | Spec code has no such references | OK |
| Cargo alongside pnpm | Spec does not modify pnpm files | OK |

Project compliance: OK -- Honors PROJECT.md decisions.

**Language Profile Check:** N/A (this spec creates the Language Profile; it does not yet exist to validate against).

**Comment:** This is a very well-crafted specification. Every file has exact content provided, all acceptance criteria are concrete and verifiable, constraints prevent scope creep, and assumptions are clearly documented with rationale. The spec is fully implementation-ready.

**Recommendations:**
1. [Compatibility] The `pnpm-workspace.yaml` uses `packages/*` which will match the new `packages/core-rust/` and `packages/server-rust/` directories. Modern pnpm (v10) silently skips directories without `package.json`, but this should be verified during implementation (AC-5 and AC-6 will catch any issues). If pnpm emits warnings, the fix would be to change the glob to explicitly list TS packages or exclude Rust directories -- but this would violate the "DO NOT modify pnpm-workspace.yaml" constraint, which may need revisiting.
2. [CI] The `clippy::pedantic` lint combined with `CI -D warnings` may trigger pedantic warnings on future non-trivial code that are tedious to address (e.g., `must_use_candidate`, `missing_errors_doc`). Consider whether `clippy::pedantic` should be scoped or whether specific noisy lints should be allowed in workspace config. Not a blocker for skeleton crates.
3. [Spec structure] The Implementation Tasks section is nested under Assumptions (as a subsection). It should be a top-level section for consistency. Minor formatting concern that does not affect implementation.

### Response v1 (2026-02-13 16:45)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] pnpm workspace glob compatibility — Added note to Assumptions section acknowledging pnpm v10.13.1 silently skips directories without package.json, and updated AC-5 to note that if pnpm emits warnings, the constraint about not modifying pnpm-workspace.yaml may need to be relaxed.
2. [✓] Pedantic lint tuning — Added note to Assumptions section explaining that specific noisy lints (must_use_candidate, missing_errors_doc) can be allowed in workspace Cargo.toml as real code is added.
3. [✓] Implementation Tasks section structure — Promoted "Implementation Tasks" from nested subsection under Assumptions (###) to top-level section (##). Changed heading level from ### to ## and subheadings from #### to ###.

### Audit v2 (2026-02-13)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total (small spec, all files are new skeleton files with exact content provided)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Every file has exact content specified; no ambiguity |
| Completeness | Excellent | All 9 files listed with full contents; modifications clearly described |
| Testability | Excellent | All 11 acceptance criteria are concrete and verifiable |
| Scope | Excellent | Clear boundaries; 5 explicit constraints prevent scope creep |
| Feasibility | Issue | Skeleton test code will fail clippy (see Critical 1) |
| Architecture fit | Excellent | Matches existing benchmark.yml patterns; Cargo alongside pnpm is proven |
| Non-duplication | Excellent | No existing Rust infrastructure to duplicate |
| Cognitive load | Excellent | Straightforward boilerplate |
| Strategic fit | Excellent | Directly unblocks TODO-060 through TODO-072; critical path item |
| Project compliance | Excellent | Honors all PROJECT.md decisions |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| All truths have artifacts | OK | No gaps |
| All artifacts have purpose | OK | No orphans |
| Wiring completeness | OK | server-rust depends on core-rust |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Rust stable channel sufficient | Low impact, change rust-toolchain.toml |
| A2 | clippy::pedantic warn + CI deny is balanced | Low impact, tunable |
| A3 | pnpm ignores dirs without package.json | AC-5/AC-6 will catch |
| A4 | assert!(true) passes clippy | HIGH impact -- AC-3 fails (see Critical 1) |

Strategic fit: OK -- Aligned with project goals.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Stable Rust, not nightly | channel = "stable" | OK |
| MsgPack wire protocol | Not added yet (correct) | OK |
| 6 upfront traits in TODO-060 | Deferred (correct) | OK |
| No phase/spec refs in code | No such references | OK |
| Cargo alongside pnpm | Does not modify pnpm files | OK |

Project compliance: OK -- Honors PROJECT.md decisions.

**Language Profile Check:** N/A (this spec creates the Language Profile).

**Critical:**
1. **`assert!(true)` triggers `clippy::assertions_on_constants` lint.** Both `lib.rs` files (Requirements 4 and 6) use `assert!(true, "...")` as the skeleton test body. The `clippy::assertions_on_constants` lint is part of the `clippy::correctness` category, which is included in `clippy::all`. The workspace `Cargo.toml` sets `all = "warn"`, and CI runs `cargo clippy --all-targets --all-features -- -D warnings`. This means CI will promote the warning to an error and **AC-3 will fail**. Fix: replace `assert!(true, "...")` with an empty test body (an empty test that compiles and runs is sufficient proof the crate loads), or use a non-constant assertion like `assert_eq!(2 + 2, 4)`.

**Recommendations:**
1. [Redundancy] The `cargo check` step in the CI workflow (line: `cargo check --all-targets`) is redundant after `cargo clippy --all-targets --all-features` since clippy already runs the full check pass. Removing it would save a few seconds of CI time. Not a blocker -- it provides a clearer signal if clippy itself has issues vs. compilation issues.

### Response v2 (2026-02-13 17:30)
**Applied:** All items from Audit v2 (1 critical + 1 recommendation)

**Changes:**
1. [✓] Critical 1: `assert!(true)` triggers `clippy::assertions_on_constants` — Replaced `assert!(true, "...")` with empty test bodies in both `lib.rs` code blocks (Requirements 4 and 6). Added explanatory comment in each test body and a note after the Requirement 4 code block explaining why the body is empty.
2. [✓] Recommendation 1: Redundant `cargo check` step in CI — Removed the `cargo check --all-targets` step from the CI workflow (Requirement 7) since clippy already performs the full check pass. Updated the bullet list after the workflow to note the removal. Updated AC-7 from "fmt, clippy, check, and test" to "fmt, clippy, and test".

### Audit v3 (2026-02-13 18:00)
**Status:** APPROVED

**Context Estimate:** ~25% total (small spec, all files are new skeleton files with exact content provided)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~15% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Every file has exact content specified; no ambiguity |
| Completeness | Excellent | All 9 files (7 create + 2 modify) listed with full contents |
| Testability | Excellent | All 11 acceptance criteria are concrete commands with expected exit codes |
| Scope | Excellent | Clear boundaries; 5 explicit constraints prevent scope creep |
| Feasibility | Excellent | Previous clippy issue (assert!(true)) resolved; empty test bodies are correct |
| Architecture fit | Excellent | CI follows existing benchmark.yml patterns; Cargo alongside pnpm is proven |
| Non-duplication | Excellent | No existing Rust infrastructure to duplicate |
| Cognitive load | Excellent | Straightforward boilerplate; easy for any Rust-familiar developer |
| Strategic fit | Excellent | Directly unblocks TODO-060 through TODO-072; critical path item |
| Project compliance | Excellent | Honors all PROJECT.md decisions (stable Rust, no nightly, no deps) |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (cargo check) has artifacts | OK | Cargo.toml + crate files |
| Truth 2 (cargo test) has artifacts | OK | lib.rs with empty test functions |
| Truth 3 (cargo clippy) has artifacts | OK | workspace lints config + empty test bodies (no clippy issues) |
| Truth 4 (cargo fmt) has artifacts | OK | rust-toolchain.toml with rustfmt |
| Truth 5 (CI pipeline) has artifacts | OK | rust.yml with fmt, clippy, test steps |
| Truth 6 (pnpm regression) has artifacts | OK | Constraint: do not modify pnpm files; AC-5/AC-6 verify |
| Truth 7 (Language Profile) has artifacts | OK | PROJECT.md update specified |
| All artifacts have purpose | OK | No orphan artifacts |
| Wiring completeness | OK | server-rust depends on core-rust |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Rust stable channel sufficient | Low impact, change rust-toolchain.toml |
| A2 | clippy::pedantic warn + CI deny is balanced | Low impact, tunable per Assumptions section |
| A3 | pnpm ignores dirs without package.json | AC-5/AC-6 will catch; Assumptions section documents escape hatch |

Strategic fit: OK -- Aligned with project goals. Minimal viable bootstrap for Phase 1.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Stable Rust, not nightly | channel = "stable" | OK |
| MsgPack wire protocol | Not added yet (correct for skeleton) | OK |
| 6 upfront traits in TODO-060 | Deferred (correct) | OK |
| No phase/spec refs in code | No such references in code blocks | OK |
| Cargo alongside pnpm | Does not modify pnpm files | OK |

Project compliance: OK -- Honors PROJECT.md decisions.

**Language Profile Check:** N/A (this spec creates the Language Profile; it does not yet exist to validate against).

**Verification of Audit v2 fixes:**
- Critical 1 (assert!(true) clippy lint): FIXED. Both lib.rs files now use empty test bodies with explanatory comments. No clippy-triggering assertions.
- Recommendation 1 (redundant cargo check): FIXED. CI workflow has three steps (fmt, clippy, test). AC-7 updated to match.

**Comment:** All issues from Audit v2 have been correctly addressed. The specification is well-crafted with exact file contents, concrete acceptance criteria, clear constraints, and thorough assumptions documentation. Ready for implementation.

**Recommendations:**
1. [Consistency] Observable Truth 5 states "CI pipeline runs all four Cargo commands" but the CI workflow runs three steps (fmt, clippy, test) since clippy subsumes check. The text should say "three Cargo commands" or "fmt, clippy, and test" to match AC-7. This is a cosmetic inconsistency that does not affect implementation.

### Response v3 (2026-02-13 18:15)
**Applied:** Recommendation 1 from Audit v3

**Changes:**
1. [✓] Consistency: Observable Truth 5 "all four Cargo commands" → "fmt, clippy, and test" to match AC-7 and the actual CI workflow (3 steps, not 4).

### Audit v4 (2026-02-13 19:00)
**Status:** APPROVED

**Context Estimate:** ~25% total (small spec, all files are new skeleton files with exact content provided)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~25% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Dimensions:**

| Dimension | Rating | Notes |
|-----------|--------|-------|
| Clarity | Excellent | Every file has exact content in code blocks; no ambiguity whatsoever |
| Completeness | Excellent | All 9 files (7 create + 2 modify) fully specified with exact content |
| Testability | Excellent | 11 acceptance criteria expressed as concrete commands with expected exit codes |
| Scope | Excellent | 5 explicit DO NOT constraints; complexity correctly marked as small |
| Feasibility | Excellent | Empty test bodies are clippy-safe; all Cargo/CI patterns are standard |
| Architecture fit | Excellent | CI mirrors existing benchmark.yml style; Cargo-alongside-pnpm is proven in Turso/Deno/SurrealDB |
| Non-duplication | Excellent | No existing Rust infrastructure in the repository |
| Cognitive load | Excellent | Pure boilerplate; no business logic, no complex wiring |
| Strategic fit | Excellent | Critical path item: blocks TODO-060 through TODO-072 |
| Project compliance | Excellent | Stable Rust, no deps, no pnpm modification, no phase/spec refs in code |

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (cargo check) has artifacts | OK | Workspace Cargo.toml + 2 crate Cargo.toml + 2 lib.rs |
| Truth 2 (cargo test) has artifacts | OK | Both lib.rs files contain one test each |
| Truth 3 (cargo clippy) has artifacts | OK | Workspace lints config; empty test bodies avoid lint triggers |
| Truth 4 (cargo fmt) has artifacts | OK | rust-toolchain.toml includes rustfmt component |
| Truth 5 (CI pipeline) has artifacts | OK | rust.yml with fmt, clippy, test steps |
| Truth 6 (pnpm regression) has artifacts | OK | Constraint prevents pnpm file modification; AC-5/AC-6 verify |
| Truth 7 (Language Profile) has artifacts | OK | PROJECT.md modification specified with exact content |
| All artifacts have purpose | OK | No orphan artifacts |
| Wiring completeness | OK | topgun-server depends on topgun-core via path dependency |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Rust stable channel sufficient for all planned features | Low -- change one line in rust-toolchain.toml |
| A2 | clippy::pedantic warn + CI deny is the right strictness | Low -- tunable; Assumptions section documents escape hatch |
| A3 | pnpm v10.13.1 silently skips directories without package.json | Low -- AC-5/AC-6 catch it; Assumptions section documents fallback |

Strategic fit: OK -- Aligned with project goals. Minimal viable bootstrap for Phase 1.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| Stable Rust, not nightly | channel = "stable" | OK |
| MsgPack wire protocol | Not added yet (correct for skeleton) | OK |
| 6 upfront traits in TODO-060 | Deferred to TODO-060 (correct) | OK |
| No phase/spec refs in code | No such references in any code block | OK |
| Cargo alongside pnpm | Does not modify pnpm-workspace.yaml or package.json | OK |
| No new runtime dependencies (constraint) | Empty [dependencies] for core-rust; only path dep for server-rust | OK |

Project compliance: OK -- Honors all PROJECT.md decisions and constraints.

**Language Profile Check:** N/A (this spec creates the Language Profile; it does not yet exist to validate against).

**Verification of Response v3 fix:**
- Audit v3 Recommendation 1 (Observable Truth 5 consistency): FIXED. Truth 5 now reads "CI pipeline runs fmt, clippy, and test on push/PR" matching AC-7 and the CI workflow.

**Comment:** The specification has been through three revision cycles and all previously identified issues have been resolved. The current state is clean and internally consistent: Observable Truths match Acceptance Criteria, Acceptance Criteria match Requirements, and Constraints align with PROJECT.md. All file contents are exact, all criteria are measurable, and the scope is appropriately bounded. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-13
**Commits:** 5

### Files Created
- `/Cargo.toml` — Workspace root with resolver 2, pedantic clippy, forbid unsafe
- `/Cargo.lock` — Generated lockfile committed for reproducible builds
- `/rust-toolchain.toml` — Pinned to stable channel with rustfmt and clippy
- `/packages/core-rust/Cargo.toml` — topgun-core crate config, no deps
- `/packages/core-rust/src/lib.rs` — Skeleton with crate_loads test
- `/packages/server-rust/Cargo.toml` — topgun-server crate config, depends on topgun-core
- `/packages/server-rust/src/lib.rs` — Skeleton with crate_loads test
- `/.github/workflows/rust.yml` — CI pipeline with fmt, clippy, test steps

### Files Modified
- `/.gitignore` — Appended `/target/` entry for Rust build artifacts
- `/.specflow/PROJECT.md` — Added Language Profile section (Rust, max 5 files, trait-first)

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC-1: `cargo check` exits with code 0
- [x] AC-2: `cargo test` exits with code 0, reports 2 passing tests (1 per crate)
- [x] AC-3: `cargo clippy --all-targets --all-features -- -D warnings` exits with code 0
- [x] AC-4: `cargo fmt --all -- --check` exits with code 0
- [x] AC-5: `pnpm build` still succeeds (no regression)
- [x] AC-6: `pnpm test` — core (1982), client (501), server (1211) all pass; mcp-server timeout failures are pre-existing
- [x] AC-7: `rust.yml` exists with fmt, clippy, and test steps
- [x] AC-8: `rust-toolchain.toml` specifies `channel = "stable"` with rustfmt and clippy
- [x] AC-9: `.gitignore` contains `/target/` entry
- [x] AC-10: `PROJECT.md` contains Language Profile with `Max files per spec: 5` and `Trait-first: Yes`
- [x] AC-11: `topgun-server` has dependency on `topgun-core`

### Deviations
1. [Rule 1 - Bug] Fixed `clippy::doc_markdown` lint in both lib.rs files. The spec's exact doc comments (`TopGun`, `MerkleTree`, `WebSocket`, `PostgreSQL`) triggered the pedantic lint because CamelCase words in doc comments must be backticked. Added backticks: `` `TopGun` ``, `` `MerkleTree` ``, `` `WebSocket` ``, `` `PostgreSQL` ``.

### Notes
- Rust was not installed on the machine; installed via rustup during verification (stable-aarch64-apple-darwin, rustc 1.93.1)
- The `pnpm test` shows pre-existing timeout failures in `@topgunbuild/mcp-server` (4 tests in tools.test.ts). This is unrelated to the Rust bootstrap. All core/client/server packages pass their full test suites.
- `Cargo.lock` was committed per Cargo best practices since server-rust will eventually produce a binary.

---

## Review History

### Review v1 (2026-02-13 19:45)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC-1: `cargo check` exits 0 -- verified locally
- [✓] AC-2: `cargo test` exits 0 with 2 passing tests (1 per crate) -- verified locally: `topgun_core::tests::crate_loads` and `topgun_server::tests::crate_loads` both pass
- [✓] AC-3: `cargo clippy --all-targets --all-features -- -D warnings` exits 0 -- verified locally, zero warnings
- [✓] AC-4: `cargo fmt --all -- --check` exits 0 -- verified locally, code is formatted
- [✓] AC-5: `pnpm build` succeeds -- verified locally, all packages build including examples/apps
- [✓] AC-6: `pnpm test` succeeds -- verified locally: core (1982 pass), client (501 pass), server (1211 pass); mcp-server timeouts are pre-existing
- [✓] AC-7: `/.github/workflows/rust.yml` exists with fmt, clippy, and test steps -- file verified, 3 steps present at lines 50-57
- [✓] AC-8: `/rust-toolchain.toml` specifies `channel = "stable"` with rustfmt and clippy -- file verified
- [✓] AC-9: `/.gitignore` contains `/target/` entry at line 99 -- verified
- [✓] AC-10: `PROJECT.md` contains Language Profile section with `Max files per spec | 5` and `Trait-first | Yes` at lines 83-95 -- verified
- [✓] AC-11: `topgun-server` depends on `topgun-core` via `topgun-core = { path = "../core-rust" }` at `/packages/server-rust/Cargo.toml:12` -- verified
- [✓] Constraint: No runtime dependencies -- only path dependency on topgun-core in server-rust, no tokio/serde/axum
- [✓] Constraint: No main.rs -- neither crate has `src/main.rs`
- [✓] Constraint: pnpm-workspace.yaml unmodified -- still `packages/*` glob, no changes
- [✓] Constraint: No nightly features -- `channel = "stable"` confirmed
- [✓] Constraint: No Rust steps in benchmark.yml -- rust.yml is a separate workflow
- [✓] Deviation properly documented -- `clippy::doc_markdown` fix for backticked identifiers in doc comments is a legitimate bug fix (spec's exact text would fail AC-3)
- [✓] Cargo.lock committed -- per Cargo best practices for binary-producing workspaces, not in .gitignore
- [✓] File contents match spec exactly for all files except the documented deviation (backticked doc comments)
- [✓] No security issues -- no hardcoded secrets, no dependencies to audit, no unsafe code (forbidden at workspace level)
- [✓] Architecture alignment -- Cargo workspace alongside pnpm, following Turso/Deno/SurrealDB pattern
- [✓] No code duplication -- all files are unique bootstrap artifacts
- [✓] Cognitive load is minimal -- pure boilerplate, no business logic

**Summary:** Clean implementation that matches the specification exactly, with one well-documented deviation (backticking identifiers in doc comments to satisfy `clippy::doc_markdown` pedantic lint). All 11 acceptance criteria pass. All 5 constraints are respected. No critical, major, or minor issues found. The Rust bootstrap is ready for TODO-060 (Upfront Trait Definitions) to proceed.

---

## Completion

**Completed:** 2026-02-13
**Total Commits:** 5
**Audit Cycles:** 4
**Review Cycles:** 1
