/**
 * QueryManager - Manages query subscriptions and local query execution
 *
 * This module extracts query management from SyncEngine:
 * - Standard query subscriptions (QueryHandle)
 * - Hybrid query subscriptions (HybridQueryHandle with FTS)
 * - Local query execution against storage adapter
 */

import { evaluatePredicate } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { QueryHandle, QueryFilter } from '../QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from '../HybridQueryHandle';
import type { IQueryManager, QueryManagerConfig } from './types';

/**
 * QueryManager handles all query-related operations for SyncEngine.
 * It owns the queries and hybridQueries Maps (single source of truth).
 */
export class QueryManager implements IQueryManager {
  private readonly config: QueryManagerConfig;

  /** Standard queries (single source of truth) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry; typed handles are created in TopGunClient
  private queries: Map<string, QueryHandle<any>> = new Map();

  /** Hybrid queries with FTS support (single source of truth) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased in the manager registry
  private hybridQueries: Map<string, HybridQueryHandle<any>> = new Map();

  constructor(config: QueryManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Query Access Methods
  // ============================================

  /**
   * Get all queries (read-only access).
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the manager interface; individual handles are typed at creation
  public getQueries(): Map<string, QueryHandle<any>> {
    return this.queries;
  }

  /**
   * Get all hybrid queries.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the manager interface
  public getHybridQueries(): Map<string, HybridQueryHandle<any>> {
    return this.hybridQueries;
  }

  /**
   * Get a hybrid query by ID.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- hybrid query type parameter erased at lookup time; caller narrows the result
  public getHybridQuery(queryId: string): HybridQueryHandle<any> | undefined {
    return this.hybridQueries.get(queryId);
  }

  // ============================================
  // Standard Query Methods
  // ============================================

  /**
   * Subscribe to a standard query.
   * Adds to queries Map and sends subscription to server if authenticated.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at subscription time; actual type lives in the handle's internal state
  public subscribeToQuery(query: QueryHandle<any>): void {
    this.queries.set(query.id, query);
    if (this.config.isAuthenticated()) {
      this.sendQuerySubscription(query);
    }
  }

  /**
   * Unsubscribe from a query.
   * Removes from Map and sends unsubscription to server if authenticated.
   */
  public unsubscribeFromQuery(queryId: string): void {
    this.queries.delete(queryId);
    if (this.config.isAuthenticated()) {
      this.config.sendMessage({
        type: 'QUERY_UNSUB',
        payload: { queryId },
      });
    }
  }

  /**
   * Send query subscription message to server.
   * Includes field projection when specified in the query filter.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- query type parameter erased at the routing layer; only the filter and map name are needed for the subscription message
  private sendQuerySubscription(query: QueryHandle<any>): void {
    const filter = query.getFilter();
    this.config.sendMessage({
      type: 'QUERY_SUB',
      payload: {
        queryId: query.id,
        mapName: query.getMapName(),
        query: filter,
        fields: filter.fields,
      },
    });
  }

  // ============================================
  // Hybrid Query Methods
  // ============================================

  /**
   * Subscribe to a hybrid query (FTS + filter combination).
   */
  public subscribeToHybridQuery<T>(query: HybridQueryHandle<T>): void {
    this.hybridQueries.set(query.id, query);

    const filter = query.getFilter();
    const mapName = query.getMapName();

    // If query has FTS predicate and authenticated, send to server
    if (query.hasFTSPredicate() && this.config.isAuthenticated()) {
      this.sendHybridQuerySubscription(query.id, mapName, filter);
    }

    // Load initial local data
    this.runLocalHybridQuery<T>(mapName, filter).then((results) => {
      query.onResult(results, 'local');
    });
  }

  /**
   * Unsubscribe from a hybrid query.
   */
  public unsubscribeFromHybridQuery(queryId: string): void {
    const query = this.hybridQueries.get(queryId);
    if (query) {
      this.hybridQueries.delete(queryId);

      // Notify server to unsubscribe
      if (this.config.isAuthenticated() && query.hasFTSPredicate()) {
        this.config.sendMessage({
          type: 'HYBRID_QUERY_UNSUBSCRIBE',
          payload: { subscriptionId: queryId },
        });
      }
    }
  }

  /**
   * Send hybrid query subscription message to server.
   */
  private sendHybridQuerySubscription(
    queryId: string,
    mapName: string,
    filter: HybridQueryFilter,
  ): void {
    this.config.sendMessage({
      type: 'HYBRID_QUERY_SUBSCRIBE',
      payload: {
        subscriptionId: queryId,
        mapName,
        predicate: filter.predicate,
        where: filter.where,
        sort: filter.sort,
        limit: filter.limit,
        cursor: filter.cursor,
      },
    });
  }

  // ============================================
  // Local Query Execution
  // ============================================

