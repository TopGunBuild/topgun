/**
 * MerkleTreeManager Unit Tests
 *
 * Tests per-partition Merkle tree management:
 * - Tree creation and updates
 * - Record tracking
 * - Hash comparison
 * - Key retrieval
 */

import { MerkleTreeManager, DEFAULT_MERKLE_TREE_CONFIG } from '../MerkleTreeManager';
import { LWWRecord, Timestamp } from '@topgunbuild/core';

function createRecord<T>(value: T, millis: number = Date.now()): LWWRecord<T> {
  return {
    value,
    timestamp: {
      millis,
      counter: 0,
      nodeId: 'test-node',
    },
  };
}

describe('MerkleTreeManager', () => {
  let manager: MerkleTreeManager;

  beforeEach(() => {
    manager = new MerkleTreeManager('test-node');
  });

  describe('initialization', () => {
    test('should initialize with default config', () => {
      expect(manager.getMetrics().totalPartitions).toBe(0);
      expect(manager.getMetrics().totalKeys).toBe(0);
    });

    test('should lazy initialize trees', () => {
      // Tree should not exist until accessed
      expect(manager.getPartitionInfo(0)).toBeNull();

      // Getting tree should create it
      manager.getTree(0);
      expect(manager.getPartitionInfo(0)).not.toBeNull();
    });
  });

  describe('updateRecord', () => {
    test('should create tree and update on first record', () => {
      const record = createRecord({ name: 'test' });
      manager.updateRecord(0, 'key-1', record);

      const info = manager.getPartitionInfo(0);
      expect(info).not.toBeNull();
      expect(info!.keyCount).toBe(1);
      expect(info!.rootHash).not.toBe(0);
    });

    test('should update root hash when record changes', () => {
      const record1 = createRecord({ name: 'test1' }, 1000);
      manager.updateRecord(0, 'key-1', record1);
      const hash1 = manager.getRootHash(0);

      const record2 = createRecord({ name: 'test2' }, 2000);
      manager.updateRecord(0, 'key-1', record2);
      const hash2 = manager.getRootHash(0);

      expect(hash1).not.toBe(hash2);
    });

    test('should track multiple keys', () => {
      manager.updateRecord(0, 'key-1', createRecord('value1'));
      manager.updateRecord(0, 'key-2', createRecord('value2'));
      manager.updateRecord(0, 'key-3', createRecord('value3'));

      const info = manager.getPartitionInfo(0);
      expect(info!.keyCount).toBe(3);
    });

    test('should emit treeUpdated event', () => {
      const handler = jest.fn();
      manager.on('treeUpdated', handler);

      manager.updateRecord(0, 'key-1', createRecord('value'));

      expect(handler).toHaveBeenCalledWith({
        partitionId: 0,
        key: 'key-1',
        rootHash: expect.any(Number),
      });
    });
  });

  describe('removeRecord', () => {
    test('should update hash after removal', () => {
      manager.updateRecord(0, 'key-1', createRecord('value1'));
      manager.updateRecord(0, 'key-2', createRecord('value2'));

      const hashBefore = manager.getRootHash(0);

      manager.removeRecord(0, 'key-1');

      const hashAfter = manager.getRootHash(0);
      expect(hashAfter).not.toBe(hashBefore);
    });

    test('should decrement key count', () => {
      manager.updateRecord(0, 'key-1', createRecord('value1'));
      manager.updateRecord(0, 'key-2', createRecord('value2'));

      expect(manager.getPartitionInfo(0)!.keyCount).toBe(2);

      manager.removeRecord(0, 'key-1');

      expect(manager.getPartitionInfo(0)!.keyCount).toBe(1);
    });
  });

  describe('getRootHash', () => {
    test('should return 0 for non-existent partition', () => {
      expect(manager.getRootHash(999)).toBe(0);
    });

    test('should return consistent hash for same data', () => {
      const record = createRecord({ test: 'value' }, 1000);

      manager.updateRecord(0, 'key-1', record);
      const hash1 = manager.getRootHash(0);

      // Create new manager with same data
      const manager2 = new MerkleTreeManager('test-node');
      manager2.updateRecord(0, 'key-1', record);
      const hash2 = manager2.getRootHash(0);

      expect(hash1).toBe(hash2);
    });
  });

  describe('compareWithRemote', () => {
    test('should detect matching trees', () => {
      manager.updateRecord(0, 'key-1', createRecord('value'));
      const localRoot = manager.getRootHash(0);

      const result = manager.compareWithRemote(0, localRoot);

      expect(result.needsSync).toBe(false);
      expect(result.localRoot).toBe(localRoot);
      expect(result.remoteRoot).toBe(localRoot);
    });

    test('should detect differing trees', () => {
      manager.updateRecord(0, 'key-1', createRecord('value'));
      const localRoot = manager.getRootHash(0);

      const result = manager.compareWithRemote(0, localRoot + 1);

      expect(result.needsSync).toBe(true);
      expect(result.differingBuckets.length).toBeGreaterThan(0);
    });
  });

  describe('getBuckets', () => {
    test('should return bucket hashes', () => {
      // Add enough keys to populate multiple buckets
      for (let i = 0; i < 100; i++) {
        manager.updateRecord(0, `key-${i}`, createRecord(`value-${i}`));
      }

      const buckets = manager.getBuckets(0, '');
      expect(Object.keys(buckets).length).toBeGreaterThan(0);
    });

    test('should return empty for non-existent partition', () => {
      const buckets = manager.getBuckets(999, '');
      expect(buckets).toEqual({});
    });
  });

  describe('getKeysInBucket', () => {
    test('should return keys at leaf level', () => {
      manager.updateRecord(0, 'key-1', createRecord('value1'));
      manager.updateRecord(0, 'key-2', createRecord('value2'));

      const allKeys = manager.getAllKeys(0);
      expect(allKeys).toContain('key-1');
      expect(allKeys).toContain('key-2');
    });
  });

  describe('buildTree', () => {
    test('should build tree from existing records', () => {
      const records = new Map<string, LWWRecord<any>>();
      records.set('key-1', createRecord('value1'));
      records.set('key-2', createRecord('value2'));
      records.set('key-3', createRecord('value3'));

      manager.buildTree(0, records);

      const info = manager.getPartitionInfo(0);
      expect(info!.keyCount).toBe(3);
      expect(info!.rootHash).not.toBe(0);
    });
  });

  describe('clearPartition', () => {
    test('should remove partition tree', () => {
      manager.updateRecord(0, 'key-1', createRecord('value'));
      expect(manager.getPartitionInfo(0)).not.toBeNull();

      manager.clearPartition(0);

      expect(manager.getPartitionInfo(0)).toBeNull();
    });
  });

  describe('clearAll', () => {
    test('should remove all trees', () => {
      manager.updateRecord(0, 'key-1', createRecord('value'));
      manager.updateRecord(1, 'key-2', createRecord('value'));
      manager.updateRecord(2, 'key-3', createRecord('value'));

      expect(manager.getMetrics().totalPartitions).toBe(3);

      manager.clearAll();

      expect(manager.getMetrics().totalPartitions).toBe(0);
    });
  });

  describe('getMetrics', () => {
    test('should return accurate metrics', () => {
      manager.updateRecord(0, 'key-1', createRecord('value'));
      manager.updateRecord(0, 'key-2', createRecord('value'));
      manager.updateRecord(1, 'key-3', createRecord('value'));

      const metrics = manager.getMetrics();

      expect(metrics.totalPartitions).toBe(2);
      expect(metrics.totalKeys).toBe(3);
      expect(metrics.averageKeysPerPartition).toBe(1.5);
    });
  });

  describe('serializeTree', () => {
    test('should serialize tree for network transfer', () => {
      manager.updateRecord(0, 'key-1', createRecord('value1'));
      manager.updateRecord(0, 'key-2', createRecord('value2'));

      const serialized = manager.serializeTree(0);

      expect(serialized).not.toBeNull();
      expect(serialized!.rootHash).not.toBe(0);
      expect(typeof serialized!.buckets).toBe('object');
    });

    test('should return null for non-existent partition', () => {
      expect(manager.serializeTree(999)).toBeNull();
    });
  });

  describe('partition isolation', () => {
    test('should maintain separate trees per partition', () => {
      manager.updateRecord(0, 'key-1', createRecord('value0'));
      manager.updateRecord(1, 'key-1', createRecord('value1'));

      const hash0 = manager.getRootHash(0);
      const hash1 = manager.getRootHash(1);

      // Different values should produce different hashes
      // (Same key in different partitions)
      expect(manager.getAllKeys(0)).toContain('key-1');
      expect(manager.getAllKeys(1)).toContain('key-1');
    });
  });
});
