import { EventEmitter } from 'events';
import {
  WriteConcern,
  WriteResult,
  PendingWrite,
  DEFAULT_WRITE_CONCERN_TIMEOUT,
  isWriteConcernAchieved,
  getHighestWriteConcernLevel,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface WriteAckManagerConfig {
  /** Default timeout for write acknowledgments (ms) */
  defaultTimeout?: number;
}

export interface WriteAckStats {
  /** Number of pending writes */
  pending: number;
  /** Pending writes by Write Concern level */
  byLevel: Record<WriteConcern, number>;
}

/**
 * Manages pending write acknowledgments for different Write Concern levels.
 *
 * Flow:
 * 1. Operation received → registerPending()
 * 2. Level achieved → notifyLevel()
 * 3. Target level reached → resolve promise, emit 'resolved'
 * 4. Timeout → resolve with partial success, emit 'timeout'
 *
 * @example
 * ```typescript
 * const ackManager = new WriteAckManager();
 *
 * // Register pending write
 * const promise = ackManager.registerPending(opId, WriteConcern.PERSISTED, 5000);
 *
 * // As processing progresses, notify levels
 * ackManager.notifyLevel(opId, WriteConcern.MEMORY);    // Validated
 * ackManager.notifyLevel(opId, WriteConcern.APPLIED);   // CRDT merged
 * ackManager.notifyLevel(opId, WriteConcern.REPLICATED); // Broadcast sent
 * ackManager.notifyLevel(opId, WriteConcern.PERSISTED);  // Storage written
 *
 * // Promise resolves when target level is reached
 * const result = await promise;
 * ```
 */
export class WriteAckManager extends EventEmitter {
  private pending: Map<string, PendingWrite> = new Map();
  private readonly defaultTimeout: number;

  constructor(config?: WriteAckManagerConfig) {
    super();
    this.defaultTimeout = config?.defaultTimeout ?? DEFAULT_WRITE_CONCERN_TIMEOUT;
  }

  /**
   * Register a pending write operation.
   * Returns a promise that resolves when target Write Concern is achieved.
   *
   * @param opId - Operation ID
   * @param writeConcern - Target Write Concern level
   * @param timeout - Optional timeout in ms (defaults to config or 5000ms)
   * @returns Promise that resolves with WriteResult
   */
  registerPending(
    opId: string,
    writeConcern: WriteConcern,
    timeout?: number
  ): Promise<WriteResult> {
    // FIRE_AND_FORGET resolves immediately
    if (writeConcern === WriteConcern.FIRE_AND_FORGET) {
      return Promise.resolve({
        success: true,
        opId,
        achievedLevel: WriteConcern.FIRE_AND_FORGET,
        latencyMs: 0,
      });
    }

    return new Promise((resolve, reject) => {
      const effectiveTimeout = timeout ?? this.defaultTimeout;
      const timestamp = Date.now();

      const pendingWrite: PendingWrite = {
        opId,
        writeConcern,
        timestamp,
        timeout: effectiveTimeout,
        resolve,
        reject,
        achievedLevels: new Set([WriteConcern.FIRE_AND_FORGET]),
      };

      // Set timeout
      pendingWrite.timeoutHandle = setTimeout(() => {
        this.handleTimeout(opId);
      }, effectiveTimeout);

      this.pending.set(opId, pendingWrite);

      logger.debug(
        { opId, writeConcern, timeout: effectiveTimeout },
        'Registered pending write'
      );

      // MEMORY level is achieved immediately after registration
      // (operation is validated and queued for processing)
      if (writeConcern === WriteConcern.MEMORY) {
        this.notifyLevel(opId, WriteConcern.MEMORY);
      }
    });
  }

  /**
   * Notify that a Write Concern level has been achieved for an operation.
   *
   * @param opId - Operation ID
   * @param level - Write Concern level that was achieved
   */
  notifyLevel(opId: string, level: WriteConcern): void {
    const pending = this.pending.get(opId);
    if (!pending) {
      // Operation not tracked (might be FIRE_AND_FORGET or already resolved)
      return;
    }

    pending.achievedLevels.add(level);

    logger.debug(
      { opId, level, target: pending.writeConcern },
      'Write Concern level achieved'
    );

    // Check if target level is achieved
    if (isWriteConcernAchieved(pending.achievedLevels, pending.writeConcern)) {
      this.resolvePending(opId, level);
    }
  }

  /**
   * Notify multiple operations that a Write Concern level has been achieved.
   * Useful for batch operations.
   *
   * @param opIds - Array of operation IDs
   * @param level - Write Concern level that was achieved
   */
  notifyLevelBatch(opIds: string[], level: WriteConcern): void {
    for (const opId of opIds) {
      this.notifyLevel(opId, level);
    }
  }

  /**
   * Check if an operation is still pending.
   *
   * @param opId - Operation ID
   * @returns true if operation is pending
   */
  isPending(opId: string): boolean {
    return this.pending.has(opId);
  }

  /**
   * Get the target Write Concern level for a pending operation.
   *
   * @param opId - Operation ID
   * @returns Target Write Concern level or undefined if not pending
   */
  getTargetLevel(opId: string): WriteConcern | undefined {
    return this.pending.get(opId)?.writeConcern;
  }

  /**
   * Get the highest achieved level for a pending operation.
   *
   * @param opId - Operation ID
   * @returns Highest achieved level or undefined if not pending
   */
  getAchievedLevel(opId: string): WriteConcern | undefined {
    const pending = this.pending.get(opId);
    if (!pending) return undefined;
    return getHighestWriteConcernLevel(pending.achievedLevels);
  }

  /**
   * Resolve a pending write with success.
   */
  private resolvePending(opId: string, achievedLevel: WriteConcern): void {
    const pending = this.pending.get(opId);
    if (!pending) return;

    // Clear timeout
    if (pending.timeoutHandle) {
      clearTimeout(pending.timeoutHandle);
    }

    const latencyMs = Date.now() - pending.timestamp;

    const result: WriteResult = {
      success: true,
      opId,
      achievedLevel,
      latencyMs,
    };

    pending.resolve(result);
    this.pending.delete(opId);

    logger.debug(
      { opId, achievedLevel, latencyMs },
      'Write resolved successfully'
    );

    this.emit('resolved', result);
  }

  /**
   * Handle timeout for a pending write.
   */
  private handleTimeout(opId: string): void {
    const pending = this.pending.get(opId);
    if (!pending) return;

    const highestAchieved = getHighestWriteConcernLevel(pending.achievedLevels);
    const latencyMs = Date.now() - pending.timestamp;

    // Resolve with partial success (achieved lower level than requested)
    const result: WriteResult = {
      success: false,
      opId,
      achievedLevel: highestAchieved,
      latencyMs,
      error: `Timeout: achieved ${highestAchieved}, requested ${pending.writeConcern}`,
    };

    pending.resolve(result);
    this.pending.delete(opId);

    logger.warn(
      { opId, requested: pending.writeConcern, achieved: highestAchieved, latencyMs },
      'Write timed out'
    );

    this.emit('timeout', {
      opId,
      requested: pending.writeConcern,
      achieved: highestAchieved,
      latencyMs,
    });
  }

  /**
   * Fail a pending write with an error.
   *
   * @param opId - Operation ID
   * @param error - Error message
   */
  failPending(opId: string, error: string): void {
    const pending = this.pending.get(opId);
    if (!pending) return;

    // Clear timeout
    if (pending.timeoutHandle) {
      clearTimeout(pending.timeoutHandle);
    }

    const latencyMs = Date.now() - pending.timestamp;
    const highestAchieved = getHighestWriteConcernLevel(pending.achievedLevels);

    const result: WriteResult = {
      success: false,
      opId,
      achievedLevel: highestAchieved,
      latencyMs,
      error,
    };

    pending.resolve(result);
    this.pending.delete(opId);

    logger.error({ opId, error, latencyMs }, 'Write failed');

    this.emit('failed', result);
  }

  /**
   * Get pending writes statistics.
   */
  getStats(): WriteAckStats {
    const byLevel: Record<WriteConcern, number> = {
      [WriteConcern.FIRE_AND_FORGET]: 0,
      [WriteConcern.MEMORY]: 0,
      [WriteConcern.APPLIED]: 0,
      [WriteConcern.REPLICATED]: 0,
      [WriteConcern.PERSISTED]: 0,
    };

    for (const pending of this.pending.values()) {
      byLevel[pending.writeConcern]++;
    }

    return { pending: this.pending.size, byLevel };
  }

  /**
   * Get all pending operation IDs.
   */
  getPendingIds(): string[] {
    return Array.from(this.pending.keys());
  }

  /**
   * Clear all pending writes (for shutdown).
   * Rejects all pending promises with an error.
   */
  clear(): void {
    const count = this.pending.size;

    for (const pending of this.pending.values()) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }
      pending.reject(new Error('WriteAckManager cleared'));
    }

    this.pending.clear();

    if (count > 0) {
      logger.info({ count }, 'WriteAckManager cleared');
    }
  }

  /**
   * Graceful shutdown - resolves all pending writes with their current achieved level.
   */
  shutdown(): void {
    const count = this.pending.size;

    for (const [opId, pending] of this.pending.entries()) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }

      const highestAchieved = getHighestWriteConcernLevel(pending.achievedLevels);
      const latencyMs = Date.now() - pending.timestamp;

      const result: WriteResult = {
        success: highestAchieved === pending.writeConcern,
        opId,
        achievedLevel: highestAchieved,
        latencyMs,
        error: highestAchieved !== pending.writeConcern
          ? `Shutdown: achieved ${highestAchieved}, requested ${pending.writeConcern}`
          : undefined,
      };

      pending.resolve(result);
    }

    this.pending.clear();

    if (count > 0) {
      logger.info({ count }, 'WriteAckManager shutdown');
    }
  }
}
