import { WriteConcern, WriteResult, WriteOptions } from '@topgunbuild/core';

/**
 * Integration tests for Write Concern feature
 *
 * These tests verify the Write Concern behavior from client perspective.
 * They test the schema types and helper functions, as full integration
 * testing requires server setup which is handled in e2e tests.
 */
describe('WriteConcern Integration', () => {
  describe('WriteConcern Enum', () => {
    it('should have all expected values', () => {
      expect(WriteConcern.FIRE_AND_FORGET).toBe('FIRE_AND_FORGET');
      expect(WriteConcern.MEMORY).toBe('MEMORY');
      expect(WriteConcern.APPLIED).toBe('APPLIED');
      expect(WriteConcern.REPLICATED).toBe('REPLICATED');
      expect(WriteConcern.PERSISTED).toBe('PERSISTED');
    });

    it('should be usable in switch statements', () => {
      const level: WriteConcern = WriteConcern.APPLIED;

      const getDescription = (wc: WriteConcern): string => {
        switch (wc) {
          case WriteConcern.FIRE_AND_FORGET:
            return 'fire-and-forget';
          case WriteConcern.MEMORY:
            return 'memory';
          case WriteConcern.APPLIED:
            return 'applied';
          case WriteConcern.REPLICATED:
            return 'replicated';
          case WriteConcern.PERSISTED:
            return 'persisted';
          default:
            return 'unknown';
        }
      };

      expect(getDescription(level)).toBe('applied');
    });
  });

  describe('WriteOptions Interface', () => {
    it('should allow undefined writeConcern (defaults to MEMORY)', () => {
      const options: WriteOptions = {};

      expect(options.writeConcern).toBeUndefined();
      expect(options.timeout).toBeUndefined();
    });

    it('should allow setting writeConcern', () => {
      const options: WriteOptions = {
        writeConcern: WriteConcern.PERSISTED,
      };

      expect(options.writeConcern).toBe(WriteConcern.PERSISTED);
    });

    it('should allow setting timeout', () => {
      const options: WriteOptions = {
        writeConcern: WriteConcern.APPLIED,
        timeout: 10000,
      };

      expect(options.writeConcern).toBe(WriteConcern.APPLIED);
      expect(options.timeout).toBe(10000);
    });
  });

  describe('WriteResult Interface', () => {
    it('should represent successful write', () => {
      const result: WriteResult = {
        success: true,
        opId: 'op-123',
        achievedLevel: WriteConcern.PERSISTED,
        latencyMs: 45,
      };

      expect(result.success).toBe(true);
      expect(result.opId).toBe('op-123');
      expect(result.achievedLevel).toBe(WriteConcern.PERSISTED);
      expect(result.latencyMs).toBe(45);
      expect(result.error).toBeUndefined();
    });

    it('should represent failed write with error', () => {
      const result: WriteResult = {
        success: false,
        opId: 'op-456',
        achievedLevel: WriteConcern.APPLIED,
        latencyMs: 5000,
        error: 'Timeout: achieved APPLIED, requested PERSISTED',
      };

      expect(result.success).toBe(false);
      expect(result.opId).toBe('op-456');
      expect(result.achievedLevel).toBe(WriteConcern.APPLIED);
      expect(result.latencyMs).toBe(5000);
      expect(result.error).toContain('Timeout');
    });

    it('should allow latencyMs of 0 for FIRE_AND_FORGET', () => {
      const result: WriteResult = {
        success: true,
        opId: 'op-789',
        achievedLevel: WriteConcern.FIRE_AND_FORGET,
        latencyMs: 0,
      };

      expect(result.latencyMs).toBe(0);
    });
  });

  describe('Backwards Compatibility', () => {
    it('should work without Write Concern options (existing behavior)', () => {
      // When no writeConcern is specified, system should use MEMORY (default)
      const defaultOptions: WriteOptions = {};

      // The default should be MEMORY, which is the current early ACK behavior
      const effectiveLevel = defaultOptions.writeConcern ?? WriteConcern.MEMORY;
      expect(effectiveLevel).toBe(WriteConcern.MEMORY);
    });
  });

  describe('Write Concern Levels Comparison', () => {
    const levelOrder = [
      WriteConcern.FIRE_AND_FORGET,
      WriteConcern.MEMORY,
      WriteConcern.APPLIED,
      WriteConcern.REPLICATED,
      WriteConcern.PERSISTED,
    ];

    it('should have correct order for durability guarantees', () => {
      // FIRE_AND_FORGET < MEMORY < APPLIED < REPLICATED < PERSISTED
      for (let i = 0; i < levelOrder.length - 1; i++) {
        const currentIndex = levelOrder.indexOf(levelOrder[i]);
        const nextIndex = levelOrder.indexOf(levelOrder[i + 1]);
        expect(currentIndex).toBeLessThan(nextIndex);
      }
    });

    it('should have PERSISTED as highest durability', () => {
      const highestIndex = Math.max(...levelOrder.map((l, i) => i));
      expect(levelOrder[highestIndex]).toBe(WriteConcern.PERSISTED);
    });

    it('should have FIRE_AND_FORGET as lowest durability', () => {
      const lowestIndex = Math.min(...levelOrder.map((l, i) => i));
      expect(levelOrder[lowestIndex]).toBe(WriteConcern.FIRE_AND_FORGET);
    });
  });

  describe('Use Case Examples', () => {
    it('should support metrics/logs use case (FIRE_AND_FORGET)', () => {
      const metricsOptions: WriteOptions = {
        writeConcern: WriteConcern.FIRE_AND_FORGET,
      };

      // For non-critical data like metrics, we don't need confirmation
      expect(metricsOptions.writeConcern).toBe(WriteConcern.FIRE_AND_FORGET);
    });

    it('should support real-time updates use case (MEMORY - default)', () => {
      const realtimeOptions: WriteOptions = {
        writeConcern: WriteConcern.MEMORY,
      };

      // For most real-time operations, MEMORY (early ACK) is sufficient
      expect(realtimeOptions.writeConcern).toBe(WriteConcern.MEMORY);
    });

    it('should support immediate consistency use case (APPLIED)', () => {
      const consistencyOptions: WriteOptions = {
        writeConcern: WriteConcern.APPLIED,
        timeout: 2000,
      };

      // When we need to read immediately after write
      expect(consistencyOptions.writeConcern).toBe(WriteConcern.APPLIED);
    });

    it('should support important data use case (REPLICATED)', () => {
      const importantDataOptions: WriteOptions = {
        writeConcern: WriteConcern.REPLICATED,
        timeout: 5000,
      };

      // For data that must survive node failure
      expect(importantDataOptions.writeConcern).toBe(WriteConcern.REPLICATED);
    });

    it('should support critical data use case (PERSISTED)', () => {
      const criticalOptions: WriteOptions = {
        writeConcern: WriteConcern.PERSISTED,
        timeout: 10000,
      };

      // For financial transactions, audit logs, etc.
      expect(criticalOptions.writeConcern).toBe(WriteConcern.PERSISTED);
    });
  });
});
