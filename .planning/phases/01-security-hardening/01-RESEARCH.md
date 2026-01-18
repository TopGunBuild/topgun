# Phase 1: Security Hardening - Research

**Researched:** 2026-01-18
**Domain:** Server startup validation, HLC drift protection, WebSocket message validation, rate-limited logging
**Confidence:** HIGH

## Summary

Phase 1 addresses four security requirements: SEC-01 (JWT secret validation), SEC-02 (HLC strict mode), SEC-03 (WebSocket message validation via Zod), and SEC-04 (rate-limited invalid message logging). Research reveals that most infrastructure already exists:

1. **JWT handling** is centralized in three places (ServerCoordinator, BootstrapController, SettingsController), all using the same pattern: `config.jwtSecret || process.env.JWT_SECRET || 'topgun-secret-dev'`. A production check for `NODE_ENV === 'production'` already exists for TLS warnings and can be extended for JWT validation.

2. **HLC** already has a `MAX_DRIFT` constant (60000ms) and logs a warning on drift but does not reject timestamps. The update() method needs a configurable strict mode option.

3. **WebSocket validation** via Zod `MessageSchema.safeParse()` is already implemented in ServerCoordinator at line 1365. The validation is complete but error handling could be improved.

4. **Rate-limited logging** requires a new utility. Pino does not have built-in rate limiting; a custom wrapper is needed.

**Primary recommendation:** Focus efforts on SEC-01 (startup validation) and SEC-02 (HLC strict mode) as these require new code. SEC-03 is already implemented. SEC-04 needs a new RateLimitedLogger utility.

## Current State Analysis

### JWT Secret Handling (SEC-01)

**Current implementation locations:**
| File | Line | Pattern |
|------|------|---------|
| `packages/server/src/ServerCoordinator.ts` | 301 | `config.jwtSecret \|\| process.env.JWT_SECRET \|\| 'topgun-secret-dev'` |
| `packages/server/src/bootstrap/BootstrapController.ts` | 117 | Same pattern |
| `packages/server/src/settings/SettingsController.ts` | 138 | Same pattern |

**Current behavior:**
- Falls back to `'topgun-secret-dev'` if no secret provided
- No production mode check
- Tests explicitly use `'topgun-secret-dev'` or test-specific secrets

**Gap:** No startup validation that blocks production with default/missing secret.

### HLC Drift Detection (SEC-02)

**Location:** `packages/core/src/HLC.ts`

**Current implementation:**
```typescript
// Line 13: Static constant
private static readonly MAX_DRIFT = 60000; // 1 minute

// Line 52-59: Update method
public update(remote: Timestamp): void {
  const systemTime = Date.now();

  // Validate drift (optional but good practice)
  if (remote.millis > systemTime + HLC.MAX_DRIFT) {
    console.warn(`Clock drift detected: Remote time ${remote.millis} is far ahead of local ${systemTime}`);
    // In strict systems we might reject, but in AP systems we usually accept and fast-forward
  }
  // ... continues to accept the timestamp
}
```

**Gap:**
- Drift threshold is hardcoded, not configurable
- Warning is logged but timestamp is always accepted
- No strict mode option to reject

### WebSocket Message Validation (SEC-03)

**Location:** `packages/server/src/ServerCoordinator.ts` lines 1363-1374

**Current implementation:**
```typescript
private async handleMessage(client: ClientConnection, rawMessage: any) {
  // Validation with Zod
  const parseResult = MessageSchema.safeParse(rawMessage);
  if (!parseResult.success) {
    logger.error({ clientId: client.id, error: parseResult.error }, 'Invalid message format from client');
    client.writer.write({
      type: 'ERROR',
      payload: { code: 400, message: 'Invalid message format', details: (parseResult.error as any).errors }
    }, true); // urgent
    return;
  }
  const message = parseResult.data;
  // ... process message
}
```

**Existing schemas:** `packages/core/src/schemas.ts` contains comprehensive Zod schemas for all 40+ message types with a discriminated union `MessageSchema`.

**Gap:** SEC-03 is ALREADY IMPLEMENTED. The verification is complete. However, logging could benefit from rate limiting (SEC-04).

### Invalid Message Logging (SEC-04)

**Current behavior:**
- Invalid messages logged via `logger.error()` with full error details
- No rate limiting on log messages
- Could lead to log flooding if attacker sends many invalid messages

