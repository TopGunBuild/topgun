import { defineMap, t, MapBuilder, SchemaRegistry, schema } from '../dsl';
import type { JsonMapSchema } from '../types';

describe('defineMap', () => {
  test('creates a MapBuilder instance', () => {
    const builder = defineMap('users');
    expect(builder).toBeInstanceOf(MapBuilder);
  });

  test('build() returns correct map name', () => {
    const result = defineMap('posts').build();
    expect(result.name).toBe('posts');
  });

  test('default strict is false', () => {
    const result = defineMap('test').build();
    expect(result.schema.strict).toBe(false);
  });

  test('default version is 1', () => {
    const result = defineMap('test').build();
    expect(result.schema.version).toBe(1);
  });

  test('options: strict true sets strict', () => {
    const result = defineMap('test', { strict: true }).build();
    expect(result.schema.strict).toBe(true);
  });

  test('options: version sets version', () => {
    const result = defineMap('test', { version: 3 }).build();
    expect(result.schema.version).toBe(3);
  });

  test('fields array is empty by default', () => {
    const result = defineMap('test').build();
    expect(result.schema.fields).toHaveLength(0);
  });

  test('build() returns a JsonMapSchema with correct structure', () => {
    const result = defineMap('users', { strict: true, version: 2 })
      .field('name', t.string(), { required: true })
      .build();

    const schema: JsonMapSchema = result.schema;
    expect(schema.version).toBe(2);
    expect(schema.strict).toBe(true);
    expect(schema.fields).toHaveLength(1);
    expect(schema.fields[0]).toMatchObject({
      name: 'name',
      required: true,
      field_type: 'String',
    });
  });
});

describe('t field type factory functions', () => {
  test('t.string() returns "String"', () => {
    expect(t.string()).toBe('String');
  });

  test('t.int() returns "Int"', () => {
    expect(t.int()).toBe('Int');
  });

  test('t.float() returns "Float"', () => {
    expect(t.float()).toBe('Float');
  });

  test('t.bool() returns "Bool"', () => {
    expect(t.bool()).toBe('Bool');
  });

  test('t.binary() returns "Binary"', () => {
    expect(t.binary()).toBe('Binary');
  });

  test('t.timestamp() returns "Timestamp"', () => {
    expect(t.timestamp()).toBe('Timestamp');
  });

  test('t.map() returns "Map"', () => {
    expect(t.map()).toBe('Map');
  });

  test('t.any() returns "Any"', () => {
    expect(t.any()).toBe('Any');
  });

  test('t.array(t.string()) produces { Array: "String" }', () => {
    expect(t.array(t.string())).toEqual({ Array: 'String' });
  });

  test('nested arrays: t.array(t.array(t.int())) produces { Array: { Array: "Int" } }', () => {
    expect(t.array(t.array(t.int()))).toEqual({ Array: { Array: 'Int' } });
  });

  test('triple nested array works correctly', () => {
    expect(t.array(t.array(t.array(t.bool())))).toEqual({
      Array: { Array: { Array: 'Bool' } },
    });
  });
});

describe('MapBuilder.field()', () => {
  test('adds a field with default required=false', () => {
    const result = defineMap('test').field('age', t.int()).build();
    expect(result.schema.fields[0].required).toBe(false);
  });

  test('required: true sets required to true', () => {
    const result = defineMap('test').field('age', t.int(), { required: true }).build();
    expect(result.schema.fields[0].required).toBe(true);
  });

  test('field_type is set correctly', () => {
    const result = defineMap('test').field('score', t.float()).build();
    expect(result.schema.fields[0].field_type).toBe('Float');
  });

  test('constraints not present when no constraint options given', () => {
    const result = defineMap('test').field('name', t.string()).build();
    expect(result.schema.fields[0].constraints).toBeUndefined();
  });

  test('minLength constraint maps correctly', () => {
    const result = defineMap('test').field('name', t.string(), { minLength: 2 }).build();
    expect(result.schema.fields[0].constraints?.minLength).toBe(2);
  });

  test('maxLength constraint maps correctly', () => {
    const result = defineMap('test').field('name', t.string(), { maxLength: 100 }).build();
    expect(result.schema.fields[0].constraints?.maxLength).toBe(100);
  });

  test('minValue constraint maps correctly', () => {
    const result = defineMap('test').field('age', t.int(), { minValue: 0 }).build();
    expect(result.schema.fields[0].constraints?.minValue).toBe(0);
  });

  test('maxValue constraint maps correctly', () => {
    const result = defineMap('test').field('age', t.int(), { maxValue: 150 }).build();
    expect(result.schema.fields[0].constraints?.maxValue).toBe(150);
  });

  test('pattern constraint maps correctly', () => {
    const result = defineMap('test').field('email', t.string(), { pattern: '^[^@]+@[^@]+$' }).build();
    expect(result.schema.fields[0].constraints?.pattern).toBe('^[^@]+@[^@]+$');
  });

  test('enumValues constraint maps correctly', () => {
    const result = defineMap('test').field('status', t.string(), { enumValues: ['active', 'inactive'] }).build();
    expect(result.schema.fields[0].constraints?.enumValues).toEqual(['active', 'inactive']);
  });

  test('all constraints together', () => {
    const result = defineMap('test')
      .field('name', t.string(), {
        required: true,
        minLength: 1,
        maxLength: 100,
        pattern: '^\\w+$',
      })
      .build();

    const field = result.schema.fields[0];
    expect(field.required).toBe(true);
    expect(field.constraints).toEqual({
      minLength: 1,
      maxLength: 100,
      pattern: '^\\w+$',
    });
  });

  test('method chaining works', () => {
    const result = defineMap('test')
      .field('a', t.string())
      .field('b', t.int())
      .field('c', t.bool())
      .build();

    expect(result.schema.fields).toHaveLength(3);
    expect(result.schema.fields[0].name).toBe('a');
    expect(result.schema.fields[1].name).toBe('b');
    expect(result.schema.fields[2].name).toBe('c');
  });
});

describe('SchemaRegistry', () => {
  let registry: SchemaRegistry;

  beforeEach(() => {
    registry = new SchemaRegistry();
  });

  test('getSchemas returns empty object initially', () => {
    expect(registry.getSchemas()).toEqual({});
  });

  test('register accumulates schemas', () => {
    registry.register(defineMap('users').field('name', t.string()));
    registry.register(defineMap('posts').field('title', t.string()));

    const schemas = registry.getSchemas();
    expect(Object.keys(schemas)).toHaveLength(2);
    expect(schemas['users']).toBeDefined();
    expect(schemas['posts']).toBeDefined();
  });

  test('register stores correct schema for map name', () => {
    registry.register(
      defineMap('items', { strict: true, version: 2 })
        .field('id', t.string(), { required: true })
    );

    const schemas = registry.getSchemas();
    expect(schemas['items']).toMatchObject({
      version: 2,
      strict: true,
      fields: [{ name: 'id', required: true, field_type: 'String' }],
    });
  });

  test('getSchemas returns a copy (mutations do not affect registry)', () => {
    registry.register(defineMap('test').field('x', t.int()));
    const schemas = registry.getSchemas();
    delete schemas['test'];

    // Registry should still have 'test'
    expect(registry.getSchemas()['test']).toBeDefined();
  });

  test('SchemaRegistry.global is a SchemaRegistry instance', () => {
    expect(SchemaRegistry.global).toBeInstanceOf(SchemaRegistry);
  });

  test('schema export is SchemaRegistry.global', () => {
    expect(schema).toBe(SchemaRegistry.global);
  });
});
