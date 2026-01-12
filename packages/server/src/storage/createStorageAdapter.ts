import { IServerStorage } from './IServerStorage';
import { PostgresAdapter } from './PostgresAdapter';
import { MemoryServerAdapter } from './MemoryServerAdapter';

export type StorageMode = 'sqlite' | 'postgres' | 'memory';

export interface StorageConfig {
  mode: StorageMode;
  // SQLite options
  sqlitePath?: string;
  sqliteVerbose?: boolean;
  // PostgreSQL options
  postgresHost?: string;
  postgresPort?: number;
  postgresUser?: string;
  postgresPassword?: string;
  postgresDatabase?: string;
  postgresConnectionString?: string;
  // Table name (for both SQLite and PostgreSQL)
  tableName?: string;
}

/**
 * Creates a storage adapter based on configuration or environment variables.
 *
 * Priority:
 * 1. Explicit config object
 * 2. Environment variables
 * 3. Defaults (SQLite for development)
 *
 * Environment variables:
 * - STORAGE_MODE: 'sqlite' | 'postgres' | 'memory'
 * - DB_PATH: SQLite file path (default: './topgun.db')
 * - DATABASE_URL: PostgreSQL connection string
 * - DB_HOST, DB_PORT, DB_USER, DB_PASSWORD, DB_NAME: PostgreSQL config
 */
export async function createStorageAdapter(config?: StorageConfig): Promise<IServerStorage> {
  const mode = config?.mode || (process.env.STORAGE_MODE as StorageMode) || 'memory';

  switch (mode) {
    case 'sqlite': {
      // Dynamic import to make better-sqlite3 optional
      const { BetterSqlite3Adapter } = await import('./BetterSqlite3Adapter');

      const sqlitePath = config?.sqlitePath || process.env.DB_PATH || './topgun.db';
      console.log(`[Storage] Using SQLite: ${sqlitePath}`);

      const adapter = new BetterSqlite3Adapter({
        filename: sqlitePath,
        verbose: config?.sqliteVerbose || process.env.SQLITE_VERBOSE === 'true',
        tableName: config?.tableName,
      });

      await adapter.initialize();
      return adapter;
    }

    case 'postgres': {
      const connectionString = config?.postgresConnectionString || process.env.DATABASE_URL;

      let pgConfig: any;

      if (connectionString) {
        pgConfig = { connectionString };
      } else {
        pgConfig = {
          host: config?.postgresHost || process.env.DB_HOST || 'localhost',
          port: config?.postgresPort || parseInt(process.env.DB_PORT || '5432', 10),
          user: config?.postgresUser || process.env.DB_USER || 'topgun',
          password: config?.postgresPassword || process.env.DB_PASSWORD,
          database: config?.postgresDatabase || process.env.DB_NAME || 'topgun',
        };
      }

      console.log(
        `[Storage] Using PostgreSQL: ${pgConfig.host || 'from connection string'}:${pgConfig.port || ''}/${pgConfig.database || ''}`
      );

      const adapter = new PostgresAdapter(pgConfig, { tableName: config?.tableName });
      await adapter.initialize();
      return adapter;
    }

    case 'memory': {
      console.log('[Storage] Using in-memory storage (data will be lost on restart)');
      const adapter = new MemoryServerAdapter();
      await adapter.initialize();
      return adapter;
    }

    default:
      throw new Error(`Unknown storage mode: ${mode}. Use: sqlite, postgres, or memory`);
  }
}

/**
 * Creates a storage adapter synchronously (for backwards compatibility).
 * Only supports memory adapter synchronously.
 * For sqlite/postgres, use createStorageAdapter() instead.
 */
export function createStorageAdapterSync(config?: StorageConfig): IServerStorage | null {
  const mode = config?.mode || (process.env.STORAGE_MODE as StorageMode) || 'memory';

  if (mode === 'memory') {
    console.log('[Storage] Using in-memory storage (data will be lost on restart)');
    return new MemoryServerAdapter();
  }

  // Cannot create sqlite/postgres synchronously
  console.warn(
    `[Storage] Cannot create ${mode} adapter synchronously. Use createStorageAdapter() instead.`
  );
  return null;
}
