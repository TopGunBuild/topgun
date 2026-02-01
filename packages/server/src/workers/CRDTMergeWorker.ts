/**
 * CRDTMergeWorker - High-level API for CRDT merge operations in worker threads
 * CRDTMergeWorker Implementation
 *
 * Provides a clean interface for CPU-intensive CRDT merge operations.
 * Delegates actual work to worker threads via WorkerPool for large batches.
 */

import { join } from 'path';
import { WorkerPool } from './WorkerPool';
import type { WorkerTask, WorkerTaskType } from './types';
import type {
  LWWMergePayload,
  LWWMergeResult,
  ORMapMergePayload,
  ORMapMergeResult,
  LWWMergeRecord,
  LWWExistingRecord,
  ORMapMergeItem,
  ORMapMergeTombstone,
} from './crdt-types';

// Threshold: use worker only if batch exceeds this count
const WORKER_THRESHOLD = 10;

let taskIdCounter = 0;

function generateTaskId(): string {
  return `crdt-${Date.now()}-${++taskIdCounter}`;
}

/**
 * Compare two timestamps (same logic as HLC.compare)
 */
function compareTimestamps(
  a: { millis: number; counter: number; nodeId: string },
  b: { millis: number; counter: number; nodeId: string }
): number {
  if (a.millis !== b.millis) {
    return a.millis - b.millis;
  }
  if (a.counter !== b.counter) {
    return a.counter - b.counter;
  }
  return a.nodeId.localeCompare(b.nodeId);
}

/**
 * CRDTMergeWorker provides methods for CRDT merge operations.
 * Automatically decides whether to use worker threads based on batch size.
 */
export class CRDTMergeWorker {
  private readonly pool: WorkerPool;

  /** Threshold for using worker (operations below this go to main thread) */
  static readonly BATCH_THRESHOLD = WORKER_THRESHOLD;

  constructor(pool: WorkerPool) {
    this.pool = pool;
  }

  /**
   * Decide if batch should go to worker
   */
  shouldUseWorker(batchSize: number): boolean {
    return batchSize >= WORKER_THRESHOLD;
  }

  /**
   * Merge LWW records
   * @param payload - Records to merge and existing state
   * @returns Which records should be applied
   */
  async mergeLWW(payload: LWWMergePayload): Promise<LWWMergeResult> {
    if (!this.shouldUseWorker(payload.records.length)) {
      return this.mergeLWWInline(payload);
    }

    const task: WorkerTask<LWWMergePayload, LWWMergeResult> = {
      id: generateTaskId(),
      type: 'lww-merge' as WorkerTaskType,
      payload,
      priority: 'high', // Merge operations should be prioritized
    };

    return this.pool.submit(task);
  }

  /**
   * Merge ORMap items and tombstones
   * @param payload - Items, tombstones, and existing state
   * @returns Which items/tombstones should be applied
   */
  async mergeORMap(payload: ORMapMergePayload): Promise<ORMapMergeResult> {
    const totalOps = payload.items.length + payload.tombstones.length;

    if (!this.shouldUseWorker(totalOps)) {
      return this.mergeORMapInline(payload);
    }

    const task: WorkerTask<ORMapMergePayload, ORMapMergeResult> = {
      id: generateTaskId(),
      type: 'ormap-merge' as WorkerTaskType,
      payload,
      priority: 'high',
    };

    return this.pool.submit(task);
  }

  // ============ Inline implementations for small batches ============

  private mergeLWWInline(payload: LWWMergePayload): LWWMergeResult {
    const { records, existingState } = payload;

    // Build existing state map
    const existingMap = new Map<string, {
      value: unknown;
      timestamp: { millis: number; counter: number; nodeId: string };
      ttlMs?: number;
    }>();

    for (const existing of existingState) {
      existingMap.set(existing.key, {
        value: existing.value,
        timestamp: existing.timestamp,
        ttlMs: existing.ttlMs,
      });
    }

    const toApply: LWWMergeResult['toApply'] = [];
    const conflicts: string[] = [];
    let skipped = 0;

    for (const record of records) {
      const existing = existingMap.get(record.key);

      if (!existing) {
        toApply.push({
          key: record.key,
          value: record.value,
          timestamp: record.timestamp,
          ttlMs: record.ttlMs,
        });
        existingMap.set(record.key, {
          value: record.value,
          timestamp: record.timestamp,
          ttlMs: record.ttlMs,
        });
        continue;
      }

      const cmp = compareTimestamps(record.timestamp, existing.timestamp);

      // Detect conflict: same millis but different counter/nodeId (concurrent writes)
      const isConflict = record.timestamp.millis === existing.timestamp.millis &&
        (record.timestamp.counter !== existing.timestamp.counter ||
         record.timestamp.nodeId !== existing.timestamp.nodeId);

      if (cmp > 0) {
        // New record wins
        toApply.push({
          key: record.key,
          value: record.value,
          timestamp: record.timestamp,
          ttlMs: record.ttlMs,
        });
        existingMap.set(record.key, {
          value: record.value,
          timestamp: record.timestamp,
          ttlMs: record.ttlMs,
        });
        if (isConflict) {
          conflicts.push(record.key);
        }
      } else if (cmp === 0) {
        // Identical timestamps - skip but note conflict
        conflicts.push(record.key);
        skipped++;
      } else {
        // Old record - skip
        if (isConflict) {
          conflicts.push(record.key);
        }
        skipped++;
      }
    }

    return { toApply, skipped, conflicts };
  }

  private mergeORMapInline(payload: ORMapMergePayload): ORMapMergeResult {
    const { items, tombstones, existingTags, existingTombstones } = payload;

    const tagSet = new Set(existingTags);
    const tombstoneSet = new Set(existingTombstones);

    const itemsToApply: ORMapMergeResult['itemsToApply'] = [];
    const tombstonesToApply: string[] = [];
    const tagsToRemove: string[] = [];
    let itemsSkipped = 0;
    let tombstonesSkipped = 0;

    // Process tombstones first
    for (const tombstone of tombstones) {
      if (tombstoneSet.has(tombstone.tag)) {
        tombstonesSkipped++;
        continue;
      }

      tombstonesToApply.push(tombstone.tag);
      tombstoneSet.add(tombstone.tag);

      if (tagSet.has(tombstone.tag)) {
        tagsToRemove.push(tombstone.tag);
        tagSet.delete(tombstone.tag);
      }
    }

    // Process items
    for (const item of items) {
      if (tombstoneSet.has(item.tag)) {
        itemsSkipped++;
        continue;
      }

      if (tagSet.has(item.tag)) {
        itemsSkipped++;
        continue;
      }

      itemsToApply.push({
        key: item.key,
        value: item.value,
        timestamp: item.timestamp,
        tag: item.tag,
        ttlMs: item.ttlMs,
      });
      tagSet.add(item.tag);
    }

    return {
      itemsToApply,
      tombstonesToApply,
      tagsToRemove,
      itemsSkipped,
      tombstonesSkipped,
    };
  }
}
