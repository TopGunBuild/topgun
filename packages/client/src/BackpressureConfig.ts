/**
 * Backpressure strategy when maxPendingOps is reached.
 */
export type BackpressureStrategy = 'pause' | 'throw' | 'drop-oldest';

/**
 * Configuration for backpressure control on SyncEngine.
 */
export interface BackpressureConfig {
  /**
   * Maximum number of operations waiting for server acknowledgment.
   * When this limit is reached, the configured strategy will be applied.
   * @default 1000
   */
  maxPendingOps: number;

  /**
   * Strategy when maxPendingOps is reached:
   * - 'pause': Wait for capacity (returns Promise that resolves when space available)
   * - 'throw': Throw BackpressureError immediately
   * - 'drop-oldest': Remove oldest pending op to make room (data loss!)
   * @default 'pause'
   */
  strategy: BackpressureStrategy;

  /**
   * High water mark (percentage of maxPendingOps).
   * Emit 'backpressure:high' event when reached.
   * Value should be between 0 and 1.
   * @default 0.8 (80%)
   */
  highWaterMark: number;

  /**
   * Low water mark (percentage of maxPendingOps).
   * Resume paused writes and emit 'backpressure:low' when pending ops drop below this.
   * Value should be between 0 and 1.
   * @default 0.5 (50%)
   */
  lowWaterMark: number;
}

/**
 * Default backpressure configuration.
 */
export const DEFAULT_BACKPRESSURE_CONFIG: BackpressureConfig = {
  maxPendingOps: 1000,
  strategy: 'pause',
  highWaterMark: 0.8,
  lowWaterMark: 0.5,
};

/**
 * Status of backpressure mechanism.
 */
export interface BackpressureStatus {
  /** Current number of pending (unacknowledged) operations */
  pending: number;
  /** Maximum allowed pending operations */
  max: number;
  /** Percentage of capacity used (0-1) */
  percentage: number;
  /** Whether writes are currently paused due to backpressure */
  isPaused: boolean;
  /** Current backpressure strategy */
  strategy: BackpressureStrategy;
}

/**
 * Event data for backpressure:high and backpressure:low events.
 */
export interface BackpressureThresholdEvent {
  pending: number;
  max: number;
}

/**
 * Event data for operation:dropped event.
 */
export interface OperationDroppedEvent {
  opId: string;
  mapName: string;
  opType: string;
  key: string;
}
