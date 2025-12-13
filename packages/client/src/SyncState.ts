/**
 * Defines the possible states for the SyncEngine connection state machine.
 */
export enum SyncState {
  /** Initial state before any connection attempt */
  INITIAL = 'INITIAL',
  /** WebSocket connection is being established */
  CONNECTING = 'CONNECTING',
  /** Connected, waiting for authentication response */
  AUTHENTICATING = 'AUTHENTICATING',
  /** Authenticated, performing initial data sync */
  SYNCING = 'SYNCING',
  /** Fully connected and synchronized */
  CONNECTED = 'CONNECTED',
  /** Intentionally or unexpectedly disconnected */
  DISCONNECTED = 'DISCONNECTED',
  /** Waiting before retry (exponential backoff) */
  BACKOFF = 'BACKOFF',
  /** Fatal error requiring manual intervention or reset */
  ERROR = 'ERROR',
}

/**
 * Defines valid state transitions for the SyncEngine FSM.
 * Each key is a current state, and the value is an array of valid target states.
 */
export const VALID_TRANSITIONS: Record<SyncState, SyncState[]> = {
  [SyncState.INITIAL]: [SyncState.CONNECTING],
  [SyncState.CONNECTING]: [SyncState.AUTHENTICATING, SyncState.BACKOFF, SyncState.ERROR, SyncState.DISCONNECTED],
  [SyncState.AUTHENTICATING]: [SyncState.SYNCING, SyncState.BACKOFF, SyncState.ERROR, SyncState.DISCONNECTED],
  [SyncState.SYNCING]: [SyncState.CONNECTED, SyncState.BACKOFF, SyncState.ERROR, SyncState.DISCONNECTED],
  [SyncState.CONNECTED]: [SyncState.SYNCING, SyncState.DISCONNECTED, SyncState.BACKOFF],
  [SyncState.DISCONNECTED]: [SyncState.CONNECTING, SyncState.BACKOFF, SyncState.INITIAL],
  [SyncState.BACKOFF]: [SyncState.CONNECTING, SyncState.DISCONNECTED, SyncState.INITIAL],
  [SyncState.ERROR]: [SyncState.INITIAL],
};

/**
 * Helper function to check if a transition is valid
 */
export function isValidTransition(from: SyncState, to: SyncState): boolean {
  return VALID_TRANSITIONS[from]?.includes(to) ?? false;
}
