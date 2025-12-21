/**
 * MerkleWorker - High-level API for Merkle tree operations in worker threads
 * Phase 1.03: MerkleWorker Implementation
 *
 * Provides a clean interface for CPU-intensive Merkle tree operations.
 * Delegates actual work to worker threads via WorkerPool.
 */

import { join } from 'path';
import { WorkerPool } from './WorkerPool';
import type { WorkerTask, WorkerTaskType } from './types';
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
} from './merkle-types';
import { hashString as coreHashString } from '@topgunbuild/core';

// Threshold: use worker only if entries exceed this count
const WORKER_THRESHOLD = 10;

let taskIdCounter = 0;

function generateTaskId(): string {
  return `merkle-${Date.now()}-${++taskIdCounter}`;
}

/**
 * MerkleWorker provides methods for Merkle tree operations.
 * Automatically decides whether to use worker threads based on workload size.
 */
export class MerkleWorker {
  private readonly pool: WorkerPool;
  private readonly workerScript: string;

  constructor(pool: WorkerPool) {
    this.pool = pool;
    // Resolve path to merkle worker script
    this.workerScript = this.resolveMerkleWorkerScript();
  }

  private resolveMerkleWorkerScript(): string {
    const jsPath = join(__dirname, 'worker-scripts', 'merkle.worker.js');
    const tsPath = join(__dirname, 'worker-scripts', 'merkle.worker.ts');

    try {
      require.resolve(jsPath);
      return jsPath;
    } catch {
      return tsPath;
    }
  }

  /**
   * Compute hashes for a batch of LWWMap entries.
   * Uses worker thread if entries count exceeds threshold.
   */
  async computeHashes(payload: MerkleHashPayload): Promise<MerkleHashResult> {
    if (payload.entries.length < WORKER_THRESHOLD) {
      // Execute inline for small batches
      return this.computeHashesInline(payload);
    }

    const task: WorkerTask<MerkleHashPayload, MerkleHashResult> = {
      id: generateTaskId(),
      type: 'merkle-hash' as WorkerTaskType,
      payload,
      priority: 'normal',
    };

    return this.pool.submit(task);
  }

  /**
   * Compute hashes for a batch of ORMap entries.
   * Uses worker thread if entries count exceeds threshold.
   */
  async computeORMapHashes(payload: ORMapMerkleHashPayload): Promise<ORMapMerkleHashResult> {
    if (payload.entries.length < WORKER_THRESHOLD) {
      return this.computeORMapHashesInline(payload);
    }

    const task: WorkerTask<ORMapMerkleHashPayload, ORMapMerkleHashResult> = {
      id: generateTaskId(),
      type: 'merkle-hash-ormap' as WorkerTaskType,
      payload,
      priority: 'normal',
    };

    return this.pool.submit(task);
  }

  /**
   * Find differences between local and remote Merkle trees.
   */
  async diff(payload: MerkleDiffPayload): Promise<MerkleDiffResult> {
    const totalKeys =
      payload.localBuckets.reduce((sum, [, b]) => sum + b.keys.length, 0) +
      payload.remoteBuckets.reduce((sum, [, b]) => sum + b.keys.length, 0);

    if (totalKeys < WORKER_THRESHOLD * 2) {
      return this.diffInline(payload);
    }

    const task: WorkerTask<MerkleDiffPayload, MerkleDiffResult> = {
      id: generateTaskId(),
      type: 'merkle-diff' as WorkerTaskType,
      payload,
      priority: 'high', // Sync operations should be prioritized
    };

    return this.pool.submit(task);
  }

  /**
   * Rebuild Merkle tree from LWWMap records.
   * Always uses worker thread as this is typically a heavy operation.
   */
  async rebuild(payload: MerkleRebuildPayload): Promise<MerkleRebuildResult> {
    if (payload.records.length < WORKER_THRESHOLD) {
      return this.rebuildInline(payload);
    }

    const task: WorkerTask<MerkleRebuildPayload, MerkleRebuildResult> = {
      id: generateTaskId(),
      type: 'merkle-rebuild' as WorkerTaskType,
      payload,
      priority: 'low', // Rebuild can wait
    };

    return this.pool.submit(task);
  }

  /**
   * Rebuild Merkle tree from ORMap records.
   */
  async rebuildORMap(payload: ORMapMerkleRebuildPayload): Promise<MerkleRebuildResult> {
    if (payload.records.length < WORKER_THRESHOLD) {
      return this.rebuildORMapInline(payload);
    }

    const task: WorkerTask<ORMapMerkleRebuildPayload, MerkleRebuildResult> = {
      id: generateTaskId(),
      type: 'merkle-rebuild-ormap' as WorkerTaskType,
      payload,
      priority: 'low',
    };

    return this.pool.submit(task);
  }

  // ============ Inline implementations for small batches ============

  private computeHashesInline(payload: MerkleHashPayload): MerkleHashResult {
    const { entries, depth = 3 } = payload;
    const hashes: Array<[string, number]> = [];
    const hashEntries: Array<{ key: string; hash: number }> = [];

    for (const entry of entries) {
      const itemHash = this.hashString(
        `${entry.key}:${entry.timestamp.millis}:${entry.timestamp.counter}:${entry.timestamp.nodeId}`
      );
      hashes.push([entry.key, itemHash]);
      hashEntries.push({ key: entry.key, hash: itemHash });
    }

    const { root, buckets } = this.buildTree(hashEntries, depth);

    return {
      hashes,
      rootHash: root.hash,
      buckets: Array.from(buckets.entries()),
    };
  }

