/**
 * Tests for SyncEngine auth-optional fast path.
 *
 * Covers three connection scenarios:
 *   1. No token + server sends no AUTH_REQUIRED → client drives to CONNECTED via grace timeout
 *   2. No token + server sends AUTH_REQUIRED     → client parks in AUTHENTICATING
 *   3. Token configured                          → client sends AUTH immediately
 */

import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { SyncState } from '../SyncState';

// ──────────────────────────────────────────────
// Mock WebSocket (same pattern as SyncEngine.test.ts)
// ──────────────────────────────────────────────
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.OPEN;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: any) => void) | null = null;
  sentMessages: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    // Simulate async connection — fires after current microtask queue drains.
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
  }

  send(data: Uint8Array | string) {
    if (this.readyState !== MockWebSocket.OPEN) {
      throw new Error('WebSocket is not open');
    }
    if (data instanceof Uint8Array) {
      this.sentMessages.push(deserialize(data));
    } else {
      this.sentMessages.push(JSON.parse(data));
    }
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    if (this.onclose) this.onclose({ code: 1000, reason: 'Normal closure' });
  }

  simulateMessage(message: any) {
    if (this.onmessage) {
      const data = serialize(message);
      const exactBuffer = new Uint8Array(data).buffer;
      this.onmessage({ data: exactBuffer });
    }
  }

  static reset() {
    MockWebSocket.instances = [];
  }

  static getLastInstance(): MockWebSocket | undefined {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

(global as any).WebSocket = MockWebSocket;

// ──────────────────────────────────────────────
// Mock Storage Adapter
// ──────────────────────────────────────────────
function createMockStorageAdapter(): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getMeta: jest.fn().mockResolvedValue(undefined),
    setMeta: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn().mockResolvedValue([]),
  };
}

// Mock crypto.randomUUID
let uuidCounter = 0;
(global as any).crypto = {
  randomUUID: () => `test-uuid-${++uuidCounter}`,
};

