# SPEC-005: Remove Unused Artifacts from ServerCoordinator

---
id: SPEC-005
type: refactor
status: draft
priority: medium
complexity: small
created: 2026-01-28
---

## Context

After the SPEC-003/SPEC-004 refactoring series, ServerCoordinator contains residual artifacts from the previous architecture. Handler creation was moved to ServerFactory, but the corresponding class properties and imports remain in ServerCoordinator. These artifacts:

1. Increase cognitive load when reading the code
2. Suggest incorrect dependencies
3. Add memory overhead (property slots)
4. Violate YAGNI principle

**Prior work:** SPEC-003 series extracted handlers, SPEC-004 moved handler instantiation to ServerFactory.

## Task

Remove unused imports, properties, and convert constructor-only properties to local variables in ServerCoordinator.ts.

### Part 1: Remove Unused Imports (4 items)

Remove these imports that are no longer needed after handler creation moved to ServerFactory:

```typescript
// Line 42-44 - factory functions no longer called in ServerCoordinator
import { createDebugEndpoints, DebugEndpoints } from './debug';
import { BootstrapController, createBootstrapController } from './bootstrap';
import { SettingsController, createSettingsController } from './settings';

// Line 40 - SearchConfig type not used
import { SearchCoordinator, SearchConfig, ClusterSearchCoordinator, type ClusterSearchConfig } from './search';
```

**Action:**
- Remove `createDebugEndpoints` from line 42
- Remove `createBootstrapController` from line 43
- Remove `createSettingsController` from line 44
- Remove `SearchConfig` from line 40

### Part 2: Remove Unused Properties (20 items)

These properties are assigned in the constructor but never read anywhere in the class:

| Property | Line | Reason Unused |
|----------|------|---------------|
| `jwtSecret` | 206, 373 | Auth moved to AuthHandler |
| `interceptors` | 200, 326 | Passed to handlers at creation |
| `metricsService` | 195, 332 | Passed to handlers at creation |
| `topicManager` | 213, 360 | Passed to TopicHandler at creation |
| `securityManager` | 214, 333 | Passed to handlers at creation |
| `authHandler` | 215, 346 | Passed to WebSocketHandler at creation |
| `writeCoalescingEnabled` | 249, 327 | Passed to handlers at creation |
| `writeCoalescingOptions` | 250, 336 | Passed to handlers at creation |
| `rateLimitingEnabled` | 254, 328 | Passed to WebSocketHandler at creation |
| `rateLimitedLogger` | 257, 347 | Passed to WebSocketHandler at creation |
| `writeAckManager` | 272, 344 | Passed to WriteConcernHandler at creation |
| `entryProcessorHandler` | 278, 362 | Passed to EntryProcessorAdapter at creation |
| `bootstrapController` | 305, 354 | Created and wired in ServerFactory |
| `settingsController` | 308, 355 | Created and wired in ServerFactory |
| `lockHandler` | 230, 447 | Registered in MessageRegistry |
| `topicHandler` | 231, 448 | Registered in MessageRegistry |
| `partitionHandler` | 232, 449 | Registered in MessageRegistry |
| `searchHandler` | 233, 450 | Registered in MessageRegistry |
| `journalHandler` | 234, 451 | Registered in MessageRegistry |
| `writeConcernHandler` | 235, 458 | Created in ServerFactory |

### Part 3: Convert Constructor-Only Properties to Locals (5 items)

These properties are only used during constructor wiring and should become local variables:

| Property | Current Line | Usage |
|----------|--------------|-------|
| `lockManager` | 212, 359 | Only for `lockManager.on('lockGranted', ...)` |
| `conflictResolverHandler` | 281, 363 | Only for `onRejection(...)` wiring |
| `replicationPipeline` | 211, 358 | Only for `setOperationApplier(...)` |
| `wss` | 196, 357 | Only for `.on('connection')` wiring |
| `messageRegistry` | 226, 459 | Only for `setMessageRegistry()` wiring |

**Change pattern:**
```typescript
// Before (constructor)
this.replicationPipeline = dependencies.replicationPipeline;
if (this.replicationPipeline) {
    this.replicationPipeline.setOperationApplier(this.applyReplicatedOperation.bind(this));
}

// After (constructor)
const replicationPipeline = dependencies.replicationPipeline;
if (replicationPipeline) {
    replicationPipeline.setOperationApplier(this.applyReplicatedOperation.bind(this));
}
// Property declaration removed
```

## Requirements

### Files to Modify

| File | Action | Changes |
|------|--------|---------|
| `packages/server/src/ServerCoordinator.ts` | modify | Remove 4 imports, 20 properties, convert 5 to locals |

### Expected Line Changes

| File | Before | After | Delta |
|------|--------|-------|-------|
| ServerCoordinator.ts | 858 | ~810 | -48 |

### Verification Steps

1. Remove unused imports
2. Remove property declarations (lines ~195-310 area)
3. Remove property assignments in constructor (lines ~326-460 area)
4. Convert 5 properties to local variables in constructor
5. Verify build passes: `pnpm --filter @topgunbuild/server build`
6. Verify no runtime errors in any dependent code

## Acceptance Criteria

1. [ ] All 4 unused imports removed
2. [ ] All 20 unused properties removed (declarations + assignments)
3. [ ] 5 constructor-only properties converted to local variables
4. [ ] Build passes with no TypeScript errors
5. [ ] No changes to public API
6. [ ] No changes to ServerDependencies interface (properties still passed, just not stored)

