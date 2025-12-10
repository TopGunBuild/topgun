import { MerkleTree } from '../MerkleTree';
import { LWWRecord } from '../LWWMap';

// Helper to create a dummy record
const createRecord = (val: string, millis: number): LWWRecord<string> => ({
  value: val,
  timestamp: { millis, counter: 0, nodeId: 'test' }
});

describe('MerkleTree', () => {
  test('should produce same root hash for same data', () => {
    const data1 = new Map<string, LWWRecord<string>>();
    data1.set('a', createRecord('v1', 100));
    data1.set('b', createRecord('v2', 200));

    const data2 = new Map<string, LWWRecord<string>>();
    data2.set('b', createRecord('v2', 200));
    data2.set('a', createRecord('v1', 100));

    const tree1 = new MerkleTree(data1);
    const tree2 = new MerkleTree(data2);

    expect(tree1.getRootHash()).toBe(tree2.getRootHash());
    expect(tree1.getRootHash()).not.toBe(0);
  });

  test('should have different root hash if data differs', () => {
    const data1 = new Map<string, LWWRecord<string>>();
    data1.set('a', createRecord('v1', 100));

    const data2 = new Map<string, LWWRecord<string>>();
    data2.set('a', createRecord('v1', 101)); // Changed timestamp

    const tree1 = new MerkleTree(data1);
    const tree2 = new MerkleTree(data2);

    expect(tree1.getRootHash()).not.toBe(tree2.getRootHash());
  });

  test('should identify changed buckets', () => {
    // Create a tree with sufficient depth
    const depth = 3;
    const data = new Map<string, LWWRecord<string>>();
    
    // Add enough items to likely populate different buckets
    data.set('key1', createRecord('val1', 100));
    data.set('key2', createRecord('val2', 100));

    const tree = new MerkleTree(data, depth);
    const rootHash = tree.getRootHash();
    
    // Check level 1 buckets
    const buckets = tree.getBuckets(""); // Root children
    expect(Object.keys(buckets).length).toBeGreaterThan(0);
  });

  test('should handle empty maps', () => {
    const data = new Map<string, LWWRecord<string>>();
    const tree = new MerkleTree(data);
    expect(tree.getRootHash()).toBe(0);
  });
});

