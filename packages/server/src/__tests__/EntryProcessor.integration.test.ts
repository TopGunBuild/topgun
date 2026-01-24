/**
 * Entry Processor Integration Tests
 *
 * Tests for Phase 5.03: Entry Processor feature
 * - Server-side atomic operations on map entries
 * - Built-in processors
 * - Custom processor execution
 * - Concurrent execution
 */

import { ServerCoordinator, ServerCoordinatorConfig } from '../ServerCoordinator';
import { TopGunClient } from '@topgunbuild/client';
import { MemoryStorageAdapter } from '@topgunbuild/adapters';
import { BuiltInProcessors } from '@topgunbuild/core';
import { waitForConnection } from './utils/test-helpers';

const TEST_PORT_BASE = 12100;

describe('Entry Processor Integration', () => {
  let server: ServerCoordinator;
  let client: TopGunClient;
  let port: number;

  beforeEach(async () => {
    port = TEST_PORT_BASE + Math.floor(Math.random() * 1000);

    const serverConfig: ServerCoordinatorConfig = {
      port,
      nodeId: 'test-server',
      jwtSecret: 'test-secret',
    };

    server = new ServerCoordinator(serverConfig);
    await server.ready();

    client = new TopGunClient({
      serverUrl: `ws://localhost:${port}`,
      storage: new MemoryStorageAdapter(),
      nodeId: 'test-client',
    });

    await client.start();
    client.setAuthToken(
      require('jsonwebtoken').sign({ sub: 'test-user', role: 'admin' }, 'test-secret'),
    );

    // Wait for connection with bounded polling
    await waitForConnection(client, 'CONNECTED', 5000);
  }, 10000);

  afterEach(async () => {
    client?.close();
    await server?.shutdown();
  });

  describe('executeOnKey', () => {
    it('should execute INCREMENT processor', async () => {
      // Set initial value
      const map = client.getMap<string, number>('counters');
      map.set('views', 100);

      // Wait for sync
      await new Promise((r) => setTimeout(r, 200));

      // Execute processor
      const result = await client.executeOnKey('counters', 'views', BuiltInProcessors.INCREMENT(5));

      expect(result.success).toBe(true);
      expect(result.result).toBe(105);
      expect(result.newValue).toBe(105);
    });

    it('should execute INCREMENT on non-existent key', async () => {
      const result = await client.executeOnKey(
        'counters',
        'newCounter',
        BuiltInProcessors.INCREMENT(10),
      );

      expect(result.success).toBe(true);
      expect(result.result).toBe(10);
      expect(result.newValue).toBe(10);
    });

    it('should execute DECREMENT_FLOOR with flooring', async () => {
      const map = client.getMap<string, number>('inventory');
      map.set('stock', 3);

      await new Promise((r) => setTimeout(r, 200));

      const result = await client.executeOnKey(
        'inventory',
        'stock',
        BuiltInProcessors.DECREMENT_FLOOR(10),
      );

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(0);
      expect(result.result).toEqual({ newValue: 0, wasFloored: true });
    });

    it('should execute PUT_IF_ABSENT for new key', async () => {
      const result = await client.executeOnKey(
        'users',
        'user1',
        BuiltInProcessors.PUT_IF_ABSENT({ name: 'John', role: 'admin' }),
      );

      expect(result.success).toBe(true);
      expect(result.result).toBe(true);
      expect(result.newValue).toEqual({ name: 'John', role: 'admin' });
    });

    it('should execute PUT_IF_ABSENT for existing key', async () => {
      const map = client.getMap<string, object>('users');
      map.set('user1', { name: 'Existing' });

      await new Promise((r) => setTimeout(r, 200));

      const result = await client.executeOnKey(
        'users',
        'user1',
        BuiltInProcessors.PUT_IF_ABSENT({ name: 'New' }),
      );

      expect(result.success).toBe(true);
      expect(result.result).toBe(false);
      expect((result.newValue as any).name).toBe('Existing');
    });

    it('should execute DELETE_IF_EQUALS', async () => {
      const map = client.getMap<string, string>('cache');
      map.set('key1', 'value1');

      await new Promise((r) => setTimeout(r, 200));

      const result = await client.executeOnKey(
        'cache',
        'key1',
        BuiltInProcessors.DELETE_IF_EQUALS('value1'),
      );

      expect(result.success).toBe(true);
      expect(result.result).toBe(true);
      expect(result.newValue).toBeUndefined();
    });

    it('should execute custom processor', async () => {
      const map = client.getMap<string, number>('scores');
      map.set('player1', 100);

      await new Promise((r) => setTimeout(r, 200));

      const result = await client.executeOnKey('scores', 'player1', {
        name: 'custom_multiply',
        code: `
          const current = value ?? 1;
          const newValue = current * args.multiplier;
          return { value: newValue, result: { oldValue: current, newValue } };
        `,
        args: { multiplier: 2 },
      });

      expect(result.success).toBe(true);
      expect(result.result).toEqual({ oldValue: 100, newValue: 200 });
      expect(result.newValue).toBe(200);
    });

    it('should handle processor errors gracefully', async () => {
      const result = await client.executeOnKey('test', 'key', {
        name: 'error_processor',
        code: `
          throw new Error('Intentional error');
        `,
      });

      expect(result.success).toBe(false);
      expect(result.error).toContain('Intentional error');
    });

    it('should reject invalid processor code', async () => {
      const result = await client.executeOnKey('test', 'key', {
        name: 'invalid',
        code: `
          eval("dangerous");
          return { value: 1 };
        `,
      });

      expect(result.success).toBe(false);
      expect(result.error).toBeDefined();
    });
  });

  describe('executeOnKeys', () => {
    it('should execute processor on multiple keys', async () => {
      const map = client.getMap<string, number>('counters');
      map.set('a', 10);
      map.set('b', 20);
      map.set('c', 30);

      await new Promise((r) => setTimeout(r, 200));

      const results = await client.executeOnKeys(
        'counters',
        ['a', 'b', 'c'],
        BuiltInProcessors.INCREMENT(5),
      );

      expect(results.size).toBe(3);

      const resultA = results.get('a');
      expect(resultA?.success).toBe(true);
      expect(resultA?.result).toBe(15);

      const resultB = results.get('b');
      expect(resultB?.success).toBe(true);
      expect(resultB?.result).toBe(25);

      const resultC = results.get('c');
      expect(resultC?.success).toBe(true);
      expect(resultC?.result).toBe(35);
    });

    it('should handle mixed success/failure', async () => {
      const map = client.getMap<string, any>('mixed');
      map.set('valid', 10);
      map.set('invalid', 'not a number');

      await new Promise((r) => setTimeout(r, 200));

      const results = await client.executeOnKeys('mixed', ['valid', 'invalid', 'missing'], {
        name: 'double',
        code: `
          const num = value ?? 0;
          if (typeof num !== 'number') {
            throw new Error('Not a number');
          }
          return { value: num * 2, result: num * 2 };
        `,
      });

      expect(results.size).toBe(3);

      // Valid key should succeed
      const validResult = results.get('valid');
      expect(validResult?.success).toBe(true);
      expect(validResult?.result).toBe(20);

      // Invalid key should fail
      const invalidResult = results.get('invalid');
      expect(invalidResult?.success).toBe(false);

      // Missing key should start from 0
      const missingResult = results.get('missing');
      expect(missingResult?.success).toBe(true);
      expect(missingResult?.result).toBe(0);
    });
  });

  describe('concurrent execution', () => {
    it('should handle concurrent increments correctly', async () => {
      // Fire 10 concurrent increments
      const promises = Array(10)
        .fill(null)
        .map(() =>
          client.executeOnKey('concurrent', 'counter', BuiltInProcessors.INCREMENT(1)),
        );

      const results = await Promise.all(promises);

      // All should succeed
      expect(results.every((r) => r.success)).toBe(true);

      // Final value should be 10 (server handles atomicity)
      const finalResult = await client.executeOnKey(
        'concurrent',
        'counter',
        BuiltInProcessors.GET(),
      );

      expect(finalResult.result).toBe(10);
    });

    it('should handle conditional updates with contention', async () => {
      // Set initial value with version
      const map = client.getMap<string, { data: string; version: number }>('versioned');
      map.set('doc', { data: 'initial', version: 1 });

      await new Promise((r) => setTimeout(r, 200));

      // Try to update with version check
      const result1 = await client.executeOnKey('versioned', 'doc', {
        name: 'conditional_update',
        code: `
          if (!value || value.version !== args.expectedVersion) {
            return { value, result: { success: false, reason: 'version mismatch' } };
          }
          return {
            value: { data: args.newData, version: value.version + 1 },
            result: { success: true, newVersion: value.version + 1 }
          };
        `,
        args: { expectedVersion: 1, newData: 'updated' },
      });

      expect(result1.success).toBe(true);
      expect((result1.result as any).success).toBe(true);

      // Second update with old version should fail
      const result2 = await client.executeOnKey('versioned', 'doc', {
        name: 'conditional_update',
        code: `
          if (!value || value.version !== args.expectedVersion) {
            return { value, result: { success: false, reason: 'version mismatch' } };
          }
          return {
            value: { data: args.newData, version: value.version + 1 },
            result: { success: true, newVersion: value.version + 1 }
          };
        `,
        args: { expectedVersion: 1, newData: 'should fail' },
      });

      expect(result2.success).toBe(true); // Processor executed successfully
      expect((result2.result as any).success).toBe(false); // But update was rejected
      expect((result2.result as any).reason).toBe('version mismatch');
    });
  });

  describe('array operations', () => {
    it('should push items to array', async () => {
      const result1 = await client.executeOnKey('lists', 'items', BuiltInProcessors.ARRAY_PUSH('a'));
      expect(result1.success).toBe(true);
      expect(result1.newValue).toEqual(['a']);

      const result2 = await client.executeOnKey('lists', 'items', BuiltInProcessors.ARRAY_PUSH('b'));
      expect(result2.success).toBe(true);
      expect(result2.newValue).toEqual(['a', 'b']);
    });

    it('should pop items from array', async () => {
      // Setup array
      await client.executeOnKey('lists', 'stack', BuiltInProcessors.ARRAY_PUSH(1));
      await client.executeOnKey('lists', 'stack', BuiltInProcessors.ARRAY_PUSH(2));
      await client.executeOnKey('lists', 'stack', BuiltInProcessors.ARRAY_PUSH(3));

      // Pop
      const result = await client.executeOnKey('lists', 'stack', BuiltInProcessors.ARRAY_POP());

      expect(result.success).toBe(true);
      expect(result.result).toBe(3);
      expect(result.newValue).toEqual([1, 2]);
    });
  });
});
