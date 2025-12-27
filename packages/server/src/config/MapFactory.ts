/**
 * MapFactory Implementation
 *
 * Factory for creating LWWMap or IndexedLWWMap based on configuration.
 * Used by ServerCoordinator to create maps with proper indexes.
 *
 * @module config/MapFactory
 */

import {
  HLC,
  LWWMap,
  ORMap,
  IndexedLWWMap,
  IndexedORMap,
  simpleAttribute,
  type Attribute,
} from '@topgunbuild/core';
import type {
  ServerIndexConfig,
  MapIndexConfig,
  IndexDefinition,
} from './IndexConfig';
import { mergeWithDefaults } from './IndexConfig';

/**
 * Factory for creating indexed or regular CRDT maps.
 */
export class MapFactory {
  private readonly config: ServerIndexConfig;
  private readonly mapConfigs: Map<string, MapIndexConfig>;

  /**
   * Create a MapFactory.
   *
   * @param config - Server index configuration
   */
  constructor(config?: Partial<ServerIndexConfig>) {
    this.config = mergeWithDefaults(config ?? {});

    // Build lookup map for quick access
    this.mapConfigs = new Map();
    for (const mapConfig of this.config.maps ?? []) {
      this.mapConfigs.set(mapConfig.mapName, mapConfig);
    }
  }

  /**
   * Create an LWWMap or IndexedLWWMap based on configuration.
   *
   * @param mapName - Name of the map
   * @param hlc - Hybrid Logical Clock instance
   * @returns LWWMap or IndexedLWWMap depending on configuration
   */
  createLWWMap<V>(mapName: string, hlc: HLC): LWWMap<string, V> | IndexedLWWMap<string, V> {
    const mapConfig = this.mapConfigs.get(mapName);

    // No indexes configured - return regular LWWMap
    if (!mapConfig || mapConfig.indexes.length === 0) {
      return new LWWMap<string, V>(hlc);
    }

    // Create IndexedLWWMap with configured indexes
    const map = new IndexedLWWMap<string, V>(hlc);

    for (const indexDef of mapConfig.indexes) {
      this.addIndexToLWWMap(map, indexDef);
    }

    return map;
  }

  /**
   * Create an ORMap or IndexedORMap based on configuration.
   *
   * @param mapName - Name of the map
   * @param hlc - Hybrid Logical Clock instance
   * @returns ORMap or IndexedORMap depending on configuration
   */
  createORMap<V>(mapName: string, hlc: HLC): ORMap<string, V> | IndexedORMap<string, V> {
    const mapConfig = this.mapConfigs.get(mapName);

    // No indexes configured - return regular ORMap
    if (!mapConfig || mapConfig.indexes.length === 0) {
      return new ORMap<string, V>(hlc);
    }

    // Create IndexedORMap with configured indexes
    const map = new IndexedORMap<string, V>(hlc);

    for (const indexDef of mapConfig.indexes) {
      this.addIndexToORMap(map, indexDef);
    }

    return map;
  }

  /**
   * Add an index to an IndexedLWWMap based on definition.
   */
  private addIndexToLWWMap<V>(
    map: IndexedLWWMap<string, V>,
    indexDef: IndexDefinition
  ): void {
    const attribute = this.createAttribute<V>(indexDef.attribute);

    if (indexDef.type === 'hash') {
      map.addHashIndex(attribute);
    } else if (indexDef.type === 'navigable') {
      // For navigable indexes, we need to ensure type safety
      const navAttribute = attribute as Attribute<V, string | number>;
      const comparator = this.createComparator(indexDef.comparator);
      map.addNavigableIndex(navAttribute, comparator);
    }
  }

  /**
   * Add an index to an IndexedORMap based on definition.
   */
  private addIndexToORMap<V>(
    map: IndexedORMap<string, V>,
    indexDef: IndexDefinition
  ): void {
    const attribute = this.createAttribute<V>(indexDef.attribute);

    if (indexDef.type === 'hash') {
      map.addHashIndex(attribute);
    } else if (indexDef.type === 'navigable') {
      const navAttribute = attribute as Attribute<V, string | number>;
      const comparator = this.createComparator(indexDef.comparator);
      map.addNavigableIndex(navAttribute, comparator);
    }
  }

  /**
   * Create an Attribute for extracting values from records.
   * Supports dot notation for nested paths.
   */
  private createAttribute<V>(path: string): Attribute<V, unknown> {
    return simpleAttribute(path, (record: V) => {
      return this.getNestedValue(record, path);
    });
  }

  /**
   * Get a nested value from an object using dot notation.
   *
   * @param obj - Object to extract value from
   * @param path - Dot-notation path (e.g., "user.email")
   * @returns Value at the path or undefined
   */
  private getNestedValue(obj: unknown, path: string): unknown {
    if (obj === null || obj === undefined) {
      return undefined;
    }

    const parts = path.split('.');
    let current: unknown = obj;

    for (const part of parts) {
      if (current === undefined || current === null) {
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
   * Create a comparator function for navigable indexes.
   */
  private createComparator(
    type?: 'number' | 'string' | 'date'
  ): ((a: string | number, b: string | number) => number) | undefined {
    switch (type) {
      case 'number':
        return (a, b) => {
          const numA = typeof a === 'number' ? a : parseFloat(String(a));
          const numB = typeof b === 'number' ? b : parseFloat(String(b));
          return numA - numB;
        };

      case 'date':
        return (a, b) => {
          const dateA = new Date(a as string | number).getTime();
          const dateB = new Date(b as string | number).getTime();
          return dateA - dateB;
        };

      case 'string':
        return (a, b) => {
          const strA = String(a);
          const strB = String(b);
          return strA.localeCompare(strB);
        };

      default:
        // Use default comparator (natural ordering)
        return undefined;
    }
  }

  /**
   * Check if a map should be indexed based on configuration.
   *
   * @param mapName - Name of the map
   * @returns true if map has index configuration
   */
  hasIndexConfig(mapName: string): boolean {
    const config = this.mapConfigs.get(mapName);
    return config !== undefined && config.indexes.length > 0;
  }

  /**
   * Get index configuration for a map.
   *
   * @param mapName - Name of the map
   * @returns Map index config or undefined
   */
  getMapConfig(mapName: string): MapIndexConfig | undefined {
    return this.mapConfigs.get(mapName);
  }

  /**
   * Get all configured map names.
   *
   * @returns Array of map names with index configuration
   */
  getConfiguredMaps(): string[] {
    return Array.from(this.mapConfigs.keys());
  }

  /**
   * Get the full server index configuration.
   */
  getConfig(): ServerIndexConfig {
    return this.config;
  }
}
