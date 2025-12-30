/**
 * NavigableIndex Implementation
 *
 * Sorted index for O(log N) range queries.
 * Supports: equal, in, has, gt, gte, lt, lte, between queries.
 *
 * Structure: SortedMap<AttributeValue, Set<RecordKey>>
 *
 * CQEngine Reference: NavigableIndex.java (retrieval cost: 40)
 *
 * @module query/indexes/NavigableIndex
 */

import type { Attribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';
import { LazyResultSet } from '../resultset/LazyResultSet';
import { SortedMap } from '../ds/SortedMap';
import type { Comparator } from '../ds/types';

/**
 * Sorted index for O(log N) range queries.
 *
 * K = record key type, V = record value type, A = attribute value type (must be orderable)
 */
export class NavigableIndex<K, V, A extends string | number>
  implements Index<K, V, A>
{
  readonly type = 'navigable' as const;

  /** Sorted map from attribute value to set of record keys */
  private data: SortedMap<A, Set<K>>;

  /** Set of all keys with non-null attribute value */
  private allKeys: Set<K> = new Set();

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
   * Create a NavigableIndex.
   *
   * @param attribute - Attribute to index
   * @param comparator - Optional custom comparator for ordering
   */
  constructor(
    readonly attribute: Attribute<V, A>,
    comparator?: Comparator<A>
  ) {
    this.data = new SortedMap<A, Set<K>>(comparator);
  }

  getRetrievalCost(): number {
    return NavigableIndex.RETRIEVAL_COST;
  }

  supportsQuery(queryType: string): boolean {
    return NavigableIndex.SUPPORTED_QUERIES.includes(queryType);
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
          `NavigableIndex does not support query type: ${query.type}`
        );
    }
  }

  // ============== Equality Queries ==============

  private retrieveEqual(value: A): ResultSet<K> {
    const keys = this.data.get(value);
    return new SetResultSet(
      keys ? new Set(keys) : new Set(),
      NavigableIndex.RETRIEVAL_COST
    );
  }

  private retrieveIn(values: A[]): ResultSet<K> {
    const result = new Set<K>();
    for (const value of values) {
      const keys = this.data.get(value);
      if (keys) {
        for (const key of keys) {
          result.add(key);
        }
      }
    }
    return new SetResultSet(result, NavigableIndex.RETRIEVAL_COST);
  }

  private retrieveHas(): ResultSet<K> {
    return new SetResultSet(
      new Set(this.allKeys),
      NavigableIndex.RETRIEVAL_COST
    );
  }

  // ============== Range Queries ==============

  private retrieveGreaterThan(value: A, inclusive: boolean): ResultSet<K> {
    return new LazyResultSet(
      () => this.iterateGreaterThan(value, inclusive),
      NavigableIndex.RETRIEVAL_COST,
      this.estimateGreaterThanSize()
    );
  }

  private retrieveLessThan(value: A, inclusive: boolean): ResultSet<K> {
    return new LazyResultSet(
      () => this.iterateLessThan(value, inclusive),
      NavigableIndex.RETRIEVAL_COST,
      this.estimateLessThanSize()
    );
  }

  private retrieveBetween(
    from: A,
    to: A,
    fromInclusive: boolean,
    toInclusive: boolean
  ): ResultSet<K> {
    return new LazyResultSet(
      () => this.iterateBetween(from, to, fromInclusive, toInclusive),
      NavigableIndex.RETRIEVAL_COST,
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

  /**
   * Estimate size for gt/gte queries.
   * Uses rough estimate: assume uniform distribution, return half.
   */
  private estimateGreaterThanSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 2));
  }

  /**
   * Estimate size for lt/lte queries.
   * Uses rough estimate: assume uniform distribution, return half.
   */
  private estimateLessThanSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 2));
  }

  /**
   * Estimate size for between queries.
   * Uses rough estimate: assume uniform distribution, return quarter.
   */
  private estimateBetweenSize(): number {
    return Math.max(1, Math.floor(this.allKeys.size / 4));
  }

  // ============== Index Mutations ==============

  add(key: K, record: V): void {
    const values = this.attribute.getValues(record);
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
    const values = this.attribute.getValues(record);

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
    const oldValues = this.attribute.getValues(oldRecord);
    const newValues = this.attribute.getValues(newRecord);

    // Optimize: check if values actually changed
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
   * Get the minimum indexed value.
   * Useful for debugging and range estimation.
   */
  getMinValue(): A | undefined {
    return this.data.minKey();
  }

  /**
   * Get the maximum indexed value.
   * Useful for debugging and range estimation.
   */
  getMaxValue(): A | undefined {
    return this.data.maxKey();
  }
}
