# Phase 2: Worker Test Fixes - Research

**Researched:** 2026-01-18
**Domain:** Node.js Worker Threads, Jest Testing, TypeScript Test Infrastructure
**Confidence:** HIGH

## Summary

Phase 2 requires fixing all skipped worker tests to restore CI confidence. The investigation reveals a consistent root cause across all skipped tests: **ts-node cannot load TypeScript worker scripts in worker_threads**. This is a known limitation where Jest runs via ts-node, but Node.js worker_threads require JavaScript files or special configuration.

The skipped tests fall into two categories:
1. **Worker thread operation tests** (6 tests) - Tests that specifically exercise worker thread code paths for large batches
2. **E2E test suite** (1 describe block) - DistributedSearch E2E that has complex server setup requirements

**Primary recommendation:** Configure Jest to use compiled JavaScript for worker tests, or refactor tests to validate inline execution paths and move large-batch worker-specific tests to a separate compiled test runner.

## Skipped Tests Inventory

### CRDTMergeWorker.test.ts
| Line | Test Name | Skip Reason | Category |
|------|-----------|-------------|----------|
| 182 | `should handle large batches (worker thread)` - LWW | ts-node limitation | Worker thread |
| 371 | `should handle large batches (worker thread)` - ORMap | ts-node limitation | Worker thread |

**Comment in code:** `// Skip worker thread tests in Jest`

### MerkleWorker.test.ts
| Line | Test Name | Skip Reason | Category |
|------|-----------|-------------|----------|
| 100 | `should handle large batches (worker thread)` | ts-node limitation | Worker thread |
| 223 | `should handle large rebuilds (worker thread)` | ts-node limitation | Worker thread |
| 288 | `should handle 10,000+ entries` | ts-node limitation | Performance |

**Comment in code:** `// Skip worker thread tests in Jest (ts-node doesn't support workers with .ts files)`

### SerializationWorker.test.ts
| Line | Test Name | Skip Reason | Category |
|------|-----------|-------------|----------|
| 334 | `describe.skip('Worker Thread Operations')` | ts-node limitation | Worker thread (2 tests) |

**Comment in code:** `// Worker thread tests - skipped due to ts-node limitations`

### DistributedSearch.e2e.test.ts
| Line | Test Name | Skip Reason | Category |
|------|-----------|-------------|----------|
| 21 | `describe.skip('Distributed Search E2E')` | Complex server setup | E2E integration |

**Comment in code:** `// Skip E2E tests in CI - they require complex server setup`

**Total:** 9 tests skipped across 4 files

## Worker Architecture

### WorkerPool (packages/server/src/workers/WorkerPool.ts)

The WorkerPool manages a thread pool for CPU-bound operations:

```
WorkerPool
├── Configuration (min/max workers, timeouts, idle scaling)
├── Task queue with priority (high > normal > low)
├── Worker lifecycle (create, assign, timeout, crash recovery)
└── Statistics tracking (completed, failed, duration)
```

**Key mechanism for TypeScript support (line 259-262):**
```typescript
const workerOptions = this.config.workerScript.endsWith('.ts')
  ? { execArgv: ['--require', 'ts-node/register'] }
  : {};
```

The pool attempts to use ts-node for .ts files, but this fails in Jest because:
1. Jest already runs under ts-node
2. Worker threads spawn new Node.js processes that don't inherit Jest's ts-node configuration
3. The nested ts-node registration conflicts with Jest's setup

### Worker Classes

All workers follow the same pattern:

```
[Worker]Worker
├── constructor(pool: WorkerPool)
├── shouldUseWorker(size: number): boolean  // Threshold check (10 items)
├── operation(payload): Promise<Result>     // Calls inline or worker
├── operationInline(payload): Result        // Direct execution
└── Constants: WORKER_THRESHOLD = 10
```

**CRDTMergeWorker** (251 lines)
- `mergeLWW()` - Last-Write-Wins merge
- `mergeORMap()` - Observed-Remove Map merge
- Threshold: 10 records

