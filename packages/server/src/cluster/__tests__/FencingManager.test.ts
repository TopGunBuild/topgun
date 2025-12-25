import { FencingManager, DEFAULT_FENCING_CONFIG } from '../FencingManager';

describe('FencingManager', () => {
  let manager: FencingManager;

  beforeEach(() => {
    manager = new FencingManager({
      gracePeriodMs: 100,
    });
  });

  describe('epoch management', () => {
    it('should start with initial epoch 0', () => {
      expect(manager.getEpoch()).toBe(0);
    });

    it('should increment epoch', () => {
      const newEpoch = manager.incrementEpoch('test', 'node-1');
      expect(newEpoch).toBe(1);
      expect(manager.getEpoch()).toBe(1);
    });

    it('should track epoch history', () => {
      manager.incrementEpoch('reason1', 'node-1');
      manager.incrementEpoch('reason2', 'node-2');

      const history = manager.getEpochHistory();
      expect(history.length).toBe(3); // initial + 2 changes
      expect(history[1].reason).toBe('reason1');
      expect(history[2].reason).toBe('reason2');
    });

    it('should emit epochChanged event', () => {
      const handler = jest.fn();
      manager.on('epochChanged', handler);

      manager.incrementEpoch('test', 'node-1');

      expect(handler).toHaveBeenCalledWith({
        previousEpoch: 0,
        newEpoch: 1,
        reason: 'test',
        changedBy: 'node-1',
      });
    });
  });

  describe('epoch validation', () => {
    it('should accept current epoch', () => {
      expect(manager.isEpochValid(0)).toBe(true);
    });

    it('should reject stale epoch', () => {
      manager.incrementEpoch('test');
      manager.incrementEpoch('test');

      // Epoch 0 is now 2 versions behind
      expect(manager.isEpochValid(0)).toBe(false);
      expect(manager.isEpochStale(0)).toBe(true);
    });

    it('should accept previous epoch during grace period', () => {
      manager.incrementEpoch('test');

      // Previous epoch (0) should still be valid during grace period
      expect(manager.isEpochValid(0)).toBe(true);
    });

    it('should reject previous epoch after grace period', async () => {
      manager.incrementEpoch('test');

      // Wait for grace period to expire
      await new Promise((r) => setTimeout(r, 150));

      expect(manager.isEpochValid(0)).toBe(false);
    });
  });

  describe('fencing tokens', () => {
    it('should create a token with current epoch', () => {
      const token = manager.createToken('node-1');

      expect(token.epoch).toBe(0);
      expect(token.nodeId).toBe('node-1');
      expect(token.createdAt).toBeDefined();
    });

    it('should create token with resource', () => {
      const token = manager.createToken('node-1', 'resource-A');

      expect(token.resource).toBe('resource-A');
    });

    it('should validate valid token', () => {
      const token = manager.createToken('node-1');
      expect(manager.validateToken(token)).toBe(true);
    });

    it('should reject token with stale epoch', async () => {
      const token = manager.createToken('node-1');

      manager.incrementEpoch('test');
      manager.incrementEpoch('test');

      // Wait for grace period
      await new Promise((r) => setTimeout(r, 150));

      expect(manager.validateToken(token)).toBe(false);
    });

    it('should release token', () => {
      const token = manager.createToken('node-1');
      expect(manager.validateToken(token)).toBe(true);

      manager.releaseToken('node-1');
      expect(manager.validateToken(token)).toBe(false);
    });

    it('should invalidate stale tokens on epoch change', async () => {
      const invalidatedHandler = jest.fn();
      manager.on('tokenInvalidated', invalidatedHandler);

      manager.createToken('node-1');
      manager.incrementEpoch('test');
      manager.incrementEpoch('test');

      // Tokens from epoch 0 should be invalidated
      expect(invalidatedHandler).toHaveBeenCalled();
    });
  });

  describe('node failure handling', () => {
    it('should increment epoch on node failure', () => {
      manager.onNodeFailure('failed-node');
      expect(manager.getEpoch()).toBe(1);
    });

    it('should invalidate tokens held by failed node', () => {
      const invalidatedHandler = jest.fn();
      manager.on('tokenInvalidated', invalidatedHandler);

      manager.createToken('failed-node', 'resource-A');
      manager.onNodeFailure('failed-node');

      expect(invalidatedHandler).toHaveBeenCalled();
    });

    it('should not invalidate tokens from other nodes', () => {
      manager.createToken('node-1');
      manager.createToken('node-2');

      manager.onNodeFailure('failed-node');

      // Tokens should still be valid (though epoch changed)
      expect(manager.getActiveTokens().length).toBe(2);
    });
  });

  describe('membership change', () => {
    it('should increment epoch on membership change', () => {
      manager.onMembershipChange('node_joined');
      expect(manager.getEpoch()).toBe(1);
    });
  });

  describe('metrics', () => {
    it('should return correct metrics', () => {
      manager.createToken('node-1');
      manager.createToken('node-2', 'resource-A');
      manager.incrementEpoch('test');

      const metrics = manager.getMetrics();
      expect(metrics.currentEpoch).toBe(1);
      expect(metrics.activeTokens).toBe(2);
      expect(metrics.inGracePeriod).toBe(true);
      expect(metrics.epochChanges).toBe(2); // initial + 1
    });

    it('should track grace period correctly', async () => {
      manager.incrementEpoch('test');

      let metrics = manager.getMetrics();
      expect(metrics.inGracePeriod).toBe(true);

      await new Promise((r) => setTimeout(r, 150));

      metrics = manager.getMetrics();
      expect(metrics.inGracePeriod).toBe(false);
    });
  });

  describe('default config', () => {
    it('should have sensible defaults', () => {
      expect(DEFAULT_FENCING_CONFIG.initialEpoch).toBe(0);
      expect(DEFAULT_FENCING_CONFIG.gracePeriodMs).toBe(1000);
    });
  });
});
