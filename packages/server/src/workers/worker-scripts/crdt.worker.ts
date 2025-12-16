/**
 * CRDT Merge Worker Script
 * Phase 1.04: CRDTMergeWorker Implementation
 *
 * Handles CPU-intensive CRDT merge operations:
 * - lww-merge: Merge LWWMap records (Last-Write-Wins)
 * - ormap-merge: Merge ORMap items and tombstones
 */

import { registerHandler } from './base.worker';
import type {
  LWWMergePayload,
  LWWMergeResult,
  ORMapMergePayload,
  ORMapMergeResult,
} from '../crdt-types';

// ============ Timestamp Comparison ============

interface Timestamp {
  millis: number;
  counter: number;
  nodeId: string;
}

/**
 * Compare two timestamps (same logic as HLC.compare)
 * Returns:
 *   < 0 if a < b
 *   > 0 if a > b
 *   = 0 if a === b
 */
function compareTimestamps(a: Timestamp, b: Timestamp): number {
  if (a.millis !== b.millis) {
    return a.millis - b.millis;
  }
  if (a.counter !== b.counter) {
    return a.counter - b.counter;
  }
  return a.nodeId.localeCompare(b.nodeId);
}

// ============ Handler: lww-merge ============

registerHandler('lww-merge', (payload: unknown): LWWMergeResult => {
  const { records, existingState } = payload as LWWMergePayload;

  // Build existing state map for O(1) lookup
  const existingMap = new Map<string, {
    value: unknown;
    timestamp: Timestamp;
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
      // No existing record - apply new one
      toApply.push({
        key: record.key,
        value: record.value,
        timestamp: record.timestamp,
        ttlMs: record.ttlMs,
      });
      // Update existingMap for subsequent records in batch
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
      // New record is newer - apply it
      toApply.push({
        key: record.key,
        value: record.value,
        timestamp: record.timestamp,
        ttlMs: record.ttlMs,
      });
      // Update for subsequent records
      existingMap.set(record.key, {
        value: record.value,
        timestamp: record.timestamp,
        ttlMs: record.ttlMs,
      });
      if (isConflict) {
        conflicts.push(record.key);
      }
    } else if (cmp === 0) {
      // Same timestamp - this is a conflict
      conflicts.push(record.key);
      skipped++;
    } else {
      // New record is older - skip
      if (isConflict) {
        conflicts.push(record.key);
      }
      skipped++;
    }
  }

  return {
    toApply,
    skipped,
    conflicts,
  };
});

// ============ Handler: ormap-merge ============

registerHandler('ormap-merge', (payload: unknown): ORMapMergeResult => {
  const {
    items,
    tombstones,
    existingTags,
    existingTombstones,
  } = payload as ORMapMergePayload;

  // Build sets for O(1) lookup
  const tagSet = new Set(existingTags);
  const tombstoneSet = new Set(existingTombstones);

  const itemsToApply: ORMapMergeResult['itemsToApply'] = [];
  const tombstonesToApply: string[] = [];
  const tagsToRemove: string[] = [];
  let itemsSkipped = 0;
  let tombstonesSkipped = 0;

  // Process tombstones first (they take precedence)
  for (const tombstone of tombstones) {
    if (tombstoneSet.has(tombstone.tag)) {
      // Already have this tombstone
      tombstonesSkipped++;
      continue;
    }

    // New tombstone - should be applied
    tombstonesToApply.push(tombstone.tag);
    tombstoneSet.add(tombstone.tag);

    // If this tag exists in items, mark for removal
    if (tagSet.has(tombstone.tag)) {
      tagsToRemove.push(tombstone.tag);
      tagSet.delete(tombstone.tag);
    }
  }

  // Process items
  for (const item of items) {
    // Check if tag is tombstoned
    if (tombstoneSet.has(item.tag)) {
      itemsSkipped++;
      continue;
    }

    // Check if tag already exists
    if (tagSet.has(item.tag)) {
      // Tag already exists - OR-Map semantics: same tag = same item
      // We could update value if timestamp is newer, but in pure ORMap
      // the tag is unique and immutable
      itemsSkipped++;
      continue;
    }

    // New item - apply it
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
});
