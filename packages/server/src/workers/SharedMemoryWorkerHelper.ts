/**
 * SharedMemoryWorkerHelper - Worker-side helper for shared memory operations
 *
 * Used by workers to read/write data from/to shared memory allocated by main thread.
 *
 * Phase 3.04: SharedArrayBuffer Integration
 */

import { SlotStatus, SharedWorkerConfig } from './SharedMemoryManager';

/**
 * Helper class for workers to interact with shared memory.
 *
 * Usage in worker:
 * ```typescript
 * const helper = new SharedMemoryWorkerHelper(workerData);
 *
 * parentPort.on('message', (msg) => {
 *   const data = helper.readData(msg.slotIndex);
 *   // Process data...
 *   helper.writeResult(msg.slotIndex, result);
 *   parentPort.postMessage({ taskId: msg.taskId, success: true });
 * });
 * ```
 */
export class SharedMemoryWorkerHelper {
  private readonly buffer: SharedArrayBuffer;
  private readonly statusArray: Int32Array;
  private readonly slotSize: number;
  private readonly slotCount: number;
  private readonly metadataSize: number;

  constructor(config: SharedWorkerConfig) {
    this.buffer = config.sharedBuffer;
    this.slotSize = config.slotSize;
    this.slotCount = config.slotCount;
    this.metadataSize = config.metadataSize;
    this.statusArray = new Int32Array(this.buffer);
  }

  /**
   * Get offset in Int32Array for slot status.
   */
  private getStatusOffset(slotIndex: number): number {
    return (slotIndex * this.slotSize) / 4;
  }

  /**
   * Get byte offset for slot length field.
   */
  private getLengthOffset(slotIndex: number): number {
    return slotIndex * this.slotSize + 4;
  }

  /**
   * Get byte offset for slot data.
   */
  private getDataOffset(slotIndex: number): number {
    return slotIndex * this.slotSize + this.metadataSize;
  }

  /**
   * Get maximum data size for a slot.
   */
  getMaxDataSize(): number {
    return this.slotSize - this.metadataSize;
  }

  /**
   * Get current slot status.
   */
  getStatus(slotIndex: number): SlotStatus {
    return Atomics.load(this.statusArray, this.getStatusOffset(slotIndex));
  }

  /**
   * Set slot status.
   */
  private setStatus(slotIndex: number, status: SlotStatus): void {
    Atomics.store(this.statusArray, this.getStatusOffset(slotIndex), status);
    Atomics.notify(this.statusArray, this.getStatusOffset(slotIndex));
  }

  /**
   * Read data from a slot.
   * Optionally waits for DATA_READY status.
   *
   * @param slotIndex - Index of the slot to read from
   * @param wait - Whether to wait for data (default: true)
   * @param timeoutMs - Timeout in milliseconds (default: 5000)
   * @returns View into shared buffer (zero-copy!) or null on timeout
   */
  readData(
    slotIndex: number,
    wait: boolean = true,
    timeoutMs: number = 5000
  ): Uint8Array | null {
    const statusOffset = this.getStatusOffset(slotIndex);

    if (wait) {
      const deadline = Date.now() + timeoutMs;

      while (Date.now() < deadline) {
        const status = Atomics.load(this.statusArray, statusOffset);

        if (status === SlotStatus.DATA_READY) {
          break;
        }

        if (status === SlotStatus.ERROR) {
          return null;
        }

        // Wait for status change
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

      // Check final status
      const finalStatus = Atomics.load(this.statusArray, statusOffset);
      if (finalStatus !== SlotStatus.DATA_READY) {
        return null; // Timeout
      }
    }

    // Read length
    const lengthView = new DataView(
      this.buffer,
      this.getLengthOffset(slotIndex),
      4
    );
    const length = lengthView.getUint32(0, true);

    // Return view into shared buffer (zero-copy!)
    const dataOffset = this.getDataOffset(slotIndex);
    return new Uint8Array(this.buffer, dataOffset, length);
  }

  /**
   * Read data and copy it (for cases where original buffer may be modified).
   */
  readDataCopy(
    slotIndex: number,
    wait: boolean = true,
    timeoutMs: number = 5000
  ): Uint8Array | null {
    const view = this.readData(slotIndex, wait, timeoutMs);
    if (!view) return null;

    const copy = new Uint8Array(view.length);
    copy.set(view);
    return copy;
  }

  /**
   * Mark slot as being processed.
   */
  markProcessing(slotIndex: number): void {
    this.setStatus(slotIndex, SlotStatus.PROCESSING);
  }

  /**
   * Write result to a slot and signal completion.
   *
   * @param slotIndex - Index of the slot to write to
   * @param result - Result data to write
   * @returns true if successful, false if data too large
   */
  writeResult(slotIndex: number, result: Uint8Array): boolean {
    const maxDataSize = this.getMaxDataSize();

    if (result.length > maxDataSize) {
      this.signalError(slotIndex);
      return false;
    }

    // Write length
    const lengthView = new DataView(
      this.buffer,
      this.getLengthOffset(slotIndex),
      4
    );
    lengthView.setUint32(0, result.length, true);

    // Write data
    const dataOffset = this.getDataOffset(slotIndex);
    const dataView = new Uint8Array(this.buffer, dataOffset, maxDataSize);
    dataView.set(result);

    // Signal result ready
    this.setStatus(slotIndex, SlotStatus.RESULT_READY);

    return true;
  }

  /**
   * Write result directly to slot's data view (for in-place modification).
   * Use when result is already in the slot's buffer.
   *
   * @param slotIndex - Index of the slot
   * @param length - Length of result data
   */
  signalResultReady(slotIndex: number, length: number): void {
    // Write length
    const lengthView = new DataView(
      this.buffer,
      this.getLengthOffset(slotIndex),
      4
    );
    lengthView.setUint32(0, length, true);

    // Signal result ready
    this.setStatus(slotIndex, SlotStatus.RESULT_READY);
  }

  /**
   * Get writable view for result (for in-place modification).
   * Use with signalResultReady() for zero-copy result writing.
   */
  getResultView(slotIndex: number): Uint8Array {
    const dataOffset = this.getDataOffset(slotIndex);
    const maxDataSize = this.getMaxDataSize();
    return new Uint8Array(this.buffer, dataOffset, maxDataSize);
  }

  /**
   * Signal error for a slot.
   */
  signalError(slotIndex: number): void {
    this.setStatus(slotIndex, SlotStatus.ERROR);
  }

  /**
   * Check if SharedArrayBuffer is available.
   */
  static isAvailable(): boolean {
    try {
      new SharedArrayBuffer(1);
      return true;
    } catch {
      return false;
    }
  }
}
