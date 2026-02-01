# SPEC-020c: Clean packages/client/ Comments

---
id: SPEC-020c
parent: SPEC-020
depends_on: []
type: refactor
status: draft
priority: medium
complexity: medium
created: 2026-02-01
---

## Context

The codebase contains 488 references to internal process artifacts (Phase X, BUG-XX, SPEC-XXX) scattered across 191 files in packages/. These are temporary development tracking markers that should live in commit messages and project management tools, not in production code.

A new convention has been added to CLAUDE.md and PROJECT.md explicitly prohibiting such references:
> No phase/spec/bug references in code comments - use WHY-comments instead

This sub-specification focuses on **packages/client/** which contains:
- 24 files with references
- 100 total occurrences
- Affected areas: sync/, cluster/, TopGunClient, tests

**Examples found in client:**
- `// Phase 5: Sync engine improvements` - redundant comment
- `// Initialize cluster support (Phase 4)` - phase adds no value
- JSDoc with `* Part of Phase X: Feature`

## Task

Remove or rewrite all process artifact references in code comments within **packages/client/** only:

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

Only modify files in: `packages/client/`

Estimated files: 24
Estimated references: 100

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

1. [ ] Zero matches for `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/client/ --include="*.ts"`
2. [ ] Tests pass: `pnpm --filter @topgunbuild/client test` exits 0
3. [ ] Build succeeds: `pnpm --filter @topgunbuild/client build` exits 0
4. [ ] No functional code changes (diff shows only comment modifications)
5. [ ] Comments with meaningful context are preserved (not blindly deleted)
6. [ ] Index.ts export comments retain their grouping context

## Constraints

1. **DO NOT** modify any executable code - comments only
2. **DO NOT** add new comments - only remove or simplify existing ones
3. **DO NOT** modify files outside packages/client/
4. **DO NOT** remove comments that explain WHY something is done a certain way
5. **PRESERVE** comment structure (if a section header exists, keep it as a section header)

## Verification Commands

```bash
# Find all remaining references in client (should return nothing)
grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/client/ --include="*.ts"

# Verify tests pass
pnpm --filter @topgunbuild/client test

# Verify build succeeds
pnpm --filter @topgunbuild/client build

# Count modified files
git diff --stat packages/client/ | tail -1
```

## Assumptions

1. **Pattern is consistent** - All process references follow: `Phase X`, `Phase X.Y`, `BUG-XX`, `SPEC-XXX`
2. **No semantic meaning** - Phase numbers do not carry semantic meaning that needs preserving
3. **Test files included** - Process references in test files should also be cleaned
