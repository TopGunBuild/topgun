/**
 * IndexConfig Types
 *
 * Configuration types for server-side index management.
 * Used to define indexes per map at server startup.
 *
 * @module config/IndexConfig
 */

/**
 * Definition of a single index on a map.
 */
export interface IndexDefinition {
  /** Attribute name (supports dot notation for nested attributes, e.g., "user.email") */
  attribute: string;

  /** Index type */
  type: 'hash' | 'navigable';

  /**
   * Comparator type for navigable indexes.
   * Defaults to natural ordering (string/number).
   */
  comparator?: 'number' | 'string' | 'date';
}

/**
 * Index configuration for a specific map.
 */
export interface MapIndexConfig {
  /** Map name */
  mapName: string;

  /** Indexes to create on this map */
  indexes: IndexDefinition[];
}

/**
 * Server-wide index configuration.
 */
export interface ServerIndexConfig {
  /**
   * Auto-create indexes based on query patterns.
   * When enabled, the server will analyze query patterns and suggest/create indexes.
   * Default: false
   */
  autoIndex?: boolean;

  /**
   * Maximum number of auto-created indexes per map.
   * Prevents unbounded memory growth from auto-indexing.
   * Default: 10
   */
  maxAutoIndexesPerMap?: number;

  /**
   * Pre-configured indexes per map.
   * These indexes are created at map initialization.
   */
  maps?: MapIndexConfig[];

  /**
   * Whether to log index usage statistics.
   * Default: false
   */
  logStats?: boolean;

  /**
   * Interval in milliseconds for logging index statistics.
   * Only used if logStats is true.
   * Default: 60000 (1 minute)
   */
  statsLogInterval?: number;
}

/**
 * Default index configuration.
 */
export const DEFAULT_INDEX_CONFIG: ServerIndexConfig = {
  autoIndex: false,
  maxAutoIndexesPerMap: 10,
  maps: [],
  logStats: false,
  statsLogInterval: 60000,
};

/**
 * Validate a ServerIndexConfig object.
 *
 * @param config - Config to validate
 * @returns Array of validation errors (empty if valid)
 */
export function validateIndexConfig(config: ServerIndexConfig): string[] {
  const errors: string[] = [];

  if (config.maxAutoIndexesPerMap !== undefined) {
    if (
      typeof config.maxAutoIndexesPerMap !== 'number' ||
      config.maxAutoIndexesPerMap < 1
    ) {
      errors.push('maxAutoIndexesPerMap must be a positive number');
    }
  }

  if (config.statsLogInterval !== undefined) {
    if (
      typeof config.statsLogInterval !== 'number' ||
      config.statsLogInterval < 1000
    ) {
      errors.push('statsLogInterval must be at least 1000ms');
    }
  }

  if (config.maps) {
    const mapNames = new Set<string>();

    for (const mapConfig of config.maps) {
      if (!mapConfig.mapName || typeof mapConfig.mapName !== 'string') {
        errors.push('Each map config must have a valid mapName');
        continue;
      }

      if (mapNames.has(mapConfig.mapName)) {
        errors.push(`Duplicate map config for: ${mapConfig.mapName}`);
      }
      mapNames.add(mapConfig.mapName);

      if (!Array.isArray(mapConfig.indexes)) {
        errors.push(`Map ${mapConfig.mapName}: indexes must be an array`);
        continue;
      }

      const attrNames = new Set<string>();
      for (const indexDef of mapConfig.indexes) {
        if (!indexDef.attribute || typeof indexDef.attribute !== 'string') {
          errors.push(`Map ${mapConfig.mapName}: index must have valid attribute`);
          continue;
        }

        if (!['hash', 'navigable'].includes(indexDef.type)) {
          errors.push(
            `Map ${mapConfig.mapName}: index type must be 'hash' or 'navigable'`
          );
        }

        if (
          indexDef.comparator &&
          !['number', 'string', 'date'].includes(indexDef.comparator)
        ) {
          errors.push(
            `Map ${mapConfig.mapName}: comparator must be 'number', 'string', or 'date'`
          );
        }

        // Warn about duplicate attribute indexes (same type)
        const key = `${indexDef.attribute}:${indexDef.type}`;
        if (attrNames.has(key)) {
          errors.push(
            `Map ${mapConfig.mapName}: duplicate ${indexDef.type} index on ${indexDef.attribute}`
          );
        }
        attrNames.add(key);
      }
    }
  }

  return errors;
}

/**
 * Merge user config with defaults.
 *
 * @param userConfig - User-provided config
 * @returns Merged config with defaults
 */
export function mergeWithDefaults(
  userConfig: Partial<ServerIndexConfig>
): ServerIndexConfig {
  return {
    ...DEFAULT_INDEX_CONFIG,
    ...userConfig,
    maps: userConfig.maps ?? DEFAULT_INDEX_CONFIG.maps,
  };
}
