/**
 * Merkle Worker Script
 * MerkleWorker Implementation
 *
 * Handles CPU-intensive Merkle tree operations:
 * - merkle-hash: Compute hashes for batch of LWWMap entries
 * - merkle-hash-ormap: Compute hashes for batch of ORMap entries
 * - merkle-diff: Find differences between local and remote trees
 * - merkle-rebuild: Rebuild tree from records
 */

import { registerHandler } from './base.worker';
import type {
  MerkleHashPayload,
  MerkleHashResult,
  ORMapMerkleHashPayload,
  ORMapMerkleHashResult,
  MerkleDiffPayload,
  MerkleDiffResult,
  MerkleRebuildPayload,
  MerkleRebuildResult,
  ORMapMerkleRebuildPayload,
  BucketInfo,
} from '../merkle-types';

// ============ Hash Functions (same as core) ============

/**
 * FNV-1a Hash implementation for strings.
 * Identical to packages/core/src/utils/hash.ts
 */
function hashString(str: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * Compute item hash for LWWMap entry.
 * Same as in MerkleTree.update()
 */
function computeItemHash(
  key: string,
  millis: number,
  counter: number,
  nodeId: string
): number {
  return hashString(`${key}:${millis}:${counter}:${nodeId}`);
}

/**
 * Compute entry hash for ORMap entry.
 * Same as hashORMapEntry in ORMapMerkle.ts
 */
function computeORMapEntryHash(
  key: string,
  records: Array<{ tag: string; timestamp: { millis: number; counter: number; nodeId: string } }>
): number {
  // Sort records by tag for deterministic hashing
  const sortedRecords = [...records].sort((a, b) => a.tag.localeCompare(b.tag));

  let combinedStr = key;
  for (const record of sortedRecords) {
    combinedStr += `:${record.tag}:${record.timestamp.millis}:${record.timestamp.counter}:${record.timestamp.nodeId}`;
  }

  return hashString(combinedStr);
}

// ============ Merkle Tree Node Structure ============

interface MerkleNode {
  hash: number;
  children?: { [key: string]: MerkleNode };
  entries?: Map<string, number>;
}

/**
 * Build a Merkle tree from entries
 */
function buildMerkleTree(
  entries: Array<{ key: string; hash: number }>,
  depth: number
): { root: MerkleNode; buckets: Map<string, { hash: number; keys: string[] }> } {
  const root: MerkleNode = { hash: 0, children: {} };
  const buckets = new Map<string, { hash: number; keys: string[] }>();

  for (const { key, hash: itemHash } of entries) {
    const pathHash = hashString(key).toString(16).padStart(8, '0');
    updateNode(root, key, itemHash, pathHash, 0, depth);
  }

  // Collect buckets at leaf level
  collectBuckets(root, '', depth, buckets);

  return { root, buckets };
}

function updateNode(
  node: MerkleNode,
  key: string,
  itemHash: number,
  pathHash: string,
  level: number,
  depth: number
): number {
  // Leaf Node Logic
  if (level >= depth) {
    if (!node.entries) node.entries = new Map();
    node.entries.set(key, itemHash);

    // Recalculate leaf hash (Sum of item hashes)
    let h = 0;
    for (const val of node.entries.values()) {
      h = (h + val) | 0;
    }
    node.hash = h >>> 0;
    return node.hash;
  }

  // Intermediate Node Logic
  const bucketChar = pathHash[level];
  if (!node.children) node.children = {};

  if (!node.children[bucketChar]) {
    node.children[bucketChar] = { hash: 0 };
  }

  updateNode(node.children[bucketChar], key, itemHash, pathHash, level + 1, depth);

  // Recalculate this node's hash from children
  let h = 0;
  for (const child of Object.values(node.children)) {
    h = (h + child.hash) | 0;
  }
  node.hash = h >>> 0;
  return node.hash;
}

function collectBuckets(
  node: MerkleNode,
  path: string,
  depth: number,
  buckets: Map<string, { hash: number; keys: string[] }>
): void {
  if (path.length >= depth) {
    // Leaf level
    if (node.entries && node.entries.size > 0) {
      buckets.set(path, {
        hash: node.hash,
        keys: Array.from(node.entries.keys()),
      });
    }
    return;
  }

  if (node.children) {
    for (const [char, child] of Object.entries(node.children)) {
      collectBuckets(child, path + char, depth, buckets);
    }
  }
}

// ============ Handler: merkle-hash (LWWMap) ============

registerHandler('merkle-hash', (payload: unknown): MerkleHashResult => {
  const { entries, depth = 3 } = payload as MerkleHashPayload;

  // Compute hashes for each entry
  const hashEntries: Array<{ key: string; hash: number }> = [];
  const hashes: Array<[string, number]> = [];

  for (const entry of entries) {
    const itemHash = computeItemHash(
      entry.key,
      entry.timestamp.millis,
      entry.timestamp.counter,
      entry.timestamp.nodeId
    );
    hashEntries.push({ key: entry.key, hash: itemHash });
    hashes.push([entry.key, itemHash]);
  }

  // Build tree
  const { root, buckets } = buildMerkleTree(hashEntries, depth);

  return {
    hashes,
    rootHash: root.hash,
    buckets: Array.from(buckets.entries()),
  };
});

// ============ Handler: merkle-hash-ormap ============

registerHandler('merkle-hash-ormap', (payload: unknown): ORMapMerkleHashResult => {
  const { entries, depth = 3 } = payload as ORMapMerkleHashPayload;

  // Compute hashes for each entry
  const hashEntries: Array<{ key: string; hash: number }> = [];
  const hashes: Array<[string, number]> = [];

  for (const entry of entries) {
    const entryHash = computeORMapEntryHash(entry.key, entry.records);
    hashEntries.push({ key: entry.key, hash: entryHash });
    hashes.push([entry.key, entryHash]);
  }

  // Build tree
  const { root, buckets } = buildMerkleTree(hashEntries, depth);

  return {
    hashes,
    rootHash: root.hash,
    buckets: Array.from(buckets.entries()),
  };
});

// ============ Handler: merkle-diff ============

registerHandler('merkle-diff', (payload: unknown): MerkleDiffResult => {
  const { localBuckets, remoteBuckets } = payload as MerkleDiffPayload;

  const localMap = new Map<string, BucketInfo>(localBuckets);
  const remoteMap = new Map<string, BucketInfo>(remoteBuckets);

  const missingLocal: string[] = [];
  const missingRemote: string[] = [];
  const differingPaths: string[] = [];

  // Find keys missing locally (exist on remote but not local)
  for (const [path, remoteBucket] of remoteMap) {
    const localBucket = localMap.get(path);

    if (!localBucket) {
      // Entire bucket missing locally
      missingLocal.push(...remoteBucket.keys);
    } else if (localBucket.hash !== remoteBucket.hash) {
      // Buckets differ - need deeper comparison
      differingPaths.push(path);

      // Find specific keys that differ
      const localKeys = new Set(localBucket.keys);
      const remoteKeys = new Set(remoteBucket.keys);

      for (const key of remoteKeys) {
        if (!localKeys.has(key)) {
          missingLocal.push(key);
        }
      }

      for (const key of localKeys) {
        if (!remoteKeys.has(key)) {
          missingRemote.push(key);
        }
      }
    }
  }

  // Find keys missing on remote (exist locally but not on remote)
  for (const [path, localBucket] of localMap) {
    if (!remoteMap.has(path)) {
      missingRemote.push(...localBucket.keys);
    }
  }

  return {
    missingLocal,
    missingRemote,
    differingPaths,
  };
});

// ============ Handler: merkle-rebuild (LWWMap) ============

registerHandler('merkle-rebuild', (payload: unknown): MerkleRebuildResult => {
  const { records, depth = 3 } = payload as MerkleRebuildPayload;

  // Compute hashes for each record
  const hashEntries: Array<{ key: string; hash: number }> = [];

  for (const record of records) {
    const itemHash = computeItemHash(
      record.key,
      record.timestamp.millis,
      record.timestamp.counter,
      record.timestamp.nodeId
    );
    hashEntries.push({ key: record.key, hash: itemHash });
  }

  // Build tree
  const { root, buckets } = buildMerkleTree(hashEntries, depth);

  return {
    rootHash: root.hash,
    buckets: Array.from(buckets.entries()),
  };
});

// ============ Handler: merkle-rebuild-ormap ============

registerHandler('merkle-rebuild-ormap', (payload: unknown): MerkleRebuildResult => {
  const { records, depth = 3 } = payload as ORMapMerkleRebuildPayload;

  // Compute hashes for each record
  const hashEntries: Array<{ key: string; hash: number }> = [];

  for (const record of records) {
    const entryHash = computeORMapEntryHash(record.key, record.tags);
    hashEntries.push({ key: record.key, hash: entryHash });
  }

  // Build tree
  const { root, buckets } = buildMerkleTree(hashEntries, depth);

  return {
    rootHash: root.hash,
    buckets: Array.from(buckets.entries()),
  };
});
