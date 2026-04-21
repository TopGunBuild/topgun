import { DistributedLock } from '../DistributedLock';
import { SyncEngine } from '../SyncEngine';

// Spy on logger before importing DistributedLock so the mock is in place
let mockLoggerDebug: jest.Mock;
jest.mock('../utils/logger', () => ({
  logger: {
    debug: jest.fn(),
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
  },
}));

// Mock SyncEngine
const mockSyncEngine = {
  requestLock: jest.fn(),
  releaseLock: jest.fn(),
} as unknown as SyncEngine;

describe('DistributedLock', () => {
  let lock: DistributedLock;

  beforeEach(() => {
    jest.clearAllMocks();
    mockLoggerDebug = require('../utils/logger').logger.debug;
    lock = new DistributedLock(mockSyncEngine, 'test-lock');
  });

  test('should acquire lock successfully', async () => {
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });

    const result = await lock.lock(5000);

    expect(result).toBe(true);
    expect(lock.isLocked()).toBe(true);
    expect(mockSyncEngine.requestLock).toHaveBeenCalledWith('test-lock', expect.any(String), 5000);
  });

  test('should fail to acquire lock if request fails', async () => {
    (mockSyncEngine.requestLock as jest.Mock).mockRejectedValue(new Error('Timeout'));

    const result = await lock.lock();

    expect(result).toBe(false);
    expect(lock.isLocked()).toBe(false);
  });

  test('should unlock successfully', async () => {
    // Setup locked state
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });
    await lock.lock();

    await lock.unlock();

    expect(lock.isLocked()).toBe(false);
    expect(mockSyncEngine.releaseLock).toHaveBeenCalledWith('test-lock', expect.any(String), 10);
  });

  test('should do nothing on unlock if not locked', async () => {
    await lock.unlock();
    expect(mockSyncEngine.releaseLock).not.toHaveBeenCalled();
  });

  // ============================================
  // Regression tests for SPEC-220 fixes
  // ============================================

  test('unlock clears local state even when server release times out (releaseLock resolves false)', async () => {
    // Lock the resource
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });
    await lock.lock();
    expect(lock.isLocked()).toBe(true);

    // Release times out — releaseLock resolves false
    (mockSyncEngine.releaseLock as jest.Mock).mockResolvedValue(false);
    await lock.unlock();

    // Local state must be cleared regardless of ack outcome (idempotency)
    expect(lock.isLocked()).toBe(false);
  });

  test('unlock emits debug log when releaseLock returns false', async () => {
    (mockSyncEngine.requestLock as jest.Mock).mockResolvedValue({ fencingToken: 10 });
    await lock.lock();

    (mockSyncEngine.releaseLock as jest.Mock).mockResolvedValue(false);
    await lock.unlock();

    // pino debug signature: logger.debug(obj, message)
    expect(mockLoggerDebug).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'test-lock' }),
      'DistributedLock: release not acknowledged by server',
    );
  });

  test('lock(60000) with requestLock delayed to 45s resolves true (not rejected at 30s)', async () => {
    jest.useFakeTimers();
    try {
      // Simulate a slow server that responds at 45s
      (mockSyncEngine.requestLock as jest.Mock).mockImplementation(
        () =>
          new Promise((resolve) => {
            setTimeout(() => resolve({ fencingToken: 99 }), 45000);
          }),
      );

      const lockPromise = lock.lock(60000);

      // Advance to 30s — should still be pending (the old 30s hardcoded bug would reject here)
      jest.advanceTimersByTime(30001);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('still-pending'),
      ]);
      expect(raceResult).toBe('still-pending');

      // Advance remaining time so the mock resolves at 45s
      jest.advanceTimersByTime(15000);
      const result = await lockPromise;
      expect(result).toBe(true);
      expect(lock.isLocked()).toBe(true);
    } finally {
      jest.useRealTimers();
    }
  });
});

