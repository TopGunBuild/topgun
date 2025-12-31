/**
 * MerkleTreeManager - Per-Partition Merkle Tree Management
 *
 * Phase 10.04: Manages Merkle trees for each partition to enable:
 * - Efficient delta sync between nodes
 * - Anti-entropy repair detection
 * - Incremental updates on writes
 *
 * Each partition maintains its own Merkle tree for independent
 * consistency checking and repair.
 */

import { EventEmitter } from 'events';
import { MerkleTree, LWWRecord, hashString, PARTITION_COUNT } from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface MerkleTreeManagerConfig {
  /** Tree depth for Merkle trees. Default: 3 */
  treeDepth: number;
  /** Enable automatic tree updates on write. Default: true */
  autoUpdate: boolean;
  /** Lazy initialization of trees. Default: true */
  lazyInit: boolean;
}

export const DEFAULT_MERKLE_TREE_CONFIG: MerkleTreeManagerConfig = {
  treeDepth: 3,
  autoUpdate: true,
  lazyInit: true,
};

export interface MerkleComparisonResult {
  partitionId: number;
  localRoot: number;
  remoteRoot: number;
  needsSync: boolean;
  differingBuckets: string[];
}

export interface PartitionMerkleInfo {
  partitionId: number;
  rootHash: number;
  keyCount: number;
  lastUpdated: number;
}

export class MerkleTreeManager extends EventEmitter {
  private config: MerkleTreeManagerConfig;
  private trees: Map<number, MerkleTree> = new Map();
  private keyCounts: Map<number, number> = new Map();
  private lastUpdated: Map<number, number> = new Map();
  private nodeId: string;

  constructor(nodeId: string, config: Partial<MerkleTreeManagerConfig> = {}) {
    super();
    this.nodeId = nodeId;
    this.config = { ...DEFAULT_MERKLE_TREE_CONFIG, ...config };
  }

  /**
   * Get or create a Merkle tree for a partition
   */
  getTree(partitionId: number): MerkleTree {
    let tree = this.trees.get(partitionId);
    if (!tree) {
      tree = new MerkleTree(new Map(), this.config.treeDepth);
      this.trees.set(partitionId, tree);
      this.keyCounts.set(partitionId, 0);
      this.lastUpdated.set(partitionId, Date.now());
    }
    return tree;
  }

  /**
   * Build tree for a partition from existing data
   */
  buildTree(partitionId: number, records: Map<string, LWWRecord<any>>): void {
    const tree = new MerkleTree(records, this.config.treeDepth);
    this.trees.set(partitionId, tree);
    this.keyCounts.set(partitionId, records.size);
    this.lastUpdated.set(partitionId, Date.now());

    logger.debug({
      partitionId,
      keyCount: records.size,
      rootHash: tree.getRootHash()
    }, 'Built Merkle tree for partition');
  }

  /**
   * Incrementally update tree when a record changes
   */
  updateRecord(partitionId: number, key: string, record: LWWRecord<any>): void {
    if (!this.config.autoUpdate) return;

    const tree = this.getTree(partitionId);
    const previousKeyCount = this.keyCounts.get(partitionId) ?? 0;

    // Check if this is a new key (not update)
    const existingBuckets = tree.getBuckets('');
    const wasNewKey = Object.keys(existingBuckets).length === 0 ||
      !tree.getKeysInBucket(this.getKeyPath(key)).includes(key);

    tree.update(key, record);

    if (wasNewKey) {
      this.keyCounts.set(partitionId, previousKeyCount + 1);
    }

    this.lastUpdated.set(partitionId, Date.now());

    this.emit('treeUpdated', {
      partitionId,
      key,
      rootHash: tree.getRootHash()
    });
  }

  /**
   * Remove a key from the tree (e.g., after GC)
   */
  removeRecord(partitionId: number, key: string): void {
    const tree = this.trees.get(partitionId);
    if (!tree) return;

    tree.remove(key);

    const currentCount = this.keyCounts.get(partitionId) ?? 0;
    if (currentCount > 0) {
      this.keyCounts.set(partitionId, currentCount - 1);
    }

    this.lastUpdated.set(partitionId, Date.now());

    this.emit('treeUpdated', {
      partitionId,
      key,
      rootHash: tree.getRootHash()
    });
  }

  /**
   * Get the path prefix for a key in the Merkle tree
   */
  private getKeyPath(key: string): string {
    const hash = hashString(key).toString(16).padStart(8, '0');
    return hash.slice(0, this.config.treeDepth);
  }

  /**
   * Get root hash for a partition
   */
  getRootHash(partitionId: number): number {
    const tree = this.trees.get(partitionId);
    return tree?.getRootHash() ?? 0;
  }

  /**
   * Compare local tree with remote root hash
   */
  compareWithRemote(partitionId: number, remoteRoot: number): MerkleComparisonResult {
    const tree = this.getTree(partitionId);
    const localRoot = tree.getRootHash();

    return {
      partitionId,
      localRoot,
      remoteRoot,
      needsSync: localRoot !== remoteRoot,
      differingBuckets: localRoot !== remoteRoot ? this.findDifferingBuckets(tree, remoteRoot) : [],
    };
  }

