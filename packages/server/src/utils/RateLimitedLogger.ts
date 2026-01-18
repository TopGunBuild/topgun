import { logger } from './logger';

/**
 * Minimal logger interface for dependency injection.
 */
export interface BaseLogger {
    warn(obj: object, msg: string): void;
    error(obj: object, msg: string): void;
}

/**
 * Configuration for the rate-limited logger.
 */
export interface RateLimitConfig {
    /** Time window in milliseconds (default: 10000) */
    windowMs: number;
    /** Maximum logs per window per key (default: 5) */
    maxPerWindow: number;
    /** Optional custom logger for testing/customization */
    baseLogger?: BaseLogger;
}

/**
 * Internal state for tracking a single key's logging window.
 */
export interface WindowState {
    /** Number of logs emitted in current window */
    count: number;
    /** Number of logs suppressed in current window */
    suppressedCount: number;
    /** Timestamp when the current window started */
    windowStart: number;
}

const DEFAULT_CONFIG: RateLimitConfig = {
    windowMs: 10000,
    maxPerWindow: 5,
};

/**
 * A rate-limited logger that prevents log flooding by throttling logs per key.
 *
 * When logs exceed the configured threshold within a time window, subsequent logs
 * are suppressed. When the window resets, a summary of suppressed messages is emitted.
 *
 * Use case: Preventing malicious clients from flooding logs with invalid message errors.
 */
export class RateLimitedLogger {
    private states: Map<string, WindowState> = new Map();
    private config: RateLimitConfig;
    private baseLogger: BaseLogger;

    constructor(config: Partial<RateLimitConfig> = {}) {
        this.config = { ...DEFAULT_CONFIG, ...config };
        this.baseLogger = config.baseLogger ?? logger;
    }

    /**
     * Log a warning message with rate limiting.
     * @param key - Unique key for rate limiting (e.g., `invalid-message:${clientId}`)
     * @param obj - Object to include in the log entry
     * @param msg - Log message
     */
    warn(key: string, obj: object, msg: string): void {
        if (this.shouldLog(key)) {
            this.baseLogger.warn(obj, msg);
        }
    }

    /**
     * Log an error message with rate limiting.
     * @param key - Unique key for rate limiting (e.g., `invalid-message:${clientId}`)
     * @param obj - Object to include in the log entry
     * @param msg - Log message
     */
    error(key: string, obj: object, msg: string): void {
        if (this.shouldLog(key)) {
            this.baseLogger.error(obj, msg);
        }
    }

    /**
     * Determine if a log should be emitted for the given key.
     * Handles window expiration, suppression counting, and summary emission.
     */
    private shouldLog(key: string): boolean {
        const now = Date.now();
        let state = this.states.get(key);

        // Check if we need to start a new window
        if (!state || now - state.windowStart >= this.config.windowMs) {
            // Log suppression summary if there were any suppressed messages
            if (state && state.suppressedCount > 0) {
                this.baseLogger.warn(
                    { key, suppressedCount: state.suppressedCount, windowMs: this.config.windowMs },
                    `Rate-limited logger suppressed ${state.suppressedCount} messages for key "${key}"`
                );
            }

            // Start new window
            state = {
                count: 0,
                suppressedCount: 0,
                windowStart: now,
            };
            this.states.set(key, state);
        }

        // Check if under limit
        if (state.count < this.config.maxPerWindow) {
            state.count++;
            return true;
        }

        // Over limit - suppress
        state.suppressedCount++;
        return false;
    }

    /**
     * Remove stale entries that haven't been accessed recently.
     * Call periodically for long-running servers to prevent memory leaks.
     * @param maxAgeMs - Remove entries older than this (default: 5 * windowMs)
     */
    cleanup(maxAgeMs?: number): void {
        const now = Date.now();
        const threshold = maxAgeMs ?? this.config.windowMs * 5;

        for (const [key, state] of this.states) {
            if (now - state.windowStart >= threshold) {
                // Emit final summary if there were suppressed messages
                if (state.suppressedCount > 0) {
                    this.baseLogger.warn(
                        { key, suppressedCount: state.suppressedCount, windowMs: this.config.windowMs },
                        `Rate-limited logger suppressed ${state.suppressedCount} messages for key "${key}" (cleanup)`
                    );
                }
                this.states.delete(key);
            }
        }
    }

    /**
     * Get the number of tracked keys (for monitoring/testing).
     */
    getTrackedKeyCount(): number {
        return this.states.size;
    }
}
