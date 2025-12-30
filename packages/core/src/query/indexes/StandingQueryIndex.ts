/**
 * StandingQueryIndex Implementation
 *
 * Pre-computed index for a specific query.
 * Maintains result set incrementally as data changes.
 *
 * Lowest retrieval cost (10) - O(1) query execution.
 * Critical for Live Queries in TopGun where the same query
 * is executed repeatedly on every data change.
 *
 * CQEngine Reference: StandingQueryIndex.java (retrieval cost: 10)
 *
 * @module query/indexes/StandingQueryIndex
 */

import type { Attribute } from '../Attribute';
import { simpleAttribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';
import type { Query, SimpleQueryNode, LogicalQueryNode } from '../QueryTypes';
import { isSimpleQuery, isLogicalQuery } from '../QueryTypes';

/**
 * Change type for standing query updates.
 */
export type StandingQueryChange = 'added' | 'removed' | 'updated' | 'unchanged';

/**
 * Options for creating a StandingQueryIndex.
 */
export interface StandingQueryIndexOptions<K, V> {
  /** Query this index answers */
  query: Query;
  /** Function to get record by key (optional, for validation) */
  getRecord?: (key: K) => V | undefined;
}

/**
 * Pre-computed index for a specific query.
 * Maintains result set incrementally as data changes.
 *
 * K = record key type, V = record value type
 */
export class StandingQueryIndex<K, V> implements Index<K, V, unknown> {
  readonly type = 'standing' as const;

  /**
   * Wildcard attribute - StandingQueryIndex doesn't index by attribute,
   * it indexes by query match.
   */
  readonly attribute: Attribute<V, unknown> = simpleAttribute<V, unknown>(
    '*',
    () => undefined
  );

  /** Pre-computed result set */
  private results: Set<K> = new Set();

  /** Query this index answers */
  private readonly query: Query;

  /** Record accessor (optional) */
  private readonly getRecord?: (key: K) => V | undefined;

  /** Retrieval cost - lowest of all index types */
  private static readonly RETRIEVAL_COST = 10;

  constructor(options: StandingQueryIndexOptions<K, V>) {
    this.query = options.query;
    this.getRecord = options.getRecord;
  }

  /**
   * Get the query this index answers.
   */
  getQuery(): Query {
    return this.query;
  }

  /**
   * Check if this index answers the given query.
   */
  answersQuery(query: Query): boolean {
    return this.queriesEqual(this.query, query);
  }

  getRetrievalCost(): number {
    return StandingQueryIndex.RETRIEVAL_COST;
  }

  supportsQuery(_queryType: string): boolean {
    // StandingQueryIndex supports any query type since it pre-computes results
    // The actual query matching is done via answersQuery()
    return true;
  }

  retrieve(_query: IndexQuery<unknown>): ResultSet<K> {
    // Return pre-computed results (copy to avoid mutation)
    return new SetResultSet(new Set(this.results), StandingQueryIndex.RETRIEVAL_COST);
  }

  /**
   * Get current result set (for live query updates).
   * Returns a copy to avoid external mutation.
   */
  getResults(): Set<K> {
    return new Set(this.results);
  }

  /**
   * Get result count.
   */
  getResultCount(): number {
    return this.results.size;
  }

  /**
   * Check if key is in results.
   */
  contains(key: K): boolean {
    return this.results.has(key);
  }

  add(key: K, record: V): void {
    // Evaluate predicate and add if matches
    if (this.evaluateRecord(record)) {
      this.results.add(key);
    }
  }

  remove(key: K, _record: V): void {
    // Always try to remove (may or may not be in results)
    this.results.delete(key);
  }

  update(key: K, oldRecord: V, newRecord: V): void {
    const wasMatch = this.evaluateRecord(oldRecord);
    const isMatch = this.evaluateRecord(newRecord);

    if (wasMatch && !isMatch) {
      // Was in results, no longer matches
      this.results.delete(key);
    } else if (!wasMatch && isMatch) {
      // Was not in results, now matches
      this.results.add(key);
    }
    // If both match or neither match, no change to results set
    // (though the record itself may have changed)
  }

  /**
   * Determine what changed for a record update.
   * Returns the type of change relative to the query results.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value (undefined for new records)
   * @param newRecord - New record value (undefined for deleted records)
   * @returns Change type: 'added', 'removed', 'updated', or 'unchanged'
   */
  determineChange(
    _key: K,
    oldRecord: V | undefined,
    newRecord: V | undefined
  ): StandingQueryChange {
    const wasMatch = oldRecord ? this.evaluateRecord(oldRecord) : false;
    const isMatch = newRecord ? this.evaluateRecord(newRecord) : false;

    if (!wasMatch && isMatch) {
      return 'added';
    } else if (wasMatch && !isMatch) {
      return 'removed';
    } else if (wasMatch && isMatch) {
      return 'updated';
    }
    return 'unchanged';
  }

  clear(): void {
    this.results.clear();
  }

  getStats(): IndexStats {
    return {
      distinctValues: 1, // Single query
      totalEntries: this.results.size,
      avgEntriesPerValue: this.results.size,
    };
  }

  /**
   * Build index from existing data.
   *
   * @param entries - Iterable of [key, record] pairs
   */
  buildFromData(entries: Iterable<[K, V]>): void {
    this.results.clear();
    for (const [key, record] of entries) {
      if (this.evaluateRecord(record)) {
        this.results.add(key);
      }
    }
  }

  /**
   * Evaluate a record against the query predicate.
   *
   * @param record - Record to evaluate
   * @returns true if record matches the query
   */
  private evaluateRecord(record: V): boolean {
    try {
      return this.evaluateQuery(this.query, record);
    } catch {
      return false;
    }
  }

  /**
   * Evaluate a query node against a record.
   * Implements predicate evaluation logic.
   */
  private evaluateQuery(query: Query, record: V): boolean {
    if (isSimpleQuery(query)) {
      return this.evaluateSimpleQuery(query, record);
    } else if (isLogicalQuery(query)) {
      return this.evaluateLogicalQuery(query, record);
    }
    return false;
  }

  /**
   * Evaluate a simple query (attribute-based condition).
   */
  private evaluateSimpleQuery(query: SimpleQueryNode, record: V): boolean {
    const value = this.getAttributeValue(record, query.attribute);

    switch (query.type) {
      case 'eq':
        return value === query.value;

      case 'neq':
        return value !== query.value;

      case 'gt':
        return (
          value !== undefined &&
          value !== null &&
          (value as number) > (query.value as number)
        );

      case 'gte':
        return (
          value !== undefined &&
          value !== null &&
          (value as number) >= (query.value as number)
        );

      case 'lt':
        return (
          value !== undefined &&
          value !== null &&
          (value as number) < (query.value as number)
        );

      case 'lte':
        return (
          value !== undefined &&
          value !== null &&
          (value as number) <= (query.value as number)
        );

      case 'in':
        return query.values !== undefined && query.values.includes(value);

      case 'has':
        return value !== undefined && value !== null;

      case 'like':
        if (typeof value !== 'string' || typeof query.value !== 'string') {
          return false;
        }
        return this.matchLike(value, query.value);

      case 'regex':
        if (typeof value !== 'string' || typeof query.value !== 'string') {
          return false;
        }
        try {
          return new RegExp(query.value).test(value);
        } catch {
          return false;
        }

      case 'between':
        if (value === undefined || value === null) {
          return false;
        }
        const val = value as number | string;
        const from = query.from as number | string;
        const to = query.to as number | string;
        const fromOk = query.fromInclusive !== false ? val >= from : val > from;
        const toOk = query.toInclusive !== false ? val <= to : val < to;
        return fromOk && toOk;

      case 'contains':
        if (typeof value !== 'string' || typeof query.value !== 'string') {
          return false;
        }
        return value.toLowerCase().includes((query.value as string).toLowerCase());

      case 'containsAll':
        if (typeof value !== 'string' || !query.values) {
          return false;
        }
        return query.values.every(
          (v) => typeof v === 'string' && value.toLowerCase().includes(v.toLowerCase())
        );

      case 'containsAny':
        if (typeof value !== 'string' || !query.values) {
          return false;
        }
        return query.values.some(
          (v) => typeof v === 'string' && value.toLowerCase().includes(v.toLowerCase())
        );

      default:
        return false;
    }
  }

  /**
   * Evaluate a logical query (AND/OR/NOT).
   */
  private evaluateLogicalQuery(query: LogicalQueryNode, record: V): boolean {
    switch (query.type) {
      case 'and':
        if (!query.children || query.children.length === 0) {
          return true;
        }
        return query.children.every((child) => this.evaluateQuery(child, record));

      case 'or':
        if (!query.children || query.children.length === 0) {
          return false;
        }
        return query.children.some((child) => this.evaluateQuery(child, record));

      case 'not':
        if (!query.child) {
          return true;
        }
        return !this.evaluateQuery(query.child, record);

      default:
        return false;
    }
  }

  /**
   * Get attribute value from record using dot notation.
   */
  private getAttributeValue(record: V, path: string): unknown {
    if (record === null || record === undefined) {
      return undefined;
    }

    const parts = path.split('.');
    let current: unknown = record;

    for (const part of parts) {
      if (current === null || current === undefined) {
        return undefined;
      }
      if (typeof current !== 'object') {
        return undefined;
      }
      current = (current as Record<string, unknown>)[part];
    }

    return current;
  }

  /**
   * Match a value against a LIKE pattern.
   * Supports % as wildcard for any characters.
   */
  private matchLike(value: string, pattern: string): boolean {
    // Convert LIKE pattern to regex
    const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&');
    const regex = escaped.replace(/%/g, '.*').replace(/_/g, '.');
    return new RegExp(`^${regex}$`, 'i').test(value);
  }

  /**
   * Deep equality check for queries.
   */
  private queriesEqual(a: Query, b: Query): boolean {
    return JSON.stringify(a) === JSON.stringify(b);
  }
}
