import { HLC, LWWMap } from '@topgunbuild/core';
import type { HttpSyncRequest } from '@topgunbuild/core';
import { HttpSyncHandler } from '../http-sync-handler';
import type { HttpSyncHandlerConfig } from '../http-sync-handler';

describe('HttpSyncHandler', () => {
  let hlc: HLC;
  let handler: HttpSyncHandler;
  let mockAuthHandler: any;
  let mockOperationHandler: any;
  let mockStorageManager: any;
  let mockQueryConversionHandler: any;
  let mockSearchCoordinator: any;
  let mockSecurityManager: any;
  let testMap: LWWMap<string, any>;

  const validPrincipal = { userId: 'user-1', roles: ['USER'] };

  beforeEach(() => {
    hlc = new HLC('test-server');
    testMap = new LWWMap<string, any>(hlc);

    mockAuthHandler = {
      verifyToken: jest.fn().mockReturnValue(validPrincipal),
      handleAuth: jest.fn(),
    };

    mockOperationHandler = {
      applyOpToMap: jest.fn().mockResolvedValue({ eventPayload: {}, oldRecord: null }),
      processClientOp: jest.fn(),
      processOpBatch: jest.fn(),
      processLocalOp: jest.fn(),
    };

    mockStorageManager = {
      getMapAsync: jest.fn().mockResolvedValue(testMap),
      getMap: jest.fn().mockReturnValue(testMap),
      getMaps: jest.fn().mockReturnValue(new Map()),
      hasMap: jest.fn().mockReturnValue(false),
      loadMapFromStorage: jest.fn(),
      isMapLoading: jest.fn().mockReturnValue(false),
    };

    mockQueryConversionHandler = {
      executeLocalQuery: jest.fn().mockResolvedValue([]),
      convertToCoreQuery: jest.fn(),
      predicateToCoreQuery: jest.fn(),
      convertOperator: jest.fn(),
      finalizeClusterQuery: jest.fn(),
      stop: jest.fn(),
    };

    mockSearchCoordinator = {
      search: jest.fn().mockReturnValue({ results: [], totalCount: 0, requestId: '' }),
    };

    mockSecurityManager = {
      checkPermission: jest.fn().mockReturnValue(true),
    };

    handler = new HttpSyncHandler({
      authHandler: mockAuthHandler,
      operationHandler: mockOperationHandler,
      storageManager: mockStorageManager,
      queryConversionHandler: mockQueryConversionHandler,
      searchCoordinator: mockSearchCoordinator,
      hlc,
      securityManager: mockSecurityManager,
    });
  });

  function makeRequest(overrides: Partial<HttpSyncRequest> = {}): HttpSyncRequest {
    return {
      clientId: 'client-1',
      clientHlc: { millis: Date.now(), counter: 0, nodeId: 'client-node' },
      ...overrides,
    };
  }

  it('processes operations and returns ack', async () => {
    const request = makeRequest({
      operations: [
        { mapName: 'users', key: 'user-1', id: 'op-1', record: { value: { name: 'Alice' }, timestamp: { millis: 1, counter: 0, nodeId: 'c' } } },
        { mapName: 'users', key: 'user-2', id: 'op-2', record: { value: { name: 'Bob' }, timestamp: { millis: 2, counter: 0, nodeId: 'c' } } },
      ],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.ack).toBeDefined();
    expect(response.ack!.lastId).toBe('op-2');
    expect(response.ack!.results).toHaveLength(2);
    expect(response.ack!.results![0].success).toBe(true);
    expect(mockOperationHandler.applyOpToMap).toHaveBeenCalledTimes(2);
  });

  it('returns deltas for maps with newer records', async () => {
    // Seed the map with records (set() auto-generates HLC timestamps)
    testMap.set('key-1', { name: 'Alice' });
    testMap.set('key-2', { name: 'Bob' });

    // Client's last sync is before any records
    const clientTimestamp = { millis: 0, counter: 0, nodeId: '' };

    const request = makeRequest({
      syncMaps: [{ mapName: 'users', lastSyncTimestamp: clientTimestamp }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.deltas).toBeDefined();
    expect(response.deltas).toHaveLength(1);
    expect(response.deltas![0].mapName).toBe('users');
    expect(response.deltas![0].records).toHaveLength(2);
    expect(response.deltas![0].records[0].eventType).toBe('PUT');
  });

  it('rejects invalid auth token with error', async () => {
    mockAuthHandler.verifyToken.mockImplementation(() => {
      throw new Error('Invalid token');
    });

    const request = makeRequest();

    await expect(handler.handleSyncRequest(request, 'bad-token')).rejects.toThrow('401: Authentication failed');
  });

  it('executes one-shot query and returns results', async () => {
    const queryResults = [{ id: 'user-1', name: 'Alice' }];
    mockQueryConversionHandler.executeLocalQuery.mockResolvedValue(queryResults);

    const request = makeRequest({
      queries: [{ queryId: 'q1', mapName: 'users', filter: { name: 'Alice' } }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.queryResults).toBeDefined();
    expect(response.queryResults).toHaveLength(1);
    expect(response.queryResults![0].queryId).toBe('q1');
    expect(response.queryResults![0].results).toEqual(queryResults);
  });

  it('executes one-shot search and returns results', async () => {
    mockSearchCoordinator.search.mockReturnValue({
      results: [{ key: 'user-1', score: 0.95 }],
      totalCount: 1,
      requestId: '',
    });

    const request = makeRequest({
      searches: [{ searchId: 's1', mapName: 'users', query: 'alice' }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.searchResults).toBeDefined();
    expect(response.searchResults).toHaveLength(1);
    expect(response.searchResults![0].searchId).toBe('s1');
    expect(response.searchResults![0].totalCount).toBe(1);
  });

  it('handles empty request (no ops, no syncMaps) gracefully', async () => {
    const request = makeRequest();
    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.serverHlc).toBeDefined();
    expect(response.ack).toBeUndefined();
    expect(response.deltas).toBeUndefined();
    expect(response.queryResults).toBeUndefined();
    expect(response.searchResults).toBeUndefined();
    expect(response.errors).toBeUndefined();
  });

  it('handles concurrent map access correctly', async () => {
    // Seed the map (set() auto-generates HLC timestamp)
    testMap.set('key-1', { value: 'a' });

    const request1 = makeRequest({
      syncMaps: [{ mapName: 'users', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } }],
    });
    const request2 = makeRequest({
      syncMaps: [{ mapName: 'users', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } }],
    });

    const [response1, response2] = await Promise.all([
      handler.handleSyncRequest(request1, 'valid-token'),
      handler.handleSyncRequest(request2, 'valid-token'),
    ]);

    expect(response1.deltas).toBeDefined();
    expect(response2.deltas).toBeDefined();
    expect(response1.deltas![0].records).toHaveLength(1);
    expect(response2.deltas![0].records).toHaveLength(1);
  });

  it('checks permissions on operations and returns errors for denied ops', async () => {
    mockSecurityManager.checkPermission.mockReturnValue(false);

    const request = makeRequest({
      operations: [
        { mapName: 'secret', key: 'key-1', id: 'op-1', record: { value: { data: 'sensitive' }, timestamp: { millis: 1, counter: 0, nodeId: 'c' } } },
      ],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.errors).toBeDefined();
    expect(response.errors).toHaveLength(1);
    expect(response.errors![0].code).toBe(403);
    expect(mockOperationHandler.applyOpToMap).not.toHaveBeenCalled();
  });

  it('checks permissions on queries', async () => {
    mockSecurityManager.checkPermission.mockReturnValue(false);

    const request = makeRequest({
      queries: [{ queryId: 'q1', mapName: 'secret', filter: {} }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.errors).toBeDefined();
    expect(response.errors!.some(e => e.code === 403)).toBe(true);
    expect(mockQueryConversionHandler.executeLocalQuery).not.toHaveBeenCalled();
  });

  it('returns correct serverSyncTimestamp from hlc.now()', async () => {
    const request = makeRequest();
    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.serverHlc).toBeDefined();
    expect(response.serverHlc.nodeId).toBe('test-server');
    expect(response.serverHlc.millis).toBeGreaterThan(0);
  });

  it('emits REMOVE events for records with null values', async () => {
    // Set a record then tombstone it
    testMap.set('key-1', { name: 'Alice' });
    testMap.remove('key-1');

    const request = makeRequest({
      syncMaps: [{ mapName: 'users', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.deltas).toBeDefined();
    const record = response.deltas![0].records.find(r => r.key === 'key-1');
    expect(record).toBeDefined();
    expect(record!.eventType).toBe('REMOVE');
  });

  it('only returns records newer than client lastSyncTimestamp', async () => {
    // Set old record (auto-generates HLC timestamp)
    testMap.set('old-key', { value: 'old' });

    // Record the midpoint timestamp after old record
    const midTimestamp = hlc.now();

    // Set new record (will have a timestamp after midTimestamp)
    testMap.set('new-key', { value: 'new' });

    const request = makeRequest({
      syncMaps: [{ mapName: 'users', lastSyncTimestamp: midTimestamp }],
    });

    const response = await handler.handleSyncRequest(request, 'valid-token');

    expect(response.deltas).toBeDefined();
    expect(response.deltas![0].records).toHaveLength(1);
    expect(response.deltas![0].records[0].key).toBe('new-key');
  });
});
