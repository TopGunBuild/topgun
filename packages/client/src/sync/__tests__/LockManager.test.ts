import { LockManager } from '../LockManager';
import type { LockManagerConfig } from '../types';

// ============================================
// Helpers
// ============================================

function makeConfig(overrides: Partial<LockManagerConfig> = {}): LockManagerConfig {
  return {
    sendMessage: jest.fn().mockReturnValue(true),
    isAuthenticated: jest.fn().mockReturnValue(true),
    isOnline: jest.fn().mockReturnValue(true),
    ...overrides,
  };
}

// ============================================
// Suite
// ============================================

describe('LockManager', () => {
  let mockDebug: jest.Mock;

  beforeEach(() => {
    jest.useFakeTimers();
    mockDebug = jest.fn();
    // Provide a mock logger to capture debug calls
    jest.mock('../../utils/logger', () => ({
      logger: {
        debug: mockDebug,
        info: jest.fn(),
        warn: jest.fn(),
        error: jest.fn(),
      },
    }));
  });

  afterEach(() => {
    jest.useRealTimers();
    jest.resetModules();
  });

  // ============================================
  // requestLock — TTL-coordinated timeout derivation
  // ============================================

  describe('requestLock — TTL-coordinated timeout', () => {
    test('rejects at 65s (not 30s) for ttl=60000 when no server response', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const lockPromise = manager.requestLock('lock-a', 'req-1', 60000);

      // Should still be pending at 30s
      jest.advanceTimersByTime(30001);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('pending'),
      ]);
      expect(raceResult).toBe('pending');

      // Should reject at 65s (60000 + 5000 grace)
      jest.advanceTimersByTime(35000); // total: 65001ms
      await expect(lockPromise).rejects.toThrow('Lock request timed out waiting for server response');
    });

    test('rejects at ~6s (1s + 5s grace) for ttl=1000 when no server response', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const lockPromise = manager.requestLock('lock-a', 'req-2', 1000);

      // Should still be pending just before 6s
      jest.advanceTimersByTime(5999);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('pending'),
      ]);
      expect(raceResult).toBe('pending');

      // Should reject at 6s
      jest.advanceTimersByTime(2); // total: 6001ms
      await expect(lockPromise).rejects.toThrow('Lock request timed out waiting for server response');
    });

    test('rejects at 5100ms (floor) for ttl=100', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const lockPromise = manager.requestLock('lock-a', 'req-3', 100);

      // For ttl=100: max(100+5000, 5000) = 5100ms
      jest.advanceTimersByTime(5099);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('pending'),
      ]);
      expect(raceResult).toBe('pending');

      jest.advanceTimersByTime(2); // total: 5101ms
      await expect(lockPromise).rejects.toThrow('Lock request timed out waiting for server response');
    });

    test('rejects at 5s (floor) for ttl=0 edge case', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const lockPromise = manager.requestLock('lock-a', 'req-4', 0);

      // max(0+5000, 5000) = 5000ms
      jest.advanceTimersByTime(4999);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('pending'),
      ]);
      expect(raceResult).toBe('pending');

      jest.advanceTimersByTime(2); // total: 5001ms
      await expect(lockPromise).rejects.toThrow('Lock request timed out waiting for server response');
    });

    test('rejects immediately when not authenticated', async () => {
      const config = makeConfig({ isAuthenticated: jest.fn().mockReturnValue(false) });
      const manager = new LockManager(config);
      await expect(manager.requestLock('lock-a', 'req-5', 5000)).rejects.toThrow('Not connected or authenticated');
    });
  });

  // ============================================
  // releaseLock — ACK disambiguation
  // ============================================

  describe('releaseLock — ACK disambiguation', () => {
    test('resolves true when server responds success: true', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const releasePromise = manager.releaseLock('lock-b', 'req-10', 42);

      manager.handleLockReleased('req-10', 'lock-b', true);
      await expect(releasePromise).resolves.toBe(true);
    });

    test('resolves false when server responds success: false', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const releasePromise = manager.releaseLock('lock-b', 'req-11', 42);

      manager.handleLockReleased('req-11', 'lock-b', false);
      await expect(releasePromise).resolves.toBe(false);
    });

    test('resolves false on ACK timeout after 5s', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const releasePromise = manager.releaseLock('lock-b', 'req-12', 42);

      jest.advanceTimersByTime(5001);
      await expect(releasePromise).resolves.toBe(false);
    });

    test('resolves false immediately when offline', async () => {
      const config = makeConfig({ isOnline: jest.fn().mockReturnValue(false) });
      const manager = new LockManager(config);
      const result = await manager.releaseLock('lock-b', 'req-13', 42);
      expect(result).toBe(false);
    });

    test('resolves false when sendMessage returns false', async () => {
      const config = makeConfig({ sendMessage: jest.fn().mockReturnValue(false) });
      const manager = new LockManager(config);
      const result = await manager.releaseLock('lock-b', 'req-14', 42);
      expect(result).toBe(false);
    });

    test('resolves false when sendMessage throws', async () => {
      const config = makeConfig({ sendMessage: jest.fn().mockImplementation(() => { throw new Error('socket closed'); }) });
      const manager = new LockManager(config);
      const result = await manager.releaseLock('lock-b', 'req-15', 42);
      expect(result).toBe(false);
    });
  });

  // ============================================
  // handleLockReleased — empty requestId
  // ============================================

  describe('handleLockReleased — empty requestId', () => {
    test('is a no-op for empty requestId and does not affect other pending requests', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);

      // Register a real pending release to verify the map is not corrupted
      const releasePromise = manager.releaseLock('lock-c', 'req-20', 99);

      // Fire empty-requestId handler — should not affect req-20
      expect(() => manager.handleLockReleased('', 'lock-c', true)).not.toThrow();

      // req-20 should still be pending (resolvable normally)
      manager.handleLockReleased('req-20', 'lock-c', true);
      await expect(releasePromise).resolves.toBe(true);
    });

    test('does not mutate pendingLockRequests when requestId is empty', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);

      // Start a pending release with a non-empty id
      const releasePromise = manager.releaseLock('lock-c', 'req-21', 99);

      // Empty requestId call — should be a no-op
      manager.handleLockReleased('', 'lock-c', false);

      // The real pending entry should still resolve
      manager.handleLockReleased('req-21', 'lock-c', true);
      await expect(releasePromise).resolves.toBe(true);
    });
  });

  // ============================================
  // handleLockReleased — normal path
  // ============================================

  describe('handleLockReleased — matching pending entry', () => {
    test('resolves the release promise with server success value', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const releasePromise = manager.releaseLock('lock-d', 'req-30', 7);

      manager.handleLockReleased('req-30', 'lock-d', true);
      await expect(releasePromise).resolves.toBe(true);
    });

    test('resolves false when server responds success: false (server_rejected)', async () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      const releasePromise = manager.releaseLock('lock-d', 'req-31', 7);

      manager.handleLockReleased('req-31', 'lock-d', false);
      await expect(releasePromise).resolves.toBe(false);
    });

    test('is a no-op when requestId has no matching pending entry', () => {
      const config = makeConfig();
      const manager = new LockManager(config);
      expect(() => manager.handleLockReleased('unknown', 'lock-d', true)).not.toThrow();
    });
  });
});
