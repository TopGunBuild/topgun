/**
 * Centralized timer management for proper cleanup during shutdown.
 * Tracks all setTimeout/setInterval handles for coordinated disposal.
 */
export class TimerRegistry {
    private timeouts: Map<string, NodeJS.Timeout> = new Map();
    private intervals: Map<string, NodeJS.Timeout> = new Map();
    private idCounter = 0;

    /**
     * Generate a unique ID for a timer.
     */
    private generateId(prefix: string): string {
        return `${prefix}-${++this.idCounter}-${Date.now()}`;
    }

    /**
     * Register a timeout with optional ID (auto-generated if not provided).
     * @param callback The function to call after the delay
     * @param delayMs The delay in milliseconds
     * @param id Optional unique identifier for this timeout
     * @returns The ID used to identify this timeout
     */
    setTimeout(callback: () => void, delayMs: number, id?: string): string {
        const timerId = id ?? this.generateId('timeout');

        // Clear existing timeout with same ID if any
        if (this.timeouts.has(timerId)) {
            clearTimeout(this.timeouts.get(timerId)!);
        }

        const handle = setTimeout(() => {
            this.timeouts.delete(timerId);
            callback();
        }, delayMs);

        this.timeouts.set(timerId, handle);
        return timerId;
    }

    /**
     * Register an interval with optional ID (auto-generated if not provided).
     * @param callback The function to call on each interval
     * @param intervalMs The interval in milliseconds
     * @param id Optional unique identifier for this interval
     * @returns The ID used to identify this interval
     */
    setInterval(callback: () => void, intervalMs: number, id?: string): string {
        const timerId = id ?? this.generateId('interval');

        // Clear existing interval with same ID if any
        if (this.intervals.has(timerId)) {
            clearInterval(this.intervals.get(timerId)!);
        }

        const handle = setInterval(callback, intervalMs);
        this.intervals.set(timerId, handle);
        return timerId;
    }

    /**
     * Clear a specific timeout by ID.
     * @param id The timeout ID to clear
     * @returns true if the timeout was found and cleared, false otherwise
     */
    clearTimeout(id: string): boolean {
        const handle = this.timeouts.get(id);
        if (handle) {
            clearTimeout(handle);
            this.timeouts.delete(id);
            return true;
        }
        return false;
    }

    /**
     * Clear a specific interval by ID.
     * @param id The interval ID to clear
     * @returns true if the interval was found and cleared, false otherwise
     */
    clearInterval(id: string): boolean {
        const handle = this.intervals.get(id);
        if (handle) {
            clearInterval(handle);
            this.intervals.delete(id);
            return true;
        }
        return false;
    }

    /**
     * Clear all registered timers (for shutdown).
     * @returns Object with counts of cleared timeouts and intervals
     */
    clear(): { timeoutsCleared: number; intervalsCleared: number } {
        const timeoutsCleared = this.timeouts.size;
        const intervalsCleared = this.intervals.size;

        for (const handle of this.timeouts.values()) {
            clearTimeout(handle);
        }
        this.timeouts.clear();

        for (const handle of this.intervals.values()) {
            clearInterval(handle);
        }
        this.intervals.clear();

        return { timeoutsCleared, intervalsCleared };
    }

    /**
     * Get count of active timers (for debugging).
     * @returns Object with counts of active timeouts and intervals
     */
    getActiveCount(): { timeouts: number; intervals: number } {
        return {
            timeouts: this.timeouts.size,
            intervals: this.intervals.size,
        };
    }
}
