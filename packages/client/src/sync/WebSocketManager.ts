/**
 * WebSocketManager - Handles all WebSocket/connection operations for SyncEngine
 *
 * Responsibilities:
 * - WebSocket lifecycle management (connect, close, reset)
 * - Message serialization/deserialization
 * - Heartbeat mechanism (PING/PONG)
 * - Reconnection with exponential backoff
 * - Event forwarding from connection provider
 */

import { serialize, deserialize } from '@topgunbuild/core';
import type {
  IConnectionProvider,
  ConnectionProviderEvent,
  ConnectionEventHandler,
} from '../types';
import { SyncState } from '../SyncState';
import { logger } from '../utils/logger';
import { unrefTimer } from '../utils/unrefTimer';
import type { IWebSocketManager, WebSocketManagerConfig } from './types';

/**
 * Duck-typed check for the terminal reconnect-give-up signal. Matched by the
 * `terminal === true` flag rather than `instanceof` so it holds across realms
 * and for any connection provider (single-server, cluster, custom) that adopts
 * the same convention without importing the connection module here.
 */
function isReconnectExhaustedError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as { terminal?: unknown }).terminal === true
  );
}

/**
 * Largest `CHUNK` sequence the client reassembles, in bytes (1 GiB). A message
 * above it could not be decoded by a JS client anyway (msgpackr materialises
 * objects several times the wire size); refusing it reconnects visibly instead
 * of exhausting the tab or process, and it also ends a sequence that never
 * receives its `last` chunk (TG-WS-002).
 */
export const MAX_REASSEMBLY_BYTES = 2 ** 30;

/**
 * The heartbeat expiry rule (TG-WS-003), shared by the tick and
 * `isConnectionHealthy` so the two can never disagree. The link is dead only
 * while a PING is outstanding and more than `timeoutMs` has passed since the
 * last inbound message: any message proves the server is still delivering, and
 * with no PING outstanding (every PING answered, e.g. a backgrounded tab whose
 * timer ticks late) silence alone proves nothing.
 */
export function heartbeatExpired(
  now: number,
  lastInboundAt: number,
  pingOutstanding: boolean,
  timeoutMs: number,
): boolean {
  return pingOutstanding && now - lastInboundAt > timeoutMs;
}

/**
 * Collects the `data` of one `CHUNK` sequence (TG-WS-002) until its `last`
 * chunk. Every violation throws and leaves the buffer empty, so a broken
 * sequence can never prefix a later message.
 */
export class ChunkReassembler {
  private parts: Uint8Array[] = [];
  private total = 0;

  constructor(private readonly maxBytes: number) {}

  /** True when no sequence is in progress. */
  isEmpty(): boolean {
    return this.parts.length === 0;
  }

  clear(): void {
    this.parts = [];
    this.total = 0;
  }

  /**
   * Appends one decoded `CHUNK`. Returns the whole original message bytes when
   * the chunk is the last of its sequence, else `null`. Throws on a malformed
   * chunk or when the sequence would exceed `maxBytes`.
   */
  push(chunk: { last?: unknown; data?: unknown }): Uint8Array | null {
    try {
      const { last, data } = chunk;
      if (!(data instanceof Uint8Array) || typeof last !== 'boolean') {
        throw new Error('malformed CHUNK: data must be bytes and last a boolean');
      }
      if (this.total + data.length > this.maxBytes) {
        throw new Error(`CHUNK sequence exceeds ${this.maxBytes} bytes`);
      }
      this.parts.push(data);
      this.total += data.length;
      if (!last) {
        return null;
      }
      const whole = new Uint8Array(this.total);
      let offset = 0;
      for (const part of this.parts) {
        whole.set(part, offset);
        offset += part.length;
      }
      this.clear();
      return whole;
    } catch (err) {
      this.clear();
      throw err;
    }
  }
}

/**
 * WebSocketManager implements IWebSocketManager.
 *
 * Manages WebSocket connections via IConnectionProvider.
 * Supports both single-server and cluster modes through the provider abstraction.
 */
export class WebSocketManager implements IWebSocketManager {
  private readonly config: WebSocketManagerConfig;
  private readonly connectionProvider: IConnectionProvider;

  // Reconnection state
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffAttempt: number = 0;

