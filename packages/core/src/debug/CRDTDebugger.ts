import { Timestamp, HLC } from '../HLC';
import { LWWMap, LWWRecord } from '../LWWMap';

/**
 * Snapshot of a CRDT operation for debugging and replay.
 */
export interface CRDTSnapshot {
  id: string;
  timestamp: Timestamp;
  operation: 'set' | 'delete' | 'merge';
  mapId: string;
  key?: string;
  value?: unknown;
  oldValue?: unknown;
  nodeId: string;
  merkleRoot?: string;
  metadata?: Record<string, unknown>;
}

/**
 * Information about a resolved conflict.
 */
export interface ConflictInfo {
  key: string;
  winnerTimestamp: Timestamp;
  winnerNodeId: string;
  winnerValue: unknown;
  loserTimestamp: Timestamp;
  loserNodeId: string;
  loserValue: unknown;
  resolvedAt: Date;
}

/**
 * Statistics about CRDT operations.
 */
export interface DebugStatistics {
  totalOperations: number;
  operationsByType: Record<string, number>;
  operationsByNode: Record<string, number>;
  conflictsResolved: number;
  timeRange: {
    start: Timestamp | null;
    end: Timestamp | null;
  };
  uniqueKeys: number;
  averageOpsPerSecond: number;
}

/**
 * Options for querying operations.
 */
export interface OperationQueryOptions {
  mapId?: string;
  nodeId?: string;
  operation?: string;
  since?: Timestamp;
  until?: Timestamp;
  limit?: number;
}

/**
 * CRDTDebugger - Records and analyzes CRDT operations for debugging.
 *
 * Features:
 * - Record all CRDT operations (set, delete, merge)
 * - Detect and track conflicts
 * - Replay operations to any point in time
 * - Export/import operation history
 * - Statistics and analysis
 *
 * @see PHASE_14C_OBSERVABILITY.md for specification
 */
export class CRDTDebugger {
  private snapshots: CRDTSnapshot[] = [];
  private conflicts: ConflictInfo[] = [];
  private maxSnapshots: number;
  private enabled: boolean;
  private idCounter = 0;

  constructor(options: { maxSnapshots?: number; enabled?: boolean } = {}) {
    this.maxSnapshots = options.maxSnapshots || 10000;
    this.enabled = options.enabled ?? process.env.CRDT_DEBUG === 'true';
  }

  // ============================================================================
  // Recording Control
  // ============================================================================

  isEnabled(): boolean {
    return this.enabled;
  }

  enable(): void {
    this.enabled = true;
  }

  disable(): void {
    this.enabled = false;
  }

  // ============================================================================
  // Operation Recording
  // ============================================================================

  recordOperation(snapshot: Omit<CRDTSnapshot, 'id'>): void {
    if (!this.enabled) return;

    this.snapshots.push({
      ...snapshot,
      id: `op-${++this.idCounter}`,
    });

    // Trim if exceeds max
    if (this.snapshots.length > this.maxSnapshots) {
      this.snapshots = this.snapshots.slice(-this.maxSnapshots);
    }
  }

  recordSet(
    mapId: string,
    key: string,
    value: unknown,
    timestamp: Timestamp,
    nodeId: string,
    oldValue?: unknown,
    merkleRoot?: string
  ): void {
    this.recordOperation({
      timestamp,
      operation: 'set',
      mapId,
      key,
      value,
      oldValue,
      nodeId,
      merkleRoot,
    });
  }

  recordDelete(
    mapId: string,
    key: string,
    timestamp: Timestamp,
    nodeId: string,
    oldValue?: unknown,
    merkleRoot?: string
  ): void {
    this.recordOperation({
      timestamp,
      operation: 'delete',
      mapId,
      key,
      oldValue,
      nodeId,
      merkleRoot,
    });
  }

  recordMerge(
    mapId: string,
    key: string,
    value: unknown,
    timestamp: Timestamp,
    nodeId: string,
    wasUpdated: boolean,
    oldValue?: unknown,
    merkleRoot?: string
  ): void {
    this.recordOperation({
      timestamp,
      operation: 'merge',
      mapId,
      key,
      value,
      oldValue,
      nodeId,
      merkleRoot,
      metadata: { wasUpdated },
    });
  }

  recordConflict(conflict: ConflictInfo): void {
    if (!this.enabled) return;
    this.conflicts.push(conflict);
  }

  // ============================================================================
  // Querying
  // ============================================================================