**MerkleWorker** (374 lines)
- `computeHashes()` - Hash computation for entries
- `computeORMapHashes()` - ORMap-specific hashing
- `diff()` - Find differences between trees
- `rebuild()` - Rebuild tree from records
- Threshold: 10 entries

**SerializationWorker** (226 lines)
- `serializeBatch()` - Batch serialize to msgpack
- `deserializeBatch()` - Batch deserialize
- Threshold: 10 items OR 50KB payload size

### Worker Script Architecture

```
packages/server/src/workers/worker-scripts/
├── base.worker.ts    - Handler registration, message loop
├── crdt.worker.ts    - CRDT merge handlers
├── merkle.worker.ts  - Merkle tree handlers
├── serialization.worker.ts - Serialization handlers
└── test.worker.ts    - Test/echo handlers
```

The base worker provides the message handling infrastructure:
```typescript
if (parentPort) {
  parentPort.on('message', async (task: TaskMessage) => {
    const handler = handlers.get(type);
    if (!handler) throw new Error(`Unknown task type: ${type}`);
    const result = await handler(payload);
    parentPort!.postMessage({ id, success: true, result });
  });
}
```

## Root Cause Analysis

### Category 1: ts-node Worker Thread Limitation (6 tests)

**What goes wrong:**
1. Jest runs tests via ts-node (TypeScript to JavaScript JIT compilation)
2. WorkerPool creates workers with `new Worker(workerScript, { execArgv: ['--require', 'ts-node/register'] })`
3. Worker thread spawns a new Node.js process
4. ts-node in the worker tries to compile .ts files but encounters issues:
   - Module resolution differences between Jest and worker context
   - Missing jest configuration in worker process
   - Conflicting ts-node instances

**Evidence:**
- All skipped tests have explicit comments: "ts-node doesn't support workers with .ts files"
- Tests pass when compiled to .js first (comment: "These tests pass after compilation to .js")
- Inline operations (batch < 10) all pass - only worker thread paths fail

**Affected tests:**
- CRDTMergeWorker: 2 tests
- MerkleWorker: 3 tests
- SerializationWorker: 2 tests (1 describe block)

### Category 2: Complex E2E Setup (1 describe block)

**What goes wrong:**
1. DistributedSearch E2E requires full multi-node cluster setup
2. WebSocket connections, authentication, FTS indexing
3. Comment indicates: "require complex server setup"

**Evidence:**
- Similar tests exist that pass (distributed-subscriptions.integration.test.ts)
- The test structure matches working integration tests
- Skipped "in CI" suggests it may work locally with proper setup

**Key differences from working integration tests:**
- DistributedSearch uses simpler auth (`test-token` string)
- Working tests use proper JWT with `jwt.sign()`
- DistributedSearch uses internal `(node as any)` accessors

## Test Infrastructure

### Existing Patterns

**Jest Configuration (packages/server/jest.config.js):**
```javascript
module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  testMatch: [
    '<rootDir>/src/**/__tests__/**/*.test.ts',
    '<rootDir>/tests/**/*.test.ts'
  ],
  // No special handling for worker tests
};
```

**No slow test pattern exists** - The glob `**/*.slow.test.ts` returns no files.

**WorkerBenchmark.test.ts pattern:**
- Only tests inline operations (batch size < 10)
- Documents limitation: "Worker thread tests are skipped in Jest due to ts-node limitations"
- Provides benchmark infrastructure that could be extended

**Working integration test pattern (distributed-subscriptions.integration.test.ts):**
- Uses proper JWT authentication: `jwt.sign({ userId, roles }, JWT_SECRET)`
- Has helper functions: `createClient()`, `authenticateClient()`, `sendAndWait()`
- Uses WebSocket message handling with type checking
- 30-second beforeAll timeout for cluster setup

### Test Timeout Configuration

Current skipped tests have explicit timeouts:
- Worker thread tests: 15000ms (15 seconds)
- Performance test: 30000ms (30 seconds)
- E2E cluster setup: Would need 30000ms+

## Standard Stack

### For Worker Thread Testing in Jest

