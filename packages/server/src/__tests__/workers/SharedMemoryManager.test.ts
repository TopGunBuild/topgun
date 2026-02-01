/**
 * Tests for SharedMemoryManager
 *
 * SharedArrayBuffer Integration
 */

import { Worker } from 'worker_threads';
import { join } from 'path';
import {
  SharedMemoryManager,
  SlotStatus,
} from '../../workers/SharedMemoryManager';
import { SharedMemoryWorkerHelper } from '../../workers/SharedMemoryWorkerHelper';

describe('SharedMemoryManager', () => {
  describe('Availability', () => {
    it('should detect SharedArrayBuffer availability', () => {
      const available = SharedMemoryManager.isAvailable();
      expect(typeof available).toBe('boolean');
      // In Node.js, SharedArrayBuffer should be available
      expect(available).toBe(true);
    });

    it('should also be available via helper', () => {
      expect(SharedMemoryWorkerHelper.isAvailable()).toBe(true);
    });
  });

  describe('Construction', () => {
    it('should create with default config', () => {
      const manager = new SharedMemoryManager();
      const stats = manager.getStats();

      expect(stats.totalSize).toBeGreaterThan(0);
      expect(stats.slotCount).toBe(256);
      expect(stats.allocatedSlots).toBe(0);
      expect(stats.availableSlots).toBe(256);
    });

    it('should create with custom config', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024, // 1MB
        slotCount: 16,
      });
      const stats = manager.getStats();

      expect(stats.slotCount).toBe(16);
      expect(stats.availableSlots).toBe(16);
      expect(stats.slotSize).toBe(Math.floor(1024 * 1024 / 16 / 8) * 8);
    });

    it('should reject invalid metadata size', () => {
      expect(() => {
        new SharedMemoryManager({ metadataSize: 15 }); // Not multiple of 8
      }).toThrow('metadataSize must be a multiple of 8');
    });

    it('should reject buffer too small for slots', () => {
      expect(() => {
        new SharedMemoryManager({
          bufferSize: 100,
          slotCount: 100,
        });
      }).toThrow('Buffer too small');
    });
  });

  describe('Allocation', () => {
    it('should allocate slots', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const slot = manager.allocate();

      expect(slot).not.toBeNull();
      expect(slot!.index).toBeGreaterThanOrEqual(0);
      expect(slot!.index).toBeLessThan(10);
      expect(slot!.maxDataSize).toBeGreaterThan(0);
      expect(slot!.dataView).toBeInstanceOf(Uint8Array);
    });

    it('should allocate multiple slots with unique indices', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const slots = [];
      const indices = new Set<number>();

      for (let i = 0; i < 5; i++) {
        const slot = manager.allocate();
        expect(slot).not.toBeNull();
        slots.push(slot!);
        indices.add(slot!.index);
      }

      // All indices should be unique
      expect(indices.size).toBe(5);

      // Cleanup
      slots.forEach((s) => manager.release(s));
    });

    it('should return null when no slots available', () => {
      const manager = new SharedMemoryManager({ slotCount: 2 });

      const slot1 = manager.allocate();
      const slot2 = manager.allocate();
      const slot3 = manager.allocate();

      expect(slot1).not.toBeNull();
      expect(slot2).not.toBeNull();
      expect(slot3).toBeNull();

      // Cleanup
      manager.release(slot1!);
      manager.release(slot2!);
    });

    it('should release slots for reuse', () => {
      const manager = new SharedMemoryManager({ slotCount: 1 });

      const slot1 = manager.allocate();
      expect(slot1).not.toBeNull();

      manager.release(slot1!);

      const slot2 = manager.allocate();
      expect(slot2).not.toBeNull();
      expect(slot2!.index).toBe(slot1!.index);

      manager.release(slot2!);
    });

    it('should handle double release gracefully', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const slot = manager.allocate()!;

      manager.release(slot);
      manager.release(slot); // Should not throw

      const stats = manager.getStats();
      expect(stats.allocatedSlots).toBe(0);
    });

    it('should set correct status on allocation', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const slot = manager.allocate()!;

      expect(manager.getStatus(slot.index)).toBe(SlotStatus.ALLOCATED);

      manager.release(slot);
      expect(manager.getStatus(slot.index)).toBe(SlotStatus.FREE);
    });
  });

  describe('Data Transfer', () => {
    it('should write and read data', () => {
      const manager = new SharedMemoryManager();
      const slot = manager.allocate()!;

      const testData = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
      const written = manager.writeData(slot, testData);

      expect(written).toBe(true);
      expect(manager.getStatus(slot.index)).toBe(SlotStatus.DATA_READY);

      // Read data
      const readData = manager.getDataView(slot.index);
      expect(readData.length).toBe(testData.length);
      expect(Array.from(readData)).toEqual(Array.from(testData));

      manager.release(slot);
    });

    it('should write large data', () => {
      // Use fewer slots to get larger slot size
      const manager = new SharedMemoryManager({
        bufferSize: 16 * 1024 * 1024, // 16MB
        slotCount: 128, // Larger slots: ~128KB each
      });
      const slot = manager.allocate()!;

      // Write 64KB of data (should fit easily)
      const testData = new Uint8Array(64 * 1024);
      for (let i = 0; i < testData.length; i++) {
        testData[i] = i % 256;
      }

      expect(slot.maxDataSize).toBeGreaterThan(testData.length);

      const written = manager.writeData(slot, testData);
      expect(written).toBe(true);

      const readData = manager.getDataView(slot.index);
      expect(readData.length).toBe(testData.length);

      // Verify content (sample check for performance)
      expect(readData[0]).toBe(0);
      expect(readData[255]).toBe(255);
      expect(readData[256]).toBe(0);
      expect(readData[testData.length - 1]).toBe((testData.length - 1) % 256);

      manager.release(slot);
    });

    it('should reject oversized data', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024,
        slotCount: 1,
      });
      const slot = manager.allocate()!;

      const oversizedData = new Uint8Array(slot.maxDataSize + 100);
      const written = manager.writeData(slot, oversizedData);

      expect(written).toBe(false);
      // Status should not change to DATA_READY
      expect(manager.getStatus(slot.index)).toBe(SlotStatus.ALLOCATED);

      manager.release(slot);
    });

    it('should correctly report data length', () => {
      const manager = new SharedMemoryManager();
      const slot = manager.allocate()!;

      const testData = new Uint8Array(42);
      manager.writeData(slot, testData);

      expect(manager.getDataLength(slot.index)).toBe(42);

      manager.release(slot);
    });
  });

  describe('Worker Config', () => {
    it('should provide correct worker config', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024,
        slotCount: 32,
        metadataSize: 24,
      });

      const config = manager.getWorkerConfig();

      expect(config.sharedBuffer).toBe(manager.getBuffer());
      expect(config.slotCount).toBe(32);
      expect(config.metadataSize).toBe(24);
      expect(config.slotSize).toBeGreaterThan(0);
    });
  });

  describe('Statistics', () => {
    it('should track allocations', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });

      const slot1 = manager.allocate();
      const slot2 = manager.allocate();

      const stats = manager.getStats();
      expect(stats.allocatedSlots).toBe(2);
      expect(stats.availableSlots).toBe(8);
      expect(stats.totalAllocations).toBe(2);

      manager.release(slot1!);
      manager.release(slot2!);

      const stats2 = manager.getStats();
      expect(stats2.allocatedSlots).toBe(0);
      expect(stats2.availableSlots).toBe(10);
      expect(stats2.totalReleases).toBe(2);
    });

    it('should track peak usage', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });

      const slots = [];
      for (let i = 0; i < 5; i++) {
        slots.push(manager.allocate());
      }

      expect(manager.getStats().peakUsage).toBe(5);

      slots.forEach((s) => manager.release(s!));

      // Peak should remain
      expect(manager.getStats().peakUsage).toBe(5);
      expect(manager.getStats().allocatedSlots).toBe(0);
    });
  });

  describe('SharedMemoryWorkerHelper', () => {
    it('should read data written by manager', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;
      const testData = new Uint8Array([10, 20, 30, 40, 50]);
      manager.writeData(slot, testData);

      // Helper reads the same data
      const readData = helper.readData(slot.index, false);
      expect(readData).not.toBeNull();
      expect(Array.from(readData!)).toEqual(Array.from(testData));

      manager.release(slot);
    });

    it('should write result readable by manager', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;

      // Simulate worker writing result
      const result = new Uint8Array([100, 200]);
      const written = helper.writeResult(slot.index, result);
      expect(written).toBe(true);

      // Manager reads result
      expect(manager.getStatus(slot.index)).toBe(SlotStatus.RESULT_READY);
      const readResult = manager.getDataView(slot.index);
      expect(Array.from(readResult)).toEqual(Array.from(result));

      manager.release(slot);
    });

    it('should signal error correctly', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;
      helper.signalError(slot.index);

      expect(manager.getStatus(slot.index)).toBe(SlotStatus.ERROR);

      manager.release(slot);
    });

    it('should provide max data size', () => {
      const manager = new SharedMemoryManager({
        bufferSize: 1024 * 1024,
        slotCount: 16,
        metadataSize: 16,
      });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;
      expect(helper.getMaxDataSize()).toBe(slot.maxDataSize);

      manager.release(slot);
    });

    it('should support zero-copy result writing via getResultView', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;

      // Write directly to result view (zero-copy)
      const resultView = helper.getResultView(slot.index);
      resultView[0] = 42;
      resultView[1] = 43;
      resultView[2] = 44;

      helper.signalResultReady(slot.index, 3);

      // Manager reads result
      const readResult = manager.getDataView(slot.index);
      expect(Array.from(readResult)).toEqual([42, 43, 44]);

      manager.release(slot);
    });

    it('should copy data when using readDataCopy', () => {
      const manager = new SharedMemoryManager({ slotCount: 10 });
      const helper = new SharedMemoryWorkerHelper(manager.getWorkerConfig());

      const slot = manager.allocate()!;
      const testData = new Uint8Array([1, 2, 3]);
      manager.writeData(slot, testData);

      const copied = helper.readDataCopy(slot.index, false);
      expect(copied).not.toBeNull();

      // Modify original
      slot.dataView[0] = 99;

      // Copy should be unchanged
      expect(copied![0]).toBe(1);

      manager.release(slot);
    });
  });
});

