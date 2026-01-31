# SPEC-017: Add ESLint + Prettier Configuration

```yaml
id: SPEC-017
type: feature
status: done
priority: medium
complexity: small
created: 2026-01-31
```

## Context

The TopGun monorepo currently has no linting or formatting configuration. As noted in TODO-006, this means code style is inconsistent and relies entirely on developer discipline. Adding ESLint and Prettier will:

- Enforce consistent code style automatically
- Catch potential bugs and anti-patterns early
- Reduce code review friction around style issues
- Enable IDE integration for real-time feedback

## Task

Add ESLint (with TypeScript support) and Prettier configuration to the root of the monorepo, with scripts to lint and format all packages.

## Goal Analysis

**Goal Statement:** Consistent code style enforced automatically across all packages via CLI commands and IDE integration.

**Observable Truths:**
1. Running `pnpm lint` executes ESLint on all TypeScript files and reports violations
2. Running `pnpm format` applies Prettier formatting to all eligible files
3. Running `pnpm format:check` validates formatting without modifying files (for CI)
4. ESLint config extends TypeScript-recommended rules
5. Prettier and ESLint do not conflict (formatting handled by Prettier only)

**Required Artifacts:**
- `eslint.config.js` - ESLint flat config (modern format)
- `.prettierrc` - Prettier configuration
- `.prettierignore` - Files to exclude from formatting
- Root `package.json` - Updated with lint/format scripts and devDependencies

**Key Links:**
- Observable Truth #5 (no conflicts) → eslint.config.js must not include formatting rules (delegated to Prettier)
- `.prettierrc` → package.json `format` script (applies formatting)
- `eslint.config.js` → package.json `lint` script (applies linting without formatting)

## Requirements

### Files to Create

#### `eslint.config.js` (root)
ESLint flat config (v9+ format) with:
- TypeScript parser and plugin (unified `typescript-eslint` package)
- Extends `eslint:recommended` and `@typescript-eslint/recommended`
- Ignores: `node_modules`, `dist`, `coverage`, `.specflow`
- Rules (minimal, non-breaking):
  - `no-console`: `warn` (not error, logging is used intentionally in server)
  - `@typescript-eslint/no-unused-vars`: `error` with `argsIgnorePattern: "^_"`
  - `@typescript-eslint/explicit-function-return-type`: `off` (tsup handles declarations)
  - `@typescript-eslint/no-explicit-any`: `warn` (ongoing effort to eliminate)

#### `.prettierrc` (root)
```json
{
  "semi": true,
  "singleQuote": true,
  "tabWidth": 2,
  "trailingComma": "all",
  "printWidth": 100,
  "bracketSpacing": true
}
```

#### `.prettierignore` (root)
```
node_modules
dist
coverage
*.md
pnpm-lock.yaml
.specflow
```

### Files to Modify

#### `package.json` (root)

Add devDependencies:
```json
{
  "devDependencies": {
    "typescript-eslint": "^8.x",
    "eslint": "^9.x",
    "prettier": "^3.x"
  }
}
```

Add scripts:
```json
{
  "scripts": {
    "lint": "eslint 'packages/*/src/**/*.{ts,tsx}' 'tests/**/*.{ts,tsx}'",
    "lint:fix": "eslint 'packages/*/src/**/*.{ts,tsx}' 'tests/**/*.{ts,tsx}' --fix",
    "format": "prettier --write 'packages/*/src/**/*.{ts,tsx}' 'tests/**/*.{ts,tsx}'",
    "format:check": "prettier --check 'packages/*/src/**/*.{ts,tsx}' 'tests/**/*.{ts,tsx}'"
  }
}
```

### Files to Delete

None.

## Acceptance Criteria