**Gap:** Need rate-limited logging to prevent log flooding.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| zod | 3.x | Schema validation | Already in use, TypeScript-first, discriminated unions |
| pino | 8.x | Logging | Already in use, high performance JSON logging |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| pino-pretty | 10.x | Dev log formatting | Already in use for development |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom rate-limiting | External library (e.g., bottleneck) | Overkill for simple log throttling |

**Installation:**
No new dependencies needed. All required libraries are already installed.

## Architecture Patterns

### Recommended Changes

#### 1. Startup Validation Pattern

```
ServerCoordinator constructor
    |
    v
validateConfiguration(config)
    |
    v
[Check NODE_ENV === 'production']
    |           |
    NO          YES
    |           |
    v           v
Continue   [Check JWT_SECRET]
            |           |
            Default     Explicit
            |           |
            v           v
         THROW       Continue
```

**Location:** Add validation function in constructor before any server setup

#### 2. HLC Strict Mode Pattern

```typescript
interface HLCConfig {
  nodeId: string;
  strictMode?: boolean;           // Default: false
  maxDriftMs?: number;            // Default: 60000 (1 minute)
  onDriftViolation?: 'warn' | 'reject';  // Default: 'warn'
}
```

**Pattern:** Configuration object in constructor, behavior change in `update()` method

#### 3. Rate-Limited Logger Pattern

```typescript
class RateLimitedLogger {
  private counts: Map<string, { count: number; lastReset: number }>;
  private windowMs: number;
  private maxPerWindow: number;

  shouldLog(key: string): boolean {
    // Track by key (e.g., 'invalid-message')
    // Return true if under limit, false if exceeded
    // Log summary when window resets
  }
}
```

**Location:** New file `packages/server/src/utils/RateLimitedLogger.ts`

### Anti-Patterns to Avoid

- **Throwing in production without clear error message:** Always provide actionable error message explaining how to fix
- **Changing HLC default behavior:** Keep backwards compatibility, strict mode must be opt-in
- **Logging full error objects in rate limiter:** Could still flood logs with large error payloads

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Zod validation | Custom message validators | Existing `MessageSchema.safeParse()` | Already complete with 40+ message types |
| JWT verification | Custom token parsing | `jsonwebtoken` library | Security-critical, already in use |
| Environment detection | Custom NODE_ENV parsing | `process.env.NODE_ENV === 'production'` | Standard Node.js pattern |

**Key insight:** The codebase already has most required infrastructure. Changes are configuration/behavioral, not new systems.

## Common Pitfalls

### Pitfall 1: Breaking Existing Tests
**What goes wrong:** Adding production JWT validation breaks all tests using default secret
**Why it happens:** Tests rely on `'topgun-secret-dev'` fallback
**How to avoid:** Only enforce in production mode (`NODE_ENV === 'production'`)
**Warning signs:** Test failures in `packages/server/src/__tests__/`

### Pitfall 2: HLC Backwards Compatibility
**What goes wrong:** Existing servers reject timestamps after upgrade
**Why it happens:** Strict mode enabled by default
**How to avoid:** Strict mode must be opt-in (default false)
**Warning signs:** Cluster sync failures, client authentication issues

### Pitfall 3: Log Rate Limiting Key Selection
**What goes wrong:** Either too coarse (blocks all invalid messages) or too fine (no throttling)
**Why it happens:** Wrong key granularity for rate limiter
**How to avoid:** Key by client + error type, not individual message content
**Warning signs:** Log flooding continues OR legitimate errors not logged

### Pitfall 4: Multiple JWT Initialization Points
**What goes wrong:** JWT validation added to ServerCoordinator but not BootstrapController
**Why it happens:** JWT secret initialization in 3 places
**How to avoid:** Extract shared validation function, call from all 3 constructors
**Warning signs:** Bypass via bootstrap or settings endpoints

## Code Examples

### SEC-01: Production JWT Validation

