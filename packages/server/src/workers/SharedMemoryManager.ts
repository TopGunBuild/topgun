/**
 * SharedMemoryManager - Zero-copy data transfer between main thread and workers
 *
 * Uses SharedArrayBuffer with Atomics for synchronization.
 * Provides slot-based allocation for concurrent operations.
 *
 * SharedArrayBuffer Integration
 */

export interface SharedMemoryConfig {
  /**
   * Size of shared buffer in bytes.
   * Default: 16MB
   */
  bufferSize?: number;

  /**
   * Number of slots for concurrent operations.
   * Each slot can hold one transfer.
   * Default: 256
   */
  slotCount?: number;

  /**
   * Reserved bytes per slot for metadata (length, status, etc).
   * Default: 16 (must be multiple of 8 for alignment)
   */
  metadataSize?: number;
}

export interface SharedMemoryStats {
  /** Total buffer size in bytes */
  totalSize: number;
  /** Number of slots */
  slotCount: number;
  /** Size of each slot in bytes */
  slotSize: number;
  /** Currently allocated slots */
  allocatedSlots: number;
  /** Available slots */
  availableSlots: number;
  /** Peak concurrent allocations */
  peakUsage: number;
  /** Total allocations since creation */
  totalAllocations: number;
  /** Total releases since creation */
  totalReleases: number;
}

export interface SharedSlot {
  /** Slot index */
  index: number;
  /** View into shared buffer for this slot's data area */
  dataView: Uint8Array;
  /** Maximum data size (excluding metadata) */
  maxDataSize: number;
}

/**
 * Slot status values (stored in first 4 bytes of slot metadata)
 */
export enum SlotStatus {
  FREE = 0, // Slot is available
  ALLOCATED = 1, // Slot is allocated, no data yet
  DATA_READY = 2, // Data written, ready for reading
  PROCESSING = 3, // Worker is processing
  RESULT_READY = 4, // Worker has written result
  ERROR = 255, // Error occurred
}

/**
 * Slot metadata layout (16 bytes):
 * - Bytes 0-3: Status (Int32 for Atomics)
 * - Bytes 4-7: Data length (Uint32)
 * - Bytes 8-15: Reserved for future use
 */
const DEFAULT_METADATA_SIZE = 16;

/**
 * Manages shared memory for zero-copy data transfer between threads.
 *
 * Usage:
 * 1. Main thread allocates a slot
 * 2. Main thread writes data to slot
 * 3. Main thread sends slot index to worker via postMessage
 * 4. Worker reads data from shared memory (zero-copy)
 * 5. Worker writes result to slot
 * 6. Main thread reads result (zero-copy)
 * 7. Main thread releases slot
 */
export class SharedMemoryManager {
  private readonly buffer: SharedArrayBuffer;
  private readonly statusArray: Int32Array; // For Atomics operations
  private readonly slotSize: number;
  private readonly slotCount: number;
  private readonly metadataSize: number;
  private readonly freeSlots: Set<number>;

  // Stats
  private allocatedCount = 0;
  private peakUsage = 0;
  private totalAllocations = 0;
  private totalReleases = 0;

  constructor(config?: SharedMemoryConfig) {
    const bufferSize = config?.bufferSize ?? 16 * 1024 * 1024; // 16MB
    this.slotCount = config?.slotCount ?? 256;
    this.metadataSize = config?.metadataSize ?? DEFAULT_METADATA_SIZE;

    // Ensure metadata size is aligned to 8 bytes
    if (this.metadataSize % 8 !== 0) {
      throw new Error('metadataSize must be a multiple of 8');
    }

    // Calculate slot size
    this.slotSize = Math.floor(bufferSize / this.slotCount);

    // Ensure slot size is aligned to 8 bytes
    this.slotSize = Math.floor(this.slotSize / 8) * 8;

    if (this.slotSize <= this.metadataSize) {
      throw new Error('Buffer too small for given slot count');
    }

    // Allocate shared buffer
    const actualSize = this.slotSize * this.slotCount;
    this.buffer = new SharedArrayBuffer(actualSize);

    // Create Int32Array view for Atomics operations on status fields
    this.statusArray = new Int32Array(this.buffer);

    // Initialize free slots
    this.freeSlots = new Set();
    for (let i = 0; i < this.slotCount; i++) {
      this.freeSlots.add(i);
      // Initialize status to FREE
      Atomics.store(this.statusArray, this.getStatusOffset(i), SlotStatus.FREE);
    }
  }

  /**
   * Get offset in Int32Array for slot status.
   * Status is at the beginning of each slot's metadata.
   */
  private getStatusOffset(slotIndex: number): number {
    return (slotIndex * this.slotSize) / 4; // Int32 = 4 bytes
  }

  /**
   * Get byte offset for slot length field.
   */
  private getLengthOffset(slotIndex: number): number {
    return slotIndex * this.slotSize + 4; // After status (4 bytes)
  }

  /**
   * Get byte offset for slot data (after metadata).
   */
  private getDataOffset(slotIndex: number): number {
    return slotIndex * this.slotSize + this.metadataSize;
  }

  /**
   * Allocate a slot for data transfer.
   * Returns null if no slots available.
   */
  allocate(): SharedSlot | null {
    // Find free slot
    const iterator = this.freeSlots.values();
    const result = iterator.next();

    if (result.done) {
      return null; // No free slots
    }

    const index = result.value;
    this.freeSlots.delete(index);

    // Update stats
    this.allocatedCount++;
    this.totalAllocations++;
    this.peakUsage = Math.max(this.peakUsage, this.allocatedCount);

    // Set status to ALLOCATED using Atomics
    Atomics.store(
      this.statusArray,
      this.getStatusOffset(index),
      SlotStatus.ALLOCATED
    );

    // Create data view
    const dataOffset = this.getDataOffset(index);
    const maxDataSize = this.slotSize - this.metadataSize;

    return {
      index,
      dataView: new Uint8Array(this.buffer, dataOffset, maxDataSize),
      maxDataSize,
    };
  }

