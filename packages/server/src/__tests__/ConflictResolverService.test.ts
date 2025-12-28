import {
  ConflictResolverService,
  DEFAULT_CONFLICT_RESOLVER_CONFIG,
} from '../ConflictResolverService';
import { ProcessorSandbox } from '../ProcessorSandbox';
import {
  BuiltInResolvers,
  MergeContext,
  MergeRejection,
} from '@topgunbuild/core';

describe('ConflictResolverService', () => {
  let sandbox: ProcessorSandbox;
  let service: ConflictResolverService;

  beforeEach(() => {
    sandbox = new ProcessorSandbox(); // Falls back to VM when isolated-vm is not available
    service = new ConflictResolverService(sandbox);
  });

  afterEach(() => {
    service.dispose();
    sandbox.dispose();
  });

  describe('register', () => {
    it('should register a native resolver', () => {
      const resolver = BuiltInResolvers.FIRST_WRITE_WINS<number>();
      service.register('test-map', resolver);

      expect(service.hasResolvers('test-map')).toBe(true);
      expect(service.size).toBe(1);
    });

    it('should register a code-based resolver', () => {
      service.register('test-map', {
        name: 'custom',
        code: `
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'Already exists' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 80,
      });

      expect(service.hasResolvers('test-map')).toBe(true);
    });

    it('should reject invalid code', () => {
      expect(() => {
        service.register('test-map', {
          name: 'evil',
          code: `eval('alert(1)')`,
        });
      }).toThrow(/Forbidden pattern/);
    });

    it('should throw when max resolvers exceeded', () => {
      // Create service with low limit
      const limitedService = new ConflictResolverService(sandbox, {
        maxResolversPerMap: 2,
      });

      limitedService.register('map', { name: 'r1', fn: () => ({ action: 'local' }) });
      limitedService.register('map', { name: 'r2', fn: () => ({ action: 'local' }) });

      expect(() => {
        limitedService.register('map', { name: 'r3', fn: () => ({ action: 'local' }) });
      }).toThrow(/Maximum resolvers/);

      limitedService.dispose();
    });

    it('should replace existing resolver with same name', () => {
      service.register('test-map', {
        name: 'test',
        fn: () => ({ action: 'accept', value: 1 }),
        priority: 50,
      });

      service.register('test-map', {
        name: 'test',
        fn: () => ({ action: 'reject', reason: 'updated' }),
        priority: 60,
      });

      expect(service.size).toBe(1);
      const list = service.list('test-map');
      expect(list[0].priority).toBe(60);
    });

    it('should sort resolvers by priority', () => {
      service.register('test-map', { name: 'low', fn: () => ({ action: 'local' }), priority: 10 });
      service.register('test-map', { name: 'high', fn: () => ({ action: 'local' }), priority: 90 });
      service.register('test-map', { name: 'mid', fn: () => ({ action: 'local' }), priority: 50 });

      const list = service.list('test-map');
      expect(list.map(r => r.name)).toEqual(['high', 'mid', 'low']);
    });
  });

  describe('unregister', () => {
    it('should unregister a resolver', () => {
      service.register('test-map', {
        name: 'test',
        fn: () => ({ action: 'local' }),
      });

      expect(service.unregister('test-map', 'test')).toBe(true);
      expect(service.hasResolvers('test-map')).toBe(false);
    });

    it('should return false for non-existent resolver', () => {
      expect(service.unregister('test-map', 'nonexistent')).toBe(false);
    });

    it('should only unregister if client matches', () => {
      service.register('test-map', {
        name: 'test',
        fn: () => ({ action: 'local' }),
      }, 'client-1');

      // Different client cannot unregister
      expect(service.unregister('test-map', 'test', 'client-2')).toBe(false);

      // Same client can unregister
      expect(service.unregister('test-map', 'test', 'client-1')).toBe(true);
    });
  });

  describe('resolve', () => {
    it('should execute native resolver', async () => {
      service.register('test-map', BuiltInResolvers.FIRST_WRITE_WINS());

      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: undefined,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = await service.resolve(context);
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should use LWW as fallback', async () => {
      // No custom resolvers registered
      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = await service.resolve(context);
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should execute resolvers in priority order', async () => {
      const executionOrder: string[] = [];

      service.register('test-map', {
        name: 'low',
        fn: () => {
          executionOrder.push('low');
          return { action: 'local' };
        },
        priority: 10,
      });

      service.register('test-map', {
        name: 'high',
        fn: () => {
          executionOrder.push('high');
          return { action: 'local' };
        },
        priority: 90,
      });

      service.register('test-map', {
        name: 'mid',
        fn: () => {
          executionOrder.push('mid');
          return { action: 'local' };
        },
        priority: 50,
      });

      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      await service.resolve(context);

      // All returned 'local', so LWW fallback was used
      expect(executionOrder).toEqual(['high', 'mid', 'low']);
    });

    it('should stop at first non-local action', async () => {
      const executionOrder: string[] = [];

      service.register('test-map', {
        name: 'first',
        fn: () => {
          executionOrder.push('first');
          return { action: 'local' };
        },
        priority: 100,
      });

      service.register('test-map', {
        name: 'second',
        fn: () => {
          executionOrder.push('second');
          return { action: 'reject', reason: 'blocked' };
        },
        priority: 90,
      });

      service.register('test-map', {
        name: 'third',
        fn: () => {
          executionOrder.push('third');
          return { action: 'accept', value: 999 };
        },
        priority: 80,
      });

      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = await service.resolve(context);

      expect(executionOrder).toEqual(['first', 'second']);
      expect(result.action).toBe('reject');
    });

    it('should emit rejection events', async () => {
      const rejections: MergeRejection[] = [];
      service.onRejection((r) => rejections.push(r));

      service.register('test-map', BuiltInResolvers.FIRST_WRITE_WINS());

      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: 10, // Already exists
        remoteValue: 42,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = await service.resolve(context);

      expect(result.action).toBe('reject');
      expect(rejections.length).toBe(1);
      expect(rejections[0].reason).toContain('already exists');
    });

    it('should continue on resolver error', async () => {
      service.register('test-map', {
        name: 'broken',
        fn: () => {
          throw new Error('Resolver crashed');
        },
        priority: 100,
      });

      service.register('test-map', {
        name: 'working',
        fn: () => ({ action: 'accept', value: 999 }),
        priority: 50,
      });

      const context: MergeContext<number> = {
        mapName: 'test-map',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = await service.resolve(context);

      // Should skip broken resolver and use working one
      expect(result).toEqual({ action: 'accept', value: 999 });
    });
  });

  describe('list', () => {
    it('should list resolvers for specific map', () => {
      service.register('map1', { name: 'r1', fn: () => ({ action: 'local' }) });
      service.register('map1', { name: 'r2', fn: () => ({ action: 'local' }) });
      service.register('map2', { name: 'r3', fn: () => ({ action: 'local' }) });

      const map1Resolvers = service.list('map1');
      expect(map1Resolvers.length).toBe(2);
      expect(map1Resolvers.map(r => r.name).sort()).toEqual(['r1', 'r2']);
    });

    it('should list all resolvers when no map specified', () => {
      service.register('map1', { name: 'r1', fn: () => ({ action: 'local' }) });
      service.register('map2', { name: 'r2', fn: () => ({ action: 'local' }) });

      const all = service.list();
      expect(all.length).toBe(2);
    });
  });

  describe('clearByClient', () => {
    it('should clear resolvers by client ID', () => {
      service.register('map1', { name: 'r1', fn: () => ({ action: 'local' }) }, 'client-1');
      service.register('map1', { name: 'r2', fn: () => ({ action: 'local' }) }, 'client-2');
      service.register('map2', { name: 'r3', fn: () => ({ action: 'local' }) }, 'client-1');

      const removed = service.clearByClient('client-1');

      expect(removed).toBe(2);
      expect(service.size).toBe(1);
      expect(service.list()[0].name).toBe('r2');
    });
  });

  describe('key pattern matching', () => {
    it('should match * wildcard', async () => {
      let matched = false;

      service.register('test-map', {
        name: 'user-pattern',
        fn: () => {
          matched = true;
          return { action: 'accept', value: 'matched' };
        },
        keyPattern: 'user:*',
        priority: 100,
      });

      // Should match
      await service.resolve({
        mapName: 'test-map',
        key: 'user:123',
        localValue: undefined,
        remoteValue: 'test',
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(matched).toBe(true);

      // Should not match
      matched = false;
      await service.resolve({
        mapName: 'test-map',
        key: 'post:123',
        localValue: undefined,
        remoteValue: 'test',
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(matched).toBe(false);
    });

    it('should match ? wildcard', async () => {
      let matched = false;

      service.register('test-map', {
        name: 'single-char',
        fn: () => {
          matched = true;
          return { action: 'accept', value: 'matched' };
        },
        keyPattern: 'item-?',
        priority: 100,
      });

      // Should match
      await service.resolve({
        mapName: 'test-map',
        key: 'item-1',
        localValue: undefined,
        remoteValue: 'test',
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(matched).toBe(true);

      // Should not match (too many chars)
      matched = false;
      await service.resolve({
        mapName: 'test-map',
        key: 'item-123',
        localValue: undefined,
        remoteValue: 'test',
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(matched).toBe(false);
    });
  });

  describe('deletion handling', () => {
    it('should pass deletions through resolvers with null remoteValue', async () => {
      let receivedContext: MergeContext<any> | null = null;

      service.register('test-map', {
        name: 'deletion-tracker',
        fn: (ctx) => {
          receivedContext = ctx;
          return { action: 'accept', value: ctx.remoteValue };
        },
        priority: 100,
      });

      // Simulate deletion (remoteValue is null)
      await service.resolve({
        mapName: 'test-map',
        key: 'key1',
        localValue: { data: 'existing' },
        remoteValue: null,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });

      expect(receivedContext).not.toBeNull();
      expect(receivedContext!.remoteValue).toBeNull();
      expect(receivedContext!.localValue).toEqual({ data: 'existing' });
    });

    it('should reject deletion with immutable resolver', async () => {
      service.register('test-map', {
        name: 'immutable',
        fn: (ctx) => {
          // Reject any operation if local value exists
          if (ctx.localValue !== undefined) {
            return { action: 'reject', reason: 'Entry is immutable' };
          }
          return { action: 'accept', value: ctx.remoteValue };
        },
        priority: 100,
      });

      // Try to delete existing entry
      const result = await service.resolve({
        mapName: 'test-map',
        key: 'key1',
        localValue: { data: 'protected' },
        remoteValue: null, // Deletion
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });

      expect(result.action).toBe('reject');
      expect((result as any).reason).toContain('immutable');
    });

    it('should allow deletion when no protective resolver exists', async () => {
      // No custom resolvers - LWW fallback should accept deletion
      const result = await service.resolve({
        mapName: 'unprotected-map',
        key: 'key1',
        localValue: { data: 'can be deleted' },
        remoteValue: null, // Deletion
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });

      expect(result.action).toBe('accept');
    });

    it('should emit rejection event for blocked deletion', async () => {
      const rejections: MergeRejection[] = [];
      service.onRejection((r) => rejections.push(r));

      service.register('test-map', {
        name: 'no-delete',
        fn: (ctx) => {
          if (ctx.remoteValue === null && ctx.localValue !== undefined) {
            return { action: 'reject', reason: 'Deletion not allowed' };
          }
          return { action: 'accept', value: ctx.remoteValue };
        },
        priority: 100,
      });

      await service.resolve({
        mapName: 'test-map',
        key: 'protected-key',
        localValue: 'existing',
        remoteValue: null,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote-node',
        readEntry: () => undefined,
      });

      expect(rejections.length).toBe(1);
      expect(rejections[0].key).toBe('protected-key');
      expect(rejections[0].attemptedValue).toBeNull();
      expect(rejections[0].reason).toBe('Deletion not allowed');
    });
  });
});
