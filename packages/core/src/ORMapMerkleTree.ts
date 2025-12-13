import { ORMap, ORMapRecord } from './ORMap';
import { hashString, combineHashes } from './utils/hash';
import { hashORMapEntry } from './ORMapMerkle';

/**
 * Merkle Node for ORMap.
 * Uses a prefix trie structure based on key hash (similar to LWWMap MerkleTree).
 */
export interface ORMapMerkleNode {
  hash: number;
  children?: { [key: string]: ORMapMerkleNode }; // Keyed by bucket index (hex char)
  entries?: Map<string, number>; // Leaf node: Key -> ContentHash
}

/**
 * A Merkle Tree implementation specifically for ORMap synchronization.
 * Uses a Prefix Trie structure based on the hash of the Record Key.
 *
 * Structure:
 * - Level 0: Root
 * - Level 1..N: Buckets based on hex digits of Key Hash.
 *
 * Key difference from LWWMap MerkleTree:
 * - Each key can have multiple records (tags), so the entry hash includes all records for that key.
 */
export class ORMapMerkleTree {
  private root: ORMapMerkleNode;
  private readonly depth: number;

  constructor(depth: number = 3) {
    this.depth = depth;
    this.root = { hash: 0, children: {} };
  }

  /**
   * Update tree from ORMap data.
   * Rebuilds hashes for all entries in the map.
   */
  updateFromORMap<K, V>(map: ORMap<K, V>): void {
    // Clear and rebuild
    this.root = { hash: 0, children: {} };

    // Access internal items through available methods
    // We need to iterate over all keys and get their records
    const snapshot = map.getSnapshot();

    for (const [key, records] of snapshot.items) {
      if (records.size > 0) {
        const keyStr = String(key);
        const entryHash = hashORMapEntry(keyStr, records);
        const pathHash = hashString(keyStr).toString(16).padStart(8, '0');
        this.updateNode(this.root, keyStr, entryHash, pathHash, 0);
      }
    }
  }

  /**
   * Incrementally update a single key's hash.
   * Call this when records for a key change.
   */
  update<V>(key: string, records: Map<string, ORMapRecord<V>>): void {
    const pathHash = hashString(key).toString(16).padStart(8, '0');

    if (records.size === 0) {
      // Key has no records, remove from tree
      this.removeNode(this.root, key, pathHash, 0);
    } else {
      const entryHash = hashORMapEntry(key, records);
      this.updateNode(this.root, key, entryHash, pathHash, 0);
    }
  }

  /**
   * Remove a key from the tree.
   * Called when all records for a key are removed.
   */
  remove(key: string): void {
    const pathHash = hashString(key).toString(16).padStart(8, '0');
    this.removeNode(this.root, key, pathHash, 0);
  }

  private updateNode(
    node: ORMapMerkleNode,
    key: string,
    entryHash: number,
    pathHash: string,
    level: number
  ): number {
    // Leaf Node Logic
    if (level >= this.depth) {
      if (!node.entries) node.entries = new Map();
      node.entries.set(key, entryHash);

      // Recalculate leaf hash (Sum of entry hashes)
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

    this.updateNode(node.children[bucketChar], key, entryHash, pathHash, level + 1);

    // Recalculate this node's hash from children
    let h = 0;
    for (const child of Object.values(node.children)) {
      h = (h + child.hash) | 0;
    }
    node.hash = h >>> 0;
    return node.hash;
  }

  private removeNode(
    node: ORMapMerkleNode,
    key: string,
    pathHash: string,
    level: number
  ): number {
    // Leaf Node Logic
    if (level >= this.depth) {
      if (node.entries) {
        node.entries.delete(key);

        // Recalculate leaf hash
        let h = 0;
        for (const val of node.entries.values()) {
          h = (h + val) | 0;
        }
        node.hash = h >>> 0;
      }
      return node.hash;
    }

    // Intermediate Node Logic
    const bucketChar = pathHash[level];
    if (node.children && node.children[bucketChar]) {
      this.removeNode(node.children[bucketChar], key, pathHash, level + 1);
    }

    // Recalculate this node's hash from children
    let h = 0;
    if (node.children) {
      for (const child of Object.values(node.children)) {
        h = (h + child.hash) | 0;
      }
    }
    node.hash = h >>> 0;
    return node.hash;
  }

  /**
   * Get the root hash for quick comparison.
   */
  getRootHash(): number {
    return this.root.hash;
  }

  /**
   * Get node at a specific path.
   */
  getNode(path: string): ORMapMerkleNode | undefined {
    let current = this.root;
    for (const char of path) {
      if (!current.children || !current.children[char]) {
        return undefined;
      }
      current = current.children[char];
    }
    return current;
  }

  /**
   * Returns the hashes of the children at the given path.
   * Used by the client/server to compare buckets.
   */
  getBuckets(path: string): Record<string, number> {
    const node = this.getNode(path);
    if (!node || !node.children) return {};

    const result: Record<string, number> = {};
    for (const [key, child] of Object.entries(node.children)) {
      result[key] = child.hash;
    }
    return result;
  }

  /**
   * For a leaf node (bucket), returns the actual keys it contains.
   * Used to request specific keys when a bucket differs.
   */
  getKeysInBucket(path: string): string[] {
    const node = this.getNode(path);
    if (!node || !node.entries) return [];
    return Array.from(node.entries.keys());
  }

  /**
   * Find keys that differ between this tree and bucket info from remote.
   * Returns keys that:
   * - Exist locally but have different hash on remote
   * - Exist on remote but not locally
   * - Exist locally but not on remote
   */
  findDiffKeys(path: string, remoteEntries: Map<string, number>): Set<string> {
    const diffKeys = new Set<string>();
    const node = this.getNode(path);
    const localEntries = node?.entries || new Map();

    // Keys in local but not remote, or different hash
    for (const [key, hash] of localEntries) {
      const remoteHash = remoteEntries.get(key);
      if (remoteHash === undefined || remoteHash !== hash) {
        diffKeys.add(key);
      }
    }

    // Keys in remote but not local
    for (const key of remoteEntries.keys()) {
      if (!localEntries.has(key)) {
        diffKeys.add(key);
      }
    }

    return diffKeys;
  }

  /**
   * Get all entry hashes at a leaf path.
   * Used when sending bucket details to remote.
   */
  getEntryHashes(path: string): Map<string, number> {
    const node = this.getNode(path);
    return node?.entries || new Map();
  }

  /**
   * Check if a path leads to a leaf node.
   */
  isLeaf(path: string): boolean {
    const node = this.getNode(path);
    return node !== undefined && node.entries !== undefined && node.entries.size > 0;
  }
}
