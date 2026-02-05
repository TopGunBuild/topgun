import { SeededRNG } from './SeededRNG';
import { VirtualClock } from './VirtualClock';

/**
 * Network configuration for chaos injection.
 */
export interface NetworkConfig {
  /** Latency range in milliseconds */
  latencyMs: { min: number; max: number };
  /** Packet loss rate (0.0 to 1.0) */
  packetLossRate: number;
  /** Groups that cannot communicate with each other */
  partitions: string[][];
}

/**
 * A message in flight through the virtual network.
 */
export interface Message {
  from: string;
  to: string;
  payload: unknown;
  scheduledTime: number;
}

/**
 * Virtual network for deterministic chaos testing.
 * Simulates packet loss, latency, and network partitions.
 *
 * Usage:
 * ```typescript
 * const rng = new SeededRNG(123);
 * const clock = new VirtualClock(1000);
 * const network = new VirtualNetwork(rng, clock);
 *
 * network.configure({ packetLossRate: 0.1, latencyMs: { min: 10, max: 50 } });
 * network.send('node-a', 'node-b', { type: 'sync' });
 *
 * clock.advance(30);
 * const delivered = network.tick(); // Messages delivered at current time
 * ```
 */
export class VirtualNetwork {
  private readonly rng: SeededRNG;
  private readonly clock: VirtualClock;
  private config: NetworkConfig;
  private pendingMessages: Message[] = [];

  constructor(rng: SeededRNG, clock: VirtualClock) {
    this.rng = rng;
    this.clock = clock;
    this.config = {
      latencyMs: { min: 0, max: 0 },
      packetLossRate: 0,
      partitions: []
    };
  }

  /**
   * Updates network configuration.
   * Partially updates existing config with provided values.
   */
  public configure(config: Partial<NetworkConfig>): void {
    if (config.latencyMs !== undefined) {
      const { min, max } = config.latencyMs;
      if (min < 0 || max < 0 || min > max) {
        throw new Error('Invalid latency range');
      }
      this.config.latencyMs = config.latencyMs;
    }
    if (config.packetLossRate !== undefined) {
      if (config.packetLossRate < 0 || config.packetLossRate > 1) {
        throw new Error('Packet loss rate must be between 0 and 1');
      }
      this.config.packetLossRate = config.packetLossRate;
    }
    if (config.partitions !== undefined) {
      this.config.partitions = config.partitions;
    }
  }

  /**
   * Sends a message through the network.
   * Subject to packet loss, latency, and partition rules.
   */
  public send(from: string, to: string, payload: unknown): void {
    // Check packet loss
    if (this.rng.random() < this.config.packetLossRate) {
      return; // Message dropped
    }

    // Check partitions
    if (this.isPartitioned(from, to)) {
      return; // Message blocked by partition
    }

    // Calculate delivery time
    const latency = this.rng.randomInt(
      this.config.latencyMs.min,
      this.config.latencyMs.max
    );
    const scheduledTime = this.clock.now() + latency;

    // Add to pending messages
    this.pendingMessages.push({
      from,
      to,
      payload,
      scheduledTime
    });
  }

  /**
   * Creates a network partition between two groups.
   * Nodes in groupA cannot communicate with nodes in groupB.
   */
  public partition(groupA: string[], groupB: string[]): void {
    this.config.partitions.push(groupA, groupB);
  }

  /**
   * Removes all network partitions.
   */
  public heal(): void {
    this.config.partitions = [];
  }

  /**
   * Delivers all messages scheduled at or before the current time.
   * @returns Array of delivered messages
   */
  public tick(): Message[] {
    const currentTime = this.clock.now();
    const delivered: Message[] = [];
    const remaining: Message[] = [];

    for (const msg of this.pendingMessages) {
      if (msg.scheduledTime <= currentTime) {
        delivered.push(msg);
      } else {
        remaining.push(msg);
      }
    }

    this.pendingMessages = remaining;
    return delivered;
  }

  /**
   * Returns the number of messages currently in flight.
   */
  public getPendingCount(): number {
    return this.pendingMessages.length;
  }

  /**
   * Clears all pending messages.
   */
  public clear(): void {
    this.pendingMessages = [];
  }

  /**
   * Checks if two nodes are partitioned from each other.
   */
  private isPartitioned(from: string, to: string): boolean {
    for (let i = 0; i < this.config.partitions.length; i += 2) {
      const groupA = this.config.partitions[i];
      const groupB = this.config.partitions[i + 1];

      if (
        (groupA.includes(from) && groupB.includes(to)) ||
        (groupB.includes(from) && groupA.includes(to))
      ) {
        return true;
      }
    }
    return false;
  }

  /**
   * Returns all pending messages (useful for debugging).
   */
  public getPendingMessages(): Message[] {
    return [...this.pendingMessages];
  }
}
