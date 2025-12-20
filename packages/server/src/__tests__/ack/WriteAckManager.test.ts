import { WriteAckManager } from '../../ack/WriteAckManager';
import { WriteConcern } from '@topgunbuild/core';

describe('WriteAckManager', () => {
  let manager: WriteAckManager;

  beforeEach(() => {
    manager = new WriteAckManager({ defaultTimeout: 1000 });
  });

  afterEach(() => {
    // Use shutdown instead of clear to avoid unhandled promise rejections
    manager.shutdown();
  });

  describe('Level Resolution', () => {
    it('should resolve FIRE_AND_FORGET immediately', async () => {
      const result = await manager.registerPending('op1', WriteConcern.FIRE_AND_FORGET);

      expect(result.success).toBe(true);
      expect(result.opId).toBe('op1');
      expect(result.achievedLevel).toBe(WriteConcern.FIRE_AND_FORGET);
      expect(result.latencyMs).toBe(0);
    });

    it('should resolve MEMORY after registration', async () => {
      const promise = manager.registerPending('op1', WriteConcern.MEMORY);

      // MEMORY should resolve immediately after registration
      const result = await promise;

      expect(result.success).toBe(true);
      expect(result.opId).toBe('op1');
      expect(result.achievedLevel).toBe(WriteConcern.MEMORY);
    });

    it('should resolve APPLIED after notifyLevel(APPLIED)', async () => {
      const promise = manager.registerPending('op1', WriteConcern.APPLIED);

      // Simulate processing
      manager.notifyLevel('op1', WriteConcern.MEMORY);
      manager.notifyLevel('op1', WriteConcern.APPLIED);

      const result = await promise;

      expect(result.success).toBe(true);
      expect(result.achievedLevel).toBe(WriteConcern.APPLIED);
    });

    it('should resolve REPLICATED after notifyLevel(REPLICATED)', async () => {
      const promise = manager.registerPending('op1', WriteConcern.REPLICATED);

      // Simulate processing
      manager.notifyLevel('op1', WriteConcern.MEMORY);
      manager.notifyLevel('op1', WriteConcern.APPLIED);
      manager.notifyLevel('op1', WriteConcern.REPLICATED);

      const result = await promise;

      expect(result.success).toBe(true);
      expect(result.achievedLevel).toBe(WriteConcern.REPLICATED);
    });

    it('should resolve PERSISTED after notifyLevel(PERSISTED)', async () => {
      const promise = manager.registerPending('op1', WriteConcern.PERSISTED);

      // Simulate full processing
      manager.notifyLevel('op1', WriteConcern.MEMORY);
      manager.notifyLevel('op1', WriteConcern.APPLIED);
      manager.notifyLevel('op1', WriteConcern.REPLICATED);
      manager.notifyLevel('op1', WriteConcern.PERSISTED);

      const result = await promise;

      expect(result.success).toBe(true);
      expect(result.achievedLevel).toBe(WriteConcern.PERSISTED);
    });
  });

  describe('Timeout Handling', () => {
    it('should timeout if level not achieved', async () => {
      // Use shorter timeout for test
      const shortManager = new WriteAckManager({ defaultTimeout: 100 });

      const promise = shortManager.registerPending('op1', WriteConcern.PERSISTED);

      // Don't notify any levels - let it timeout
      const result = await promise;

      expect(result.success).toBe(false);
      expect(result.error).toContain('Timeout');
      expect(result.achievedLevel).toBe(WriteConcern.FIRE_AND_FORGET);

      shortManager.shutdown();
    });

    it('should return highest achieved level on timeout', async () => {
      const shortManager = new WriteAckManager({ defaultTimeout: 100 });

      const promise = shortManager.registerPending('op1', WriteConcern.PERSISTED);

      // Notify partial progress
      shortManager.notifyLevel('op1', WriteConcern.MEMORY);
      shortManager.notifyLevel('op1', WriteConcern.APPLIED);

      // Wait for timeout
      const result = await promise;

      expect(result.success).toBe(false);
      expect(result.achievedLevel).toBe(WriteConcern.APPLIED);
      expect(result.error).toContain('achieved APPLIED');
      expect(result.error).toContain('requested PERSISTED');

      shortManager.shutdown();
    });

    it('should emit timeout event', async () => {
      const shortManager = new WriteAckManager({ defaultTimeout: 100 });

      const timeoutSpy = jest.fn();
      shortManager.on('timeout', timeoutSpy);

      const promise = shortManager.registerPending('op1', WriteConcern.PERSISTED);

      await promise;

      expect(timeoutSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          opId: 'op1',
          requested: WriteConcern.PERSISTED,
          achieved: WriteConcern.FIRE_AND_FORGET,
        })
      );

      shortManager.shutdown();
    });
  });

  describe('Level Ordering', () => {
    it('should resolve APPLIED when PERSISTED achieved', async () => {
      const promise = manager.registerPending('op1', WriteConcern.APPLIED);

      // Skip directly to PERSISTED
      manager.notifyLevel('op1', WriteConcern.PERSISTED);

      const result = await promise;

      expect(result.success).toBe(true);
      // PERSISTED is higher than APPLIED, so we get PERSISTED
      expect(result.achievedLevel).toBe(WriteConcern.PERSISTED);
    });

    it('should not resolve PERSISTED when only APPLIED achieved', async () => {
      const shortManager = new WriteAckManager({ defaultTimeout: 100 });

      const promise = shortManager.registerPending('op1', WriteConcern.PERSISTED);

      // Only achieve APPLIED
      shortManager.notifyLevel('op1', WriteConcern.APPLIED);

      const result = await promise;

      expect(result.success).toBe(false);

      shortManager.shutdown();
    });

    it('should handle out-of-order level notifications', async () => {
      const promise = manager.registerPending('op1', WriteConcern.REPLICATED);

      // Notify levels out of order
      manager.notifyLevel('op1', WriteConcern.APPLIED);
      manager.notifyLevel('op1', WriteConcern.MEMORY);
      manager.notifyLevel('op1', WriteConcern.REPLICATED);

      const result = await promise;

      expect(result.success).toBe(true);
      expect(result.achievedLevel).toBe(WriteConcern.REPLICATED);
    });
  });

  describe('Statistics', () => {
    it('should track pending writes by level', () => {
      manager.registerPending('op1', WriteConcern.APPLIED);
      manager.registerPending('op2', WriteConcern.PERSISTED);
      manager.registerPending('op3', WriteConcern.APPLIED);

      const stats = manager.getStats();

      expect(stats.pending).toBe(3);
      expect(stats.byLevel[WriteConcern.APPLIED]).toBe(2);
      expect(stats.byLevel[WriteConcern.PERSISTED]).toBe(1);
      expect(stats.byLevel[WriteConcern.MEMORY]).toBe(0);
    });

    it('should update stats on resolution', async () => {
      const promise = manager.registerPending('op1', WriteConcern.APPLIED);

      expect(manager.getStats().pending).toBe(1);

      manager.notifyLevel('op1', WriteConcern.APPLIED);
      await promise;

      expect(manager.getStats().pending).toBe(0);
    });
  });

  describe('Batch Operations', () => {
    it('should notify multiple operations at once', async () => {
      const promise1 = manager.registerPending('op1', WriteConcern.APPLIED);
      const promise2 = manager.registerPending('op2', WriteConcern.APPLIED);
      const promise3 = manager.registerPending('op3', WriteConcern.APPLIED);

      manager.notifyLevelBatch(['op1', 'op2', 'op3'], WriteConcern.APPLIED);

      const [result1, result2, result3] = await Promise.all([promise1, promise2, promise3]);

      expect(result1.success).toBe(true);
      expect(result2.success).toBe(true);
      expect(result3.success).toBe(true);
    });
  });

  describe('Fail Pending', () => {
    it('should fail a pending write with error', async () => {
      const promise = manager.registerPending('op1', WriteConcern.PERSISTED);

      manager.failPending('op1', 'Storage error');

      const result = await promise;

      expect(result.success).toBe(false);
      expect(result.error).toBe('Storage error');
    });

    it('should emit failed event', async () => {
      const failedSpy = jest.fn();
      manager.on('failed', failedSpy);

      const promise = manager.registerPending('op1', WriteConcern.PERSISTED);
      manager.failPending('op1', 'Storage error');

      await promise;

      expect(failedSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          opId: 'op1',
          error: 'Storage error',
        })
      );
    });
  });

  describe('Clear and Shutdown', () => {
    it('should reject all pending on clear', async () => {
      const promise1 = manager.registerPending('op1', WriteConcern.PERSISTED);
      const promise2 = manager.registerPending('op2', WriteConcern.PERSISTED);

      manager.clear();

      await expect(promise1).rejects.toThrow('WriteAckManager cleared');
      await expect(promise2).rejects.toThrow('WriteAckManager cleared');
    });

    it('should resolve all pending on shutdown with current level', async () => {
      const promise1 = manager.registerPending('op1', WriteConcern.PERSISTED);
      const promise2 = manager.registerPending('op2', WriteConcern.PERSISTED);

      // Notify some progress
      manager.notifyLevel('op1', WriteConcern.APPLIED);

      manager.shutdown();

      const [result1, result2] = await Promise.all([promise1, promise2]);

      // op1 achieved APPLIED
      expect(result1.achievedLevel).toBe(WriteConcern.APPLIED);
      expect(result1.success).toBe(false); // Didn't achieve PERSISTED

      // op2 only has FIRE_AND_FORGET (default)
      expect(result2.achievedLevel).toBe(WriteConcern.FIRE_AND_FORGET);
    });
  });

  describe('Event Emission', () => {
    it('should emit resolved event on success', async () => {
      const resolvedSpy = jest.fn();
      manager.on('resolved', resolvedSpy);

      const promise = manager.registerPending('op1', WriteConcern.APPLIED);
      manager.notifyLevel('op1', WriteConcern.APPLIED);

      await promise;

      expect(resolvedSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          opId: 'op1',
          achievedLevel: WriteConcern.APPLIED,
          success: true,
        })
      );
    });
  });

  describe('Edge Cases', () => {
    it('should ignore notifyLevel for unknown operation', () => {
      // Should not throw
      manager.notifyLevel('unknown-op', WriteConcern.APPLIED);
    });

    it('should handle duplicate level notifications', async () => {
      const promise = manager.registerPending('op1', WriteConcern.APPLIED);

      manager.notifyLevel('op1', WriteConcern.APPLIED);
      manager.notifyLevel('op1', WriteConcern.APPLIED); // Duplicate

      const result = await promise;

      expect(result.success).toBe(true);
    });

    it('should track pending operation correctly', () => {
      manager.registerPending('op1', WriteConcern.APPLIED);

      expect(manager.isPending('op1')).toBe(true);
      expect(manager.isPending('op2')).toBe(false);
      expect(manager.getTargetLevel('op1')).toBe(WriteConcern.APPLIED);
    });

    it('should return pending IDs', () => {
      manager.registerPending('op1', WriteConcern.APPLIED);
      manager.registerPending('op2', WriteConcern.PERSISTED);

      const ids = manager.getPendingIds();

      expect(ids).toContain('op1');
      expect(ids).toContain('op2');
      expect(ids.length).toBe(2);
    });
  });
});
