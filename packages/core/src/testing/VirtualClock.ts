/**
 * Clock source interface for time dependency injection.
 * Allows swapping real time with virtual time for deterministic testing.
 */
export interface ClockSource {
  now(): number;
}

/**
 * Real clock source using system time.
 */
export const RealClock: ClockSource = {
  now: () => Date.now()
};

/**
 * Virtual clock for deterministic testing.
 * Time only advances when explicitly requested via advance() or set().
 *
 * Usage:
 * ```typescript
 * const clock = new VirtualClock(1000000);
 * clock.now(); // 1000000
 * clock.advance(500);
 * clock.now(); // 1000500
 * ```
 */
export class VirtualClock implements ClockSource {
  private currentTime: number;

  /**
   * @param initialTime Starting timestamp in milliseconds (default: 0)
   */
  constructor(initialTime: number = 0) {
    if (!Number.isFinite(initialTime) || initialTime < 0) {
      throw new Error('Initial time must be a non-negative finite number');
    }
    this.currentTime = initialTime;
  }

  /**
   * Returns the current virtual time.
   * Time remains frozen until advance() or set() is called.
   */
  public now(): number {
    return this.currentTime;
  }

  /**
   * Advances time forward by the specified milliseconds.
   * @param ms Milliseconds to advance (must be non-negative)
   */
  public advance(ms: number): void {
    if (!Number.isFinite(ms) || ms < 0) {
      throw new Error('Advance amount must be a non-negative finite number');
    }
    this.currentTime += ms;
  }

  /**
   * Sets the clock to a specific time.
   * Allows moving time forward or backward (useful for testing).
   * @param time Absolute timestamp in milliseconds
   */
  public set(time: number): void {
    if (!Number.isFinite(time) || time < 0) {
      throw new Error('Time must be a non-negative finite number');
    }
    this.currentTime = time;
  }

  /**
   * Resets the clock to zero.
   */
  public reset(): void {
    this.currentTime = 0;
  }
}