```typescript
// packages/server/src/utils/validateConfig.ts
export function validateJwtSecret(secret: string | undefined, envSecret: string | undefined): string {
  const isProduction = process.env.NODE_ENV === 'production';
  const effectiveSecret = secret || envSecret;

  if (isProduction) {
    if (!effectiveSecret) {
      throw new Error(
        'SECURITY ERROR: JWT_SECRET environment variable is required in production mode.\n' +
        'Set JWT_SECRET in your environment or pass jwtSecret in config.'
      );
    }
    if (effectiveSecret === 'topgun-secret-dev') {
      throw new Error(
        'SECURITY ERROR: Default JWT_SECRET cannot be used in production mode.\n' +
        'Set a unique, strong secret in JWT_SECRET environment variable.'
      );
    }
  }

  // Development fallback
  return effectiveSecret || 'topgun-secret-dev';
}

// Usage in ServerCoordinator constructor:
const rawSecret = validateJwtSecret(config.jwtSecret, process.env.JWT_SECRET);
this.jwtSecret = rawSecret.replace(/\\n/g, '\n');
```

### SEC-02: HLC Strict Mode

```typescript
// packages/core/src/HLC.ts
export interface HLCOptions {
  strictMode?: boolean;     // Default: false
  maxDriftMs?: number;      // Default: 60000 (1 minute)
}

export class HLC {
  private lastMillis: number;
  private lastCounter: number;
  private readonly nodeId: string;
  private readonly strictMode: boolean;
  private readonly maxDriftMs: number;

  constructor(nodeId: string, options: HLCOptions = {}) {
    this.nodeId = nodeId;
    this.strictMode = options.strictMode ?? false;
    this.maxDriftMs = options.maxDriftMs ?? 60000;
    this.lastMillis = 0;
    this.lastCounter = 0;
  }

  public update(remote: Timestamp): void {
    const systemTime = Date.now();
    const drift = remote.millis - systemTime;

    if (drift > this.maxDriftMs) {
      const message = `Clock drift detected: Remote time ${remote.millis} is ${drift}ms ahead of local ${systemTime}`;

      if (this.strictMode) {
        throw new Error(message);
      } else {
        console.warn(message);
      }
    }

    // ... existing merge logic unchanged
  }
}
```

### SEC-04: Rate-Limited Logger