  private computeORMapHashesInline(payload: ORMapMerkleHashPayload): ORMapMerkleHashResult {
    const { entries, depth = 3 } = payload;
    const hashes: Array<[string, number]> = [];
    const hashEntries: Array<{ key: string; hash: number }> = [];

    for (const entry of entries) {
      const sortedRecords = [...entry.records].sort((a, b) => a.tag.localeCompare(b.tag));
      let combinedStr = entry.key;
      for (const record of sortedRecords) {
        combinedStr += `:${record.tag}:${record.timestamp.millis}:${record.timestamp.counter}:${record.timestamp.nodeId}`;
      }
      const entryHash = this.hashString(combinedStr);
      hashes.push([entry.key, entryHash]);
      hashEntries.push({ key: entry.key, hash: entryHash });
    }

    const { root, buckets } = this.buildTree(hashEntries, depth);

    return {
      hashes,
      rootHash: root.hash,
      buckets: Array.from(buckets.entries()),
    };
  }

  private diffInline(payload: MerkleDiffPayload): MerkleDiffResult {
    const localMap = new Map<string, BucketInfo>(payload.localBuckets);
    const remoteMap = new Map<string, BucketInfo>(payload.remoteBuckets);

    const missingLocal: string[] = [];
    const missingRemote: string[] = [];
    const differingPaths: string[] = [];

    for (const [path, remoteBucket] of remoteMap) {
      const localBucket = localMap.get(path);

      if (!localBucket) {
        missingLocal.push(...remoteBucket.keys);
      } else if (localBucket.hash !== remoteBucket.hash) {
        differingPaths.push(path);

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

    for (const [path, localBucket] of localMap) {
      if (!remoteMap.has(path)) {
        missingRemote.push(...localBucket.keys);
      }
    }

    return { missingLocal, missingRemote, differingPaths };
  }

  private rebuildInline(payload: MerkleRebuildPayload): MerkleRebuildResult {
    const { records, depth = 3 } = payload;
    const hashEntries: Array<{ key: string; hash: number }> = [];

    for (const record of records) {
      const itemHash = this.hashString(
        `${record.key}:${record.timestamp.millis}:${record.timestamp.counter}:${record.timestamp.nodeId}`
      );
      hashEntries.push({ key: record.key, hash: itemHash });
    }

    const { root, buckets } = this.buildTree(hashEntries, depth);

    return {
      rootHash: root.hash,
      buckets: Array.from(buckets.entries()),
    };
  }

  private rebuildORMapInline(payload: ORMapMerkleRebuildPayload): MerkleRebuildResult {
    const { records, depth = 3 } = payload;
    const hashEntries: Array<{ key: string; hash: number }> = [];

    for (const record of records) {
      const sortedTags = [...record.tags].sort((a, b) => a.tag.localeCompare(b.tag));
      let combinedStr = record.key;
      for (const tag of sortedTags) {
        combinedStr += `:${tag.tag}:${tag.timestamp.millis}:${tag.timestamp.counter}:${tag.timestamp.nodeId}`;
      }
      const entryHash = this.hashString(combinedStr);
      hashEntries.push({ key: record.key, hash: entryHash });
    }

    const { root, buckets } = this.buildTree(hashEntries, depth);

    return {
      rootHash: root.hash,
      buckets: Array.from(buckets.entries()),
    };
  }

  // ============ Hash utilities ============

  private hashString(str: string): number {
    return coreHashString(str);
  }

  private buildTree(
    entries: Array<{ key: string; hash: number }>,
    depth: number
  ): { root: { hash: number }; buckets: Map<string, { hash: number; keys: string[] }> } {
    interface Node {
      hash: number;
      children?: { [key: string]: Node };
      entries?: Map<string, number>;
    }

    const root: Node = { hash: 0, children: {} };
    const buckets = new Map<string, { hash: number; keys: string[] }>();

    const updateNode = (
      node: Node,
      key: string,
      itemHash: number,
      pathHash: string,
      level: number
    ): number => {
      if (level >= depth) {
        if (!node.entries) node.entries = new Map();
        node.entries.set(key, itemHash);

        let h = 0;
        for (const val of node.entries.values()) {
          h = (h + val) | 0;
        }
        node.hash = h >>> 0;
        return node.hash;
      }

      const bucketChar = pathHash[level];
      if (!node.children) node.children = {};
      if (!node.children[bucketChar]) {
        node.children[bucketChar] = { hash: 0 };
      }

      updateNode(node.children[bucketChar], key, itemHash, pathHash, level + 1);

      let h = 0;
      for (const child of Object.values(node.children)) {
        h = (h + child.hash) | 0;
      }
      node.hash = h >>> 0;
      return node.hash;
    };

    for (const { key, hash } of entries) {
      const pathHash = this.hashString(key).toString(16).padStart(8, '0');
      updateNode(root, key, hash, pathHash, 0);
    }

    // Collect buckets
    const collectBuckets = (node: Node, path: string): void => {
      if (path.length >= depth) {
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
          collectBuckets(child, path + char);
        }
      }
    };

    collectBuckets(root, '');

    return { root, buckets };
  }
}
