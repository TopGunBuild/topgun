# Coding Conventions

**Analysis Date:** 2026-01-18

## Naming Patterns

**Files:**
- PascalCase for classes: `LWWMap.ts`, `HLC.ts`, `SyncStateMachine.ts`
- camelCase for utilities: `logger.ts`, `hash.ts`, `serializer.ts`
- kebab-case for multi-word utilities: `crdt-types.ts`, `merkle-types.ts`
- Pattern: `{Name}.ts` for implementation, `{Name}.test.ts` for tests

**Classes:**
- PascalCase: `ServerCoordinator`, `WorkerPool`, `TopGunClient`
- Descriptive compound names: `SyncStateMachine`, `EncryptionManager`, `BufferPool`
- Implementation suffix for interfaces: `PNCounterImpl` implements `PNCounter`
- Error suffix for custom errors: `BackpressureError`, `WorkerTimeoutError`

**Functions:**
- camelCase: `createTestServer()`, `waitForSync()`, `hashORMapEntry()`
- Verbs for actions: `create*`, `get*`, `set*`, `wait*`, `validate*`
- Boolean getters: `isConnected()`, `isReady()`, `canTransition()`

**Variables:**
- camelCase: `nodeId`, `lastMillis`, `pendingCount`
- Constants: SCREAMING_SNAKE_CASE in `const` declarations: `MAX_DRIFT`, `JWT_SECRET`
- Private members: no underscore prefix, use TypeScript `private` keyword

**Types:**
- PascalCase for interfaces: `Timestamp`, `LWWRecord`, `TestContext`
- Interfaces for data shapes: `interface TopGunConfig`
- Type aliases for unions/primitives: `type PredicateOp = 'eq' | 'neq' | ...`
- Schema suffix for Zod schemas: `TimestampSchema`, `WriteConcernSchema`

## Code Style

**Formatting:**
- No explicit Prettier/ESLint config at root (uses defaults)
- 2-space indentation (TypeScript standard)
- Single quotes for strings
- Semicolons at end of statements
- Trailing commas in multi-line arrays/objects

**Linting:**
- No global ESLint config - uses TypeScript strict mode for enforcement
- TypeScript strict mode enabled in `tsconfig.json`

**TypeScript Config:**
```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "moduleResolution": "bundler"
  }
}
```

## Import Organization

**Order:**
1. External dependencies (npm packages)
2. Internal package imports (`@topgunbuild/*`)
3. Relative imports (local files)

**Example from `packages/core/src/index.ts`:**
```typescript
// Internal class imports
import { HLC, Timestamp } from './HLC';
import { LWWMap, LWWRecord } from './LWWMap';
import { ORMap, ORMapRecord, MergeKeyResult, ORMapSnapshot } from './ORMap';

// Re-exports
export { HLC, LWWMap, ORMap, MerkleTree };
export type { Timestamp, LWWRecord, ORMapRecord };
```

**Path Aliases:**
- `@topgunbuild/core` → `packages/core/src`
- `@topgunbuild/client` → `packages/client/src`
- `@topgunbuild/server` → `packages/server/src`
- `@topgunbuild/adapters` → `packages/adapters/src`
- `@topgunbuild/react` → `packages/react/src`

## Error Handling

**Custom Error Pattern:**
```typescript
export class BackpressureError extends Error {
  public readonly name = 'BackpressureError';

  constructor(
    public readonly pendingCount: number,
    public readonly maxPending: number
  ) {
    super(`Backpressure limit reached: ${pendingCount}/${maxPending} pending operations.`);

    // Maintains proper stack trace (V8 only)
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, BackpressureError);
    }
  }
}
```

**Error Hierarchy:**
```typescript
// packages/server/src/workers/errors.ts
export class WorkerError extends Error { ... }
export class WorkerTimeoutError extends WorkerError { ... }
export class WorkerTaskError extends WorkerError { ... }
export class WorkerPoolShutdownError extends WorkerError { ... }
```

**Error Properties:**
- Always include relevant context as readonly public properties
- Include `taskId`, `timeout`, `exitCode` etc. for debugging
- Provide descriptive error messages with interpolated values

