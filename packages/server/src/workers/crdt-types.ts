/**
 * CRDT Worker Types
 * CRDTMergeWorker Implementation
 */

import type { Timestamp } from '@topgunbuild/core';

// ============ LWW Merge Types ============

/**
 * Single LWW record for merge
 */
export interface LWWMergeRecord {
  key: string;
  value: unknown;
  timestamp: Timestamp;
  ttlMs?: number;
}

/**
 * Existing state for a key (for merge comparison)
 * Note: value is optional since we only need timestamp for comparison
 */
export interface LWWExistingRecord {
  key: string;
  timestamp: Timestamp;
  value?: unknown;
  ttlMs?: number;
}

/**
 * Payload for lww-merge task
 */
export interface LWWMergePayload {
  mapName: string;
  /** Records to merge */
  records: LWWMergeRecord[];
  /** Current state of affected keys (for comparison) */
  existingState: LWWExistingRecord[];
}

/**
 * Result of lww-merge task
 */
export interface LWWMergeResult {
  /** Records that should be applied (newer than existing) */
  toApply: Array<{
    key: string;
    value: unknown;
    timestamp: Timestamp;
    ttlMs?: number;
  }>;
  /** Number of records skipped (older timestamp) */
  skipped: number;
  /** Keys with concurrent updates (same timestamp, different nodeId) */
  conflicts: string[];
}

// ============ ORMap Merge Types ============

/**
 * Single ORMap item for merge
 */
export interface ORMapMergeItem {
  key: string;
  value: unknown;
  timestamp: Timestamp;
  tag: string;
  ttlMs?: number;
}

/**
 * Single ORMap tombstone for merge
 */
export interface ORMapMergeTombstone {
  tag: string;
  timestamp: Timestamp;
}

/**
 * Existing ORMap item state
 */
export interface ORMapExistingItem {
  key: string;
  tag: string;
  value: unknown;
  timestamp: Timestamp;
  ttlMs?: number;
}

/**
 * Payload for ormap-merge task
 */
export interface ORMapMergePayload {
  mapName: string;
  /** Items to merge */
  items: ORMapMergeItem[];
  /** Tombstones to merge */
  tombstones: ORMapMergeTombstone[];
  /** Existing tags in the map (for tombstone check) */
  existingTags: string[];
  /** Existing tombstones (to avoid re-adding deleted items) */
  existingTombstones: string[];
}

/**
 * Result of ormap-merge task
 */
export interface ORMapMergeResult {
  /** Items that should be applied */
  itemsToApply: Array<{
    key: string;
    value: unknown;
    timestamp: Timestamp;
    tag: string;
    ttlMs?: number;
  }>;
  /** Tombstones that should be applied */
  tombstonesToApply: string[];
  /** Tags that need to be removed due to new tombstones */
  tagsToRemove: string[];
  /** Number of items skipped */
  itemsSkipped: number;
  /** Number of tombstones skipped */
  tombstonesSkipped: number;
}

// ============ Batch Merge Types ============

/**
 * Combined batch merge payload (for mixed operations)
 */
export interface BatchMergePayload {
  lww?: LWWMergePayload;
  ormap?: ORMapMergePayload;
}

/**
 * Combined batch merge result
 */
export interface BatchMergeResult {
  lww?: LWWMergeResult;
  ormap?: ORMapMergeResult;
}
