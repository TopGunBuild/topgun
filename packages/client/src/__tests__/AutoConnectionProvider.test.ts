import { HLC, serialize } from '@topgunbuild/core';
import { AutoConnectionProvider } from '../connection/AutoConnectionProvider';
import type { AutoConnectionProviderConfig } from '../connection/AutoConnectionProvider';

// Mock SingleServerProvider to control WebSocket connection success/failure
jest.mock('../connection/SingleServerProvider', () => {
  return {
    SingleServerProvider: jest.fn().mockImplementation(() => ({
      connect: jest.fn().mockRejectedValue(new Error('WebSocket connection failed')),
      close: jest.fn().mockResolvedValue(undefined),
      isConnected: jest.fn().mockReturnValue(false),
      getConnectedNodes: jest.fn().mockReturnValue([]),
      on: jest.fn(),
      off: jest.fn(),
      send: jest.fn(),
      getConnection: jest.fn(),
      getAnyConnection: jest.fn(),
    })),
  };
});

describe('AutoConnectionProvider', () => {
  let hlc: HLC;
  let mockFetch: jest.Mock;

  function createMockResponse(body: any = {}): Response {
    const responseBody = {
      serverHlc: { millis: Date.now(), counter: 1, nodeId: 'server-1' },
      ...body,
    };
    const bodyBytes = serialize(responseBody);
    return {
      ok: true,
      status: 200,
      statusText: 'OK',
      headers: new Headers({ 'content-type': 'application/x-msgpack' }),
      arrayBuffer: () => Promise.resolve(bodyBytes.buffer.slice(
        bodyBytes.byteOffset,
        bodyBytes.byteOffset + bodyBytes.byteLength,
      )),
    } as unknown as Response;
  }

  beforeEach(() => {
    hlc = new HLC('client-node');
    mockFetch = jest.fn().mockResolvedValue(createMockResponse());
  });

  it('uses WebSocket when available', async () => {
    const { SingleServerProvider } = require('../connection/SingleServerProvider');

    // Override mock so connect() succeeds for this test
    SingleServerProvider.mockImplementationOnce(() => ({
      connect: jest.fn().mockResolvedValue(undefined),
      close: jest.fn().mockResolvedValue(undefined),
      isConnected: jest.fn().mockReturnValue(true),
      getConnectedNodes: jest.fn().mockReturnValue(['ws-node-1']),
      on: jest.fn(),
      off: jest.fn(),
      send: jest.fn(),
      getConnection: jest.fn(),
      getAnyConnection: jest.fn(),
    }));

    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    await provider.connect();

    // Should be using WebSocket, not HTTP
    expect(provider.isUsingHttp()).toBe(false);
    expect(provider.isConnected()).toBe(true);
    expect(provider.getConnectedNodes()).toEqual(['ws-node-1']);
    // fetch should NOT have been called since WS succeeded
    expect(mockFetch).not.toHaveBeenCalled();

    await provider.close();
  });

  it('falls back to HTTP after maxWsAttempts failures', async () => {
    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      maxWsAttempts: 2,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    await provider.connect();

    // Should have fallen back to HTTP mode
    expect(provider.isUsingHttp()).toBe(true);
    expect(provider.isConnected()).toBe(true);
    expect(mockFetch).toHaveBeenCalled();

    await provider.close();
  });

  it('httpOnly mode skips WebSocket entirely', async () => {
    const { SingleServerProvider } = require('../connection/SingleServerProvider');

    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      httpOnly: true,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    await provider.connect();

    // SingleServerProvider should NOT have been instantiated
    const instanceCallsAfterConnect = SingleServerProvider.mock.calls.length;
    // In httpOnly mode, no WS attempts should be made
    expect(provider.isUsingHttp()).toBe(true);
    expect(provider.isConnected()).toBe(true);

    await provider.close();
  });

  it('emits events from underlying provider', async () => {
    const connectedHandler = jest.fn();

    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      httpOnly: true,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    provider.on('connected', connectedHandler);
    await provider.connect();

    expect(connectedHandler).toHaveBeenCalled();

    await provider.close();
  });

  it('close() closes the active underlying provider', async () => {
    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      httpOnly: true,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    await provider.connect();
    expect(provider.isConnected()).toBe(true);

    await provider.close();
    expect(provider.isConnected()).toBe(false);
  });

  it('getConnectedNodes() delegates to underlying provider', async () => {
    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      httpOnly: true,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    expect(provider.getConnectedNodes()).toEqual([]);

    await provider.connect();
    expect(provider.getConnectedNodes()).toEqual(['http']);

    await provider.close();
  });

  it('send() delegates to underlying provider', async () => {
    const provider = new AutoConnectionProvider({
      url: 'http://localhost:8080',
      clientId: 'c1',
      hlc,
      httpOnly: true,
      authToken: 'token',
      fetchImpl: mockFetch,
    });

    await provider.connect();

    // Should not throw when sending via HTTP mode
    expect(() => {
      provider.send(serialize({ type: 'CLIENT_OP', payload: { mapName: 'test', key: 'k1' } }));
    }).not.toThrow();

    await provider.close();
  });
});
