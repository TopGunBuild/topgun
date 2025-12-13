import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { BackpressureError } from '../errors/BackpressureError';
import type { BackpressureConfig, BackpressureStatus } from '../BackpressureConfig';
import type { IStorageAdapter } from '../IStorageAdapter';

// Mock WebSocket
class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  binaryType = 'arraybuffer';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: any }) => void) | null = null;
  onerror: ((error: any) => void) | null = null;

  constructor(public url: string) {
    // Simulate connection
    setTimeout(() => {
      this.onopen?.();
    }, 0);
  }

  send = jest.fn();
  close = jest.fn(() => {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  });
}

// Mock storage adapter
function createMockStorage(): IStorageAdapter {
  let opId = 0;

  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(null),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getAllKeys: jest.fn().mockResolvedValue([]),
    getMeta: jest.fn().mockResolvedValue(null),
    setMeta: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockImplementation(() => {
      opId++;
      return Promise.resolve(opId);
    }),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
  };
}

describe('Backpressure', () => {
  let originalWebSocket: typeof WebSocket;

  beforeEach(() => {
    originalWebSocket = (globalThis as any).WebSocket;
    (globalThis as any).WebSocket = MockWebSocket;
  });

  afterEach(() => {
    (globalThis as any).WebSocket = originalWebSocket;
  });

  function createSyncEngine(backpressure?: Partial<BackpressureConfig>): SyncEngine {
    const config: SyncEngineConfig = {
      nodeId: 'test-node',
      serverUrl: 'ws://localhost:8080',
      storageAdapter: createMockStorage(),
      backpressure,
    };
    return new SyncEngine(config);
  }

  function createTimestamp() {
    return { millis: Date.now(), counter: 0, nodeId: 'test-node' };
  }

  describe('pause strategy', () => {
    it('should allow writes when under limit', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 10,
        strategy: 'pause',
      });

      // Should complete without blocking
      const opId = await engine.recordOperation('map1', 'PUT', 'key1', {
        record: { value: 'test', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(opId).toBeDefined();
      expect(engine.getPendingOpsCount()).toBe(1);
    });

    it('should pause writes when limit reached', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 3,
        strategy: 'pause',
        lowWaterMark: 0.3,
      });

      // Fill up to limit
      for (let i = 0; i < 3; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      expect(engine.getPendingOpsCount()).toBe(3);

      // This should pause (not resolve until capacity freed)
      let resolved = false;
      const writePromise = engine.recordOperation('map1', 'PUT', 'key-blocked', {
        record: { value: 'blocked', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      }).then(() => {
        resolved = true;
      });

      // Wait a bit to ensure it's paused
      await new Promise(resolve => setTimeout(resolve, 50));
      expect(resolved).toBe(false);
      expect(engine.isBackpressurePaused()).toBe(true);

      // Simulate ACKs to free up capacity (mark all as synced)
      // Access internal opLog to simulate ACK
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });
      (engine as any).checkLowWaterMark();

      // Now the write should resolve
      await writePromise;
      expect(resolved).toBe(true);
    });

    it('should emit backpressure:paused event', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'pause',
      });

      const pausedHandler = jest.fn();
      engine.onBackpressure('backpressure:paused', pausedHandler);

      // Fill up
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // Trigger pause
      engine.recordOperation('map1', 'PUT', 'key-pause', {
        record: { value: 'pause', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(pausedHandler).toHaveBeenCalled();
    });

    it('should emit backpressure:resumed event', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'pause',
        lowWaterMark: 0.5,
      });

      const resumedHandler = jest.fn();
      engine.onBackpressure('backpressure:resumed', resumedHandler);

      // Fill up and pause
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      engine.recordOperation('map1', 'PUT', 'key-pause', {
        record: { value: 'pause', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));

      // Simulate ACKs
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });
      (engine as any).checkLowWaterMark();

      expect(resumedHandler).toHaveBeenCalled();
    });

    it('should handle multiple paused writes', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'pause',
        lowWaterMark: 0.5,
      });

      // Fill up
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // Start multiple paused writes
      const promises: Promise<string>[] = [];
      for (let i = 0; i < 3; i++) {
        promises.push(
          engine.recordOperation('map1', 'PUT', `key-waiting-${i}`, {
            record: { value: `waiting-${i}`, timestamp: createTimestamp() },
            timestamp: createTimestamp(),
          })
        );
      }

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(engine.isBackpressurePaused()).toBe(true);

      // Simulate ACKs
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });
      (engine as any).checkLowWaterMark();

      // All should resolve
      const results = await Promise.all(promises);
      expect(results).toHaveLength(3);
      results.forEach(id => expect(id).toBeDefined());
    });
  });

  describe('throw strategy', () => {
    it('should throw BackpressureError when limit reached', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'throw',
      });

      // Fill up
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // This should throw
      await expect(
        engine.recordOperation('map1', 'PUT', 'key-throw', {
          record: { value: 'throw', timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        })
      ).rejects.toThrow(BackpressureError);
    });

    it('should include pending count in error', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 3,
        strategy: 'throw',
      });

      // Fill up
      for (let i = 0; i < 3; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      try {
        await engine.recordOperation('map1', 'PUT', 'key-throw', {
          record: { value: 'throw', timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
        fail('Should have thrown');
      } catch (err) {
        expect(err).toBeInstanceOf(BackpressureError);
        const bpError = err as BackpressureError;
        expect(bpError.pendingCount).toBe(3);
        expect(bpError.maxPending).toBe(3);
      }
    });

    it('should allow writes after ACKs', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'throw',
      });

      // Fill up
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // This should throw
      await expect(
        engine.recordOperation('map1', 'PUT', 'key-throw', {
          record: { value: 'throw', timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        })
      ).rejects.toThrow(BackpressureError);

      // Simulate ACKs
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });

      // Now it should work
      const opId = await engine.recordOperation('map1', 'PUT', 'key-success', {
        record: { value: 'success', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });
      expect(opId).toBeDefined();
    });
  });

  describe('drop-oldest strategy', () => {
    it('should drop oldest pending op when limit reached', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'drop-oldest',
      });

      // Fill up
      await engine.recordOperation('map1', 'PUT', 'key-oldest', {
        record: { value: 'oldest', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });
      await engine.recordOperation('map1', 'PUT', 'key-second', {
        record: { value: 'second', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(engine.getPendingOpsCount()).toBe(2);

      // This should drop the oldest
      await engine.recordOperation('map1', 'PUT', 'key-new', {
        record: { value: 'new', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      // Still at limit because we dropped one and added one
      expect(engine.getPendingOpsCount()).toBe(2);

      // Check that oldest was dropped
      const opLog = (engine as any).opLog;
      const keys = opLog.filter((op: any) => !op.synced).map((op: any) => op.key);
      expect(keys).not.toContain('key-oldest');
      expect(keys).toContain('key-second');
      expect(keys).toContain('key-new');
    });

    it('should emit operation:dropped event', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 1,
        strategy: 'drop-oldest',
      });

      const droppedHandler = jest.fn();
      engine.onBackpressure('operation:dropped', droppedHandler);

      await engine.recordOperation('map1', 'PUT', 'key-oldest', {
        record: { value: 'oldest', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      // This should drop the oldest
      await engine.recordOperation('map1', 'PUT', 'key-new', {
        record: { value: 'new', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(droppedHandler).toHaveBeenCalled();
      const eventData = droppedHandler.mock.calls[0][0];
      expect(eventData.key).toBe('key-oldest');
      expect(eventData.mapName).toBe('map1');
    });

    it('should select correct oldest by array order', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 3,
        strategy: 'drop-oldest',
      });

      const droppedKeys: string[] = [];
      engine.onBackpressure('operation:dropped', (data: any) => {
        droppedKeys.push(data.key);
      });

      // Fill up
      await engine.recordOperation('map1', 'PUT', 'first', {
        record: { value: 1, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });
      await engine.recordOperation('map1', 'PUT', 'second', {
        record: { value: 2, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });
      await engine.recordOperation('map1', 'PUT', 'third', {
        record: { value: 3, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      // Drop sequence
      await engine.recordOperation('map1', 'PUT', 'fourth', {
        record: { value: 4, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });
      await engine.recordOperation('map1', 'PUT', 'fifth', {
        record: { value: 5, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(droppedKeys).toEqual(['first', 'second']);
    });
  });

  describe('high/low water marks', () => {
    it('should emit backpressure:high at highWaterMark', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 10,
        strategy: 'pause',
        highWaterMark: 0.5, // 5 ops
      });

      const highHandler = jest.fn();
      engine.onBackpressure('backpressure:high', highHandler);

      // Add ops up to high water mark
      for (let i = 0; i < 5; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      expect(highHandler).toHaveBeenCalledTimes(1);
      expect(highHandler).toHaveBeenCalledWith({
        pending: 5,
        max: 10,
      });
    });

    it('should emit backpressure:low only when previously paused', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 3,
        strategy: 'pause',
        highWaterMark: 0.8,
        lowWaterMark: 0.3,
      });

      const lowHandler = jest.fn();
      engine.onBackpressure('backpressure:low', lowHandler);

      // Fill up to trigger pause
      for (let i = 0; i < 3; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // Trigger pause
      engine.recordOperation('map1', 'PUT', 'key-pause', {
        record: { value: 'pause', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(engine.isBackpressurePaused()).toBe(true);

      // Simulate ACKs
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });
      (engine as any).checkLowWaterMark();

      expect(lowHandler).toHaveBeenCalled();
    });
  });

  describe('monitoring', () => {
    it('should return pending ops count', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 100,
      });

      expect(engine.getPendingOpsCount()).toBe(0);

      await engine.recordOperation('map1', 'PUT', 'key1', {
        record: { value: 1, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(engine.getPendingOpsCount()).toBe(1);

      await engine.recordOperation('map1', 'PUT', 'key2', {
        record: { value: 2, timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      expect(engine.getPendingOpsCount()).toBe(2);
    });

    it('should return backpressure status', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 10,
        strategy: 'pause',
        highWaterMark: 0.8,
        lowWaterMark: 0.5,
      });

      const status = engine.getBackpressureStatus();

      expect(status).toEqual<BackpressureStatus>({
        pending: 0,
        max: 10,
        percentage: 0,
        isPaused: false,
        strategy: 'pause',
      });
    });

    it('should update status after operations', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 10,
        strategy: 'throw',
      });

      for (let i = 0; i < 5; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      const status = engine.getBackpressureStatus();

      expect(status.pending).toBe(5);
      expect(status.max).toBe(10);
      expect(status.percentage).toBe(0.5);
      expect(status.isPaused).toBe(false);
      expect(status.strategy).toBe('throw');
    });

    it('should report isPaused correctly', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 2,
        strategy: 'pause',
      });

      expect(engine.isBackpressurePaused()).toBe(false);

      // Fill up
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // Trigger pause
      engine.recordOperation('map1', 'PUT', 'key-pause', {
        record: { value: 'pause', timestamp: createTimestamp() },
        timestamp: createTimestamp(),
      });

      await new Promise(resolve => setTimeout(resolve, 10));
      expect(engine.isBackpressurePaused()).toBe(true);
      expect(engine.getBackpressureStatus().isPaused).toBe(true);
    });
  });

  describe('event unsubscription', () => {
    it('should allow unsubscribing from events', async () => {
      const engine = createSyncEngine({
        maxPendingOps: 5,
        strategy: 'pause',
        highWaterMark: 0.4,
      });

      const handler = jest.fn();
      const unsubscribe = engine.onBackpressure('backpressure:high', handler);

      // Trigger high water mark
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      expect(handler).toHaveBeenCalledTimes(1);

      // Unsubscribe
      unsubscribe();

      // Reset high water mark flag
      (engine as any).highWaterMarkEmitted = false;
      (engine as any).opLog.forEach((op: any) => { op.synced = true; });

      // Trigger again
      for (let i = 0; i < 2; i++) {
        await engine.recordOperation('map1', 'PUT', `key-new${i}`, {
          record: { value: i, timestamp: createTimestamp() },
          timestamp: createTimestamp(),
        });
      }

      // Should not be called again
      expect(handler).toHaveBeenCalledTimes(1);
    });
  });

  describe('BackpressureError', () => {
    it('should have correct name', () => {
      const error = new BackpressureError(10, 10);
      expect(error.name).toBe('BackpressureError');
    });

    it('should have correct message', () => {
      const error = new BackpressureError(10, 10);
      expect(error.message).toContain('10/10');
      expect(error.message).toContain('Backpressure limit reached');
    });

    it('should be instanceof Error', () => {
      const error = new BackpressureError(10, 10);
      expect(error).toBeInstanceOf(Error);
      expect(error).toBeInstanceOf(BackpressureError);
    });
  });

  describe('default configuration', () => {
    it('should use default config when not specified', () => {
      const engine = createSyncEngine();
      const status = engine.getBackpressureStatus();

      expect(status.max).toBe(1000);
      expect(status.strategy).toBe('pause');
    });

    it('should allow partial config override', () => {
      const engine = createSyncEngine({
        maxPendingOps: 500,
      });
      const status = engine.getBackpressureStatus();

      expect(status.max).toBe(500);
      expect(status.strategy).toBe('pause'); // Default
    });
  });
});
