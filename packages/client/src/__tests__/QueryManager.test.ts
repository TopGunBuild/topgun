import { QueryManager } from '../sync/QueryManager';
import type { QueryManagerConfig } from '../sync/types';
import type { IStorageAdapter } from '../IStorageAdapter';
import { QueryHandle } from '../QueryHandle';
import { HybridQueryHandle } from '../HybridQueryHandle';
import { SyncEngine } from '../SyncEngine';

// Mock SyncEngine for QueryHandle/HybridQueryHandle
const mockSyncEngine = {
  subscribeToQuery: jest.fn(),
  unsubscribeFromQuery: jest.fn(),
  runLocalQuery: jest.fn().mockResolvedValue([]),
  subscribeToHybridQuery: jest.fn(),
  unsubscribeFromHybridQuery: jest.fn(),
  runLocalHybridQuery: jest.fn().mockResolvedValue([]),
} as unknown as SyncEngine;

// --- Mock Storage Adapter ---
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
    getAllKeys: jest.fn().mockResolvedValue([]),
  };
}

describe('QueryManager', () => {
  let queryManager: QueryManager;
  let mockStorage: jest.Mocked<IStorageAdapter>;
  let mockSendMessage: jest.Mock;
  let mockIsAuthenticated: jest.Mock;
  let config: QueryManagerConfig;

  beforeEach(() => {
    mockStorage = createMockStorageAdapter();
    mockSendMessage = jest.fn().mockReturnValue(true);
    mockIsAuthenticated = jest.fn().mockReturnValue(false);

    config = {
      storageAdapter: mockStorage,
      sendMessage: mockSendMessage,
      isAuthenticated: mockIsAuthenticated,
    };

    queryManager = new QueryManager(config);
  });

  describe('Standard Queries', () => {
    it('should subscribe to a query and add it to the map', () => {
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToQuery(query);

      expect(queryManager.getQueries().has(query.id)).toBe(true);
    });

    it('should send subscription message when authenticated', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', { where: { status: 'active' } });

      queryManager.subscribeToQuery(query);

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: {
          queryId: query.id,
          mapName: 'test-map',
          query: { where: { status: 'active' } },
        },
      });
    });

    it('should not send subscription message when not authenticated', () => {
      mockIsAuthenticated.mockReturnValue(false);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToQuery(query);

      expect(mockSendMessage).not.toHaveBeenCalled();
    });

    it('should unsubscribe from a query and remove it from the map', () => {
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToQuery(query);

      queryManager.unsubscribeFromQuery(query.id);

      expect(queryManager.getQueries().has(query.id)).toBe(false);
    });

    it('should send unsubscription message when authenticated', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query = new QueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToQuery(query);
      mockSendMessage.mockClear();

      queryManager.unsubscribeFromQuery(query.id);

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_UNSUB',
        payload: { queryId: query.id },
      });
    });
  });

  describe('Hybrid Queries', () => {
    it('should subscribe to a hybrid query and add it to the map', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});

      queryManager.subscribeToHybridQuery(query);

      expect(queryManager.getHybridQueries().has(query.id)).toBe(true);
    });

    it('should get a hybrid query by ID', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToHybridQuery(query);

      const result = queryManager.getHybridQuery(query.id);

      expect(result).toBe(query);
    });

    it('should unsubscribe from a hybrid query and remove it from the map', () => {
      const query = new HybridQueryHandle<any>(mockSyncEngine, 'test-map', {});
      queryManager.subscribeToHybridQuery(query);

      queryManager.unsubscribeFromHybridQuery(query.id);

      expect(queryManager.getHybridQueries().has(query.id)).toBe(false);
    });
  });

  describe('runLocalQuery', () => {
    it('should return matching records from storage', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:1', 'users:2', 'users:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'users:1': { value: { name: 'Alice', status: 'active' } },
          'users:2': { value: { name: 'Bob', status: 'inactive' } },
          'users:3': { value: { name: 'Charlie', status: 'active' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalQuery('users', { where: { status: 'active' } });

      expect(results).toHaveLength(2);
      expect(results[0].key).toBe('1');
      expect(results[0].value.name).toBe('Alice');
      expect(results[1].key).toBe('3');
      expect(results[1].value.name).toBe('Charlie');
    });

    it('should return empty array when no records match', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:1']);
      mockStorage.get.mockResolvedValue({ value: { status: 'inactive' } });

      const results = await queryManager.runLocalQuery('users', { where: { status: 'active' } });

      expect(results).toHaveLength(0);
    });

    it('should filter by predicate', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { price: 50 } },
          'items:2': { value: { price: 150 } },
          'items:3': { value: { price: 200 } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalQuery('items', {
        predicate: { op: 'gt', attribute: 'price', value: 100 },
      });

      expect(results).toHaveLength(2);
      expect(results[0].value.price).toBe(150);
      expect(results[1].value.price).toBe(200);
    });
  });

  describe('runLocalHybridQuery', () => {
    it('should return matching records with score metadata', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['docs:1', 'docs:2']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'docs:1': { value: { title: 'Hello World', category: 'tech' } },
          'docs:2': { value: { title: 'Goodbye', category: 'tech' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('docs', {
        where: { category: 'tech' },
      });

      expect(results).toHaveLength(2);
      expect(results[0].score).toBe(0); // Local queries have score 0
      expect(results[0].matchedTerms).toEqual([]);
    });

    it('should apply sorting', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { name: 'C', price: 30 } },
          'items:2': { value: { name: 'A', price: 10 } },
          'items:3': { value: { name: 'B', price: 20 } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('items', {
        sort: { name: 'asc' },
      });

      expect(results[0].value.name).toBe('A');
      expect(results[1].value.name).toBe('B');
      expect(results[2].value.name).toBe('C');
    });

    it('should apply limit', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['items:1', 'items:2', 'items:3']);
      mockStorage.get.mockImplementation(async (key: string) => {
        const data: Record<string, any> = {
          'items:1': { value: { name: 'A' } },
          'items:2': { value: { name: 'B' } },
          'items:3': { value: { name: 'C' } },
        };
        return data[key];
      });

      const results = await queryManager.runLocalHybridQuery<any>('items', {
        limit: 2,
      });

      expect(results).toHaveLength(2);
    });
  });

  describe('resubscribeAll', () => {
    it('should resubscribe all standard queries', () => {
      mockIsAuthenticated.mockReturnValue(true);
      const query1 = new QueryHandle<any>(mockSyncEngine, 'map1', { where: { a: 1 } });
      const query2 = new QueryHandle<any>(mockSyncEngine, 'map2', { where: { b: 2 } });

      // Subscribe while authenticated
      queryManager.subscribeToQuery(query1);
      queryManager.subscribeToQuery(query2);
      mockSendMessage.mockClear();

      // Resubscribe
      queryManager.resubscribeAll();

      expect(mockSendMessage).toHaveBeenCalledTimes(2);
      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: expect.objectContaining({ queryId: query1.id }),
      });
      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'QUERY_SUB',
        payload: expect.objectContaining({ queryId: query2.id }),
      });
    });

    it('should resubscribe hybrid queries with FTS predicates', () => {
      mockIsAuthenticated.mockReturnValue(true);

      // Create a hybrid query with FTS predicate
      const hybridQuery = new HybridQueryHandle<any>(mockSyncEngine, 'docs', {
        predicate: { op: 'match', attribute: 'content', query: 'search term' },
      });

      queryManager.subscribeToHybridQuery(hybridQuery);
      mockSendMessage.mockClear();

      queryManager.resubscribeAll();

      expect(mockSendMessage).toHaveBeenCalledWith({
        type: 'HYBRID_QUERY_SUBSCRIBE',
        payload: expect.objectContaining({ subscriptionId: hybridQuery.id }),
      });
    });
  });
});