  /**
   * Find buckets that differ between local and remote tree
   * Note: This is a simplified version - full implementation would
   * need to exchange bucket hashes with the remote node
   */
  private findDifferingBuckets(tree: MerkleTree, _remoteRoot: number): string[] {
    // For now, return all leaf buckets as candidates
    // In full implementation, this would be done via network exchange
    const buckets: string[] = [];
    this.collectLeafBuckets(tree, '', buckets);
    return buckets;
  }

  /**
   * Recursively collect all leaf bucket paths
   */
  private collectLeafBuckets(tree: MerkleTree, path: string, result: string[]): void {
    if (path.length >= this.config.treeDepth) {
      const keys = tree.getKeysInBucket(path);
      if (keys.length > 0) {
        result.push(path);
      }
      return;
    }

    const buckets = tree.getBuckets(path);
    for (const char of Object.keys(buckets)) {
      this.collectLeafBuckets(tree, path + char, result);
    }
  }

  /**
   * Get bucket hashes for a partition at a given path
   */
  getBuckets(partitionId: number, path: string): Record<string, number> {
    const tree = this.trees.get(partitionId);
    return tree?.getBuckets(path) ?? {};
  }

  /**
   * Get keys in a specific bucket
   */
  getKeysInBucket(partitionId: number, path: string): string[] {
    const tree = this.trees.get(partitionId);
    return tree?.getKeysInBucket(path) ?? [];
  }

  /**
   * Get all keys across all buckets for a partition
   */
  getAllKeys(partitionId: number): string[] {
    const tree = this.trees.get(partitionId);
    if (!tree) return [];

    const keys: string[] = [];
    this.collectAllKeys(tree, '', keys);
    return keys;
  }

  /**
   * Recursively collect all keys from the tree
   */
  private collectAllKeys(tree: MerkleTree, path: string, result: string[]): void {
    if (path.length >= this.config.treeDepth) {
      const keys = tree.getKeysInBucket(path);
      result.push(...keys);
      return;
    }

    const buckets = tree.getBuckets(path);
    for (const char of Object.keys(buckets)) {
      this.collectAllKeys(tree, path + char, result);
    }
  }

  /**
   * Get info about all managed partitions
   */
  getPartitionInfos(): PartitionMerkleInfo[] {
    const infos: PartitionMerkleInfo[] = [];

    for (const [partitionId, tree] of this.trees) {
      infos.push({
        partitionId,
        rootHash: tree.getRootHash(),
        keyCount: this.keyCounts.get(partitionId) ?? 0,
        lastUpdated: this.lastUpdated.get(partitionId) ?? 0,
      });
    }

    return infos;
  }

  /**
   * Get info for a specific partition
   */
  getPartitionInfo(partitionId: number): PartitionMerkleInfo | null {
    const tree = this.trees.get(partitionId);
    if (!tree) return null;

    return {
      partitionId,
      rootHash: tree.getRootHash(),
      keyCount: this.keyCounts.get(partitionId) ?? 0,
      lastUpdated: this.lastUpdated.get(partitionId) ?? 0,
    };
  }

  /**
   * Clear tree for a partition (e.g., after migration)
   */
  clearPartition(partitionId: number): void {
    this.trees.delete(partitionId);
    this.keyCounts.delete(partitionId);
    this.lastUpdated.delete(partitionId);
  }

  /**
   * Clear all trees
   */
  clearAll(): void {
    this.trees.clear();
    this.keyCounts.clear();
    this.lastUpdated.clear();
  }

  /**
   * Get metrics for monitoring
   */
  getMetrics(): {
    totalPartitions: number;
    totalKeys: number;
    averageKeysPerPartition: number;
  } {
    let totalKeys = 0;
    for (const count of this.keyCounts.values()) {
      totalKeys += count;
    }

    return {
      totalPartitions: this.trees.size,
      totalKeys,
      averageKeysPerPartition: this.trees.size > 0 ? totalKeys / this.trees.size : 0,
    };
  }

  /**
   * Serialize tree state for network transfer
   */
  serializeTree(partitionId: number): {
    rootHash: number;
    buckets: Record<string, Record<string, number>>;
  } | null {
    const tree = this.trees.get(partitionId);
    if (!tree) return null;

    const buckets: Record<string, Record<string, number>> = {};

    // Collect buckets at each level
    for (let depth = 0; depth < this.config.treeDepth; depth++) {
      this.collectBucketsAtDepth(tree, '', depth, buckets);
    }

    return {
      rootHash: tree.getRootHash(),
      buckets,
    };
  }

  private collectBucketsAtDepth(
    tree: MerkleTree,
    path: string,
    targetDepth: number,
    result: Record<string, Record<string, number>>
  ): void {
    if (path.length === targetDepth) {
      const buckets = tree.getBuckets(path);
      if (Object.keys(buckets).length > 0) {
        result[path] = buckets;
      }
      return;
    }

    if (path.length > targetDepth) return;

    const buckets = tree.getBuckets(path);
    for (const char of Object.keys(buckets)) {
      this.collectBucketsAtDepth(tree, path + char, targetDepth, result);
    }
  }
}
