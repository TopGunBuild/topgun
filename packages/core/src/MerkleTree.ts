import { LWWRecord } from './LWWMap';
import { hashString } from './utils/hash';

export interface MerkleNode {
  hash: number;
  children?: { [key: string]: MerkleNode }; // Keyed by bucket index (hex char)
  entries?: Map<string, number>; // Leaf node: Key -> ContentHash
}

/**
 * A specific implementation of Merkle Tree for syncing LWW-Maps.
 * It uses a Prefix Trie structure based on the hash of the Record Key.
 * 
 * Structure:
 * - Level 0: Root
 * - Level 1..N: Buckets based on hex digits of Key Hash.
 * 
 * This allows us to quickly identify which "bucket" of keys is out of sync.
 */
export class MerkleTree {
  private root: MerkleNode;
  private readonly depth: number;

  constructor(records: Map<string, LWWRecord<any>> = new Map(), depth: number = 3) {
    this.depth = depth;
    this.root = { hash: 0, children: {} };
    // Build initial tree
    for (const [key, record] of records) {
        this.update(key, record);
    }
  }

  /**
   * Incrementally updates the Merkle Tree with a single record.
   * @param key The key of the record
   * @param record The record (value + timestamp)
   */
  public update(key: string, record: LWWRecord<any>) {
    const itemHash = hashString(`${key}:${record.timestamp.millis}:${record.timestamp.counter}:${record.timestamp.nodeId}`);
    // We use the hash of the KEY for routing, so the record stays in the same bucket
    // regardless of timestamp changes.
    const pathHash = hashString(key).toString(16).padStart(8, '0'); 
    
    this.updateNode(this.root, key, itemHash, pathHash, 0);
  }

  /**
   * Removes a key from the Merkle Tree.
   * Necessary for Garbage Collection of tombstones.
   */
  public remove(key: string) {
    const pathHash = hashString(key).toString(16).padStart(8, '0');
    this.removeNode(this.root, key, pathHash, 0);
  }

  private removeNode(node: MerkleNode, key: string, pathHash: string, level: number): number {
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
      const childHash = this.removeNode(node.children[bucketChar], key, pathHash, level + 1);
      
      // Optimization: if child is empty/zero, we might want to remove it, but for now just recalc.
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

  private updateNode(node: MerkleNode, key: string, itemHash: number, pathHash: string, level: number): number {
    // Leaf Node Logic
    if (level >= this.depth) {
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
    
    this.updateNode(node.children[bucketChar], key, itemHash, pathHash, level + 1);
    
    // Recalculate this node's hash from children
    let h = 0;
    for (const child of Object.values(node.children)) {
      h = (h + child.hash) | 0;
    }
    node.hash = h >>> 0;
    return node.hash;
  }

  public getRootHash(): number {
    return this.root.hash;
  }

  public getNode(path: string): MerkleNode | undefined {
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
  public getBuckets(path: string): Record<string, number> {
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
  public getKeysInBucket(path: string): string[] {
    const node = this.getNode(path);
    if (!node || !node.entries) return [];
    return Array.from(node.entries.keys());
  }
}
