import { Pool } from 'pg';
import {
  EventJournalImpl,
  JournalEvent,
  JournalEventInput,
  EventJournalConfig,
  DEFAULT_EVENT_JOURNAL_CONFIG,
} from '@topgunbuild/core';
import { logger } from './utils/logger';

/**
 * Export options for streaming journal events.
 */
export interface ExportOptions {
  /** Start from this sequence (inclusive) */
  fromSequence?: bigint;
  /** End at this sequence (inclusive) */
  toSequence?: bigint;
  /** Filter by map name */
  mapName?: string;
  /** Filter by event types */
  types?: ('PUT' | 'UPDATE' | 'DELETE')[];
}

/**
 * Configuration for EventJournalService.
 */
export interface EventJournalServiceConfig extends EventJournalConfig {
  /** PostgreSQL connection pool */
  pool: Pool;
  /** Table name for journal storage */
  tableName?: string;
  /** Batch size for persistence */
  persistBatchSize?: number;
  /** Interval for periodic persistence (ms) */
  persistIntervalMs?: number;
}

/**
 * Default configuration for EventJournalService.
 */
export const DEFAULT_JOURNAL_SERVICE_CONFIG: Omit<EventJournalServiceConfig, 'pool'> = {
  ...DEFAULT_EVENT_JOURNAL_CONFIG,
  tableName: 'event_journal',
  persistBatchSize: 100,
  persistIntervalMs: 1000,
};

const TABLE_NAME_REGEX = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

function validateTableName(name: string): void {
  if (!TABLE_NAME_REGEX.test(name)) {
    throw new Error(
      `Invalid table name "${name}". Table name must start with a letter or underscore and contain only alphanumeric characters and underscores.`
    );
  }
}

/**
 * Server-side Event Journal Service with PostgreSQL persistence.
 * Extends EventJournalImpl to add durable storage.
 */
export class EventJournalService extends EventJournalImpl {
  private readonly pool: Pool;
  private readonly tableName: string;
  private readonly persistBatchSize: number;
  private readonly persistIntervalMs: number;
  private pendingPersist: JournalEvent[] = [];
  private persistTimer?: ReturnType<typeof setInterval>;
  private isPersisting: boolean = false;
  private isInitialized: boolean = false;
  private isLoadingFromStorage: boolean = false;

  constructor(config: EventJournalServiceConfig) {
    super(config);
    this.pool = config.pool;
    this.tableName = config.tableName ?? DEFAULT_JOURNAL_SERVICE_CONFIG.tableName!;
    this.persistBatchSize = config.persistBatchSize ?? DEFAULT_JOURNAL_SERVICE_CONFIG.persistBatchSize!;
    this.persistIntervalMs = config.persistIntervalMs ?? DEFAULT_JOURNAL_SERVICE_CONFIG.persistIntervalMs!;

    validateTableName(this.tableName);

    // Subscribe to events for persistence
    this.subscribe((event) => {
      // Skip persistence for events being loaded from storage
      if (this.isLoadingFromStorage) return;

      if (event.sequence >= 0n && this.getConfig().persistent) {
        this.pendingPersist.push(event);

        if (this.pendingPersist.length >= this.persistBatchSize) {
          this.persistToStorage().catch((err) => {
            logger.error({ err }, 'Failed to persist journal events');
          });
        }
      }
    });

    // Start periodic persistence
    this.startPersistTimer();
  }

  /**
   * Initialize the journal service, creating table if needed.
   */
  async initialize(): Promise<void> {
    if (this.isInitialized) return;

    const client = await this.pool.connect();
    try {
      await client.query(`
        CREATE TABLE IF NOT EXISTS ${this.tableName} (
          sequence BIGINT PRIMARY KEY,
          type VARCHAR(10) NOT NULL CHECK (type IN ('PUT', 'UPDATE', 'DELETE')),
          map_name VARCHAR(255) NOT NULL,
          key VARCHAR(1024) NOT NULL,
          value JSONB,
          previous_value JSONB,
          timestamp JSONB NOT NULL,
          node_id VARCHAR(64) NOT NULL,
          metadata JSONB,
          created_at TIMESTAMPTZ DEFAULT NOW()
        );
      `);

      // Create indexes for common queries
      await client.query(`
        CREATE INDEX IF NOT EXISTS idx_${this.tableName}_map_name
        ON ${this.tableName}(map_name);
      `);
      await client.query(`
        CREATE INDEX IF NOT EXISTS idx_${this.tableName}_key
        ON ${this.tableName}(map_name, key);
      `);
      await client.query(`
        CREATE INDEX IF NOT EXISTS idx_${this.tableName}_created_at
        ON ${this.tableName}(created_at);
      `);
      await client.query(`
        CREATE INDEX IF NOT EXISTS idx_${this.tableName}_node_id
        ON ${this.tableName}(node_id);
      `);

      this.isInitialized = true;
      logger.info({ tableName: this.tableName }, 'EventJournalService initialized');
    } finally {
      client.release();
    }
  }

