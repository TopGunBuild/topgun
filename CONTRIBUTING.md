# Contributing to TopGun

Thank you for your interest in contributing to TopGun! This document provides guidelines and instructions for contributing.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Node.js** >= 18.0.0
- **pnpm** 10.13.1 or later (package manager)
- **Git**

### Installing pnpm

```bash
npm install -g pnpm@10.13.1
```

Or using corepack:

```bash
corepack enable
corepack prepare pnpm@10.13.1 --activate
```

## Getting Started

### 1. Fork and Clone

```bash
git clone https://github.com/YOUR_USERNAME/topgun.git
cd topgun
```

### 2. Install Dependencies

```bash
pnpm install
```

### 3. Build All Packages

```bash
pnpm build
```

This builds all packages in the monorepo in the correct order.

## Rust Toolchain

### Pin mechanism

The Rust toolchain is pinned at the repo root via [`rust-toolchain.toml`](rust-toolchain.toml) to the exact patch-level version `1.93.1`. When you `cd` into the repository, [`rustup`](https://rustup.rs) reads the file and automatically installs the pinned `rustc`, `cargo`, `rustfmt`, and `clippy` for you on first use — no manual `rustup install` step is required.

CI honors the same pin. Every job in [`.github/workflows/rust.yml`](.github/workflows/rust.yml) — namely:

- `check`
- `cross-lang`
- `perf-gate`
- `vector-perf-gate`
- `simulation`
- `audit`

— installs the toolchain via `dtolnay/rust-toolchain@stable` (or via the default `rustup` already present on `ubuntu-latest` in the case of `audit`). The `@stable` suffix on the dtolnay action refers to the **action's own** release tag, not to the Rust release channel. With `rust-toolchain.toml` present, both the dtolnay action and the system `rustup` defer to the pin and install `1.93.1` regardless of what the latest stable rust release happens to be that day.

### Why this is pinned

The channel was previously set to `"stable"` (a floating channel that resolves to whatever the latest stable rust release is at install time). Between minor versions, `clippy` rotates default-warn lints in and out of its base ruleset — and across several silent bumps, **54 `clippy` violations accumulated undetected** in the workspace's test code until SPEC-202 surfaced and cleared them in a single mechanical sweep.

Pinning to a specific patch-level version closes that gap: the `clippy` ruleset is now identical across every developer machine, every CI job, and every release artifact. A new lint can only land via an explicit pin bump that goes through the review gate described below.

### Upgrade cadence policy

| Field | Value |
|-------|-------|
| **Trigger** | Bump on a **quarterly** cadence (target: first PR after each calendar quarter rolls over) **OR** when a **security advisory** affecting the toolchain (`rustc` / `cargo` / `rustfmt` / `clippy`) lands, whichever comes first. |
| **Owner** | Whoever opens the next Rust-touching PR after the cadence trigger fires owns the bump. If no Rust-touching PR is pending when the trigger fires, the bump is filed as a standalone `TODO-*` entry and picked up by the maintainer rotation. |
| **Pin format** | New pin MUST be a **patch-level** version string (e.g. `"1.94.2"`). Never `"stable"`, never minor-level (`"1.94"`), never `nightly`. Patch-level precision prevents the `1.NN.x → 1.NN.x+1` failure mode where a patch release silently re-enables a lint that was disabled in the previous patch. |
| **Validation gate** | The PR that bumps the pin MUST be green on **all four** of the commands below. |

The four validation-gate commands the bump PR must pass:

```bash
# 1. Zero clippy warnings across all targets and feature combinations.
cargo clippy --all-targets --all-features -- -D warnings

# 2. Zero rustfmt format drift.
cargo fmt --all -- --check

# 3. Full server lib test suite passes (floor: 1254 tests per current baseline).
cargo test --release -p topgun-server --lib

# 4. Load harness fire-and-wait scenario within 20% of the baseline in
#    packages/server-rust/benches/load_harness/baseline.json.
cargo bench --bench load_harness -- --connections 50 --duration 10
```

If any of the four fails, the pin bump is blocked — investigate (new lint? new test break? perf regression?), open follow-up issues, and only land the pin once all four are green.

### Lint strictness policy (clippy)

The Rust workspace runs `clippy::all` and `clippy::pedantic` at `warn` for every crate in `members = ["packages/core-rust", "packages/server-rust"]` via the `[workspace.lints.clippy]` block in the root [`Cargo.toml`](Cargo.toml). On top of that baseline, the following additional rules from `clippy::restriction` are adopted at `warn` level:

- `clippy::dbg_macro` — ban `dbg!()` in production code; `dbg!()` is a debug-time helper and shipping it leaks to stderr in production. Zero existing fires at adoption time.
- `clippy::todo` — ban `todo!()` in production code; placeholder panics must not reach a release artifact. Zero existing fires at adoption time.
- `clippy::unimplemented` — ban `unimplemented!()` in production code; same intent as `todo!`. Two test-only fires in `eviction_orchestrator.rs`'s `MockStore` are relaxed via the per-target override below.

**CI enforcement:** every rule in `[workspace.lints.clippy]` is enforced by the CI `check` job at `cargo clippy --all-targets --all-features -- -D warnings` (see [`.github/workflows/rust.yml`](.github/workflows/rust.yml)). A new `dbg!()`, `todo!()`, or `unimplemented!()` in a non-test path will fail the PR — `warn` is upgraded to `error` by the `-D warnings` flag in CI.

**Test-scope override mechanism.** Test code legitimately uses `unimplemented!()` (mock-trait stubs that the test path never calls) and other production-targeted constructs. Relax these once per crate at the top of `lib.rs` via the inner `#![cfg_attr(test, allow(...))]` attribute, NOT call-site `#[allow]` annotations:

```rust
// In packages/server-rust/src/lib.rs (top of file, after the crate doc-comment):
#![cfg_attr(test, allow(clippy::unimplemented))]
```

The attribute MUST list only rules that actually fire in test code under that crate. Adding a rule that does not fire is dead config. A pattern of call-site `#[allow(clippy::X)]` sprinkled across test files is a signal that the rule is the wrong fit — either downgrade the rule or remove it from `[workspace.lints.clippy]`.

**Rules evaluated but rejected** (each was probed via `cargo clippy --all-targets --all-features --message-format=json -- -A clippy::all -A clippy::pedantic -W clippy::<rule>` with awk-driven classification of unique `file:line` fire sites; numbers below are deduplicated across `--all-targets`):

| Rule | Production fires | Test fires (inline + integration) | Rejection rationale |
|------|------------------|-----------------------------------|---------------------|
| `clippy::unwrap_used` | 25 | 910 | `>3` production fires exceed the per-spec sweep ceiling. A focused cleanup spec (`?` propagation, `.expect("WHY")` annotation) is the right scope for adoption; re-evaluate when that cleanup lands. |
| `clippy::expect_used` | 30 | 776 | `.expect("WHY")` with a WHY-comment is the canonical escape hatch in the codebase when `?` propagation is not available. Banning it fights the existing convention rather than complementing it. |
| `clippy::panic` | 0 | 170 | Zero production fires, but 16 integration-test fires across 7 separate test crates under `packages/{core-rust,server-rust}/tests/`. Each integration test is its own crate, so a single `lib.rs` `#![cfg_attr(test, allow(...))]` does not reach them — adoption would require a 7-file sweep that exceeds the 5-file ceiling per Rust spec. Defer to a future per-integration-test relaxation spec. |
| `clippy::string_slice` | 9 | 8 | `>3` production fires across 3 files (`expr_parser.rs`, `sync.rs`, `predicate.rs`). The fix shape is parser/sync-protocol rewrites — out of scope for a strictness-policy spec. |

**Nursery rules** (`clippy::nursery`) were enumerated as a category and rejected wholesale at this round. The top five by frequency: `use_self` (187), `missing_const_for_fn` (160), `derive_partial_eq_without_eq` (114), `option_if_let_else` (92), `significant_drop_tightening` (65). None fit cleanly under the `<3` production-fire ceiling, and nursery rules are explicitly experimental — re-evaluate on the next quarterly toolchain bump.

The validation-gate commands above (`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `cargo test --release -p topgun-server --lib`, `cargo bench --bench load_harness ...`) double as the validation gate for any future rule adoption. Re-run them after editing `[workspace.lints.clippy]`.

## Project Structure

```
topgun/
├── packages/           # Core packages
│   ├── core/          # @topgunbuild/core - CRDT, types, utilities
│   ├── client/        # @topgunbuild/client - Browser/Node client
│   ├── server-rust/   # Rust WebSocket server (axum, tokio, sqlx)
│   ├── core-rust/     # Rust CRDTs, HLC, MerkleTree
│   ├── adapters/      # @topgunbuild/adapters - Storage adapters
│   ├── react/         # @topgunbuild/react - React bindings
│   ├── adapter-better-auth/  # @topgunbuild/adapter-better-auth
│   ├── mcp-server/    # @topgunbuild/mcp-server - MCP for Claude Desktop / Cursor
│   ├── schema/        # @topgunbuild/schema - shared Zod schemas + codegen
│   └── create-topgun-app/    # `npx create-topgun-app` scaffold CLI
│
├── apps/              # Applications
│   ├── docs-astro/    # Documentation site
│   └── admin-dashboard/
│
├── examples/          # Example applications
│   ├── notes-app/     # PWA notes app with offline sync
│   └── ...
│
└── tests/             # Integration tests
    ├── integration-rust/  # TS client → Rust server tests
    └── k6/            # Load/performance tests
```

### Package Dependencies

The packages have the following dependency hierarchy:

```
@topgunbuild/core (no internal deps)
    ↓
@topgunbuild/client (depends on core)
    ↓
@topgunbuild/adapters, @topgunbuild/react (depend on client)

Server (Rust): packages/server-rust depends on packages/core-rust
```

## Running Tests

### Unit Tests

Run all package tests:

```bash
pnpm test
```

Run tests for a specific package:

```bash
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/client test

# Rust server tests
SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server
```

### Test Coverage

```bash
pnpm test:coverage
```

Or for a specific package:

```bash
pnpm --filter @topgunbuild/core test:coverage
```

### Integration Tests (TS client → Rust server)

```bash
pnpm test:integration-rust
```

### Load Tests (k6)

```bash
pnpm test:k6:smoke         # quick sanity-check
pnpm test:k6:throughput    # write-read mixed load
pnpm test:k6:write         # write-heavy
pnpm test:k6:connections   # connection-storm
```

The k6 binary is custom-built with msgpack support — run `pnpm test:k6:build` once to install it under `bin/k6`. For dockerized k6, see `tests/k6/Dockerfile.k6`.

For the in-process Rust load harness (no network), see [`packages/server-rust/benches/load_harness/`](packages/server-rust/benches/load_harness/) and run `cargo bench --bench load_harness`.

## Development Workflow

### Working on a Package

1. Navigate to the package or work from root with filters:

```bash
# Build specific package
pnpm --filter @topgunbuild/core build

# Run tests in watch mode (if configured)
pnpm --filter @topgunbuild/core test
```

2. If your changes affect dependent packages, rebuild them:

```bash
pnpm build
```

### Running Examples

Start the development server:

```bash
pnpm start:server
```

Run an example app:

```bash
cd examples/notes-app
pnpm install
pnpm dev
```

## Pull Request Guidelines

### Before Submitting

1. **Create a feature branch** from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the code style guidelines

3. **Write/update tests** for your changes

4. **Run the full test suite**:
   ```bash
   pnpm test
   ```

5. **Build all packages** to ensure no TypeScript errors:
   ```bash
   pnpm build
   ```

### PR Requirements

- Clear, descriptive title
- Description of what changes were made and why
- Reference any related issues (e.g., "Fixes #123")
- All tests pass
- No TypeScript errors
- New features include tests

### Commit Messages

Use clear, descriptive commit messages:

```
feat(core): add new CRDT merge strategy
fix(client): resolve sync race condition
docs: update API documentation
test(server): add cluster integration tests
chore: update dependencies
```

Format: `type(scope): description`

Types: `feat`, `fix`, `docs`, `test`, `chore`, `refactor`, `perf`

## Contributor License Agreement (CLA)

All contributions to TopGun are governed by a Contributor License Agreement (CLA) — specifically, the [Apache Individual Contributor License Agreement](.github/CLA.md). Signing the CLA grants TopGun the right to distribute and sublicense your contribution, including under future licenses for enterprise modules.

**How it works:**

1. Open a PR as usual.
2. The [cla-assistant.io](https://cla-assistant.io) bot will comment with a sign-link if you have not yet signed.
3. Click the link, sign in with GitHub, and accept the CLA. Takes about 30 seconds.
4. The bot's status check turns green; your PR is then mergeable (subject to normal review).
5. You only sign once. Future PRs from the same GitHub account auto-pass.

**Existing committers:** Anyone with merged commits in `TopGunBuild/topgun` prior to CLA introduction (see [`legal/GRANDFATHERED_COMMITTERS.md`](legal/GRANDFATHERED_COMMITTERS.md)) is grandfathered under the existing Apache License 2.0 grant. No retroactive signing required for past contributions.

**Questions about the CLA:** Open a GitHub Discussion or contact the maintainer.

## Code Style

### TypeScript

- Use TypeScript for all source code
- Enable strict mode
- Prefer explicit types over `any`
- Use interfaces for object shapes
- Export types alongside implementations

### General Guidelines

- Keep functions small and focused
- Write self-documenting code with clear naming
- Add comments for complex logic
- Follow existing patterns in the codebase

### Building

All packages use `tsup` for building. Each package outputs:
- CommonJS (`dist/index.js`)
- ESM (`dist/index.mjs`)
- Type declarations (`dist/index.d.ts`)

## Questions and Support

- **GitHub Issues**: For bug reports and feature requests
- **Discussions**: For questions and general discussion

## License

By contributing to TopGun, you agree that your contributions will be licensed under the Apache License 2.0.
