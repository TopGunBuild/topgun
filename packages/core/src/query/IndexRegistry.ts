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
import { isCompoundIndex, type CompoundIndex } from './indexes/CompoundIndex';

/**
 * Registry for managing indexes on a collection.
 * Provides index lookup and lifecycle management.
 *
 * K = record key type, V = record value type
 */
export class IndexRegistry<K, V> {
  /** Indexes grouped by attribute name */
  private attributeIndexes: Map<string, Index<K, V, unknown>[]> = new Map();

  /** Compound indexes (Phase 9.03) - keyed by sorted attribute names */
  private compoundIndexes: Map<string, CompoundIndex<K, V>> = new Map();

  /** Fallback index for full scan (optional) */
  private fallbackIndex: Index<K, V, unknown> | null = null;

  /**
   * Register an index for an attribute.
   * Multiple indexes can be registered for the same attribute.
   *
   * @param index - Index to register
   */
  addIndex<A>(index: Index<K, V, A>): void {
    // Handle compound indexes specially (Phase 9.03)
    if (isCompoundIndex(index)) {
      this.addCompoundIndex(index as unknown as CompoundIndex<K, V>);
      return;
    }

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
   * Register a compound index (Phase 9.03).
   *
   * @param index - Compound index to register
   */
  addCompoundIndex(index: CompoundIndex<K, V>): void {
    const key = this.makeCompoundKey(index.attributes.map((a) => a.name));
    this.compoundIndexes.set(key, index);
  }

  /**
   * Remove an index from the registry.
   *
   * @param index - Index to remove
   * @returns true if index was found and removed
   */
  removeIndex<A>(index: Index<K, V, A>): boolean {
    // Handle compound indexes specially (Phase 9.03)
    if (isCompoundIndex(index)) {
      return this.removeCompoundIndex(index as unknown as CompoundIndex<K, V>);
    }

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
   * Remove a compound index (Phase 9.03).
   *
   * @param index - Compound index to remove
   * @returns true if index was found and removed
   */
  removeCompoundIndex(index: CompoundIndex<K, V>): boolean {
    const key = this.makeCompoundKey(index.attributes.map((a) => a.name));
    return this.compoundIndexes.delete(key);
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

  // ========================================
  // Phase 9.03: Compound Index Methods
  // ========================================

  /**
   * Find a compound index that covers the given attribute names (Phase 9.03).
   * The compound index must cover ALL the attributes (exact match or superset).
   *
   * @param attributeNames - Array of attribute names to search for
   * @returns Matching compound index or null
   */
  findCompoundIndex(attributeNames: string[]): CompoundIndex<K, V> | null {
    if (attributeNames.length < 2) {
      return null;
    }

    // Try exact match first (most efficient)
    const key = this.makeCompoundKey(attributeNames);
    const exactMatch = this.compoundIndexes.get(key);
    if (exactMatch) {
      return exactMatch;
    }

    // No exact match - compound indexes require exact attribute match
    // (unlike SQL where prefix matching is possible)
    return null;
  }

  /**
   * Check if a compound index exists for the given attributes (Phase 9.03).
   *
   * @param attributeNames - Array of attribute names
   * @returns true if a compound index exists
   */
  hasCompoundIndex(attributeNames: string[]): boolean {
    return this.findCompoundIndex(attributeNames) !== null;
  }

  /**
   * Get all compound indexes (Phase 9.03).
   *
   * @returns Array of all compound indexes
   */
  getCompoundIndexes(): CompoundIndex<K, V>[] {
    return Array.from(this.compoundIndexes.values());
  }

  /**
   * Create a compound key from attribute names (sorted for consistency).
   */
  private makeCompoundKey(attributeNames: string[]): string {
    return [...attributeNames].sort().join('+');
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
    // Also update compound indexes (Phase 9.03)
    for (const compoundIndex of this.compoundIndexes.values()) {
      compoundIndex.add(key, record);
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
    // Also update compound indexes (Phase 9.03)
    for (const compoundIndex of this.compoundIndexes.values()) {
      compoundIndex.update(key, oldRecord, newRecord);
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
    // Also update compound indexes (Phase 9.03)
    for (const compoundIndex of this.compoundIndexes.values()) {
      compoundIndex.remove(key, record);
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
    // Also clear compound indexes (Phase 9.03)
    for (const compoundIndex of this.compoundIndexes.values()) {
      compoundIndex.clear();
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
    // Include compound indexes (Phase 9.03)
    count += this.compoundIndexes.size;
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

    // Add compound index stats (Phase 9.03)
    for (const compoundIndex of this.compoundIndexes.values()) {
      indexStats.push({
        attribute: compoundIndex.compoundName,
        type: 'compound',
        stats: compoundIndex.getStats(),
      });
    }

    return {
      totalIndexes: indexes.length + this.compoundIndexes.size,
      indexedAttributes: this.getIndexedAttributes().length,
      compoundIndexes: this.compoundIndexes.size,
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
  /** Number of compound indexes (Phase 9.03) */
  compoundIndexes?: number;
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
