import { IDBAdapter } from '../adapters/IDBAdapter';

// Mock the 'idb' module
const mockDB = {
  get: jest.fn(),
  put: jest.fn(),
  delete: jest.fn(),
  add: jest.fn(),
  getAll: jest.fn(),
  getAllKeys: jest.fn(),
  transaction: jest.fn(),
  close: jest.fn(),
};

jest.mock('idb', () => ({
  openDB: jest.fn(() => Promise.resolve(mockDB)),
}));

describe('IDBAdapter - Non-blocking Initialization', () => {
  let adapter: IDBAdapter;

  beforeEach(() => {
    jest.clearAllMocks();
    adapter = new IDBAdapter();

    // Default mock implementations
    mockDB.get.mockResolvedValue(undefined);
    mockDB.put.mockResolvedValue(undefined);
    mockDB.delete.mockResolvedValue(undefined);
    mockDB.add.mockResolvedValue(1);
    mockDB.getAll.mockResolvedValue([]);
    mockDB.getAllKeys.mockResolvedValue([]);
    mockDB.transaction.mockReturnValue({
      store: {
        put: jest.fn().mockResolvedValue(undefined),
        openCursor: jest.fn().mockResolvedValue(null),
      },
      done: Promise.resolve(),
    });
  });

  describe('initialize()', () => {
    it('should return immediately without blocking', async () => {
      const startTime = Date.now();
      await adapter.initialize('test_db');
      const elapsed = Date.now() - startTime;

      // Should complete nearly instantly (< 50ms)
      expect(elapsed).toBeLessThan(50);
    });

    it('should start IndexedDB initialization in the background', async () => {
      const { openDB } = require('idb');
      await adapter.initialize('test_db');

      expect(openDB).toHaveBeenCalledWith('test_db', 2, expect.any(Object));
    });
  });

  describe('waitForReady()', () => {
    it('should resolve when IndexedDB is ready', async () => {
      await adapter.initialize('test_db');
      await expect(adapter.waitForReady()).resolves.toBeUndefined();
    });

    it('should be safe to call multiple times', async () => {
      await adapter.initialize('test_db');
      await adapter.waitForReady();
      await adapter.waitForReady();
      await adapter.waitForReady();
      // Should not throw
    });
  });

  describe('Write Operations - Queue before ready', () => {
    it('should queue put operations before IndexedDB is ready', async () => {
      // Don't await initialize - test queueing behavior
      adapter.initialize('test_db');

      // This should queue the operation, not fail
      const putPromise = adapter.put('key1', { value: 'test' });

      // Wait for initialization to complete
      await adapter.waitForReady();

      // Now the queued operation should resolve
      await expect(putPromise).resolves.toBeUndefined();
      expect(mockDB.put).toHaveBeenCalledWith('kv_store', { key: 'key1', value: { value: 'test' } });
    });

    it('should queue multiple operations and execute in order', async () => {
      const callOrder: string[] = [];
      mockDB.put.mockImplementation(async (store, data) => {
        callOrder.push(data.key);
      });

      adapter.initialize('test_db');

      // Queue multiple operations
      const p1 = adapter.put('first', { v: 1 });
      const p2 = adapter.put('second', { v: 2 });
      const p3 = adapter.put('third', { v: 3 });

      await adapter.waitForReady();
      await Promise.all([p1, p2, p3]);

      // Should execute in order
      expect(callOrder).toEqual(['first', 'second', 'third']);
    });

    it('should queue appendOpLog operations', async () => {
      mockDB.add.mockResolvedValue(42);

      adapter.initialize('test_db');
      const promise = adapter.appendOpLog({ key: 'test', op: 'PUT' });

      await adapter.waitForReady();
      const result = await promise;

      expect(result).toBe(42);
      expect(mockDB.add).toHaveBeenCalled();
    });
  });

  describe('Read Operations - Wait for ready', () => {
    it('should wait for ready before get operations', async () => {
      mockDB.get.mockResolvedValue({ key: 'k1', value: 'data' });

      adapter.initialize('test_db');

      // Get should wait for initialization
      const getPromise = adapter.get('k1');

      // Result should come after initialization
      const result = await getPromise;
      expect(result).toBe('data');
    });

    it('should wait for ready before getAllKeys', async () => {
      mockDB.getAllKeys.mockResolvedValue(['key1', 'key2']);

      adapter.initialize('test_db');
      const keys = await adapter.getAllKeys();

      expect(keys).toEqual(['key1', 'key2']);
    });
  });

  describe('Post-initialization - Direct execution', () => {
    it('should execute operations directly once ready', async () => {
      await adapter.initialize('test_db');
      await adapter.waitForReady();

      // Clear any previous calls
      mockDB.put.mockClear();

      // This should execute immediately, not queue
      await adapter.put('direct', { value: 'immediate' });

      expect(mockDB.put).toHaveBeenCalledTimes(1);
    });
  });
});
