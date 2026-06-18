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
 *
 * `maxReconnectAttempts` defaults to Infinity: for a real-time + offline product,
 * a server that is briefly gone (deploy window, laptop sleep, server-started-after-client)
 * is the NORMAL case, not a terminal one. Giving up after a finite budget silently
 * stops sync while the app still "works". The retry rate is bounded by the capped
 * backoff (~1 attempt / maxReconnectDelayMs), so unbounded attempts do not hammer the
 * server. Callers who genuinely want a bounded policy can set a finite
 * `maxReconnectAttempts`; exhaustion is then surfaced honestly via a
 * ReconnectExhaustedError 'error' event (see scheduleReconnect).
 */
const DEFAULT_CONFIG = {
  maxReconnectAttempts: Infinity,
  reconnectDelayMs: 1000,
  backoffMultiplier: 2,
  maxReconnectDelayMs: 30000,
};

/**
 * Emitted via the 'error' connection event when a caller-configured finite
 * `maxReconnectAttempts` budget is exhausted and the provider stops retrying.
 * Distinct from transient WebSocket 'error' events (which do NOT stop reconnect):
 * this is the honest, terminal "giving up" signal. SyncEngine maps it to
 * SyncState.ERROR so consumers can react instead of silently believing they are
 * still trying to reconnect. With the default Infinity budget this is never thrown.
 */
export class ReconnectExhaustedError extends Error {
  /** Marks this as the terminal give-up signal (survives cross-realm instanceof gaps). */
  readonly terminal = true;
  /** Number of reconnect attempts made before giving up. */
  readonly attempts: number;

  constructor(attempts: number, url: string) {
    super(
      `Reconnect budget exhausted after ${attempts} attempt(s) to ${url}; ` +
        `giving up (set maxReconnectAttempts: Infinity to retry indefinitely, ` +
        `or call resetConnection() to start a fresh budget).`,
    );
    this.name = 'ReconnectExhaustedError';
    this.attempts = attempts;
  }
}

/**
 * Detach a background timer from the host event loop. A pending reconnect or
 * connection-timeout is background machinery — it must never be the sole reason
 * a Node process (or a Jest worker) stays alive. Node timers expose unref();
 * in browsers setTimeout returns a number with no unref(), so this is a no-op there.
 */
