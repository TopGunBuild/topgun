/**
 * QuantizedNavigableIndex Implementation
 *
 * NavigableIndex with quantization support for grouping nearby values.
 * Useful for timestamps, prices, and other numeric attributes where
 * exact values are less important than ranges.
 *
 * @module query/indexes/QuantizedNavigableIndex
 */

import type { Attribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';
import { LazyResultSet } from '../resultset/LazyResultSet';
import { SortedMap } from '../ds/SortedMap';
import type { Comparator } from '../ds/types';

/**
 * Quantizer interface for mapping values to buckets.
 */
export interface Quantizer<A> {
  /**
   * Map a value to its quantized bucket key.
   * Multiple values may map to the same bucket.
   */
  quantize(value: A): A;
}

/**
 * Collection of common quantizers.
 */
export const Quantizers = {
  /**
   * Round numbers to nearest multiple.
   * Example: multiple=10 maps 23 → 20, 27 → 20, 35 → 30
   */
  integerMultiple(multiple: number): Quantizer<number> {
    return {
      quantize: (value: number) => Math.floor(value / multiple) * multiple,
    };
  },

  /**
   * Truncate timestamps to interval.
   * Example: intervalMs=60000 (1 minute) rounds to minute boundaries.
   */
  timestampInterval(intervalMs: number): Quantizer<number> {
    return {
      quantize: (value: number) =>
        Math.floor(value / intervalMs) * intervalMs,
    };
  },

  /**
   * Round to power of 10.
   * Example: 5 → 1, 50 → 10, 500 → 100, 5000 → 1000
   */
  powerOf10(): Quantizer<number> {
    return {
      quantize: (value: number) => {
        if (value <= 0) return 0;
        const magnitude = Math.floor(Math.log10(value));
        return Math.pow(10, magnitude);
      },
    };
  },

  /**
   * Logarithmic buckets (base 2).
   * Useful for sizes, counts, or any exponentially distributed values.
   */
  logarithmic(base: number = 2): Quantizer<number> {
    return {
      quantize: (value: number) => {
        if (value <= 0) return 0;
        const exp = Math.floor(Math.log(value) / Math.log(base));
        return Math.pow(base, exp);
      },
    };
  },
};

/**
 * Quantized attribute wrapper.
 * Applies quantization when extracting values.
 */
class QuantizedAttribute<V, A extends number> implements Attribute<V, A> {
  readonly type: 'simple' | 'multi';

  constructor(
    private readonly inner: Attribute<V, A>,
    private readonly quantizer: Quantizer<A>
  ) {
    this.type = inner.type;
  }

  get name(): string {
    return this.inner.name;
  }

  getValue(record: V): A | undefined {
    const value = this.inner.getValue(record);
    return value !== undefined ? this.quantizer.quantize(value) : undefined;
  }

  getValues(record: V): A[] {
    return this.inner.getValues(record).map((v) => this.quantizer.quantize(v));
  }
}

/**
 * NavigableIndex with quantization support.
 *
 * Reduces index size by grouping nearby values into buckets.
 * Queries return all records in matching buckets (may include false positives).
 *
 * Use cases:
 * - Timestamps: Group by minute/hour/day
 * - Prices: Group by $10/$100/$1000 ranges
 * - Sizes: Group by powers of 2 or 10
 *
 * K = record key type, V = record value type, A = attribute value type (must be number)
 */
export class QuantizedNavigableIndex<K, V, A extends number>
  implements Index<K, V, A>
{
  readonly type = 'navigable' as const;

  /** Sorted map from quantized value to set of record keys */
  private data: SortedMap<A, Set<K>>;

  /** Set of all keys with non-null attribute value */
  private allKeys: Set<K> = new Set();

  /** The original attribute (unquantized) */
  private originalAttribute: Attribute<V, A>;

  /** Quantized attribute for index operations */
  private quantizedAttribute: QuantizedAttribute<V, A>;

  /** The quantizer */
  private quantizer: Quantizer<A>;

  /** Retrieval cost as per CQEngine cost model */
  private static readonly RETRIEVAL_COST = 40;

  /** Supported query types */
  private static readonly SUPPORTED_QUERIES = [
    'equal',
    'in',
    'has',
    'gt',
    'gte',
    'lt',
    'lte',
    'between',
  ];

  /**
   * Create a QuantizedNavigableIndex.
   *
   * @param attribute - Attribute to index
   * @param quantizer - Quantizer for grouping values
   * @param comparator - Optional custom comparator for ordering
   */
  constructor(
    attribute: Attribute<V, A>,
    quantizer: Quantizer<A>,
    comparator?: Comparator<A>
  ) {
    this.originalAttribute = attribute;
    this.quantizer = quantizer;
    this.quantizedAttribute = new QuantizedAttribute(attribute, quantizer);
    this.data = new SortedMap<A, Set<K>>(comparator);
  }

  get attribute(): Attribute<V, A> {
    return this.originalAttribute;
  }

  getRetrievalCost(): number {
    return QuantizedNavigableIndex.RETRIEVAL_COST;
  }

  supportsQuery(queryType: string): boolean {
    return QuantizedNavigableIndex.SUPPORTED_QUERIES.includes(queryType);
  }

  retrieve(query: IndexQuery<A>): ResultSet<K> {
    switch (query.type) {
      case 'equal':
        return this.retrieveEqual(query.value as A);
      case 'in':
        return this.retrieveIn(query.values as A[]);
      case 'has':
        return this.retrieveHas();
      case 'gt':
        return this.retrieveGreaterThan(query.value as A, false);
      case 'gte':
        return this.retrieveGreaterThan(query.value as A, true);
      case 'lt':
        return this.retrieveLessThan(query.value as A, false);
      case 'lte':
        return this.retrieveLessThan(query.value as A, true);
      case 'between':
        return this.retrieveBetween(
          query.from as A,
          query.to as A,
          query.fromInclusive ?? true,
          query.toInclusive ?? false
        );
      default:
        throw new Error(
          `QuantizedNavigableIndex does not support query type: ${query.type}`
        );
    }
  }

  // ============== Equality Queries ==============

  private retrieveEqual(value: A): ResultSet<K> {
    // Quantize the search value
    const quantizedValue = this.quantizer.quantize(value);
    const keys = this.data.get(quantizedValue);
    return new SetResultSet(
      keys ? new Set(keys) : new Set(),
      QuantizedNavigableIndex.RETRIEVAL_COST
    );
  }

  private retrieveIn(values: A[]): ResultSet<K> {
    const result = new Set<K>();
    const seenBuckets = new Set<A>();

    for (const value of values) {
      const quantizedValue = this.quantizer.quantize(value);
      // Skip if we already checked this bucket
      if (seenBuckets.has(quantizedValue)) continue;
      seenBuckets.add(quantizedValue);

      const keys = this.data.get(quantizedValue);
      if (keys) {
        for (const key of keys) {
          result.add(key);
        }
      }
    }
    return new SetResultSet(result, QuantizedNavigableIndex.RETRIEVAL_COST);
  }

  private retrieveHas(): ResultSet<K> {
    return new SetResultSet(
      new Set(this.allKeys),
      QuantizedNavigableIndex.RETRIEVAL_COST
    );
  }

  // ============== Range Queries ==============

  private retrieveGreaterThan(value: A, inclusive: boolean): ResultSet<K> {
    const quantizedValue = this.quantizer.quantize(value);
    return new LazyResultSet(
      () => this.iterateGreaterThan(quantizedValue, inclusive),
      QuantizedNavigableIndex.RETRIEVAL_COST,
      this.estimateGreaterThanSize()
    );
  }

  private retrieveLessThan(value: A, inclusive: boolean): ResultSet<K> {
    const quantizedValue = this.quantizer.quantize(value);
    return new LazyResultSet(
      () => this.iterateLessThan(quantizedValue, inclusive),
      QuantizedNavigableIndex.RETRIEVAL_COST,
      this.estimateLessThanSize()
    );
  }

  private retrieveBetween(
    from: A,
    to: A,
    fromInclusive: boolean,
    toInclusive: boolean
  ): ResultSet<K> {
    const quantizedFrom = this.quantizer.quantize(from);
    const quantizedTo = this.quantizer.quantize(to);
    return new LazyResultSet(
      () =>
        this.iterateBetween(quantizedFrom, quantizedTo, fromInclusive, toInclusive),
      QuantizedNavigableIndex.RETRIEVAL_COST,
      this.estimateBetweenSize()
    );
  }

  // ============== Lazy Iterators ==============

  private *iterateGreaterThan(value: A, inclusive: boolean): Generator<K> {
    for (const [, keys] of this.data.greaterThan(value, inclusive)) {
      for (const key of keys) {
        yield key;
      }
    }
  }

  private *iterateLessThan(value: A, inclusive: boolean): Generator<K> {
    for (const [, keys] of this.data.lessThan(value, inclusive)) {
      for (const key of keys) {
        yield key;
      }
    }
  }

  private *iterateBetween(
    from: A,
    to: A,
    fromInclusive: boolean,
    toInclusive: boolean
  ): Generator<K> {
    for (const [, keys] of this.data.range(from, to, {
      fromInclusive,
      toInclusive,
    })) {
      for (const key of keys) {
        yield key;
      }
    }
  }

  // ============== Size Estimation ==============

  private estimateGreaterThanSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 2));
  }

  private estimateLessThanSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 2));
  }

  private estimateBetweenSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 4));
  }

  // ============== Index Mutations ==============

  add(key: K, record: V): void {
    const values = this.quantizedAttribute.getValues(record);
    if (values.length === 0) return;

    for (const value of values) {
      let keys = this.data.get(value);
      if (!keys) {
        keys = new Set();
        this.data.set(value, keys);
      }
      keys.add(key);
    }

    this.allKeys.add(key);
  }

  remove(key: K, record: V): void {
    const values = this.quantizedAttribute.getValues(record);

    for (const value of values) {
      const keys = this.data.get(value);
      if (keys) {
        keys.delete(key);
        if (keys.size === 0) {
          this.data.delete(value);
        }
      }
    }

    this.allKeys.delete(key);
  }

  update(key: K, oldRecord: V, newRecord: V): void {
    const oldValues = this.quantizedAttribute.getValues(oldRecord);
    const newValues = this.quantizedAttribute.getValues(newRecord);

    // Check if quantized values changed
    if (this.arraysEqual(oldValues, newValues)) {
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
    for (const [, keys] of this.data.entries()) {
      totalEntries += keys.size;
    }

    return {
      distinctValues: this.data.size,
      totalEntries,
      avgEntriesPerValue:
        this.data.size > 0 ? totalEntries / this.data.size : 0,
    };
  }

  // ============== Helpers ==============

  private arraysEqual(a: A[], b: A[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return false;
    }
    return true;
  }

  /**
   * Get the quantizer used by this index.
   */
  getQuantizer(): Quantizer<A> {
    return this.quantizer;
  }

  /**
   * Get the number of distinct buckets.
   */
  getBucketCount(): number {
    return this.data.size;
  }
}