| Approach | When to Use | Tradeoff |
|----------|-------------|----------|
| Pre-compile workers | Production-like testing | Requires build step |
| Use `esbuild-register` | Faster compilation | Different module resolution |
| Mock worker threads | Unit testing | Doesn't test real worker behavior |
| Separate test runner | Integration/E2E | More complex CI setup |

### Recommended Stack

| Tool | Purpose | Version |
|------|---------|---------|
| Jest | Test runner | Already in use |
| tsup | Build workers before test | Already in use for build |
| jest-worker | Alternative worker abstraction | v29.x |

## Architecture Patterns

### Pattern 1: Build-Before-Test for Workers

**What:** Run `pnpm build` before worker tests, point tests at compiled .js files
**When:** Need to test actual worker thread behavior
**Advantage:** Tests real production behavior

```bash
# In CI/package.json
"test:workers": "pnpm build && jest --testPathPattern=workers"
```

### Pattern 2: Separate Slow Test File

**What:** Move large-batch tests to `*.slow.test.ts`, run separately
**When:** Want fast CI for regular tests, comprehensive tests on schedule
**Structure:**

```
src/__tests__/workers/
├── MerkleWorker.test.ts       # Inline tests only
├── MerkleWorker.slow.test.ts  # Worker thread + perf tests
└── workers.slow.config.js     # Jest config with build
```

### Pattern 3: Mock Worker Thread Path

**What:** Mock WorkerPool to always use inline execution in tests
**When:** Want to test business logic without worker complexity
**Limitation:** Doesn't validate actual worker thread behavior

### Anti-Patterns to Avoid

- **Skipping tests permanently:** Tests exist because behavior matters
- **Running ts-node workers:** Known to fail, wastes CI time
- **Testing only inline paths:** Misses worker-specific bugs (serialization, message passing)

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TypeScript worker compilation | Custom ts-node setup | tsup/esbuild pre-compile | Known working pattern |
| Slow test isolation | Manual test filtering | Jest `testPathIgnorePatterns` | Standard Jest feature |
| Worker thread mocking | Custom mock | jest-worker or manual mock | Consistent patterns |

## Common Pitfalls

### Pitfall 1: Nested ts-node Registration
**What goes wrong:** ts-node inside worker fails when Jest already uses ts-node
**Why it happens:** Conflicting module resolution, different contexts
**How to avoid:** Pre-compile workers to JavaScript before running tests
**Warning signs:** "Cannot find module" errors in worker, silent test hangs

### Pitfall 2: Test Timeout Too Short
**What goes wrong:** Worker thread startup takes time, tests timeout
**Why it happens:** Worker spawn + compilation overhead
**How to avoid:** Use 15-30 second timeouts for worker tests
**Warning signs:** Tests pass locally, fail in CI (slower machines)

### Pitfall 3: Port Conflicts in E2E
**What goes wrong:** Multiple test files use same ports, tests fail randomly
**Why it happens:** Tests don't clean up servers properly
**How to avoid:** Use port 0 for dynamic allocation, proper afterAll cleanup
**Warning signs:** "EADDRINUSE" errors, tests pass in isolation but fail in suite

### Pitfall 4: Auth Token Format
**What goes wrong:** E2E tests fail authentication
**Why it happens:** Using plain string tokens instead of JWT
**How to avoid:** Use `jwt.sign()` with proper secret matching server config
**Warning signs:** "AUTH_FAIL" or "unauthorized" responses

## Recommended Approach

### For Worker Thread Tests (BUG-01, BUG-02, BUG-03)

**Option A: Pre-compile approach (Recommended)**
1. Add build step before worker tests
2. Configure WorkerPool to use .js files in test environment
3. Run tests against compiled workers

**Option B: Separate slow test file**
1. Create `*.slow.test.ts` pattern
2. Move worker thread tests there
3. Configure Jest to exclude slow tests by default
4. Run slow tests with separate command after build

**Option C: Reduce test scope**
1. Unskip tests
2. Lower batch sizes to stay in inline threshold
3. Accept that worker thread paths are tested only via integration

### For DistributedSearch E2E (BUG-04)

1. Fix authentication to use JWT (match working integration tests)
2. Use proper cluster setup helpers
3. Add sufficient timeouts (30s setup, 15s per test)
4. Ensure proper cleanup in afterAll