## Constraints

- DO NOT change public API of ServerCoordinator
- DO NOT modify ServerDependencies interface
- DO NOT modify ServerFactory
- DO NOT remove properties that are used (even once) outside constructor
- Preserve all constructor wiring logic (just use locals instead of `this.xxx`)

## Assumptions

1. **Properties passed via dependencies but not stored are safe to remove** - ServerFactory handles wiring at creation time
2. **MessageRegistry late binding does not require storing the registry** - Once set on WebSocketHandler, reference not needed
3. **Constructor-only wiring can use locals** - The wiring callbacks capture closures, so locals work fine

## Verification Checklist

Before implementation, verify each property is truly unused:

```bash
# For each property, search for reads (not assignments):
rg 'this\.jwtSecret[^=]' packages/server/src/ServerCoordinator.ts
rg 'this\.interceptors[^=]' packages/server/src/ServerCoordinator.ts
# ... etc for all 20 properties
```

After implementation:

```bash
pnpm --filter @topgunbuild/server build
pnpm --filter @topgunbuild/server test
```

---
*Generated by SpecFlow spec-creator on 2026-01-28*

## Audit History

### Audit v1 (2026-01-28 14:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~15% total (small refactor, single file)

**Critical Issues:**

1. **`wss` incorrectly listed as "Remove" (Part 2)** - Property IS used on line 479: `this.wss.on('connection', ...)`. This is constructor-only usage so it should be moved to Part 3 (Convert to locals), not removed.

2. **`messageRegistry` incorrectly listed as "Remove" (Part 2)** - Property IS used on line 473: `this.webSocketHandler.setMessageRegistry(this.messageRegistry)`. This is constructor-only usage so it should be moved to Part 3 (Convert to locals), not removed.

**Corrections Applied:**
- Moved `wss` from Part 2 (22 items) to Part 3 (now 5 items)
- Moved `messageRegistry` from Part 2 (22 items) to Part 3 (now 5 items)
- Updated Part 2 count: 22 -> 20 properties
- Updated Part 3 count: 3 -> 5 properties
- Updated Acceptance Criteria counts accordingly

**Recommendations:**

3. [Strategic] The spec correctly identifies the right problem (dead code cleanup after refactoring). The approach aligns well with YAGNI principles and improves maintainability.

4. Consider adding a verification step to run the full test suite, not just the server package tests: `pnpm test` (though this may be blocked by existing integration test failures noted in STATE.md).

**Strategic fit:** Aligned with project goals - cleanup after SPEC-003/SPEC-004 refactoring series.

---

## Execution Summary

**Executed:** 2026-01-28
**Commits:** 2

### Files Created
None

### Files Modified
- `packages/server/src/ServerCoordinator.ts` — Removed 4 unused imports, 20 unused properties, and converted 5 constructor-only properties to local variables

### Files Deleted
None

### Acceptance Criteria Status
- [x] All 4 unused imports removed
- [x] All 20 unused properties removed (declarations + assignments)
- [x] 5 constructor-only properties converted to local variables
- [x] Build passes with no TypeScript errors
- [x] No changes to public API
- [x] No changes to ServerDependencies interface (properties still passed, just not stored)

### Implementation Details

**Commit 1: Part 1 - Remove Unused Imports**
- Removed `SearchConfig` from search imports (not used after handler extraction)
- Removed `createDebugEndpoints` from debug imports (factory function not called)
- Removed `createBootstrapController` from bootstrap imports (factory function not called)
- Removed `createSettingsController` from settings imports (factory function not called)

**Commit 2: Parts 2 & 3 - Remove Unused Properties and Convert to Locals**
Part 2 removed 20 unused properties that were only assigned but never read:
- Authentication/Security: `jwtSecret`, `authHandler`, `securityManager`
- Metrics/Monitoring: `metricsService`, `rateLimitedLogger`
- Write Configuration: `writeCoalescingEnabled`, `writeCoalescingOptions`, `rateLimitingEnabled`, `writeAckManager`
- Managers: `topicManager`, `interceptors`
- Handlers: `entryProcessorHandler`, `lockHandler`, `topicHandler`, `partitionHandler`, `searchHandler`, `journalHandler`, `writeConcernHandler`
- Controllers: `bootstrapController`, `settingsController`

Part 3 converted 5 constructor-only properties to local variables:
- `lockManager` — only used for `lockManager.on('lockGranted', ...)` event wiring
- `conflictResolverHandler` — only used for `onRejection(...)` callback wiring
- `replicationPipeline` — only used for `setOperationApplier(...)` setup
- `wss` — only used for `wss.on('connection')` event wiring
- `messageRegistry` — only used for `setMessageRegistry()` late binding

### Results
- **Line count:** 858 → 798 lines (-60 lines, 7.0% reduction)
- **Expected delta:** ~48 lines
- **Actual delta:** 60 lines (20% better than expected)
- **Build status:** Clean build with no TypeScript errors or warnings
- **Public API:** Unchanged
- **ServerDependencies:** Unchanged (all properties still passed, just not stored)

### Deviations
None. All changes followed the specification exactly.

### Notes
The refactoring successfully removed residual artifacts from the SPEC-003/SPEC-004 handler extraction work. The class is now cleaner with reduced cognitive load, no misleading dependencies, and lower memory overhead. All constructor wiring logic was preserved using local variables where appropriate.
