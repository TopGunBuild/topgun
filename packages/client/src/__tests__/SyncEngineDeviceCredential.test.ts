/**
 * Tests for SyncEngine server-issued device credential handling.
 *
 * A token-less client presents its device credential on a dedicated DEVICE_HELLO
 * frame (NOT an empty-token AUTH, which a real JWT server would AUTH_FAIL + tear
 * down) and receives it back on DEVICE_ACK.
 *
 * Covers:
 *   (a) deviceToken from DEVICE_ACK is persisted and presented on the next DEVICE_HELLO
 *   (b) DEVICE_ACK without a deviceToken keeps the existing (persisted) token
 *   (c) legacy server (no DEVICE_ACK, or a non-DEVICE_ACK message first) proceeds
 *       without a device identity and never errors/hangs
 */

import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { SyncState } from '../SyncState';

// ──────────────────────────────────────────────
// Mock WebSocket (same pattern as SyncEngineAuthOptional.test.ts)
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

  deviceHelloFrames(): any[] {
    return this.sentMessages.filter((m) => m.type === 'DEVICE_HELLO');
  }

  authFrames(): any[] {
    return this.sentMessages.filter((m) => m.type === 'AUTH');
  }
}

(global as any).WebSocket = MockWebSocket;

// ──────────────────────────────────────────────
// Mock Storage Adapter with an in-memory meta store so persisted device
// credentials survive across engine instances within a test.
// ──────────────────────────────────────────────
function createMockStorageAdapter(
  meta: Map<string, any> = new Map(),
): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getMeta: jest.fn().mockImplementation(async (key: string) => meta.get(key)),
    setMeta: jest.fn().mockImplementation(async (key: string, value: any) => {
      meta.set(key, value);
    }),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn().mockResolvedValue([]),
    getAllMetaKeys: jest.fn().mockImplementation(async () => Array.from(meta.keys())),
  } as unknown as jest.Mocked<IStorageAdapter>;
}

let uuidCounter = 0;
(global as any).crypto = {
  randomUUID: () => `test-uuid-${++uuidCounter}`,
};

