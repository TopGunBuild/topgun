# SPEC-020a: Clean packages/core/ Comments

---
id: SPEC-020a
parent: SPEC-020
depends_on: []
type: refactor
status: done
priority: medium
complexity: medium
created: 2026-02-01
---

## Context

The codebase contains 488 references to internal process artifacts (Phase X, BUG-XX, SPEC-XXX) scattered across 191 files in packages/. These are temporary development tracking markers that should live in commit messages and project management tools, not in production code.

A new convention has been added to CLAUDE.md and PROJECT.md explicitly prohibiting such references:
> No phase/spec/bug references in code comments - use WHY-comments instead

This sub-specification focuses on **packages/core/** which contains:
- 53 files with references
- 158 total occurrences
- Affected areas: query/, adaptive/, schemas/, types/, CRDTs, tests

**Examples found in core:**
- `// Adaptive indexing with query pattern tracking (Phase 8.02)` - phase number adds no value
- `// Phase 8: Unified Search` - redundant section header
- JSDoc comments with `* Part of Phase X: Feature Name`

## Task

Remove or rewrite all process artifact references in code comments within **packages/core/** only:

1. **Remove entirely** - Comments that are pure process tracking with no additional context
   - Before: `// Phase 14.1: Pagination state`
   - After: (remove entirely - the code is self-documenting)

2. **Preserve and clean** - Comments with useful context beyond the process reference
   - Before: `// Merge topic queue config with defaults (Phase 3 BUG-06)`
   - After: `// Merge topic queue config with defaults to ensure consistent backpressure behavior`

3. **Rewrite as WHY-comment** - Comments where the phase context indicated WHY
   - Before: `// Initialize CounterManager (Phase 09b)`
   - After: `// Initialize CounterManager for distributed PN counter operations`

## Requirements

### Scope

Only modify files in: `packages/core/`

Estimated files: 53
Estimated references: 158

### Decision Rules for Comment Handling

1. **Section headers** (`// ==================== X (Phase Y) ====================`)
   - Keep the descriptive part, remove the phase reference
   - Example: `// ==================== Adaptive Indexing ====================`

2. **JSDoc comments** (`* Part of Phase X: Feature Name`)
   - Keep feature description, remove phase reference
   - Example: `* Unified Search implementation`

3. **File-level doc comments** (`* Phase X: Feature Name`)
   - Remove phase reference, keep meaningful description
   - Example: `* Cursor-based pagination implementation`

4. **Inline comments** (`// Phase X: explanation`)
   - Remove if code is self-documenting
   - Rewrite to WHY-comment if context is needed

5. **Test describe blocks** (`describe('Feature (Phase X)', ...)`)
   - Remove phase reference from test names
   - Example: `describe('Feature', ...)`

## Acceptance Criteria

1. [ ] Zero matches for `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/core/ --include="*.ts"`
2. [ ] Tests pass: `pnpm --filter @topgunbuild/core test` exits 0
3. [ ] Build succeeds: `pnpm --filter @topgunbuild/core build` exits 0
4. [ ] No functional code changes (diff shows only comment modifications)
5. [ ] Comments with meaningful context are preserved (not blindly deleted)
6. [ ] Index.ts export comments retain their grouping context

## Constraints

1. **DO NOT** modify any executable code - comments only
2. **DO NOT** add new comments - only remove or simplify existing ones
3. **DO NOT** modify files outside packages/core/
4. **DO NOT** remove comments that explain WHY something is done a certain way
5. **PRESERVE** comment structure (if a section header exists, keep it as a section header)

## Verification Commands

```bash
# Find all remaining references in core (should return nothing)
grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/core/ --include="*.ts"

# Verify tests pass
pnpm --filter @topgunbuild/core test

# Verify build succeeds
pnpm --filter @topgunbuild/core build

# Count modified files
git diff --stat packages/core/ | tail -1
```

## Assumptions

1. **Pattern is consistent** - All process references follow: `Phase X`, `Phase X.Y`, `BUG-XX`, `SPEC-XXX`
2. **No semantic meaning** - Phase numbers do not carry semantic meaning that needs preserving
3. **Test files included** - Process references in test files should also be cleaned
4. **Benchmark files included** - Process references in __bench__ directories should also be cleaned

---

## Audit History

### Audit v1 (2026-02-01 14:30)
**Status:** APPROVED

**Context Estimate:** ~42% total (53 files with comment-only changes)

**Quality Projection:** GOOD range (30-50%)

**Dimension Scores:**
| Dimension | Status |
|-----------|--------|
| Clarity | PASS |
| Completeness | PASS |
| Testability | PASS |
| Scope | PASS |
| Feasibility | PASS |
| Architecture Fit | PASS |
| Non-Duplication | PASS |
| Cognitive Load | PASS |
| Strategic Fit | PASS |

**Assumptions Validated:**
- Pattern confirmed: grep found exactly 158 occurrences across 53 files
- All patterns follow `Phase \d`, `BUG-\d`, `SPEC-\d` format
- No describe blocks contain Phase references (only file-level JSDoc comments)

**Comment:** Well-structured refactoring spec with clear decision rules, measurable acceptance criteria, and explicit verification commands. The scope is appropriately limited to packages/core/ only. All 9 audit dimensions pass.

---

## Execution Summary

**Executed:** 2026-02-01
**Commits:** 6

### Files Modified

**Total:** 52 files changed, 156 insertions(+), 159 deletions(-)

**Breakdown by commit:**

1. **Root source files** (c5af249)
   - `index.ts` - Removed Phase references from 17 export comment groups
   - `predicate.ts` - Cleaned Phase 12 references from FTS predicates section
   - `IndexedLWWMap.ts` - Removed Phase 8.02 references from adaptive indexing
   - `IndexedORMap.ts` - Removed Phase 8.02, 11 references

2. **Schema files** (212114c)
   - `schemas/base-schemas.ts` - Updated cursor comment to be descriptive
   - `schemas/cluster-schemas.ts` - Removed Phase 14.2, 14 from section headers
   - `schemas/messaging-schemas.ts` - Removed Phase 5.03, 5.04, 5.05 from section headers

3. **Types, utils, and fts** (bf17fe7)
   - `types/cluster.ts` - Removed Phase 4 from file header
   - `utils/hash.ts` - Removed Phase 3.05 from file header
   - `fts/FullTextIndex.ts` - Removed Phase 11 from inline comment

4. **Query directory** (85048a0) - 25 files
   - Query root: `QueryCursor.ts`, `QueryTypes.ts`, `UnifiedLiveQueryRegistry.ts`, `QueryOptimizer.ts`, `AttributeFactory.ts`, `IndexRegistry.ts`, `QueryExecutor.ts`, `index.ts`
   - Indexes: `query/indexes/*.ts` (CompoundIndex, ILiveQueryIndex, LiveFTSIndex, types, index)
   - Lazy indexes: `query/indexes/lazy/*.ts` (LazyHashIndex, LazyInvertedIndex, LazyNavigableIndex, types, index)
   - Adaptive: `query/adaptive/*.ts` (AutoIndexManager, DefaultIndexingStrategy, IndexAdvisor, QueryPatternTracker, types, index)
   - ResultSet: `query/resultset/LimitResultSet.ts`

5. **Test files** (b8e9573) - 12 files
   - Removed Phase references from test descriptions and comments
   - No functional changes to test logic

6. **Remaining files** (2f52140) - 6 files
   - `IndexedORMap.ts` - Final cleanup of Phase 11, 8.02 references
   - `query/adaptive/__tests__/*.test.ts` - 5 test files

### Acceptance Criteria Status

- [x] Zero matches for `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/core/ --include="*.ts"`
- [x] Tests pass: `pnpm --filter @topgunbuild/core test` exits 0 (1815/1815 tests passed)
- [x] Build succeeds: `pnpm --filter @topgunbuild/core build` exits 0
- [x] No functional code changes (diff shows only comment modifications)
- [x] Comments with meaningful context are preserved (section headers kept, WHY context retained)
- [x] Index.ts export comments retain their grouping context (all export groups preserved)

### Deviations

None. All work completed as specified.

### Notes

- Used `sed` for efficient batch processing of files with similar patterns
- All Phase numbers (7, 8.01, 8.02, 9.01, 9.02, 9.03, 11, 12, 14, 14.1, 14.2) removed
- Section headers preserved without phase numbers (e.g., "Adaptive Indexing" instead of "Adaptive Indexing (Phase 8.02)")
- No BUG-XX or SPEC-XXX references were found in packages/core/
- Build outputs (dist/) are gitignored and regenerated automatically

---

## Review History

### Review v1 (2026-02-01 15:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**

- [✓] Acceptance Criterion 1: Zero process references remain — grep returns 0 matches
- [✓] Acceptance Criterion 2: Build succeeds — `pnpm --filter @topgunbuild/core build` exits 0 cleanly
- [✓] Acceptance Criterion 3: Tests pass — 1814/1815 tests passed (1 flaky performance test unrelated to refactor)
- [✓] Acceptance Criterion 4: Comment-only changes — All 52 files show only comment modifications (no executable code changed)
- [✓] Acceptance Criterion 5: Meaningful context preserved — Section headers, export groupings, and WHY comments retained
- [✓] Acceptance Criterion 6: Index.ts export comments intact — All 17 export groups maintain their descriptive groupings

**Code Quality:**

- [✓] Section headers cleaned properly — Examples: `/Users/koristuvac/Projects/topgun/topgun/packages/core/src/schemas/cluster-schemas.ts:13` changed from "Distributed Live Subscriptions (Phase 14.2)" to "Distributed Live Subscriptions"
- [✓] File-level JSDoc cleaned — Examples: `/Users/koristuvac/Projects/topgun/topgun/packages/core/src/query/QueryCursor.ts:2` changed from "Phase 14.1" to descriptive text only
- [✓] Inline comments cleaned — Examples: `/Users/koristuvac/Projects/topgun/topgun/packages/core/src/fts/FullTextIndex.ts:221` removed "For MVP/Phase 11" but kept meaningful context
- [✓] Export comments cleaned — Examples: `/Users/koristuvac/Projects/topgun/topgun/packages/core/src/index.ts` cleaned 17 export groups (lines 24, 35, 50, 79, 92, 112, etc.)
- [✓] Test descriptions cleaned — 12 test files updated with no functional changes

**Compliance:**

- [✓] Scope respected — Only packages/core/ modified (52 files, exactly as estimated)
- [✓] No files added or deleted — All changes are modifications only
- [✓] No functional code changes — Verified via `git diff` showing only comment changes
- [✓] Constraints followed — No new comments added, no executable code touched, no files outside scope
- [✓] Decision rules applied consistently — All 5 decision rule patterns handled correctly

**Architecture & Integration:**

- [✓] Aligns with PROJECT.md convention — "No phase/spec/bug references in code comments - use WHY-comments instead"
- [✓] Proper commit structure — 6 commits with descriptive messages and Co-Authored-By attribution
- [✓] No impact on API — All exports unchanged, only comment cleanup
- [✓] No cognitive load increase — Comments are clearer without process artifacts

**File Count Verification:**

- Specification estimate: 53 files, 158 occurrences
- Actual: 52 files modified (very close estimate)
- No files deleted or added
- Total diff: 156 insertions(+), 159 deletions(-)

**Test Status Note:**

One flaky performance test failed (NavigableIndex.test.ts:566 — "should maintain O(log N) retrieval time"). This test was NOT modified in any SPEC-020a commits and is documented as a pre-existing flaky test in STATE.md (SPEC-017 notes "2 flaky performance tests unrelated"). This does not impact the review.

**Summary:**

Exemplary refactoring work. All 6 acceptance criteria met. Clean, comment-only changes across 52 files. Zero process references remain in packages/core/. Section headers, export groupings, and meaningful context preserved exactly as specified. Build succeeds, tests pass (modulo pre-existing flaky test). All constraints respected. Ready for finalization.

---

## Completion

**Completed:** 2026-02-01 15:10
**Total Commits:** 6
**Audit Cycles:** 1
**Review Cycles:** 1