**Async Error Handling:**
```typescript
// Catch and re-throw with context
this.initPromise = this.client.start().catch(err => {
  console.error('Failed to start TopGun client:', err);
  throw err;
});

// Promise rejection handling in tests
const promises = tasks.map(t => pool.submit(t).catch(() => {}));
```

## Logging

**Framework:** `pino` (server package)

**Configuration (`packages/server/src/utils/logger.ts`):**
```typescript
import pino from 'pino';

const logLevel = process.env.LOG_LEVEL || 'info';

export const logger = pino({
  level: logLevel,
  transport: process.env.NODE_ENV !== 'production' ? {
    target: 'pino-pretty',
    options: {
      colorize: true,
      translateTime: 'SYS:standard',
      ignore: 'pid,hostname'
    }
  } : undefined,
});
```

**Client Package:** Uses `console.*` directly for browser compatibility

**Patterns:**
- Use structured logging with pino for server
- Log level controlled via `LOG_LEVEL` env var
- Pretty printing in development only

## Comments

**When to Comment:**
- Above complex algorithms (e.g., CRDT merge logic)
- For HLC timestamp explanation
- Public API documentation

**JSDoc/TSDoc:**
- Used for public class methods
- Document parameters and return types
```typescript
/**
 * Generates a new unique timestamp for a local event.
 * Ensures monotonicity: always greater than any previously generated or received timestamp.
 */
public now(): Timestamp { ... }

/**
 * Merges a record from a remote source.
 * Returns true if the local state was updated.
 */
public merge(key: K, remoteRecord: LWWRecord<V>): boolean { ... }
```

**Inline Comments:**
- Explain LWW logic and edge cases
- Mark Phase numbers: `// Phase 1.02: Worker Threads Implementation`

## Function Design

**Size:**
- Keep functions focused on single responsibility
- Extract complex logic into helper functions

**Parameters:**
- Use object destructuring for multiple optional params
- Type config objects as interfaces
```typescript
export interface TopGunConfig {
  sync: string;
  persist: 'indexeddb' | IStorageAdapter;
  nodeId?: string;
}
```

**Return Values:**
- Return `undefined` for missing values (not `null`)
- Return objects with status info for operations: `{ success: boolean, error?: string }`
- Methods that modify state return the result/record

## Module Design

**Exports (`packages/*/src/index.ts`):**
- Single barrel file per package
- Export classes, functions, and types explicitly
- Re-export types with `export type { ... }`

**Pattern:**
```typescript
// Export classes
export { HLC, LWWMap, ORMap, MerkleTree };

// Export type aliases separately
export type { Timestamp, LWWRecord, ORMapRecord };

// Re-export from submodules
export * from './utils/hash';
export * from './serializer';
```

**Barrel Files:**
- `packages/server/src/workers/index.ts` - worker exports
- `packages/core/src/query/indexes/index.ts` - index exports
- Each package has top-level `index.ts`

## Zod Schema Pattern

**Schema Definition:**
```typescript
import { z } from 'zod';

export const TimestampSchema = z.object({
  millis: z.union([z.number(), z.bigint()]).transform(Number),
  counter: z.union([z.number(), z.bigint()]).transform(Number),
  nodeId: z.string(),
});

export type Timestamp = z.infer<typeof TimestampSchema>;
```

**Schema Naming:**
- Suffix with `Schema`: `TimestampSchema`, `QuerySchema`
- Inferred types match schema name without suffix

## Interface vs Type

**Use `interface` for:**
- Object shapes that may be extended
- Public API contracts
- Config objects

**Use `type` for:**
- Union types: `type PredicateOp = 'eq' | 'neq' | ...`
- Function signatures: `type EntryProcessorFn<V, R> = (...) => R`
- Mapped types and utilities

## Async Patterns

**Initialization:**
```typescript
// Non-blocking init with Promise storage
this.initPromise = this.client.start().catch(err => {
  console.error('Failed to start:', err);
  throw err;
});

// Optional await
public async waitForReady(): Promise<void> {
  await this.initPromise;
}
```

**Event Subscription:**
```typescript
// Return unsubscribe function
public onChange(callback: () => void): () => void {
  this.listeners.push(callback);
  return () => {
    this.listeners = this.listeners.filter(cb => cb !== callback);
  };
}
```

---

*Convention analysis: 2026-01-18*