  /**
   * Executes a query against local storage immediately.
   */
  public async runLocalQuery(
    mapName: string,
    filter: QueryFilter,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- local query returns raw record values whose type is unknown at the query manager level; callers cast to T
  ): Promise<{ key: string; value: any }[]> {
    // Retrieve all keys for the map
    const keys = await this.config.storageAdapter.getAllKeys();
    const mapKeys = keys.filter((k) => k.startsWith(mapName + ':'));

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- accumulator for untyped local records; typed after return by the caller
    const results: { key: string; value: any }[] = [];
    for (const fullKey of mapKeys) {
      const record = await this.config.storageAdapter.get(fullKey);
      if (record && record.value) {
        // Extract actual key from "mapName:key"
        const actualKey = fullKey.slice(mapName.length + 1);

        let matches = true;

        // Apply 'where' (equality)
        if (filter.where) {
          for (const [k, v] of Object.entries(filter.where)) {
            if (record.value[k] !== v) {
              matches = false;
              break;
            }
          }
        }

        // Apply 'predicate'
        if (matches && filter.predicate) {
          if (!evaluatePredicate(filter.predicate, record.value)) {
            matches = false;
          }
        }

        if (matches) {
          results.push({ key: actualKey, value: record.value });
        }
      }
    }
    return results;
  }

  /**
   * Run a local hybrid query (FTS + filter combination).
   * For FTS predicates, returns results with score = 0 (local-only mode).
   * Server provides actual FTS scoring.
   */
  public async runLocalHybridQuery<T>(
    mapName: string,
    filter: HybridQueryFilter,
  ): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>> {
    const results: Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }> = [];

    // Get all entries from the map using storage adapter
    const allKeys = await this.config.storageAdapter.getAllKeys();
    const mapPrefix = `${mapName}:`;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- raw storage entries before type narrowing; value type determined per-record after deserialization
    const entries: Array<[string, any]> = [];

    for (const fullKey of allKeys) {
      if (fullKey.startsWith(mapPrefix)) {
        const key = fullKey.substring(mapPrefix.length);
        const record = await this.config.storageAdapter.get(fullKey);
        if (record) {
          entries.push([key, record]);
        }
      }
    }

    for (const [key, record] of entries) {
      if (record === null || record.value === null) continue;

      const value = record.value as T;

      // Evaluate predicate (including FTS predicates - basic local evaluation)
      if (filter.predicate) {
        const matches = evaluatePredicate(filter.predicate, value as Record<string, unknown>);
        if (!matches) continue;
      }

      // Evaluate where clause
      if (filter.where) {
        let whereMatches = true;
        for (const [field, expected] of Object.entries(filter.where)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- value is cast to any for dynamic field access by string key in the where clause
          if ((value as any)[field] !== expected) {
            whereMatches = false;
            break;
          }
        }
        if (!whereMatches) continue;
      }

      results.push({
        key,
        value,
        score: 0, // Local doesn't have FTS scoring
        matchedTerms: [],
      });
    }

    // Sort results
    if (filter.sort) {
      results.sort((a, b) => {
        for (const [field, direction] of Object.entries(filter.sort!)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- sort comparator accesses dynamic field keys on typed results; any is used to index by runtime field name
          let valA: any;
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- sort comparator accesses dynamic field keys on typed results
          let valB: any;

          if (field === '_score') {
            valA = a.score ?? 0;
            valB = b.score ?? 0;
          } else if (field === '_key') {
            valA = a.key;
            valB = b.key;
          } else {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access on typed value; sort field name is a runtime string
            valA = (a.value as any)[field];
            // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic field access on typed value; sort field name is a runtime string
            valB = (b.value as any)[field];
          }

          if (valA < valB) return direction === 'asc' ? -1 : 1;
          if (valA > valB) return direction === 'asc' ? 1 : -1;
        }
        return 0;
      });
    }

    // Apply limit (cursor filtering is done server-side)
    let sliced = results;
    if (filter.limit) {
      sliced = sliced.slice(0, filter.limit);
    }

    return sliced;
  }

  // ============================================
  // Re-subscription (after auth)
  // ============================================

  /**
   * Re-subscribe all queries after authentication.
   * Called by SyncEngine after AUTH_ACK.
   */
  public resubscribeAll(): void {
    logger.debug(
      { queryCount: this.queries.size, hybridCount: this.hybridQueries.size },
      'QueryManager: resubscribing all queries',
    );

    // Re-subscribe standard queries
    for (const query of this.queries.values()) {
      this.sendQuerySubscription(query);

      // Delta reconnect for field-projected queries: send QUERY_SYNC_INIT when we have
      // a stored Merkle root hash so the server can diff and send only changed records
      const filter = query.getFilter();
      if (filter.fields && filter.fields.length > 0 && query.merkleRootHash !== 0) {
        this.config.sendMessage({
          type: 'QUERY_SYNC_INIT',
          payload: {
            queryId: query.id,
            rootHash: query.merkleRootHash,
          },
        });
      }
    }

    // Re-subscribe hybrid queries with FTS predicates
    for (const query of this.hybridQueries.values()) {
      if (query.hasFTSPredicate()) {
        this.sendHybridQuerySubscription(query.id, query.getMapName(), query.getFilter());
      }
    }
  }
}
