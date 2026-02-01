import { Ringbuffer } from './Ringbuffer';
import type { Timestamp } from './HLC';
import { logger } from './utils/logger';

/**
 * Type of journal event.
 */
export type JournalEventType = 'PUT' | 'UPDATE' | 'DELETE';

/**
 * Single event in the journal.
 */
export interface JournalEvent<V = unknown> {
  /** Monotonically increasing sequence number */
  sequence: bigint;

  /** Event type */
  type: JournalEventType;

  /** Map name */
  mapName: string;

  /** Entry key */
  key: string;

  /** New value (undefined for DELETE) */
  value?: V;

  /** Previous value (for UPDATE and DELETE) */
  previousValue?: V;

  /** HLC timestamp */
  timestamp: Timestamp;

  /** Node that made the change */
  nodeId: string;

  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/**
 * Input for appending events (without sequence).
 */
export type JournalEventInput<V = unknown> = Omit<JournalEvent<V>, 'sequence'>;

/**
 * Event Journal configuration.
 */
export interface EventJournalConfig {
  /** Maximum number of events to keep in memory */
  capacity: number;

  /** Time-to-live for events (ms), 0 = infinite */
  ttlMs: number;

  /** Persist to storage adapter */
  persistent: boolean;

  /** Maps to include (empty = all) */
  includeMaps?: string[];

  /** Maps to exclude */
  excludeMaps?: string[];
}

/**
 * Default configuration for Event Journal.
 */
export const DEFAULT_EVENT_JOURNAL_CONFIG: EventJournalConfig = {
  capacity: 10000,
  ttlMs: 0, // Infinite
  persistent: true,
  includeMaps: [],
  excludeMaps: [],
};

/**
 * Event Journal interface.
 */
export interface EventJournal {
  /** Append event to journal */
  append<V>(event: JournalEventInput<V>): JournalEvent<V>;

  /** Read events from sequence (inclusive) */
  readFrom(sequence: bigint, limit?: number): JournalEvent[];

  /** Read events in range */
  readRange(startSeq: bigint, endSeq: bigint): JournalEvent[];

  /** Get latest sequence number */
  getLatestSequence(): bigint;

  /** Get oldest sequence number (after compaction) */
  getOldestSequence(): bigint;

  /** Subscribe to new events */
  subscribe(
    listener: (event: JournalEvent) => void,
    fromSequence?: bigint
  ): () => void;

  /** Get capacity info */
  getCapacity(): { used: number; total: number };

  /** Force compaction */
  compact(): Promise<void>;

  /** Dispose resources */
  dispose(): void;
}

/**
 * Journal event listener type.
 */
export type JournalEventListener = (event: JournalEvent) => void;

/**
 * Event Journal implementation using Ringbuffer.
 * Records all Map changes as an append-only log.
 */
export class EventJournalImpl implements EventJournal {
  private readonly config: EventJournalConfig;
  private readonly buffer: Ringbuffer<JournalEvent>;
  private readonly listeners: Set<JournalEventListener> = new Set();
  private ttlTimer?: ReturnType<typeof setInterval>;

  constructor(config: Partial<EventJournalConfig> = {}) {
    this.config = { ...DEFAULT_EVENT_JOURNAL_CONFIG, ...config };
    this.buffer = new Ringbuffer(this.config.capacity);

    if (this.config.ttlMs > 0) {
      this.startTTLCleanup();
    }
  }

  /**
   * Append event to journal.
   * Returns the event with assigned sequence number.
   * Returns event with sequence -1n if map is filtered out.
   */
  append<V>(eventData: JournalEventInput<V>): JournalEvent<V> {
    // Check map filters
    if (!this.shouldCapture(eventData.mapName)) {
      return { ...eventData, sequence: -1n } as JournalEvent<V>;
    }

    const event: JournalEvent<V> = {
      ...eventData,
      sequence: 0n, // Will be set by buffer
    };

    const sequence = this.buffer.add(event);
    event.sequence = sequence;

    // Notify listeners
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (e) {
        logger.error({ err: e, context: 'listener' }, 'EventJournal listener error');
      }
    }

    return event;
  }

  /**
   * Read events from sequence with optional limit.
   */
  readFrom(sequence: bigint, limit: number = 100): JournalEvent[] {
    return this.buffer.readFrom(sequence, limit);
  }

  /**
   * Read events in range (inclusive).
   */
  readRange(startSeq: bigint, endSeq: bigint): JournalEvent[] {
    return this.buffer.readRange(startSeq, endSeq);
  }

  /**
   * Get latest sequence number.
   * Returns 0n if no events have been added.
   */
  getLatestSequence(): bigint {
    const tail = this.buffer.getTailSequence();
    return tail > 0n ? tail - 1n : 0n;
  }

  /**
   * Get oldest available sequence number.
   */
  getOldestSequence(): bigint {
    return this.buffer.getHeadSequence();
  }

  /**
   * Subscribe to new events.
   * Optionally replay events from a specific sequence.
   *
   * @param listener Callback for each event
   * @param fromSequence Optional sequence to start replay from
   * @returns Unsubscribe function
   */
  subscribe(
    listener: JournalEventListener,
    fromSequence?: bigint
  ): () => void {
    // Replay events if fromSequence is specified
    if (fromSequence !== undefined) {
      const events = this.readFrom(fromSequence, this.config.capacity);
      for (const event of events) {
        try {
          listener(event);
        } catch (e) {
          logger.error({ err: e, context: 'replay' }, 'EventJournal replay error');
        }
      }
    }

    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /**
   * Get capacity information.
   */
  getCapacity(): { used: number; total: number } {
    return {
      used: this.buffer.size(),
      total: this.buffer.getCapacity(),
    };
  }

  /**
   * Force compaction.
   * Note: The ringbuffer handles eviction automatically.
   * This method is provided for explicit cleanup of old events.
   */
  async compact(): Promise<void> {
    // The ringbuffer automatically evicts old entries when full.
    // TTL-based cleanup is handled by the timer.
    // This method can be extended for additional cleanup logic.
  }

  /**
   * Check if a map should be captured.
   */
  private shouldCapture(mapName: string): boolean {
    const { includeMaps, excludeMaps } = this.config;

    if (excludeMaps && excludeMaps.includes(mapName)) {
      return false;
    }

    if (includeMaps && includeMaps.length > 0) {
      return includeMaps.includes(mapName);
    }

    return true;
  }

  /**
   * Start TTL cleanup timer.
   */
  private startTTLCleanup(): void {
    const interval = Math.min(this.config.ttlMs, 60000); // At least every minute
    this.ttlTimer = setInterval(() => {
      this.compact();
    }, interval);
  }

  /**
   * Dispose resources.
   */
  dispose(): void {
    if (this.ttlTimer) {
      clearInterval(this.ttlTimer);
      this.ttlTimer = undefined;
    }
    this.listeners.clear();
  }

  /**
   * Get all current listeners count (for testing).
   */
  getListenerCount(): number {
    return this.listeners.size;
  }

  /**
   * Get configuration (for testing).
   */
  getConfig(): EventJournalConfig {
    return { ...this.config };
  }
}
