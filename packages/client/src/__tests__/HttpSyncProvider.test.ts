import { HLC, serialize, deserialize } from '@topgunbuild/core';
import { HttpSyncProvider } from '../connection/HttpSyncProvider';
import type { HttpSyncProviderConfig } from '../connection/HttpSyncProvider';

describe('HttpSyncProvider', () => {
  let hlc: HLC;
  let mockFetch: jest.Mock;
  let provider: HttpSyncProvider;

  function createMockResponse(body: any, status: number = 200): Response {
    const bodyBytes = serialize(body);
    return {
      ok: status >= 200 && status < 300,
      status,
      statusText: status === 200 ? 'OK' : 'Error',
      headers: new Headers({ 'content-type': 'application/x-msgpack' }),
      arrayBuffer: () => Promise.resolve(bodyBytes.buffer.slice(
        bodyBytes.byteOffset,
        bodyBytes.byteOffset + bodyBytes.byteLength,
      )),
    } as unknown as Response;
  }

  function defaultSyncResponse(overrides: any = {}): any {
    return {
      serverHlc: { millis: Date.now(), counter: 1, nodeId: 'server-1' },
      ...overrides,
    };
  }

  beforeEach(() => {
    hlc = new HLC('client-node');
    mockFetch = jest.fn().mockResolvedValue(createMockResponse(defaultSyncResponse()));

    provider = new HttpSyncProvider({
      url: 'http://localhost:8080',
      clientId: 'test-client',
      hlc,
      authToken: 'test-token',
      pollIntervalMs: 100,
      requestTimeoutMs: 5000,
      syncMaps: ['users'],
      fetchImpl: mockFetch,
    });
  });

  afterEach(async () => {
    await provider.close();
  });

  it('connect() sends initial sync request', async () => {
    await provider.connect();

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/sync',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({
          'Content-Type': 'application/x-msgpack',
          'Authorization': 'Bearer test-token',
        }),
      }),
    );
  });

  it('send() queues OP_BATCH operations for next poll', async () => {
    await provider.connect();

    const opBatchMsg = serialize({
      type: 'OP_BATCH',
      payload: {
        ops: [
          { mapName: 'users', key: 'user-1', record: { value: { name: 'Alice' }, timestamp: hlc.now() } },
        ],
      },
    });

    provider.send(opBatchMsg);

    // Wait for next poll cycle
    await new Promise((r) => setTimeout(r, 150));

    // The second call (first poll after connect) should include the operations
    expect(mockFetch).toHaveBeenCalledTimes(2);
    const secondCallBody = mockFetch.mock.calls[1][1].body;
    const parsed = deserialize<any>(new Uint8Array(secondCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
  });

  it('send() silently ignores AUTH messages', async () => {
    await provider.connect();

    const authMsg = serialize({ type: 'AUTH', token: 'some-token' });
    provider.send(authMsg);

    // No error thrown, no operations queued
    await new Promise((r) => setTimeout(r, 150));

    // Poll should not include any operations
    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeUndefined();
  });

  it('send() silently ignores SYNC_INIT messages', async () => {
    await provider.connect();

    const syncInitMsg = serialize({ type: 'SYNC_INIT', mapName: 'users' });
    provider.send(syncInitMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeUndefined();
  });

  it('send() queues QUERY_SUB as one-shot query', async () => {
    await provider.connect();

    const querySubMsg = serialize({
      type: 'QUERY_SUB',
      payload: {
        requestId: 'q-1',
        mapName: 'users',
        query: { where: { active: true } },
      },
    });
    provider.send(querySubMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.queries).toBeDefined();
    expect(parsed.queries.length).toBe(1);
    expect(parsed.queries[0].queryId).toBe('q-1');
  });

  it('send() queues CLIENT_OP operations for next poll', async () => {
    await provider.connect();

    const clientOpMsg = serialize({
      type: 'CLIENT_OP',
      payload: {
        mapName: 'users',
        key: 'user-2',
        record: { value: { name: 'Bob' }, timestamp: hlc.now() },
      },
    });
    provider.send(clientOpMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
    expect(parsed.operations[0].key).toBe('user-2');
  });

  it('polling loop sends queued ops at interval', async () => {
    await provider.connect();
    expect(mockFetch).toHaveBeenCalledTimes(1);

    // Wait for several poll cycles
    await new Promise((r) => setTimeout(r, 350));

    // Should have made multiple calls (initial + polls)
    expect(mockFetch.mock.calls.length).toBeGreaterThanOrEqual(3);
  });

  it('emits message events for deltas received', async () => {
    const messageHandler = jest.fn();
    provider.on('message', messageHandler);

    mockFetch.mockResolvedValue(createMockResponse(defaultSyncResponse({
      deltas: [{
        mapName: 'users',
        records: [
          { key: 'user-1', record: { value: { name: 'Alice' }, timestamp: hlc.now() }, eventType: 'PUT' },
        ],
        serverSyncTimestamp: hlc.now(),
      }],
    })));

    await provider.connect();

    // One SERVER_EVENT message for the delta record
    expect(messageHandler).toHaveBeenCalled();
  });

  it('emits connected after first successful request', async () => {
    const connectedHandler = jest.fn();
    provider.on('connected', connectedHandler);

    await provider.connect();

    expect(connectedHandler).toHaveBeenCalledWith('http');
  });

  it('emits disconnected on request failure', async () => {
    const disconnectedHandler = jest.fn();
    provider.on('disconnected', disconnectedHandler);

    await provider.connect();

    // Make next request fail
    mockFetch.mockRejectedValue(new Error('Network error'));

    // Wait for poll cycle
    await new Promise((r) => setTimeout(r, 150));

    expect(disconnectedHandler).toHaveBeenCalledWith('http');
  });

  it('emits reconnected when requests succeed again after failure', async () => {
    const reconnectedHandler = jest.fn();
    provider.on('reconnected', reconnectedHandler);

    await provider.connect();

    // Make request fail
    mockFetch.mockRejectedValueOnce(new Error('Network error'));
    await new Promise((r) => setTimeout(r, 150));

    // Make request succeed again
    mockFetch.mockResolvedValue(createMockResponse(defaultSyncResponse()));
    await new Promise((r) => setTimeout(r, 150));

    expect(reconnectedHandler).toHaveBeenCalledWith('http');
  });

  it('isConnected() reflects last request status', async () => {
    expect(provider.isConnected()).toBe(false);

    await provider.connect();
    expect(provider.isConnected()).toBe(true);

    // Simulate failure
    mockFetch.mockRejectedValueOnce(new Error('fail'));
    await new Promise((r) => setTimeout(r, 150));

    expect(provider.isConnected()).toBe(false);
  });

  it('custom fetch implementation is used', async () => {
    const customFetch = jest.fn().mockResolvedValue(createMockResponse(defaultSyncResponse()));
    const customProvider = new HttpSyncProvider({
      url: 'http://custom:9999',
      clientId: 'c1',
      hlc,
      authToken: 'token',
      fetchImpl: customFetch,
    });

    await customProvider.connect();

    expect(customFetch).toHaveBeenCalled();
    expect(customFetch.mock.calls[0][0]).toBe('http://custom:9999/sync');

    await customProvider.close();
  });

  it('getConnectedNodes() returns [http] when connected and [] when disconnected', async () => {
    expect(provider.getConnectedNodes()).toEqual([]);

    await provider.connect();
    expect(provider.getConnectedNodes()).toEqual(['http']);

    await provider.close();
    expect(provider.getConnectedNodes()).toEqual([]);
  });

  it('close() stops polling loop and clears queued operations', async () => {
    await provider.connect();

    // Queue some operations
    provider.send(serialize({
      type: 'CLIENT_OP',
      payload: { mapName: 'users', key: 'k1', record: { value: {}, timestamp: hlc.now() } },
    }));

    await provider.close();

    const callCountAtClose = mockFetch.mock.calls.length;

    // Wait to verify no more polls happen
    await new Promise((r) => setTimeout(r, 200));

    expect(mockFetch.mock.calls.length).toBe(callCountAtClose);
    expect(provider.isConnected()).toBe(false);
  });

  it('getConnection() returns a valid IConnection', () => {
    const conn = provider.getConnection('key');
    expect(conn).toBeDefined();
    expect(typeof conn.send).toBe('function');
    expect(typeof conn.close).toBe('function');
    expect(typeof conn.readyState).toBe('number');
  });

  it('getAnyConnection() returns a valid IConnection', () => {
    const conn = provider.getAnyConnection();
    expect(conn).toBeDefined();
    expect(typeof conn.send).toBe('function');
    expect(typeof conn.close).toBe('function');
    expect(typeof conn.readyState).toBe('number');
  });

  it('getConnection() readyState reflects connected state', async () => {
    // Before connect: should be CLOSED
    const connBefore = provider.getConnection('key');
    expect(connBefore.readyState).toBe(3); // CLOSED

    await provider.connect();

    // After connect: should be OPEN
    const connAfter = provider.getConnection('key');
    expect(connAfter.readyState).toBe(1); // OPEN
  });

  it('getConnection().send() queues operations like provider.send()', async () => {
    await provider.connect();

    const conn = provider.getConnection('key');
    const opBatchMsg = serialize({
      type: 'CLIENT_OP',
      payload: { mapName: 'users', key: 'user-x', record: { value: { name: 'Test' }, timestamp: hlc.now() } },
    });
    conn.send(opBatchMsg);

    // Wait for poll cycle to flush
    await new Promise((r) => setTimeout(r, 150));

    // The poll call should include the queued operation
    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
  });
});
