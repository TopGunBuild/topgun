/**
 * IndexRegistry Tests
 */

import { IndexRegistry } from '../../query/IndexRegistry';
import { HashIndex } from '../../query/indexes/HashIndex';
import { NavigableIndex } from '../../query/indexes/NavigableIndex';
import { simpleAttribute } from '../../query/Attribute';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  status: string;
}

describe('IndexRegistry', () => {
  let registry: IndexRegistry<string, TestRecord>;
  let nameIndex: HashIndex<string, TestRecord, string>;
  let ageIndex: NavigableIndex<string, TestRecord, number>;
  let statusIndex: HashIndex<string, TestRecord, string>;

  beforeEach(() => {
    registry = new IndexRegistry<string, TestRecord>();

    const nameAttr = simpleAttribute<TestRecord, string>('name', (r) => r.name);
    const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
    const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);

    nameIndex = new HashIndex(nameAttr);
    ageIndex = new NavigableIndex(ageAttr);
    statusIndex = new HashIndex(statusAttr);
  });

  describe('addIndex', () => {
    it('should register an index for an attribute', () => {
      registry.addIndex(nameIndex);

      expect(registry.getIndexes('name')).toContain(nameIndex);
      expect(registry.size).toBe(1);
    });

    it('should allow multiple indexes for same attribute', () => {
      // Use age attribute since NavigableIndex requires string | number
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageHashIndex = new HashIndex<string, TestRecord, number>(ageAttr);
      const ageNavigableIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(ageHashIndex);
      registry.addIndex(ageNavigableIndex);

      const indexes = registry.getIndexes('age');
      expect(indexes).toHaveLength(2);
      expect(indexes).toContain(ageHashIndex);
      expect(indexes).toContain(ageNavigableIndex);
    });

    it('should not add duplicate index', () => {
      registry.addIndex(nameIndex);
      registry.addIndex(nameIndex);

      expect(registry.getIndexes('name')).toHaveLength(1);
    });

    it('should register indexes for different attributes', () => {
      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);
      registry.addIndex(statusIndex);

      expect(registry.size).toBe(3);
      expect(registry.getIndexedAttributes()).toEqual(
        expect.arrayContaining(['name', 'age', 'status'])
      );
    });
  });

  describe('removeIndex', () => {
    it('should remove a registered index', () => {
      registry.addIndex(nameIndex);
      const removed = registry.removeIndex(nameIndex);

      expect(removed).toBe(true);
      expect(registry.getIndexes('name')).toHaveLength(0);
      expect(registry.size).toBe(0);
    });

    it('should return false for non-existent index', () => {
      const removed = registry.removeIndex(nameIndex);

      expect(removed).toBe(false);
    });

    it('should keep other indexes for same attribute', () => {
      // Use age attribute since NavigableIndex requires string | number
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageHashIndex = new HashIndex<string, TestRecord, number>(ageAttr);
      const ageNavigableIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(ageHashIndex);
      registry.addIndex(ageNavigableIndex);
      registry.removeIndex(ageHashIndex);

      const indexes = registry.getIndexes('age');
      expect(indexes).toHaveLength(1);
      expect(indexes).toContain(ageNavigableIndex);
    });
  });

  describe('getIndexes', () => {
    it('should return empty array for unindexed attribute', () => {
      expect(registry.getIndexes('unknown')).toEqual([]);
    });

    it('should return all indexes for attribute', () => {
      registry.addIndex(nameIndex);

      expect(registry.getIndexes('name')).toEqual([nameIndex]);
    });
  });

  describe('getAllIndexes', () => {
    it('should return empty array when no indexes', () => {
      expect(registry.getAllIndexes()).toEqual([]);
    });

    it('should return all indexes across all attributes', () => {
      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);
      registry.addIndex(statusIndex);

      const all = registry.getAllIndexes();
      expect(all).toHaveLength(3);
      expect(all).toContain(nameIndex);
      expect(all).toContain(ageIndex);
      expect(all).toContain(statusIndex);
    });
  });

  describe('hasIndex', () => {
    it('should return false for unindexed attribute', () => {
      expect(registry.hasIndex('name')).toBe(false);
    });

    it('should return true for indexed attribute', () => {
      registry.addIndex(nameIndex);

      expect(registry.hasIndex('name')).toBe(true);
    });
  });

  describe('findBestIndex', () => {
    it('should return null when no index available', () => {
      expect(registry.findBestIndex('name', 'equal')).toBeNull();
    });

    it('should return null when no index supports query type', () => {
      registry.addIndex(nameIndex);

      // HashIndex doesn't support range queries
      expect(registry.findBestIndex('name', 'gt')).toBeNull();
    });

    it('should return index that supports query type', () => {
      registry.addIndex(nameIndex);

      expect(registry.findBestIndex('name', 'equal')).toBe(nameIndex);
    });

    it('should return lowest cost index when multiple available', () => {
      // Use age attribute since NavigableIndex requires string | number
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageHashIndex = new HashIndex<string, TestRecord, number>(ageAttr);
      const ageNavigableIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(ageNavigableIndex); // cost 40
      registry.addIndex(ageHashIndex); // cost 30

      // Both support 'equal', HashIndex has lower cost
      expect(registry.findBestIndex('age', 'equal')).toBe(ageHashIndex);
    });

    it('should find NavigableIndex for range queries', () => {
      registry.addIndex(ageIndex);

      expect(registry.findBestIndex('age', 'gt')).toBe(ageIndex);
      expect(registry.findBestIndex('age', 'gte')).toBe(ageIndex);
      expect(registry.findBestIndex('age', 'lt')).toBe(ageIndex);
      expect(registry.findBestIndex('age', 'lte')).toBe(ageIndex);
      expect(registry.findBestIndex('age', 'between')).toBe(ageIndex);
    });
  });

  describe('findIndexes', () => {
    it('should return empty array when no matching indexes', () => {
      expect(registry.findIndexes('name', 'equal')).toEqual([]);
    });

    it('should return matching indexes sorted by cost', () => {
      // Use age attribute since NavigableIndex requires string | number
      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageHashIndex = new HashIndex<string, TestRecord, number>(ageAttr);
      const ageNavigableIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);

      registry.addIndex(ageNavigableIndex); // cost 40
      registry.addIndex(ageHashIndex); // cost 30

      const indexes = registry.findIndexes('age', 'equal');
      expect(indexes).toHaveLength(2);
      expect(indexes[0]).toBe(ageHashIndex); // Lower cost first
      expect(indexes[1]).toBe(ageNavigableIndex);
    });
  });

  describe('index update notifications', () => {
    beforeEach(() => {
      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);
    });

    it('should notify all indexes on record added', () => {
      const record: TestRecord = { id: '1', name: 'Alice', age: 30, status: 'active' };

      registry.onRecordAdded('1', record);

      const nameResult = nameIndex.retrieve({ type: 'equal', value: 'Alice' });
      expect([...nameResult]).toContain('1');

      const ageResult = ageIndex.retrieve({ type: 'equal', value: 30 });
      expect([...ageResult]).toContain('1');
    });

    it('should notify all indexes on record updated', () => {
      const oldRecord: TestRecord = { id: '1', name: 'Alice', age: 30, status: 'active' };
      const newRecord: TestRecord = { id: '1', name: 'Bob', age: 35, status: 'active' };

      registry.onRecordAdded('1', oldRecord);
      registry.onRecordUpdated('1', oldRecord, newRecord);

      // Old values should be gone
      expect([...nameIndex.retrieve({ type: 'equal', value: 'Alice' })]).not.toContain('1');
      expect([...ageIndex.retrieve({ type: 'equal', value: 30 })]).not.toContain('1');

      // New values should be present
      expect([...nameIndex.retrieve({ type: 'equal', value: 'Bob' })]).toContain('1');
      expect([...ageIndex.retrieve({ type: 'equal', value: 35 })]).toContain('1');
    });

    it('should notify all indexes on record removed', () => {
      const record: TestRecord = { id: '1', name: 'Alice', age: 30, status: 'active' };

      registry.onRecordAdded('1', record);
      registry.onRecordRemoved('1', record);

      expect([...nameIndex.retrieve({ type: 'equal', value: 'Alice' })]).not.toContain('1');
      expect([...ageIndex.retrieve({ type: 'equal', value: 30 })]).not.toContain('1');
    });
  });

  describe('clear', () => {
    it('should clear all indexes', () => {
      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const record: TestRecord = { id: '1', name: 'Alice', age: 30, status: 'active' };
      registry.onRecordAdded('1', record);

      registry.clear();

      expect([...nameIndex.retrieve({ type: 'equal', value: 'Alice' })]).toHaveLength(0);
      expect([...ageIndex.retrieve({ type: 'equal', value: 30 })]).toHaveLength(0);

      // Registry itself should still have indexes registered
      expect(registry.size).toBe(2);
    });
  });

  describe('getStats', () => {
    it('should return stats for empty registry', () => {
      const stats = registry.getStats();

      expect(stats.totalIndexes).toBe(0);
      expect(stats.indexedAttributes).toBe(0);
      expect(stats.indexes).toEqual([]);
    });

    it('should return stats for registry with indexes', () => {
      registry.addIndex(nameIndex);
      registry.addIndex(ageIndex);

      const record: TestRecord = { id: '1', name: 'Alice', age: 30, status: 'active' };
      registry.onRecordAdded('1', record);

      const stats = registry.getStats();

      expect(stats.totalIndexes).toBe(2);
      expect(stats.indexedAttributes).toBe(2);
      expect(stats.indexes).toHaveLength(2);

      const nameStats = stats.indexes.find((i) => i.attribute === 'name');
      expect(nameStats).toBeDefined();
      expect(nameStats!.type).toBe('hash');
      expect(nameStats!.stats.totalEntries).toBe(1);
    });
  });

  describe('fallback index', () => {
    it('should return null when no fallback set', () => {
      expect(registry.getFallbackIndex()).toBeNull();
    });

    it('should store and return fallback index', () => {
      const fallback = nameIndex; // Using as placeholder
      registry.setFallbackIndex(fallback);

      expect(registry.getFallbackIndex()).toBe(fallback);
    });
  });
});
