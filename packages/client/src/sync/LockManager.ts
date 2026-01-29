/**
 * LockManager - Handles distributed lock operations for SyncEngine
 *
 * Responsibilities:
 * - Lock acquisition with timeout
 * - Lock release with acknowledgment
 * - Pending lock request tracking
 * - Message handlers for LOCK_GRANTED and LOCK_RELEASED
 */

import { logger } from '../utils/logger';
import type { ILockManager, LockManagerConfig } from './types';

/**
 * Pending lock request state.
 */
interface PendingLockRequest {
  resolve: (res: any) => void;
  reject: (err: any) => void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * LockManager implements ILockManager.
 *
 * Manages distributed locks with support for:
 * - Request/release pattern with fencing tokens
 * - Timeout handling for lost messages
 * - Server acknowledgment tracking
 */
export class LockManager implements ILockManager {
  private readonly config: LockManagerConfig;

  // Pending lock requests (single source of truth)
  private pendingLockRequests: Map<string, PendingLockRequest> = new Map();

  constructor(config: LockManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Request a distributed lock.
   */
  public requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }> {
    if (!this.config.isAuthenticated()) {
      return Promise.reject(new Error('Not connected or authenticated'));
    }

    return new Promise((resolve, reject) => {
      // Timeout if no response (server might be down or message lost)
      // We set a client-side timeout slightly larger than TTL if TTL is short,
      // but usually we want a separate "Wait Timeout".
      // For now, use a fixed 30s timeout for the *response*.
      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          reject(new Error('Lock request timed out waiting for server response'));
        }
      }, 30000);

      this.pendingLockRequests.set(requestId, { resolve, reject, timer });

      try {
        const sent = this.config.sendMessage({
          type: 'LOCK_REQUEST',
          payload: { requestId, name, ttl }
        });
        if (!sent) {
          clearTimeout(timer);
          this.pendingLockRequests.delete(requestId);
          reject(new Error('Failed to send lock request'));
        }
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        reject(e);
      }
    });
  }

  /**
   * Release a distributed lock.
   */
  public releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean> {
    if (!this.config.isOnline()) return Promise.resolve(false);

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          // Resolve false on timeout? Or reject?
          // Release is usually fire-and-forget but we wanted ACK.
          resolve(false);
        }
      }, 5000);

      this.pendingLockRequests.set(requestId, { resolve, reject, timer });

      try {
        const sent = this.config.sendMessage({
          type: 'LOCK_RELEASE',
          payload: { requestId, name, fencingToken }
        });
        if (!sent) {
          clearTimeout(timer);
          this.pendingLockRequests.delete(requestId);
          resolve(false);
        }
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        resolve(false);
      }
    });
  }

  /**
   * Handle lock granted message from server.
   */
  public handleLockGranted(requestId: string, fencingToken: number): void {
    const req = this.pendingLockRequests.get(requestId);
    if (req) {
      clearTimeout(req.timer);
      this.pendingLockRequests.delete(requestId);
      req.resolve({ fencingToken });
    }
  }

  /**
   * Handle lock released message from server.
   */
  public handleLockReleased(requestId: string, success: boolean): void {
    const req = this.pendingLockRequests.get(requestId);
    if (req) {
      clearTimeout(req.timer);
      this.pendingLockRequests.delete(requestId);
      req.resolve(success);
    }
  }
}
