# Testing Patterns

**Analysis Date:** 2026-01-18

## Test Framework

**Runner:**
- Jest 29.7.0 (primary runner for all packages)
- ts-jest 29.4.5 (TypeScript transformer)
- Vitest (benchmarks only in `@topgunbuild/core`)

**Config Files:**
- Root: `/jest.config.js`
- Core: `/packages/core/jest.config.js`
- Server: `/packages/server/jest.config.js`
- Client: `/packages/client/jest.config.js`
- E2E: `/tests/e2e/jest.config.js`

**Assertion Library:**
- Jest built-in `expect()` matchers

**Run Commands:**
```bash
# Run all tests
pnpm test

# Run tests for specific package
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/server test
pnpm --filter @topgunbuild/client test

# Run single test file
cd packages/server && pnpm test -- --testPathPattern="Cluster"

# Watch mode
pnpm --filter @topgunbuild/core test -- --watch

# Coverage
pnpm test:coverage
pnpm --filter @topgunbuild/core test:coverage

# E2E tests
pnpm test:e2e
pnpm test:e2e:coverage

# Benchmarks (Vitest)
pnpm --filter @topgunbuild/core bench
```

## Test File Organization

**Location:** Co-located with source in `__tests__` directories
```
packages/core/src/
├── HLC.ts
├── LWWMap.ts
├── __tests__/
│   ├── HLC.test.ts
│   ├── LWWMap.test.ts
│   ├── LWWMap.properties.test.ts
│   └── query/
│       ├── Attribute.test.ts
│       └── IndexRegistry.test.ts
```

**Naming:**
- Unit tests: `{ClassName}.test.ts`
- Property tests: `{ClassName}.properties.test.ts`
- Integration tests: `{Feature}.integration.test.ts`

**E2E Structure:**
```
tests/e2e/
├── jest.config.js
├── helpers/
│   ├── setup.ts
│   ├── index.ts
│   └── MemoryStorageAdapter.ts
├── basic-sync.test.ts
├── multi-client.test.ts
├── offline-online.test.ts
├── cluster/
│   ├── node-failure.test.ts
│   └── partition-routing.test.ts
```

**Test Match Patterns (from configs):**
```javascript
// Core/Client packages
testMatch: ['<rootDir>/src/**/__tests__/**/*.test.ts']

// Server package
testMatch: [
  '<rootDir>/src/**/__tests__/**/*.test.ts',
  '<rootDir>/tests/**/*.test.ts'
]

// E2E tests
testMatch: ['<rootDir>/**/*.test.ts']
```

## Test Structure

**Suite Organization:**
```typescript
describe('HLC (Hybrid Logical Clock)', () => {
  let hlc: HLC;

  beforeEach(() => {
    hlc = new HLC('test-node');
    jest.restoreAllMocks();
  });

  describe('Timestamp Creation', () => {
    test('should create a new HLC timestamp with correct structure', () => {
      const ts = hlc.now();

      expect(ts).toHaveProperty('millis');
      expect(ts).toHaveProperty('counter');
      expect(ts).toHaveProperty('nodeId');
      expect(typeof ts.millis).toBe('number');
      expect(ts.nodeId).toBe('test-node');
    });

    test('should generate monotonically increasing timestamps', () => {
      const ts1 = hlc.now();
      const ts2 = hlc.now();

      expect(HLC.compare(ts1, ts2)).toBeLessThan(0);
    });
  });

  describe('Edge Cases', () => {
    test('should handle same wall-clock time across multiple calls', () => {
      // Test implementation
    });
  });
});
```

**Patterns:**
- Outer `describe` for class/module name
- Nested `describe` for feature groups
- Use `test()` or `it()` interchangeably (project uses `test()`)
- Descriptive names starting with "should"

**Setup/Teardown:**
```typescript
// Per-test setup (most common)
beforeEach(() => {
  hlc = new HLC('test-node');
});

// Suite-level for expensive setup (servers)
beforeAll(async () => {
  server = await createTestServer();
});

afterAll(async () => {
  await server.shutdown();
});

// Cleanup after each test
afterEach(async () => {
  if (pool) {
    await pool.shutdown(5000);
  }
});
```

## Mocking

**Framework:** Jest built-in mocks

**Time Mocking:**
```typescript
test('should respect TTL options', () => {
  const now = Date.now();
  jest.spyOn(Date, 'now').mockImplementation(() => now);

  map.set('tempKey', 'tempVal', 100);
  expect(map.get('tempKey')).toBe('tempVal');

  // Advance time
  jest.spyOn(Date, 'now').mockImplementation(() => now + 150);
  expect(map.get('tempKey')).toBeUndefined();

  // Cleanup
  jest.restoreAllMocks();
});
```

