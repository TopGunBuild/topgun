/**
 * WriteConcernManager - Handles write concern tracking for SyncEngine
 *
 * Responsibilities:
 * - Register pending write concern promises for operations
 * - Resolve promises when server ACK is received
 * - Cancel all promises on disconnect
 * - Timeout handling for operations that don't receive ACK
 */

import { logger } from '../utils/logger';
import type { IWriteConcernManager, WriteConcernManagerConfig } from './types';

/**
 * Pending write concern promise state.
 */
interface PendingWriteConcernPromise {
  resolve: (result: any) => void;
  reject: (error: Error) => void;
  timeoutHandle?: ReturnType<typeof setTimeout>;
}

/**
 * WriteConcernManager implements IWriteConcernManager.
 *
 * Tracks write concern promises and resolves them when the server
 * sends OP_ACK with operation results.
 */
export class WriteConcernManager implements IWriteConcernManager {
  private readonly config: WriteConcernManagerConfig;

  // Pending write concern promises (single source of truth)
  private pendingWriteConcernPromises: Map<string, PendingWriteConcernPromise> = new Map();

  constructor(config: WriteConcernManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Register a pending Write Concern promise for an operation.
   * The promise will be resolved when the server sends an ACK with the operation result.
   *
   * @param opId - Operation ID
   * @param timeout - Timeout in ms (default: 5000)
   * @returns Promise that resolves with the Write Concern result
   */
  public registerWriteConcernPromise(opId: string, timeout: number = 5000): Promise<any> {
    const actualTimeout = timeout ?? this.config.defaultTimeout ?? 5000;

    return new Promise((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        this.pendingWriteConcernPromises.delete(opId);
        reject(new Error(`Write Concern timeout for operation ${opId}`));
      }, actualTimeout);

      this.pendingWriteConcernPromises.set(opId, {
        resolve,
        reject,
        timeoutHandle,
      });
    });
  }

  /**
   * Resolve a pending Write Concern promise with the server result.
   *
   * @param opId - Operation ID
   * @param result - Result from server ACK
   */
  public resolveWriteConcernPromise(opId: string, result: any): void {
    const pending = this.pendingWriteConcernPromises.get(opId);
    if (pending) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }
      pending.resolve(result);
      this.pendingWriteConcernPromises.delete(opId);
    }
  }

  /**
   * Cancel all pending Write Concern promises (e.g., on disconnect).
   */
  public cancelAllWriteConcernPromises(error: Error): void {
    for (const [opId, pending] of this.pendingWriteConcernPromises.entries()) {
      if (pending.timeoutHandle) {
        clearTimeout(pending.timeoutHandle);
      }
      pending.reject(error);
    }
    this.pendingWriteConcernPromises.clear();
  }
}
