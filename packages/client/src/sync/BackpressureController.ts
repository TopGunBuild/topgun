/**
 * BackpressureController
 *
 * Handles all backpressure operations for SyncEngine:
 * - Monitors pending operations count via shared opLog reference
 * - Implements pause/throw/drop-oldest strategies
 * - Emits high/low water mark events
 * - Manages paused writes and capacity waiting
 */

import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from '../BackpressureConfig';
import { BackpressureError } from '../errors/BackpressureError';
import type { OpLogEntry } from '../SyncEngine';
import type { IBackpressureController, BackpressureControllerConfig } from './types';
import { logger } from '../utils/logger';

/**
 * BackpressureController manages flow control for pending operations.
 *
 * It receives a shared reference to opLog from SyncEngine and uses it
 * to count pending operations and implement drop-oldest strategy.
 * SyncEngine retains ownership of opLog.
 */
export class BackpressureController implements IBackpressureController {
  private readonly config: BackpressureConfig;
  private readonly opLog: OpLogEntry[];

  // Internal state
  private backpressurePaused: boolean = false;
  private waitingForCapacity: Array<() => void> = [];
  private highWaterMarkEmitted: boolean = false;
  private backpressureListeners: Map<string, Set<(...args: any[]) => void>> = new Map();

  constructor(controllerConfig: BackpressureControllerConfig) {
    this.config = controllerConfig.config;
    this.opLog = controllerConfig.opLog; // Shared reference, not a copy
  }

  // ============================================
  // Status Methods
  // ============================================

  /**
   * Get the current number of pending (unsynced) operations.
   */
  public getPendingOpsCount(): number {
    return this.opLog.filter(op => !op.synced).length;
  }

  /**
   * Get the current backpressure status.
   */
  public getBackpressureStatus(): BackpressureStatus {
    const pending = this.getPendingOpsCount();
    const max = this.config.maxPendingOps;
    return {
      pending,
      max,
      percentage: max > 0 ? pending / max : 0,
      isPaused: this.backpressurePaused,
      strategy: this.config.strategy,
    };
  }

  /**
   * Returns true if writes are currently paused due to backpressure.
   */
  public isBackpressurePaused(): boolean {
    return this.backpressurePaused;
  }

  // ============================================
  // Check Methods
  // ============================================

  /**
   * Check backpressure before adding a new operation.
   * May pause, throw, or drop depending on strategy.
   */
  public async checkBackpressure(): Promise<void> {
    const pendingCount = this.getPendingOpsCount();

    if (pendingCount < this.config.maxPendingOps) {
      return; // Capacity available
    }

    switch (this.config.strategy) {
      case 'pause':
        await this.waitForCapacity();
        break;
      case 'throw':
        throw new BackpressureError(
          pendingCount,
          this.config.maxPendingOps
        );
      case 'drop-oldest':
        this.dropOldestOp();
        break;
    }
  }

  /**
   * Check high water mark and emit event if threshold reached.
   */
  public checkHighWaterMark(): void {
    const pendingCount = this.getPendingOpsCount();
    const threshold = Math.floor(
      this.config.maxPendingOps * this.config.highWaterMark
    );

    if (pendingCount >= threshold && !this.highWaterMarkEmitted) {
      this.highWaterMarkEmitted = true;
      logger.warn(
        { pending: pendingCount, max: this.config.maxPendingOps },
        'Backpressure high water mark reached'
      );
      this.emitBackpressureEvent('backpressure:high', {
        pending: pendingCount,
        max: this.config.maxPendingOps,
      });
    }
  }

  /**
   * Check low water mark and resume paused writes if threshold reached.
   */
  public checkLowWaterMark(): void {
    const pendingCount = this.getPendingOpsCount();
    const lowThreshold = Math.floor(
      this.config.maxPendingOps * this.config.lowWaterMark
    );
    const highThreshold = Math.floor(
      this.config.maxPendingOps * this.config.highWaterMark
    );

    // Reset high water mark flag when below high threshold
    if (pendingCount < highThreshold && this.highWaterMarkEmitted) {
      this.highWaterMarkEmitted = false;
    }

    // Emit low water mark event when crossing below threshold
    if (pendingCount <= lowThreshold) {
      if (this.backpressurePaused) {
        this.backpressurePaused = false;
        logger.info(
          { pending: pendingCount, max: this.config.maxPendingOps },
          'Backpressure low water mark reached, resuming writes'
        );
        this.emitBackpressureEvent('backpressure:low', {
          pending: pendingCount,
          max: this.config.maxPendingOps,
        });
        this.emitBackpressureEvent('backpressure:resumed');

        // Resume all waiting writes
        const waiting = this.waitingForCapacity;
        this.waitingForCapacity = [];
        for (const resolve of waiting) {
          resolve();
        }
      }
    }
  }

  // ============================================
  // Event Methods
  // ============================================

  /**
   * Subscribe to backpressure events.
   * @param event Event name: 'backpressure:high', 'backpressure:low', 'backpressure:paused', 'backpressure:resumed', 'operation:dropped'
   * @param listener Callback function
   * @returns Unsubscribe function
   */
  public onBackpressure(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void {
    if (!this.backpressureListeners.has(event)) {
      this.backpressureListeners.set(event, new Set());
    }
    this.backpressureListeners.get(event)!.add(listener);

    return () => {
      this.backpressureListeners.get(event)?.delete(listener);
    };
  }

  // ============================================
  // Private Methods
  // ============================================

  /**
   * Emit a backpressure event to all listeners.
   */
  private emitBackpressureEvent(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    data?: BackpressureThresholdEvent | OperationDroppedEvent
  ): void {
    const listeners = this.backpressureListeners.get(event);
    if (listeners) {
      for (const listener of listeners) {
        try {
          listener(data);
        } catch (err) {
          logger.error({ err, event }, 'Error in backpressure event listener');
        }
      }
    }
  }

  /**
   * Wait for capacity to become available (used by 'pause' strategy).
   */
  private async waitForCapacity(): Promise<void> {
    if (!this.backpressurePaused) {
      this.backpressurePaused = true;
      logger.warn('Backpressure paused - waiting for capacity');
      this.emitBackpressureEvent('backpressure:paused');
    }

    return new Promise<void>((resolve) => {
      this.waitingForCapacity.push(resolve);
    });
  }

  /**
   * Drop the oldest pending operation (used by 'drop-oldest' strategy).
   * Modifies opLog via shared reference.
   */
  private dropOldestOp(): void {
    // Find oldest unsynced operation by array order (oldest first)
    const oldestIndex = this.opLog.findIndex(op => !op.synced);

    if (oldestIndex !== -1) {
      const dropped = this.opLog[oldestIndex];
      this.opLog.splice(oldestIndex, 1);

      logger.warn(
        { opId: dropped.id, mapName: dropped.mapName, key: dropped.key },
        'Dropped oldest pending operation due to backpressure'
      );

      this.emitBackpressureEvent('operation:dropped', {
        opId: dropped.id,
        mapName: dropped.mapName,
        opType: dropped.opType,
        key: dropped.key,
      });
    }
  }
}

export type { BackpressureControllerConfig } from './types';
