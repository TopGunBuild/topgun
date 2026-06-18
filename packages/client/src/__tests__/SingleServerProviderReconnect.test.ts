/**
 * Reconnect-honesty tests for SingleServerProvider (F2 / TODO-414 + TODO-429).
 *
 * Verifies the resilient-by-default reconnect contract:
 * - Default config retries indefinitely with capped backoff (survives many
 *   transient drops — the offline-first promise).
 * - Backoff grows exponentially and is bounded by maxReconnectDelayMs (never
 *   hammers the server).
 * - An opt-in finite cap is honoured: on exhaustion the provider stops AND emits
 *   a typed terminal ReconnectExhaustedError instead of failing silently.
 */

import { SingleServerProvider, ReconnectExhaustedError } from '../connection/SingleServerProvider';

// --- Controllable mock WebSocket -------------------------------------------
// mode 'success' → the socket opens shortly after construction.
// mode 'refuse'  → the socket closes shortly after construction (connection
//                  refused / server down), driving the provider's reconnect path.

type WsMode = 'success' | 'refuse';

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static mode: WsMode = 'success';
  static OPEN = 1;
  static CLOSED = 3;
  static CONNECTING = 0;

  readyState: number = MockWebSocket.CONNECTING;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    const mode = MockWebSocket.mode;
    // Resolve open/close asynchronously, mirroring a real socket handshake.
    setTimeout(() => {
      if (mode === 'success') {
        this.readyState = MockWebSocket.OPEN;
        this.onopen?.();
      } else {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.({ code: 1006, reason: 'Connection refused' });
      }
    }, 1);
  }

  send(): void {
    if (this.readyState !== MockWebSocket.OPEN) throw new Error('WebSocket is not open');
  }

  close(): void {
    if (this.readyState === MockWebSocket.CLOSED) return;
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: 'Normal closure' });
  }

  /** Simulate a server-initiated drop on an already-open socket. */
  serverDrop(): void {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1006, reason: 'Server dropped' });
  }

  static reset(): void {
    MockWebSocket.instances = [];
    MockWebSocket.mode = 'success';
  }

  static last(): MockWebSocket {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

(global as unknown as { WebSocket: unknown }).WebSocket = MockWebSocket;

function waitMs(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitUntil(cond: () => boolean, timeoutMs = 4000, interval = 5): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (cond()) return;
    await waitMs(interval);
  }
  throw new Error('waitUntil timeout');
}

