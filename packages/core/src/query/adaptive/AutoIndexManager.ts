/**
 * * AutoIndexManager
 *
 * Automatically creates indexes based on query patterns.
 * Intended for development mode to simplify index management.
 *
 * Features:
 * - Automatic index creation after threshold queries
 * - Safety limits to prevent memory exhaustion
 * - Callback notifications for index creation events
 * - Integration with IndexAdvisor for type selection
 *
 * @module query/adaptive/AutoIndexManager
 */

import type { IndexAdvisor } from './IndexAdvisor';
import type { QueryPatternTracker } from './QueryPatternTracker';
import type {
  AutoIndexConfig,
  RecommendedIndexType,
  TrackedQueryType,
} from './types';
import { ADAPTIVE_INDEXING_DEFAULTS } from './types';
import type { Attribute } from '../Attribute';

/**
 * Interface for indexed map operations.
 * Used to decouple AutoIndexManager from IndexedLWWMap/IndexedORMap.
 */
export interface IndexableMap<K, V> {
  /** Get all current indexes */
  getIndexes(): { attribute: { name: string }; type: string }[];

  /** Check if attribute has an index */
  hasIndexOn(attributeName: string): boolean;

  /** Add a hash index */
  addHashIndex<A>(attribute: Attribute<V, A>): void;

  /** Add a navigable index */
  addNavigableIndex<A extends string | number>(attribute: Attribute<V, A>): void;

  /** Add an inverted index */
  addInvertedIndex<A extends string>(attribute: Attribute<V, A>): void;
}

/**
 * Registered attribute info for auto-indexing.
 */
interface RegisteredAttribute<V, A> {
  attribute: Attribute<V, A>;
  allowedIndexTypes?: RecommendedIndexType[];
}

/**
 * AutoIndexManager automatically creates indexes based on query patterns.
 *
 * @example
 * ```typescript
 * const manager = new AutoIndexManager(tracker, advisor, {
 *   enabled: true,
 *   threshold: 10,
 *   maxIndexes: 20,
 *   onIndexCreated: (attr, type) => console.log(`Created ${type} on ${attr}`)
 * });
 *
 * manager.registerAttribute(simpleAttribute('category', p => p.category));
 * manager.setMap(indexedMap);
 *
 * // After 10 queries on 'category', index is auto-created
 * ```
 */
export class AutoIndexManager<K, V> {
  private readonly config: Required<AutoIndexConfig>;
  private readonly attributeQueryCounts = new Map<string, number>();
  private readonly registeredAttributes = new Map<string, RegisteredAttribute<V, unknown>>();
  private readonly createdIndexes = new Set<string>();
  private map: IndexableMap<K, V> | null = null;

  constructor(
    private readonly tracker: QueryPatternTracker,
    private readonly advisor: IndexAdvisor,
    config: AutoIndexConfig
  ) {
    this.config = {
      enabled: config.enabled,
      threshold: config.threshold ?? ADAPTIVE_INDEXING_DEFAULTS.autoIndex.threshold!,
      maxIndexes: config.maxIndexes ?? ADAPTIVE_INDEXING_DEFAULTS.autoIndex.maxIndexes!,
      onIndexCreated: config.onIndexCreated ?? (() => {}),
    };
  }

  /**
   * Set the indexed map to create indexes on.
   */
  setMap(map: IndexableMap<K, V>): void {
    this.map = map;
    // Count existing indexes
    for (const index of map.getIndexes()) {
      this.createdIndexes.add(index.attribute.name);
    }
  }

  /**
   * Register an attribute that can be auto-indexed.
   *
   * @param attribute - The attribute to register
   * @param allowedIndexTypes - Optional list of allowed index types
   */
  registerAttribute<A>(
    attribute: Attribute<V, A>,
    allowedIndexTypes?: RecommendedIndexType[]
  ): void {
    this.registeredAttributes.set(attribute.name, {
      attribute: attribute as unknown as Attribute<V, unknown>,
      allowedIndexTypes,
    });
  }

  /**
   * Unregister an attribute.
   *
   * @param attributeName - Name of attribute to unregister
   */
  unregisterAttribute(attributeName: string): void {
    this.registeredAttributes.delete(attributeName);
  }

  /**
   * Check if an attribute is registered.
   *
   * @param attributeName - Name of attribute to check
   * @returns True if attribute is registered
   */
  hasAttribute(attributeName: string): boolean {
    return this.registeredAttributes.has(attributeName);
  }

  /**
   * Get a registered attribute.
   *
   * @param attributeName - Name of attribute
   * @returns The attribute or undefined
   */
  getAttribute(attributeName: string): Attribute<V, unknown> | undefined {
    return this.registeredAttributes.get(attributeName)?.attribute;
  }

