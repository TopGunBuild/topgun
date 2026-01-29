/**
 * CounterManager - Handles PN counter operations for SyncEngine
 *
 * Responsibilities:
 * - Counter update subscriptions
 * - Requesting initial counter state from server
 * - Syncing local counter state to server
 * - Handling incoming counter updates from server
 */

import { logger } from '../utils/logger';
import type { ICounterManager, CounterManagerConfig } from './types';

/**
 * CounterManager implements ICounterManager.
 *
 * Manages PN counter operations with support for:
 * - Subscribe/unsubscribe to counter updates
 * - Server state synchronization
 * - State conversion between Map and object formats
 */
export class CounterManager implements ICounterManager {
  private readonly config: CounterManagerConfig;

  // Counter update listeners by name
  private counterUpdateListeners: Map<string, Set<(state: any) => void>> = new Map();

  constructor(config: CounterManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Subscribe to counter updates from server.
   * @param name Counter name
   * @param listener Callback when counter state is updated
   * @returns Unsubscribe function
   */
  public onCounterUpdate(name: string, listener: (state: any) => void): () => void {
    if (!this.counterUpdateListeners.has(name)) {
      this.counterUpdateListeners.set(name, new Set());
    }
    this.counterUpdateListeners.get(name)!.add(listener);

    return () => {
      this.counterUpdateListeners.get(name)?.delete(listener);
      if (this.counterUpdateListeners.get(name)?.size === 0) {
        this.counterUpdateListeners.delete(name);
      }
    };
  }

  /**
   * Request initial counter state from server.
   * @param name Counter name
   */
  public requestCounter(name: string): void {
    if (this.config.isAuthenticated()) {
      this.config.sendMessage({
        type: 'COUNTER_REQUEST',
        payload: { name }
      });
    }
  }

  /**
   * Sync local counter state to server.
   * @param name Counter name
   * @param state Counter state to sync
   */
  public syncCounter(name: string, state: any): void {
    if (this.config.isAuthenticated()) {
      // Convert Maps to objects for serialization
      const stateObj = {
        positive: Object.fromEntries(state.positive),
        negative: Object.fromEntries(state.negative),
      };

      this.config.sendMessage({
        type: 'COUNTER_SYNC',
        payload: {
          name,
          state: stateObj
        }
      });
    }
  }

  /**
   * Handle incoming counter update from server.
   * Called by SyncEngine for COUNTER_UPDATE and COUNTER_RESPONSE messages.
   */
  public handleCounterUpdate(name: string, stateObj: { positive: Record<string, number>; negative: Record<string, number> }): void {
    // Convert objects to Maps
    const state = {
      positive: new Map(Object.entries(stateObj.positive)),
      negative: new Map(Object.entries(stateObj.negative)),
    };

    const listeners = this.counterUpdateListeners.get(name);
    if (listeners) {
      for (const listener of listeners) {
        try {
          listener(state);
        } catch (e) {
          logger.error({ err: e, counterName: name }, 'Counter update listener error');
        }
      }
    }
  }

  /**
   * Clean up resources.
   * Clears all counter update listeners.
   */
  public close(): void {
    this.counterUpdateListeners.clear();
  }
}
