# SPEC-021: Startup Environment Variable Validation

```yaml
id: SPEC-021
type: feature
status: audited
priority: high
complexity: small
created: 2026-02-01
source: TODO-009
```

## Context

The server currently reads 20+ environment variables in `start-server.ts` with no centralized validation. The only existing validation is `validateJwtSecret()` which handles a single variable. Other critical variables like `DATABASE_URL`, `TOPGUN_CLUSTER_SEEDS`, and TLS paths have ad-hoc validation scattered throughout the startup code.

This creates several problems:
1. **Late failures**: Invalid config discovered deep in initialization, not at startup
2. **Unclear errors**: Missing variables produce cryptic errors
3. **No documentation**: Required vs optional variables are not codified
4. **Security gaps**: Production requirements not enforced systematically

## Task

Create a Zod schema for environment variable validation that runs at server startup, failing fast with clear error messages.

## Requirements

### Files to Create

**`packages/server/src/config/env-schema.ts`**
- Zod schema defining all environment variables
- Required variables: `NODE_ENV` (production requires stricter validation)
- Conditional requirements: TLS paths required when `TOPGUN_TLS_ENABLED=true`
- Type coercion: Port numbers as integers, booleans from string
- Default values: Documented inline
- Export parsed config type

**`packages/server/src/config/__tests__/env-schema.test.ts`**
- Unit tests for `validateEnv()` function
- Test cases for error messages and edge cases
- Test invalid port numbers (negative, zero, >65535)
- Test missing TLS paths when TLS enabled
- Test production mode requirements (JWT_SECRET)
- Test default values are applied correctly
- Test type coercion (string to number, string to boolean)
- Test all validation errors are collected and reported

### Files to Modify

**`packages/server/src/start-server.ts`**
- Import and call `validateEnv()` at very top of file (before any other logic)
- Replace direct `process.env` access with validated config object
- Remove ad-hoc TLS validation (lines 36-77) - schema handles it

**`packages/server/src/utils/validateConfig.ts`**
- Keep `validateJwtSecret()` but update it to accept parsed config
- Or inline JWT validation into env-schema if cleaner

**`packages/server/src/config/index.ts`**
- Export new `validateEnv` function and `EnvConfig` type

### Interface

```typescript
// packages/server/src/config/env-schema.ts
import { z } from 'zod';

const EnvSchema = z.object({
  // Required
  NODE_ENV: z.enum(['development', 'test', 'production']).default('development'),

  // Server ports
  TOPGUN_PORT: z.coerce.number().int().min(1).max(65535).default(8080),
  TOPGUN_CLUSTER_PORT: z.coerce.number().int().min(1).max(65535).default(9080),
  TOPGUN_METRICS_PORT: z.coerce.number().int().min(1).max(65535).optional(),

  // Node identity
  NODE_ID: z.string().optional(), // Generated if not provided

  // Clustering
  TOPGUN_PEERS: z.string().optional(), // Comma-separated list
  TOPGUN_DISCOVERY_SERVICE: z.string().optional(),
  TOPGUN_DISCOVERY_INTERVAL: z.coerce.number().int().positive().default(10000),

  // Database
  DATABASE_URL: z.string().url().optional(), // Optional: in-memory mode if missing

  // Security
  JWT_SECRET: z.string().min(32).optional(), // Required in production

  // TLS - Client facing
  TOPGUN_TLS_ENABLED: z.enum(['true', 'false']).transform(v => v === 'true').default('false'),
  TOPGUN_TLS_CERT_PATH: z.string().optional(),
  TOPGUN_TLS_KEY_PATH: z.string().optional(),
  TOPGUN_TLS_CA_PATH: z.string().optional(),
  TOPGUN_TLS_MIN_VERSION: z.enum(['TLSv1.2', 'TLSv1.3']).default('TLSv1.2'),
  TOPGUN_TLS_PASSPHRASE: z.string().optional(),

  // TLS - Cluster
  TOPGUN_CLUSTER_TLS_ENABLED: z.enum(['true', 'false']).transform(v => v === 'true').default('false'),
  TOPGUN_CLUSTER_TLS_CERT_PATH: z.string().optional(),
  TOPGUN_CLUSTER_TLS_KEY_PATH: z.string().optional(),
  TOPGUN_CLUSTER_TLS_CA_PATH: z.string().optional(),
  TOPGUN_CLUSTER_MTLS: z.enum(['true', 'false']).transform(v => v === 'true').default('false'),
  TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED: z.enum(['true', 'false']).transform(v => v === 'true').default('true'),

  // Debug
  TOPGUN_DEBUG: z.enum(['true', 'false']).transform(v => v === 'true').default('false'),
}).superRefine((data, ctx) => {
  // Production requirements
  if (data.NODE_ENV === 'production') {
    if (!data.JWT_SECRET) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'JWT_SECRET is required in production',
        path: ['JWT_SECRET'],
      });
    }
  }

  // TLS cert/key pairs
  if (data.TOPGUN_TLS_ENABLED) {
    if (!data.TOPGUN_TLS_CERT_PATH) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'TOPGUN_TLS_CERT_PATH required when TLS enabled',
        path: ['TOPGUN_TLS_CERT_PATH'],
      });
    }
    if (!data.TOPGUN_TLS_KEY_PATH) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'TOPGUN_TLS_KEY_PATH required when TLS enabled',
        path: ['TOPGUN_TLS_KEY_PATH'],
      });
    }
  }

  if (data.TOPGUN_CLUSTER_TLS_ENABLED) {
    if (!data.TOPGUN_CLUSTER_TLS_CERT_PATH && !data.TOPGUN_TLS_CERT_PATH) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Cluster TLS requires cert path (TOPGUN_CLUSTER_TLS_CERT_PATH or TOPGUN_TLS_CERT_PATH)',
        path: ['TOPGUN_CLUSTER_TLS_CERT_PATH'],
      });
    }
    if (!data.TOPGUN_CLUSTER_TLS_KEY_PATH && !data.TOPGUN_TLS_KEY_PATH) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Cluster TLS requires key path (TOPGUN_CLUSTER_TLS_KEY_PATH or TOPGUN_TLS_KEY_PATH)',
        path: ['TOPGUN_CLUSTER_TLS_KEY_PATH'],
      });
    }
  }
});

export type EnvConfig = z.infer<typeof EnvSchema>;

export function validateEnv(): EnvConfig {
  const result = EnvSchema.safeParse(process.env);
  if (!result.success) {
    const errors = result.error.errors
      .map(e => `  - ${e.path.join('.')}: ${e.message}`)
      .join('\n');
    console.error(`Environment validation failed:\n${errors}`);
    process.exit(1);
  }
  return result.data;
}
```

