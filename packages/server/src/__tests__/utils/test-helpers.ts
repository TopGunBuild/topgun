/**
 * Test utility module with hardened timeout/polling helpers.
 *
 * All polling functions have:
 * - Bounded iteration limits
 * - Clear timeout errors with description
 * - Consistent PollOptions interface
 */

import type { ServerCoordinator } from '../../ServerCoordinator';
import type { LWWMap } from '@topgunbuild/core';
import type { TopGunClient, SyncEngine } from '@topgunbuild/client';
import { createTestHarness } from './ServerTestHarness';

export interface PollOptions {
  /** Max wait time in milliseconds (default: 5000) */
  timeoutMs?: number;
  /** Poll interval in milliseconds (default: 100) */
  intervalMs?: number;
  /** Max poll attempts (default: timeoutMs/intervalMs) */
  maxIterations?: number;
  /** Description for error messages */
  description?: string;
}

const DEFAULT_TIMEOUT_MS = 5000;
const DEFAULT_INTERVAL_MS = 100;

/**
 * Poll until condition returns true, with bounded iterations.
 *
 * @throws Error if timeout or max iterations exceeded
 */
export async function pollUntil(
  condition: () => boolean | Promise<boolean>,
  options: PollOptions = {}
): Promise<void> {
  const {
    timeoutMs = DEFAULT_TIMEOUT_MS,
    intervalMs = DEFAULT_INTERVAL_MS,
    maxIterations = Math.ceil(timeoutMs / intervalMs),
    description = 'condition',
  } = options;

  const startTime = Date.now();
  let iterations = 0;

  while (iterations < maxIterations) {
    const result = await condition();
    if (result) {
      return;
    }

    const elapsed = Date.now() - startTime;
    if (elapsed >= timeoutMs) {
      throw new Error(
        `pollUntil timed out after ${elapsed}ms waiting for ${description}. ` +
          `Iterations: ${iterations}/${maxIterations}`
      );
    }

    iterations++;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  throw new Error(
    `pollUntil exceeded max iterations (${maxIterations}) waiting for ${description}. ` +
      `Elapsed: ${Date.now() - startTime}ms`
  );
}

/**
 * Poll until condition returns non-null value, with bounded iterations.
 *
 * @returns The non-null value once condition is met
 * @throws Error if timeout or max iterations exceeded
 */
export async function pollUntilValue<T>(
  getter: () => T | null | undefined | Promise<T | null | undefined>,
  options: PollOptions = {}
): Promise<T> {
  const {
    timeoutMs = DEFAULT_TIMEOUT_MS,
    intervalMs = DEFAULT_INTERVAL_MS,
    maxIterations = Math.ceil(timeoutMs / intervalMs),
    description = 'value',
  } = options;

  const startTime = Date.now();
  let iterations = 0;
  let lastValue: T | null | undefined;

  while (iterations < maxIterations) {
    lastValue = await getter();
    if (lastValue != null) {
      return lastValue;
    }

    const elapsed = Date.now() - startTime;
    if (elapsed >= timeoutMs) {
      throw new Error(
        `pollUntilValue timed out after ${elapsed}ms waiting for ${description}. ` +
          `Iterations: ${iterations}/${maxIterations}, Last value: ${JSON.stringify(lastValue)}`
      );
    }

    iterations++;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  throw new Error(
    `pollUntilValue exceeded max iterations (${maxIterations}) waiting for ${description}. ` +
      `Elapsed: ${Date.now() - startTime}ms, Last value: ${JSON.stringify(lastValue)}`
  );
}

/**
 * Wait for server ready with timeout.
 *
 * @param server The ServerCoordinator instance
 * @param timeoutMs Maximum wait time (default: 5000)
 */
export async function waitForServerReady(
  server: ServerCoordinator,
  timeoutMs = DEFAULT_TIMEOUT_MS
): Promise<void> {
  const timeoutPromise = new Promise<never>((_, reject) => {
    setTimeout(
      () => reject(new Error(`Server ready timed out after ${timeoutMs}ms`)),
      timeoutMs
    );
  });

  await Promise.race([server.ready(), timeoutPromise]);
}

/**
 * Wait for cluster formation with bounded polling.
 *
 * @param nodes Array of ServerCoordinator instances
 * @param expectedSize Expected number of nodes in cluster
 * @param timeoutMs Maximum wait time (default: 15000 for cluster operations)
 */
export async function waitForCluster(
  nodes: ServerCoordinator[],
  expectedSize: number,
  timeoutMs = 15000
): Promise<void> {
  await pollUntil(
    () => {
      for (const node of nodes) {
        const harness = createTestHarness(node);
        const members = harness.cluster?.getMembers() || [];
        if (members.length < expectedSize) {
          return false;
        }
      }
      return true;
    },
    {
      timeoutMs,
      intervalMs: 200,
      description: `cluster formation (expected ${expectedSize} nodes)`,
    }
  );
}

/**
 * Wait for client connection state with bounded polling.
 *
 * @param client TopGunClient or SyncEngine instance
 * @param targetState Target state to wait for (default: 'CONNECTED')
 * @param timeoutMs Maximum wait time (default: 5000)
 */
export async function waitForConnection(
  client: TopGunClient | SyncEngine,
  targetState = 'CONNECTED',
  timeoutMs = DEFAULT_TIMEOUT_MS
): Promise<void> {
  await pollUntil(
    () => client.getConnectionState() === targetState,
    {
      timeoutMs,
      intervalMs: 50,
      description: `client connection state ${targetState}`,
    }
  );
}

/**
 * Wait for map convergence between maps.
 *
 * Supports both the two-map signature (from Resilience.test.ts) and
 * the multi-map signature from the specification.
 *
 * @overload Two maps with key and expected value
 * @param mapA First LWWMap
 * @param mapB Second LWWMap
 * @param key Key to check
 * @param expectedValue Expected value at key
 * @param timeoutMs Maximum wait time (default: 10000)
 */
export async function waitForConvergence(
  mapA: LWWMap<any, any>,
  mapB: LWWMap<any, any>,
  key: string,
  expectedValue: any,
  timeoutMs?: number
): Promise<void>;

/**
 * @overload Array of maps with key and expected value
 * @param maps Array of LWWMaps to check
 * @param key Key to check
 * @param expectedValue Expected value at key
 * @param timeoutMs Maximum wait time (default: 10000)
 */
export async function waitForConvergence(
  maps: LWWMap<any, any>[],
  key: string,
  expectedValue: any,
  timeoutMs?: number
): Promise<void>;

export async function waitForConvergence(
  mapAOrMaps: LWWMap<any, any> | LWWMap<any, any>[],
  mapBOrKey: LWWMap<any, any> | string,
  keyOrExpected: string | any,
  expectedOrTimeout?: any | number,
  timeoutMsArg?: number
): Promise<void> {
  // Determine which overload was called
  let maps: LWWMap<any, any>[];
  let key: string;
  let expectedValue: any;
  let timeoutMs: number;

  if (Array.isArray(mapAOrMaps)) {
    // Array overload: (maps, key, expectedValue, timeoutMs?)
    maps = mapAOrMaps;
    key = mapBOrKey as string;
    expectedValue = keyOrExpected;
    timeoutMs = (expectedOrTimeout as number) ?? 10000;
  } else {
    // Two-map overload: (mapA, mapB, key, expectedValue, timeoutMs?)
    maps = [mapAOrMaps, mapBOrKey as LWWMap<any, any>];
    key = keyOrExpected as string;
    expectedValue = expectedOrTimeout;
    timeoutMs = timeoutMsArg ?? 10000;
  }

  const startTime = Date.now();

  await pollUntil(
    () => {
      for (const map of maps) {
        const val = map.get(key);
        if (val !== expectedValue) {
          return false;
        }
      }
      return true;
    },
    {
      timeoutMs,
      intervalMs: 100,
      description: `map convergence on key "${key}" to value "${JSON.stringify(expectedValue)}"`,
    }
  ).catch(() => {
    // Enhance error with current state
    const currentValues = maps.map((m, i) => `map[${i}]=${JSON.stringify(m.get(key))}`);
    throw new Error(
      `Convergence failed after ${Date.now() - startTime}ms. ` +
        `Expected: ${JSON.stringify(expectedValue)}, ` +
        `Current: ${currentValues.join(', ')}`
    );
  });
}

/**
 * Wait for a server map to contain an expected value at a specific key.
 *
 * Polls server.getMap(mapName).get(key) until it equals the expected value.
 * Useful for replacing fixed setTimeout delays in tests that wait for
 * operations to be applied to a server's in-memory CRDT map.
 *
 * @param server The ServerCoordinator instance
 * @param mapName Name of the map to poll
 * @param key Key within the map to check
 * @param expected Expected value at the key
 * @param opts Poll options (default timeout: 5000ms, interval: 100ms)
 */
export async function waitForMapValue(
  server: ServerCoordinator,
  mapName: string,
  key: string,
  expected: any,
  opts: PollOptions = {}
): Promise<void> {
  const startTime = Date.now();

  await pollUntil(
    () => {
      const map = server.getMap(mapName);
      return map.get(key) === expected;
    },
    {
      timeoutMs: opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
      intervalMs: opts.intervalMs ?? DEFAULT_INTERVAL_MS,
      maxIterations: opts.maxIterations,
      description:
        opts.description ??
        `map "${mapName}" key "${key}" to equal ${JSON.stringify(expected)}`,
    }
  ).catch(() => {
    const map = server.getMap(mapName);
    const current = map.get(key);
    throw new Error(
      `waitForMapValue failed after ${Date.now() - startTime}ms. ` +
        `Map: "${mapName}", Key: "${key}", ` +
        `Expected: ${JSON.stringify(expected)}, ` +
        `Current: ${JSON.stringify(current)}`
    );
  });
}

/**
 * Wait for all nodes to replicate a specific map value.
 *
 * Polls each node's getMap(mapName).get(key) until all nodes have the expected value.
 * Useful for replacing fixed setTimeout delays in cluster replication tests.
 *
 * @param nodes Array of ServerCoordinator instances to check
 * @param mapName Name of the map to poll
 * @param key Key within the map to check
 * @param expected Expected value at the key
 * @param opts Poll options (default timeout: 10000ms, interval: 200ms)
 */
export async function waitForReplication(
  nodes: ServerCoordinator[],
  mapName: string,
  key: string,
  expected: any,
  opts: PollOptions = {}
): Promise<void> {
  const startTime = Date.now();

  await pollUntil(
    () => {
      for (const node of nodes) {
        const map = node.getMap(mapName);
        if (map.get(key) !== expected) {
          return false;
        }
      }
      return true;
    },
    {
      timeoutMs: opts.timeoutMs ?? 10000,
      intervalMs: opts.intervalMs ?? 200,
      maxIterations: opts.maxIterations,
      description:
        opts.description ??
        `replication of "${mapName}" key "${key}" to ${JSON.stringify(expected)} across ${nodes.length} nodes`,
    }
  ).catch(() => {
    const currentValues = nodes.map((node, i) => {
      const map = node.getMap(mapName);
      return `node[${i}]=${JSON.stringify(map.get(key))}`;
    });
    throw new Error(
      `waitForReplication failed after ${Date.now() - startTime}ms. ` +
        `Map: "${mapName}", Key: "${key}", ` +
        `Expected: ${JSON.stringify(expected)}, ` +
        `Current: ${currentValues.join(', ')}`
    );
  });
}

/**
 * Wait for a Jest spy to have been called a minimum number of times.
 *
 * Polls the spy's mock.calls array length until it reaches the expected count.
 * Useful for replacing fixed setTimeout delays in tests that wait for
 * async callbacks, broadcasts, or event handlers to fire.
 *
 * @param spy Object with mock.calls array (Jest spy-compatible)
 * @param opts Poll options with optional callCount (default: 1, timeout: 5000ms, interval: 50ms)
 */
export async function waitForSpyCall(
  spy: { mock: { calls: any[][] } },
  opts: PollOptions & { callCount?: number } = {}
): Promise<void> {
  const expectedCount = opts.callCount ?? 1;

  await pollUntil(
    () => spy.mock.calls.length >= expectedCount,
    {
      timeoutMs: opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
      intervalMs: opts.intervalMs ?? 50,
      maxIterations: opts.maxIterations,
      description:
        opts.description ??
        `spy to be called at least ${expectedCount} time(s) (current: ${spy.mock.calls.length})`,
    }
  );
}

/**
 * Re-export waitForAuthReady for backward compatibility.
 * The existing import path should continue to work.
 */
export { waitForAuthReady } from './waitForAuthReady';