**Console Mocking:**
```typescript
test('should warn on clock drift', () => {
  const consoleWarnSpy = jest.spyOn(console, 'warn').mockImplementation();

  // ... test code that triggers warning

  expect(consoleWarnSpy).toHaveBeenCalledWith(
    expect.stringContaining('Clock drift detected')
  );

  consoleWarnSpy.mockRestore();
});
```

**Mock Socket/Writer:**
```typescript
const createMockWriter = (socket: any) => ({
  write: jest.fn((message: any, _urgent?: boolean) => {
    const data = serialize(message);
    socket.send(data);
  }),
  writeRaw: jest.fn((data: Uint8Array) => {
    socket.send(data);
  }),
  flush: jest.fn(),
  close: jest.fn(),
  getMetrics: jest.fn(() => ({
    messagesSent: 0,
    batchesSent: 0,
    bytesSent: 0,
  })),
});

const clientSocket = {
  send: jest.fn(),
  readyState: 1 // OPEN
};
```

**What to Mock:**
- External time (`Date.now()`)
- Console methods for warning tests
- WebSocket connections in integration tests
- Internal server methods for unit tests (`(server as any).handleMessage`)

**What NOT to Mock:**
- CRDT logic (test actual merge behavior)
- HLC timestamp generation (critical algorithm)
- Serialization/deserialization (use real msgpackr)

## Fixtures and Factories

**Test Data Factories (`tests/e2e/helpers/index.ts`):**
```typescript
/**
 * Creates an LWW record for testing
 */
export function createLWWRecord<T>(value: T, nodeId = 'test-node'): any {
  return {
    value,
    timestamp: {
      millis: Date.now(),
      counter: 0,
      nodeId,
    },
  };
}

/**
 * Creates an OR record for testing
 */
export function createORRecord<T>(value: T, nodeId = 'test-node'): any {
  const ts = Date.now();
  return {
    value,
    timestamp: {
      millis: ts,
      counter: 0,
      nodeId,
    },
    tag: `${nodeId}-${ts}-0`,
  };
}
```

**Test Server Factory:**
```typescript
export async function createTestServer(
  overrides: Partial<ServerCoordinatorConfig> = {}
): Promise<ServerCoordinator> {
  const server = new ServerCoordinator({
    port: 0, // Let OS assign port
    nodeId: `test-server-${Date.now()}`,
    host: 'localhost',
    clusterPort: 0,
    metricsPort: 0,
    peers: [],
    jwtSecret: JWT_SECRET,
    ...overrides,
  });

  await server.ready();
  return server;
}
```

**Test Context Factory:**
```typescript
export async function createTestContext(
  numClients = 1,
  serverConfig: Partial<ServerCoordinatorConfig> = {}
): Promise<TestContext> {
  const server = await createTestServer(serverConfig);
  const serverUrl = `ws://localhost:${server.port}`;

  const clients: TestClient[] = [];
  for (let i = 0; i < numClients; i++) {
    const client = await createTestClient(serverUrl, {
      nodeId: `client-${i}`,
      userId: `user-${i}`,
      roles: ['ADMIN'],
    });
    clients.push(client);
  }

  await Promise.all(clients.map(c => c.waitForMessage('AUTH_ACK')));

  return {
    server,
    clients,
    cleanup: async () => {
      for (const client of clients) {
        client.close();
      }
      await server.shutdown();
    },
  };
}
```

**Location:**
- E2E fixtures: `tests/e2e/helpers/`
- Unit test fixtures: inline in test files or `__tests__/utils/`

## Coverage

**Requirements:**
| Package | Lines | Branches | Functions | Statements |
|---------|-------|----------|-----------|------------|
| core    | 70%   | 60%      | 60%       | 70%        |
| server  | 65%   | 55%      | 60%       | 65%        |
| client  | 70%   | 60%      | 60%       | 70%        |

**Configuration (from `packages/server/jest.config.js`):**
```javascript
collectCoverageFrom: [
  'src/**/*.ts',
  '!src/**/*.test.ts',
  '!src/**/__tests__/**',
  '!src/**/index.ts',
  '!src/start-server.ts'
],
coverageDirectory: 'coverage',
coverageReporters: ['text', 'text-summary', 'html', 'lcov'],
coverageThreshold: {
  global: {
    lines: 65,
    branches: 55,
    functions: 60,
    statements: 65
  }
}
```

**View Coverage:**
```bash
# Generate coverage report
pnpm --filter @topgunbuild/core test:coverage