### Deletions

- Remove ad-hoc TLS validation blocks in `start-server.ts` (lines 36-77)
- Remove duplicate validation logic once centralized

## Acceptance Criteria

1. **Schema validates all env vars**: All 20+ environment variables used in `start-server.ts` are defined in the Zod schema
2. **Fail-fast behavior**: Server exits with code 1 and clear error message if validation fails
3. **Production enforcement**: `JWT_SECRET` required in production mode
4. **TLS conditional requirements**: Cert/key paths required when TLS enabled
5. **Type safety**: Parsed config exports correct TypeScript types
6. **Default values documented**: Each optional variable has a default or is explicitly optional
7. **Clear error messages**: Validation errors list all failing variables, not just the first
8. **Unit tests exist**: Test file created with coverage for error messages and edge cases
9. **Unit tests validate errors**: Tests verify invalid port numbers produce correct error messages
10. **Unit tests validate TLS**: Tests verify missing TLS paths when enabled produce correct errors
11. **Unit tests validate production**: Tests verify production mode enforces JWT_SECRET requirement
12. **Unit tests validate defaults**: Tests verify default values are applied when env vars not set
13. **Unit tests validate coercion**: Tests verify type coercion (string to number, string to boolean)
14. **All tests pass**: Unit tests run successfully with `pnpm --filter @topgunbuild/server test`

## Constraints

