import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { SchemaRegistry, defineMap, t } from '../dsl';
import { runCodegen } from '../codegen';
import type { JsonSchemaFile } from '../types';

/**
 * Helper: reset the global registry before each test and reload it with given schemas.
 */
function resetRegistry(): void {
  // Access the private _schemas map via casting to reset it
  const registry = SchemaRegistry.global as unknown as { _schemas: Record<string, unknown> };
  for (const key of Object.keys(registry._schemas)) {
    delete registry._schemas[key];
  }
}

/**
 * Helper: create a temp directory for output files and clean it up after.
 */
function withTempDir(fn: (dir: string) => void): void {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'schema-codegen-test-'));
  try {
    fn(dir);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/**
 * Run codegen against a pre-populated registry (bypasses file loading).
 */
function runCodegenWithRegistry(
  schemas: Record<string, unknown>,
  outDir: string,
  opts: { typescript?: boolean; json?: boolean } = {}
): void {
  // Populate global registry
  resetRegistry();
  const registry = SchemaRegistry.global as unknown as { _schemas: Record<string, unknown> };
  Object.assign(registry._schemas, schemas);

  // Write a minimal JS schema file that does nothing (registry already populated)
  const schemaFile = path.join(outDir, 'dummy.schema.js');
  fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

  runCodegen({
    schemaPath: schemaFile,
    outDir,
    typescript: opts.typescript ?? true,
    json: opts.json ?? true,
  });
}

describe('JSON output', () => {
  test('produces topgun.schema.json with correct JsonSchemaFile structure', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('users', { strict: true, version: 1 })
          .field('name', t.string(), { required: true, maxLength: 100 })
          .field('age', t.int(), { minValue: 0, maxValue: 150 })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const jsonPath = path.join(dir, 'topgun.schema.json');
      expect(fs.existsSync(jsonPath)).toBe(true);

      const parsed: JsonSchemaFile = JSON.parse(fs.readFileSync(jsonPath, 'utf-8'));
      expect(parsed).toHaveProperty('maps');
      expect(parsed.maps).toHaveProperty('users');
    });
  });

  test('JSON field names use snake_case for FieldDef (field_type not fieldType)', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('items').field('label', t.string(), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      const field = parsed.maps['items'].fields[0];
      // snake_case: field_type, not fieldType
      expect(field).toHaveProperty('field_type');
      expect((field as unknown as Record<string, unknown>)['fieldType']).toBeUndefined();
    });
  });

  test('FieldConstraint fields use camelCase (minLength not min_length)', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('items').field('name', t.string(), { minLength: 3, maxLength: 50 })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      const constraints = parsed.maps['items'].fields[0].constraints;
      expect(constraints).toBeDefined();
      expect(constraints).toHaveProperty('minLength', 3);
      expect(constraints).toHaveProperty('maxLength', 50);
      // snake_case variants should NOT be present
      expect((constraints as Record<string, unknown>)['min_length']).toBeUndefined();
    });
  });

  test('MapSchema fields use snake_case (version, fields, strict)', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('test', { strict: true, version: 2 }));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      const mapSchema = parsed.maps['test'];
      expect(mapSchema.version).toBe(2);
      expect(mapSchema.strict).toBe(true);
      expect(Array.isArray(mapSchema.fields)).toBe(true);
    });
  });

  test('Array FieldType serializes as { Array: inner }', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('test').field('tags', t.array(t.string()))
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      const field = parsed.maps['test'].fields[0];
      expect(field.field_type).toEqual({ Array: 'String' });
    });
  });

  test('empty schema produces empty maps object', () => {
    withTempDir((dir) => {
      resetRegistry();

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      expect(parsed.maps).toEqual({});
    });
  });
});