# Open HTML report
open packages/core/coverage/index.html
```

## Test Types

**Unit Tests:**
- Scope: Single class/function
- Location: `packages/*/src/__tests__/*.test.ts`
- No external dependencies
- Fast execution
```typescript
describe('LWWMap', () => {
  test('should set and get values', () => {
    map.set('key1', 'value1');
    expect(map.get('key1')).toBe('value1');
  });
});
```

**Integration Tests:**
- Scope: Multiple components interacting
- Location: `packages/*/src/__tests__/*.integration.test.ts`
- May use real server instances
- Timeout: 30000ms (configured in client jest.config.js)
```typescript
describe('Sync Protocol Integration', () => {
  let server: ServerCoordinator;

  beforeAll(async () => {
    server = new ServerCoordinator({ ... });
    await server.ready();
  });

  test('Should handle OP_BATCH and send OP_ACK', async () => {
    // Inject mock client, test message flow
  });
});
```

**E2E Tests:**
- Scope: Full client-server communication
- Location: `tests/e2e/`
- Real WebSocket connections
- Timeout: 30000ms
- Port range: 10000+ for servers
```typescript
describe('E2E: Basic Sync', () => {
  describe('Connection', () => {
    let server: ServerCoordinator;

    beforeEach(async () => {
      server = await createTestServer();
    });

    test('client successfully connects to server', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`);
      expect(client.ws.readyState).toBe(WebSocket.OPEN);
      client.close();
    });
  });
});
```

## Common Patterns

**Async Testing:**
```typescript
// Async with await
test('should handle async operations', async () => {
  const result = await asyncOperation();
  expect(result).toBeDefined();
});

// Promise rejection testing
test('should reject tasks after shutdown', async () => {
  await pool.shutdown(1000);
  await expect(pool.submit(task)).rejects.toThrow(WorkerPoolShutdownError);
});

// Wait for condition
await waitUntil(
  () => client2.messages.some(m => m.type === 'QUERY_UPDATE'),
  3000
);
```

**Error Testing:**
```typescript
// Test error is thrown
test('should throw error for invalid format', () => {
  expect(() => HLC.parse('invalid')).toThrow('Invalid timestamp format: invalid');
});

// Test async rejection
test('should timeout long-running tasks', async () => {
  const task = createTask('slow-task', { delay: 5000 });
  await expect(pool.submit(task)).rejects.toThrow(WorkerTimeoutError);
});

// Test error properties
test('WorkerTimeoutError should contain task details', () => {
  const error = new WorkerTimeoutError('task-123', 5000);
  expect(error.name).toBe('WorkerTimeoutError');
  expect(error.taskId).toBe('task-123');
  expect(error.timeout).toBe(5000);
});
```

**Helper Wait Functions:**
```typescript
// Simple delay
export function waitForSync(ms = 100): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// Condition-based wait
export async function waitUntil(
  condition: () => boolean | Promise<boolean>,
  timeout = 5000,
  interval = 50
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await condition()) return;
    await waitForSync(interval);
  }
  throw new Error('waitUntil timeout');
}
```

**Test Timeouts:**
```typescript
// Per-test timeout
test('scaling should complete', async () => {
  // long running test
}, 15000);

// Global timeout in config
testTimeout: 30000
```

## Test Setup Files

**E2E Setup (`tests/e2e/helpers/setup.ts`):**
```typescript
// Increase timeout for E2E tests
jest.setTimeout(30000);

// Suppress console logs during tests unless DEBUG=true
if (!process.env.DEBUG) {
  global.console = {
    ...console,
    log: jest.fn(),
    debug: jest.fn(),
    info: jest.fn(),
    warn: jest.fn(),
  };
}
```

**Module Name Mapper (all configs):**
```javascript
moduleNameMapper: {
  '^@topgunbuild/core$': '<rootDir>/../core/src/index.ts',
  '^@topgunbuild/server$': '<rootDir>/src/index.ts',
  '^@topgunbuild/client$': '<rootDir>/../client/src/index.ts'
}
```

## Benchmarks

**Framework:** Vitest (bench mode)

**Config (`packages/core/vitest.config.ts`):**
```typescript
import { defineConfig } from 'vitest/config';

const isCI = process.env.CI === 'true';

export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    benchmark: {
      include: ['src/__benchmarks__/**/*.bench.ts'],
      ...(isCI && {
        warmupIterations: 3,
        iterations: 50,
        time: 500,
      }),
    },
  },
});
```

**Location:** `packages/core/src/__benchmarks__/*.bench.ts`

**Run:**
```bash
pnpm --filter @topgunbuild/core bench
```

## Port Management

**Test Ports:**
- Server tests: 10000+ range
- Cluster tests: 11000+ range
- Dynamic port assignment: `port: 0` (let OS assign)

**Port Counter Pattern:**
```typescript
let portCounter = 10000;

function getNextPort(): number {
  return portCounter++;
}
```

**CI Note:** Run tests sequentially to avoid port conflicts:
```bash
pnpm test -- --runInBand
```

---

*Testing analysis: 2026-01-18*
