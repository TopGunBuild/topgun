import type { HLC } from '@topgunbuild/core';
import type {
  IConnectionProvider,
  ConnectionProviderEvent,
  ConnectionEventHandler,
} from '../types';
import { SingleServerProvider } from './SingleServerProvider';
import { HttpSyncProvider } from './HttpSyncProvider';
import type { HttpSyncProviderConfig } from './HttpSyncProvider';
import { logger } from '../utils/logger';

/**
 * Configuration for AutoConnectionProvider.
 */
export interface AutoConnectionProviderConfig {
  /** Server URL (ws:// or http://) */
  url: string;
  /** Client identifier */
  clientId: string;
  /** Hybrid Logical Clock instance */
  hlc: HLC;
  /** Max WebSocket connection attempts before falling back to HTTP (default: 3) */
  maxWsAttempts?: number;
  /** JWT auth token */
  authToken?: string;
  /** Skip WebSocket and go HTTP-only */
  httpOnly?: boolean;
  /** HTTP polling interval in ms (default: 5000) */
  httpPollIntervalMs?: number;
  /** Map names to sync via HTTP */
  syncMaps?: string[];
  /** Custom fetch implementation for HTTP mode */
  fetchImpl?: typeof fetch;
}

/**
 * AutoConnectionProvider implements protocol negotiation by trying WebSocket
 * first and falling back to HTTP sync when WebSocket connection fails.
 *
 * This enables seamless deployment in both traditional server environments
 * (using WebSockets) and serverless environments (using HTTP polling).
 */
export class AutoConnectionProvider implements IConnectionProvider {
  private readonly config: AutoConnectionProviderConfig;
  private readonly maxWsAttempts: number;

  /** The active underlying provider */
  private activeProvider: IConnectionProvider | null = null;
  /** Whether we're using HTTP mode */
  private isHttpMode: boolean;

  private listeners: Map<ConnectionProviderEvent, Set<ConnectionEventHandler>> = new Map();

  constructor(config: AutoConnectionProviderConfig) {
    this.config = config;
    this.maxWsAttempts = config.maxWsAttempts ?? 3;
    this.isHttpMode = config.httpOnly ?? false;
  }

  /**
   * Connect using WebSocket first, falling back to HTTP after maxWsAttempts failures.
   * If httpOnly is true, skips WebSocket entirely.
   */
  async connect(): Promise<void> {
    if (this.isHttpMode) {
      await this.connectHttp();
      return;
    }

    // Try WebSocket first
    const wsUrl = this.toWsUrl(this.config.url);
    let lastError: Error | null = null;

    for (let attempt = 0; attempt < this.maxWsAttempts; attempt++) {
      try {
        const wsProvider = new SingleServerProvider({
          url: wsUrl,
          maxReconnectAttempts: 1,
          reconnectDelayMs: 1000,
        });

        await wsProvider.connect();

        // WebSocket connected successfully
        this.activeProvider = wsProvider;
        this.proxyEvents(wsProvider);
        logger.info({ url: wsUrl }, 'AutoConnectionProvider: WebSocket connected');
        return;
      } catch (err: any) {
        lastError = err;
        logger.debug(
          { attempt: attempt + 1, maxAttempts: this.maxWsAttempts, err: err.message },
          'AutoConnectionProvider: WebSocket attempt failed',
        );
      }
    }

    // WebSocket failed after all attempts, fall back to HTTP
    logger.info(
      { wsAttempts: this.maxWsAttempts, url: this.config.url },
      'AutoConnectionProvider: WebSocket failed, falling back to HTTP',
    );

    this.isHttpMode = true;
    await this.connectHttp();
  }

  /**
   * Connect using HTTP sync provider.
   */
  private async connectHttp(): Promise<void> {
    const httpUrl = this.toHttpUrl(this.config.url);
    const httpProvider = new HttpSyncProvider({
      url: httpUrl,
      clientId: this.config.clientId,
      hlc: this.config.hlc,
      authToken: this.config.authToken,
      pollIntervalMs: this.config.httpPollIntervalMs,
      syncMaps: this.config.syncMaps,
      fetchImpl: this.config.fetchImpl,
    });

    await httpProvider.connect();

    this.activeProvider = httpProvider;
    this.proxyEvents(httpProvider);
    logger.info({ url: httpUrl }, 'AutoConnectionProvider: HTTP connected');
  }

  getConnection(key: string): WebSocket {
    if (!this.activeProvider) {
      throw new Error('Not connected');
    }
    return this.activeProvider.getConnection(key);
  }

  getAnyConnection(): WebSocket {
    if (!this.activeProvider) {
      throw new Error('Not connected');
    }
    return this.activeProvider.getAnyConnection();
  }

  isConnected(): boolean {
    return this.activeProvider?.isConnected() ?? false;
  }

  getConnectedNodes(): string[] {
    return this.activeProvider?.getConnectedNodes() ?? [];
  }

  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(handler);

    // If we already have an active provider, register on it too
    if (this.activeProvider) {
      this.activeProvider.on(event, handler);
    }
  }

  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.listeners.get(event)?.delete(handler);

    if (this.activeProvider) {
      this.activeProvider.off(event, handler);
    }
  }

  send(data: ArrayBuffer | Uint8Array, key?: string): void {
    if (!this.activeProvider) {
      throw new Error('Not connected');
    }
    this.activeProvider.send(data, key);
  }

  /**
   * Close the active underlying provider.
   */
  async close(): Promise<void> {
    if (this.activeProvider) {
      await this.activeProvider.close();
      this.activeProvider = null;
    }
  }

  /**
   * Whether currently using HTTP mode.
   */
  isUsingHttp(): boolean {
    return this.isHttpMode;
  }

  /**
   * Proxy events from the underlying provider to our listeners.
   */
  private proxyEvents(provider: IConnectionProvider): void {
    const events: ConnectionProviderEvent[] = [
      'connected',
      'disconnected',
      'reconnected',
      'message',
      'partitionMapUpdated',
      'error',
    ];

    for (const event of events) {
      const handlers = this.listeners.get(event);
      if (handlers) {
        for (const handler of handlers) {
          provider.on(event, handler);
        }
      }
    }
  }

  /**
   * Convert a URL to WebSocket URL format.
   */
  private toWsUrl(url: string): string {
    return url
      .replace(/^http:\/\//, 'ws://')
      .replace(/^https:\/\//, 'wss://');
  }

  /**
   * Convert a URL to HTTP URL format.
   */
  private toHttpUrl(url: string): string {
    return url
      .replace(/^ws:\/\//, 'http://')
      .replace(/^wss:\/\//, 'https://');
  }
}
