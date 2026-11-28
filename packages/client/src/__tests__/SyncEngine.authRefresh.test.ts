/**
 * Principal switch, proactive in-band token refresh and the expiry guard of
 * SyncEngine, against a scripted server socket under fake timers.
 */
import { serialize, deserialize } from '@topgunbuild/core';
import type { WriteRejection } from '@topgunbuild/core';
import { SyncEngine } from '../SyncEngine';
import type { IStorageAdapter } from '../IStorageAdapter';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { SyncState } from '../SyncState';
import { QueryHandle } from '../QueryHandle';
import { logger } from '../utils/logger';

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- decoded client frames of every type
  sent: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => this.onopen?.(), 0);
  }

  send(data: Uint8Array) {
    if (this.readyState !== MockWebSocket.OPEN) throw new Error('WebSocket is not open');
    this.sent.push(deserialize(data));
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: 'Normal closure' });
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- any server frame
  deliver(message: any) {
    this.onmessage?.({ data: new Uint8Array(serialize(message)).buffer });
  }

  auths(): string[] {
    return this.sent.filter((m) => m.type === 'AUTH').map((m) => m.token);
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- replacing the global constructor
(global as any).WebSocket = MockWebSocket;

function storage(pending: unknown[] = []): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getMeta: jest
      .fn()
      .mockImplementation((key: string) =>
        Promise.resolve(key === '__sys__:ormapBackfillDone' ? true : undefined),
      ),
    setMeta: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue(pending),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn().mockResolvedValue([]),
    getAllMetaKeys: jest.fn().mockResolvedValue([]),
  } as unknown as jest.Mocked<IStorageAdapter>;
}

const NOW = Date.UTC(2026, 10, 28, 12, 0, 0);
const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url');
/** A token for `sub` whose `exp` lies `inSeconds` from the fake clock's start. */
const jwt = (sub: string, inSeconds: number) =>
  `${part({ alg: 'HS256' })}.${part({ sub, exp: Math.floor(NOW / 1000) + inSeconds })}.sig`;
/** A token for `sub` whose `exp` lies `inSeconds` from the fake clock's present. */
const jwtFromNow = (sub: string, inSeconds: number) =>
  `${part({ alg: 'HS256' })}.${part({ sub, exp: Math.floor(Date.now() / 1000) + inSeconds })}.sig`;

let engine: SyncEngine | undefined;
let states: SyncState[];

function sockets(): MockWebSocket[] {
  return MockWebSocket.instances;
}
function last(): MockWebSocket {
  return sockets()[sockets().length - 1];
}

async function settle(ms = 0): Promise<void> {
  await jest.advanceTimersByTimeAsync(ms);
}

function createEngine(pending: unknown[] = [], onAuthRequired?: () => void): SyncEngine {
  engine = new SyncEngine({
    nodeId: 'test-node',
    connectionProvider: new SingleServerProvider({
      url: 'ws://localhost:8080',
      listenNetworkEvents: false,
    }),
    storageAdapter: storage(pending),
    heartbeat: { enabled: false },
    onAuthRequired,
  });
  engine.onConnectionStateChange((event) => states.push(event.to));
  return engine;
}

/** Opens the next socket the engine dials and lets its handshake AUTH go out. */
async function opened(): Promise<MockWebSocket> {
  // One tick, not zero: a socket dialled at the very end of an earlier advance
  // is opened only by a later timer turn.
  await settle(1);
  const ws = last();
  ws.deliver({ type: 'AUTH_REQUIRED' });
  await settle(0);
  return ws;
}

/** An engine with `provider` whose first socket the server has admitted. */
async function admitted(provider: () => Promise<string | null>, pending: unknown[] = []) {
  createEngine(pending).setTokenProvider(provider);
  const ws = await opened();
  ws.deliver({ type: 'AUTH_ACK' });
  await settle(0);
  expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
  return ws;
}

/** Waits until the engine has dialled `count` sockets in all, at most `withinMs`. */
async function socketCountWithin(count: number, withinMs: number): Promise<number> {
  for (let waited = 0; waited < withinMs && sockets().length < count; waited += 50) {
    await settle(50);
  }
  return sockets().length;
}

/** A provider that answers each call with the next token of `tokens`, repeating the last. */
function sequence(...tokens: (string | null)[]) {
  let i = 0;
  return jest.fn(async () => tokens[Math.min(i++, tokens.length - 1)]);
}

beforeEach(() => {
  jest.useFakeTimers({ now: NOW });
  // Every backoff runs at its middle: the reconnect waits are then exact.
  jest.spyOn(Math, 'random').mockReturnValue(0.5);
  MockWebSocket.instances = [];
  states = [];
});

