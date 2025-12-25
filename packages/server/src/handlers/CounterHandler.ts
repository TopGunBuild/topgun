import { PNCounterImpl } from '@topgunbuild/core';
import type { PNCounterState, PNCounterStateObject } from '@topgunbuild/core';
import { logger } from '../utils/logger';

/**
 * Server-side handler for PN Counter CRDT synchronization.
 *
 * Responsibilities:
 * - Store counter state in memory (with optional persistence)
 * - Merge incoming client states using CRDT semantics
 * - Broadcast updates to subscribed clients
 * - Handle initial state requests
 */
export class CounterHandler {
  private counters: Map<string, PNCounterImpl> = new Map();
  private subscriptions: Map<string, Set<string>> = new Map(); // counterName -> Set<clientId>

  constructor(private readonly nodeId: string = 'server') {}

  /**
   * Get or create a counter by name.
   */
  private getOrCreateCounter(name: string): PNCounterImpl {
    let counter = this.counters.get(name);
    if (!counter) {
      counter = new PNCounterImpl({ nodeId: this.nodeId });
      this.counters.set(name, counter);
      logger.debug({ name }, 'Created new counter');
    }
    return counter;
  }

  /**
   * Handle COUNTER_REQUEST - client wants initial state.
   * @returns Response message to send back to client
   */
  handleCounterRequest(
    clientId: string,
    name: string
  ): { type: string; payload: { name: string; state: PNCounterStateObject } } {
    const counter = this.getOrCreateCounter(name);

    // Subscribe client to this counter
    this.subscribe(clientId, name);

    const state = counter.getState();
    logger.debug({ clientId, name, value: counter.get() }, 'Counter request handled');

    return {
      type: 'COUNTER_RESPONSE',
      payload: {
        name,
        state: this.stateToObject(state),
      },
    };
  }

  /**
   * Handle COUNTER_SYNC - client sends their state to merge.
   * @returns Merged state and list of clients to broadcast to
   */
  handleCounterSync(
    clientId: string,
    name: string,
    stateObj: PNCounterStateObject
  ): {
    response: { type: string; payload: { name: string; state: PNCounterStateObject } };
    broadcastTo: string[];
    broadcastMessage: { type: string; payload: { name: string; state: PNCounterStateObject } };
  } {
    const counter = this.getOrCreateCounter(name);

    // Convert object to Map-based state
    const incomingState = this.objectToState(stateObj);

    // Merge client state into server counter
    counter.merge(incomingState);

    const mergedState = counter.getState();
    const mergedStateObj = this.stateToObject(mergedState);

    logger.debug(
      { clientId, name, value: counter.get() },
      'Counter sync handled'
    );

    // Subscribe client to this counter (in case they weren't already)
    this.subscribe(clientId, name);

    // Get all other subscribed clients for broadcast
    const subscribers = this.subscriptions.get(name) || new Set();
    const broadcastTo = Array.from(subscribers).filter((id) => id !== clientId);

    return {
      // Response to the sending client
      response: {
        type: 'COUNTER_UPDATE',
        payload: {
          name,
          state: mergedStateObj,
        },
      },
      // Broadcast to other clients
      broadcastTo,
      broadcastMessage: {
        type: 'COUNTER_UPDATE',
        payload: {
          name,
          state: mergedStateObj,
        },
      },
    };
  }

  /**
   * Subscribe a client to counter updates.
   */
  subscribe(clientId: string, counterName: string): void {
    if (!this.subscriptions.has(counterName)) {
      this.subscriptions.set(counterName, new Set());
    }
    this.subscriptions.get(counterName)!.add(clientId);
    logger.debug({ clientId, counterName }, 'Client subscribed to counter');
  }

  /**
   * Unsubscribe a client from counter updates.
   */
  unsubscribe(clientId: string, counterName: string): void {
    const subs = this.subscriptions.get(counterName);
    if (subs) {
      subs.delete(clientId);
      if (subs.size === 0) {
        this.subscriptions.delete(counterName);
      }
    }
  }

  /**
   * Unsubscribe a client from all counters (e.g., on disconnect).
   */
  unsubscribeAll(clientId: string): void {
    for (const [counterName, subs] of this.subscriptions) {
      subs.delete(clientId);
      if (subs.size === 0) {
        this.subscriptions.delete(counterName);
      }
    }
    logger.debug({ clientId }, 'Client unsubscribed from all counters');
  }

  /**
   * Get current counter value (for monitoring/debugging).
   */
  getCounterValue(name: string): number {
    const counter = this.counters.get(name);
    return counter ? counter.get() : 0;
  }

  /**
   * Get all counter names.
   */
  getCounterNames(): string[] {
    return Array.from(this.counters.keys());
  }

  /**
   * Get number of subscribers for a counter.
   */
  getSubscriberCount(name: string): number {
    return this.subscriptions.get(name)?.size || 0;
  }

  /**
   * Convert Map-based state to plain object for serialization.
   */
  private stateToObject(state: PNCounterState): PNCounterStateObject {
    return {
      p: Object.fromEntries(state.positive),
      n: Object.fromEntries(state.negative),
    };
  }

  /**
   * Convert plain object to Map-based state.
   */
  private objectToState(obj: PNCounterStateObject): PNCounterState {
    return {
      positive: new Map(Object.entries(obj.p || {})),
      negative: new Map(Object.entries(obj.n || {})),
    };
  }
}
