/**
 * ILiveQueryIndex Interface (Phase 12)
 *
 * Common interface for live query indexes that track query results
 * and provide delta notifications on record changes.
 *
 * Implementations:
 * - StandingQueryIndex: Binary match (exact/range predicates)
 * - LiveFTSIndex: Scored match (FTS predicates with BM25)
 *
 * @module query/indexes/ILiveQueryIndex
 */

import type { Query } from '../QueryTypes';

/**
 * Delta notification for live query updates.
 * Describes how a record change affected the query results.
 */
export interface LiveQueryDelta<K> {
  /**
   * Type of change:
   * - 'added': Record newly matches the query (ENTER)
   * - 'removed': Record no longer matches the query (LEAVE)
   * - 'updated': Record still matches but score/data changed (UPDATE)
   */
  type: 'added' | 'removed' | 'updated';

  /** Key of the affected record */
  key: K;

  /** New score (for FTS queries) */
  score?: number;

  /** Previous score (for UPDATE deltas) */
  oldScore?: number;

  /** Terms that matched the query (for FTS queries) */
  matchedTerms?: string[];
}

/**
 * Ranked result with score for FTS queries.
 */
export interface RankedResult<K> {
  /** Record key */
  key: K;

  /** BM25 score */
  score: number;

  /** Matched query terms */
  matchedTerms?: string[];
}

/**
 * Options for creating a LiveFTSIndex.
 */
export interface LiveFTSIndexOptions {
  /** Maximum number of results to track (Top-K) */
  maxResults?: number;

  /** Minimum score threshold for results */
  minScore?: number;

  /** Search query */
  query: string;

  /** Field being searched */
  field: string;
}

/**
 * Common interface for live query indexes.
 *
 * K = record key type
 * V = record value type
 * TResult = result type (K for binary, RankedResult<K> for scored)
 */
export interface ILiveQueryIndex<K, V, TResult = K> {
  /** Unique identifier for this index */
  readonly id: string;

  /** Query this index answers */
  readonly query: Query;

  /**
   * Get current results.
   * For binary indexes: returns keys
   * For scored indexes: returns RankedResult[]
   */
  getResults(): TResult[];

  /**
   * Get the number of results.
   */
  getResultCount(): number;

  /**
   * Check if a key is in the results.
   */
  contains(key: K): boolean;

  /**
   * Handle a new record being added.
   * Returns a delta if the record affects the query results.
   *
   * @param key - Record key
   * @param record - Record value
   * @returns Delta describing the change, or null if no effect
   */
  onRecordAdded(key: K, record: V): LiveQueryDelta<K> | null;

  /**
   * Handle a record being updated.
   * Returns a delta if the change affects the query results.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value
   * @param newRecord - New record value
   * @returns Delta describing the change, or null if no effect
   */
  onRecordUpdated(key: K, oldRecord: V, newRecord: V): LiveQueryDelta<K> | null;

  /**
   * Handle a record being removed.
   * Returns a delta if the record was in the results.
   *
   * @param key - Record key
   * @param record - Removed record value
   * @returns Delta describing the change, or null if no effect
   */
  onRecordRemoved(key: K, record: V): LiveQueryDelta<K> | null;

  /**
   * Build the index from existing data.
   *
   * @param entries - Iterable of [key, record] pairs
   */
  buildFromData(entries: Iterable<[K, V]>): void;

  /**
   * Clear all data from the index.
   */
  clear(): void;
}