  /**
   * Persist pending events to PostgreSQL.
   */
  async persistToStorage(): Promise<void> {
    if (this.pendingPersist.length === 0 || this.isPersisting) return;

    this.isPersisting = true;
    const batch = this.pendingPersist.splice(0, this.persistBatchSize);

    try {
      if (batch.length === 0) return;

      // Build parameterized query for batch insert
      const values: any[] = [];
      const placeholders: string[] = [];

      batch.forEach((e, i) => {
        const offset = i * 9;
        placeholders.push(
          `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}, $${offset + 6}, $${offset + 7}, $${offset + 8}, $${offset + 9})`
        );
        values.push(
          e.sequence.toString(),
          e.type,
          e.mapName,
          e.key,
          e.value !== undefined ? JSON.stringify(e.value) : null,
          e.previousValue !== undefined ? JSON.stringify(e.previousValue) : null,
          JSON.stringify(e.timestamp),
          e.nodeId,
          e.metadata ? JSON.stringify(e.metadata) : null
        );
      });

      await this.pool.query(
        `INSERT INTO ${this.tableName}
         (sequence, type, map_name, key, value, previous_value, timestamp, node_id, metadata)
         VALUES ${placeholders.join(', ')}
         ON CONFLICT (sequence) DO NOTHING`,
        values
      );

      logger.debug({ count: batch.length }, 'Persisted journal events');
    } catch (error) {
      // Re-queue failed events
      this.pendingPersist.unshift(...batch);
      throw error;
    } finally {
      this.isPersisting = false;
    }
  }

  /**
   * Load journal events from PostgreSQL on startup.
   */
  async loadFromStorage(): Promise<void> {
    const config = this.getConfig();
    const result = await this.pool.query(
      `SELECT sequence, type, map_name, key, value, previous_value, timestamp, node_id, metadata
       FROM ${this.tableName}
       ORDER BY sequence DESC
       LIMIT $1`,
      [config.capacity]
    );

    // Load in reverse order (oldest first)
    const events = result.rows.reverse();

    // Set flag to prevent re-persisting loaded events
    this.isLoadingFromStorage = true;
    try {
      for (const row of events) {
        this.append({
          type: row.type,
          mapName: row.map_name,
          key: row.key,
          value: row.value,
          previousValue: row.previous_value,
          timestamp: typeof row.timestamp === 'string' ? JSON.parse(row.timestamp) : row.timestamp,
          nodeId: row.node_id,
          metadata: row.metadata,
        });
      }
    } finally {
      this.isLoadingFromStorage = false;
    }

    logger.info({ count: events.length }, 'Loaded journal events from storage');
  }

  /**
   * Export events as NDJSON stream.
   */
  exportStream(options: ExportOptions = {}): ReadableStream<string> {
    const self = this;

    return new ReadableStream({
      start(controller) {
        const startSeq = options.fromSequence ?? self.getOldestSequence();
        const endSeq = options.toSequence ?? self.getLatestSequence();

        for (let seq = startSeq; seq <= endSeq; seq++) {
          const events = self.readFrom(seq, 1);
          if (events.length > 0) {
            const event = events[0];

            // Apply filters
            if (options.mapName && event.mapName !== options.mapName) continue;
            if (options.types && !options.types.includes(event.type)) continue;

            // Convert bigint to string for JSON serialization
            const serializable = {
              ...event,
              sequence: event.sequence.toString(),
            };
            controller.enqueue(JSON.stringify(serializable) + '\n');
          }
        }

        controller.close();
      },
    });
  }

  /**
   * Get events for a specific map.
   */
  getMapEvents(mapName: string, fromSeq?: bigint): JournalEvent[] {
    const events = this.readFrom(fromSeq ?? this.getOldestSequence(), this.getConfig().capacity);
    return events.filter((e) => e.mapName === mapName);
  }

