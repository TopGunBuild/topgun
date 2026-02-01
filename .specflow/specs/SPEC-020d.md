# SPEC-020d: Clean packages/react/, mcp-server/, adapter-better-auth/ Comments

---
id: SPEC-020d
parent: SPEC-020
depends_on: []
type: refactor
status: draft
priority: medium
complexity: small
created: 2026-02-01
---

## Context

The codebase contains 488 references to internal process artifacts (Phase X, BUG-XX, SPEC-XXX) scattered across 191 files in packages/. These are temporary development tracking markers that should live in commit messages and project management tools, not in production code.

A new convention has been added to CLAUDE.md and PROJECT.md explicitly prohibiting such references:
> No phase/spec/bug references in code comments - use WHY-comments instead

This sub-specification focuses on the **smaller packages** combined:

| Package | Files | References |
|---------|-------|------------|
| packages/react/ | 5 | 29 |
| packages/mcp-server/ | 2 | 3 |
| packages/adapter-better-auth/ | 1 | 1 |
| **Total** | **8** | **33** |

**Examples found:**
- `// Phase 14: useQuery pagination support` in react hooks
- `// Phase 19: MCP integration` in mcp-server
- JSDoc with `* Part of Phase X: Feature`

## Task

Remove or rewrite all process artifact references in code comments within **packages/react/**, **packages/mcp-server/**, and **packages/adapter-better-auth/** only:

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

Only modify files in:
- `packages/react/`
- `packages/mcp-server/`
- `packages/adapter-better-auth/`

Estimated files: 8
Estimated references: 33

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

1. [ ] Zero matches for `grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/react/ packages/mcp-server/ packages/adapter-better-auth/ --include="*.ts" --include="*.tsx"`
2. [ ] Tests pass: `pnpm --filter @topgunbuild/react test` exits 0
3. [ ] Tests pass: `pnpm --filter @topgunbuild/mcp-server test` exits 0
4. [ ] Tests pass: `pnpm --filter @topgunbuild/adapter-better-auth test` exits 0
5. [ ] Build succeeds for all three packages
6. [ ] No functional code changes (diff shows only comment modifications)
7. [ ] Comments with meaningful context are preserved (not blindly deleted)

## Constraints

1. **DO NOT** modify any executable code - comments only
2. **DO NOT** add new comments - only remove or simplify existing ones
3. **DO NOT** modify files outside packages/react/, packages/mcp-server/, packages/adapter-better-auth/
4. **DO NOT** remove comments that explain WHY something is done a certain way
5. **PRESERVE** comment structure (if a section header exists, keep it as a section header)

## Verification Commands

```bash
# Find all remaining references (should return nothing)
grep -rE "(Phase \d|BUG-\d|SPEC-\d)" packages/react/ packages/mcp-server/ packages/adapter-better-auth/ --include="*.ts" --include="*.tsx"

# Verify tests pass
pnpm --filter @topgunbuild/react test
pnpm --filter @topgunbuild/mcp-server test
pnpm --filter @topgunbuild/adapter-better-auth test

# Verify builds succeed
pnpm --filter @topgunbuild/react build
pnpm --filter @topgunbuild/mcp-server build
pnpm --filter @topgunbuild/adapter-better-auth build

# Count modified files
git diff --stat packages/react/ packages/mcp-server/ packages/adapter-better-auth/ | tail -1
```

## Assumptions

1. **Pattern is consistent** - All process references follow: `Phase X`, `Phase X.Y`, `BUG-XX`, `SPEC-XXX`
2. **No semantic meaning** - Phase numbers do not carry semantic meaning that needs preserving
3. **Test files included** - Process references in test files should also be cleaned
