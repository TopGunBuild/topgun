---
id: SPEC-083c
type: feature
status: done
priority: P2
complexity: small
created: 2026-03-08
parent: SPEC-083
depends_on: []
---

# Sync-Lab Session Isolation and Example Cleanup

## Context

The `sync-lab` demo needs session isolation before it can be safely embedded on the docs homepage -- currently all visitors share the same global state. The `examples/collaborative-tasks/` app is being replaced by inline code snippets in the adoption guide (SPEC-083b) and should be deleted.

**Parent:** SPEC-083 (Update Documentation Content for Rust Server)
**Source TODO:** TODO-106

## Task

Session-isolate the sync-lab demo with per-tab namespaces and delete the `examples/collaborative-tasks/` directory.

## Requirements

### R9: Delete `examples/collaborative-tasks/`

**Deletion:** `examples/collaborative-tasks/` (entire directory)

- Remove from pnpm workspace if referenced (workspace uses `examples/*` glob, so no config change needed unless explicitly listed)
- Remove any references to it in docs
- The Tier 1 snippet in `guides/adoption-path.mdx` (SPEC-083b) replaces this

### R11: Session-Isolate `sync-lab` for Homepage Embedding

**Files:**
- `examples/sync-lab/src/hooks/useDeviceClient.ts`
- `examples/sync-lab/src/App.tsx`
- `examples/sync-lab/src/lib/device-manager.ts`
- `examples/sync-lab/src/components/LatencyRace.tsx`

Changes:
- Generate a unique `sessionId` per browser tab (e.g., first 8 characters of `crypto.randomUUID()`, stored in `sessionStorage`)
- Prefix all map names with `sl-${sessionId}:` (e.g., `sl-a1b2c3d4:sync-lab-todos`, `sl-a1b2c3d4:latency-bench`)
- Dual-device simulation (Conflict Arena) continues to work because both panels share the same `sessionId`
- No shared public state between different visitors
- Add optional "Share session" button in `App.tsx` (header area) that copies a URL with `?session=<sessionId>` to the clipboard for cross-device demo (conscious opt-in)
- When `?session=` query param is present, use that sessionId instead of generating a new one

## Acceptance Criteria

1. `examples/collaborative-tasks/` directory does not exist
2. No docs page links to or references `examples/collaborative-tasks/`
3. `sync-lab` generates per-session namespace prefixes; two tabs produce independent state
4. `sync-lab` "Share session" button generates URL with `?session=` parameter
5. `pnpm start:docs` builds without errors (no broken imports/references from deleted example)

## Constraints

- Do NOT add `sync-lab` server-side session management -- session isolation is purely client-side via `sessionStorage`
- `pnpm-workspace.yaml` uses `examples/*` glob, so deleting the directory is sufficient (no workspace config change needed)

## Assumptions

- The `pnpm-workspace.yaml` references `examples/*` as a glob pattern, not individual example directories.
- No other package in the monorepo has a hard dependency on `examples/collaborative-tasks/`.

## Audit History

### Audit v1 (2026-03-09)
**Status:** NEEDS_REVISION

**Context Estimate:** ~15% total

**Critical:**
1. Missing file in R11: `examples/sync-lab/src/components/LatencyRace.tsx` hardcodes map name `latency-bench` (line 53) which also needs session prefixing. Without this, LatencyRace tab data leaks across sessions, violating AC3. Add this file to the R11 file list.

**Recommendations:**
2. The example map names in R11 (`sync-lab-abc123:counter`, `sync-lab-abc123:conflict-arena`) do not match actual codebase map names (`sync-lab-todos` in useDeviceClient.ts, `latency-bench` in LatencyRace.tsx). Update examples to reflect real names (e.g., `sync-lab-abc123:sync-lab-todos`, `sync-lab-abc123:latency-bench`) to avoid implementer confusion.
3. The "Share session" button placement is unspecified. Clarify which component file should contain it -- likely `App.tsx` (in the header area) or a new component. This helps scope the work precisely.
4. [Strategic] Consider whether the sessionId prefix format `sync-lab-${sessionId}:mapName` creates excessively long map names on the server. A shorter prefix (e.g., first 8 chars of UUID) would reduce storage overhead while still being collision-resistant for a demo.

