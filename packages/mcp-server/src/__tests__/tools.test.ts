/**
 * MCP Tools Unit Tests
 */

import type { ToolContext, ResolvedMCPServerConfig } from '../types';
import { handleQuery } from '../tools/query';
import { handleMutate } from '../tools/mutate';
import { handleSchema } from '../tools/schema';
import { handleStats } from '../tools/stats';
import { handleExplain } from '../tools/explain';
import { handleListMaps } from '../tools/listMaps';

// Mock TopGunClient
class MockLWWMap {
  private data = new Map<string, unknown>();

  set(key: string, value: unknown) {
    this.data.set(key, value);
    return { value, timestamp: Date.now() };
  }

  get(key: string) {
    return this.data.get(key);
  }

  remove(key: string) {
    this.data.delete(key);
    return { value: null, timestamp: Date.now() };
  }

  entries() {
    return this.data.entries();
  }
}

class MockTopGunClient {
  private maps = new Map<string, MockLWWMap>();

  getMap(name: string): MockLWWMap {
    if (!this.maps.has(name)) {
      this.maps.set(name, new MockLWWMap());
    }
    return this.maps.get(name)!;
  }

  getConnectionState() {
    return 'connected';
  }

  isCluster() {
    return false;
  }

  getPendingOpsCount() {
    return 0;
  }

  isBackpressurePaused() {
    return false;
  }

  getConnectedNodes() {
    return [];
  }

  getPartitionMapVersion() {
    return 0;
  }

  isRoutingActive() {
    return false;
  }

  async search(_map: string, _query: string, _options?: unknown) {
    return [];
  }

  query(_map: string, _filter: unknown) {
    return {
      subscribe: () => () => {},
      dispose: () => {},
    };
  }
}

function createTestContext(config?: Partial<ResolvedMCPServerConfig>): ToolContext {
  return {
    client: new MockTopGunClient() as unknown as ToolContext['client'],
    config: {
      name: 'test-server',
      version: '1.0.0',
      topgunUrl: 'ws://localhost:8080',
      enableMutations: true,
      enableSubscriptions: true,
      defaultLimit: 10,
      maxLimit: 100,
      subscriptionTimeoutSeconds: 60,
      debug: false,
      ...config,
    },
  };
}

describe('MCP Tools', () => {
  describe('handleListMaps', () => {
    it('should list allowed maps when configured', async () => {
      const ctx = createTestContext({
        allowedMaps: ['tasks', 'users', 'products'],
      });

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('users');
      expect(result.content[0].text).toContain('products');
    });

    it('should indicate no restrictions when allowedMaps not set', async () => {
      const ctx = createTestContext();

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('all maps');
    });
  });

  describe('handleQuery', () => {
    it('should query an empty map', async () => {
      const ctx = createTestContext();

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });

    it('should return results from populated map', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Another Task', status: 'done' });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('2 result');
      expect(result.content[0].text).toContain('Test Task');
      expect(result.content[0].text).toContain('Another Task');
    });

    it('should filter results', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Done Task', status: 'done' });

      const result = await handleQuery(
        { map: 'tasks', filter: { status: 'done' } },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).toContain('Done Task');
      expect(result.content[0].text).not.toContain('Test Task');
    });

    it('should respect limit', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 20; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, index: i });
      }

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('5 result');
      expect(result.content[0].text).toContain('of 20 total');
    });

    it('should deny access to restricted maps', async () => {
      const ctx = createTestContext({
        allowedMaps: ['users'],
      });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });
  });

  describe('handleMutate', () => {
    it('should create a new record', async () => {
      const ctx = createTestContext();

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'New Task', status: 'todo' },
        },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully created');
      expect(result.content[0].text).toContain('task1');
    });

    it('should update an existing record', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Old Title', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Updated Title', status: 'done' },
        },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully updated');
    });

    it('should remove a record', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'To Delete', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'task1',
        },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully removed');
    });

    it('should warn when removing non-existent record', async () => {
      const ctx = createTestContext();

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'nonexistent',
        },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('does not exist');
    });

    it('should error when mutations are disabled', async () => {
      const ctx = createTestContext({ enableMutations: false });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Test' },
        },
        ctx
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should require data for set operation', async () => {
      const ctx = createTestContext();

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
        },
        ctx
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('"data" is required');
    });
  });

  describe('handleSchema', () => {
    it('should report empty map', async () => {
      const ctx = createTestContext();

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('empty');
    });

    it('should infer schema from data', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', {
        title: 'Test',
        count: 42,
        active: true,
        createdAt: '2025-01-01T00:00:00Z',
      });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('title: string');
      expect(result.content[0].text).toContain('count: number');
      expect(result.content[0].text).toContain('active: boolean');
      expect(result.content[0].text).toContain('timestamp');
    });

    it('should detect enum fields', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { status: 'todo' });
      map.set('task2', { status: 'in-progress' });
      map.set('task3', { status: 'done' });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('enum');
      expect(result.content[0].text).toContain('todo');
      expect(result.content[0].text).toContain('done');
    });
  });

  describe('handleStats', () => {
    it('should return connection stats', async () => {
      const ctx = createTestContext();

      const result = await handleStats({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('connected');
      expect(result.content[0].text).toContain('Pending Operations: 0');
    });

    it('should return map stats when specified', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleStats({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('Records: 2');
    });
  });

  describe('handleExplain', () => {
    it('should explain full scan without filter', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleExplain({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FULL_SCAN');
      expect(result.content[0].text).toContain('Total Records: 2');
    });

    it('should explain filter scan', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 10; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: i % 2 === 0 ? 'done' : 'todo' });
      }

      const result = await handleExplain(
        { map: 'tasks', filter: { status: 'done' } },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FILTER_SCAN');
      expect(result.content[0].text).toContain('Apply filter');
      expect(result.content[0].text).toContain('Estimated Results: 5');
      expect(result.content[0].text).toContain('Selectivity: 50');
    });

    it('should provide recommendations for large datasets', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 1000; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: 'todo' });
      }

      const result = await handleExplain(
        { map: 'tasks', filter: { status: 'todo' } },
        ctx
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Recommendations');
    });
  });
});
