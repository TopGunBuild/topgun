import type {
  IConnectionProvider,
  IConnection,
  ConnectionProviderEvent,
  ConnectionEventHandler,
  SingleServerProviderConfig,
} from '../types';
import { WebSocketConnection } from './WebSocketConnection';
import { logger } from '../utils/logger';

/**
 * Default configuration values for SingleServerProvider.
 */
const DEFAULT_CONFIG = {
  maxReconnectAttempts: 10,
  reconnectDelayMs: 1000,
  backoffMultiplier: 2,
  maxReconnectDelayMs: 30000,
};

/**
 * SingleServerProvider implements IConnectionProvider for single-server mode.
 *
 * This is an adapter that wraps direct WebSocket connection handling,
 * providing the same interface used by ClusterClient for multi-node mode.
 */
export class SingleServerProvider implements IConnectionProvider {
  private readonly url: string;
  private readonly config: Required<SingleServerProviderConfig>;
  private ws: WebSocket | null = null;
  private reconnectAttempts: number = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private isClosing: boolean = false;
  private listeners: Map<ConnectionProviderEvent, Set<ConnectionEventHandler>> = new Map();

  constructor(config: SingleServerProviderConfig) {
    this.url = config.url;
    this.config = {
      url: config.url,
      maxReconnectAttempts: config.maxReconnectAttempts ?? DEFAULT_CONFIG.maxReconnectAttempts,
      reconnectDelayMs: config.reconnectDelayMs ?? DEFAULT_CONFIG.reconnectDelayMs,
      backoffMultiplier: config.backoffMultiplier ?? DEFAULT_CONFIG.backoffMultiplier,
      maxReconnectDelayMs: config.maxReconnectDelayMs ?? DEFAULT_CONFIG.maxReconnectDelayMs,
    };
  }

  /**
   * Connect to the WebSocket server.
   */
  async connect(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      return; // Already connected
    }

    this.isClosing = false;

    return new Promise((resolve, reject) => {
      try {
        this.ws = new WebSocket(this.url);
        this.ws.binaryType = 'arraybuffer';

        this.ws.onopen = () => {
          this.reconnectAttempts = 0;
          logger.info({ url: this.url }, 'SingleServerProvider connected');
          this.emit('connected', 'default');
          resolve();
        };

        this.ws.onerror = (error) => {
          logger.error({ err: error, url: this.url }, 'SingleServerProvider WebSocket error');
          this.emit('error', error);
          // Don't reject here - wait for onclose
        };

        this.ws.onclose = (event) => {
          logger.info({ url: this.url, code: event.code }, 'SingleServerProvider disconnected');
          this.emit('disconnected', 'default');

          if (!this.isClosing) {
            this.scheduleReconnect();
          }
        };

        this.ws.onmessage = (event) => {
          this.emit('message', 'default', event.data);
        };

        // Set up initial connection timeout
        const timeoutId = setTimeout(() => {
          if (this.ws && this.ws.readyState === WebSocket.CONNECTING) {
            this.ws.close();
            reject(new Error(`Connection timeout to ${this.url}`));
          }
        }, this.config.reconnectDelayMs * 5); // 5x initial delay as connection timeout

        // Clear timeout on successful connection
        const originalOnOpen = this.ws.onopen;
        const wsRef = this.ws;
        this.ws.onopen = (ev) => {
          clearTimeout(timeoutId);
          if (originalOnOpen) {
            originalOnOpen.call(wsRef, ev);
          }
        };
      } catch (error) {
        reject(error);
      }
    });
  }

  /**
   * Get connection for a specific key.
   * In single-server mode, key is ignored.
   */
  getConnection(_key: string): IConnection {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error('Not connected');
    }
    return new WebSocketConnection(this.ws);
  }

  /**
   * Get any available connection.
   */
  getAnyConnection(): IConnection {
    return this.getConnection('');
  }

  /**
   * Check if connected.
   */
  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /**
   * Get connected node IDs.
   * Single-server mode returns ['default'] when connected.
   */
  getConnectedNodes(): string[] {
    return this.isConnected() ? ['default'] : [];
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
   * Send data via the WebSocket connection.
   * In single-server mode, key parameter is ignored.
   */
  send(data: ArrayBuffer | Uint8Array, _key?: string): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error('Not connected');
    }
    this.ws.send(data);
  }

  /**
   * Close the WebSocket connection.
   */
  async close(): Promise<void> {
    this.isClosing = true;

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.ws) {
      // Remove onclose handler to prevent reconnect
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;

      if (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING) {
        this.ws.close();
      }
      this.ws = null;
    }

    logger.info({ url: this.url }, 'SingleServerProvider closed');
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
          logger.error({ err, event }, 'Error in SingleServerProvider event handler');
        }
      }
    }
  }

  /**
   * Schedule a reconnection attempt with exponential backoff.
   */
  private scheduleReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.reconnectAttempts >= this.config.maxReconnectAttempts) {
      logger.error(
        { attempts: this.reconnectAttempts, url: this.url },
        'SingleServerProvider max reconnect attempts reached'
      );
      this.emit('error', new Error('Max reconnection attempts reached'));
      return;
    }

    const delay = this.calculateBackoffDelay();
    logger.info(
      { delay, attempt: this.reconnectAttempts, url: this.url },
      `SingleServerProvider scheduling reconnect in ${delay}ms`
    );

    this.reconnectTimer = setTimeout(async () => {
      this.reconnectTimer = null;
      this.reconnectAttempts++;

      try {
        await this.connect();
        this.emit('reconnected', 'default');
      } catch (error) {
        logger.error({ err: error }, 'SingleServerProvider reconnection failed');
        this.scheduleReconnect();
      }
    }, delay);
  }

  /**
   * Calculate backoff delay with exponential increase.
   */
  private calculateBackoffDelay(): number {
    const { reconnectDelayMs, backoffMultiplier, maxReconnectDelayMs } = this.config;
    let delay = reconnectDelayMs * Math.pow(backoffMultiplier, this.reconnectAttempts);
    delay = Math.min(delay, maxReconnectDelayMs);

    // Add jitter (0.5x to 1.5x)
    delay = delay * (0.5 + Math.random());

    return Math.floor(delay);
  }

  /**
   * Get the WebSocket URL this provider connects to.
   */
  getUrl(): string {
    return this.url;
  }

  /**
   * Get current reconnection attempt count.
   */
  getReconnectAttempts(): number {
    return this.reconnectAttempts;
  }

  /**
   * Reset reconnection counter.
   * Called externally after successful authentication.
   */
  resetReconnectAttempts(): void {
    this.reconnectAttempts = 0;
  }
}