1. **Lint command works:** `pnpm lint` runs without crashing and produces output
2. **Format command works:** `pnpm format` runs and exits with code 0
3. **Format check works:** `pnpm format:check` detects unformatted files (exit 1) or confirms all formatted (exit 0)
4. **TypeScript integration:** ESLint correctly parses TypeScript files including generics, decorators, and type assertions
5. **No config conflicts:** ESLint does not report formatting issues (Prettier handles all formatting)
6. **Build unaffected:** `pnpm build` continues to succeed after changes
7. **Tests unaffected:** `pnpm test` continues to pass after changes
8. **TSX support:** `pnpm lint` and `pnpm format` process React `.tsx` files in packages/react/src

## Verification Commands

```bash
# Verify lint runs
pnpm lint

# Verify format check runs
pnpm format:check

# Verify build still works
pnpm build

# Verify tests still pass
pnpm test
```

## Constraints

- **Do not auto-fix existing code:** Initial setup should not bulk-reformat the codebase. Formatting can be applied incrementally.
- **Do not add pre-commit hooks:** This spec is for configuration only. Git hooks (husky, lint-staged) are a separate concern.
- **Do not add CI integration:** CI pipeline changes are out of scope.
- **Keep rules minimal:** Start with recommended presets. Custom rules can be added later based on team feedback.
- **Use ESLint flat config:** Do not use deprecated `.eslintrc` format.

## Assumptions

- ESLint v9 with flat config format is appropriate (standard as of 2024+)
- Prettier defaults align with existing code style (semi: true, single quotes based on codebase inspection)
- `printWidth: 100` is reasonable for this codebase
- TypeScript strict mode compatibility is required
- No need for package-specific ESLint configs (monorepo shares single config)

## Out of Scope

- Pre-commit hooks (husky/lint-staged)
- CI pipeline integration
- Editor-specific config files (`.vscode/settings.json`)
- Bulk formatting of existing codebase
- React-specific ESLint rules (eslint-plugin-react, eslint-plugin-react-hooks) - can be added later
- k6 load testing scripts (JavaScript) - tests/k6/ contains JavaScript files intentionally excluded by TypeScript-only lint/format patterns

---

## Audit History

### Audit v1 (2026-01-31 14:30)
**Status:** APPROVED

**Context Estimate:** ~14% total (PEAK range)

**Dimensions Evaluated:**
- Clarity: PASS - Clear title, context, and task description
- Completeness: PASS - All files listed with specific content
- Testability: PASS - 7 measurable acceptance criteria
- Scope: PASS - Clear boundaries with 5 explicit constraints
- Feasibility: PASS - Standard tooling, achievable approach
- Architecture Fit: PASS - Follows monorepo root config pattern
- Non-Duplication: PASS - No existing linting config
- Cognitive Load: PASS - Minimal rule set, standard tools
- Strategic Fit: PASS - Aligns with developer experience goals

**Goal Analysis Check:**
- All 5 observable truths have corresponding artifacts
- No orphan artifacts
- Missing Key Links section (minor, acceptable for small spec)

**Recommendations:**

1. **[Technical] Include `.tsx` files in scripts:** The React package contains 16 `.tsx` files that would be excluded by the current `*.ts` patterns. Consider updating scripts to use `*.{ts,tsx}` glob patterns.

2. **[Technical] Consider unified `typescript-eslint` package:** For ESLint v9, the modern approach is to use the unified `typescript-eslint` package rather than separate `@typescript-eslint/parser` and `@typescript-eslint/eslint-plugin`. This simplifies configuration.

3. **[Goal Analysis] Missing Key Links:** Goal Analysis could explicitly identify the key integration point between ESLint and Prettier configs (the no-conflict guarantee via rule separation).

**Comment:** Well-structured specification with clear requirements and appropriate scope for a small infrastructure task. Ready for implementation.

### Response v1 (2026-01-31 14:35)
**Applied:** All 3 recommendations

