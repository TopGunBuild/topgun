/**
 * FencingManager - Epoch-based Fencing for Split-Brain Protection
 *
 * Implements epoch-based fencing to prevent zombie writes in distributed systems.
 * When a node is suspected or confirmed failed, the cluster epoch increments,
 * and operations with stale epochs are rejected.
 *
 * This is similar to Hazelcast's FencingManager which uses fencing tokens
 * to ensure only one leader can perform operations at a time.
 *
 * Use cases:
 * - Preventing zombie writes after network partition heals
 * - Ensuring leader uniqueness in leader-elected scenarios
 * - Protecting critical sections across cluster members
 */

import { EventEmitter } from 'events';
import { logger } from '../utils/logger';

export interface FencingManagerConfig {
  /** Initial epoch value. Default: 0 */
  initialEpoch: number;
  /** Grace period after epoch change before rejecting stale ops (ms). Default: 1000 */
  gracePeriodMs: number;
}

export const DEFAULT_FENCING_CONFIG: FencingManagerConfig = {
  initialEpoch: 0,
  gracePeriodMs: 1000,
};

export interface EpochInfo {
  /** Current epoch number */
  epoch: number;
  /** Node ID that caused the last epoch change */
  changedBy?: string;
  /** Timestamp of last epoch change */
  changedAt: number;
  /** Reason for last epoch change */
  reason?: string;
}

export interface FencingToken {
  /** Epoch at time of token creation */
  epoch: number;
  /** Node ID that created the token */
  nodeId: string;
  /** Timestamp when token was created */
  createdAt: number;
  /** Optional resource identifier this token is for */
  resource?: string;
}

export class FencingManager extends EventEmitter {
  private config: FencingManagerConfig;
  private currentEpoch: number;
  private epochHistory: EpochInfo[] = [];
  private activeTokens: Map<string, FencingToken> = new Map();
  private graceEndTime: number = 0;

  constructor(config: Partial<FencingManagerConfig> = {}) {
    super();
    this.config = { ...DEFAULT_FENCING_CONFIG, ...config };
    this.currentEpoch = this.config.initialEpoch;

    // Record initial epoch
    this.epochHistory.push({
      epoch: this.currentEpoch,
      changedAt: Date.now(),
      reason: 'initialization',
    });
  }

  /**
   * Get the current epoch.
   */
  getEpoch(): number {
    return this.currentEpoch;
  }

  /**
   * Get epoch info including history.
   */
  getEpochInfo(): EpochInfo {
    return this.epochHistory[this.epochHistory.length - 1];
  }

  /**
   * Increment the epoch due to a cluster event.
   * Returns the new epoch value.
   */
  incrementEpoch(reason: string, changedBy?: string): number {
    const previousEpoch = this.currentEpoch;
    this.currentEpoch++;

    const epochInfo: EpochInfo = {
      epoch: this.currentEpoch,
      changedBy,
      changedAt: Date.now(),
      reason,
    };

    this.epochHistory.push(epochInfo);

    // Keep only last 100 epoch changes
    if (this.epochHistory.length > 100) {
      this.epochHistory.shift();
    }

    // Set grace period end time
    this.graceEndTime = Date.now() + this.config.gracePeriodMs;

    // Invalidate all tokens from previous epoch
    this.invalidateStaleTokens();

    logger.info(
      { previousEpoch, newEpoch: this.currentEpoch, reason, changedBy },
      'Epoch incremented'
    );

    this.emit('epochChanged', {
      previousEpoch,
      newEpoch: this.currentEpoch,
      reason,
      changedBy,
    });

    return this.currentEpoch;
  }

  /**
   * Check if an epoch is valid (current or within grace period).
   */
  isEpochValid(epoch: number): boolean {
    if (epoch === this.currentEpoch) {
      return true;
    }

    // Allow one epoch behind during grace period
    if (epoch === this.currentEpoch - 1 && Date.now() < this.graceEndTime) {
      return true;
    }

    return false;
  }