function unrefTimer(timer: ReturnType<typeof setTimeout> | null): void {
  (timer as unknown as { unref?: () => void } | null)?.unref?.();
}

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
  private connectionTimeoutId: ReturnType<typeof setTimeout> | null = null;
  private isClosing: boolean = false;
  // When a connect() promise is in-flight, its reject handle is stored here so
  // that close() can reject it immediately instead of leaving a pending Promise
  // that would keep the Jest event loop alive after teardown.
  private pendingConnectReject: ((err: Error) => void) | null = null;
  private listeners: Map<ConnectionProviderEvent, Set<ConnectionEventHandler>> = new Map();
  private onlineHandler: (() => void) | null = null;
  private offlineHandler: (() => void) | null = null;

  constructor(config: SingleServerProviderConfig) {
    this.url = config.url;
    this.config = {
      url: config.url,
      maxReconnectAttempts: config.maxReconnectAttempts ?? DEFAULT_CONFIG.maxReconnectAttempts,
      reconnectDelayMs: config.reconnectDelayMs ?? DEFAULT_CONFIG.reconnectDelayMs,
      backoffMultiplier: config.backoffMultiplier ?? DEFAULT_CONFIG.backoffMultiplier,
      maxReconnectDelayMs: config.maxReconnectDelayMs ?? DEFAULT_CONFIG.maxReconnectDelayMs,
      listenNetworkEvents: config.listenNetworkEvents ?? true,
    };

    this.setupNetworkListeners();
  }

  /**
   * Connect to the WebSocket server.
   */
  async connect(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      return; // Already connected
    }

    // Skip connection attempt when browser reports no network —
    // the 'online' event listener will trigger forceReconnect() when network returns
    if (typeof globalThis.navigator !== 'undefined' && globalThis.navigator.onLine === false) {
      throw new Error('Browser is offline — skipping connection attempt');
    }

    this.isClosing = false;

    return new Promise((resolve, reject) => {
      // Store reject so close() can reject this Promise immediately if called
      // while the connect attempt is in-flight — prevents a permanently-pending
      // Promise from keeping the Jest event loop alive after test teardown.
      this.pendingConnectReject = reject;

      // Per-attempt lifecycle flags. `opened` distinguishes a post-establishment
      // drop (reconnect, don't reject the already-resolved promise) from a failed
      // connect. `settled` makes succeed()/fail() run exactly once per attempt —
      // necessary because a single failed connect can surface through onerror AND
      // onclose AND the timeout, and the undici/WHATWG client does NOT reliably
      // fire 'close' for a failed connect, so the retry must also be driven from
      // onerror and the timeout (relying on onclose alone strands the client and
      // breaks server-after-client recovery — TODO-414).
      let opened = false;
      let settled = false;

      const clearConnectionTimeout = () => {
        if (this.connectionTimeoutId !== null) {
          clearTimeout(this.connectionTimeoutId);
          this.connectionTimeoutId = null;
        }
      };

      try {
        const ws = new WebSocket(this.url);
        this.ws = ws;
        ws.binaryType = 'arraybuffer';

        const succeed = () => {
          if (settled) return;
          settled = true;
          opened = true;
          clearConnectionTimeout();
          this.pendingConnectReject = null;
          this.reconnectAttempts = 0;
          logger.info({ url: this.url }, 'SingleServerProvider connected');
          this.emit('connected', 'default');
          resolve();
        };

        // An attempt failed to establish (refused / closed-before-open / timed out).
        // De-duplicated via `settled`; schedules the next retry (the single retry
        // driver for failed connects).
        const fail = (reason: string) => {
          if (settled) return;
          settled = true;
          clearConnectionTimeout();
          this.pendingConnectReject = null;
          this.emit('disconnected', 'default');
          if (!this.isClosing) {
            this.scheduleReconnect();
          }
          reject(new Error(reason));
        };

        // Per-attempt connection timeout, bound to THIS socket (never `this.ws`),
        // so a timeout from an earlier failed attempt can only ever close its OWN
        // socket — it can no longer kill a later attempt's in-flight handshake.
        this.connectionTimeoutId = setTimeout(() => {
          this.connectionTimeoutId = null;
          if (ws.readyState !== WebSocket.OPEN) {
            try {
              ws.close();
            } catch {
              // Closing a still-CONNECTING socket can throw; ignore.
            }
            fail(`Connection timeout to ${this.url}`);
          }
        }, this.config.reconnectDelayMs * 5); // 5x initial delay as connection timeout
        unrefTimer(this.connectionTimeoutId);

        ws.onopen = () => succeed();

        ws.onerror = (error) => {
          logger.error({ err: error, url: this.url }, 'SingleServerProvider WebSocket error');
          this.emit('error', error);
          // A pre-open error means the connect failed. The client may not fire
          // 'close', so drive the retry from here. An error on an already-OPEN
          // socket is left to onclose.
          if (!opened) {
            fail('WebSocket connection failed');
          }
        };

        ws.onclose = (event) => {
          logger.info({ url: this.url, code: event.code }, 'SingleServerProvider disconnected');
          if (opened) {
            // Drop AFTER the connection was established: not part of the connect()
            // promise (already resolved). Emit + reconnect.
            clearConnectionTimeout();
            this.emit('disconnected', 'default');
            if (!this.isClosing) {
              this.scheduleReconnect();
            }
          } else {
            // Closed before it ever opened — a failed attempt. `settled` makes this
            // a no-op if onerror/timeout already handled it.
            fail(`WebSocket closed before open (code ${event.code})`);
          }
        };

        ws.onmessage = (event) => {
          this.emit('message', 'default', event.data);
        };
      } catch (error) {
        // Synchronous construction failure (e.g. invalid URL). Schedule a retry too.
        clearConnectionTimeout();
        this.pendingConnectReject = null;
        if (!settled) {
          settled = true;
          if (!this.isClosing) {
            this.scheduleReconnect();
          }
        }
        reject(error instanceof Error ? error : new Error(String(error)));
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
    this.teardownNetworkListeners();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    // Cancel the pending connection-timeout if close() is called mid-CONNECTING.
    // Without this, the timer would outlive the test and keep Jest's event loop alive.
    if (this.connectionTimeoutId) {
      clearTimeout(this.connectionTimeoutId);
      this.connectionTimeoutId = null;
    }

    // Reject any in-flight connect() Promise so it does not hang indefinitely.
    // If we null ws.onclose before calling ws.close(), the Promise's close-triggered
    // reject path never fires — the pending Promise would keep the event loop alive.
    if (this.pendingConnectReject) {
      this.pendingConnectReject(new Error('Provider closed'));
      this.pendingConnectReject = null;
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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- emit spreads heterogeneous args per event type (connected/disconnected pass nodeId; error passes Error); rest param accepts any
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
    // Do not schedule if close() has been called — prevents a stale timer from
    // keeping the Jest event loop alive after test teardown.
    if (this.isClosing) return;

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    // When browser reports offline, don't schedule reconnects —
    // the 'online' event listener will call forceReconnect() when network returns
    if (
      typeof globalThis.navigator !== 'undefined' &&
      globalThis.navigator.onLine === false &&
      this.config.listenNetworkEvents
    ) {
      logger.info(
        { url: this.url },
        'Browser offline — waiting for online event instead of polling',
      );
      return;
    }

    if (this.reconnectAttempts >= this.config.maxReconnectAttempts) {
      logger.error(
        { attempts: this.reconnectAttempts, url: this.url },
        'SingleServerProvider max reconnect attempts reached — giving up',
      );
      // Terminal, typed signal so SyncEngine can transition to SyncState.ERROR
      // instead of leaving the client silently parked in DISCONNECTED.
      this.emit('error', new ReconnectExhaustedError(this.reconnectAttempts, this.url));
      return;
    }

    const delay = this.calculateBackoffDelay();
    logger.info(
      { delay, attempt: this.reconnectAttempts, url: this.url },
      `SingleServerProvider scheduling reconnect in ${delay}ms`,
    );

    this.reconnectTimer = setTimeout(async () => {
      this.reconnectTimer = null;
      // Guard: close() may have been called while this timer was pending.
      // Without this check, the reconnect would call connect() which resets
      // isClosing = false and starts a new connection cycle after teardown.
      if (this.isClosing) return;
      this.reconnectAttempts++;

      try {
        await this.connect();
        this.emit('reconnected', 'default');
      } catch (error) {
        logger.error({ err: error }, 'SingleServerProvider reconnection failed');
        this.scheduleReconnect();
      }
    }, delay);
    unrefTimer(this.reconnectTimer);
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
   * Force-close the current WebSocket and immediately schedule reconnection.
   * Unlike close(), this does NOT set isClosing and preserves reconnect behavior.
   * Resets the reconnect counter so the full backoff budget is available.
   *
   * Critically, this does NOT wait for the TCP close handshake (which can
   * hang 20+ seconds on a dead network). Instead it strips all handlers from
   * the old WebSocket, fires a best-effort close(), nulls the reference, and
   * schedules reconnect right away.
   */
  forceReconnect(): void {
    this.reconnectAttempts = 0;
    this.isClosing = false;

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.ws) {
      // Detach all handlers so the lingering socket cannot fire events
      this.ws.onopen = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;

      // Best-effort close — don't await the TCP handshake
      try {
        this.ws.close();
      } catch {
        // Ignore errors on already-dead sockets
      }
      this.ws = null;
    }

    // Emit disconnected so SyncEngine knows connection is down NOW
    this.emit('disconnected', 'default');

    // Schedule reconnect immediately (delay 0 → first attempt uses jittered base delay)
    this.scheduleReconnect();
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

  /**
   * Update the reconnect ceiling at runtime.
   *
   * Used by AutoConnectionProvider to promote a single-shot WebSocket *probe*
   * (built with maxReconnectAttempts: 0 so it never spins a background loop while
   * negotiating WS-vs-HTTP) into a *resilient persistent* connection once the
   * probe succeeds — without opening a second socket.
   */
  setMaxReconnectAttempts(maxReconnectAttempts: number): void {
    this.config.maxReconnectAttempts = maxReconnectAttempts;
  }

  /**
   * Listen for browser 'online' event to trigger instant reconnect
   * when network comes back. Only active in browser environments.
   */
  private setupNetworkListeners(): void {
    if (!this.config.listenNetworkEvents) return;
    if (typeof globalThis.addEventListener !== 'function') return;

    this.onlineHandler = () => {
      if (this.isClosing) return;
      if (this.isConnected()) return;

      logger.info({ url: this.url }, 'Network online detected — forcing reconnect');
      this.forceReconnect();
    };

    this.offlineHandler = () => {
      if (this.isClosing) return;
      if (!this.isConnected()) return;

      logger.info({ url: this.url }, 'Network offline detected — disconnecting immediately');
      this.forceReconnect();
    };

    globalThis.addEventListener('online', this.onlineHandler);
    globalThis.addEventListener('offline', this.offlineHandler);
  }

  /**
   * Remove browser network event listeners.
   */
  private teardownNetworkListeners(): void {
    if (typeof globalThis.removeEventListener === 'function') {
      if (this.onlineHandler) {
        globalThis.removeEventListener('online', this.onlineHandler);
        this.onlineHandler = null;
      }
      if (this.offlineHandler) {
        globalThis.removeEventListener('offline', this.offlineHandler);
        this.offlineHandler = null;
      }
    }
  }
}
