import { PNCounterImpl } from '@topgunbuild/core';
import type { PNCounter, PNCounterState, PNCounterStateObject } from '@topgunbuild/core';
import { SyncEngine } from './SyncEngine';
import type { IStorageAdapter } from './IStorageAdapter';
import { logger } from './utils/logger';

/**
 * Storage key prefix for PNCounter state persistence.
 */
const COUNTER_STORAGE_PREFIX = '__counter__:';

/**
 * Client-side handle for a PN Counter.
 *
 * Wraps the core PNCounterImpl and integrates with SyncEngine for:
 * - Automatic sync to server when counter changes
 * - Receiving remote updates from other clients
 * - Local persistence via IStorageAdapter (IndexedDB in browser)
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
  private readonly storageAdapter?: IStorageAdapter;
  private syncScheduled = false;
  private persistScheduled = false;
  private unsubscribeFromUpdates?: () => void;

  constructor(name: string, nodeId: string, syncEngine: SyncEngine, storageAdapter?: IStorageAdapter) {
    this.name = name;
    this.syncEngine = syncEngine;
    this.storageAdapter = storageAdapter;
    this.counter = new PNCounterImpl({ nodeId });

    // Restore state from local storage first (async, but fast)
    this.restoreFromStorage();

    // Subscribe to remote updates via SyncEngine
    this.unsubscribeFromUpdates = this.syncEngine.onCounterUpdate(name, (state) => {
      this.counter.merge(state);
      // Persist merged state to local storage
      this.schedulePersist();
    });

    // Request initial state from server
    this.syncEngine.requestCounter(name);

    logger.debug({ name, nodeId }, 'PNCounterHandle created');
  }

  /**
   * Restore counter state from local storage.
   * Called during construction to recover offline state.
   */
  private async restoreFromStorage(): Promise<void> {
    if (!this.storageAdapter) {
      return;
    }

    try {
      const storageKey = COUNTER_STORAGE_PREFIX + this.name;
      const stored = await this.storageAdapter.getMeta(storageKey);

      if (stored && typeof stored === 'object' && 'p' in stored && 'n' in stored) {
        // Convert stored object to PNCounterState
        const state = PNCounterImpl.objectToState(stored as PNCounterStateObject);
        this.counter.merge(state);
        logger.debug({ name: this.name, value: this.counter.get() }, 'PNCounter restored from storage');
      }
    } catch (err) {
      logger.error({ err, name: this.name }, 'Failed to restore PNCounter from storage');
    }
  }

  /**
   * Persist counter state to local storage.
   * Debounced to avoid excessive writes during rapid operations.
   */
  private schedulePersist(): void {
    if (!this.storageAdapter || this.persistScheduled) return;
    this.persistScheduled = true;

    // Debounce persistence (100ms) to batch rapid changes
    setTimeout(() => {
      this.persistScheduled = false;
      this.persistToStorage();
    }, 100);
  }

  /**
   * Actually persist state to storage.
   */
  private async persistToStorage(): Promise<void> {
    if (!this.storageAdapter) return;

    try {
      const storageKey = COUNTER_STORAGE_PREFIX + this.name;
      const stateObj = PNCounterImpl.stateToObject(this.counter.getState());
      await this.storageAdapter.setMeta(storageKey, stateObj);
      logger.debug({ name: this.name, value: this.counter.get() }, 'PNCounter persisted to storage');
    } catch (err) {
      logger.error({ err, name: this.name }, 'Failed to persist PNCounter to storage');
    }
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
    this.schedulePersist();
    return value;
  }

  /**
   * Decrement by 1 and return new value.
   */
  decrement(): number {
    const value = this.counter.decrement();
    this.scheduleSync();
    this.schedulePersist();
    return value;
  }

  /**
   * Add delta (positive or negative) and return new value.
   */
  addAndGet(delta: number): number {
    const value = this.counter.addAndGet(delta);
    if (delta !== 0) {
      this.scheduleSync();
      this.schedulePersist();
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