describe('SyncEngine — auth-optional fast path', () => {
  let engine: SyncEngine | undefined;
  let mockStorage: jest.Mocked<IStorageAdapter>;
  let config: SyncEngineConfig;

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.reset();
    uuidCounter = 0;

    mockStorage = createMockStorageAdapter();
    config = {
      nodeId: 'test-node',
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
      storageAdapter: mockStorage,
      // Heartbeat disabled so heartbeat timers don't interfere with grace-timer assertions.
      heartbeat: { enabled: false },
    };
  });

  afterEach(() => {
    // Dispose the engine before switching back to real timers. SingleServerProvider's
    // reconnectTimer (scheduled from WebSocket onclose handler) is cleared synchronously
    // inside close(); doing it under fake timers ensures any pending fake-timer callbacks
    // are flushed in fake-timer mode rather than leaking into the real event loop.
    if (engine) {
      engine.close();
      engine = undefined;
    }
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  // ────────────────────────────────────────────
  // Case 1: No token, no AUTH_REQUIRED
  // ────────────────────────────────────────────
  it('case 1: reaches CONNECTED within grace window when server sends no AUTH_ACK and no token is configured', async () => {
    engine = new SyncEngine(config);

    // Flush the setTimeout(..., 0) that fires onopen, triggering handleConnectionEstablished.
    // This sends the opportunistic NO_AUTH AUTH and arms the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    // NO_AUTH now presents an opportunistic AUTH{token:''} and moves to AUTHENTICATING.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);
    const ws = MockWebSocket.getLastInstance()!;
    const optimisticAuth = ws.sentMessages.find((m) => m.type === 'AUTH');
    expect(optimisticAuth).toBeDefined();
    expect(optimisticAuth?.token).toBe('');
    // No persisted device credential → no deviceToken presented.
    expect(optimisticAuth?.deviceToken).toBeUndefined();

    // Advance past the 500ms grace window — legacy server sent no AUTH_ACK.
    jest.advanceTimersByTime(500);
    await Promise.resolve(); // let any microtasks flush

    // Client should have reached CONNECTED (degraded-to-legacy).
    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    // AC-4a: Exact observed transition sequence via history (includes CONNECTING which
    // fires synchronously during construction, before an external listener can attach).
    const history = engine.getStateHistory();
    const historyStates = history.map((e) => e.to);
    expect(historyStates).toEqual([
      SyncState.CONNECTING,
      SyncState.AUTHENTICATING,
      SyncState.SYNCING,
      SyncState.CONNECTED,
    ]);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 2: No token, AUTH_REQUIRED arrives before grace expiry
  // ────────────────────────────────────────────
  it('case 2: parks in AUTHENTICATING when AUTH_REQUIRED arrives before grace expires and no token is configured', async () => {
    engine = new SyncEngine(config);

    // Flush onopen (0ms timer), which sets the 500ms grace timer.
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    // Simulate AUTH_REQUIRED arriving at t=100ms (before 500ms grace expires).
    jest.advanceTimersByTime(100);
    const ws = MockWebSocket.getLastInstance()!;
    ws.simulateMessage({ type: 'AUTH_REQUIRED' });
    await Promise.resolve();

    // Grace timer should have been cancelled; state should be AUTHENTICATING.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // Advance past the original grace window — state must NOT change to CONNECTED.
    jest.advanceTimersByTime(500);
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // Only the opportunistic NO_AUTH AUTH{token:''} was sent; no credentialed AUTH
    // follows because there is still no token.
    const authFrames = ws.sentMessages.filter((m) => m.type === 'AUTH');
    expect(authFrames).toHaveLength(1);
    expect(authFrames[0]?.token).toBe('');

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 3: No token → AUTH_REQUIRED → setAuthToken → AUTH_ACK → CONNECTED
  // ────────────────────────────────────────────
  it('case 3: transitions to CONNECTED after AUTH_REQUIRED + setAuthToken + AUTH_ACK, with no spurious intermediate states', async () => {
    engine = new SyncEngine(config);

    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    // AUTH_REQUIRED arrives at t=100ms.
    jest.advanceTimersByTime(100);
    const ws = MockWebSocket.getLastInstance()!;
    ws.simulateMessage({ type: 'AUTH_REQUIRED' });
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // History so far must not contain CONNECTED.
    const historyBeforeSetToken = engine.getStateHistory().map((e) => e.to);
    expect(historyBeforeSetToken).not.toContain(SyncState.CONNECTED);

    // setAuthToken must NOT reset to CONNECTING or any other non-AUTHENTICATING state.
    engine.setAuthToken('xyz');
    await Promise.resolve();

    // State must still be AUTHENTICATING immediately after setAuthToken.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // Credentialed AUTH frame must have been sent (after the earlier opportunistic
    // AUTH{token:''} from the NO_AUTH connect path).
    const authFrames = ws.sentMessages.filter((m) => m.type === 'AUTH');
    const credentialedAuth = authFrames[authFrames.length - 1];
    expect(credentialedAuth).toBeDefined();
    expect(credentialedAuth?.token).toBe('xyz');

    // Simulate AUTH_ACK → transitions to CONNECTED.
    ws.simulateMessage({ type: 'AUTH_ACK' });
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    // Full observed sequence via history: [CONNECTING, AUTHENTICATING, SYNCING, CONNECTED].
    // No extra intermediate states between setAuthToken and AUTH_ACK.
    const history = engine.getStateHistory();
    const historyStates = history.map((e) => e.to);
    expect(historyStates).toEqual([
      SyncState.CONNECTING,
      SyncState.AUTHENTICATING,
      SyncState.SYNCING,
      SyncState.CONNECTED,
    ]);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 4: Token configured — sends AUTH immediately, no grace-timer wait
  // ────────────────────────────────────────────
  it('case 4: sends AUTH immediately on WS open when authToken is configured, then reaches CONNECTED on AUTH_ACK', async () => {
    engine = new SyncEngine({
      ...config,
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
    });

    // Set token before connection opens (before onopen fires).
    engine.setAuthToken('xyz');

    // Flush onopen — triggers handleConnectionEstablished which sees the token
    // and sends AUTH immediately without a grace timer.
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    const ws = MockWebSocket.getLastInstance()!;
    const authFrame = ws.sentMessages.find((m) => m.type === 'AUTH');
    expect(authFrame).toBeDefined();
    expect(authFrame?.token).toBe('xyz');

    // State is AUTHENTICATING, waiting for AUTH_ACK.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // No grace timer was scheduled — advancing 500ms must NOT drive to CONNECTED
    // via the auth-optional path (token-configured path skips the timer).
    jest.advanceTimersByTime(500);
    await Promise.resolve();

    // Still AUTHENTICATING because AUTH_ACK hasn't arrived.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    ws.simulateMessage({ type: 'AUTH_ACK' });
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 5: tokenProvider configured — sends AUTH immediately
  // ────────────────────────────────────────────
  it('case 5: sends AUTH immediately when tokenProvider is configured, then reaches CONNECTED on AUTH_ACK', async () => {
    engine = new SyncEngine(config);

    // Set tokenProvider before onopen fires.
    engine.setTokenProvider(async () => 'provider-token');

    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    const ws = MockWebSocket.getLastInstance()!;

    // tokenProvider is async — advance timers and flush promises.
    await jest.advanceTimersByTimeAsync(0);
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    const authFrame = ws.sentMessages.find((m) => m.type === 'AUTH');
    expect(authFrame).toBeDefined();
    expect(authFrame?.token).toBe('provider-token');

    ws.simulateMessage({ type: 'AUTH_ACK' });
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 6: Disconnect during grace window clears timer
  // ────────────────────────────────────────────
  it('case 6: clears grace timer when WS disconnects during the grace window', async () => {
    engine = new SyncEngine(config);

    // Flush onopen — opportunistic AUTH sent + grace timer set (not yet fired).
    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    // NO_AUTH connect path moves to AUTHENTICATING while awaiting AUTH_ACK.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // Disconnect at t=100ms (before grace expires).
    jest.advanceTimersByTime(100);
    const ws = MockWebSocket.getLastInstance()!;
    ws.close(); // triggers handleConnectionLost → clears grace timer
    await Promise.resolve();

    // Advance past the original grace window.
    jest.advanceTimersByTime(500);
    await Promise.resolve();

    // State must NOT be CONNECTED — the timer was cleared on disconnect.
    expect(engine.getConnectionState()).not.toBe(SyncState.CONNECTED);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 7: Full connect → disconnect → reconnect cycle without token
  // ────────────────────────────────────────────
  it('case 7: reaches CONNECTED after reconnect on an auth-optional server (full cycle without token)', async () => {
    engine = new SyncEngine(config);

    // First connection: run all timers (0ms onopen + 500ms grace window).
    await jest.runAllTimersAsync();

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    // Simulate disconnect — provider schedules a 1000ms reconnect timer.
    const ws1 = MockWebSocket.getLastInstance()!;
    ws1.close();
    await Promise.resolve();

    // Run the reconnect timer (1000ms) which creates a new WS and fires onopen (0ms),
    // then the new grace timer (500ms). runAllTimersAsync flushes async promise chains
    // so the await-based connect() in SingleServerProvider resolves properly.
    await jest.runAllTimersAsync();

    // After the reconnect grace window expires, state should be CONNECTED again.
    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    engine.close();
  });

  // ────────────────────────────────────────────
  // Case 8: AUTH_REQUIRED arrives while already in AUTHENTICATING (token-configured path)
  // ────────────────────────────────────────────
  it('case 8: handleAuthRequired does not re-transition to AUTHENTICATING when already in AUTHENTICATING (token-configured path)', async () => {
    // require() accesses the Jest-mocked logger module to spy on the warn method
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const loggerModule = require('../utils/logger');
    const warnSpy = jest.spyOn(loggerModule.logger, 'warn');

    // Construct with token configured — handleConnectionEstablished will
    // transition to AUTHENTICATING and call sendAuth() immediately.
    engine = new SyncEngine({
      ...config,
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
    });
    engine.setAuthToken('xyz');

    // Flush 0ms timers (onopen) without advancing the 500ms grace timer.
    await jest.advanceTimersByTimeAsync(0);
    await Promise.resolve();

    const ws = MockWebSocket.getLastInstance()!;

    // State is AUTHENTICATING after token-configured path.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    const sentCountBefore = ws.sentMessages.filter((m) => m.type === 'AUTH').length;

    // Simulate AUTH_REQUIRED arriving while already in AUTHENTICATING
    // (e.g. server re-auth ping after session setup). The state guard in
    // handleAuthRequired() must prevent AUTHENTICATING → AUTHENTICATING.
    ws.simulateMessage({ type: 'AUTH_REQUIRED' });
    await Promise.resolve();

    // State guard fired — still AUTHENTICATING, no invalid transition.
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    // No "Invalid state transition" warning should have been logged.
    const invalidTransitionWarn = warnSpy.mock.calls.find(
      (args) =>
        typeof args[args.length - 1] === 'string' &&
        (args[args.length - 1] as string).includes('Invalid state transition'),
    );
    expect(invalidTransitionWarn).toBeUndefined();

    // sendAuth() IS called again (re-auth), so an extra AUTH frame is sent.
    const sentCountAfter = ws.sentMessages.filter((m) => m.type === 'AUTH').length;
    expect(sentCountAfter).toBeGreaterThan(sentCountBefore);

    warnSpy.mockRestore();
    engine.close();
  });
});
