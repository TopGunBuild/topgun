import {
  BuiltInResolvers,
  validateResolverCode,
  deepMerge,
  compareHLCTimestamps,
  MergeContext,
  MergeResult,
} from '../ConflictResolver';
import { HLC, Timestamp } from '../HLC';

describe('ConflictResolver', () => {
  describe('validateResolverCode', () => {
    it('should accept valid code', () => {
      const result = validateResolverCode(`
        if (context.localValue !== undefined) {
          return { action: 'reject', reason: 'Already exists' };
        }
        return { action: 'accept', value: context.remoteValue };
      `);
      expect(result.valid).toBe(true);
      expect(result.error).toBeUndefined();
    });

    it('should reject code with eval', () => {
      const result = validateResolverCode(`
        eval('alert(1)');
        return { action: 'accept', value: context.remoteValue };
      `);
      expect(result.valid).toBe(false);
      expect(result.error).toContain('eval');
    });

    it('should reject code with Function constructor', () => {
      const result = validateResolverCode(`
        new Function('return 1')();
        return { action: 'accept', value: context.remoteValue };
      `);
      expect(result.valid).toBe(false);
      expect(result.error).toContain('Function');
    });

    it('should reject code with process access', () => {
      const result = validateResolverCode(`
        process.exit(1);
        return { action: 'accept', value: context.remoteValue };
      `);
      expect(result.valid).toBe(false);
      expect(result.error).toContain('process');
    });

    it('should reject code with fetch', () => {
      const result = validateResolverCode(`
        fetch('http://evil.com');
        return { action: 'accept', value: context.remoteValue };
      `);
      expect(result.valid).toBe(false);
      expect(result.error).toContain('fetch');
    });
  });

  describe('compareHLCTimestamps', () => {
    it('should compare by millis first', () => {
      const a: Timestamp = { millis: 1000, counter: 0, nodeId: 'a' };
      const b: Timestamp = { millis: 2000, counter: 0, nodeId: 'a' };
      expect(compareHLCTimestamps(a, b)).toBeLessThan(0);
      expect(compareHLCTimestamps(b, a)).toBeGreaterThan(0);
    });

    it('should compare by counter when millis equal', () => {
      const a: Timestamp = { millis: 1000, counter: 1, nodeId: 'a' };
      const b: Timestamp = { millis: 1000, counter: 2, nodeId: 'a' };
      expect(compareHLCTimestamps(a, b)).toBeLessThan(0);
      expect(compareHLCTimestamps(b, a)).toBeGreaterThan(0);
    });

    it('should compare by nodeId when millis and counter equal', () => {
      const a: Timestamp = { millis: 1000, counter: 1, nodeId: 'a' };
      const b: Timestamp = { millis: 1000, counter: 1, nodeId: 'b' };
      expect(compareHLCTimestamps(a, b)).toBeLessThan(0);
      expect(compareHLCTimestamps(b, a)).toBeGreaterThan(0);
    });

    it('should return 0 for equal timestamps', () => {
      const a: Timestamp = { millis: 1000, counter: 1, nodeId: 'a' };
      const b: Timestamp = { millis: 1000, counter: 1, nodeId: 'a' };
      expect(compareHLCTimestamps(a, b)).toBe(0);
    });
  });

  describe('deepMerge', () => {
    it('should merge flat objects', () => {
      const target = { a: 1, b: 2, c: 0 };
      const source = { a: 1, b: 3, c: 4 };
      const result = deepMerge(target, source);
      expect(result).toEqual({ a: 1, b: 3, c: 4 });
    });

    it('should merge nested objects', () => {
      type NestedObj = { a: { x?: number; y: number; z?: number }; b?: number; c?: number };
      const target: NestedObj = { a: { x: 1, y: 2 }, b: 3 };
      const source: NestedObj = { a: { y: 3, z: 4 }, c: 5 };
      const result = deepMerge(target, source);
      expect(result).toEqual({ a: { x: 1, y: 3, z: 4 }, b: 3, c: 5 });
    });

    it('should overwrite arrays instead of merging', () => {
      const target = { arr: [1, 2, 3] };
      const source = { arr: [4, 5] };
      const result = deepMerge(target, source);
      expect(result).toEqual({ arr: [4, 5] });
    });

    it('should handle null values', () => {
      const target = { a: { b: 1 } as { b: number } | null };
      const source = { a: null as { b: number } | null };
      const result = deepMerge(target, source);
      expect(result.a).toBeNull();
    });
  });

  describe('BuiltInResolvers.LWW', () => {
    it('should accept remote when no local value', () => {
      const resolver = BuiltInResolvers.LWW<number>();
      const context: MergeContext<number> = {
        mapName: 'test',
        key: 'key1',
        localValue: undefined,
        remoteValue: 42,
        localTimestamp: undefined,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = resolver.fn!(context);
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should accept remote when remote timestamp is newer', () => {
      const resolver = BuiltInResolvers.LWW<number>();
      const context: MergeContext<number> = {
        mapName: 'test',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = resolver.fn!(context);
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should keep local when local timestamp is newer', () => {
      const resolver = BuiltInResolvers.LWW<number>();
      const context: MergeContext<number> = {
        mapName: 'test',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        localTimestamp: { millis: 2000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = resolver.fn!(context);
      expect(result).toEqual({ action: 'local' });
    });
  });

  describe('BuiltInResolvers.FIRST_WRITE_WINS', () => {
    it('should accept when no local value', () => {
      const resolver = BuiltInResolvers.FIRST_WRITE_WINS<number>();
      const context: MergeContext<number> = {
        mapName: 'test',
        key: 'key1',
        localValue: undefined,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = resolver.fn!(context);
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should reject when local value exists', () => {
      const resolver = BuiltInResolvers.FIRST_WRITE_WINS<number>();
      const context: MergeContext<number> = {
        mapName: 'test',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      };

      const result = resolver.fn!(context) as MergeResult<number>;
      expect(result.action).toBe('reject');
      expect((result as any).reason).toContain('already exists');
    });
  });

  describe('BuiltInResolvers.NUMERIC_MIN', () => {
    it('should keep minimum value', () => {
      const resolver = BuiltInResolvers.NUMERIC_MIN();

      // Remote is smaller
      let result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 100,
        remoteValue: 50,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'merge', value: 50 });

      // Local is smaller
      result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 25,
        remoteValue: 50,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'merge', value: 25 });
    });
  });

  describe('BuiltInResolvers.NUMERIC_MAX', () => {
    it('should keep maximum value', () => {
      const resolver = BuiltInResolvers.NUMERIC_MAX();

      // Remote is larger
      let result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 50,
        remoteValue: 100,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'merge', value: 100 });

      // Local is larger
      result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 150,
        remoteValue: 100,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'merge', value: 150 });
    });
  });

  describe('BuiltInResolvers.NON_NEGATIVE', () => {
    it('should accept non-negative values', () => {
      const resolver = BuiltInResolvers.NON_NEGATIVE();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 0,
        remoteValue: 100,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'accept', value: 100 });
    });

    it('should reject negative values', () => {
      const resolver = BuiltInResolvers.NON_NEGATIVE();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 100,
        remoteValue: -50,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      }) as MergeResult<number>;
      expect(result.action).toBe('reject');
      expect((result as any).reason).toContain('negative');
    });
  });

  describe('BuiltInResolvers.ARRAY_UNION', () => {
    it('should merge arrays taking union', () => {
      const resolver = BuiltInResolvers.ARRAY_UNION<number>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: [1, 2, 3],
        remoteValue: [3, 4, 5],
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      }) as MergeResult<number[]>;
      expect(result.action).toBe('merge');
      expect((result as any).value.sort()).toEqual([1, 2, 3, 4, 5]);
    });
  });

  describe('BuiltInResolvers.DEEP_MERGE', () => {
    it('should deep merge objects', () => {
      type ObjType = { a: { x?: number; y: number; z?: number }; b?: number; c?: number };
      const resolver = BuiltInResolvers.DEEP_MERGE<ObjType>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: { a: { x: 1, y: 2 }, b: 3 },
        remoteValue: { a: { y: 3, z: 4 }, c: 5 },
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      }) as MergeResult<ObjType>;
      expect(result.action).toBe('merge');
      expect((result as any).value).toEqual({ a: { x: 1, y: 3, z: 4 }, b: 3, c: 5 });
    });
  });

  describe('BuiltInResolvers.IMMUTABLE', () => {
    it('should accept first write', () => {
      const resolver = BuiltInResolvers.IMMUTABLE<number>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: undefined,
        remoteValue: 42,
        remoteTimestamp: { millis: 1000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'accept', value: 42 });
    });

    it('should reject modifications', () => {
      const resolver = BuiltInResolvers.IMMUTABLE<number>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: 10,
        remoteValue: 42,
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      }) as MergeResult<number>;
      expect(result.action).toBe('reject');
      expect((result as any).reason).toContain('immutable');
    });
  });

  describe('BuiltInResolvers.VERSION_INCREMENT', () => {
    it('should accept correct version increment', () => {
      const resolver = BuiltInResolvers.VERSION_INCREMENT<{ version: number; data: string }>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: { version: 1, data: 'old' },
        remoteValue: { version: 2, data: 'new' },
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      });
      expect(result).toEqual({ action: 'accept', value: { version: 2, data: 'new' } });
    });

    it('should reject incorrect version', () => {
      const resolver = BuiltInResolvers.VERSION_INCREMENT<{ version: number; data: string }>();

      const result = resolver.fn!({
        mapName: 'test',
        key: 'key1',
        localValue: { version: 1, data: 'old' },
        remoteValue: { version: 5, data: 'new' },
        localTimestamp: { millis: 1000, counter: 0, nodeId: 'local' },
        remoteTimestamp: { millis: 2000, counter: 0, nodeId: 'remote' },
        remoteNodeId: 'remote',
        readEntry: () => undefined,
      }) as MergeResult<{ version: number; data: string }>;
      expect(result.action).toBe('reject');
      expect((result as any).reason).toContain('Version conflict');
    });
  });
});