  // Heartbeat state
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private lastInboundAt: number = Date.now();
  private pingOutstanding: boolean = false;
  private lastRoundTripTime: number | null = null;

  // One CHUNK sequence per node: a cluster provider multiplexes several sockets.
  private readonly reassemblers: Map<string, ChunkReassembler> = new Map();

  constructor(config: WebSocketManagerConfig) {
    this.config = config;
    this.connectionProvider = config.connectionProvider;

    // Attached once for the manager's lifetime: connect() runs again after
    // setAuthToken during a disconnect and after resetConnection, and the provider
    // keeps its listeners across close(), so attaching them there would run every
    // handler once more per call (one AUTH and one routed message per listener).
    this.connectionProvider.on('connected', (nodeId: string) => {
      logger.info('ConnectionProvider connected.');
      // Cleared on both events, each on its own, so a partial sequence of a
      // dropped socket cannot prefix a message of the next one whichever event
      // the provider emits.
      this.reassemblers.delete(nodeId);
      // Re-arm the state machine to CONNECTING on every freshly-opened socket.
      // On a RECONNECT the machine sits in DISCONNECTED when this fires, so without
      // this re-arm the following CONNECTING→AUTHENTICATING auth step would instead
      // be an invalid DISCONNECTED→AUTHENTICATING transition; the AUTH_ACK would then
      // land in the wrong state and strand the client in CONNECTING (the TODO-414
      // "connects but never finishes" symptom). No-op on the initial connect, where
      // WebSocketManager.connect() already transitioned to CONNECTING.
      this.config.stateMachine.transition(SyncState.CONNECTING);
      this.config.onConnected?.();
    });

    this.connectionProvider.on('disconnected', (nodeId: string) => {
      logger.info('ConnectionProvider disconnected.');
      this.reassemblers.delete(nodeId);
      this.stopHeartbeat();
      this.config.stateMachine.transition(SyncState.DISCONNECTED);
      this.config.onDisconnected?.();
      // Don't schedule reconnect - provider handles it
    });

    this.connectionProvider.on('reconnected', (_nodeId: string) => {
      // Informational only. Every successful socket open — initial OR reconnect —
      // drives re-arm + auth exactly once through the 'connected' handler above,
      // which always fires first. Running auth again here produced a duplicate AUTH
      // frame and an invalid AUTHENTICATING→CONNECTING transition (the F7 double-auth
      // bug). Kept purely for provider-level observability.
      logger.info('ConnectionProvider reconnected.');
      this.config.onReconnected?.();
    });

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- WebSocket message data is a raw binary/string payload; type is narrowed after msgpack decode in deserializeMessage
    this.connectionProvider.on('message', (nodeId: string, data: any) => {
      // Liveness comes before decoding: any inbound message, a CHUNK or even
      // undecodable bytes, proves the server is still delivering. A PONG can sit
      // behind a long transfer in the server's single outbound queue (TG-WS-003).
      this.lastInboundAt = Date.now();
      this.pingOutstanding = false;
      const message = this.deserializeMessage(data);
      if (message) {
        this.receive(nodeId, message);
      }
    });

    this.connectionProvider.on('partitionMapUpdated', () => {
      logger.debug('Partition map updated');
    });

    this.connectionProvider.on('error', (error: Error) => {
      logger.error({ err: error }, 'ConnectionProvider error');
      // A ReconnectExhaustedError is the terminal "we gave up" signal — only ever
      // emitted when a caller configured a finite reconnect budget and it ran out.
      // Surface it honestly as SyncState.ERROR so consumers observing
      // onConnectionStateChange learn that reconnection has stopped, instead of
      // believing the client is still silently retrying. Transient WebSocket
      // 'error' events (which do NOT stop reconnect) carry an ordinary Error and
      // leave the state machine in DISCONNECTED/backoff.
      if (isReconnectExhaustedError(error)) {
        this.config.stateMachine.transition(SyncState.ERROR);
      }
    });
  }

  /**
   * Start the connection process. Listeners are attached in the constructor.
   */
  connect(): void {
    this.config.stateMachine.transition(SyncState.CONNECTING);
    this.connectionProvider.connect().catch((err) => {
      logger.error({ err }, 'Failed to connect via ConnectionProvider');
      this.config.stateMachine.transition(SyncState.DISCONNECTED);
    });
  }

