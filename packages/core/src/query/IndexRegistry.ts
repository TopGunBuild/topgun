/**
 * IndexRegistry Implementation
 *
 * Central registry for managing indexes on a collection.
 * Provides index lookup, lifecycle management, and bulk operations.
 *
 * @module query/IndexRegistry
 */

import type { Index, IndexQuery } from './indexes/types';
import type { Attribute } from './Attribute';

/**
 * Registry for managing indexes on a collection.
 * Provides index lookup and lifecycle management.
 *
 * K = record key type, V = record value type
 */
export class IndexRegistry<K, V> {
  /** Indexes grouped by attribute name */
  private attributeIndexes: Map<string, Index<K, V, unknown>[]> = new Map();

  /** Fallback index for full scan (optional) */
  private fallbackIndex: Index<K, V, unknown> | null = null;

  /**
   * Register an index for an attribute.
   * Multiple indexes can be registered for the same attribute.
   *
   * @param index - Index to register
   */
  addIndex<A>(index: Index<K, V, A>): void {
    const attrName = index.attribute.name;
    let indexes = this.attributeIndexes.get(attrName);

    if (!indexes) {
      indexes = [];
      this.attributeIndexes.set(attrName, indexes);
    }

    // Avoid duplicate registration
    if (!indexes.includes(index as Index<K, V, unknown>)) {
      indexes.push(index as Index<K, V, unknown>);
    }
  }

  /**
   * Remove an index from the registry.
   *
   * @param index - Index to remove
   * @returns true if index was found and removed
   */
  removeIndex<A>(index: Index<K, V, A>): boolean {
    const attrName = index.attribute.name;
    const indexes = this.attributeIndexes.get(attrName);

    if (!indexes) {
      return false;
    }

    const idx = indexes.indexOf(index as Index<K, V, unknown>);
    if (idx === -1) {
      return false;
    }

    indexes.splice(idx, 1);

    // Clean up empty arrays
    if (indexes.length === 0) {
      this.attributeIndexes.delete(attrName);
    }

    return true;
  }

  /**
   * Get all indexes for an attribute.
   *
   * @param attributeName - Attribute name
   * @returns Array of indexes (empty if none)
   */
  getIndexes(attributeName: string): Index<K, V, unknown>[] {
    return this.attributeIndexes.get(attributeName) ?? [];
  }

  /**
   * Get all registered indexes across all attributes.
   *
   * @returns Array of all indexes
   */
  getAllIndexes(): Index<K, V, unknown>[] {
    const all: Index<K, V, unknown>[] = [];
    for (const indexes of this.attributeIndexes.values()) {
      all.push(...indexes);
    }
    return all;
  }

  /**
   * Get all indexed attribute names.
   *
   * @returns Array of attribute names
   */
  getIndexedAttributes(): string[] {
    return Array.from(this.attributeIndexes.keys());
  }

  /**
   * Check if an attribute has any indexes.
   *
   * @param attributeName - Attribute name
   * @returns true if attribute has indexes
   */
  hasIndex(attributeName: string): boolean {
    const indexes = this.attributeIndexes.get(attributeName);
    return indexes !== undefined && indexes.length > 0;
  }

  /**
   * Find the best index for a query type on an attribute.
   * Returns the index with lowest retrieval cost that supports the query type.
   *
   * @param attributeName - Attribute name to search on
   * @param queryType - Query type (e.g., 'equal', 'gt', 'between')
   * @returns Best matching index or null if none found
   */
  findBestIndex(
    attributeName: string,
    queryType: string
  ): Index<K, V, unknown> | null {
    const indexes = this.getIndexes(attributeName);
    let best: Index<K, V, unknown> | null = null;
    let bestCost = Infinity;

    for (const index of indexes) {
      if (index.supportsQuery(queryType) && index.getRetrievalCost() < bestCost) {
        best = index;
        bestCost = index.getRetrievalCost();
      }
    }

    return best;
  }

  /**
   * Find all indexes that support a query type on an attribute.
   *
   * @param attributeName - Attribute name
   * @param queryType - Query type
   * @returns Array of matching indexes sorted by retrieval cost
   */
  findIndexes(
    attributeName: string,
    queryType: string
  ): Index<K, V, unknown>[] {
    const indexes = this.getIndexes(attributeName);
    return indexes
      .filter((index) => index.supportsQuery(queryType))
      .sort((a, b) => a.getRetrievalCost() - b.getRetrievalCost());
  }

  /**
   * Set a fallback index for queries without a suitable index.
   * Typically a FallbackIndex that performs full scan.
   *
   * @param fallback - Fallback index
   */
  setFallbackIndex(fallback: Index<K, V, unknown>): void {
    this.fallbackIndex = fallback;
  }

  /**
   * Get the fallback index.
   *
   * @returns Fallback index or null if not set
   */
  getFallbackIndex(): Index<K, V, unknown> | null {
    return this.fallbackIndex;
  }

  /**
   * Notify all indexes of a record addition.
   * Should be called when a new record is added to the collection.
   *
   * @param key - Record key
   * @param record - Record value
   */
  onRecordAdded(key: K, record: V): void {
    for (const indexes of this.attributeIndexes.values()) {
      for (const index of indexes) {
        index.add(key, record);
      }
    }
  }

  /**
   * Notify all indexes of a record update.
   * Should be called when a record's value changes.
   *
   * @param key - Record key
   * @param oldRecord - Previous record value
   * @param newRecord - New record value
   */
  onRecordUpdated(key: K, oldRecord: V, newRecord: V): void {
    for (const indexes of this.attributeIndexes.values()) {
      for (const index of indexes) {
        index.update(key, oldRecord, newRecord);
      }
    }
  }

  /**
   * Notify all indexes of a record removal.
   * Should be called when a record is removed from the collection.
   *
   * @param key - Record key
   * @param record - Removed record value
   */
  onRecordRemoved(key: K, record: V): void {
    for (const indexes of this.attributeIndexes.values()) {
      for (const index of indexes) {
        index.remove(key, record);
      }
    }
  }

  /**
   * Clear all indexes.
   * Does not remove index registrations, only clears their data.
   */
  clear(): void {
    for (const indexes of this.attributeIndexes.values()) {
      for (const index of indexes) {
        index.clear();
      }
    }
  }

  /**
   * Get total number of registered indexes.
   */
  get size(): number {
    let count = 0;
    for (const indexes of this.attributeIndexes.values()) {
      count += indexes.length;
    }
    return count;
  }

  /**
   * Get statistics about the registry.
   */
  getStats(): IndexRegistryStats {
    const indexes = this.getAllIndexes();
    const indexStats = indexes.map((index) => ({
      attribute: index.attribute.name,
      type: index.type,
      stats: index.getStats(),
    }));

    return {
      totalIndexes: indexes.length,
      indexedAttributes: this.getIndexedAttributes().length,
      indexes: indexStats,
    };
  }
}

/**
 * Statistics about the IndexRegistry.
 */
export interface IndexRegistryStats {
  /** Total number of indexes */
  totalIndexes: number;
  /** Number of indexed attributes */
  indexedAttributes: number;
  /** Stats for each index */
  indexes: Array<{
    attribute: string;
    type: string;
    stats: {
      distinctValues: number;
      totalEntries: number;
      avgEntriesPerValue: number;
    };
  }>;
}