### Response v1 (2026-03-09)
**Applied:** All critical issues and recommendations (items 1-4)

**Changes:**
1. [✓] Added `examples/sync-lab/src/components/LatencyRace.tsx` to R11 file list
2. [✓] Updated example map names to reflect real codebase names (`sl-a1b2c3d4:sync-lab-todos`, `sl-a1b2c3d4:latency-bench`)
3. [✓] Specified "Share session" button placement in `App.tsx` (header area) with clipboard-copy behavior
4. [✓] Shortened prefix format from `sync-lab-${sessionId}:` to `sl-${sessionId}:` using first 8 chars of UUID (e.g., `sl-a1b2c3d4:`) for reduced storage overhead

### Audit v2 (2026-03-09)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Comment:** All v1 critical issues and recommendations have been addressed. The spec is clear, complete, and implementable. File list correctly covers all 4 files with hardcoded map names (verified against source). The `CodeSnippets.tsx` reference to `sync-lab-todos` is a display-only string in a code example, not live code, so it correctly does not need session prefixing. Both assumptions verified against the actual codebase (workspace glob confirmed, no external references to collaborative-tasks found). Strategic fit is good -- session isolation is a prerequisite for homepage embedding. Project compliance verified -- this is a TypeScript example app, so the Rust Language Profile does not apply.

---

## Execution Summary

**Executed:** 2026-03-09
**Commits:** 2

### Files Created
- `examples/sync-lab/src/lib/session.ts` -- session ID management (getSessionId, prefixMap, getShareUrl)

### Files Modified
- `examples/sync-lab/src/hooks/useDeviceClient.ts` -- use prefixMap for MAP_NAME constant
- `examples/sync-lab/src/components/LatencyRace.tsx` -- use prefixMap for offline benchmark map name
- `examples/sync-lab/src/App.tsx` -- add "Share session" button in header, import getShareUrl

### Files Deleted
- `examples/collaborative-tasks/` -- entire directory (10 files), replaced by adoption guide snippets

### Acceptance Criteria Status
- [x] AC1: `examples/collaborative-tasks/` directory does not exist
- [x] AC2: No docs page links to or references `examples/collaborative-tasks/`
- [x] AC3: sync-lab generates per-session namespace prefixes; two tabs produce independent state (via sessionStorage-backed sessionId and prefixMap)
- [x] AC4: sync-lab "Share session" button generates URL with `?session=` parameter (copies to clipboard)
- [x] AC5: pnpm start:docs builds without errors (no broken imports/references from deleted example -- workspace uses glob pattern)

### Deviations
- `examples/sync-lab/src/lib/device-manager.ts` was listed in the spec but did not require changes -- it already receives mapName as a parameter from callers, so prefixing at the call site (useDeviceClient.ts) is sufficient.

### Notes
- Session logic extracted into a dedicated `session.ts` module rather than inlining into each file, reducing duplication and making the session ID consistent across all components.
- The `?session=` param is persisted into sessionStorage on first read, so subsequent navigations within the same tab maintain the shared session.

---

## Review History

### Review v1 (2026-03-09)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. Stale lockfile entry
   - File: `pnpm-lock.yaml:263`
   - Issue: `pnpm-lock.yaml` still contains a dependency entry for `examples/collaborative-tasks` which no longer exists. While harmless, it is stale metadata that will persist until the next `pnpm install`.
   - Fix: Run `pnpm install` to regenerate the lockfile without the deleted package.

2. Redundant URL parsing on every `prefixMap` call
   - File: `examples/sync-lab/src/lib/session.ts:8-27`
   - Issue: `getSessionId()` re-parses `URLSearchParams` from `window.location.search` on every invocation. Since `prefixMap` is called at module scope (e.g., `useDeviceClient.ts:14`), this only fires once per module load in practice, but if `prefixMap` were called in a render loop it would be wasteful. A module-level cache variable (set on first call) would be more defensive.
   - Fix: Cache the resolved sessionId in a module-level `let` variable after first computation.

