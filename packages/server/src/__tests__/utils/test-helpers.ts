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
        const members = (node as any).cluster?.getMembers() || [];
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
 * Re-export waitForAuthReady for backward compatibility.
 * The existing import path should continue to work.
 */
export { waitForAuthReady } from './waitForAuthReady';
