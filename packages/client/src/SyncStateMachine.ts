import { SyncState, isValidTransition } from './SyncState';
import { logger } from './utils/logger';

/**
 * Event emitted when the state machine transitions between states.
 */
export interface StateChangeEvent {
  /** The state before the transition */
  from: SyncState;
  /** The state after the transition */
  to: SyncState;
  /** Unix timestamp (ms) when the transition occurred */
  timestamp: number;
}

/**
 * Listener callback for state change events.
 */
export type StateChangeListener = (event: StateChangeEvent) => void;

/**
 * Configuration options for the state machine.
 */
export interface SyncStateMachineConfig {
  /** Maximum number of state transitions to keep in history (default: 50) */
  maxHistorySize?: number;
}

const DEFAULT_MAX_HISTORY_SIZE = 50;

/**
 * A finite state machine for managing SyncEngine connection states.
 *
 * Features:
 * - Validates all state transitions against allowed paths
 * - Emits events on state changes for observability
 * - Maintains a history of transitions for debugging
 * - Logs invalid transition attempts (graceful degradation)
 */
export class SyncStateMachine {
  private state: SyncState = SyncState.INITIAL;
  private readonly listeners: Set<StateChangeListener> = new Set();
  private history: StateChangeEvent[] = [];
  private readonly maxHistorySize: number;

  constructor(config: SyncStateMachineConfig = {}) {
    this.maxHistorySize = config.maxHistorySize ?? DEFAULT_MAX_HISTORY_SIZE;
  }

  /**
   * Attempt to transition to a new state.
   * @param to The target state
   * @returns true if the transition was valid and executed, false otherwise
   */
  transition(to: SyncState): boolean {
    const from = this.state;

    if (from === to) {
      // No-op: already in target state
      return true;
    }

    if (!isValidTransition(from, to)) {
      logger.warn(
        { from, to, currentHistory: this.getHistory(5) },
        `Invalid state transition attempted: ${from} → ${to}`
      );
      return false;
    }

    // Execute the transition
    this.state = to;

    const event: StateChangeEvent = {
      from,
      to,
      timestamp: Date.now(),
    };

    // Add to history
    this.history.push(event);
    if (this.history.length > this.maxHistorySize) {
      this.history.shift();
    }

    // Notify listeners
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (err) {
        logger.error({ err, event }, 'State change listener threw an error');
      }
    }

    logger.debug({ from, to }, `State transition: ${from} → ${to}`);

    return true;
  }

  /**
   * Get the current state.
   */
  getState(): SyncState {
    return this.state;
  }

  /**
   * Check if a transition from the current state to the target state is valid.
   * @param to The target state to check
   */
  canTransition(to: SyncState): boolean {
    return this.state === to || isValidTransition(this.state, to);
  }

  /**
   * Subscribe to state change events.
   * @param listener Callback function to be called on each state change
   * @returns An unsubscribe function
   */
  onStateChange(listener: StateChangeListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Get the state transition history.
   * @param limit Maximum number of entries to return (default: all)
   * @returns Array of state change events, oldest first
   */
  getHistory(limit?: number): StateChangeEvent[] {
    if (limit === undefined || limit >= this.history.length) {
      return [...this.history];
    }
    return this.history.slice(-limit);
  }

  /**
   * Reset the state machine to INITIAL state.
   * This is a forced reset that bypasses normal transition validation.
   * Use for testing or hard resets after fatal errors.
   * @param clearHistory If true, also clears the transition history (default: true)
   */
  reset(clearHistory = true): void {
    const from = this.state;
    this.state = SyncState.INITIAL;

    if (clearHistory) {
      this.history = [];
    } else {
      // Record the reset as a transition
      const event: StateChangeEvent = {
        from,
        to: SyncState.INITIAL,
        timestamp: Date.now(),
      };
      this.history.push(event);
      if (this.history.length > this.maxHistorySize) {
        this.history.shift();
      }

      // Notify listeners
      for (const listener of this.listeners) {
        try {
          listener(event);
        } catch (err) {
          logger.error({ err, event }, 'State change listener threw an error during reset');
        }
      }
    }

    logger.info({ from }, 'State machine reset to INITIAL');
  }

  /**
   * Check if the state machine is in a "connected" state
   * (either SYNCING or CONNECTED)
   */
  isConnected(): boolean {
    return this.state === SyncState.CONNECTED || this.state === SyncState.SYNCING;
  }

  /**
   * Check if the state machine is in a state where operations can be sent
   * (authenticated and connected)
   */
  isReady(): boolean {
    return this.state === SyncState.CONNECTED;
  }

  /**
   * Check if the state machine is currently attempting to connect
   */
  isConnecting(): boolean {
    return (
      this.state === SyncState.CONNECTING ||
      this.state === SyncState.AUTHENTICATING ||
      this.state === SyncState.SYNCING
    );
  }
}
