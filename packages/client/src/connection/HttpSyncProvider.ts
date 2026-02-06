import type { Timestamp } from '@topgunbuild/core';
import { HLC, serialize, deserialize } from '@topgunbuild/core';
import type {
  IConnectionProvider,
  ConnectionProviderEvent,
  ConnectionEventHandler,
} from '../types';
import { logger } from '../utils/logger';

/**
 * Configuration for HttpSyncProvider.
 */
export interface HttpSyncProviderConfig {
  /** HTTP URL of the TopGun server (e.g., 'http://localhost:8080') */
  url: string;
  /** Client identifier for the server to track HLC state */
  clientId: string;
  /** Hybrid Logical Clock instance for causality tracking */
  hlc: HLC;
  /** JWT auth token for Authorization header */
  authToken?: string;
  /** Polling interval in milliseconds (default: 5000) */
  pollIntervalMs?: number;
  /** HTTP request timeout in milliseconds (default: 10000) */
  requestTimeoutMs?: number;
  /** Map names to sync deltas for on each poll */
  syncMaps?: string[];
  /** Custom fetch implementation for testing or platform compatibility */
  fetchImpl?: typeof fetch;
}

/**
 * HTTP-based connection provider for serverless environments.
 *
 * Implements IConnectionProvider by translating WebSocket-style send() calls
 * into queued operations that are flushed via HTTP POST /sync at regular
 * polling intervals. Responses are translated back into synthetic message
 * events that SyncEngine understands.
 *
 * This provider is completely stateless on the server side -- each request
 * carries the client's HLC state and the server computes deltas without
 * maintaining per-client state.
 */
export class HttpSyncProvider implements IConnectionProvider {
  private readonly url: string;
  private readonly clientId: string;
  private readonly hlc: HLC;
  private readonly pollIntervalMs: number;
  private readonly requestTimeoutMs: number;
  private readonly syncMaps: string[];
  private readonly fetchImpl: typeof fetch;

  private authToken: string;
  private listeners: Map<ConnectionProviderEvent, Set<ConnectionEventHandler>> = new Map();
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  /** Queued operations to send on next poll */
  private pendingOperations: any[] = [];
  /** Queued one-shot queries to send on next poll */
  private pendingQueries: any[] = [];

  /** Per-map last sync timestamps for delta tracking */
  private lastSyncTimestamps: Map<string, Timestamp> = new Map();

  /** Whether the last HTTP request succeeded */
  private connected: boolean = false;
  /** Whether we were previously connected (for reconnected event) */
  private wasConnected: boolean = false;

  constructor(config: HttpSyncProviderConfig) {
    this.url = config.url.replace(/\/$/, ''); // Strip trailing slash
    this.clientId = config.clientId;
    this.hlc = config.hlc;
    this.authToken = config.authToken || '';
    this.pollIntervalMs = config.pollIntervalMs ?? 5000;
    this.requestTimeoutMs = config.requestTimeoutMs ?? 10000;
    this.syncMaps = config.syncMaps || [];
    this.fetchImpl = config.fetchImpl || globalThis.fetch.bind(globalThis);
  }

  /**
   * Connect by sending an initial sync request to verify auth and get state.
   */
  async connect(): Promise<void> {
    try {
      await this.doSyncRequest();
      this.connected = true;
      this.wasConnected = true;
      this.emit('connected', 'http');
      this.startPolling();
    } catch (err: any) {
      this.connected = false;
      this.emit('error', err);
      throw err;
    }
  }

  /**
   * Get connection for a specific key.
   * HTTP mode does not expose raw WebSocket connections.
   */
  getConnection(_key: string): WebSocket {
    throw new Error('HTTP mode does not support direct WebSocket access');
  }

  /**
   * Get any available connection.
   * HTTP mode does not expose raw WebSocket connections.
   */
  getAnyConnection(): WebSocket {
    throw new Error('HTTP mode does not support direct WebSocket access');
  }

  /**
   * Check if connected (last HTTP request succeeded).
   */
  isConnected(): boolean {
    return this.connected;
  }

  /**
   * Get connected node IDs.
   * Returns ['http'] when connected, [] when not.
   */
  getConnectedNodes(): string[] {
    return this.connected ? ['http'] : [];
  }

