import type { JournalEvent, JournalEventType } from '@topgunbuild/core';
import type { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

/**
 * Serialized journal event from network (bigint as string).
 */
export interface JournalEventData {
  sequence: string;
  type: JournalEventType;
  mapName: string;
  key: string;
  value?: unknown;
  previousValue?: unknown;
  timestamp: {
    millis: number;
    counter: number;
    nodeId: string;
  };
  nodeId: string;
  metadata?: Record<string, unknown>;
}

/**
 * Options for journal subscription.
 */
export interface JournalSubscribeOptions {
  /** Start from specific sequence */
  fromSequence?: bigint;
  /** Filter by map name */
  mapName?: string;
  /** Filter by event types */
  types?: JournalEventType[];
}

/**
 * Client-side Event Journal Reader.
 * Communicates with server to read and subscribe to journal events.
 */
export class EventJournalReader {
  private readonly syncEngine: SyncEngine;
  private readonly listeners: Map<string, (event: JournalEvent) => void> = new Map();
  private subscriptionCounter: number = 0;

  constructor(syncEngine: SyncEngine) {
    this.syncEngine = syncEngine;
  }

  /**
   * Read events from sequence with optional limit.
   *
   * @param sequence Starting sequence (inclusive)
   * @param limit Maximum events to return (default: 100)
   * @returns Promise resolving to array of events
   */
  async readFrom(sequence: bigint, limit: number = 100): Promise<JournalEvent[]> {
    const requestId = this.generateRequestId();

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Journal read timeout'));
      }, 10000);

      const handleResponse = (message: any) => {
        if (message.type === 'JOURNAL_READ_RESPONSE' && message.requestId === requestId) {
          clearTimeout(timeout);
          this.syncEngine.off('message', handleResponse);

          const events = message.events.map((e: JournalEventData) => this.parseEvent(e));
          resolve(events);
        }
      };

      this.syncEngine.on('message', handleResponse);

      this.syncEngine.send({
        type: 'JOURNAL_READ',
        requestId,
        fromSequence: sequence.toString(),
        limit,
      });
    });
  }

  /**
   * Read events for a specific map.
   *
   * @param mapName Map name to filter
   * @param sequence Starting sequence (default: 0n)
   * @param limit Maximum events to return (default: 100)
   */
  async readMapEvents(
    mapName: string,
    sequence: bigint = 0n,
    limit: number = 100
  ): Promise<JournalEvent[]> {
    const requestId = this.generateRequestId();

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Journal read timeout'));
      }, 10000);

      const handleResponse = (message: any) => {
        if (message.type === 'JOURNAL_READ_RESPONSE' && message.requestId === requestId) {
          clearTimeout(timeout);
          this.syncEngine.off('message', handleResponse);

          const events = message.events.map((e: JournalEventData) => this.parseEvent(e));
          resolve(events);
        }
      };

      this.syncEngine.on('message', handleResponse);

      this.syncEngine.send({
        type: 'JOURNAL_READ',
        requestId,
        fromSequence: sequence.toString(),
        limit,
        mapName,
      });
    });
  }

  /**
   * Subscribe to new journal events.
   *
   * @param listener Callback for each event
   * @param options Subscription options
   * @returns Unsubscribe function
   */
  subscribe(
    listener: (event: JournalEvent) => void,
    options: JournalSubscribeOptions = {}
  ): () => void {
    const subscriptionId = this.generateRequestId();

    this.listeners.set(subscriptionId, listener);

    // Set up message handler for this subscription
    const handleEvent = (message: any) => {
      if (message.type === 'JOURNAL_EVENT') {
        const event = this.parseEvent(message.event);

        // Apply client-side filters
        if (options.mapName && event.mapName !== options.mapName) return;
        if (options.types && !options.types.includes(event.type)) return;

        const listenerFn = this.listeners.get(subscriptionId);
        if (listenerFn) {
          try {
            listenerFn(event);
          } catch (e) {
            logger.error({ err: e }, 'Journal listener error');
          }
        }
      }
    };

    this.syncEngine.on('message', handleEvent);

    // Send subscription request
    this.syncEngine.send({
      type: 'JOURNAL_SUBSCRIBE',
      requestId: subscriptionId,
      fromSequence: options.fromSequence?.toString(),
      mapName: options.mapName,
      types: options.types,
    });

    // Return unsubscribe function
    return () => {
      this.listeners.delete(subscriptionId);
      this.syncEngine.off('message', handleEvent);

      this.syncEngine.send({
        type: 'JOURNAL_UNSUBSCRIBE',
        subscriptionId,
      });
    };
  }

  /**
   * Get the latest sequence number from server.
   */
  async getLatestSequence(): Promise<bigint> {
    // Read one event from the end to get latest sequence
    const events = await this.readFrom(0n, 1);
    if (events.length === 0) return 0n;
    return events[events.length - 1].sequence;
  }

  /**
   * Parse network event data to JournalEvent.
   */
  private parseEvent(raw: JournalEventData): JournalEvent {
    return {
      sequence: BigInt(raw.sequence),
      type: raw.type,
      mapName: raw.mapName,
      key: raw.key,
      value: raw.value,
      previousValue: raw.previousValue,
      timestamp: raw.timestamp,
      nodeId: raw.nodeId,
      metadata: raw.metadata,
    };
  }

  /**
   * Generate unique request ID.
   */
  private generateRequestId(): string {
    return `journal_${Date.now()}_${++this.subscriptionCounter}`;
  }
}