  /**
   * Query events from PostgreSQL with filters.
   */
  async queryFromStorage(options: {
    mapName?: string;
    key?: string;
    types?: ('PUT' | 'UPDATE' | 'DELETE')[];
    fromSequence?: bigint;
    toSequence?: bigint;
    fromDate?: Date;
    toDate?: Date;
    limit?: number;
    offset?: number;
  } = {}): Promise<JournalEvent[]> {
    const conditions: string[] = [];
    const params: any[] = [];
    let paramIndex = 1;

    if (options.mapName) {
      conditions.push(`map_name = $${paramIndex++}`);
      params.push(options.mapName);
    }
    if (options.key) {
      conditions.push(`key = $${paramIndex++}`);
      params.push(options.key);
    }
    if (options.types && options.types.length > 0) {
      conditions.push(`type = ANY($${paramIndex++})`);
      params.push(options.types);
    }
    if (options.fromSequence !== undefined) {
      conditions.push(`sequence >= $${paramIndex++}`);
      params.push(options.fromSequence.toString());
    }
    if (options.toSequence !== undefined) {
      conditions.push(`sequence <= $${paramIndex++}`);
      params.push(options.toSequence.toString());
    }
    if (options.fromDate) {
      conditions.push(`created_at >= $${paramIndex++}`);
      params.push(options.fromDate);
    }
    if (options.toDate) {
      conditions.push(`created_at <= $${paramIndex++}`);
      params.push(options.toDate);
    }

    const whereClause = conditions.length > 0 ? `WHERE ${conditions.join(' AND ')}` : '';
    const limit = options.limit ?? 100;
    const offset = options.offset ?? 0;

    const result = await this.pool.query(
      `SELECT sequence, type, map_name, key, value, previous_value, timestamp, node_id, metadata
       FROM ${this.tableName}
       ${whereClause}
       ORDER BY sequence ASC
       LIMIT $${paramIndex++} OFFSET $${paramIndex++}`,
      [...params, limit, offset]
    );

    return result.rows.map((row) => ({
      sequence: BigInt(row.sequence),
      type: row.type,
      mapName: row.map_name,
      key: row.key,
      value: row.value,
      previousValue: row.previous_value,
      timestamp: typeof row.timestamp === 'string' ? JSON.parse(row.timestamp) : row.timestamp,
      nodeId: row.node_id,
      metadata: row.metadata,
    }));
  }

  /**
   * Count events matching filters.
   */
  async countFromStorage(options: {
    mapName?: string;
    types?: ('PUT' | 'UPDATE' | 'DELETE')[];
    fromDate?: Date;
    toDate?: Date;
  } = {}): Promise<number> {
    const conditions: string[] = [];
    const params: any[] = [];
    let paramIndex = 1;

    if (options.mapName) {
      conditions.push(`map_name = $${paramIndex++}`);
      params.push(options.mapName);
    }
    if (options.types && options.types.length > 0) {
      conditions.push(`type = ANY($${paramIndex++})`);
      params.push(options.types);
    }
    if (options.fromDate) {
      conditions.push(`created_at >= $${paramIndex++}`);
      params.push(options.fromDate);
    }
    if (options.toDate) {
      conditions.push(`created_at <= $${paramIndex++}`);
      params.push(options.toDate);
    }

    const whereClause = conditions.length > 0 ? `WHERE ${conditions.join(' AND ')}` : '';

    const result = await this.pool.query(
      `SELECT COUNT(*) as count FROM ${this.tableName} ${whereClause}`,
      params
    );

    return parseInt(result.rows[0].count, 10);
  }

  /**
   * Cleanup old events based on retention policy.
   */
  async cleanupOldEvents(retentionDays: number): Promise<number> {
    const result = await this.pool.query(
      `DELETE FROM ${this.tableName}
       WHERE created_at < NOW() - ($1 || ' days')::INTERVAL
       RETURNING sequence`,
      [retentionDays]
    );

    const count = result.rowCount ?? 0;
    if (count > 0) {
      logger.info({ deletedCount: count, retentionDays }, 'Cleaned up old journal events');
    }

    return count;
  }

  /**
   * Start the periodic persistence timer.
   */
  private startPersistTimer(): void {
    this.persistTimer = setInterval(() => {
      if (this.pendingPersist.length > 0) {
        this.persistToStorage().catch((err) => {
          logger.error({ err }, 'Periodic persist failed');
        });
      }
    }, this.persistIntervalMs);
  }

  /**
   * Stop the periodic persistence timer.
   */
  private stopPersistTimer(): void {
    if (this.persistTimer) {
      clearInterval(this.persistTimer);
      this.persistTimer = undefined;
    }
  }

  /**
   * Dispose resources and persist remaining events.
   */
  override dispose(): void {
    this.stopPersistTimer();

    // Final persist
    if (this.pendingPersist.length > 0) {
      this.persistToStorage().catch((err) => {
        logger.error({ err }, 'Final persist failed on dispose');
      });
    }

    super.dispose();
  }

  /**
   * Get pending persist count (for monitoring).
   */
  getPendingPersistCount(): number {
    return this.pendingPersist.length;
  }
}
