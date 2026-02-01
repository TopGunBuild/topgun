/**
 * Merkle Worker Types
 * MerkleWorker Implementation
 */

import type { Timestamp } from '@topgunbuild/core';

/**
 * Entry for Merkle hash computation (LWWMap style)
 */
export interface MerkleHashEntry {
  key: string;
  timestamp: Timestamp;
}

/**
 * Entry for ORMap Merkle hash computation
 */
export interface ORMapMerkleHashEntry {
  key: string;
  records: Array<{
    tag: string;
    timestamp: Timestamp;
  }>;
}

/**
 * Payload for merkle-hash task (LWWMap)
 */
export interface MerkleHashPayload {
  entries: MerkleHashEntry[];
  depth?: number;
}

/**
 * Result of merkle-hash task
 */
export interface MerkleHashResult {
  /** Map of key -> hash for each entry */
  hashes: Array<[string, number]>;
  /** Root hash of all entries */
  rootHash: number;
  /** Bucket structure: path -> { hash, keys } */
  buckets: Array<[string, { hash: number; keys: string[] }]>;
}

/**
 * Payload for merkle-hash-ormap task
 */
export interface ORMapMerkleHashPayload {
  entries: ORMapMerkleHashEntry[];
  depth?: number;
}

/**
 * Result of merkle-hash-ormap task
 */
export interface ORMapMerkleHashResult {
  /** Map of key -> hash for each entry */
  hashes: Array<[string, number]>;
  /** Root hash of all entries */
  rootHash: number;
  /** Bucket structure: path -> { hash, keys } */
  buckets: Array<[string, { hash: number; keys: string[] }]>;
}

/**
 * Bucket info for diff comparison
 */
export interface BucketInfo {
  hash: number;
  keys: string[];
}

/**
 * Payload for merkle-diff task
 */
export interface MerkleDiffPayload {
  /** Local buckets: path -> bucket info */
  localBuckets: Array<[string, BucketInfo]>;
  /** Remote buckets: path -> bucket info */
  remoteBuckets: Array<[string, BucketInfo]>;
}

/**
 * Result of merkle-diff task
 */
export interface MerkleDiffResult {
  /** Keys that exist on remote but not locally */
  missingLocal: string[];
  /** Keys that exist locally but not on remote */
  missingRemote: string[];
  /** Paths where buckets differ (need deeper comparison) */
  differingPaths: string[];
}

/**
 * Payload for merkle-rebuild task (LWWMap)
 */
export interface MerkleRebuildPayload {
  records: Array<{
    key: string;
    timestamp: Timestamp;
  }>;
  depth?: number;
}

/**
 * Payload for merkle-rebuild-ormap task
 */
export interface ORMapMerkleRebuildPayload {
  records: Array<{
    key: string;
    tags: Array<{
      tag: string;
      timestamp: Timestamp;
    }>;
  }>;
  depth?: number;
}

/**
 * Result of merkle-rebuild task
 */
export interface MerkleRebuildResult {
  /** Root hash of rebuilt tree */
  rootHash: number;
  /** All buckets in the tree */
  buckets: Array<[string, { hash: number; keys: string[] }]>;
}
