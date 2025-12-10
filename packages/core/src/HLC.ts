export interface Timestamp {
  millis: number;
  counter: number;
  nodeId: string;
}

export class HLC {
  private lastMillis: number;
  private lastCounter: number;
  private readonly nodeId: string;

  // Max allowable drift in milliseconds (1 minute)
  private static readonly MAX_DRIFT = 60000;

  constructor(nodeId: string) {
    this.nodeId = nodeId;
    this.lastMillis = 0;
    this.lastCounter = 0;
  }

  public get getNodeId(): string {
    return this.nodeId;
  }

  /**
   * Generates a new unique timestamp for a local event.
   * Ensures monotonicity: always greater than any previously generated or received timestamp.
   */
  public now(): Timestamp {
    const systemTime = Date.now();
    
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
    const systemTime = Date.now();

    // Validate drift (optional but good practice)
    if (remote.millis > systemTime + HLC.MAX_DRIFT) {
      console.warn(`Clock drift detected: Remote time ${remote.millis} is far ahead of local ${systemTime}`);
      // In strict systems we might reject, but in AP systems we usually accept and fast-forward
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