- Do NOT change existing server behavior - only add validation layer
- Do NOT modify how environment variables are ultimately used (just where they're read)
- Keep `validateJwtSecret()` logic intact (default secret check, production enforcement)
- Validation must run synchronously before any async operations
- Do NOT add new dependencies - Zod is available via pnpm hoisting from `@topgunbuild/core`

## Assumptions

1. Zod should be imported directly from `'zod'` (available via pnpm hoisting since `@topgunbuild/core` depends on it)
2. `process.exit(1)` is acceptable for validation failures (consistent with existing TLS validation)
3. Environment variables not listed in schema can be ignored (other code may read additional vars)
4. `DATABASE_URL` remains optional (in-memory mode is valid)
5. Port numbers 1-65535 are valid range

## Verification Commands

```bash
# Build passes
pnpm --filter @topgunbuild/server build

# Existing tests pass
pnpm --filter @topgunbuild/server test

# Type check
pnpm --filter @topgunbuild/server exec tsc --noEmit

# Verify schema exports
grep -n "validateEnv\|EnvConfig" packages/server/src/config/index.ts
```

---

## Audit History

### Audit v1 (2026-02-01 14:30)
**Status:** APPROVED

**Context Estimate:** ~15% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~15% | <=50% | OK |
| File count | 4 files | <=8 | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**
- Clarity: PASS - Title, context, and task are clear and specific
- Completeness: PASS - All files listed, interface provided
- Testability: PASS - Acceptance criteria are measurable
- Scope: PASS - Well-bounded small task
- Feasibility: PASS - Technical approach is sound
- Architecture fit: PASS - Uses existing Zod pattern from core
- Non-duplication: PASS - Builds on existing validateJwtSecret()
- Cognitive load: PASS - Simple, focused solution
- Strategic fit: PASS - Addresses real problem (late failures, unclear errors)

**Fixes Applied During Audit:**
1. Corrected line numbers for TLS validation deletion: changed "lines 36-60" to "lines 36-77" (actual location in source)
2. Removed duplicate acceptance criterion (item 2 and 3 were identical "Fail-fast behavior")
3. Corrected Assumption #1: Zod is NOT re-exported from `@topgunbuild/core`, but is available via pnpm hoisting

**Recommendations:**
1. Consider adding unit tests for `validateEnv()` function to verify error messages and edge cases (e.g., invalid port numbers, missing TLS paths when enabled)

**Comment:** Well-structured specification with clear interface. The Zod schema is comprehensive and covers all environment variables observed in start-server.ts. Minor corrections applied to line numbers and assumptions.

### Response v1 (2026-02-01 15:00)
**Applied:** Recommendation 1 - Add unit tests for `validateEnv()` function

**Changes:**
1. Added unit test file requirement - Added `packages/server/src/config/__tests__/env-schema.test.ts` to Files to Create section with detailed test case requirements
2. Added acceptance criteria for unit tests - Added 7 new acceptance criteria (#8-14) covering test existence, error validation, TLS validation, production mode, defaults, type coercion, and test passing requirement

**Skipped:** None - recommendation fully applied

### Audit v2 (2026-02-01 16:00)
**Status:** APPROVED

**Context Estimate:** ~21% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~21% | <=50% | OK |
| File count | 5 files | <=8 | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <- Current |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions Evaluated:**
- Clarity: PASS - Title, context, and task are clear and specific
- Completeness: PASS - All files listed, interface provided, test requirements detailed
- Testability: PASS - 14 acceptance criteria, all measurable
- Scope: PASS - Well-bounded small task
- Feasibility: PASS - Technical approach is sound
- Architecture fit: PASS - Uses existing Zod pattern (verified in core/mcp-server)
- Non-duplication: PASS - Builds on existing validateJwtSecret()
- Cognitive load: PASS - Simple, focused solution
- Strategic fit: PASS - Addresses real problem (late failures, unclear errors)

**Recommendation v1 Verification:**
- Unit test file requirement added to Files to Create section
- 7 new acceptance criteria (#8-14) for unit tests added
- Test requirements are comprehensive (error messages, edge cases, coercion, defaults)

**Fixes Applied During Audit:**
1. Corrected line number reference in Files to Modify section: changed "lines 36-60" to "lines 36-77" (inconsistency with Deletions section from v1 audit fix)

**Comment:** Specification is now complete with comprehensive unit test requirements. The v1 recommendation was properly applied. Minor line number inconsistency corrected. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-01 16:57
**Commits:** 2

### Files Created
- `packages/server/src/config/env-schema.ts` — Zod schema for environment variable validation with type coercion, defaults, and conditional requirements
- `packages/server/src/config/__tests__/env-schema.test.ts` — 27 comprehensive unit tests covering all validation scenarios

### Files Modified
- `packages/server/src/config/index.ts` — Added exports for validateEnv() and EnvConfig type
- `packages/server/src/start-server.ts` — Integrated env validation at startup, removed ad-hoc TLS validation (lines 36-77), replaced process.env access with typed config

### Files Deleted
None

### Acceptance Criteria Status
- [x] Schema validates all env vars — All 20+ environment variables defined in Zod schema
- [x] Fail-fast behavior — Server exits with code 1 and clear error messages on validation failure
- [x] Production enforcement — JWT_SECRET required in production mode (tested)
- [x] TLS conditional requirements — Cert/key paths required when TLS enabled (tested)
- [x] Type safety — EnvConfig type correctly inferred from Zod schema
- [x] Default values documented — All optional variables have defaults or explicitly optional
- [x] Clear error messages — All validation errors collected and reported via error.issues
- [x] Unit tests exist — env-schema.test.ts created with 27 test cases
- [x] Unit tests validate errors — Tests verify invalid port numbers produce correct error messages
- [x] Unit tests validate TLS — Tests verify missing TLS paths when enabled produce correct errors
- [x] Unit tests validate production — Tests verify production mode enforces JWT_SECRET requirement
- [x] Unit tests validate defaults — Tests verify default values applied when env vars not set
- [x] Unit tests validate coercion — Tests verify type coercion (string to number, string to boolean)
- [x] All tests pass — 27/27 tests pass in env-schema.test.ts

### Deviations
None

### Notes
- Build passes successfully with all type declarations generated
- Pre-existing test failures in Chaos.test.ts, Resilience.test.ts, and GC.test.ts from SPEC-018 (serverUrl removal) are unrelated to this implementation
- validateJwtSecret() in utils/validateConfig.ts kept intact as per constraints
- Zod available via pnpm hoisting from @topgunbuild/core dependency

---
*Generated by SpecFlow on 2026-02-01*