  /**
   * Deserialize incoming message data.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- data is a raw WebSocket payload (ArrayBuffer, Uint8Array, or string); return is any because message type is narrowed by the caller per message.type
  private deserializeMessage(data: any): any {
    try {
      if (data instanceof ArrayBuffer) {
        return deserialize(new Uint8Array(data));
      } else if (data instanceof Uint8Array) {
        return deserialize(data);
      } else if (typeof data === 'string') {
        return JSON.parse(data);
      } else {
        return data;
      }
    } catch (e) {
      logger.error({ err: e }, 'Failed to parse message');
      return null;
    }
  }

  /**
   * Reassembles `CHUNK` sequences (TG-WS-002) and routes every other message.
   * A sequence that cannot be reassembled or decoded forces a reconnect: the
   * message it carried is then recovered by resync rather than silently lost.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message is a deserialized msgpack object; narrowed by message.type
  private receive(nodeId: string, message: any): void {
    const pending = this.reassemblers.get(nodeId);
    if (message.type !== 'CHUNK') {
      if (pending && !pending.isEmpty()) {
        this.chunkViolation(nodeId, new Error(`${message.type} arrived inside a CHUNK sequence`));
        return;
      }
      this.handleMessage(message);
      return;
    }

    const reassembler = pending ?? new ChunkReassembler(MAX_REASSEMBLY_BYTES);
    this.reassemblers.set(nodeId, reassembler);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the reassembled bytes decode to any server message
    let inner: any;
    try {
      const whole = reassembler.push(message);
      if (whole === null) {
        return;
      }
      inner = deserialize(whole);
    } catch (err) {
      this.chunkViolation(nodeId, err);
      return;
    }
    if (inner) {
      this.handleMessage(inner);
    }
  }

  private chunkViolation(nodeId: string, err: unknown): void {
    this.reassemblers.delete(nodeId);
    logger.error({ err, nodeId }, 'CHUNK sequence violation - reconnecting to resync');
    this.connectionProvider.forceReconnect();
  }

  /**
   * Handle incoming message.
   * Routes PONG to internal handler, all others to SyncEngine.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message is a deserialized msgpack object; type is narrowed by checking message.type before dispatch
  private handleMessage(message: any): void {
    // Handle PONG internally for heartbeat tracking
    if (message.type === 'PONG') {
      this.handlePong(message);
    }
    // Route all messages to SyncEngine (including PONG for HLC sync if needed)
    this.config.onMessage(message);
  }

  /**
   * Send a message through the current connection.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- message shape varies by operation type; serialized to msgpack binary before sending so the type is erased at this layer
  sendMessage(message: any, key?: string): boolean {
    const data = serialize(message);

    try {
      this.connectionProvider.send(data, key);
      return true;
    } catch (err) {
      logger.warn({ err }, 'Failed to send via ConnectionProvider');
      return false;
    }
  }

  /**
   * Check if we can send messages (connection is ready).
   */
  canSend(): boolean {
    return this.connectionProvider.isConnected();
  }

  /**
   * Check if connected to the server.
   */
  isOnline(): boolean {
    const state = this.config.stateMachine.getState();
    return (
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED
    );
  }

  /**
   * Get the connection provider.
   */
  getConnectionProvider(): IConnectionProvider {
    return this.connectionProvider;
  }

  /**
   * Close the connection and clean up resources.
   */
  close(): void {
    this.stopHeartbeat();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    this.connectionProvider.close().catch((err) => {
      logger.error({ err }, 'Error closing ConnectionProvider');
    });
  }

  /**
   * Reset connection state for a fresh reconnection.
   */
  reset(): void {
    this.close();
    this.resetBackoff();
  }

