import { serialize, deserialize } from './serializer';

/**
 * State of a PN Counter CRDT.
 * Tracks positive and negative increments per node for convergence.
 */
export interface PNCounterState {
  /** Positive increments per node */
  positive: Map<string, number>;
  /** Negative increments per node */
  negative: Map<string, number>;
}

/**
 * Serializable form of PNCounterState for network/storage.
 */
export interface PNCounterStateObject {
  /** Positive increments per node as object */
  p: Record<string, number>;
  /** Negative increments per node as object */
  n: Record<string, number>;
}

/**
 * Configuration for creating a PN Counter.
 */
export interface PNCounterConfig {
  /** Unique node identifier for this counter instance */
  nodeId: string;
  /** Initial state to restore from */
  initialState?: PNCounterState;
}

/**
 * Interface for PN Counter CRDT.
 */
export interface PNCounter {
  /** Get current value */
  get(): number;

  /** Increment by 1, return new value */
  increment(): number;

  /** Decrement by 1, return new value */
  decrement(): number;

  /** Add delta (positive or negative), return new value */
  addAndGet(delta: number): number;

  /** Get state for sync */
  getState(): PNCounterState;

  /** Merge remote state */
  merge(remote: PNCounterState): void;

  /** Subscribe to value changes */
  subscribe(listener: (value: number) => void): () => void;
}

/**
 * Positive-Negative Counter CRDT implementation.
 *
 * A PN Counter is a CRDT that supports increment and decrement operations
 * on any node, works offline, and guarantees convergence without coordination.
 *
 * How it works:
 * - Tracks positive increments per node in a G-Counter
 * - Tracks negative increments per node in another G-Counter
 * - Value = sum(positive) - sum(negative)
 * - Merge takes max for each node in both counters
 *
 * @example
 * ```typescript
 * const counter = new PNCounterImpl({ nodeId: 'node-1' });
 * counter.increment(); // 1
 * counter.increment(); // 2
 * counter.decrement(); // 1
 * counter.addAndGet(5); // 6
 * ```
 */
export class PNCounterImpl implements PNCounter {
  private readonly nodeId: string;
  private state: PNCounterState;
  private listeners: Set<(value: number) => void> = new Set();

  constructor(config: PNCounterConfig) {
    this.nodeId = config.nodeId;
    this.state = config.initialState ?? {
      positive: new Map(),
      negative: new Map(),
    };
  }

  /**
   * Get the current counter value.
   * Value = sum(positive) - sum(negative)
   */
  get(): number {
    let sum = 0;
    for (const v of this.state.positive.values()) sum += v;
    for (const v of this.state.negative.values()) sum -= v;
    return sum;
  }

  /**
   * Increment by 1 and return the new value.
   */
  increment(): number {
    return this.addAndGet(1);
  }

  /**
   * Decrement by 1 and return the new value.
   */
  decrement(): number {
    return this.addAndGet(-1);
  }

  /**
   * Add a delta (positive or negative) and return the new value.
   * @param delta The amount to add (can be negative)
   */
  addAndGet(delta: number): number {
    if (delta === 0) return this.get();

    if (delta > 0) {
      const current = this.state.positive.get(this.nodeId) ?? 0;
      this.state.positive.set(this.nodeId, current + delta);
    } else {
      const current = this.state.negative.get(this.nodeId) ?? 0;
      this.state.negative.set(this.nodeId, current + Math.abs(delta));
    }

    const newValue = this.get();
    this.notifyListeners(newValue);
    return newValue;
  }

  /**
   * Get a copy of the current state for synchronization.
   */
  getState(): PNCounterState {
    return {
      positive: new Map(this.state.positive),
      negative: new Map(this.state.negative),
    };
  }

  /**
   * Merge remote state into this counter.
   * Takes the maximum value for each node in both positive and negative counters.
   * This operation is commutative, associative, and idempotent (CRDT properties).
   *
   * @param remote The remote state to merge
   */
  merge(remote: PNCounterState): void {
    let changed = false;

    // Merge positive: take max for each node
    for (const [nodeId, value] of remote.positive) {
      const current = this.state.positive.get(nodeId) ?? 0;
      if (value > current) {
        this.state.positive.set(nodeId, value);
        changed = true;
      }
    }

    // Merge negative: take max for each node
    for (const [nodeId, value] of remote.negative) {
      const current = this.state.negative.get(nodeId) ?? 0;
      if (value > current) {
        this.state.negative.set(nodeId, value);
        changed = true;
      }
    }

    if (changed) {
      this.notifyListeners(this.get());
    }
  }

  /**
   * Subscribe to value changes.
   * The listener is immediately called with the current value.
   *
   * @param listener Callback function receiving the new value
   * @returns Unsubscribe function
   */
  subscribe(listener: (value: number) => void): () => void {
    this.listeners.add(listener);
    // Immediately notify with current value
    listener(this.get());
    return () => this.listeners.delete(listener);
  }

  private notifyListeners(value: number): void {
    for (const listener of this.listeners) {
      try {
        listener(value);
      } catch (e) {
        // Silently catch listener errors to prevent breaking other listeners
      }
    }
  }

  /**
   * Get the node ID of this counter instance.
   */
  getNodeId(): string {
    return this.nodeId;
  }

  // ============================================
  // Serialization
  // ============================================

  /**
   * Serialize state to binary format (msgpack).
   */
  static serialize(state: PNCounterState): Uint8Array {
    const obj: PNCounterStateObject = {
      p: Object.fromEntries(state.positive),
      n: Object.fromEntries(state.negative),
    };
    return serialize(obj);
  }

  /**
   * Deserialize binary data to state.
   */
  static deserialize(data: Uint8Array): PNCounterState {
    const obj = deserialize<PNCounterStateObject>(data);
    return {
      positive: new Map(Object.entries(obj.p)),
      negative: new Map(Object.entries(obj.n)),
    };
  }

  /**
   * Convert state to plain object (for JSON/network).
   */
  static stateToObject(state: PNCounterState): PNCounterStateObject {
    return {
      p: Object.fromEntries(state.positive),
      n: Object.fromEntries(state.negative),
    };
  }

  /**
   * Convert plain object to state.
   */
  static objectToState(obj: PNCounterStateObject): PNCounterState {
    return {
      positive: new Map(Object.entries(obj.p)),
      negative: new Map(Object.entries(obj.n)),
    };
  }
}
