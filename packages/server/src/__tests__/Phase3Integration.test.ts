/**
 * Integration Tests
 *
 * Tests for native module integration:
 * - Native hash (xxHash64) in core
 * - SharedArrayBuffer for worker communication
 * - Graceful fallback when native unavailable
 *
 * Integration
 */

import {
  SharedMemoryManager,
  SharedMemoryWorkerHelper,
  SlotStatus,
} from '../workers';
import {
  hashString,
  combineHashes,
  isUsingNativeHash,
  disableNativeHash,
  resetNativeHash,
} from '@topgunbuild/core';
import {
  getNativeStats,
  getNativeModuleStatus,
  logNativeStatus,
} from '../utils/nativeStats';

describe('Integration Tests', () => {
  describe('Native Module Detection', () => {
    afterEach(() => {
      // Reset native module state for next test
      resetNativeHash();
    });

    it('should report native module status correctly', () => {
      const status = getNativeModuleStatus();

      expect(status).toHaveProperty('nativeHash');
      expect(status).toHaveProperty('sharedArrayBuffer');
      expect(typeof status.nativeHash).toBe('boolean');
      expect(typeof status.sharedArrayBuffer).toBe('boolean');
    });

    it('should detect SharedArrayBuffer availability', () => {
      const isAvailable = SharedMemoryManager.isAvailable();
      expect(typeof isAvailable).toBe('boolean');
      // In Node.js test environment, should be true
      expect(isAvailable).toBe(true);
    });

    it('should detect hash module status', () => {
      const isNative = isUsingNativeHash();
      expect(typeof isNative).toBe('boolean');
      // In test environment, native hash might or might not be available
    });

    it('should get native stats', () => {
      const stats = getNativeStats();

      expect(stats).toHaveProperty('modules');
      expect(stats).toHaveProperty('sharedMemory');
      expect(stats).toHaveProperty('summary');
      expect(typeof stats.modules.nativeHash).toBe('boolean');
    });

    it('should log native status without throwing', () => {
      expect(() => logNativeStatus()).not.toThrow();
    });
  });

  describe('Hash Module Integration', () => {
    afterEach(() => {
      resetNativeHash();
    });

    it('should hash strings consistently', () => {
      const str = 'test-string';
      const hash1 = hashString(str);
      const hash2 = hashString(str);
      const hash3 = hashString(str);

      expect(hash1).toBe(hash2);
      expect(hash2).toBe(hash3);
    });

    it('should produce different hashes for different strings', () => {
      const hash1 = hashString('string1');
      const hash2 = hashString('string2');
      const hash3 = hashString('string3');

      expect(hash1).not.toBe(hash2);
      expect(hash2).not.toBe(hash3);
      expect(hash1).not.toBe(hash3);
    });

    it('should combine hashes order-independently', () => {
      const h1 = hashString('a');
      const h2 = hashString('b');
      const h3 = hashString('c');

      const combined1 = combineHashes([h1, h2, h3]);
      const combined2 = combineHashes([h3, h1, h2]);
      const combined3 = combineHashes([h2, h3, h1]);

      expect(combined1).toBe(combined2);
      expect(combined2).toBe(combined3);
    });

    it('should work with JS fallback when native is disabled', () => {
      // Disable native hash
      disableNativeHash();

      expect(isUsingNativeHash()).toBe(false);

      // Should still work
      const hash = hashString('test');
      expect(typeof hash).toBe('number');
      expect(hash).toBeGreaterThanOrEqual(0);

      // Should be consistent
      const hash2 = hashString('test');
      expect(hash).toBe(hash2);
    });

    it('should handle typical Merkle tree keys', () => {
      const keys = [
        'users/user1',
        'users/user2',
        'posts/post1',
        'posts/post2',
        'comments/comment1',
      ];

      const hashes = keys.map(hashString);

      // All unique
      const uniqueHashes = new Set(hashes);
      expect(uniqueHashes.size).toBe(keys.length);

      // All valid
      for (const hash of hashes) {
        expect(typeof hash).toBe('number');
        expect(hash).toBeGreaterThanOrEqual(0);
        expect(hash).toBeLessThanOrEqual(0xffffffff);
      }
    });
  });

  describe('SharedMemoryManager Integration', () => {
    let manager: SharedMemoryManager;

    beforeEach(() => {
      manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024, // 1MB
        slotCount: 16,
      });
    });

    afterEach(() => {
      manager.shutdown();
    });

    it('should allocate and release slots', () => {
      const slot1 = manager.allocate();
      expect(slot1).not.toBeNull();

      const slot2 = manager.allocate();
      expect(slot2).not.toBeNull();
      expect(slot1!.index).not.toBe(slot2!.index);

      manager.release(slot1!);
      manager.release(slot2!);
    });

    it('should write and read data', () => {
      const slot = manager.allocate();
      expect(slot).not.toBeNull();

      const testData = new Uint8Array([1, 2, 3, 4, 5]);
      const written = manager.writeData(slot!, testData);
      expect(written).toBe(true);

      // Get worker config for reading
      const workerConfig = manager.getWorkerConfig();
      const helper = new SharedMemoryWorkerHelper(workerConfig);

      // Read data (without waiting since it's already written)
      const readData = helper.readData(slot!.index, false);
      expect(readData).not.toBeNull();
      expect(Array.from(readData!)).toEqual(Array.from(testData));

      manager.release(slot!);
    });

    it('should handle worker result writing', () => {
      const slot = manager.allocate();
      expect(slot).not.toBeNull();

      // Write initial data
      const inputData = new Uint8Array([10, 20, 30]);
      manager.writeData(slot!, inputData);

      // Simulate worker processing
      const workerConfig = manager.getWorkerConfig();
      const helper = new SharedMemoryWorkerHelper(workerConfig);

      helper.markProcessing(slot!.index);

      // Write result
      const resultData = new Uint8Array([100, 200, 255]);
      helper.writeResult(slot!.index, resultData);

      // Read result from main thread
      const result = manager.waitForResult(slot!, 1000);
      expect(result).not.toBeNull();
      expect(Array.from(result!)).toEqual(Array.from(resultData));

      manager.release(slot!);
    });

    it('should report accurate statistics', () => {
      const stats = manager.getStats();

      expect(stats.slotCount).toBe(16);
      expect(stats.availableSlots).toBe(16);
      expect(stats.allocatedSlots).toBe(0);
      expect(stats.totalSize).toBeGreaterThan(0);

      // Allocate some slots
      const slot1 = manager.allocate();
      const slot2 = manager.allocate();
      const slot3 = manager.allocate();

      const statsAfter = manager.getStats();
      expect(statsAfter.availableSlots).toBe(13);
      expect(statsAfter.allocatedSlots).toBe(3);

      manager.release(slot1!);
      manager.release(slot2!);
      manager.release(slot3!);
    });

    it('should handle concurrent allocations', async () => {
      const slots: ReturnType<typeof manager.allocate>[] = [];

      // Allocate all slots concurrently
      const promises = Array(16)
        .fill(0)
        .map(() =>
          Promise.resolve(manager.allocate()).then((slot) => {
            if (slot) slots.push(slot);
          })
        );

      await Promise.all(promises);

      // Should have allocated all slots
      expect(slots.length).toBe(16);

      // All indices should be unique
      const indices = new Set(slots.map((s) => s!.index));
      expect(indices.size).toBe(16);

      // Trying to allocate more should return null
      const extraSlot = manager.allocate();
      expect(extraSlot).toBeNull();

      // Clean up
      for (const slot of slots) {
        manager.release(slot!);
      }
    });

    it('should handle large payloads', () => {
      const largeManager = new SharedMemoryManager({
        bufferSize: 4 * 1024 * 1024, // 4MB
        slotCount: 4, // 1MB per slot
      });

      try {
        const slot = largeManager.allocate();
        expect(slot).not.toBeNull();

        // Create 500KB payload
        const largeData = new Uint8Array(500 * 1024);
        for (let i = 0; i < largeData.length; i++) {
          largeData[i] = i % 256;
        }

        const written = largeManager.writeData(slot!, largeData);
        expect(written).toBe(true);

        const workerConfig = largeManager.getWorkerConfig();
        const helper = new SharedMemoryWorkerHelper(workerConfig);
        const readData = helper.readData(slot!.index, false);

        expect(readData).not.toBeNull();
        expect(readData!.length).toBe(largeData.length);

        // Verify first and last bytes
        expect(readData![0]).toBe(largeData[0]);
        expect(readData![readData!.length - 1]).toBe(
          largeData[largeData.length - 1]
        );

        largeManager.release(slot!);
      } finally {
        largeManager.shutdown();
      }
    });
  });

  describe('Graceful Degradation', () => {
    afterEach(() => {
      resetNativeHash();
    });

    it('should work without native hash module', () => {
      disableNativeHash();

      // All operations should still work
      const hash = hashString('test-without-native');
      expect(typeof hash).toBe('number');

      const combined = combineHashes([hash, hash, hash]);
      expect(typeof combined).toBe('number');
    });

    it('should report fallback status when native unavailable', () => {
      disableNativeHash();

      const stats = getNativeStats();
      expect(stats.modules.nativeHash).toBe(false);
      expect(stats.summary).toContain('FNV-1a');
    });

    it('should maintain hash consistency across implementation switch', () => {
      // Get hash with whatever implementation is available
      const hash1 = hashString('consistent-test');

      // Force fallback
      disableNativeHash();
      const hash2 = hashString('consistent-test');

      // Reset and get hash again
      resetNativeHash();
      const hash3 = hashString('consistent-test');

      // Hash1 and hash3 should match (same implementation)
      // Hash2 might be different (FNV-1a fallback)
      // But all should be valid numbers
      expect(typeof hash1).toBe('number');
      expect(typeof hash2).toBe('number');
      expect(typeof hash3).toBe('number');

      // If native was available initially, hash1 === hash3
      if (isUsingNativeHash()) {
        expect(hash1).toBe(hash3);
      }
    });
  });

  describe('Integration with getNativeStats', () => {
    it('should include SharedMemory stats when manager provided', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024,
        slotCount: 8,
      });

      try {
        const stats = getNativeStats(manager);

        expect(stats.sharedMemory).not.toBeNull();
        expect(stats.sharedMemory!.slotCount).toBe(8);
        expect(stats.sharedMemory!.totalSize).toBeGreaterThan(0);
      } finally {
        manager.shutdown();
      }
    });

    it('should work without SharedMemory manager', () => {
      const stats = getNativeStats();

      // Should still have module status
      expect(stats.modules).toBeDefined();
      // SharedMemory should be null
      expect(stats.sharedMemory).toBeNull();
    });
  });

  describe('Performance characteristics', () => {
    it('should hash 10000 strings quickly', () => {
      const start = performance.now();

      for (let i = 0; i < 10000; i++) {
        hashString(`test-string-${i}`);
      }

      const elapsed = performance.now() - start;

      // Should complete in reasonable time (< 100ms)
      expect(elapsed).toBeLessThan(100);
    });

    it('should allocate/release slots without memory leak', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024,
        slotCount: 100,
      });

      try {
        // Allocate and release many times
        for (let i = 0; i < 1000; i++) {
          const slot = manager.allocate();
          expect(slot).not.toBeNull();
          manager.release(slot!);
        }

        // Should still have all slots available
        const stats = manager.getStats();
        expect(stats.availableSlots).toBe(100);
      } finally {
        manager.shutdown();
      }
    });

    it('should handle rapid write/read cycles', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024,
        slotCount: 10,
      });

      try {
        const workerConfig = manager.getWorkerConfig();
        const helper = new SharedMemoryWorkerHelper(workerConfig);

        for (let i = 0; i < 100; i++) {
          const slot = manager.allocate();
          expect(slot).not.toBeNull();

          const data = new Uint8Array([i % 256, (i + 1) % 256, (i + 2) % 256]);
          manager.writeData(slot!, data);

          const readData = helper.readData(slot!.index, false);
          expect(readData).not.toBeNull();
          expect(Array.from(readData!)).toEqual(Array.from(data));

          manager.release(slot!);
        }
      } finally {
        manager.shutdown();
      }
    });
  });
});