afterEach(() => {
  engine?.close();
  engine = undefined;
  jest.useRealTimers();
  jest.restoreAllMocks();
});

describe('one handshake AUTH per socket', () => {
  test('AUTH_REQUIRED adds no AUTH, and an AUTH_ACK with no refresh in flight changes nothing', async () => {
    const provider = sequence(jwt('g1', 900));
    const ws = await admitted(provider);
    expect(ws.auths()).toHaveLength(1);
    expect(provider).toHaveBeenCalledTimes(1);

    states = [];
    ws.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(states).toEqual([]);
    expect(ws.sent.filter((m) => m.type === 'OP_BATCH')).toHaveLength(0);
  });
});

describe('switching principal on an open socket', () => {
  test('setAuthToken on a connected engine reconnects', async () => {
    const ws = await admitted(sequence(jwt('g1', 900)));
    states = [];
    engine!.setAuthToken(jwt('u1', 900));

    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    next.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(next.auths()).toEqual([jwt('u1', 900)]);
    expect(ws.auths()).toEqual([jwt('g1', 900)]);
    expect(states).toEqual([
      SyncState.DISCONNECTED,
      SyncState.CONNECTING,
      SyncState.AUTHENTICATING,
      SyncState.SYNCING,
      SyncState.CONNECTED,
    ]);
  });

  test("setTokenProvider on a connected engine reconnects with the provider's token", async () => {
    await admitted(sequence(jwt('g1', 900)));
    states = [];
    const userProvider = sequence(jwt('u1', 900));
    engine!.setTokenProvider(userProvider);

    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    next.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(userProvider).toHaveBeenCalledTimes(1);
    expect(next.auths()).toEqual([jwt('u1', 900)]);
    expect(states).toEqual([
      SyncState.DISCONNECTED,
      SyncState.CONNECTING,
      SyncState.AUTHENTICATING,
      SyncState.SYNCING,
      SyncState.CONNECTED,
    ]);
  });

  test('setAuthToken after the handshake AUTH went out, before its answer, reconnects', async () => {
    createEngine().setAuthToken(jwt('g1', 900));
    const ws = await opened();
    expect(engine!.getConnectionState()).toBe(SyncState.AUTHENTICATING);

    engine!.setAuthToken(jwt('u1', 900));
    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    expect(ws.auths()).toEqual([jwt('g1', 900)]);
    expect(next.auths()).toEqual([jwt('u1', 900)]);
  });

  test('a held guest write refused after the switch is reported', async () => {
    const op = {
      id: '1',
      mapName: 'acme.visitor.g1.notes',
      opType: 'PUT',
      key: 'n1',
      synced: false,
      record: {
        value: { text: 'hi' },
        timestamp: { millis: 1000, counter: 0, nodeId: 'test-node' },
      },
      timestamp: { millis: 1000, counter: 0, nodeId: 'test-node' },
    };
    const ws = await admitted(sequence(jwt('g1', 900)), [op]);
    await settle(0);
    // The server holds the OP_ACK: the write stays pending while CONNECTED.
    expect(ws.sent.filter((m) => m.type === 'OP_BATCH')).toHaveLength(1);

    const rejected: WriteRejection[] = [];
    engine!.onWriteRejected((r) => rejected.push(r));
    engine!.setTokenProvider(sequence(jwt('u1', 900)));
    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    next.deliver({ type: 'AUTH_ACK' });
    await settle(0);

    const replay = next.sent.filter((m) => m.type === 'OP_BATCH');
    expect(replay).toHaveLength(1);
    expect(replay[0].payload.ops.map((o: { id: string }) => o.id)).toEqual(['1']);
    next.deliver({
      type: 'OP_REJECTED',
      payload: { opId: '1', reason: 'write forbidden by policy', code: 403, permanent: true },
    });
    await settle(0);
    expect(rejected).toHaveLength(1);
    expect(rejected[0]).toMatchObject({
      id: '1',
      mapName: 'acme.visitor.g1.notes',
      key: 'n1',
      cause: 'forbidden',
    });
  });

  test('a provider result that resolves after the provider changed is never sent', async () => {
    let releaseGuest: (token: string) => void = () => undefined;
    const guestProvider = jest.fn(
      () => new Promise<string | null>((resolve) => (releaseGuest = resolve)),
    );
    createEngine().setTokenProvider(guestProvider);
    const ws = await opened();
    expect(guestProvider).toHaveBeenCalledTimes(1);

    engine!.setTokenProvider(sequence(jwt('u1', 900)));
    await settle(0);
    releaseGuest(jwt('g1', 900));
    await settle(0);
    ws.deliver({ type: 'AUTH_ACK' });
    await settle(0);

    expect(sockets().flatMap((s) => s.auths())).toEqual([jwt('u1', 900)]);
  });

  test('a provider result that resolves after its socket was lost is never sent', async () => {
    let releaseFirst: (token: string) => void = () => undefined;
    let calls = 0;
    const provider = jest.fn(() => {
      calls += 1;
      return calls === 1
        ? new Promise<string | null>((resolve) => (releaseFirst = resolve))
        : Promise.resolve(jwt('g1', 900));
    });
    createEngine().setTokenProvider(provider);
    const first = await opened();
    first.close();
    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const second = await opened();
    expect(second.auths()).toEqual([jwt('g1', 900)]);

    releaseFirst(jwt('g1', 600));
    await settle(0);
    expect(sockets().flatMap((s) => s.auths())).toEqual([jwt('g1', 900)]);
  });

  test('a provider switch resets the confirmed-apply high-water mark; a refresh does not', async () => {
    const ws = await admitted(sequence(jwt('g1', 20), jwt('g1', 900)));
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the mark is engine-private
    const mark = () => (engine as any).lastAckedEpoch as number;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- seeding the engine-private mark
    (engine as any).lastAckedEpoch = 7;

    await settle(30_000);
    expect(ws.auths()).toEqual([jwt('g1', 20), jwt('g1', 900)]);
    ws.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(mark()).toBe(7);

    engine!.setTokenProvider(sequence(jwt('u1', 900)));
    expect(mark()).toBe(0);
  });
});

