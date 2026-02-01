# To-Do List

## High Priority

### TODO-009: Environment Variable Validation
**Priority:** high | **Complexity:** small | **Source:** SCAN.md

Add startup validation for required environment variables using Zod schema.

**Context:**
- No validation for required env vars at startup (SCAN.md: Security Considerations)
- `JWT_SECRET` validation exists (`validateJwtSecret()`) but is incomplete
- Other required vars like `TOPGUN_CLUSTER_SEEDS`, `DATABASE_URL` have no validation

**Files:**
- `packages/server/src/start-server.ts:8-32` — env var usage
- Create: `packages/server/src/config/env-schema.ts`

**Acceptance Criteria:**
- [ ] Zod schema validates all required env vars at startup
- [ ] Server fails fast with clear error message if validation fails
- [ ] Optional vars have documented defaults

---

### TODO-010: Debug Endpoint Security Audit
**Priority:** high | **Complexity:** small | **Source:** SCAN.md

Document and harden debug endpoint protection to prevent accidental production exposure.

**Context:**
- Debug endpoints (`/debug/crdt/*`, `/debug/search/*`) expose internal state (SCAN.md: Security)
- Gated by `debugEnabled` flag in `ServerCoordinator.ts:137`
- Defaults to `TOPGUN_DEBUG === 'true'` in `ServerFactory.ts:155`

**Files:**
- `packages/server/src/ServerCoordinator.ts:137`
- `packages/server/src/ServerFactory.ts:155`

**Acceptance Criteria:**
- [ ] Warning logged if debug endpoints enabled
- [ ] Documentation updated with security implications
- [ ] Consider: require explicit `TOPGUN_DEBUG_ENDPOINTS=true` separate from logging

---

### TODO-011: Error Handling Standardization
**Priority:** high | **Complexity:** medium | **Source:** SCAN.md

Replace console.error with structured logging and fix empty catch blocks.

**Context:**
- 40+ `console.error()` occurrences in production code (SCAN.md: Console Logging)
- Empty catch block in `ClusterManager.ts:486` swallows WebSocket close errors
- Inconsistent logging makes production debugging difficult

**Files:**
- `packages/server/src/cluster/ClusterManager.ts:486` — empty catch
- `packages/core/src/EventJournal.ts:160,216` — console.error
- `packages/client/src/TopicHandle.ts:52` — console.error
- `packages/mcp-server/src/transport/http.ts:321` — console.error

**Acceptance Criteria:**
- [ ] Zero `console.error` in production code (tests OK)
- [ ] All catch blocks either log or re-throw
- [ ] Uses existing `TOPGUN_DEBUG` logger infrastructure

---

## Medium Priority

### TODO-012: Type Safety Cleanup
**Priority:** medium | **Complexity:** small | **Source:** SCAN.md

Remove `@ts-ignore`, `@ts-expect-error`, and `as any` casts with proper typing.

**Context:**
- 9 files with `@ts-ignore` or `@ts-expect-error` (SCAN.md: Type Safety)
- 40+ `as any` casts in `mcp-integration.test.ts`
- ESLint `@typescript-eslint/no-explicit-any` set to warn, not error

**Files:**
- `packages/core/src/utils/hash.ts:1`
- `packages/server/src/settings/SettingsController.ts:1`
- `packages/server/src/workers/worker-scripts/base.worker.ts:1`
- `packages/mcp-server/src/__tests__/mcp-integration.test.ts` — 40+ `as any`

**Acceptance Criteria:**
- [ ] Zero `@ts-ignore` / `@ts-expect-error` in codebase
- [ ] `as any` reduced to documented exceptions only
- [ ] Consider upgrading ESLint rule to error level

---

### TODO-013: Timer Cleanup System
**Priority:** medium | **Complexity:** small | **Source:** SCAN.md

Ensure all timers use TimerRegistry for proper cleanup during shutdown.

**Context:**
- 304 occurrences of `setTimeout`/`setInterval` across 97 files (SCAN.md: Tech Debt)
- `TimerRegistry` exists in `packages/server/src/utils/TimerRegistry.ts`
- Not all timers use it — potential memory leaks on shutdown

**Files:**
- `packages/server/src/utils/TimerRegistry.ts` — existing registry
- Audit all 97 files with timer usage

**Acceptance Criteria:**
- [ ] All server-side timers registered with TimerRegistry
- [ ] LifecycleManager clears all timers on shutdown
- [ ] No zombie timers after graceful shutdown

---

### TODO-014: BetterAuth Custom Foreign Keys
**Priority:** medium | **Complexity:** small | **Source:** SCAN.md

Add support for custom foreign key configuration in BetterAuth adapter.

**Context:**
- Adapter assumes standard `userId` relation (SCAN.md: Tech Debt)
- Breaks with non-standard schemas
- TODO comment at `packages/adapter-better-auth/src/TopGunAdapter.ts:176`

**Files:**
- `packages/adapter-better-auth/src/TopGunAdapter.ts:176`

**Acceptance Criteria:**
- [ ] Foreign key field configurable via adapter options
- [ ] Default remains `userId` for backwards compatibility
- [ ] Tests cover custom foreign key scenarios

---

*Last updated: 2026-02-01 (Added TODO-009 through TODO-014 from SCAN.md)*
