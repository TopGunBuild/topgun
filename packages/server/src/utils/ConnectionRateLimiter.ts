/**
 * ConnectionRateLimiter - Rate limiter for incoming WebSocket connections.
 *
 * Implements connection rate limiting to prevent connection storms and
 * protect the server from being overwhelmed during high load scenarios.
 *
 * Features:
 * - Limits connections per second to prevent TCP backlog exhaustion
 * - Tracks pending connections (in-progress handshakes)
 * - Provides graceful rejection when limits are exceeded
 * - Auto-resets counters after cooldown period
 */

import { logger } from './logger';

export interface RateLimiterConfig {
    /** Maximum new connections allowed per second (default: 100) */
    maxConnectionsPerSecond: number;
    /** Maximum pending connections waiting for handshake (default: 1000) */
    maxPendingConnections: number;
    /** Cooldown period in ms after which counters reset (default: 1000) */
    cooldownMs: number;
}

export interface RateLimiterStats {
    /** Current connections per second rate */
    connectionsPerSecond: number;
    /** Number of connections currently pending (handshake in progress) */
    pendingConnections: number;
    /** Total connections established since start */
    totalConnections: number;
    /** Total connections rejected due to rate limiting */
    totalRejected: number;
}

const DEFAULT_CONFIG: RateLimiterConfig = {
    maxConnectionsPerSecond: 100,
    maxPendingConnections: 1000,
    cooldownMs: 1000,
};

export class ConnectionRateLimiter {
    private config: RateLimiterConfig;

    /** Connection attempts in current window */
    private connectionCount: number = 0;

    /** Timestamp when current window started */
    private windowStart: number = Date.now();

    /** Currently pending connections (handshake in progress) */
    private pendingCount: number = 0;

    /** Total connections established since start */
    private totalConnections: number = 0;

    /** Total connections rejected since start */
    private totalRejected: number = 0;

    constructor(config?: Partial<RateLimiterConfig>) {
        this.config = {
            ...DEFAULT_CONFIG,
            ...config,
        };
    }

    /**
     * Check if a new connection should be accepted.
     * @returns true if the connection should be accepted, false if it should be rejected
     */
    shouldAccept(): boolean {
        this.maybeResetWindow();

        // Check if pending connections limit is exceeded
        if (this.pendingCount >= this.config.maxPendingConnections) {
            logger.debug(
                { pendingCount: this.pendingCount, maxPending: this.config.maxPendingConnections },
                'Connection rejected: pending connections limit exceeded'
            );
            return false;
        }

        // Check if connections per second limit is exceeded
        if (this.connectionCount >= this.config.maxConnectionsPerSecond) {
            logger.debug(
                { connectionCount: this.connectionCount, maxPerSecond: this.config.maxConnectionsPerSecond },
                'Connection rejected: rate limit exceeded'
            );
            return false;
        }

        return true;
    }

    /**
     * Register a connection attempt.
     * Call this when a new connection is initiated (before handshake completes).
     */
    onConnectionAttempt(): void {
        this.maybeResetWindow();
        this.connectionCount++;
        this.pendingCount++;
    }

    /**
     * Register that a connection has been established (handshake complete).
     * Call this when the connection is fully established and authenticated.
     */
    onConnectionEstablished(): void {
        this.pendingCount = Math.max(0, this.pendingCount - 1);
        this.totalConnections++;
    }

    /**
     * Register that a connection has been closed.
     * Call this when a pending connection is closed before completing handshake.
     */
    onConnectionClosed(): void {
        // If connection was pending, decrement pending count
        // Note: We can't distinguish between pending and established connections here,
        // so this is a best-effort tracking. In practice, onConnectionEstablished
        // should be called first for established connections.
    }

    /**
     * Register that a connection was rejected.
     * Call this when shouldAccept() returns false and the connection is rejected.
     */
    onConnectionRejected(): void {
        this.totalRejected++;
    }

    /**
     * Decrease pending count when a connection times out or fails.
     * Call this when a pending connection fails to complete handshake.
     */
    onPendingConnectionFailed(): void {
        this.pendingCount = Math.max(0, this.pendingCount - 1);
    }

    /**
     * Get current rate limiter statistics.
     */
    getStats(): RateLimiterStats {
        this.maybeResetWindow();

        // Calculate actual connections per second
        const elapsed = Date.now() - this.windowStart;
        const connectionsPerSecond = elapsed > 0
            ? Math.round((this.connectionCount / elapsed) * 1000)
            : this.connectionCount;

        return {
            connectionsPerSecond,
            pendingConnections: this.pendingCount,
            totalConnections: this.totalConnections,
            totalRejected: this.totalRejected,
        };
    }

    /**
     * Reset the rate limiter state.
     * Useful for testing or when recovering from errors.
     */
    reset(): void {
        this.connectionCount = 0;
        this.windowStart = Date.now();
        this.pendingCount = 0;
        this.totalConnections = 0;
        this.totalRejected = 0;
    }

    /**
     * Update configuration at runtime.
     */
    updateConfig(config: Partial<RateLimiterConfig>): void {
        this.config = {
            ...this.config,
            ...config,
        };
    }

    /**
     * Check if window has expired and reset if needed.
     */
    private maybeResetWindow(): void {
        const now = Date.now();
        if (now - this.windowStart >= this.config.cooldownMs) {
            this.connectionCount = 0;
            this.windowStart = now;
        }
    }
}