describe('proactive in-band refresh', () => {
  test('refresh timer respects the floor', async () => {
    // R = 15 min fires at R − 60 s.
    let provider = sequence(jwt('g1', 900), jwt('g1', 1_800));
    let ws = await admitted(provider);
    // R − 60 s after the acknowledgement is 60 s before the token's exp.
    await settle(NOW + 840_000 - Date.now() - 1);
    expect(provider).toHaveBeenCalledTimes(1);
    await settle(1);
    expect(provider).toHaveBeenCalledTimes(2);
    expect(ws.auths()).toEqual([jwt('g1', 900), jwt('g1', 1_800)]);
    engine!.close();

    // R = 40 s and R = −20 s fire at the 30 s floor.
    for (const inSeconds of [40, -20]) {
      provider = sequence(jwtFromNow('g1', inSeconds), jwtFromNow('g1', inSeconds + 600));
      ws = await admitted(provider);
      await settle(30_000 - 1);
      expect(provider).toHaveBeenCalledTimes(1);
      await settle(1);
      expect(provider).toHaveBeenCalledTimes(2);
      expect(ws.auths()).toHaveLength(2);
      engine!.close();
    }

    // An identical token or a provider that throws sends nothing and re-arms no
    // sooner than 30 s.
    const token = jwtFromNow('g1', 40);
    let calls = 0;
    provider = jest.fn(async () => {
      calls += 1;
      if (calls === 3) throw new Error('token endpoint down');
      return token;
    });
    ws = await admitted(provider);
    await settle(30_000);
    expect(provider).toHaveBeenCalledTimes(2);
    await settle(30_000 - 1);
    expect(provider).toHaveBeenCalledTimes(2);
    await settle(1);
    expect(provider).toHaveBeenCalledTimes(3);
    await settle(30_000 - 1);
    expect(provider).toHaveBeenCalledTimes(3);
    await settle(1);
    expect(provider).toHaveBeenCalledTimes(4);
    expect(ws.auths()).toEqual([token]);
  });

  test('refresh ack does not bounce state', async () => {
    const provider = sequence(jwt('g1', -20), jwt('g1', 900));
    const ws = await admitted(provider);
    new QueryHandle(engine!, 'acme.conv.g1', {}).subscribe(() => undefined);
    await settle(0);
    const subs = ws.sent.filter((m) => m.type === 'QUERY_SUB').length;
    states = [];

    await settle(30_000);
    expect(ws.auths()).toEqual([jwt('g1', -20), jwt('g1', 900)]);
    ws.deliver({ type: 'AUTH_ACK' });
    await settle(0);

    expect(states).toEqual([]);
    expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
    expect(ws.sent.filter((m) => m.type === 'QUERY_SUB')).toHaveLength(subs);
    expect(sockets()).toHaveLength(1);
  });

  test('a provider switch keeps proactive refresh', async () => {
    await admitted(sequence(jwt('g1', 900)));
    const p2 = sequence(jwt('u1', 900), jwt('u1', 1_800));
    engine!.setTokenProvider(p2);
    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    next.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(p2).toHaveBeenCalledTimes(1);

    // The new token's exp is 900 s after the fake clock's start; the refresh
    // fires before it.
    await settle(NOW + 900_000 - Date.now() - 1);
    expect(p2).toHaveBeenCalledTimes(2);
    expect(next.auths()).toEqual([jwt('u1', 900), jwt('u1', 1_800)]);
  });

  test('an unanswered refresh keeps the socket', async () => {
    const warn = jest.spyOn(logger, 'warn');
    const provider = sequence(jwt('g1', 40), jwt('g1', 900), jwt('g1', 1_800));
    const ws = await admitted(provider);
    await settle(30_000);
    expect(ws.auths()).toHaveLength(2);

    await settle(10_000);
    expect(
      warn.mock.calls.filter((c) =>
        String(c[c.length - 1]).includes('did not answer the token refresh'),
      ),
    ).toHaveLength(1);
    await settle(600_000);
    expect(ws.readyState).toBe(MockWebSocket.OPEN);
    expect(sockets()).toHaveLength(1);
    expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
    expect(ws.auths()).toHaveLength(2);
    expect(provider).toHaveBeenCalledTimes(2);
  });
});

