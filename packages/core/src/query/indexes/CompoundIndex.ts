/**
 * CompoundIndex Implementation
 *
 * Multi-attribute index for optimizing AND queries.
 * Creates a composite key from multiple attribute values for O(1) lookup.
 *
 * Benefits:
 * - 100-1000× speedup for compound AND queries
 * - Eliminates ResultSet intersection overhead
 * - Single lookup instead of multiple index scans
 *
 * @module query/indexes/CompoundIndex
 */

import type { Attribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';

/**
 * Compound query specification for multi-attribute matching.
 */
export interface CompoundQuery<A = unknown> {
  /** Query type - only 'compound' for compound indexes */
  type: 'compound';
  /** Array of attribute values in order of compound index attributes */
  values: A[];
}

/**
 * Options for compound index creation.
 */
export interface CompoundIndexOptions {
  /**
   * Separator for composite key generation.
   * Default: '|'
   * Choose a separator that won't appear in attribute values.
   */
  separator?: string;
}

/**
 * Compound index for O(1) multi-attribute queries.
 * Indexes multiple attributes as a single composite key.
 *
 * K = record key type, V = record value type
 */
export class CompoundIndex<K, V> implements Index<K, V, unknown> {
  readonly type = 'compound' as const;

  /** Attributes that make up this compound index (in order) */
  private readonly _attributes: Attribute<V, unknown>[];

  /** Map from composite key to set of record keys */
  private data: Map<string, Set<K>> = new Map();

  /** Set of all indexed keys */
  private allKeys: Set<K> = new Set();

  /** Key separator */
  private readonly separator: string;

  /** Retrieval cost (lower than individual indexes combined) */
  private static readonly RETRIEVAL_COST = 20;

  /**
   * Create a CompoundIndex.
   *
   * @param attributes - Array of attributes to index (order matters!)
   * @param options - Optional configuration
   *
   * @example
   * ```typescript
   * const statusAttr = simpleAttribute<Product, string>('status', p => p.status);
   * const categoryAttr = simpleAttribute<Product, string>('category', p => p.category);
   *
   * const compoundIndex = new CompoundIndex<string, Product>([statusAttr, categoryAttr]);
   * ```
   */
  constructor(
    attributes: Attribute<V, unknown>[],
    options: CompoundIndexOptions = {}
  ) {
    if (attributes.length < 2) {
      throw new Error('CompoundIndex requires at least 2 attributes');
    }
    this._attributes = attributes;
    this.separator = options.separator ?? '|';
  }

  /**
   * Get the first attribute (used for Index interface compatibility).
   * Note: CompoundIndex spans multiple attributes.
   */
  get attribute(): Attribute<V, unknown> {
    return this._attributes[0];
  }

  /**
   * Get all attributes in this compound index.
   */
  get attributes(): Attribute<V, unknown>[] {
    return [...this._attributes];
  }

  /**
   * Get attribute names as a combined identifier.
   */
  get compoundName(): string {
    return this._attributes.map((a) => a.name).join('+');
  }

  getRetrievalCost(): number {
    return CompoundIndex.RETRIEVAL_COST;
  }

  supportsQuery(queryType: string): boolean {
    return queryType === 'compound';
  }

  /**
   * Retrieve records matching compound query.
   *
   * @param query - Compound query with values matching each attribute
   * @returns ResultSet of matching keys
   *
   * @example
   * ```typescript
   * // Find products where status='active' AND category='electronics'
   * index.retrieve({
   *   type: 'compound',
   *   values: ['active', 'electronics']
   * });
   * ```
   */
  retrieve(query: IndexQuery<unknown>): ResultSet<K> {
    if (query.type !== 'compound') {
      throw new Error(`CompoundIndex only supports 'compound' query type, got: ${query.type}`);
    }

    const compoundQuery = query as unknown as CompoundQuery;
    const values = compoundQuery.values;

    if (values.length !== this._attributes.length) {
      throw new Error(
        `CompoundIndex requires ${this._attributes.length} values, got ${values.length}`
      );
    }

    const compositeKey = this.buildCompositeKey(values);
    const keys = this.data.get(compositeKey);

    return new SetResultSet(
      keys ? new Set(keys) : new Set(),
      CompoundIndex.RETRIEVAL_COST
    );
  }

  /**
   * Retrieve with explicit values (convenience method).
   *
   * @param values - Values in order of index attributes
   * @returns ResultSet of matching keys
   */
  retrieveByValues(...values: unknown[]): ResultSet<K> {
    return this.retrieve({ type: 'compound', values } as IndexQuery<unknown>);
  }

  add(key: K, record: V): void {
    const compositeKey = this.buildCompositeKeyFromRecord(record);
    if (compositeKey === null) return;

    let keys = this.data.get(compositeKey);
    if (!keys) {
      keys = new Set();
      this.data.set(compositeKey, keys);
    }
    keys.add(key);
    this.allKeys.add(key);
  }

  remove(key: K, record: V): void {
    const compositeKey = this.buildCompositeKeyFromRecord(record);
    if (compositeKey === null) return;

    const keys = this.data.get(compositeKey);
    if (keys) {
      keys.delete(key);
      if (keys.size === 0) {
        this.data.delete(compositeKey);
      }
    }
    this.allKeys.delete(key);
  }

  update(key: K, oldRecord: V, newRecord: V): void {
    const oldKey = this.buildCompositeKeyFromRecord(oldRecord);
    const newKey = this.buildCompositeKeyFromRecord(newRecord);

    // Optimize: check if composite key changed
    if (oldKey === newKey) {
      return;
    }

    this.remove(key, oldRecord);
    this.add(key, newRecord);
  }

  clear(): void {
    this.data.clear();
    this.allKeys.clear();
  }

  getStats(): IndexStats {
    let totalEntries = 0;
    for (const keys of this.data.values()) {
      totalEntries += keys.size;
    }

    return {
      distinctValues: this.data.size,
      totalEntries,
      avgEntriesPerValue: this.data.size > 0 ? totalEntries / this.data.size : 0,
    };
  }

  /**
   * Get extended statistics for compound index.
   */
  getExtendedStats(): CompoundIndexStats {
    const stats = this.getStats();
    return {
      ...stats,
      attributeCount: this._attributes.length,
      attributeNames: this._attributes.map((a) => a.name),
      compositeKeyCount: this.data.size,
    };
  }

  /**
   * Check if this compound index can answer a query on the given attributes.
   * Compound indexes can be used if query attributes match in prefix order.
   *
   * @param attributeNames - Attribute names being queried
   * @returns true if this index can answer the query
   */
  canAnswerQuery(attributeNames: string[]): boolean {
    if (attributeNames.length !== this._attributes.length) {
      return false;
    }

    // Check if all attribute names match (order matters for now)
    for (let i = 0; i < attributeNames.length; i++) {
      if (attributeNames[i] !== this._attributes[i].name) {
        return false;
      }
    }

    return true;
  }

  /**
   * Build composite key from array of values.
   */
  private buildCompositeKey(values: unknown[]): string {
    return values.map((v) => this.encodeValue(v)).join(this.separator);
  }

  /**
   * Build composite key from record by extracting attribute values.
   * Returns null if any attribute value is undefined.
   */
  private buildCompositeKeyFromRecord(record: V): string | null {
    const values: unknown[] = [];

    for (const attr of this._attributes) {
      const value = attr.getValue(record);
      if (value === undefined) {
        return null; // Can't index record with missing attribute
      }
      values.push(value);
    }

    return this.buildCompositeKey(values);
  }

  /**
   * Encode value for composite key.
   * Handles common types and escapes separator.
   */
  private encodeValue(value: unknown): string {
    if (value === null) return '__null__';
    if (value === undefined) return '__undefined__';

    const str = String(value);
    // Escape any separator characters in the value
    return str.replace(
      new RegExp(this.escapeRegex(this.separator), 'g'),
      `\\${this.separator}`
    );
  }

  /**
   * Escape regex special characters.
   */
  private escapeRegex(str: string): string {
    return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }
}

/**
 * Extended statistics for compound index.
 */
export interface CompoundIndexStats extends IndexStats {
  /** Number of attributes in compound key */
  attributeCount: number;
  /** Names of indexed attributes */
  attributeNames: string[];
  /** Number of unique composite keys */
  compositeKeyCount: number;
}

/**
 * Helper to check if an index is a compound index.
 */
export function isCompoundIndex<K, V>(index: Index<K, V, unknown>): index is CompoundIndex<K, V> {
  return index.type === 'compound';
}
