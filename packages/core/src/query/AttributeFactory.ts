/**
 * Attribute Factory (Phase 9.02)
 *
 * Type-safe attribute generation with:
 * - Auto-generated attributes from schema definition
 * - Nested path support (e.g., 'address.city')
 * - Multi-value detection
 * - Full TypeScript type inference
 *
 * @module query/AttributeFactory
 */

import { SimpleAttribute, MultiValueAttribute, type Attribute } from './Attribute';

/**
 * Supported attribute type definitions.
 */
export type AttributeType =
  | 'string'
  | 'number'
  | 'boolean'
  | 'string[]'
  | 'number[]';

/**
 * Schema definition for attribute generation.
 * Key is the path (supports dots for nested), value is the type.
 *
 * @example
 * ```typescript
 * const schema = {
 *   id: 'string',
 *   price: 'number',
 *   tags: 'string[]',
 *   'address.city': 'string'
 * } satisfies AttributeSchema;
 * ```
 */
export type AttributeSchema = Record<string, AttributeType>;

/**
 * Infer the TypeScript type from AttributeType string.
 */
type InferType<T extends AttributeType> =
  T extends 'string' ? string :
  T extends 'number' ? number :
  T extends 'boolean' ? boolean :
  T extends 'string[]' ? string :
  T extends 'number[]' ? number :
  never;

/**
 * Infer if the type is multi-value.
 */
type IsMultiValue<T extends AttributeType> =
  T extends 'string[]' ? true :
  T extends 'number[]' ? true :
  false;

/**
 * Generated attribute type based on schema entry.
 */
type GeneratedAttribute<V, T extends AttributeType> =
  IsMultiValue<T> extends true
    ? MultiValueAttribute<V, InferType<T>>
    : SimpleAttribute<V, InferType<T>>;

/**
 * Type-safe generated attributes object.
 * Supports nested paths via dot notation.
 */
export type GeneratedAttributes<V, S extends AttributeSchema> = {
  [K in keyof S]: GeneratedAttribute<V, S[K]>;
};

/**
 * Options for attribute generation.
 */
export interface GenerateAttributesOptions {
  /**
   * Prefix for all attribute names (default: empty).
   * Useful for namespace isolation.
   */
  namePrefix?: string;
}

/**
 * Get value from object by nested path.
 * Supports paths like 'address.city' → obj.address?.city
 *
 * @param obj - Object to extract value from
 * @param path - Dot-separated path string
 * @returns Value at path or undefined
 */