  /**
   * Subscribe to connection events.
   */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.connectionProvider.on(event, handler);
  }

  /**
   * Unsubscribe from connection events.
   */
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.connectionProvider.off(event, handler);
  }

  /**
   * Reset backoff counter.
   */
  resetBackoff(): void {
    this.backoffAttempt = 0;
  }

  /**
   * Get current backoff attempt count.
   */
  getBackoffAttempt(): number {
    return this.backoffAttempt;
  }

  /**
   * Clear reconnect timer (for external control, e.g., when new auth token provided).
   */
  clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  // ============================================
  // Heartbeat Mechanism
  // ============================================

  /**
   * Starts the heartbeat mechanism after successful connection.
   */
  startHeartbeat(): void {
    if (!this.config.heartbeatConfig.enabled) {
      return;
    }

    this.stopHeartbeat(); // Clear any existing interval
    this.lastInboundAt = Date.now();
    this.pingOutstanding = false;

    // Unref'd: the interval lives for as long as the connection does, so a
    // client that is never closed would otherwise hold the process open for ever.
    this.heartbeatInterval = unrefTimer(
      setInterval(() => this.heartbeatTick(), this.config.heartbeatConfig.intervalMs),
    );

    logger.info({ intervalMs: this.config.heartbeatConfig.intervalMs }, 'Heartbeat started');
  }

  /**
   * Stops the heartbeat mechanism.
   */
  stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
      logger.info('Heartbeat stopped');
    }
  }

  /**
   * Sends a PING message to the server.
   */
  private sendPing(): void {
    if (this.canSend()) {
      const pingMessage = {
        type: 'PING',
        timestamp: Date.now(),
      };
      this.sendMessage(pingMessage);
    }
  }

  /**
   * Handles incoming PONG message from server. Liveness is already recorded by
   * the message listener; a PONG only adds the round-trip time.
   */
  private handlePong(msg: { timestamp: number; serverTime: number }): void {
    const now = Date.now();
    this.lastRoundTripTime = now - msg.timestamp;

    logger.debug(
      {
        rtt: this.lastRoundTripTime,
        serverTime: msg.serverTime,
        clockSkew: msg.serverTime - (msg.timestamp + this.lastRoundTripTime / 2),
      },
      'Received PONG',
    );
  }

  /**
   * One heartbeat tick: check first, then ping. Checking first means a PING sent
   * at this tick is never judged by this tick, so expiry needs a PING that went
   * unanswered for a whole interval. The flag is set even when the socket cannot
   * send, so a socket that is no longer open but never reported `disconnected`
   * (a stalled closing handshake) still expires like a dead link (TG-WS-003).
   */
  private heartbeatTick(): void {
    const now = Date.now();
    const { timeoutMs } = this.config.heartbeatConfig;

    if (heartbeatExpired(now, this.lastInboundAt, this.pingOutstanding, timeoutMs)) {
      logger.warn(
        { timeSinceLastInbound: now - this.lastInboundAt, timeoutMs },
        'Heartbeat timeout - triggering reconnection',
      );

      this.stopHeartbeat();

      // Force reconnect — preserves the provider's reconnect behavior
      // (unlike close() which permanently kills the connection)
      this.connectionProvider.forceReconnect();
      return;
    }

    this.pingOutstanding = true;
    this.sendPing();
  }

  /**
   * Returns the last measured round-trip time in milliseconds. While a PONG
   * waits behind a long transfer in the server's queue, this reads that
   * transfer time rather than the link's round trip.
   */
  getLastRoundTripTime(): number | null {
    return this.lastRoundTripTime;
  }

  /**
   * Returns true while the connection is online, authenticated and its heartbeat
   * has not expired under `heartbeatExpired` (TG-WS-003). Expiry needs a PING
   * outstanding and strictly more than `timeoutMs` since the last inbound
   * message of any type, so exactly `timeoutMs` still reads healthy, and with
   * no PING outstanding (the last one was answered, e.g. a backgrounded tab
   * whose timer has not ticked since) the connection reads healthy at any age.
   */
  isConnectionHealthy(): boolean {
    const state = this.config.stateMachine.getState();
    const isOnline =
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED;
    const isAuthenticated = state === SyncState.SYNCING || state === SyncState.CONNECTED;

    if (!isOnline || !isAuthenticated) {
      return false;
    }

    if (!this.config.heartbeatConfig.enabled) {
      return true; // If heartbeat disabled, consider healthy if online
    }

    return !heartbeatExpired(
      Date.now(),
      this.lastInboundAt,
      this.pingOutstanding,
      this.config.heartbeatConfig.timeoutMs,
    );
  }
}