describe('the expiry guard', () => {
  test('an expired token is not resent', async () => {
    const stale = jwt('g1', -20);
    const older = jwt('g1', -30);
    const provider = sequence(stale);
    createEngine().setTokenProvider(provider);
    const ws = await opened();
    expect(ws.auths()).toEqual([stale]);
    ws.deliver({ type: 'AUTH_FAIL', error: 'token expired', code: 4401 });
    await settle(0);
    ws.close();

    const callsAtRefusal = provider.mock.calls.length;
    await settle(20_000);
    expect(provider.mock.calls.length - callsAtRefusal).toBeGreaterThanOrEqual(2);
    // No socket is opened, and no AUTH carries the refused token, while the
    // provider keeps returning it.
    expect(sockets()).toHaveLength(1);
    provider.mockImplementation(async () => older);
    await settle(60_000);
    expect(sockets()).toHaveLength(1);
    expect(sockets().flatMap((s) => s.auths())).toEqual([stale]);

    const fresh = jwt('g1', 900);
    provider.mockImplementation(async () => fresh);
    expect(await socketCountWithin(2, 31_000)).toBe(2);
    const next = await opened();
    next.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(next.auths()).toEqual([fresh]);
    expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
  });

  test('a static token refused as expired with no provider is not resent and asks for one', async () => {
    const onAuthRequired = jest.fn();
    createEngine([], onAuthRequired);
    const stale = jwt('u1', -20);
    engine!.setAuthToken(stale);
    const ws = await opened();
    ws.deliver({ type: 'AUTH_FAIL', error: 'token expired', code: 4401 });
    await settle(0);
    ws.close();

    expect(await socketCountWithin(2, 2_000)).toBe(2);
    await opened();
    expect(onAuthRequired).toHaveBeenCalled();
    expect(sockets().flatMap((s) => s.auths())).toEqual([stale]);
  });

  test('a generic refusal does not trigger the expiry guard', async () => {
    const token = jwt('g1', 900);
    const provider = sequence(token);
    createEngine().setTokenProvider(provider);
    for (let attempt = 1; attempt <= 2; attempt += 1) {
      const ws = await opened();
      expect(ws.auths()).toEqual([token]);
      ws.deliver({ type: 'AUTH_FAIL', error: 'Authentication failed' });
      await settle(0);
      ws.close();
      expect(await socketCountWithin(attempt + 1, 10_000)).toBe(attempt + 1);
    }
    const ws = await opened();
    expect(ws.auths()).toEqual([token]);
    ws.deliver({ type: 'AUTH_ACK' });
    await settle(0);
    expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
  });

  test('an expiry refusal while a refresh is in flight is charged to the admitted token', async () => {
    const admittedToken = jwt('g1', 40);
    const refreshToken = jwt('g1', 900);
    const ws = await admitted(sequence(admittedToken, refreshToken));
    await settle(30_000);
    expect(ws.auths()).toEqual([admittedToken, refreshToken]);

    // The connection's deadline passes before the refresh is acknowledged.
    ws.deliver({ type: 'AUTH_FAIL', error: 'token expired', code: 4401 });
    await settle(0);
    ws.close();

    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    expect(next.auths()).toEqual([refreshToken]);
  });

  test('a refresh naming another principal keeps its token and reconnects at once', async () => {
    const other = jwt('u1', 900);
    const ws = await admitted(sequence(jwt('g1', 40), other, null));
    await settle(30_000);
    expect(ws.auths()).toEqual([jwt('g1', 40), other]);

    ws.deliver({
      type: 'AUTH_FAIL',
      error: 'the token names another principal than the connection',
      code: 4409,
    });
    await settle(0);

    expect(await socketCountWithin(2, 2_000)).toBe(2);
    const next = await opened();
    expect(next.auths()).toEqual([other]);
  });
});