function getNestedValue(obj: unknown, path: string): unknown {
  if (obj === null || obj === undefined) return undefined;

  const keys = path.split('.');
  let current: unknown = obj;

  for (const key of keys) {
    if (current === null || current === undefined) return undefined;
    if (typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[key];
  }

  return current;
}

/**
 * Check if a type string represents a multi-value (array) type.
 */
function isMultiValueType(type: AttributeType): boolean {
  return type.endsWith('[]');
}

/**
 * Generate type-safe attributes from a schema definition.
 *
 * @param schema - Schema defining attribute paths and types
 * @param options - Optional configuration
 * @returns Type-safe attributes object with autocomplete support
 *
 * @example
 * ```typescript
 * interface Product {
 *   id: string;
 *   name: string;
 *   price: number;
 *   tags: string[];
 *   supplier: { name: string; country: string };
 * }
 *
 * const productAttrs = generateAttributes<Product>()({
 *   id: 'string',
 *   name: 'string',
 *   price: 'number',
 *   tags: 'string[]',
 *   'supplier.name': 'string',
 *   'supplier.country': 'string',
 * });
 *
 * // Type-safe usage:
 * map.addHashIndex(productAttrs.id);           // ✅ Autocomplete
 * map.addNavigableIndex(productAttrs.price);   // ✅ Type-checked
 * map.addInvertedIndex(productAttrs.tags);     // ✅ Multi-value
 * map.addHashIndex(productAttrs['supplier.name']); // ✅ Nested
 * ```
 */
export function generateAttributes<V>(): <S extends AttributeSchema>(
  schema: S,
  options?: GenerateAttributesOptions
) => GeneratedAttributes<V, S> {
  return <S extends AttributeSchema>(
    schema: S,
    options: GenerateAttributesOptions = {}
  ): GeneratedAttributes<V, S> => {
  const { namePrefix = '' } = options;
  const result: Record<string, Attribute<V, unknown>> = {};

  for (const [path, type] of Object.entries(schema)) {
    const attrName = namePrefix ? `${namePrefix}.${path}` : path;

    if (isMultiValueType(type)) {
      // Multi-value attribute (arrays)
      result[path] = new MultiValueAttribute<V, unknown>(
        attrName,
        (record: V) => {
          const value = getNestedValue(record, path);
          if (Array.isArray(value)) return value;
          return [];
        }
      );
    } else {
      // Simple attribute
      result[path] = new SimpleAttribute<V, unknown>(
        attrName,
        (record: V) => getNestedValue(record, path) as unknown
      );
    }
  }

    return result as GeneratedAttributes<V, S>;
  };
}

/**
 * Create a simple attribute helper (shorthand for simpleAttribute).
 * Uses path for both name and value extraction.
 *
 * @param path - Dot-separated path (e.g., 'user.email')
 * @returns SimpleAttribute that extracts value at path
 *
 * @example
 * ```typescript
 * const emailAttr = attr<User, string>('email');
 * const cityAttr = attr<User, string>('address.city');
 * ```
 */
export function attr<V, A>(path: string): SimpleAttribute<V, A> {
  return new SimpleAttribute<V, A>(
    path,
    (record: V) => getNestedValue(record, path) as A | undefined
  );
}

/**
 * Create a multi-value attribute helper.
 * Uses path for both name and value extraction.
 *
 * @param path - Dot-separated path to array field (e.g., 'user.tags')
 * @returns MultiValueAttribute that extracts array at path
 *
 * @example
 * ```typescript
 * const tagsAttr = multiAttr<User, string>('tags');
 * const rolesAttr = multiAttr<User, string>('permissions.roles');
 * ```
 */
export function multiAttr<V, A>(path: string): MultiValueAttribute<V, A> {
  return new MultiValueAttribute<V, A>(
    path,
    (record: V) => {
      const value = getNestedValue(record, path);
      if (Array.isArray(value)) return value as A[];
      return [];
    }
  );
}

/**
 * Schema builder for creating attribute schemas with validation.
 * Provides a fluent API for defining schemas.
 *
 * @example
 * ```typescript
 * const schema = createSchema<Product>()
 *   .string('id')
 *   .string('name')
 *   .number('price')
 *   .stringArray('tags')
 *   .string('supplier.name')
 *   .build();
 *
 * const attrs = generateAttributes<Product>(schema);
 * ```
 */
export function createSchema<V>(): SchemaBuilder<V, {}> {
  return new SchemaBuilder<V, {}>({});
}

/**
 * Fluent schema builder class.
 */
class SchemaBuilder<V, S extends AttributeSchema> {
  constructor(private readonly schema: S) {}

  /**
   * Add a string attribute.
   */
  string<P extends string>(path: P): SchemaBuilder<V, S & Record<P, 'string'>> {
    return new SchemaBuilder({ ...this.schema, [path]: 'string' } as S & Record<P, 'string'>);
  }

  /**
   * Add a number attribute.
   */
  number<P extends string>(path: P): SchemaBuilder<V, S & Record<P, 'number'>> {
    return new SchemaBuilder({ ...this.schema, [path]: 'number' } as S & Record<P, 'number'>);
  }

  /**
   * Add a boolean attribute.
   */
  boolean<P extends string>(path: P): SchemaBuilder<V, S & Record<P, 'boolean'>> {
    return new SchemaBuilder({ ...this.schema, [path]: 'boolean' } as S & Record<P, 'boolean'>);
  }

  /**
   * Add a string array (multi-value) attribute.
   */
  stringArray<P extends string>(path: P): SchemaBuilder<V, S & Record<P, 'string[]'>> {
    return new SchemaBuilder({ ...this.schema, [path]: 'string[]' } as S & Record<P, 'string[]'>);
  }

  /**
   * Add a number array (multi-value) attribute.
   */
  numberArray<P extends string>(path: P): SchemaBuilder<V, S & Record<P, 'number[]'>> {
    return new SchemaBuilder({ ...this.schema, [path]: 'number[]' } as S & Record<P, 'number[]'>);
  }

  /**
   * Build and return the schema.
   */
  build(): S {
    return this.schema;
  }

  /**
   * Build and generate attributes directly.
   */
  generate(options?: GenerateAttributesOptions): GeneratedAttributes<V, S> {
    return generateAttributes<V>()(this.schema, options);
  }
}