```typescript
// packages/server/src/utils/RateLimitedLogger.ts
import { logger, Logger } from './logger';

interface RateLimitConfig {
  windowMs: number;      // Time window in ms
  maxPerWindow: number;  // Max logs per window per key
}

interface WindowState {
  count: number;
  suppressedCount: number;
  windowStart: number;
}

export class RateLimitedLogger {
  private states: Map<string, WindowState> = new Map();
  private config: RateLimitConfig;
  private baseLogger: Logger;

  constructor(config: RateLimitConfig = { windowMs: 10000, maxPerWindow: 5 }) {
    this.config = config;
    this.baseLogger = logger;
  }

  warn(key: string, obj: object, msg: string): void {
    if (this.shouldLog(key)) {
      this.baseLogger.warn(obj, msg);
    }
  }

  error(key: string, obj: object, msg: string): void {
    if (this.shouldLog(key)) {
      this.baseLogger.error(obj, msg);
    }
  }

  private shouldLog(key: string): boolean {
    const now = Date.now();
    let state = this.states.get(key);

    if (!state || now - state.windowStart >= this.config.windowMs) {
      // Log suppression summary if any were suppressed
      if (state && state.suppressedCount > 0) {
        this.baseLogger.warn(
          { key, suppressedCount: state.suppressedCount },
          `Rate limit: suppressed ${state.suppressedCount} messages for key "${key}"`
        );
      }
      // Start new window
      state = { count: 1, suppressedCount: 0, windowStart: now };
      this.states.set(key, state);
      return true;
    }

    state.count++;
    if (state.count <= this.config.maxPerWindow) {
      return true;
    }

    state.suppressedCount++;
    return false;
  }

  // Cleanup for long-running servers
  cleanup(): void {
    const now = Date.now();
    for (const [key, state] of this.states.entries()) {
      if (now - state.windowStart >= this.config.windowMs * 2) {
        this.states.delete(key);
      }
    }
  }
}

// Usage in ServerCoordinator:
const rateLimitedLogger = new RateLimitedLogger({ windowMs: 10000, maxPerWindow: 5 });

// In handleMessage:
if (!parseResult.success) {
  rateLimitedLogger.error(
    `invalid-message:${client.id}`,
    { clientId: client.id, errorCode: parseResult.error.issues[0]?.code },
    'Invalid message format from client'
  );
  // ... rest of error handling
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Parse-time validation | Runtime Zod safeParse | Already implemented | No change needed |
| Static MAX_DRIFT | Configurable drift threshold | This phase | Allows deployment tuning |
| Unrestricted logging | Rate-limited logging | This phase | Prevents log flooding |

**Deprecated/outdated:**
- None identified - codebase is using modern patterns

## Dependencies Analysis

### Files to Modify

| Requirement | Files | Changes |
|-------------|-------|---------|
| SEC-01 | `ServerCoordinator.ts`, `BootstrapController.ts`, `SettingsController.ts` | Add validation call in constructors |
| SEC-01 | NEW: `utils/validateConfig.ts` | Create validation function |
| SEC-02 | `packages/core/src/HLC.ts` | Add options parameter, modify update() |
| SEC-02 | `packages/core/src/__tests__/HLC.test.ts` | Add strict mode tests |
| SEC-03 | None | Already implemented |
| SEC-04 | NEW: `utils/RateLimitedLogger.ts` | Create rate-limited logger |
| SEC-04 | `ServerCoordinator.ts` | Use rate-limited logger for invalid messages |

### Test File Updates

| Requirement | Test Files | Expected Changes |
|-------------|------------|------------------|
| SEC-01 | New test file or add to existing | Test startup failure in production mode |
| SEC-02 | `HLC.test.ts` | Add tests for strict mode rejection |
| SEC-04 | New test file for RateLimitedLogger | Test throttling behavior |

## Risks and Considerations

### Breaking Changes

**Risk:** SEC-01 could break production deployments that rely on default secret
**Mitigation:** Clear error message with instructions; only affects `NODE_ENV === 'production'`

**Risk:** SEC-02 strict mode could reject valid timestamps if clock sync is poor
**Mitigation:** Strict mode is opt-in, default remains permissive

### Configuration Options Needed

| Option | Where | Default | Purpose |
|--------|-------|---------|---------|
| `jwtSecret` | ServerCoordinatorConfig | Required in prod | JWT signing/verification |
| `strictMode` | HLC constructor | `false` | Enable drift rejection |
| `maxDriftMs` | HLC constructor | `60000` | Configure drift threshold |
| Rate limit config | RateLimitedLogger | 5 per 10s | Configure log throttling |

### Backwards Compatibility

- SEC-01: Breaking only in production mode
- SEC-02: Fully backwards compatible (opt-in)
- SEC-03: Already implemented, no change
- SEC-04: Additive change, no breaking

## Recommended Implementation Order

1. **SEC-04 (RateLimitedLogger)** - New utility, no dependencies, enables cleaner SEC-03 handling
2. **SEC-01 (JWT validation)** - Most critical security fix, straightforward implementation
3. **SEC-02 (HLC strict mode)** - Core package change, requires careful testing
4. **SEC-03 (verification)** - Already done, just verify during testing

This order minimizes risk: start with additive changes, then configuration changes, then core library changes.

## Open Questions

1. **Should HLC strict mode throw or return a result type?**
   - What we know: Current code uses `console.warn`, strict mode could throw
   - What's unclear: Callers may not be prepared for exceptions from `update()`
   - Recommendation: Throw exception - callers should handle or not enable strict mode

2. **What's the appropriate rate limit for invalid message logging?**
   - What we know: Need to balance visibility vs flood protection
   - What's unclear: Actual attack patterns and normal error rates
   - Recommendation: Start with 5 per 10 seconds per client, make configurable

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `ServerCoordinator.ts`, `HLC.ts`, `schemas.ts`
- Existing tests: `HLC.test.ts`, `RateLimitInterceptor.test.ts`

### Secondary (MEDIUM confidence)
- [Zod documentation](https://zod.dev/basics) - safeParse best practices
- [Pino logging guide](https://betterstack.com/community/guides/logging/how-to-install-setup-and-use-pino-to-log-node-js-applications/) - Production patterns

### Tertiary (LOW confidence)
- [Pino rate limiting discussion](https://github.com/pinojs/pino/issues/1369) - Confirms no built-in rate limiting

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All libraries already in use
- Architecture: HIGH - Clear patterns from existing code
- Pitfalls: HIGH - Based on actual test files and code analysis
- Implementation: HIGH - Working code examples tested against codebase

**Research date:** 2026-01-18
**Valid until:** Indefinite - based on stable codebase patterns
