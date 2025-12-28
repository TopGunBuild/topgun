import { ProcessorSandbox, DEFAULT_SANDBOX_CONFIG } from '../ProcessorSandbox';
import { BuiltInProcessors } from '@topgunbuild/core';

describe('ProcessorSandbox', () => {
  let sandbox: ProcessorSandbox;

  beforeEach(() => {
    sandbox = new ProcessorSandbox();
  });

  afterEach(() => {
    sandbox.dispose();
  });

  describe('basic execution', () => {
    it('should execute a simple processor', async () => {
      const processor = {
        name: 'simple',
        code: `
          return { value: 42, result: 'done' };
        `,
      };

      const result = await sandbox.execute(processor, undefined, 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(42);
      expect(result.result).toBe('done');
    });

    it('should have access to current value', async () => {
      const processor = {
        name: 'access_value',
        code: `
          return { value: value * 2, result: value };
        `,
      };

      const result = await sandbox.execute(processor, 10, 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(20);
      expect(result.result).toBe(10);
    });

    it('should have access to key', async () => {
      const processor = {
        name: 'access_key',
        code: `
          return { value: key.toUpperCase(), result: key };
        `,
      };

      const result = await sandbox.execute(processor, null, 'mykey');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe('MYKEY');
      expect(result.result).toBe('mykey');
    });

    it('should have access to args', async () => {
      const processor = {
        name: 'access_args',
        code: `
          return { value: args.multiplier * value, result: args.name };
        `,
        args: { multiplier: 3, name: 'test' },
      };

      const result = await sandbox.execute(processor, 5, 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(15);
      expect(result.result).toBe('test');
    });

    it('should handle undefined value (new key)', async () => {
      const processor = {
        name: 'handle_undefined',
        code: `
          const current = value ?? 0;
          return { value: current + 1, result: 'initialized' };
        `,
      };

      const result = await sandbox.execute(processor, undefined, 'newkey');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(1);
      expect(result.result).toBe('initialized');
    });
  });

  describe('BuiltInProcessors execution', () => {
    it('should execute INCREMENT', async () => {
      const processor = BuiltInProcessors.INCREMENT(5);
      const result = await sandbox.execute(processor, 10, 'counter');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(15);
      expect(result.result).toBe(15);
    });

    it('should execute INCREMENT with undefined value', async () => {
      const processor = BuiltInProcessors.INCREMENT(3);
      const result = await sandbox.execute(processor, undefined, 'counter');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(3);
    });

    it('should execute DECREMENT', async () => {
      const processor = BuiltInProcessors.DECREMENT(3);
      const result = await sandbox.execute(processor, 10, 'counter');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(7);
    });

    it('should execute DECREMENT_FLOOR without flooring', async () => {
      const processor = BuiltInProcessors.DECREMENT_FLOOR(5);
      const result = await sandbox.execute(processor, 10, 'counter');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(5);
      expect(result.result).toEqual({ newValue: 5, wasFloored: false });
    });

    it('should execute DECREMENT_FLOOR with flooring', async () => {
      const processor = BuiltInProcessors.DECREMENT_FLOOR(10);
      const result = await sandbox.execute(processor, 5, 'counter');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(0);
      expect(result.result).toEqual({ newValue: 0, wasFloored: true });
    });

    it('should execute PUT_IF_ABSENT when key exists', async () => {
      const processor = BuiltInProcessors.PUT_IF_ABSENT('new');
      const result = await sandbox.execute(processor, 'existing', 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe('existing');
      expect(result.result).toBe(false);
    });

    it('should execute PUT_IF_ABSENT when key is absent', async () => {
      const processor = BuiltInProcessors.PUT_IF_ABSENT('new');
      const result = await sandbox.execute(processor, undefined, 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe('new');
      expect(result.result).toBe(true);
    });

    it('should execute DELETE_IF_EQUALS when value matches', async () => {
      const processor = BuiltInProcessors.DELETE_IF_EQUALS('target');
      const result = await sandbox.execute(processor, 'target', 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBeUndefined();
      expect(result.result).toBe(true);
    });

    it('should execute DELETE_IF_EQUALS when value does not match', async () => {
      const processor = BuiltInProcessors.DELETE_IF_EQUALS('target');
      const result = await sandbox.execute(processor, 'other', 'key1');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe('other');
      expect(result.result).toBe(false);
    });

    it('should execute ARRAY_PUSH', async () => {
      const processor = BuiltInProcessors.ARRAY_PUSH('item3');
      const result = await sandbox.execute(processor, ['item1', 'item2'], 'list');

      expect(result.success).toBe(true);
      expect(result.newValue).toEqual(['item1', 'item2', 'item3']);
      expect(result.result).toBe(3);
    });

    it('should execute ARRAY_PUSH on undefined', async () => {
      const processor = BuiltInProcessors.ARRAY_PUSH('first');
      const result = await sandbox.execute(processor, undefined, 'list');

      expect(result.success).toBe(true);
      expect(result.newValue).toEqual(['first']);
      expect(result.result).toBe(1);
    });

    it('should execute MULTIPLY', async () => {
      const processor = BuiltInProcessors.MULTIPLY(3);
      const result = await sandbox.execute(processor, 5, 'number');

      expect(result.success).toBe(true);
      expect(result.newValue).toBe(15);
    });
  });

  describe('error handling', () => {
    it('should handle runtime errors', async () => {
      const processor = {
        name: 'runtime_error',
        code: `
          throw new Error('Intentional error');
        `,
      };

      const result = await sandbox.execute(processor, null, 'key1');

      expect(result.success).toBe(false);
      expect(result.error).toContain('Intentional error');
    });

    it('should reject invalid return format (primitive)', async () => {
      const processor = {
        name: 'bad_return',
        code: `
          return 42;
        `,
      };

      const result = await sandbox.execute(processor, null, 'key1');

      expect(result.success).toBe(false);
      expect(result.error).toContain('must return');
    });

    it('should reject code with forbidden patterns when validation is enabled', async () => {
      const sandboxWithValidation = new ProcessorSandbox({ strictValidation: true });

      const processor = {
        name: 'forbidden',
        code: `
          eval("1+1");
          return { value: 1 };
        `,
      };

      const result = await sandboxWithValidation.execute(processor, null, 'key1');

      expect(result.success).toBe(false);
      expect(result.error).toContain('Forbidden');

      sandboxWithValidation.dispose();
    });
  });

  describe('timeout handling', () => {
    it('should timeout on infinite loop', async () => {
      const shortTimeoutSandbox = new ProcessorSandbox({ timeoutMs: 50 });

      const processor = {
        name: 'infinite',
        code: `
          while(true) {}
          return { value: 1 };
        `,
      };

      const result = await shortTimeoutSandbox.execute(processor, null, 'key1');

      expect(result.success).toBe(false);
      expect(result.error).toContain('timed out');

      shortTimeoutSandbox.dispose();
    });
  });

  describe('cache management', () => {
    it('should cache isolates', async () => {
      const processor = {
        name: 'cached',
        code: `return { value: 1 };`,
      };

      await sandbox.execute(processor, null, 'key1');
      const stats1 = sandbox.getCacheStats();

      await sandbox.execute(processor, null, 'key2');
      const stats2 = sandbox.getCacheStats();

      // Should use same cached isolate
      expect(stats1.isolates).toBe(stats2.isolates);
    });

    it('should clear cache for specific processor', async () => {
      const processor = {
        name: 'to_clear',
        code: `return { value: 1 };`,
      };

      await sandbox.execute(processor, null, 'key1');
      const statsBefore = sandbox.getCacheStats();
      expect(statsBefore.fallbackScripts).toBeGreaterThanOrEqual(1);

      sandbox.clearCache('to_clear');
      const statsAfter = sandbox.getCacheStats();

      expect(statsAfter.fallbackScripts).toBeLessThan(statsBefore.fallbackScripts);
    });

    it('should clear all cache', async () => {
      await sandbox.execute({ name: 'proc1', code: `return { value: 1 };` }, null, 'key1');
      await sandbox.execute({ name: 'proc2', code: `return { value: 2 };` }, null, 'key2');

      const statsBefore = sandbox.getCacheStats();
      expect(statsBefore.fallbackScripts).toBeGreaterThanOrEqual(2);

      sandbox.clearCache();
      const statsAfter = sandbox.getCacheStats();

      expect(statsAfter.fallbackScripts).toBe(0);
    });
  });

  describe('disposal', () => {
    it('should reject execution after disposal', async () => {
      sandbox.dispose();

      const processor = {
        name: 'after_dispose',
        code: `return { value: 1 };`,
      };

      const result = await sandbox.execute(processor, null, 'key1');

      expect(result.success).toBe(false);
      expect(result.error).toContain('disposed');
    });
  });

  describe('security mode', () => {
    it('should report security mode', () => {
      // This will be false in most test environments without isolated-vm
      const isSecure = sandbox.isSecureMode();
      expect(typeof isSecure).toBe('boolean');
    });
  });

  describe('DEFAULT_SANDBOX_CONFIG', () => {
    it('should have reasonable defaults', () => {
      expect(DEFAULT_SANDBOX_CONFIG.memoryLimitMb).toBe(8);
      expect(DEFAULT_SANDBOX_CONFIG.timeoutMs).toBe(100);
      expect(DEFAULT_SANDBOX_CONFIG.maxCachedIsolates).toBe(100);
      expect(DEFAULT_SANDBOX_CONFIG.strictValidation).toBe(true);
    });
  });
});