  getOperations(options: OperationQueryOptions = {}): CRDTSnapshot[] {
    let results = this.snapshots;

    if (options.mapId) {
      results = results.filter((s) => s.mapId === options.mapId);
    }

    if (options.nodeId) {
      results = results.filter((s) => s.nodeId === options.nodeId);
    }

    if (options.operation) {
      results = results.filter((s) => s.operation === options.operation);
    }

    if (options.since) {
      results = results.filter(
        (s) => this.compareTimestamp(s.timestamp, options.since!) >= 0
      );
    }

    if (options.until) {
      results = results.filter(
        (s) => this.compareTimestamp(s.timestamp, options.until!) <= 0
      );
    }

    if (options.limit) {
      results = results.slice(-options.limit);
    }

    return results;
  }

  getConflicts(mapId?: string): ConflictInfo[] {
    if (mapId) {
      // Filter by looking at snapshots associated with conflicts
      return this.conflicts;
    }
    return this.conflicts;
  }

  getLastOperation(): CRDTSnapshot | undefined {
    return this.snapshots[this.snapshots.length - 1];
  }

  getOperationsForKey(mapId: string, key: string): CRDTSnapshot[] {
    return this.snapshots.filter((s) => s.mapId === mapId && s.key === key);
  }

  // ============================================================================
  // Statistics
  // ============================================================================

  getStatistics(mapId?: string): DebugStatistics {
    const ops = mapId
      ? this.snapshots.filter((s) => s.mapId === mapId)
      : this.snapshots;

    const operationsByType: Record<string, number> = {};
    const operationsByNode: Record<string, number> = {};
    const uniqueKeys = new Set<string>();

    for (const op of ops) {
      operationsByType[op.operation] = (operationsByType[op.operation] || 0) + 1;
      operationsByNode[op.nodeId] = (operationsByNode[op.nodeId] || 0) + 1;
      if (op.key) uniqueKeys.add(op.key);
    }

    const timeRange = {
      start: ops.length > 0 ? ops[0].timestamp : null,
      end: ops.length > 0 ? ops[ops.length - 1].timestamp : null,
    };

    let avgOpsPerSecond = 0;
    if (timeRange.start && timeRange.end) {
      const durationMs = timeRange.end.millis - timeRange.start.millis;
      if (durationMs > 0) {
        avgOpsPerSecond = (ops.length / durationMs) * 1000;
      }
    }

    return {
      totalOperations: ops.length,
      operationsByType,
      operationsByNode,
      conflictsResolved: this.conflicts.length,
      timeRange,
      uniqueKeys: uniqueKeys.size,
      averageOpsPerSecond: avgOpsPerSecond,
    };
  }

  // ============================================================================
  // Replay
  // ============================================================================

  /**
   * Replays operations up to the target timestamp and returns the map state.
   */
  replayUntil<K extends string, V>(
    targetTimestamp: Timestamp,
    mapId?: string,
    hlc?: HLC
  ): LWWMap<K, V> {
    const map = new LWWMap<K, V>(hlc || new HLC('replay-node'));
    const ops = this.getOperations({ mapId, until: targetTimestamp });

    for (const op of ops) {
      if (op.operation === 'set' && op.key !== undefined) {
        // Use merge to set with the original timestamp
        const record: LWWRecord<V> = {
          value: op.value as V,
          timestamp: op.timestamp,
        };
        map.merge(op.key as K, record);
      } else if (op.operation === 'delete' && op.key !== undefined) {
        const record: LWWRecord<V> = {
          value: null,
          timestamp: op.timestamp,
        };
        map.merge(op.key as K, record);
      } else if (op.operation === 'merge' && op.key !== undefined) {
        const record: LWWRecord<V> = {
          value: op.value as V | null,
          timestamp: op.timestamp,
        };
        map.merge(op.key as K, record);
      }
    }

    return map;
  }

  /**
   * Creates a timeline of operations grouped by time intervals.
   */
  getTimeline(
    intervalMs: number = 1000,
    mapId?: string
  ): Array<{ timestamp: number; operations: CRDTSnapshot[] }> {
    const ops = mapId
      ? this.snapshots.filter((s) => s.mapId === mapId)
      : this.snapshots;

    if (ops.length === 0) return [];

    const timeline: Array<{ timestamp: number; operations: CRDTSnapshot[] }> = [];
    let currentBucket: CRDTSnapshot[] = [];
    let bucketStart = Math.floor(ops[0].timestamp.millis / intervalMs) * intervalMs;

    for (const op of ops) {
      const opBucket = Math.floor(op.timestamp.millis / intervalMs) * intervalMs;

      if (opBucket !== bucketStart) {
        if (currentBucket.length > 0) {
          timeline.push({ timestamp: bucketStart, operations: currentBucket });
        }
        currentBucket = [];
        bucketStart = opBucket;
      }

      currentBucket.push(op);
    }

    if (currentBucket.length > 0) {
      timeline.push({ timestamp: bucketStart, operations: currentBucket });
    }

    return timeline;
  }

