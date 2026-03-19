import type { JsonFieldType, JsonFieldConstraint, JsonFieldDef, JsonMapSchema } from './types';

/**
 * Options for a field definition.
 * All constraint fields are optional; absent fields are omitted from output.
 */
export interface FieldOptions {
  required?: boolean;
  minLength?: number;
  maxLength?: number;
  minValue?: number;
  maxValue?: number;
  pattern?: string;
  enumValues?: string[];
}

/**
 * Fluent builder for a single map schema.
 * Use defineMap() to create an instance.
 */
export class MapBuilder {
  private readonly _name: string;
  private readonly _strict: boolean;
  private readonly _version: number;
  private readonly _fields: JsonFieldDef[] = [];

  constructor(name: string, strict: boolean, version: number) {
    this._name = name;
    this._strict = strict;
    this._version = version;
  }

  /**
   * Add a field definition to this map.
   * Returns `this` for chaining.
   */
  field(name: string, type: JsonFieldType, options: FieldOptions = {}): this {
    const { required = false, ...constraintOptions } = options;

    const constraints: JsonFieldConstraint = {};
    if (constraintOptions.minLength !== undefined) constraints.minLength = constraintOptions.minLength;
    if (constraintOptions.maxLength !== undefined) constraints.maxLength = constraintOptions.maxLength;
    if (constraintOptions.minValue !== undefined) constraints.minValue = constraintOptions.minValue;
    if (constraintOptions.maxValue !== undefined) constraints.maxValue = constraintOptions.maxValue;
    if (constraintOptions.pattern !== undefined) constraints.pattern = constraintOptions.pattern;
    if (constraintOptions.enumValues !== undefined) constraints.enumValues = constraintOptions.enumValues;

    const fieldDef: JsonFieldDef = {
      name,
      required,
      field_type: type,
    };

    if (Object.keys(constraints).length > 0) {
      fieldDef.constraints = constraints;
    }

    this._fields.push(fieldDef);
    return this;
  }

  /**
   * Finalize the builder and return the map name + JSON schema.
   */
  build(): { name: string; schema: JsonMapSchema } {
    return {
      name: this._name,
      schema: {
        version: this._version,
        fields: [...this._fields],
        strict: this._strict,
      },
    };
  }
}

/**
 * Create a new MapBuilder for the given map name.
 */
export function defineMap(
  name: string,
  options: { strict?: boolean; version?: number } = {}
): MapBuilder {
  return new MapBuilder(name, options.strict ?? false, options.version ?? 1);
}

/**
 * Factory functions for each FieldType variant.
 * Each function returns the corresponding JsonFieldType literal or object.
 */
export const t = {
  /** Returns the "String" FieldType */
  string: (): JsonFieldType => 'String',
  /** Returns the "Int" FieldType */
  int: (): JsonFieldType => 'Int',
  /** Returns the "Float" FieldType */
  float: (): JsonFieldType => 'Float',
  /** Returns the "Bool" FieldType */
  bool: (): JsonFieldType => 'Bool',
  /** Returns the "Binary" FieldType */
  binary: (): JsonFieldType => 'Binary',
  /** Returns the "Timestamp" FieldType */
  timestamp: (): JsonFieldType => 'Timestamp',
  /** Returns an Array FieldType wrapping the given inner type */
  array: (inner: JsonFieldType): JsonFieldType => ({ Array: inner }),
  /** Returns the "Map" FieldType */
  map: (): JsonFieldType => 'Map',
  /** Returns the "Any" FieldType */
  any: (): JsonFieldType => 'Any',
};

/**
 * Registry that accumulates map schema definitions.
 * Use schema.register() in topgun.schema.ts files to add schemas.
 * The codegen reads SchemaRegistry.global.getSchemas() at build time.
 */
export class SchemaRegistry {
  private readonly _schemas: Record<string, JsonMapSchema> = {};

  /** The global singleton registry, used by topgun.schema.ts files. */
  static readonly global = new SchemaRegistry();

  /**
   * Register a map schema built with defineMap().
   */
  register(builder: MapBuilder): void {
    const { name, schema } = builder.build();
    this._schemas[name] = schema;
  }

  /**
   * Return all registered schemas, keyed by map name.
   */
  getSchemas(): Record<string, JsonMapSchema> {
    return { ...this._schemas };
  }
}

/**
 * Convenience alias for SchemaRegistry.global.
 * Import as `schema` and call schema.register(...) in topgun.schema.ts files.
 */
export const schema = SchemaRegistry.global;