describe('SharedMemory Performance', () => {
  it('should achieve high throughput for write operations', () => {
    const manager = new SharedMemoryManager();
    const dataSize = 64 * 1024; // 64KB
    const iterations = 1000;

    const slot = manager.allocate()!;
    const data = new Uint8Array(dataSize);
    data.fill(0x42);

    const start = performance.now();
    for (let i = 0; i < iterations; i++) {
      manager.writeData(slot, data);
    }
    const elapsed = performance.now() - start;

    const throughputMBs = (iterations * dataSize) / elapsed / 1000;
    console.log(
      `SharedMemory write: ${elapsed.toFixed(2)}ms for ${iterations} x ${dataSize} bytes`
    );
    console.log(`Throughput: ${throughputMBs.toFixed(0)} MB/s`);

    // Should achieve at least 1 GB/s for in-memory operations
    expect(throughputMBs).toBeGreaterThan(1000);

    manager.release(slot);
  });

  it('should have low allocation overhead', () => {
    const manager = new SharedMemoryManager({ slotCount: 1000 });
    const iterations = 10000;

    const start = performance.now();
    for (let i = 0; i < iterations; i++) {
      const slot = manager.allocate()!;
      manager.release(slot);
    }
    const elapsed = performance.now() - start;

    const opsPerMs = iterations / elapsed;
    console.log(
      `Allocation: ${elapsed.toFixed(2)}ms for ${iterations} alloc/release cycles`
    );
    console.log(`Rate: ${(opsPerMs * 1000).toFixed(0)} ops/sec`);

    // Should achieve at least 100K ops/sec
    expect(opsPerMs * 1000).toBeGreaterThan(100000);
  });
});
