/**
 * LiveQueryManager Implementation
 *
 * Manages live query subscriptions using StandingQueryIndexes.
 * Provides reactive updates when data changes.
 *
 * Features:
 * - Initial results on subscribe
 * - Delta updates on record changes
 * - Shared indexes for identical queries
 * - Automatic cleanup on unsubscribe
 *
 * @module query/LiveQueryManager
 */

import {
  StandingQueryRegistry,
  type StandingQueryRegistryOptions,
  type StandingQueryRegistryStats,
} from './StandingQueryRegistry';
import type { StandingQueryChange } from './indexes/StandingQueryIndex';
import type { Query } from './QueryTypes';

/**
 * Initial results event sent when subscribing.
 */
export interface LiveQueryInitialEvent<K> {
  type: 'initial';
  query: Query;
  results: K[];
}

/**
 * Delta event sent when data changes.
 */
export interface LiveQueryDeltaEvent<K, V> {
  type: 'delta';
  query: Query;
  key: K;
  record: V;
  change: StandingQueryChange;
  operation: 'added' | 'updated' | 'removed';
  newResultCount: number;
}

/**
 * Union type for all live query events.
 */
export type LiveQueryEvent<K, V> = LiveQueryInitialEvent<K> | LiveQueryDeltaEvent<K, V>;

/**
 * Callback for live query events.
 */
export type LiveQueryCallback<K, V> = (event: LiveQueryEvent<K, V>) => void;

/**
 * Options for creating a LiveQueryManager.
 */
export interface LiveQueryManagerOptions<K, V> {
  /** Function to get record by key */
  getRecord: (key: K) => V | undefined;
  /** Function to get all entries for building index */
  getAllEntries: () => Iterable<[K, V]>;
}

/**
 * Manages live query subscriptions using StandingQueryIndexes.
 * Provides reactive updates when data changes.
 *
 * K = record key type, V = record value type
 */
export class LiveQueryManager<K, V> {
  private registry: StandingQueryRegistry<K, V>;

  /** Subscription callbacks by query hash */
  private subscriptions: Map<string, Set<LiveQueryCallback<K, V>>> = new Map();

  constructor(options: LiveQueryManagerOptions<K, V>) {
    const registryOptions: StandingQueryRegistryOptions<K, V> = {
      getRecord: options.getRecord,
      getAllEntries: options.getAllEntries,
    };
    this.registry = new StandingQueryRegistry(registryOptions);
  }

  /**
   * Subscribe to a live query.
   * Sends initial results immediately, then delta updates on changes.
   *
   * @param query - Query to subscribe to
   * @param callback - Callback for query events
   * @returns Unsubscribe function
   */
  subscribe(query: Query, callback: LiveQueryCallback<K, V>): () => void {
    const hash = this.registry.hashQuery(query);

    // Register standing query index
    const index = this.registry.register(query);

    // Add callback to subscriptions
    let callbacks = this.subscriptions.get(hash);
    if (!callbacks) {
      callbacks = new Set();
      this.subscriptions.set(hash, callbacks);
    }
    callbacks.add(callback);

    // Send initial results
    const initialResults = Array.from(index.getResults());
    try {
      callback({
        type: 'initial',
        query,
        results: initialResults,
      });
    } catch (error) {
      console.error('LiveQueryManager initial callback error:', error);
    }

    // Return unsubscribe function
    return () => {
      callbacks?.delete(callback);
      if (callbacks?.size === 0) {
        this.subscriptions.delete(hash);
      }
      this.registry.unregister(query);
    };
  }

  /**
   * Get current results for a query (snapshot).
   * Does not subscribe to updates.
   *
   * @param query - Query to execute
   * @returns Array of matching keys
   */
  getResults(query: Query): K[] {
    const index = this.registry.getIndex(query);
    return index ? Array.from(index.getResults()) : [];
  }

  /**
   * Check if a query has active subscriptions.
   *
   * @param query - Query to check
   * @returns true if query has subscribers
   */
  hasSubscribers(query: Query): boolean {
    const hash = this.registry.hashQuery(query);
    const callbacks = this.subscriptions.get(hash);
    return callbacks !== undefined && callbacks.size > 0;
  }

  /**
   * Get subscriber count for a query.
   *
   * @param query - Query to check
   * @returns Number of subscribers
   */
  getSubscriberCount(query: Query): number {
    const hash = this.registry.hashQuery(query);
    const callbacks = this.subscriptions.get(hash);
    return callbacks?.size ?? 0;
  }

  /**
   * Notify of record addition.
   * Triggers subscription callbacks for affected queries.
   *
   * @param key - Record key
   * @param record - New record value
   */
  onRecordAdded(key: K, record: V): void {
    const changes = this.registry.onRecordAdded(key, record);
    this.notifySubscribers(key, record, changes, 'added');
  }

  /**
   * Notify of record update.
   * Triggers subscription callbacks for affected queries.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value
   * @param newRecord - New record value
   */
  onRecordUpdated(key: K, oldRecord: V, newRecord: V): void {
    const changes = this.registry.onRecordUpdated(key, oldRecord, newRecord);
    this.notifySubscribers(key, newRecord, changes, 'updated');
  }

  /**
   * Notify of record removal.
   * Triggers subscription callbacks for affected queries.
   *
   * @param key - Record key
   * @param record - Removed record value
   */
  onRecordRemoved(key: K, record: V): void {
    const changes = this.registry.onRecordRemoved(key, record);
    this.notifySubscribers(key, record, changes, 'removed');
  }

  /**
   * Notify subscribers of changes.
   */
  private notifySubscribers(
    key: K,
    record: V,
    changes: Map<string, StandingQueryChange>,
    operation: 'added' | 'updated' | 'removed'
  ): void {
    for (const [hash, change] of changes) {
      const callbacks = this.subscriptions.get(hash);
      if (!callbacks || callbacks.size === 0) continue;

      const index = this.registry.getIndexByHash(hash);
      if (!index) continue;

      const query = index.getQuery();

      for (const callback of callbacks) {
        try {
          callback({
            type: 'delta',
            query,
            key,
            record,
            change,
            operation,
            newResultCount: index.getResultCount(),
          });
        } catch (error) {
          // Don't let one callback failure affect others
          console.error('LiveQueryManager callback error:', error);
        }
      }
    }
  }

  /**
   * Get the underlying registry for direct access.
   * Useful for testing and debugging.
   *
   * @returns StandingQueryRegistry instance
   */
  getRegistry(): StandingQueryRegistry<K, V> {
    return this.registry;
  }

  /**
   * Get all active query hashes.
   *
   * @returns Array of query hashes with active subscriptions
   */
  getActiveQueries(): string[] {
    return Array.from(this.subscriptions.keys());
  }

  /**
   * Get statistics about the manager.
   *
   * @returns Statistics object
   */
  getStats(): LiveQueryManagerStats {
    const registryStats = this.registry.getStats();
    const totalSubscribers = Array.from(this.subscriptions.values()).reduce(
      (sum, callbacks) => sum + callbacks.size,
      0
    );

    return {
      ...registryStats,
      activeQueries: this.subscriptions.size,
      totalSubscribers,
    };
  }

  /**
   * Clear all subscriptions and indexes.
   */
  clear(): void {
    this.subscriptions.clear();
    this.registry.clear();
  }
}

/**
 * Statistics about the LiveQueryManager.
 */
export interface LiveQueryManagerStats extends StandingQueryRegistryStats {
  /** Number of active queries with subscribers */
  activeQueries: number;
  /** Total number of subscribers across all queries */
  totalSubscribers: number;
}
