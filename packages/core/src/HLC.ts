import { logger } from './utils/logger';

export interface Timestamp {
  millis: number;
  counter: number;
  nodeId: string;
}

/**
 * Clock source interface for time dependency injection.
 */
export interface ClockSource {
  now(): number;
}

/**
 * Configuration options for HLC behavior.
 */
export interface HLCOptions {
  /**
   * When true, update() throws an error if remote timestamp drift exceeds maxDriftMs.
   * When false (default), a warning is logged but the timestamp is accepted.
   */
  strictMode?: boolean;

  /**
   * Maximum allowable clock drift in milliseconds.
   * Remote timestamps beyond this threshold trigger strict mode rejection or warning.
   * Default: 60000 (1 minute)
   */
  maxDriftMs?: number;

  /**
   * Clock source for time generation.
   * Defaults to Date.now() for production use.
   * Can be replaced with VirtualClock for deterministic testing.
   */
  clockSource?: ClockSource;
}

export class HLC {
  private lastMillis: number;
  private lastCounter: number;
  private readonly nodeId: string;
  private readonly strictMode: boolean;
  private readonly maxDriftMs: number;
  private readonly clockSource: ClockSource;

  constructor(nodeId: string, options: HLCOptions = {}) {
    this.nodeId = nodeId;
    this.strictMode = options.strictMode ?? false;
    this.maxDriftMs = options.maxDriftMs ?? 60000;
    this.clockSource = options.clockSource ?? { now: () => Date.now() };
    this.lastMillis = 0;
    this.lastCounter = 0;
  }

  public get getNodeId(): string {
    return this.nodeId;
  }

  public get getStrictMode(): boolean {
    return this.strictMode;
  }

  public get getMaxDriftMs(): number {
    return this.maxDriftMs;
  }

  /**
   * Returns the clock source used by this HLC instance.
   * Useful for LWWMap/ORMap to access the same clock for TTL checks.
   */
  public getClockSource(): ClockSource {
    return this.clockSource;
  }

  /**
   * Generates a new unique timestamp for a local event.
   * Ensures monotonicity: always greater than any previously generated or received timestamp.
   */
  public now(): Timestamp {
    const systemTime = this.clockSource.now();
    
    // If local physical time catches up to logical time, reset counter
    if (systemTime > this.lastMillis) {
      this.lastMillis = systemTime;
      this.lastCounter = 0;
    } else {
      // Else, just increment the logical counter
      this.lastCounter++;
    }

    return {
      millis: this.lastMillis,
      counter: this.lastCounter,
      nodeId: this.nodeId
    };
  }

  /**
   * Updates the local clock based on a received remote timestamp.
   * Must be called whenever a message/event is received from another node.
   */
  public update(remote: Timestamp): void {
    const systemTime = this.clockSource.now();

    // Validate drift
    const drift = remote.millis - systemTime;
    if (drift > this.maxDriftMs) {
      if (this.strictMode) {
        throw new Error(`Clock drift detected: Remote time ${remote.millis} is ${drift}ms ahead of local ${systemTime} (threshold: ${this.maxDriftMs}ms)`);
      } else {
        logger.warn({
          drift,
          remoteMillis: remote.millis,
          localMillis: systemTime,
          maxDriftMs: this.maxDriftMs
        }, 'Clock drift detected');
        // In AP systems we accept and fast-forward
      }
    }

    const maxMillis = Math.max(this.lastMillis, systemTime, remote.millis);

    if (maxMillis === this.lastMillis && maxMillis === remote.millis) {
      // Both clocks are on the same millisecond, take max counter
      this.lastCounter = Math.max(this.lastCounter, remote.counter) + 1;
    } else if (maxMillis === this.lastMillis) {
      // Local logical clock is ahead in millis (or same as remote but remote millis < local)
      this.lastCounter++;
    } else if (maxMillis === remote.millis) {
      // Remote clock is ahead, fast-forward local
      this.lastCounter = remote.counter + 1;
    } else {
      // System time is ahead of both
      this.lastCounter = 0;
    }

    this.lastMillis = maxMillis;
  }

  /**
   * Compares two timestamps.
   * Returns -1 if a < b, 1 if a > b, 0 if equal.
   */
  public static compare(a: Timestamp, b: Timestamp): number {
    if (a.millis !== b.millis) {
      return a.millis - b.millis;
    }
    if (a.counter !== b.counter) {
      return a.counter - b.counter;
    }
    return a.nodeId.localeCompare(b.nodeId);
  }

  /**
   * Serializes timestamp to a string representation (e.g., for storage/network).
   * Format: "<millis>:<counter>:<nodeId>"
   */
  public static toString(ts: Timestamp): string {
    return `${ts.millis}:${ts.counter}:${ts.nodeId}`;
  }

  /**
   * Parses a string representation back to a Timestamp object.
   */
  public static parse(str: string): Timestamp {
    const parts = str.split(':');
    if (parts.length !== 3) {
      throw new Error(`Invalid timestamp format: ${str}`);
    }
    return {
      millis: parseInt(parts[0], 10),
      counter: parseInt(parts[1], 10),
      nodeId: parts[2]
    };
  }
}