  /**
   * Get all registered attribute names.
   *
   * @returns Array of registered attribute names
   */
  getRegisteredAttributeNames(): string[] {
    return Array.from(this.registeredAttributes.keys());
  }

  /**
   * Called when a query is executed. Tracks patterns and triggers auto-indexing.
   *
   * @param attribute - The attribute being queried
   * @param queryType - The type of query
   */
  onQueryExecuted(attribute: string, queryType: TrackedQueryType): void {
    if (!this.config.enabled) return;
    if (!this.map) return;

    // Skip if already indexed
    if (this.createdIndexes.has(attribute)) return;
    if (this.map.hasIndexOn(attribute)) {
      this.createdIndexes.add(attribute);
      return;
    }

    // Increment query count for this attribute
    const key = `${attribute}:${queryType}`;
    const count = (this.attributeQueryCounts.get(key) || 0) + 1;
    this.attributeQueryCounts.set(key, count);

    // Check if threshold reached
    if (count === this.config.threshold) {
      this.tryCreateIndex(attribute, queryType);
    }
  }

  /**
   * Check if we're at the index limit.
   *
   * @returns True if max indexes reached
   */
  isAtLimit(): boolean {
    if (!this.map) return false;
    return this.map.getIndexes().length >= this.config.maxIndexes;
  }

  /**
   * Get number of auto-created indexes.
   *
   * @returns Number of indexes created by this manager
   */
  getAutoCreatedIndexCount(): number {
    return this.createdIndexes.size;
  }

  /**
   * Get remaining index capacity.
   *
   * @returns Number of indexes that can still be created
   */
  getRemainingCapacity(): number {
    if (!this.map) return this.config.maxIndexes;
    return Math.max(0, this.config.maxIndexes - this.map.getIndexes().length);
  }

  /**
   * Reset query counts (e.g., after clearing data).
   */
  resetCounts(): void {
    this.attributeQueryCounts.clear();
  }

  /**
   * Get current configuration.
   */
  getConfig(): Readonly<AutoIndexConfig> {
    return this.config;
  }

  /**
   * Update configuration at runtime.
   *
   * @param updates - Partial config updates
   */
  updateConfig(updates: Partial<AutoIndexConfig>): void {
    if (updates.enabled !== undefined) {
      this.config.enabled = updates.enabled;
    }
    if (updates.threshold !== undefined) {
      this.config.threshold = updates.threshold;
    }
    if (updates.maxIndexes !== undefined) {
      this.config.maxIndexes = updates.maxIndexes;
    }
    if (updates.onIndexCreated !== undefined) {
      this.config.onIndexCreated = updates.onIndexCreated;
    }
  }

  /**
   * Try to create an index for the attribute.
   */
  private tryCreateIndex(attribute: string, queryType: TrackedQueryType): void {
    if (!this.map) return;

    // Safety check: don't exceed max indexes
    if (this.isAtLimit()) {
      // Optionally log warning
      return;
    }

    // Check if attribute is registered
    const registered = this.registeredAttributes.get(attribute);
    if (!registered) {
      // Attribute not registered - cannot create index
      return;
    }

    // Determine index type
    const indexType = this.advisor.getRecommendedIndexType(queryType);
    if (!indexType) return;

    // Check if index type is allowed for this attribute
    if (
      registered.allowedIndexTypes &&
      !registered.allowedIndexTypes.includes(indexType)
    ) {
      return;
    }

    // Create the index
    this.createIndex(attribute, indexType, registered.attribute);
  }

  /**
   * Create an index on the map.
   */
  private createIndex(
    attributeName: string,
    indexType: RecommendedIndexType,
    attribute: Attribute<V, unknown>
  ): void {
    if (!this.map) return;

    try {
      switch (indexType) {
        case 'hash':
          this.map.addHashIndex(attribute);
          break;
        case 'navigable':
          this.map.addNavigableIndex(attribute as Attribute<V, string | number>);
          break;
        case 'inverted':
          this.map.addInvertedIndex(attribute as Attribute<V, string>);
          break;
      }

      // Track creation
      this.createdIndexes.add(attributeName);

      // Update tracker - mark as indexed
      this.tracker.updateIndexStatus(attributeName, true);

      // Notify callback
      this.config.onIndexCreated(attributeName, indexType);
    } catch (error) {
      // Index creation failed - log but don't throw
      console.error(`AutoIndexManager: Failed to create ${indexType} index on '${attributeName}':`, error);
    }
  }
}