describe('SingleServerProvider reconnect honesty (F2)', () => {
  beforeEach(() => {
    MockWebSocket.reset();
  });

  afterEach(() => {
    MockWebSocket.reset();
  });

  it('defaults to indefinite reconnect — survives far more than the old 10-attempt budget', async () => {
    MockWebSocket.mode = 'success';
    const provider = new SingleServerProvider({
      url: 'ws://localhost:9999',
      reconnectDelayMs: 2,
      maxReconnectDelayMs: 8,
      listenNetworkEvents: false,
    });

    let reconnects = 0;
    let terminalError = false;
    provider.on('reconnected', () => reconnects++);
    provider.on('error', (err: unknown) => {
      if (err instanceof ReconnectExhaustedError) terminalError = true;
    });

    await provider.connect();
    await waitUntil(() => provider.isConnected());

    // Drop the socket 15 times — well beyond the historical maxReconnectAttempts=10.
    // A finite-ceiling provider would have emitted a terminal error before here.
    const DROPS = 15;
    for (let i = 0; i < DROPS; i++) {
      MockWebSocket.last().serverDrop();
      await waitUntil(() => provider.isConnected());
    }

    expect(reconnects).toBeGreaterThanOrEqual(DROPS);
    expect(terminalError).toBe(false);
    expect(provider.isConnected()).toBe(true);

    await provider.close();
  });

  it('does NOT emit a terminal ReconnectExhaustedError while reconnecting (default) against a down server', async () => {
    MockWebSocket.mode = 'refuse';
    const provider = new SingleServerProvider({
      url: 'ws://localhost:9999',
      reconnectDelayMs: 2,
      maxReconnectDelayMs: 8,
      listenNetworkEvents: false,
    });

    let terminalError = false;
    provider.on('error', (err: unknown) => {
      if (err instanceof ReconnectExhaustedError) terminalError = true;
    });

    // connect() never resolves against a refusing server; ignore the pending promise.
    provider.connect().catch(() => {});

    // Let many reconnect cycles churn — default budget is Infinity, so the provider
    // keeps creating sockets and never gives up.
    await waitUntil(() => MockWebSocket.instances.length > 12);
    expect(terminalError).toBe(false);

    await provider.close();
  });

  it('honours an opt-in finite cap: stops after maxReconnectAttempts and emits a typed terminal error (negative control)', async () => {
    MockWebSocket.mode = 'refuse';
    const provider = new SingleServerProvider({
      url: 'ws://localhost:9999',
      maxReconnectAttempts: 4,
      reconnectDelayMs: 2,
      maxReconnectDelayMs: 8,
      listenNetworkEvents: false,
    });

    let terminalError: ReconnectExhaustedError | null = null;
    provider.on('error', (err: unknown) => {
      if (err instanceof ReconnectExhaustedError) terminalError = err;
    });

    provider.connect().catch(() => {});

    await waitUntil(() => terminalError !== null, 4000);

    expect(terminalError).toBeInstanceOf(ReconnectExhaustedError);
    expect(terminalError!.terminal).toBe(true);
    expect(terminalError!.attempts).toBe(4);
    expect(provider.getReconnectAttempts()).toBe(4);

    // After giving up, no further reconnect sockets are created.
    const countAtGiveUp = MockWebSocket.instances.length;
    await waitMs(40);
    expect(MockWebSocket.instances.length).toBe(countAtGiveUp);

    await provider.close();
  });

  it('backoff grows exponentially and is bounded by maxReconnectDelayMs (no DoS hammering)', () => {
    const provider = new SingleServerProvider({
      url: 'ws://localhost:9999',
      reconnectDelayMs: 1000,
      backoffMultiplier: 2,
      maxReconnectDelayMs: 30000,
      listenNetworkEvents: false,
    });

    // Pin jitter to 1.0x (random=0.5 → factor 0.5+0.5) for deterministic assertions.
    const randomSpy = jest.spyOn(Math, 'random').mockReturnValue(0.5);
    try {
      const delayAt = (attempt: number): number => {
        (provider as unknown as { reconnectAttempts: number }).reconnectAttempts = attempt;
        return (provider as unknown as { calculateBackoffDelay(): number }).calculateBackoffDelay();
      };

      expect(delayAt(0)).toBe(1000);
      expect(delayAt(1)).toBe(2000);
      expect(delayAt(2)).toBe(4000);
      expect(delayAt(3)).toBe(8000);

      // Grows monotonically until the cap, then stays capped — never unbounded.
      expect(delayAt(10)).toBe(30000);
      expect(delayAt(100)).toBe(30000);
      expect(delayAt(1000)).toBe(30000);
    } finally {
      randomSpy.mockRestore();
    }
  });

  it('keeps jittered backoff within [0.5x, 1.5x] of the cap at high attempt counts', () => {
    const provider = new SingleServerProvider({
      url: 'ws://localhost:9999',
      maxReconnectDelayMs: 30000,
      listenNetworkEvents: false,
    });
    (provider as unknown as { reconnectAttempts: number }).reconnectAttempts = 50;

    for (let i = 0; i < 50; i++) {
      const delay = (
        provider as unknown as { calculateBackoffDelay(): number }
      ).calculateBackoffDelay();
      expect(delay).toBeGreaterThanOrEqual(15000); // 0.5x cap
      expect(delay).toBeLessThanOrEqual(45000); // 1.5x cap
    }
  });
});