## Code Examples

### Pre-compile Jest Configuration

```javascript
// jest.workers.config.js
module.exports = {
  ...require('./jest.config'),
  testMatch: ['**/*.worker.test.ts'],
  globalSetup: '<rootDir>/test-setup/build-workers.js',
};

// test-setup/build-workers.js
const { execSync } = require('child_process');
module.exports = async () => {
  execSync('pnpm build', { stdio: 'inherit' });
};
```

### Slow Test File Pattern

```typescript
// MerkleWorker.slow.test.ts
/**
 * MerkleWorker Slow Tests
 * Run: pnpm test:slow
 * Requires: pnpm build first
 */

describe('MerkleWorker Performance (Worker Thread)', () => {
  // Use 30-second timeout for worker tests
  jest.setTimeout(30000);

  it('should handle large batches via worker thread', async () => {
    const entries = Array.from({ length: 1000 }, (_, i) => ({
      key: `key-${i}`,
      timestamp: { millis: Date.now() + i, counter: 0, nodeId: 'test' },
    }));

    const result = await merkleWorker.computeHashes({ entries });
    expect(result.hashes).toHaveLength(1000);
  });
});
```

### Fixed E2E Auth Pattern

```typescript
// DistributedSearch.e2e.test.ts - fixed auth
import jwt from 'jsonwebtoken';

const JWT_SECRET = 'test-secret-for-e2e-tests';

function createTestToken(userId = 'test-user'): string {
  return jwt.sign({ userId, roles: ['ADMIN'] }, JWT_SECRET, { expiresIn: '1h' });
}

// In server setup:
const node1 = new ServerCoordinator({
  port: 0,
  jwtSecret: JWT_SECRET,  // Must match token signing
  // ...
});

// In client auth:
ws.send(serialize({
  type: 'AUTH',
  token: createTestToken(),  // Not 'test-token' string
}));
```

## Dependencies and Risks

### Dependencies

| Dependency | Why Needed | Risk if Missing |
|------------|------------|-----------------|
| `tsup` or build tool | Compile workers before test | Worker tests cannot run |
| `jsonwebtoken` | E2E auth | Already present |
| Port availability | E2E servers | Use port 0 |

### Cross-cutting Concerns

1. **CI Configuration** - May need updated CI script to build before worker tests
2. **Test Isolation** - All 4 affected test files share WorkerPool setup
3. **Build Output** - Tests need access to compiled .js workers

### Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Build step adds time | CI slower | Only run for worker tests |
| Path differences dev/CI | Tests fail in CI only | Use relative paths, test in CI |
| Flaky cluster tests | CI false failures | Add retries, increase timeouts |

## Open Questions

1. **Build output location:** Should compiled workers go to `dist/` or `src/workers/worker-scripts/*.js`?
   - Recommendation: Build to `dist/`, configure WorkerPool to resolve correctly

2. **Slow test threshold:** What batch size constitutes "slow"?
   - Currently: 10+ triggers worker
   - Recommendation: Keep 10 for inline, use 100+ for slow tests

3. **CI strategy:** Build once and test, or build-per-test-file?
   - Recommendation: Single build step, all tests use compiled output

## Sources

### Primary (HIGH confidence)
- packages/server/src/workers/WorkerPool.ts - Worker architecture
- packages/server/src/__tests__/workers/*.test.ts - Skip comments explain reasons
- packages/server/jest.config.js - Current test configuration

### Secondary (MEDIUM confidence)
- packages/server/src/__tests__/integration/distributed-subscriptions.integration.test.ts - Working E2E pattern

### Tertiary (LOW confidence)
- Node.js worker_threads documentation - ts-node limitations are known
- Jest documentation - Worker testing patterns

## Metadata

**Confidence breakdown:**
- Skipped test inventory: HIGH - Direct code inspection
- Root cause (ts-node): HIGH - Code comments explicitly state reason
- Worker architecture: HIGH - Full code review
- Fix approach: MEDIUM - Standard patterns, not yet validated

**Research date:** 2026-01-18
**Valid until:** 60 days (stable area, no external dependencies)