  // ============================================================================
  // Export / Import
  // ============================================================================

  exportHistory(format: 'json' | 'csv' | 'ndjson' = 'json'): string {
    switch (format) {
      case 'csv':
        return this.toCSV();
      case 'ndjson':
        return this.snapshots.map((s) => JSON.stringify(s)).join('\n');
      default:
        return JSON.stringify(
          {
            version: '1.0',
            exportedAt: new Date().toISOString(),
            statistics: this.getStatistics(),
            operations: this.snapshots,
            conflicts: this.conflicts,
          },
          null,
          2
        );
    }
  }

  private toCSV(): string {
    const header =
      'id,timestamp_millis,timestamp_counter,operation,map_id,key,node_id,merkle_root\n';
    const rows = this.snapshots
      .map(
        (s) =>
          `${s.id},${s.timestamp.millis},${s.timestamp.counter},${s.operation},${s.mapId},${s.key || ''},${s.nodeId},${s.merkleRoot || ''}`
      )
      .join('\n');
    return header + rows;
  }

  importHistory(json: string): void {
    const data = JSON.parse(json);

    if (data.version === '1.0') {
      this.snapshots = data.operations || [];
      this.conflicts = data.conflicts || [];
      // Update idCounter to avoid collisions
      const maxId = this.snapshots.reduce((max, s) => {
        const num = parseInt(s.id.replace('op-', ''), 10);
        return isNaN(num) ? max : Math.max(max, num);
      }, 0);
      this.idCounter = maxId;
    } else {
      // Legacy format: just array of snapshots
      this.snapshots = Array.isArray(data) ? data : [];
    }
  }

  // ============================================================================
  // Diff
  // ============================================================================

  /**
   * Compares two points in time and returns the differences.
   */
  diff(
    fromTimestamp: Timestamp,
    toTimestamp: Timestamp,
    mapId?: string
  ): {
    added: CRDTSnapshot[];
    modified: CRDTSnapshot[];
    deleted: CRDTSnapshot[];
  } {
    const fromOps = this.getOperations({ mapId, until: fromTimestamp });
    const toOps = this.getOperations({ mapId, until: toTimestamp });

    // Build state maps
    const fromState = new Map<string, CRDTSnapshot>();
    const toState = new Map<string, CRDTSnapshot>();

    for (const op of fromOps) {
      if (op.key) {
        fromState.set(`${op.mapId}:${op.key}`, op);
      }
    }

    for (const op of toOps) {
      if (op.key) {
        toState.set(`${op.mapId}:${op.key}`, op);
      }
    }

    const added: CRDTSnapshot[] = [];
    const modified: CRDTSnapshot[] = [];
    const deleted: CRDTSnapshot[] = [];

    // Find additions and modifications
    for (const [key, toOp] of toState) {
      const fromOp = fromState.get(key);
      if (!fromOp) {
        if (toOp.operation !== 'delete') {
          added.push(toOp);
        }
      } else if (toOp.operation === 'delete' && fromOp.operation !== 'delete') {
        deleted.push(toOp);
      } else if (
        toOp.operation !== 'delete' &&
        JSON.stringify(toOp.value) !== JSON.stringify(fromOp.value)
      ) {
        modified.push(toOp);
      }
    }

    return { added, modified, deleted };
  }

  // ============================================================================
  // Utilities
  // ============================================================================

  clear(): void {
    this.snapshots = [];
    this.conflicts = [];
    this.idCounter = 0;
  }

  private compareTimestamp(a: Timestamp, b: Timestamp): number {
    if (a.millis !== b.millis) return a.millis - b.millis;
    if (a.counter !== b.counter) return a.counter - b.counter;
    return a.nodeId.localeCompare(b.nodeId);
  }
}

// Singleton instance
let globalDebugger: CRDTDebugger | null = null;

export function getCRDTDebugger(): CRDTDebugger {
  if (!globalDebugger) {
    globalDebugger = new CRDTDebugger();
  }
  return globalDebugger;
}

export function resetCRDTDebugger(): void {
  globalDebugger = null;
}
