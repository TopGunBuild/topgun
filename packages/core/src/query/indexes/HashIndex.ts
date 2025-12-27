/**
 * HashIndex Implementation
 *
 * Hash-based index for O(1) equality lookups.
 * Supports: equal, in, has queries.
 *
 * Structure: Map<AttributeValue, Set<RecordKey>>
 *
 * CQEngine Reference: HashIndex.java (retrieval cost: 30)
 *
 * @module query/indexes/HashIndex
 */

import type { Attribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';

/**
 * Hash-based index for O(1) equality lookups.
 *
 * K = record key type, V = record value type, A = attribute value type
 */
export class HashIndex<K, V, A> implements Index<K, V, A> {
  readonly type = 'hash' as const;

  /** Map from attribute value to set of record keys */
  private data: Map<A, Set<K>> = new Map();

  /** Set of all keys with non-null attribute value */
  private allKeys: Set<K> = new Set();

  private static readonly RETRIEVAL_COST = 30;
  private static readonly SUPPORTED_QUERIES = ['equal', 'in', 'has'];

  constructor(readonly attribute: Attribute<V, A>) {}

  getRetrievalCost(): number {
    return HashIndex.RETRIEVAL_COST;
  }

  supportsQuery(queryType: string): boolean {
    return HashIndex.SUPPORTED_QUERIES.includes(queryType);
  }

  retrieve(query: IndexQuery<A>): ResultSet<K> {
    switch (query.type) {
      case 'equal':
        return this.retrieveEqual(query.value as A);
      case 'in':
        return this.retrieveIn(query.values as A[]);
      case 'has':
        return this.retrieveHas();
      default:
        throw new Error(`HashIndex does not support query type: ${query.type}`);
    }
  }

  private retrieveEqual(value: A): ResultSet<K> {
    const keys = this.data.get(value);
    return new SetResultSet(keys ? new Set(keys) : new Set(), HashIndex.RETRIEVAL_COST);
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
    return new SetResultSet(result, HashIndex.RETRIEVAL_COST);
  }

  private retrieveHas(): ResultSet<K> {
    return new SetResultSet(new Set(this.allKeys), HashIndex.RETRIEVAL_COST);
  }

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
    for (const keys of this.data.values()) {
      totalEntries += keys.size;
    }

    return {
      distinctValues: this.data.size,
      totalEntries,
      avgEntriesPerValue: this.data.size > 0 ? totalEntries / this.data.size : 0,
    };
  }

  private arraysEqual(a: A[], b: A[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return false;
    }
    return true;
  }
}
