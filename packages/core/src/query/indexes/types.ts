/**
 * Base Index Interface
 *
 * Defines the contract for all index types in the Query Engine.
 * Inspired by CQEngine Index hierarchy.
 *
 * @module query/indexes/types
 */

import type { Attribute } from '../Attribute';
import type { ResultSet } from '../resultset/ResultSet';

/**
 * Base interface for all indexes.
 * K = record key type, V = record value type, A = attribute value type
 */
export interface Index<K, V, A = unknown> {
  /** Attribute this index is built on */
  readonly attribute: Attribute<V, A>;

  /** Index type identifier */
  readonly type: 'hash' | 'navigable' | 'compound' | 'standing' | 'inverted';

  /**
   * Cost of retrieving results from this index.
   * Lower is better. Used by QueryOptimizer.
   * CQEngine values: Standing=10, Compound=20, Hash=30, Navigable=40
   */
  getRetrievalCost(): number;

  /**
   * Check if this index supports the given query type.
   */
  supportsQuery(queryType: string): boolean;

  /**
   * Retrieve candidate keys matching the query.
   */
  retrieve(query: IndexQuery<A>): ResultSet<K>;

  /**
   * Add a record to the index.
   */
  add(key: K, record: V): void;

  /**
   * Remove a record from the index.
   */
  remove(key: K, record: V): void;

  /**
   * Update a record in the index.
   */
  update(key: K, oldRecord: V, newRecord: V): void;

  /**
   * Clear all entries from the index.
   */
  clear(): void;

  /**
   * Get statistics about the index.
   */
  getStats(): IndexStats;
}

/**
 * Query parameters for index retrieval.
 */
export interface IndexQuery<A> {
  /** Query type */
  type: 'equal' | 'in' | 'has' | 'gt' | 'gte' | 'lt' | 'lte' | 'between' | 'contains' | 'containsAll' | 'containsAny';
  /** Value for equality queries */
  value?: A;
  /** Values for 'in' queries */
  values?: A[];
  /** Lower bound for range queries */
  from?: A;
  /** Upper bound for range queries */
  to?: A;
  /** Include lower bound (default: true) */
  fromInclusive?: boolean;
  /** Include upper bound (default: false) */
  toInclusive?: boolean;
}

/**
 * Statistics about an index.
 */
export interface IndexStats {
  /** Number of distinct attribute values indexed */
  distinctValues: number;
  /** Total number of record references */
  totalEntries: number;
  /** Average entries per distinct value */
  avgEntriesPerValue: number;
}