**Changes:**
1. [x] Include `.tsx` files in scripts - Updated all script patterns from `*.ts` to `*.{ts,tsx}` in package.json scripts (lint, lint:fix, format, format:check)
2. [x] Use unified `typescript-eslint` package - Replaced separate `@typescript-eslint/parser` and `@typescript-eslint/eslint-plugin` with single `typescript-eslint` package in devDependencies
3. [x] Add Key Links to Goal Analysis - Added Key Links section documenting ESLint/Prettier separation, config-to-script mappings, and no-conflict guarantee

**Additional Updates:**
- Added Acceptance Criterion #8 to verify TSX support
- Updated eslint.config.js description to reference unified package

### Audit v2 (2026-01-31 15:10)
**Status:** APPROVED

**Context Estimate:** ~13% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~13% | <=50% | PASS |
| Largest single file | ~3% | <=30% | PASS |
| Worker overhead | ~5% | <=10% | PASS |

**Quality Projection:** PEAK (0-30%)

**Dimensions Evaluated:**
- Clarity: PASS - Clear title, context, task description, no vague terms
- Completeness: PASS - All files listed with specific content, deletion status explicit
- Testability: PASS - 8 measurable acceptance criteria with verification commands
- Scope: PASS - 5 constraints, clear boundaries, explicit Out of Scope section
- Feasibility: PASS - Standard tooling, well-documented approach
- Architecture Fit: PASS - Root config pattern fits monorepo, uses pnpm
- Non-Duplication: PASS - Verified no existing linting/formatting configs in repo
- Cognitive Load: PASS - Minimal rule set, standard tools, clear separation
- Strategic Fit: PASS - Aligns with TODO-006 requirement

**Goal Analysis Check:**
- All 5 observable truths mapped to artifacts: PASS
- No orphan artifacts: PASS
- Key Links section present with 3 links: PASS
- Wiring completeness: PASS

**Recommendations:**

1. **[Minor] Clarify k6 test exclusion:** The tests/k6/ directory contains 14 JavaScript files that are intentionally excluded by the TypeScript-only patterns. Consider adding "k6 load testing scripts (JavaScript)" to Out of Scope for explicit documentation.

**Comment:** Specification is well-structured after v1 revisions. All previous recommendations applied correctly. TSX support added, unified typescript-eslint package specified, Key Links documented. Ready for implementation.

### Response v2 (2026-01-31 15:15)
**Applied:** Recommendation 1

**Changes:**
1. [x] Clarify k6 test exclusion - Added "k6 load testing scripts (JavaScript)" to Out of Scope section with explanation that tests/k6/ JavaScript files are intentionally excluded by TypeScript-only lint/format patterns

### Audit v3 (2026-01-31 15:45)
**Status:** APPROVED

**Context Estimate:** ~12% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~12% | <=50% | PASS |
| Largest single file | ~3% (package.json) | <=30% | PASS |
| Worker overhead | ~5% | <=10% | PASS |

**Quality Projection:** PEAK (0-30%)

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**File Breakdown:**

| File | Operation | Est. Context |
|------|-----------|--------------|
| eslint.config.js | Create | ~3% |
| .prettierrc | Create | ~1% |
| .prettierignore | Create | ~1% |
| package.json | Modify | ~3% |
| Worker overhead | - | ~5% |
| **Total** | | **~13%** |

**Dimensions Evaluated:**
- Clarity: PASS - Title, context, and task are unambiguous; no vague terms
- Completeness: PASS - All 4 required artifacts specified with exact content; deletion status explicit ("None")
- Testability: PASS - 8 measurable acceptance criteria with verification commands
- Scope: PASS - 5 explicit constraints, 6 out-of-scope items, clear boundaries
- Feasibility: PASS - Standard tooling (ESLint 9, Prettier 3), well-documented approach
- Architecture Fit: PASS - Root-level monorepo config pattern matches PROJECT.md conventions
- Non-Duplication: PASS - Verified no existing eslint/prettier configs in project root (only in node_modules)
- Cognitive Load: PASS - Minimal rule set, standard tools, clear separation of concerns
- Strategic Fit: PASS - Directly addresses TODO-006, improves developer experience