  /**
   * Subscribe to connection events.
   */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(handler);
  }

  /**
   * Unsubscribe from connection events.
   */
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.listeners.get(event)?.delete(handler);
  }

  /**
   * Send data via the HTTP sync provider.
   *
   * Deserializes the msgpackr binary to extract the message type and routes:
   * - OP_BATCH / CLIENT_OP: queued as operations for next poll
   * - AUTH: silently ignored (auth via HTTP header)
   * - SYNC_INIT: silently ignored (HTTP uses timestamp-based deltas)
   * - QUERY_SUB: queued as one-shot query for next poll
   * - All other types: silently dropped with debug log
   */
  send(data: ArrayBuffer | Uint8Array, _key?: string): void {
    try {
      const message = deserialize<any>(
        data instanceof ArrayBuffer ? new Uint8Array(data) : data,
      );

      switch (message.type) {
        case 'OP_BATCH':
          if (message.payload?.ops) {
            this.pendingOperations.push(...message.payload.ops);
          }
          break;

        case 'CLIENT_OP':
          if (message.payload) {
            this.pendingOperations.push(message.payload);
          }
          break;

        case 'AUTH':
          // Auth handled via HTTP Authorization header, ignore
          break;

        case 'SYNC_INIT':
          // HTTP uses timestamp-based deltas, not Merkle sync, ignore
          break;

        case 'QUERY_SUB':
          if (message.payload) {
            this.pendingQueries.push({
              queryId: message.payload.requestId || `q-${Date.now()}`,
              mapName: message.payload.mapName || message.mapName,
              filter: message.payload.query?.where || message.payload.where,
              limit: message.payload.query?.limit || message.payload.limit,
            });
          }
          break;

        default:
          logger.debug(
            { type: message.type },
            'HTTP sync provider: unsupported message type dropped',
          );
          break;
      }
    } catch (err) {
      logger.warn({ err }, 'HTTP sync provider: failed to deserialize message');
    }
  }

  /**
   * Close the HTTP sync provider.
   * Stops the polling loop, clears queued operations, and sets disconnected state.
   */
  async close(): Promise<void> {
    this.stopPolling();
    this.pendingOperations = [];
    this.pendingQueries = [];
    this.connected = false;
    logger.info({ url: this.url }, 'HttpSyncProvider closed');
  }

  /**
   * Update the auth token (e.g., after token refresh).
   */
  setAuthToken(token: string): void {
    this.authToken = token;
  }

  /**
   * Send an HTTP sync request with queued operations and receive deltas.
   */
  private async doSyncRequest(): Promise<void> {
    // Build syncMaps with lastSyncTimestamp per map
    const syncMaps = this.syncMaps.map((mapName) => ({
      mapName,
      lastSyncTimestamp: this.lastSyncTimestamps.get(mapName) || {
        millis: 0,
        counter: 0,
        nodeId: '',
      },
    }));

    // Drain queued operations and queries
    const operations = this.pendingOperations.splice(0);
    const queries = this.pendingQueries.splice(0);

    const requestBody: any = {
      clientId: this.clientId,
      clientHlc: this.hlc.now(),
    };

    if (operations.length > 0) {
      requestBody.operations = operations;
    }
    if (syncMaps.length > 0) {
      requestBody.syncMaps = syncMaps;
    }
    if (queries.length > 0) {
      requestBody.queries = queries;
    }

    const bodyBytes = serialize(requestBody);
    // Create a fresh ArrayBuffer copy for fetch body compatibility
    const bodyBuffer = new ArrayBuffer(bodyBytes.byteLength);
    new Uint8Array(bodyBuffer).set(bodyBytes);

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.requestTimeoutMs);

    try {
      const response = await this.fetchImpl(`${this.url}/sync`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/x-msgpack',
          'Authorization': `Bearer ${this.authToken}`,
        },
        body: bodyBuffer,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        throw new Error(`HTTP sync request failed: ${response.status} ${response.statusText}`);
      }

      const responseBuffer = await response.arrayBuffer();
      const syncResponse = deserialize<any>(new Uint8Array(responseBuffer));

      // Update server HLC
      if (syncResponse.serverHlc) {
        this.hlc.update(syncResponse.serverHlc);
      }

      // Process operation acknowledgments
      if (syncResponse.ack) {
        this.emit('message', 'http', serialize({
          type: 'OP_ACK',
          payload: syncResponse.ack,
        }));
      }

      // Process deltas as server events
      if (syncResponse.deltas) {
        for (const delta of syncResponse.deltas) {
          // Update lastSyncTimestamp for this map
          this.lastSyncTimestamps.set(delta.mapName, delta.serverSyncTimestamp);

          // Emit each record as a server event
          for (const record of delta.records) {
            this.emit('message', 'http', serialize({
              type: 'SERVER_EVENT',
              payload: {
                mapName: delta.mapName,
                key: record.key,
                record: record.record,
                eventType: record.eventType,
              },
            }));
          }
        }
      }

      // Process query results
      if (syncResponse.queryResults) {
        for (const result of syncResponse.queryResults) {
          this.emit('message', 'http', serialize({
            type: 'QUERY_RESP',
            payload: {
              requestId: result.queryId,
              results: result.results,
              hasMore: result.hasMore,
              nextCursor: result.nextCursor,
            },
          }));
        }
      }

      // Handle connection state transitions
      if (!this.connected) {
        this.connected = true;
        if (this.wasConnected) {
          this.emit('reconnected', 'http');
        } else {
          this.wasConnected = true;
          this.emit('connected', 'http');
        }
      }
    } catch (err) {
      clearTimeout(timeoutId);

      if (this.connected) {
        this.connected = false;
        this.emit('disconnected', 'http');
      }

      // Re-queue operations that weren't sent
      if (operations.length > 0) {
        this.pendingOperations.unshift(...operations);
      }
      if (queries.length > 0) {
        this.pendingQueries.unshift(...queries);
      }

      throw err;
    }
  }

  /**
   * Start the polling loop.
   */
  private startPolling(): void {
    if (this.pollTimer) return;

    this.pollTimer = setInterval(async () => {
      try {
        await this.doSyncRequest();
      } catch (err) {
        logger.debug({ err }, 'HTTP sync poll failed');
      }
    }, this.pollIntervalMs);
  }

  /**
   * Stop the polling loop.
   */
  private stopPolling(): void {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  /**
   * Emit an event to all listeners.
   */
  private emit(event: ConnectionProviderEvent, ...args: any[]): void {
    const handlers = this.listeners.get(event);
    if (handlers) {
      for (const handler of handlers) {
        try {
          handler(...args);
        } catch (err) {
          logger.error({ err, event }, 'Error in HttpSyncProvider event handler');
        }
      }
    }
  }
}
