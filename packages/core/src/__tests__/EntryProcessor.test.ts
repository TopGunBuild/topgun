import {
  EntryProcessorDefSchema,
  validateProcessorCode,
  BuiltInProcessors,
  FORBIDDEN_PATTERNS,
} from '../EntryProcessor';

describe('EntryProcessor', () => {
  describe('EntryProcessorDefSchema', () => {
    it('should validate a valid processor definition', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: 'test',
        code: 'return { value: 1 };',
      });

      expect(result.success).toBe(true);
    });

    it('should validate processor with args', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: 'test',
        code: 'return { value: args };',
        args: { delta: 5 },
      });

      expect(result.success).toBe(true);
    });

    it('should reject empty name', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: '',
        code: 'return { value: 1 };',
      });

      expect(result.success).toBe(false);
    });

    it('should reject empty code', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: 'test',
        code: '',
      });

      expect(result.success).toBe(false);
    });

    it('should reject name over 100 characters', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: 'a'.repeat(101),
        code: 'return { value: 1 };',
      });

      expect(result.success).toBe(false);
    });

    it('should reject code over 10000 characters', () => {
      const result = EntryProcessorDefSchema.safeParse({
        name: 'test',
        code: 'a'.repeat(10001),
      });

      expect(result.success).toBe(false);
    });
  });

  describe('validateProcessorCode', () => {
    it('should accept valid code', () => {
      const result = validateProcessorCode(`
        const current = value ?? 0;
        return { value: current + 1 };
      `);

      expect(result.valid).toBe(true);
      expect(result.error).toBeUndefined();
    });

    it('should reject code with eval', () => {
      const result = validateProcessorCode(`
        eval("dangerous");
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('eval');
    });

    it('should reject code with Function constructor', () => {
      const result = validateProcessorCode(`
        new Function("return this")();
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('Function');
    });

    it('should reject code with require', () => {
      const result = validateProcessorCode(`
        const fs = require('fs');
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('require');
    });

    it('should reject code with import', () => {
      const result = validateProcessorCode(`
        import { something } from 'somewhere';
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('import');
    });

    it('should reject code with process', () => {
      const result = validateProcessorCode(`
        process.exit(1);
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('process');
    });

    it('should reject code with fetch', () => {
      const result = validateProcessorCode(`
        fetch('http://evil.com');
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('fetch');
    });

    it('should reject code with setTimeout', () => {
      const result = validateProcessorCode(`
        setTimeout(() => {}, 1000);
        return { value: 1 };
      `);

      expect(result.valid).toBe(false);
      expect(result.error).toContain('setTimeout');
    });
  });

  describe('BuiltInProcessors', () => {
    describe('INCREMENT', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.INCREMENT(5);

        expect(processor.name).toBe('builtin:increment');
        expect(processor.args).toBe(5);
        expect(processor.code).toContain('current + args');
      });

      it('should default to delta of 1', () => {
        const processor = BuiltInProcessors.INCREMENT();

        expect(processor.args).toBe(1);
      });
    });

    describe('DECREMENT', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.DECREMENT(3);

        expect(processor.name).toBe('builtin:decrement');
        expect(processor.args).toBe(3);
        expect(processor.code).toContain('current - args');
      });
    });

    describe('DECREMENT_FLOOR', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.DECREMENT_FLOOR(10);

        expect(processor.name).toBe('builtin:decrement_floor');
        expect(processor.args).toBe(10);
        expect(processor.code).toContain('Math.max(0');
        expect(processor.code).toContain('wasFloored');
      });
    });

    describe('PUT_IF_ABSENT', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.PUT_IF_ABSENT('newValue');

        expect(processor.name).toBe('builtin:put_if_absent');
        expect(processor.args).toBe('newValue');
      });

      it('should handle complex objects', () => {
        const processor = BuiltInProcessors.PUT_IF_ABSENT({ name: 'test', count: 5 });

        expect(processor.args).toEqual({ name: 'test', count: 5 });
      });
    });

    describe('DELETE_IF_EQUALS', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.DELETE_IF_EQUALS('targetValue');

        expect(processor.name).toBe('builtin:delete_if_equals');
        expect(processor.args).toBe('targetValue');
        expect(processor.code).toContain('undefined');
      });
    });

    describe('ARRAY_PUSH', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.ARRAY_PUSH('item');

        expect(processor.name).toBe('builtin:array_push');
        expect(processor.args).toBe('item');
        expect(processor.code).toContain('.push(');
      });
    });

    describe('ARRAY_POP', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.ARRAY_POP();

        expect(processor.name).toBe('builtin:array_pop');
        expect(processor.code).toContain('.pop()');
      });
    });

    describe('ARRAY_REMOVE', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.ARRAY_REMOVE('item');

        expect(processor.name).toBe('builtin:array_remove');
        expect(processor.args).toBe('item');
        expect(processor.code).toContain('findIndex');
        expect(processor.code).toContain('splice');
      });
    });

    describe('SET_PROPERTY', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.SET_PROPERTY('user.name', 'John');

        expect(processor.name).toBe('builtin:set_property');
        expect(processor.args).toEqual({ path: 'user.name', value: 'John' });
        expect(processor.code).toContain('split');
      });
    });

    describe('DELETE_PROPERTY', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.DELETE_PROPERTY('user.name');

        expect(processor.name).toBe('builtin:delete_property');
        expect(processor.args).toBe('user.name');
        expect(processor.code).toContain('delete');
      });
    });

    describe('GET', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.GET();

        expect(processor.name).toBe('builtin:get');
        expect(processor.code).toContain('return { value, result: value }');
      });
    });

    describe('REPLACE', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.REPLACE('newValue');

        expect(processor.name).toBe('builtin:replace');
        expect(processor.args).toBe('newValue');
      });
    });

    describe('REPLACE_IF_EQUALS', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.REPLACE_IF_EQUALS('oldValue', 'newValue');

        expect(processor.name).toBe('builtin:replace_if_equals');
        expect(processor.args).toEqual({ expected: 'oldValue', newValue: 'newValue' });
      });
    });

    describe('CONDITIONAL_UPDATE', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.CONDITIONAL_UPDATE<{ version?: number; name?: string }>(5, { name: 'updated' });

        expect(processor.name).toBe('builtin:conditional_update');
        expect(processor.args).toEqual({
          expectedVersion: 5,
          newData: { name: 'updated' },
        });
      });
    });

    describe('MERGE', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.MERGE({ a: 1, b: 2 });

        expect(processor.name).toBe('builtin:merge');
        expect(processor.args).toEqual({ a: 1, b: 2 });
      });
    });

    describe('MULTIPLY', () => {
      it('should create valid processor definition', () => {
        const processor = BuiltInProcessors.MULTIPLY(2);

        expect(processor.name).toBe('builtin:multiply');
        expect(processor.args).toBe(2);
        expect(processor.code).toContain('current * args');
      });
    });
  });

  describe('FORBIDDEN_PATTERNS', () => {
    it('should include eval', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('eval'))).toBe(true);
    });

    it('should include Function', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('Function'))).toBe(true);
    });

    it('should include require', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('require'))).toBe(true);
    });

    it('should include import', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('import'))).toBe(true);
    });

    it('should include process', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('process'))).toBe(true);
    });

    it('should include global', () => {
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('global'))).toBe(true);
    });

    it('should not match normal variable names', () => {
      // These should NOT match (not word boundaries)
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('evaluate'))).toBe(false);
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('processing'))).toBe(false);
      expect(FORBIDDEN_PATTERNS.some((p) => p.test('globalValue'))).toBe(false);
    });
  });
});