describe('SyncEngine — server-issued device credential', () => {
  let engine: SyncEngine | undefined;

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.reset();
    uuidCounter = 0;
  });

  afterEach(() => {
    if (engine) {
      engine.close();
      engine = undefined;
    }
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  function makeConfig(meta?: Map<string, any>): {
    config: SyncEngineConfig;
    storage: jest.Mocked<IStorageAdapter>;
  } {
    const storage = createMockStorageAdapter(meta);
    const config: SyncEngineConfig = {
      nodeId: 'test-node',
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
      storageAdapter: storage,
      heartbeat: { enabled: false },
    };
    return { config, storage };
  }

  // ────────────────────────────────────────────
  // (a) deviceToken minted on DEVICE_ACK is persisted and re-presented
  // ────────────────────────────────────────────
  it('(a) persists a minted deviceToken from DEVICE_ACK and presents it on the next DEVICE_HELLO', async () => {
    const meta = new Map<string, any>();
    const { config, storage } = makeConfig(meta);
    engine = new SyncEngine(config);

    // Open: token-less connect sends a DEVICE_HELLO with no deviceToken yet — and
    // NOT an empty-token AUTH (which a JWT server would reject).
    await jest.advanceTimersByTimeAsync(0);

    const ws1 = MockWebSocket.getLastInstance()!;
    expect(ws1.authFrames()).toHaveLength(0);
    const firstHello = ws1.deviceHelloFrames()[0];
    expect(firstHello).toBeDefined();
    expect(firstHello?.deviceToken).toBeUndefined();

    // Server mints a device identity and returns it on DEVICE_ACK.
    ws1.simulateMessage({
      type: 'DEVICE_ACK',
      deviceId: 'dev-1',
      deviceToken: 'dev-1.secretHex',
    });
    await jest.advanceTimersByTimeAsync(0);

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    // Credential persisted durably.
    expect(storage.setMeta).toHaveBeenCalledWith('deviceToken', 'dev-1.secretHex');
    expect(storage.setMeta).toHaveBeenCalledWith('deviceId', 'dev-1');
    expect(meta.get('deviceToken')).toBe('dev-1.secretHex');

    // Reconnect: the persisted/in-memory deviceToken must now be presented.
    ws1.close();
    await Promise.resolve();
    await jest.runAllTimersAsync();

    const ws2 = MockWebSocket.getLastInstance()!;
    expect(ws2).not.toBe(ws1);
    const reconnectHello = ws2.deviceHelloFrames()[0];
    expect(reconnectHello).toBeDefined();
    expect(reconnectHello?.deviceToken).toBe('dev-1.secretHex');
  });

  // ────────────────────────────────────────────
  // (b) DEVICE_ACK without deviceToken keeps the existing token
  // ────────────────────────────────────────────
  it('(b) keeps the existing persisted token when DEVICE_ACK carries no deviceToken (re-bind)', async () => {
    const meta = new Map<string, any>([
      ['deviceToken', 'existing.tok'],
      ['deviceId', 'existing-dev'],
    ]);
    const { config, storage } = makeConfig(meta);
    engine = new SyncEngine(config);

    // loadOpLog reads the persisted credential before the socket opens; the first
    // DEVICE_HELLO must present it.
    await jest.advanceTimersByTimeAsync(0);

    const ws1 = MockWebSocket.getLastInstance()!;
    const firstHello = ws1.deviceHelloFrames()[0];
    expect(firstHello?.deviceToken).toBe('existing.tok');

    storage.setMeta.mockClear();

    // Server re-binds the already-valid token: DEVICE_ACK with deviceId but NO deviceToken.
    ws1.simulateMessage({ type: 'DEVICE_ACK', deviceId: 'existing-dev' });
    await jest.advanceTimersByTimeAsync(0);

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);

    // deviceToken must not be overwritten/cleared.
    const tokenWrites = storage.setMeta.mock.calls.filter((c) => c[0] === 'deviceToken');
    expect(tokenWrites).toHaveLength(0);
    expect(meta.get('deviceToken')).toBe('existing.tok');

    // Reconnect still presents the original token.
    ws1.close();
    await Promise.resolve();
    await jest.runAllTimersAsync();

    const ws2 = MockWebSocket.getLastInstance()!;
    expect(ws2.deviceHelloFrames()[0]?.deviceToken).toBe('existing.tok');
  });

  // ────────────────────────────────────────────
  // (c1) legacy server sends no DEVICE_ACK → proceeds via grace timeout
  // ────────────────────────────────────────────
  it('(c1) proceeds to CONNECTED without a device identity when a legacy server never sends DEVICE_ACK', async () => {
    const meta = new Map<string, any>();
    const { config, storage } = makeConfig(meta);
    engine = new SyncEngine(config);

    await jest.advanceTimersByTimeAsync(0);

    // No DEVICE_ACK arrives; grace window elapses.
    jest.advanceTimersByTime(500);
    await Promise.resolve();

    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
    // No device identity was persisted.
    expect(storage.setMeta).not.toHaveBeenCalledWith('deviceId', expect.anything());
    expect(storage.setMeta).not.toHaveBeenCalledWith('deviceToken', expect.anything());
  });

  // ────────────────────────────────────────────
  // (c2) legacy server sends a non-DEVICE_ACK message first → message-first inference
  // ────────────────────────────────────────────
  it('(c2) infers a legacy server from a non-DEVICE_ACK first message and proceeds before the grace timeout', async () => {
    const meta = new Map<string, any>();
    const { config, storage } = makeConfig(meta);
    engine = new SyncEngine(config);

    await jest.advanceTimersByTimeAsync(0);

    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    const ws1 = MockWebSocket.getLastInstance()!;
    // A legacy server that predates device identity emits a non-DEVICE_ACK frame.
    ws1.simulateMessage({ type: 'PONG' });
    await Promise.resolve();

    // Proceeded to CONNECTED without waiting for the 500ms grace timeout.
    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
    expect(storage.setMeta).not.toHaveBeenCalledWith('deviceId', expect.anything());
  });
});
