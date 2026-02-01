/**
 * Server Configuration exports
 */

export {
  type IndexDefinition,
  type MapIndexConfig,
  type ServerIndexConfig,
  DEFAULT_INDEX_CONFIG,
  validateIndexConfig,
  mergeWithDefaults,
} from './IndexConfig';

export { MapFactory } from './MapFactory';

export { validateEnv, type EnvConfig } from './env-schema';