describe('TypeScript output', () => {
  test('produces topgun.types.ts', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('users').field('name', t.string(), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const tsPath = path.join(dir, 'topgun.types.ts');
      expect(fs.existsSync(tsPath)).toBe(true);
    });
  });

  test('required fields produce non-optional TS properties', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('users').field('name', t.string(), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      // Required field: no '?' after the field name
      expect(content).toContain('  name: string;');
    });
  });

  test('optional fields (required: false) produce "?" properties', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('users').field('age', t.int())
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  age?: number;');
    });
  });

  test('Binary maps to Uint8Array', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('blobs').field('data', t.binary(), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  data: Uint8Array;');
    });
  });

  test('Timestamp maps to number', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('events').field('createdAt', t.timestamp(), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  createdAt: number;');
    });
  });

  test('Array(String) maps to string[]', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('posts').field('tags', t.array(t.string()))
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  tags?: string[];');
    });
  });

  test('Nested array Array(Array(Int)) maps to number[][]', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('matrix').field('data', t.array(t.array(t.int())), { required: true })
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  data: number[][];');
    });
  });

  test('Map type maps to Record<string, unknown>', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('items').field('metadata', t.map())
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  metadata?: Record<string, unknown>;');
    });
  });

  test('Any type maps to unknown', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('items').field('value', t.any())
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('  value?: unknown;');
    });
  });

  test('interface naming: users -> UserRecord', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('users').field('x', t.string()));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('export interface UserRecord {');
    });
  });

  test('interface naming: posts -> PostRecord', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('posts').field('x', t.string()));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('export interface PostRecord {');
    });
  });

  test('interface naming: metadata -> MetadataRecord (no trailing s to strip)', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('metadata').field('x', t.string()));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('export interface MetadataRecord {');
    });
  });

  test('interface naming: people -> PeopleRecord (irregular plural, no s)', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('people').field('x', t.string()));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('export interface PeopleRecord {');
    });
  });

  test('MapSchemas interface is generated with all maps', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(defineMap('users').field('name', t.string()));
      SchemaRegistry.global.register(defineMap('posts').field('title', t.string()));

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).toContain('export interface MapSchemas {');
      expect(content).toContain('  users: UserRecord;');
      expect(content).toContain('  posts: PostRecord;');
    });
  });

  test('empty schema produces empty output (no interfaces)', () => {
    withTempDir((dir) => {
      resetRegistry();

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: true, json: false });

      const content = fs.readFileSync(path.join(dir, 'topgun.types.ts'), 'utf-8');
      expect(content).not.toContain('export interface');
    });
  });
});

describe('codegen round-trip fidelity', () => {
  test('generated JSON matches exact serde field names for full schema', () => {
    withTempDir((dir) => {
      resetRegistry();
      SchemaRegistry.global.register(
        defineMap('users', { strict: true, version: 1 })
          .field('name', t.string(), { required: true, maxLength: 100 })
          .field('email', t.string(), { required: true, pattern: '^[^@]+@[^@]+$' })
          .field('age', t.int(), { minValue: 0, maxValue: 150 })
          .field('tags', t.array(t.string()), { maxLength: 10 })
          .field('metadata', t.map())
      );

      const schemaFile = path.join(dir, 'dummy.schema.js');
      fs.writeFileSync(schemaFile, '// pre-populated\n', 'utf-8');

      runCodegen({ schemaPath: schemaFile, outDir: dir, typescript: false, json: true });

      const parsed: JsonSchemaFile = JSON.parse(
        fs.readFileSync(path.join(dir, 'topgun.schema.json'), 'utf-8')
      );

      const usersSchema = parsed.maps['users'];

      // MapSchema fields: version (u32), fields (array), strict (bool)
      expect(typeof usersSchema.version).toBe('number');
      expect(Array.isArray(usersSchema.fields)).toBe(true);
      expect(typeof usersSchema.strict).toBe('boolean');

      // FieldDef uses snake_case: name, required, field_type
      const nameField = usersSchema.fields.find(f => f.name === 'name');
      expect(nameField).toBeDefined();
      expect(nameField!.required).toBe(true);
      expect(nameField!.field_type).toBe('String');
      expect(nameField!.constraints?.maxLength).toBe(100);

      // age field with minValue/maxValue in camelCase
      const ageField = usersSchema.fields.find(f => f.name === 'age');
      expect(ageField!.constraints?.minValue).toBe(0);
      expect(ageField!.constraints?.maxValue).toBe(150);

      // tags field: Array type
      const tagsField = usersSchema.fields.find(f => f.name === 'tags');
      expect(tagsField!.field_type).toEqual({ Array: 'String' });
      expect(tagsField!.constraints?.maxLength).toBe(10);
    });
  });
});
