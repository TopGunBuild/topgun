/**
 * Write Concern - Configurable Acknowledgment Levels
 *
 * Write Concern defines when a write operation is considered successful.
 * Similar to MongoDB's writeConcern, Kafka's acks, and Cassandra's consistency levels.
 */

/**
 * Write Concern levels - determines when an operation is acknowledged.
 *
 * Levels are ordered by durability guarantee (lowest to highest):
 * FIRE_AND_FORGET < MEMORY < APPLIED < REPLICATED < PERSISTED
 */
export enum WriteConcern {
  /**
   * FIRE_AND_FORGET (acks=0)
   * - ACK sent immediately after server receives the message
   * - Operation may be lost if server crashes before processing
   * - Maximum throughput, minimum latency
   * - Use case: metrics, logs, non-critical data
   */
  FIRE_AND_FORGET = 'FIRE_AND_FORGET',

  /**
   * MEMORY (acks=1, default) - current Early ACK behavior
   * - ACK sent after validation and queuing for processing
   * - Operation is in memory but not yet applied to CRDT
   * - Use case: most operations, real-time updates
   */
  MEMORY = 'MEMORY',

  /**
   * APPLIED
   * - ACK sent after operation is applied to CRDT in memory
   * - Data is readable on this node immediately after ACK
   * - Use case: operations requiring immediate consistency on the node
   */
  APPLIED = 'APPLIED',

  /**
   * REPLICATED
   * - ACK sent after operation is broadcast to cluster (CLUSTER_EVENT sent)
   * - Data survives primary node failure
   * - Use case: important data requiring cluster durability
   */
  REPLICATED = 'REPLICATED',

  /**
   * PERSISTED
   * - ACK sent after operation is written to storage on primary node
   * - Data survives node restart
   * - Use case: critical data (financial transactions, audit logs)
   */
  PERSISTED = 'PERSISTED',
}

/**
 * Default timeout for Write Concern acknowledgments (ms)
 */
export const DEFAULT_WRITE_CONCERN_TIMEOUT = 5000;

/**
 * Write options for PUT/REMOVE operations
 */
export interface WriteOptions {
  /**
   * Write acknowledgment level.
   * @default WriteConcern.MEMORY
   */
  writeConcern?: WriteConcern;

  /**
   * Timeout for waiting for acknowledgment (ms).
   * Applies to APPLIED, REPLICATED, PERSISTED levels.
   * @default 5000
   */
  timeout?: number;
}

/**
 * Result of a write operation
 */
export interface WriteResult {
  /** Whether the operation achieved the requested Write Concern level */
  success: boolean;

  /** Operation ID */
  opId: string;

  /** The Write Concern level actually achieved */
  achievedLevel: WriteConcern;

  /** Time taken to achieve the level (ms) */
  latencyMs: number;

  /** Error message if success=false */
  error?: string;
}

/**
 * Internal structure for tracking pending write acknowledgments
 */
export interface PendingWrite {
  /** Operation ID */
  opId: string;

  /** Target Write Concern level */
  writeConcern: WriteConcern;

  /** Timestamp when operation was registered */
  timestamp: number;

  /** Timeout duration (ms) */
  timeout: number;

  /** Promise resolve callback */
  resolve: (result: WriteResult) => void;

  /** Promise reject callback */
  reject: (error: Error) => void;

  /** Timeout handle for cleanup */
  timeoutHandle?: ReturnType<typeof setTimeout>;

  /** Set of levels that have been achieved */
  achievedLevels: Set<WriteConcern>;
}

/**
 * Ordered list of Write Concern levels (lowest to highest)
 */
export const WRITE_CONCERN_ORDER: readonly WriteConcern[] = [
  WriteConcern.FIRE_AND_FORGET,
  WriteConcern.MEMORY,
  WriteConcern.APPLIED,
  WriteConcern.REPLICATED,
  WriteConcern.PERSISTED,
] as const;

/**
 * Check if a target Write Concern level is achieved based on achieved levels.
 *
 * @param achieved - Set of achieved Write Concern levels
 * @param target - Target Write Concern level to check
 * @returns true if target level (or higher) is achieved
 */
export function isWriteConcernAchieved(
  achieved: Set<WriteConcern>,
  target: WriteConcern
): boolean {
  const targetIndex = WRITE_CONCERN_ORDER.indexOf(target);
  const achievedIndex = Math.max(
    ...Array.from(achieved).map((l) => WRITE_CONCERN_ORDER.indexOf(l))
  );
  return achievedIndex >= targetIndex;
}

/**
 * Get the highest achieved Write Concern level from a set of achieved levels.
 *
 * @param achieved - Set of achieved Write Concern levels
 * @returns The highest achieved level
 */
export function getHighestWriteConcernLevel(
  achieved: Set<WriteConcern>
): WriteConcern {
  for (let i = WRITE_CONCERN_ORDER.length - 1; i >= 0; i--) {
    if (achieved.has(WRITE_CONCERN_ORDER[i])) {
      return WRITE_CONCERN_ORDER[i];
    }
  }
  return WriteConcern.FIRE_AND_FORGET;
}