**Passed:**
- [✓] AC1: `examples/collaborative-tasks/` directory confirmed deleted
- [✓] AC2: No docs pages reference `collaborative-tasks` (only stale lockfile entry, not a docs reference)
- [✓] AC3: Session isolation correctly implemented -- `session.ts` generates per-tab UUID (8 chars), `prefixMap()` applied to both `sync-lab-todos` and `latency-bench` map names, `sessionStorage` provides per-tab isolation
- [✓] AC4: "Share session" button in `App.tsx` header copies URL with `?session=` param to clipboard, with "Copied!" feedback state
- [✓] AC5: No broken imports detected -- workspace uses `examples/*` glob, all sync-lab imports resolve correctly
- [✓] Constraint respected: No server-side session management -- purely `sessionStorage` + URL param
- [✓] `device-manager.ts` correctly unchanged -- receives `mapName` as parameter, prefixing at call sites is the right approach
- [✓] `CodeSnippets.tsx` correctly unchanged -- display-only code string, not live map access
- [✓] Dual-device simulation (Conflict Arena) works because both panels share the same tab's `sessionId`
- [✓] `?session=` param persisted to `sessionStorage` so shared sessions survive navigation within the same tab
- [✓] Code quality: Clean extraction into `session.ts` module with clear JSDoc comments, good separation of concerns
- [✓] Security: No hardcoded secrets (demo JWT is intentionally public for test server), no XSS vectors, session IDs are random UUIDs
- [✓] Architecture: Fits naturally with existing React patterns, proper use of `useCallback` and state management in App.tsx

**Summary:** Implementation is clean, complete, and meets all 5 acceptance criteria. The session isolation approach is well-architected with a dedicated `session.ts` module that centralizes session ID management. The deviation from spec (not modifying `device-manager.ts`) is correct and well-reasoned. Two minor issues identified (stale lockfile, redundant URL parsing) -- neither affects functionality.

### Fix Response v1 (2026-03-09)
**Applied:** All minor issues (1, 2)

**Fixes:**
1. [✓] Stale lockfile entry — ran `pnpm install` to regenerate lockfile without `examples/collaborative-tasks`
   - Commit: 6e83629
2. [✓] Redundant URL parsing — added `cachedSessionId` module-level variable in `session.ts`, set on first call to `getSessionId()`
   - Commit: 6e83629

### Review v2 (2026-03-09)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] Fix #1 verified: `pnpm-lock.yaml` no longer contains any reference to `collaborative-tasks` -- grep across entire repo returns zero matches
- [✓] Fix #2 verified: `session.ts:3` declares `let cachedSessionId: string | null = null;` with early return on line 12-14; all three code paths (URL param, sessionStorage, new generation) set the cache before returning
- [✓] AC1: `examples/collaborative-tasks/` directory confirmed deleted (filesystem check)
- [✓] AC2: Zero references to `collaborative-tasks` anywhere in the repository (full repo grep)
- [✓] AC3: Session isolation via `session.ts` with `prefixMap()` applied to both map names (`sync-lab-todos` at `useDeviceClient.ts:14`, `latency-bench` at `LatencyRace.tsx:54`)
- [✓] AC4: "Share session" button in `App.tsx:89-94` header area, with `handleShareSession` callback using `navigator.clipboard.writeText` and "Copied!" feedback
- [✓] AC5: No broken imports -- all sync-lab imports resolve correctly
- [✓] Constraint: Purely client-side session isolation via `sessionStorage` + URL param, no server-side management
- [✓] Code quality: `cachedSessionId` caching is correctly implemented with module-level `let` variable

**Summary:** Both minor fixes from Review v1 have been correctly applied. The stale lockfile entry is fully cleaned and the session ID caching eliminates redundant URL parsing. All 5 acceptance criteria remain satisfied. No new issues found. Ready for finalization.

---

## Completion

**Completed:** 2026-03-09
**Total Commits:** 4
**Review Cycles:** 2

### Outcome

Session-isolated the sync-lab demo with per-tab namespaces (`sl-{8chars}:` prefix) and deleted the `examples/collaborative-tasks/` directory. The sync-lab is now safe for homepage embedding without cross-visitor state leakage.

### Key Files

- `examples/sync-lab/src/lib/session.ts` — centralized session ID management (getSessionId, prefixMap, getShareUrl) with module-level caching

### Patterns Established

None — followed existing patterns.

### Deviations

- `device-manager.ts` was listed in spec but did not require changes — it already receives mapName as a parameter, so prefixing at call sites was sufficient.
