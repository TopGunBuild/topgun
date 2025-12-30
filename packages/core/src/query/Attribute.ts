/**
 * Attribute System for Query Engine
 *
 * Attributes extract value(s) from records for indexing and querying.
 * Inspired by CQEngine Attribute<O, A>.
 *
 * @module query/Attribute
 */

/**
 * Attribute extracts value(s) from a record.
 * V = Record value type, A = Attribute value type
 */
export interface Attribute<V, A> {
  /** Unique attribute name */
  readonly name: string;

  /** Attribute value type */
  readonly type: 'simple' | 'multi';

  /**
   * Extract value from record.
   * Returns undefined if attribute doesn't exist.
   */
  getValue(record: V): A | undefined;

  /**
   * For multi-value attributes, returns all values.
   * For simple attributes, returns single-element array.
   */
  getValues(record: V): A[];
}

/**
 * Attribute that returns exactly one value per record.
 */
export class SimpleAttribute<V, A> implements Attribute<V, A> {
  readonly type = 'simple' as const;

  constructor(
    readonly name: string,
    private readonly extractor: (record: V) => A | undefined
  ) {}

  getValue(record: V): A | undefined {
    return this.extractor(record);
  }

  getValues(record: V): A[] {
    const value = this.getValue(record);
    return value !== undefined ? [value] : [];
  }
}

/**
 * Factory function for SimpleAttribute.
 */
export function simpleAttribute<V, A>(
  name: string,
  extractor: (record: V) => A | undefined
): SimpleAttribute<V, A> {
  return new SimpleAttribute(name, extractor);
}

/**
 * Attribute that returns zero or more values per record.
 * Example: tags, categories, roles.
 */
export class MultiValueAttribute<V, A> implements Attribute<V, A> {
  readonly type = 'multi' as const;

  constructor(
    readonly name: string,
    private readonly extractor: (record: V) => A[]
  ) {}

  getValue(record: V): A | undefined {
    const values = this.extractor(record);
    return values.length > 0 ? values[0] : undefined;
  }

  getValues(record: V): A[] {
    return this.extractor(record);
  }
}

/**
 * Factory function for MultiValueAttribute.
 */
export function multiAttribute<V, A>(
  name: string,
  extractor: (record: V) => A[]
): MultiValueAttribute<V, A> {
  return new MultiValueAttribute(name, extractor);
}
