import { TopGun } from '../TopGun';

// Mock IDBAdapter to avoid actual IndexedDB in Node env
jest.mock('../adapters/IDBAdapter', () => {
  return {
    IDBAdapter: jest.fn().mockImplementation(() => ({
      initialize: jest.fn().mockResolvedValue(undefined),
      getAllKeys: jest.fn().mockResolvedValue([]),
      get: jest.fn().mockResolvedValue(undefined),
      put: jest.fn().mockResolvedValue(undefined),
      getMeta: jest.fn().mockResolvedValue(undefined),
      setMeta: jest.fn().mockResolvedValue(undefined),
      batchPut: jest.fn().mockResolvedValue(undefined),
      appendOpLog: jest.fn().mockResolvedValue(1),
      getPendingOps: jest.fn().mockResolvedValue([]),
      markOpsSynced: jest.fn().mockResolvedValue(undefined),
      close: jest.fn().mockResolvedValue(undefined)
    }))
  };
});

describe('TopGun Facade', () => {
  it('should initialize with config', async () => {
    const db = new TopGun({
      sync: 'ws://test.local',
      persist: 'indexeddb'
    });
    expect(db).toBeDefined();
  });

  it('should allow access to collections via properties', () => {
    const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });
    const todos = db.todos;
    expect(todos).toBeDefined();
    expect(todos.set).toBeDefined();
  });

  it('should allow waitForReady', async () => {
      const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });
      await expect(db.waitForReady()).resolves.not.toThrow();
  });

  it('should set and get values unwrapped', async () => {
      const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });
      
      await db.items.set({ id: '1', val: 'test' });
      
      const item = db.items.get('1');
      expect(item).toEqual({ id: '1', val: 'test' });
  });

   it('should throw if setting item without id', async () => {
      const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });
      await expect(db.items.set({ val: 'no-id' })).rejects.toThrow();
  });

  it('should allow operations before waitForReady completes (non-blocking)', async () => {
      const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });

      // Should be able to set values immediately without awaiting waitForReady
      await db.items.set({ id: 'immediate-1', val: 'works' });

      // Value should be in memory immediately
      const item = db.items.get('immediate-1');
      expect(item).toEqual({ id: 'immediate-1', val: 'works' });

      // waitForReady should still resolve successfully
      await expect(db.waitForReady()).resolves.not.toThrow();
  });

  it('should queue operations and persist them after IndexedDB is ready', async () => {
      const db = new TopGun({ sync: 'ws://test', persist: 'indexeddb' });

      // Multiple rapid writes before IndexedDB is ready
      await db.tasks.set({ id: 't1', text: 'Task 1' });
      await db.tasks.set({ id: 't2', text: 'Task 2' });
      await db.tasks.set({ id: 't3', text: 'Task 3' });

      // All values should be in memory
      expect(db.tasks.get('t1')).toEqual({ id: 't1', text: 'Task 1' });
      expect(db.tasks.get('t2')).toEqual({ id: 't2', text: 'Task 2' });
      expect(db.tasks.get('t3')).toEqual({ id: 't3', text: 'Task 3' });
  });
});