  /**
   * Check if an epoch is stale (too old).
   */
  isEpochStale(epoch: number): boolean {
    return !this.isEpochValid(epoch);
  }

  /**
   * Create a fencing token for a resource.
   * The token is tied to the current epoch and can be used to validate operations.
   */
  createToken(nodeId: string, resource?: string): FencingToken {
    const token: FencingToken = {
      epoch: this.currentEpoch,
      nodeId,
      createdAt: Date.now(),
      resource,
    };

    const tokenKey = resource ? `${nodeId}:${resource}` : nodeId;
    this.activeTokens.set(tokenKey, token);

    logger.debug({ token }, 'Fencing token created');

    return token;
  }

  /**
   * Validate a fencing token.
   * Returns true if the token is still valid (epoch matches or within grace).
   */
  validateToken(token: FencingToken): boolean {
    if (!this.isEpochValid(token.epoch)) {
      logger.debug(
        { tokenEpoch: token.epoch, currentEpoch: this.currentEpoch },
        'Token rejected: stale epoch'
      );
      return false;
    }

    // Check if token is still active
    const tokenKey = token.resource ? `${token.nodeId}:${token.resource}` : token.nodeId;
    const activeToken = this.activeTokens.get(tokenKey);

    if (!activeToken) {
      logger.debug({ token }, 'Token rejected: not found in active tokens');
      return false;
    }

    // Token must match exactly
    if (
      activeToken.epoch !== token.epoch ||
      activeToken.nodeId !== token.nodeId ||
      activeToken.createdAt !== token.createdAt
    ) {
      logger.debug({ token, activeToken }, 'Token rejected: mismatch');
      return false;
    }

    return true;
  }

  /**
   * Release a fencing token.
   */
  releaseToken(nodeId: string, resource?: string): void {
    const tokenKey = resource ? `${nodeId}:${resource}` : nodeId;
    this.activeTokens.delete(tokenKey);
    logger.debug({ nodeId, resource }, 'Fencing token released');
  }

  /**
   * Invalidate all tokens from previous epochs.
   */
  private invalidateStaleTokens(): void {
    const staleTokens: string[] = [];

    for (const [key, token] of this.activeTokens) {
      if (this.isEpochStale(token.epoch)) {
        staleTokens.push(key);
      }
    }

    for (const key of staleTokens) {
      const token = this.activeTokens.get(key);
      this.activeTokens.delete(key);

      if (token) {
        this.emit('tokenInvalidated', token);
        logger.debug({ token }, 'Stale token invalidated');
      }
    }

    if (staleTokens.length > 0) {
      logger.info({ count: staleTokens.length }, 'Invalidated stale fencing tokens');
    }
  }

  /**
   * Get all active tokens (for debugging/monitoring).
   */
  getActiveTokens(): FencingToken[] {
    return Array.from(this.activeTokens.values());
  }

  /**
   * Get epoch history.
   */
  getEpochHistory(): EpochInfo[] {
    return [...this.epochHistory];
  }

  /**
   * Called when a node failure is detected.
   * Increments epoch to prevent zombie writes from the failed node.
   */
  onNodeFailure(nodeId: string): void {
    this.incrementEpoch(`node_failure:${nodeId}`, nodeId);

    // Remove any tokens held by the failed node
    for (const [key, token] of this.activeTokens) {
      if (token.nodeId === nodeId) {
        this.activeTokens.delete(key);
        this.emit('tokenInvalidated', token);
        logger.info({ token }, 'Token invalidated due to node failure');
      }
    }
  }

  /**
   * Called when cluster membership changes.
   */
  onMembershipChange(reason: string): void {
    this.incrementEpoch(`membership_change:${reason}`);
  }

  /**
   * Get metrics for monitoring.
   */
  getMetrics(): {
    currentEpoch: number;
    activeTokens: number;
    inGracePeriod: boolean;
    epochChanges: number;
  } {
    return {
      currentEpoch: this.currentEpoch,
      activeTokens: this.activeTokens.size,
      inGracePeriod: Date.now() < this.graceEndTime,
      epochChanges: this.epochHistory.length,
    };
  }
}
