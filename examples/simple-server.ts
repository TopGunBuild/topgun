import { ServerFactory, PostgresAdapter, MemoryServerAdapter, BetterSqlite3Adapter, IServerStorage } from '@topgunbuild/server';
import { PoolConfig } from 'pg';
import * as path from 'path';

// Storage mode: 'memory' (default), 'sqlite', or 'postgres'
const STORAGE_MODE = process.env.STORAGE_MODE || 'memory';
const DB_PATH = process.env.DB_PATH || './data/topgun.db';

// Configuration priority: DATABASE_URL > Env Vars > Defaults
const getDbConfig = (): PoolConfig => {
  if (process.env.DATABASE_URL) {
    console.log('Using DATABASE_URL for connection');
    return {
      connectionString: process.env.DATABASE_URL,
      ssl: {
        rejectUnauthorized: false // Required for many cloud providers like Neon
      }
    };
  }

  return {
    host: process.env.DB_HOST || 'localhost',
    port: parseInt(process.env.DB_PORT || '5432'),
    user: process.env.DB_USER || 'postgres',
    password: process.env.DB_PASSWORD || 'postgres',
    database: process.env.DB_NAME || 'topgun',
  };
};

// Create storage adapter based on STORAGE_MODE
const createStorageAdapter = (): IServerStorage => {
  if (STORAGE_MODE === 'postgres') {
    const dbConfig = getDbConfig();
    console.log('Storage mode: PostgreSQL');
    if (process.env.DATABASE_URL) {
      console.log('Connected to Cloud Database');
    } else {
      console.log(`Connecting to Postgres at ${dbConfig.host}:${dbConfig.port}`);
    }
    return new PostgresAdapter(dbConfig);
  }

  if (STORAGE_MODE === 'sqlite') {
    // Ensure directory exists
    const dbDir = path.dirname(DB_PATH);
    const fs = require('fs');
    if (!fs.existsSync(dbDir)) {
      fs.mkdirSync(dbDir, { recursive: true });
    }
    console.log(`Storage mode: SQLite (${DB_PATH})`);
    return new BetterSqlite3Adapter(DB_PATH);
  }

  console.log('Storage mode: In-Memory (data will be lost on restart)');
  return new MemoryServerAdapter();
};

const adapter = createStorageAdapter();

const server = ServerFactory.create({
  port: parseInt(process.env.PORT || '8080'),
  clusterPort: parseInt(process.env.CLUSTER_PORT || '9080'),
  metricsPort: parseInt(process.env.METRICS_PORT || '9091'),
  nodeId: process.env.NODE_ID || 'server-node-1',
  storage: adapter,
  securityPolicies: [
    {
      role: 'USER',
      mapNamePattern: 'notes:{userId}',
      actions: ['ALL']
    },
    {
      role: 'USER',
      mapNamePattern: 'folders:{userId}',
      actions: ['ALL']
    }
  ]
});

console.log('TopGun Server running on ws://localhost:8080');

// Graceful Shutdown Logic
const shutdown = async (signal: string) => {
  console.log(`Received ${signal}. Starting Graceful Shutdown...`);

  // Force Kill Timeout
  const forceKillTimer = setTimeout(() => {
    console.error('Shutdown timed out. Forcing exit.');
    process.exit(1);
  }, 10000); // 10 seconds

  try {
    await server.shutdown();
    console.log('Graceful shutdown successful.');
    clearTimeout(forceKillTimer);
    process.exit(0);
  } catch (err) {
    console.error('Error during shutdown:', err);
    clearTimeout(forceKillTimer);
    process.exit(1);
  }
};

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