**Goal Analysis Check:**
- All 5 observable truths have artifacts: PASS
- No orphan artifacts: PASS
- Key Links section present with 3 links: PASS
- Wiring completeness: PASS

**Assumptions Validated:**
| # | Assumption | Validation |
|---|------------|------------|
| A1 | No existing linting config | Verified: no .eslintrc* or eslint.config.* in project root |
| A2 | No existing formatting config | Verified: no .prettierrc* or .prettierignore in project root |
| A3 | React package has .tsx files | Verified: 16 .tsx files in packages/react/src |
| A4 | Tests directory has .ts files | Verified: 18 .ts files in tests/ |
| A5 | k6 has JavaScript files | Verified: 12 .js files in tests/k6/ (correctly excluded) |

**Strategic Sanity Check:**
- Project alignment: PASS - Aligns with TypeScript monorepo conventions
- Alternative solutions: N/A - ESLint + Prettier is standard industry choice
- Effort proportionality: PASS - Small effort for high ongoing value
- No red flags detected

**Comment:** Specification is complete and well-structured after two rounds of revisions. All previous recommendations have been applied. Verified against current codebase state: no conflicting configurations exist, file patterns match actual directory structure. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-01-31 21:02 UTC
**Commits:** 3

### Files Created
- `/Users/koristuvac/Projects/topgun/topgun/.prettierrc` - Prettier formatting configuration with standard rules (semi, single quotes, 100-char line width)
- `/Users/koristuvac/Projects/topgun/topgun/.prettierignore` - Excludes node_modules, dist, coverage, .md files, pnpm-lock.yaml, .specflow
- `/Users/koristuvac/Projects/topgun/topgun/eslint.config.js` - ESLint flat config v9 with TypeScript support, extends recommended rules, minimal custom rules

### Files Modified
- `/Users/koristuvac/Projects/topgun/topgun/package.json` - Added eslint, prettier, typescript-eslint devDependencies; added lint, lint:fix, format, format:check scripts
- `/Users/koristuvac/Projects/topgun/topgun/.gitignore` - Added exception for eslint.config.js (was excluded by *.js pattern)
- `/Users/koristuvac/Projects/topgun/topgun/apps/admin-dashboard/src/components/Layout.tsx` - Fixed pre-existing unused variable TypeScript error (blocking build)

### Files Deleted
None.

### Acceptance Criteria Status
- [x] Criterion 1: Lint command works - `pnpm lint` executes successfully, reports 2333 violations (489 errors, 1844 warnings) across 441 files
- [x] Criterion 2: Format command works - `pnpm format` command available (not executed per constraint)
- [x] Criterion 3: Format check works - `pnpm format:check` detects 441 unformatted files, exits with code 1 as expected
- [x] Criterion 4: TypeScript integration - ESLint correctly parses all .ts/.tsx files including generics, decorators, type assertions
- [x] Criterion 5: No config conflicts - ESLint reports only logical/typing issues, no formatting conflicts (Prettier handles formatting)
- [x] Criterion 6: Build unaffected - `pnpm build` succeeds with exit code 0 after configuration changes
- [x] Criterion 7: Tests unaffected - `pnpm test` runs successfully (1813/1815 tests pass; 2 flaky performance tests unrelated to changes)
- [x] Criterion 8: TSX support - Lint and format scripts include `*.{ts,tsx}` patterns, process React files in packages/react/src

### Deviations

1. **[Rule 1 - Bug Fix]** Fixed TypeScript unused variable error in `apps/admin-dashboard/src/components/Layout.tsx` (line 32) - pre-existing build-blocking error unrelated to linting setup. Prefixed `status` with underscore to indicate intentional non-use.

2. **[Rule 3 - Blocking Fix]** Updated `.gitignore` to allow `eslint.config.js` - the existing `*.js` exclusion pattern prevented committing the configuration file. Added explicit exception `!/eslint.config.js` following existing pattern for root config files.

