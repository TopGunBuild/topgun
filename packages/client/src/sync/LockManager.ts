/**
 * LockManager - Handles distributed lock operations for SyncEngine
 *
 * Responsibilities:
 * - Lock acquisition with TTL-coordinated timeout
 * - Lock release with acknowledgment and diagnostic logging
 * - Pending lock request tracking
 * - Message handlers for LOCK_GRANTED and LOCK_RELEASED
 */

import { logger } from '../utils/logger';
import type { ILockManager, LockManagerConfig } from './types';

/**
 * Grace period added to the TTL when computing the client-side response timeout
 * for lock acquisition. This covers realistic network round-trip and server
 * processing latency so the client does not give up before the server can respond.
 */
const ACQUIRE_RESPONSE_GRACE_MS = 5000;

/**
 * Minimum client-side response timeout for lock acquisition regardless of TTL.
 * Ensures very short TTLs (including 0) still wait long enough for a response.
 */
const MIN_ACQUIRE_RESPONSE_TIMEOUT_MS = 5000;

/**
 * Fixed client-side timeout for lock release acknowledgment.
 * Releases are lighter-weight than acquisitions (no TTL timer setup server-side),
 * so a shorter fixed bound is appropriate. ACK disambiguation is handled via
 * debug-level logs rather than by adjusting this value.
 */
const RELEASE_RESPONSE_TIMEOUT_MS = 5000;

/**
 * Pending lock request state.
 *
 * Only `resolve` and `timer` are stored on the map entry. The Promise executor's
 * `reject` is captured in the surrounding closure (for requestLock timeout /
 * send-failure paths) and never read back off the stored entry, so it is not
 * kept here.
 */
interface PendingLockRequest {
  resolve: (res: any) => void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * LockManager implements ILockManager.
 *
 * Manages distributed locks with support for:
 * - Request/release pattern with fencing tokens
 * - TTL-coordinated timeout handling for lost messages
 * - Server acknowledgment tracking with diagnostic logging
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
   *
   * The client-side response timeout is derived from the requested TTL:
   *   `max(ttl + ACQUIRE_RESPONSE_GRACE_MS, MIN_ACQUIRE_RESPONSE_TIMEOUT_MS)`
   *
   * This ensures the client waits long enough for the server to respond within
   * the TTL window. For short TTLs the floor prevents leaking pending requests.
   * For long TTLs the grace period covers network + server processing latency.
   *
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param ttl - Lock lease duration in milliseconds (server-side)
   * @returns Promise that resolves with fencing token on grant
   */
  public requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }> {
    if (!this.config.isAuthenticated()) {
      return Promise.reject(new Error('Not connected or authenticated'));
    }

    return new Promise((resolve, reject) => {
      // Response timeout scales with TTL so the client does not reject before
      // the server's TTL window elapses. The grace period covers network latency
      // and server processing. The floor prevents indefinite waits for ttl=0.
      const responseTimeoutMs = Math.max(ttl + ACQUIRE_RESPONSE_GRACE_MS, MIN_ACQUIRE_RESPONSE_TIMEOUT_MS);

      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          reject(new Error('Lock request timed out waiting for server response'));
        }
      }, responseTimeoutMs);

      this.pendingLockRequests.set(requestId, { resolve, timer });

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
   *
   * Returns `true` only when the server responds with `LOCK_RELEASED { success: true }`.
   * Returns `false` for all other outcomes (no ACK, offline, send failure, server rejection).
   * Each failure path emits a `logger.debug` call with a distinct `reason` code for diagnostics:
   *   - `timeout`      — no ACK arrived within RELEASE_RESPONSE_TIMEOUT_MS
   *   - `offline`      — client was not online when release was attempted
   *   - `send_failed`  — sendMessage returned false without throwing
   *   - `send_threw`   — sendMessage threw an exception
   *   - `server_rejected` — server responded success: false (emitted from handleLockReleased)
   *
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param fencingToken - Fencing token from lock grant
   * @returns Promise that resolves with true only on server success: true ACK
   */
  public releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean> {
    if (!this.config.isOnline()) {
      logger.debug({ name, requestId, reason: 'offline' }, 'LockManager: release not sent');
      return Promise.resolve(false);
    }

    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        if (this.pendingLockRequests.has(requestId)) {
          this.pendingLockRequests.delete(requestId);
          logger.debug({ name, requestId, reason: 'timeout' }, 'LockManager: release ACK timeout');
          resolve(false);
        }
      }, RELEASE_RESPONSE_TIMEOUT_MS);

      this.pendingLockRequests.set(requestId, { resolve, timer });

      try {
        const sent = this.config.sendMessage({
          type: 'LOCK_RELEASE',
          payload: { requestId, name, fencingToken }
        });
        if (!sent) {
          clearTimeout(timer);
          this.pendingLockRequests.delete(requestId);
          logger.debug({ name, requestId, reason: 'send_failed' }, 'LockManager: release send failed');
          resolve(false);
        }
      } catch (e) {
        clearTimeout(timer);
        this.pendingLockRequests.delete(requestId);
        logger.debug({ name, requestId, reason: 'send_threw', error: (e as Error).message }, 'LockManager: release send threw');
        resolve(false);
      }
    });
  }

  /**
   * Handle lock granted message from server.
   */
  public handleLockGranted(requestId: string, _name: string, fencingToken: number): void {
    const req = this.pendingLockRequests.get(requestId);
    if (req) {
      clearTimeout(req.timer);
      this.pendingLockRequests.delete(requestId);
      req.resolve({ fencingToken });
    }
  }

  /**
   * Handle lock released message from server.
   *
   * Empty requestId (fire-and-forget ACK per SPEC-216 Assumption #7) is a safe
   * no-op — no pending-map lookup is performed to avoid corruption.
   */
  public handleLockReleased(requestId: string, name: string, success: boolean): void {
    if (requestId === '') {
      logger.debug({ name, success }, 'LockManager: LOCK_RELEASED with empty requestId (fire-and-forget ACK)');
      return;
    }
    const pending = this.pendingLockRequests.get(requestId);
    if (!pending) return;
    this.pendingLockRequests.delete(requestId);
    clearTimeout(pending.timer);
    if (!success) {
      logger.debug({ name, requestId, reason: 'server_rejected' }, 'LockManager: release rejected by server');
    }
    pending.resolve(success);
  }
}
