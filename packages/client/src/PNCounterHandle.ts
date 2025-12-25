import { PNCounterImpl } from '@topgunbuild/core';
import type { PNCounter, PNCounterState } from '@topgunbuild/core';
import { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

/**
 * Client-side handle for a PN Counter.
 *
 * Wraps the core PNCounterImpl and integrates with SyncEngine for:
 * - Automatic sync to server when counter changes
 * - Receiving remote updates from other clients
 * - Local persistence (future)
 *
 * @example
 * ```typescript
 * const counter = client.getPNCounter('likes:post-123');
 * counter.increment(); // Immediate local update + sync to server
 *
 * counter.subscribe((value) => {
 *   console.log('Current likes:', value);
 * });
 * ```
 */
export class PNCounterHandle implements PNCounter {
  private readonly counter: PNCounterImpl;
  private readonly name: string;
  private readonly syncEngine: SyncEngine;
  private syncScheduled = false;
  private unsubscribeFromUpdates?: () => void;

  constructor(name: string, nodeId: string, syncEngine: SyncEngine) {
    this.name = name;
    this.syncEngine = syncEngine;
    this.counter = new PNCounterImpl({ nodeId });

    // Subscribe to remote updates via SyncEngine
    this.unsubscribeFromUpdates = this.syncEngine.onCounterUpdate(name, (state) => {
      this.counter.merge(state);
    });

    // Request initial state from server
    this.syncEngine.requestCounter(name);

    logger.debug({ name, nodeId }, 'PNCounterHandle created');
  }

  /**
   * Get current counter value.
   */
  get(): number {
    return this.counter.get();
  }

  /**
   * Increment by 1 and return new value.
   */
  increment(): number {
    const value = this.counter.increment();
    this.scheduleSync();
    return value;
  }

  /**
   * Decrement by 1 and return new value.
   */
  decrement(): number {
    const value = this.counter.decrement();
    this.scheduleSync();
    return value;
  }

  /**
   * Add delta (positive or negative) and return new value.
   */
  addAndGet(delta: number): number {
    const value = this.counter.addAndGet(delta);
    if (delta !== 0) {
      this.scheduleSync();
    }
    return value;
  }

  /**
   * Get state for sync.
   */
  getState(): PNCounterState {
    return this.counter.getState();
  }

  /**
   * Merge remote state.
   */
  merge(remote: PNCounterState): void {
    this.counter.merge(remote);
  }

  /**
   * Subscribe to value changes.
   */
  subscribe(listener: (value: number) => void): () => void {
    return this.counter.subscribe(listener);
  }

  /**
   * Get the counter name.
   */
  getName(): string {
    return this.name;
  }

  /**
   * Cleanup resources.
   */
  dispose(): void {
    if (this.unsubscribeFromUpdates) {
      this.unsubscribeFromUpdates();
    }
  }

  /**
   * Schedule sync to server with debouncing.
   * Batches rapid increments to avoid network spam.
   */
  private scheduleSync(): void {
    if (this.syncScheduled) return;
    this.syncScheduled = true;

    // Debounce sync to batch rapid increments (50ms)
    setTimeout(() => {
      this.syncScheduled = false;
      this.syncEngine.syncCounter(this.name, this.counter.getState());
    }, 50);
  }
}