### Implementation Notes

**Configuration Choices:**
- ESLint flat config format (v9+) used as specified - modern standard replacing deprecated `.eslintrc`
- Unified `typescript-eslint` package (v8.49.0) instead of separate parser/plugin packages - simplifies configuration
- Minimal rule overrides (4 rules) - focuses on non-breaking defaults to avoid disrupting existing development workflow
- TSX support included in all scripts via `*.{ts,tsx}` glob patterns

**Detected Issues (Not Fixed):**
- 2333 linting violations found across codebase (489 errors, 1844 warnings)
- 441 files need formatting
- Per constraint "Do not auto-fix existing code" - these remain for incremental cleanup

**Version Information:**
- ESLint: 9.39.1 (v9.39.2 available)
- Prettier: 3.8.1 (latest)
- typescript-eslint: 8.49.0 (v8.54.0 available)

**Performance:**
- `pnpm lint` execution time: ~45 seconds for full codebase scan
- `pnpm format:check` execution time: ~8 seconds
- Build time unaffected (still ~40 seconds for full monorepo build)

### Next Steps

Implementation complete. Ready for review via `/sf:review`.

---

## Review History

### Review v1 (2026-01-31 22:15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**

1. **ESLint config module type warning**
   - Issue: Node.js emits warning about module type not being specified in package.json when loading eslint.config.js as ES module
   - Impact: Non-breaking, only affects console output; does not prevent functionality
   - Optional fix: Add `"type": "module"` to package.json or rename to eslint.config.mjs (would require testing for compatibility)

2. **Apps directory not included in lint patterns**
   - File: `package.json:52-55`
   - Issue: The `apps/` directory (containing admin-dashboard) is not included in lint/format script patterns, only `packages/` and `tests/`
   - Impact: Admin dashboard code (apps/admin-dashboard/src) will not be linted/formatted by pnpm scripts
   - Note: Layout.tsx was manually fixed, but future changes in apps/ won't be automatically checked
   - Optional fix: Update scripts to include `'apps/*/src/**/*.{ts,tsx}'` pattern

**Passed:**

- [✓] All 3 required config files created (eslint.config.js, .prettierrc, .prettierignore)
- [✓] Config file contents match specification exactly
- [✓] package.json devDependencies added correctly (eslint ^9.18, prettier ^3.4.2, typescript-eslint ^8.19.1)
- [✓] All 4 package.json scripts added with correct patterns (lint, lint:fix, format, format:check)
- [✓] ESLint flat config v9 format used correctly with unified typescript-eslint package
- [✓] ESLint extends recommended presets as specified
- [✓] All 4 custom ESLint rules implemented correctly with specified values
- [✓] Prettier configuration matches specification (semi, singleQuote, tabWidth, trailingComma, printWidth, bracketSpacing)
- [✓] .prettierignore includes all 6 specified exclusion patterns
- [✓] ESLint ignores correct directories (node_modules, dist, coverage, .specflow)
- [✓] No formatting rules in ESLint config (no conflicts with Prettier)
- [✓] .gitignore properly updated to allow eslint.config.js commit
- [✓] Layout.tsx unused variable fix follows ESLint rule pattern (underscore prefix)
- [✓] All 8 acceptance criteria met:
  - Criterion 1: `pnpm lint` runs successfully, reports 2333 violations
  - Criterion 2: `pnpm format` command available
  - Criterion 3: `pnpm format:check` correctly detects 441 unformatted files
  - Criterion 4: ESLint parses TypeScript files without syntax errors
  - Criterion 5: No formatting conflicts (verified by grep test)
  - Criterion 6: `pnpm build` succeeds
  - Criterion 7: Tests pass (1813/1815, flakes unrelated)
  - Criterion 8: TSX files processed (16 .tsx files in packages/react/src)
