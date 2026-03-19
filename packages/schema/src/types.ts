/**
 * Mirrors Rust FieldType enum serde output.
 * PascalCase variants because FieldType has no rename_all attribute.
 * Array variant serializes as { Array: inner } (Rust adjacently-tagged enum default).
 */
export type JsonFieldType =
  | 'String'
  | 'Int'
  | 'Float'
  | 'Bool'
  | 'Binary'
  | 'Timestamp'
  | { Array: JsonFieldType }
  | 'Map'
  | 'Any';

/**
 * Mirrors Rust FieldConstraint struct.
 * Fields in camelCase because FieldConstraint has #[serde(rename_all = "camelCase")].
 *
 * Note: minValue and maxValue correspond to Rust i64 fields. JavaScript number
 * is a 64-bit float with safe integer range up to Number.MAX_SAFE_INTEGER (2^53 - 1).
 * Values outside this range silently lose precision. Schema constraints are expected
 * to stay well within this range in practice; this is a known limitation.
 */
export interface JsonFieldConstraint {
  minLength?: number;
  maxLength?: number;
  minValue?: number;
  maxValue?: number;
  pattern?: string;
  enumValues?: string[];
}

/**
 * Mirrors Rust FieldDef struct.
 * Fields in snake_case because FieldDef has no rename_all attribute.
 */
export interface JsonFieldDef {
  name: string;
  required: boolean;
  field_type: JsonFieldType;
  constraints?: JsonFieldConstraint;
}

/**
 * Mirrors Rust MapSchema struct.
 * Fields in snake_case because MapSchema has no rename_all attribute.
 * version corresponds to Rust u32.
 */
export interface JsonMapSchema {
  version: number;
  fields: JsonFieldDef[];
  strict: boolean;
}

/**
 * Top-level codegen output file format.
 * Keyed by map name, each value is a JsonMapSchema.
 */
export interface JsonSchemaFile {
  maps: Record<string, JsonMapSchema>;
}