  /**
   * Release a slot back to the pool.
   */
  release(slot: SharedSlot): void {
    if (this.freeSlots.has(slot.index)) {
      return; // Already free
    }

    // Set status to FREE
    Atomics.store(
      this.statusArray,
      this.getStatusOffset(slot.index),
      SlotStatus.FREE
    );

    // Return to pool
    this.freeSlots.add(slot.index);
    this.allocatedCount--;
    this.totalReleases++;
  }

  /**
   * Write data to a slot and signal it's ready.
   * Returns false if data is too large.
   */
  writeData(slot: SharedSlot, data: Uint8Array): boolean {
    if (data.length > slot.maxDataSize) {
      return false; // Data too large
    }

    // Write length
    const lengthView = new DataView(
      this.buffer,
      this.getLengthOffset(slot.index),
      4
    );
    lengthView.setUint32(0, data.length, true); // little-endian

    // Write data
    slot.dataView.set(data);

    // Signal data is ready (memory barrier via Atomics)
    Atomics.store(
      this.statusArray,
      this.getStatusOffset(slot.index),
      SlotStatus.DATA_READY
    );
    // Wake up any waiting workers
    Atomics.notify(this.statusArray, this.getStatusOffset(slot.index));

    return true;
  }

  /**
   * Read data length from a slot.
   */
  getDataLength(slotIndex: number): number {
    const lengthView = new DataView(
      this.buffer,
      this.getLengthOffset(slotIndex),
      4
    );
    return lengthView.getUint32(0, true);
  }

  /**
   * Get data view for a slot (for reading).
   */
  getDataView(slotIndex: number): Uint8Array {
    const length = this.getDataLength(slotIndex);
    const dataOffset = this.getDataOffset(slotIndex);
    return new Uint8Array(this.buffer, dataOffset, length);
  }

  /**
   * Get slot status.
   */
  getStatus(slotIndex: number): SlotStatus {
    return Atomics.load(this.statusArray, this.getStatusOffset(slotIndex));
  }

  /**
   * Wait for a specific status with timeout.
   * Returns the actual status (may differ if timeout occurred).
   */
  waitForStatus(
    slotIndex: number,
    expectedStatus: SlotStatus,
    timeoutMs: number = 5000
  ): SlotStatus {
    const statusOffset = this.getStatusOffset(slotIndex);
    const deadline = Date.now() + timeoutMs;

    while (Date.now() < deadline) {
      const status = Atomics.load(this.statusArray, statusOffset);

      if (status === expectedStatus || status === SlotStatus.ERROR) {
        return status;
      }

      // Wait with timeout
      const remaining = deadline - Date.now();
      if (remaining > 0) {
        Atomics.wait(
          this.statusArray,
          statusOffset,
          status,
          Math.min(remaining, 100)
        );
      }
    }

    return Atomics.load(this.statusArray, statusOffset);
  }

  /**
   * Wait for result and read it.
   * Returns null on timeout or error.
   */
  waitForResult(slot: SharedSlot, timeoutMs: number = 5000): Uint8Array | null {
    const status = this.waitForStatus(
      slot.index,
      SlotStatus.RESULT_READY,
      timeoutMs
    );

    if (status === SlotStatus.RESULT_READY) {
      // Read length
      const length = this.getDataLength(slot.index);

      // Copy result (we need to copy because slot will be reused)
      const result = new Uint8Array(length);
      result.set(slot.dataView.subarray(0, length));

      return result;
    }

    return null; // Timeout or error
  }

  /**
   * Get the SharedArrayBuffer for passing to workers.
   */
  getBuffer(): SharedArrayBuffer {
    return this.buffer;
  }

  /**
   * Get configuration needed by workers.
   */
  getWorkerConfig(): SharedWorkerConfig {
    return {
      sharedBuffer: this.buffer,
      slotSize: this.slotSize,
      slotCount: this.slotCount,
      metadataSize: this.metadataSize,
    };
  }

  /**
   * Get statistics.
   */
  getStats(): SharedMemoryStats {
    return {
      totalSize: this.buffer.byteLength,
      slotCount: this.slotCount,
      slotSize: this.slotSize,
      allocatedSlots: this.allocatedCount,
      availableSlots: this.freeSlots.size,
      peakUsage: this.peakUsage,
      totalAllocations: this.totalAllocations,
      totalReleases: this.totalReleases,
    };
  }

  /**
   * Check if SharedArrayBuffer is available in current environment.
   */
  static isAvailable(): boolean {
    try {
      new SharedArrayBuffer(1);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Shutdown and release resources.
   * Resets all slots to FREE status.
   */
  shutdown(): void {
    // Reset all slots to FREE
    for (let i = 0; i < this.slotCount; i++) {
      Atomics.store(this.statusArray, this.getStatusOffset(i), SlotStatus.FREE);
    }
    this.freeSlots.clear();
    for (let i = 0; i < this.slotCount; i++) {
      this.freeSlots.add(i);
    }
    this.allocatedCount = 0;
  }
}

/**
 * Configuration passed to workers for shared memory access.
 */
export interface SharedWorkerConfig {
  sharedBuffer: SharedArrayBuffer;
  slotSize: number;
  slotCount: number;
  metadataSize: number;
}