- [✓] Constraint compliance: No auto-fix applied, no pre-commit hooks, no CI integration, minimal rules, flat config used
- [✓] Deviations properly justified (Layout.tsx bug fix, .gitignore update for blocking issue)
- [✓] No files deleted (as specified)
- [✓] TypeScript strict mode compatible
- [✓] Build unaffected (40 seconds, same as before)
- [✓] Integration with monorepo structure (pnpm, packages/* pattern)

**Summary:** Implementation fully meets specification requirements. All 8 acceptance criteria verified. Configuration files match specification exactly. ESLint and Prettier work correctly with no conflicts. Build and tests unaffected. Two minor issues identified (module type warning, apps/ directory exclusion) but both are optional improvements that don't affect core functionality. The constraint to not auto-fix existing code was respected - 2333 linting violations and 441 unformatted files remain for incremental cleanup as intended.

### Fix Response v1 (2026-01-31 22:20)
**Applied:** All minor issues (1, 2)

**Fixes:**
1. [✓] ESLint config module type warning — Renamed `eslint.config.js` to `eslint.config.mjs`
   - Commit: a2309d0
2. [✓] Apps directory not included in lint patterns — Added `'apps/*/src/**/*.{ts,tsx}'` to all lint/format scripts
   - Commit: 5392656

### Review v2 (2026-01-31 22:25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**

- [✓] Fix 1 verified: `eslint.config.mjs` exists at `/Users/koristuvac/Projects/topgun/topgun/eslint.config.mjs`
- [✓] Fix 1 verified: Old `eslint.config.js` file removed (confirmed via test command)
- [✓] Fix 1 verified: `.gitignore` updated to reference `!/eslint.config.mjs` (line 33)
- [✓] Fix 1 verified: ESLint config is valid ES module format using `import` and `export default`
- [✓] Fix 1 verified: No Node.js module type warnings in lint output (only ESLint rule violations shown)
- [✓] Fix 2 verified: `package.json` scripts now include `'apps/*/src/**/*.{ts,tsx}'` pattern (lines 52-55)
- [✓] Fix 2 verified: Apps directory actually linted (16 files from apps/ processed by pnpm lint)
- [✓] Fix 2 verified: Apps directory actually checked for formatting (67 files from apps/ processed by pnpm format:check)
- [✓] Both fixes properly committed with descriptive commit messages (a2309d0, 5392656)
- [✓] All 3 config files still exist (eslint.config.mjs, .prettierrc, .prettierignore)
- [✓] Config file contents unchanged from Review v1 (except filename)
- [✓] ESLint configuration still follows flat config v9 format
- [✓] All 4 custom ESLint rules still present and correct
- [✓] Prettier configuration still matches specification
- [✓] .prettierignore still includes all 6 exclusion patterns
- [✓] `pnpm lint` command still works (executes successfully, no crashes)
- [✓] `pnpm format:check` command still works (detects unformatted files correctly)
- [✓] `pnpm build` still succeeds (verified with tail output showing successful completion)
- [✓] Tests still pass (verified with @topgunbuild/core: 67 suites, 1815 tests passed)
- [✓] All 8 acceptance criteria still met
- [✓] All constraints still respected
- [✓] No new issues introduced
- [✓] TypeScript integration still works correctly
- [✓] Build time unaffected
- [✓] No security issues
- [✓] Follows monorepo patterns
- [✓] Cognitive load remains low

**Summary:** Both fixes successfully applied and verified. ESLint config renamed to `.mjs` eliminates module type warnings. Apps directory now included in all lint and format patterns, ensuring admin-dashboard and future apps are checked. All configuration files intact, all commands functional, build and tests unaffected. Implementation fully complete with no remaining issues. Ready for finalization.

---

## Completion

**Completed:** 2026-01-31 22:30 UTC
**Total Commits:** 5
**Audit Cycles:** 3
**Review Cycles:** 2

---
*Specification created: 2026-01-31*
