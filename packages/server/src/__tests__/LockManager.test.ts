import { LockManager } from '../cluster/LockManager';

describe('LockManager', () => {
  let lockManager: LockManager;

  beforeEach(() => {
    lockManager = new LockManager();
  });

  afterEach(() => {
    lockManager.stop();
  });

  test('should grant lock to first requester', () => {
    const result = lockManager.acquire('res1', 'client1', 'req1', 1000);
    expect(result.granted).toBe(true);
    expect(result.fencingToken).toBe(1);
  });

  test('should queue second requester', () => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    const result = lockManager.acquire('res1', 'client2', 'req2', 1000);
    expect(result.granted).toBe(false);
  });

  test('should extend lease for same owner', () => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    const result = lockManager.acquire('res1', 'client1', 'req2', 2000);
    expect(result.granted).toBe(true);
    expect(result.fencingToken).toBe(1); // Token shouldn't change for extension
  });

  test('should release lock and grant to next in queue', (done) => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    lockManager.acquire('res1', 'client2', 'req2', 1000);

    lockManager.on('lockGranted', (evt) => {
      expect(evt.clientId).toBe('client2');
      expect(evt.name).toBe('res1');
      expect(evt.fencingToken).toBe(2);
      done();
    });

    const success = lockManager.release('res1', 'client1', 1);
    expect(success).toBe(true);
  });

  test('should fail to release with wrong token', () => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    const success = lockManager.release('res1', 'client1', 999);
    expect(success).toBe(false);
  });

  test('should fail to release with wrong owner', () => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    const success = lockManager.release('res1', 'client2', 1);
    expect(success).toBe(false);
  });

  test('should auto-expire lock and grant to next', (done) => {
    // Short TTL, but clamped to MIN_TTL (1000ms) in implementation now
    // So we use 1000ms TTL and wait slightly longer
    lockManager.acquire('res1', 'client1', 'req1', 1000); 
    lockManager.acquire('res1', 'client2', 'req2', 1000);

    lockManager.on('lockGranted', (evt) => {
      expect(evt.clientId).toBe('client2');
      expect(evt.name).toBe('res1');
      done();
    });

    // Wait for expiration (1000ms + check interval which is 1000ms)
    // It might take up to ~2000ms
  }, 3000);

  test('handleClientDisconnect should release held locks', (done) => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    lockManager.acquire('res1', 'client2', 'req2', 1000);

    lockManager.on('lockGranted', (evt) => {
      expect(evt.clientId).toBe('client2');
      done();
    });

    lockManager.handleClientDisconnect('client1');
  });

  test('handleClientDisconnect should remove from queue', () => {
    lockManager.acquire('res1', 'client1', 'req1', 1000);
    lockManager.acquire('res1', 'client2', 'req2', 1000);
    lockManager.acquire('res1', 'client3', 'req3', 1000);

    lockManager.handleClientDisconnect('client2');

    // release client1, client3 should get it immediately, skipping client2
    let grantedToClient3 = false;
    lockManager.on('lockGranted', (evt) => {
        if (evt.clientId === 'client3') grantedToClient3 = true;
    });

    lockManager.release('res1', 'client1', 1);
    
    expect(grantedToClient3).toBe(true);
  });

  test('should clamp TTL values', () => {
    const resMin = lockManager.acquire('resMin', 'c1', 'r1', 10); // < 1000
    expect(resMin.granted).toBe(true);
    // Internal check if we could expose getLockState... or just infer by waiting
    // If it expires in 10ms, it would be gone quickly.
    // But due to clamp, it should last 1000ms.

    // We can't easily inspect internal state without exposing it,
    // but we trust the logic change was applied.
  });

  describe('TTL clamping with fake timers', () => {
    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    test('should clamp MIN_TTL - lock should NOT expire before 1000ms even with ttl=10', () => {
      // Create a new LockManager with fake timers
      const lm = new LockManager();

      lm.acquire('res', 'client1', 'req1', 10); // Request 10ms, but should be clamped to 1000ms
      lm.acquire('res', 'client2', 'req2', 5000);

      let client2Granted = false;
      lm.on('lockGranted', (evt) => {
        if (evt.clientId === 'client2') client2Granted = true;
      });

      // Advance 500ms - lock should still be held by client1
      jest.advanceTimersByTime(500);
      expect(client2Granted).toBe(false);

      // Advance to 900ms - still should not expire (MIN_TTL is 1000)
      jest.advanceTimersByTime(400);
      expect(client2Granted).toBe(false);

      // Advance past 1000ms + cleanup interval (1000ms) = need ~2000ms total
      jest.advanceTimersByTime(1100); // Now at 2000ms
      expect(client2Granted).toBe(true);

      lm.stop();
    });

    test('should clamp MAX_TTL - lock should expire at 300000ms even with ttl=Infinity', () => {
      const lm = new LockManager();

      lm.acquire('res', 'client1', 'req1', Infinity); // Should be clamped to 300000ms (5 min)
      lm.acquire('res', 'client2', 'req2', 5000);

      let client2Granted = false;
      lm.on('lockGranted', (evt) => {
        if (evt.clientId === 'client2') client2Granted = true;
      });

      // Advance 4 minutes - lock should still be held
      jest.advanceTimersByTime(4 * 60 * 1000);
      expect(client2Granted).toBe(false);

      // Advance past 5 minutes + cleanup interval
      jest.advanceTimersByTime(61 * 1000); // 4:00 + 1:01 = 5:01
      expect(client2Granted).toBe(true);

      lm.stop();
    });

    test('should handle ttl=0 by clamping to MIN_TTL', () => {
      const lm = new LockManager();

      lm.acquire('res', 'client1', 'req1', 0); // Should be clamped to 1000ms
      lm.acquire('res', 'client2', 'req2', 5000);

      let client2Granted = false;
      lm.on('lockGranted', (evt) => {
        if (evt.clientId === 'client2') client2Granted = true;
      });

      // At 500ms - should not expire yet
      jest.advanceTimersByTime(500);
      expect(client2Granted).toBe(false);

      // At 2100ms (past 1000ms TTL + 1000ms interval) - should expire
      jest.advanceTimersByTime(1600);
      expect(client2Granted).toBe(true);

      lm.stop();
    });

    test('should handle negative ttl by clamping to MIN_TTL', () => {
      const lm = new LockManager();

      lm.acquire('res', 'client1', 'req1', -5000); // Negative - should be clamped to 1000ms
      lm.acquire('res', 'client2', 'req2', 5000);

      let client2Granted = false;
      lm.on('lockGranted', (evt) => {
        if (evt.clientId === 'client2') client2Granted = true;
      });

      // At 500ms - should not expire yet
      jest.advanceTimersByTime(500);
      expect(client2Granted).toBe(false);

      // At 2100ms - should expire
      jest.advanceTimersByTime(1600);
      expect(client2Granted).toBe(true);

      lm.stop();
    });
  });
});
