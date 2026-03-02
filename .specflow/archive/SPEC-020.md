> **SPLIT:** This specification was decomposed into:
> - SPEC-020a: Clean packages/core/ comments (medium, 53 files, 158 refs)
> - SPEC-020b: Clean packages/server/ comments (medium, 105 files, 196 refs)
> - SPEC-020c: Clean packages/client/ comments (medium, 24 files, 100 refs)
> - SPEC-020d: Clean packages/react/ + mcp-server/ + adapter-better-auth/ comments (small, 8 files, 33 refs)
>
> See child specifications for implementation.

# SPEC-020: Remove Phase/Spec/Bug References from Code Comments

---
id: SPEC-020
type: refactor
status: split
priority: medium
complexity: large
created: 2026-02-01
---

## Context

The codebase contains 488 references to internal process artifacts (Phase X, BUG-XX, SPEC-XXX) scattered across 191 files in packages/. These are temporary development tracking markers that should live in commit messages and project management tools, not in production code.

A new convention has been added to CLAUDE.md and PROJECT.md explicitly prohibiting such references:
> No phase/spec/bug references in code comments - use WHY-comments instead

**Examples found:**
- `// Merge topic queue config with defaults (Phase 3 BUG-06)` - process tracking, not context
- `// Adaptive indexing with query pattern tracking (Phase 8.02)` - phase number adds no value
- `// Phase 14.1: Pagination state` - redundant with the code itself

These markers clutter the code and become meaningless as the project evolves. They should be removed or replaced with meaningful WHY-comments that explain the purpose or rationale.

## Goal Analysis

### Goal Statement
Remove all internal process artifacts (Phase X, BUG-XX, SPEC-XXX) from code comments in packages/, replacing them with meaningful context where needed.

### Observable Truths
1. `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/` returns zero matches
2. Comments that provided useful context (beyond the phase/spec reference) are preserved
3. All tests pass: `pnpm test` exits 0
4. Build succeeds: `pnpm build` exits 0
5. No functional code changes - only comment modifications

### Required Artifacts
- 191 files across packages/ require modification (comment-only changes)

### Key Links
- This is a comment-only refactor; no code logic is affected
- Risk: accidentally removing comments that had useful context beyond the process reference

## Task

Remove or rewrite all process artifact references in code comments:

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

### Files to Modify

Total: 191 files across 7 packages with references:

| Package | Files | Total References |
|---------|-------|------------------|
| packages/core/ | 45 files | ~120 references |
| packages/server/ | 85 files | ~260 references |
| packages/client/ | 35 files | ~85 references |
| packages/react/ | 8 files | ~30 references |
| packages/mcp-server/ | 2 files | ~3 references |
| packages/adapter-better-auth/ | 1 file | 1 reference |
| packages/native/ | 1 file | 1 reference (in package-lock.json - false positive) |

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

### Deletions

None - this is a modification-only task.

## Acceptance Criteria

1. [ ] Zero matches for `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/` (excluding false positives in lock files)
2. [ ] All tests pass: `pnpm test` exits 0
3. [ ] Build succeeds: `pnpm build` exits 0
4. [ ] No functional code changes (diff shows only comment modifications)
5. [ ] Comments with meaningful context are preserved (not blindly deleted)
6. [ ] Index.ts export comments retain their grouping context (e.g., "Cluster exports" not "Cluster exports (Phase 4)")

## Constraints

1. **DO NOT** modify any executable code - comments only
2. **DO NOT** add new comments - only remove or simplify existing ones
3. **DO NOT** modify files outside packages/ (e.g., .specflow/, CLAUDE.md already updated)
4. **DO NOT** remove comments that explain WHY something is done a certain way
5. **DO NOT** modify package-lock.json files (false positives from "debug" package version)
6. **PRESERVE** comment structure (if a section header exists, keep it as a section header)

## Verification Commands

```bash
# Find all remaining references (should return nothing except lock files)
grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/ --include="*.ts" --include="*.tsx" --include="*.sql" --include="*.md"

# Verify tests pass
pnpm test

# Verify build succeeds
pnpm build

# Count modified files (should be ~190)
git diff --stat | tail -1
```

## Assumptions

1. **Pattern is consistent** - Assumed all process references follow the patterns: `Phase X`, `Phase X.Y`, `BUG-XX`, `SPEC-XXX`, `SPEC-XXXx`
2. **Lock files are false positives** - The match in packages/native/package-lock.json is from the "debug" npm package version, not a process reference
3. **No semantic meaning** - Phase numbers do not carry semantic meaning that needs to be preserved in comments
4. **Test files included** - Process references in test files should also be cleaned (test names, comments)
5. **Benchmark files included** - Process references in __bench__ directories should also be cleaned
6. **README files in packages/** - Benchmark README.md files with phase references should be cleaned

## Complexity Notes

This is marked as **large** due to:
- 191 files requiring modification
- 488 individual occurrences to review
- Need for human judgment on which comments to keep vs remove
- Spread across all 7 packages

**Recommendation:** Consider using `/sf:split SPEC-020` to decompose into per-package sub-specs:
- SPEC-020a: packages/core/
- SPEC-020b: packages/server/
- SPEC-020c: packages/client/
- SPEC-020d: packages/react/ + packages/mcp-server/ + packages/adapter-better-auth/ (small, combined)
