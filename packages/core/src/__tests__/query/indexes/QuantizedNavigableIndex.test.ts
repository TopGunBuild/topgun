import {
  QuantizedNavigableIndex,
  Quantizers,
} from '../../../query/indexes/QuantizedNavigableIndex';
import { simpleAttribute } from '../../../query/Attribute';

interface TimestampedEvent {
  id: string;
  timestamp: number;
  value: number;
}

const timestampAttr = simpleAttribute<TimestampedEvent, number>(
  'timestamp',
  (e) => e.timestamp
);
const valueAttr = simpleAttribute<TimestampedEvent, number>(
  'value',
  (e) => e.value
);

describe('QuantizedNavigableIndex', () => {
  describe('Quantizers', () => {
    describe('integerMultiple', () => {
      it('should quantize to nearest multiple', () => {
        const q = Quantizers.integerMultiple(10);
        expect(q.quantize(5)).toBe(0);
        expect(q.quantize(10)).toBe(10);
        expect(q.quantize(15)).toBe(10);
        expect(q.quantize(23)).toBe(20);
        expect(q.quantize(99)).toBe(90);
        expect(q.quantize(100)).toBe(100);
      });

      it('should handle large multiples', () => {
        const q = Quantizers.integerMultiple(1000);
        expect(q.quantize(500)).toBe(0);
        expect(q.quantize(1500)).toBe(1000);
        expect(q.quantize(9999)).toBe(9000);
      });
    });

    describe('timestampInterval', () => {
      it('should quantize timestamps to intervals', () => {
        const q = Quantizers.timestampInterval(60000); // 1 minute
        const baseTime = 1704067200000; // Some timestamp

        expect(q.quantize(baseTime)).toBe(baseTime);
        expect(q.quantize(baseTime + 30000)).toBe(baseTime); // 30 sec later -> same minute
        expect(q.quantize(baseTime + 60000)).toBe(baseTime + 60000); // Next minute
        expect(q.quantize(baseTime + 90000)).toBe(baseTime + 60000);
      });

      it('should quantize to hour boundaries', () => {
        const q = Quantizers.timestampInterval(3600000); // 1 hour
        const baseTime = 1704067200000;

        expect(q.quantize(baseTime + 1800000)).toBe(baseTime); // 30 min later
        expect(q.quantize(baseTime + 3600000)).toBe(baseTime + 3600000); // 1 hour later
      });
    });

    describe('powerOf10', () => {
      it('should quantize to power of 10', () => {
        const q = Quantizers.powerOf10();
        expect(q.quantize(1)).toBe(1);
        expect(q.quantize(5)).toBe(1);
        expect(q.quantize(10)).toBe(10);
        expect(q.quantize(50)).toBe(10);
        expect(q.quantize(100)).toBe(100);
        expect(q.quantize(999)).toBe(100);
        expect(q.quantize(1000)).toBe(1000);
      });

      it('should handle edge cases', () => {
        const q = Quantizers.powerOf10();
        expect(q.quantize(0)).toBe(0);
        expect(q.quantize(-5)).toBe(0); // Negative numbers
      });
    });

    describe('logarithmic', () => {
      it('should quantize to powers of base', () => {
        const q = Quantizers.logarithmic(2);
        expect(q.quantize(1)).toBe(1);
        expect(q.quantize(2)).toBe(2);
        expect(q.quantize(3)).toBe(2);
        expect(q.quantize(4)).toBe(4);
        expect(q.quantize(7)).toBe(4);
        expect(q.quantize(8)).toBe(8);
      });

      it('should work with different bases', () => {
        const q = Quantizers.logarithmic(10);
        expect(q.quantize(1)).toBe(1);
        expect(q.quantize(9)).toBe(1);
        expect(q.quantize(10)).toBe(10);
        expect(q.quantize(99)).toBe(10);
        expect(q.quantize(100)).toBe(100);
      });
    });
  });

  describe('basic properties', () => {
    it('should have type navigable', () => {
      const index = new QuantizedNavigableIndex(
        timestampAttr,
        Quantizers.integerMultiple(1000)
      );
      expect(index.type).toBe('navigable');
    });

    it('should return correct retrieval cost (40)', () => {
      const index = new QuantizedNavigableIndex(
        timestampAttr,
        Quantizers.integerMultiple(1000)
      );
      expect(index.getRetrievalCost()).toBe(40);
    });

    it('should expose original attribute', () => {
      const index = new QuantizedNavigableIndex(
        timestampAttr,
        Quantizers.integerMultiple(1000)
      );
      expect(index.attribute).toBe(timestampAttr);
    });

    it('should expose quantizer', () => {
      const quantizer = Quantizers.integerMultiple(1000);
      const index = new QuantizedNavigableIndex(timestampAttr, quantizer);
      expect(index.getQuantizer()).toBe(quantizer);
    });
  });

  describe('quantized indexing', () => {
    it('should group values into buckets', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      // Add values that will be quantized to the same bucket
      index.add('1', { id: '1', timestamp: 0, value: 5 }); // -> bucket 0
      index.add('2', { id: '2', timestamp: 0, value: 8 }); // -> bucket 0
      index.add('3', { id: '3', timestamp: 0, value: 15 }); // -> bucket 10
      index.add('4', { id: '4', timestamp: 0, value: 23 }); // -> bucket 20

      // Should have 3 distinct buckets
      expect(index.getBucketCount()).toBe(3);
      expect(index.getStats().distinctValues).toBe(3);
    });

    it('should retrieve all values in bucket for equal query', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 8 });
      index.add('3', { id: '3', timestamp: 0, value: 15 });

      // Query for value 7 should return all items in bucket 0 (5, 8)
      const result = index.retrieve({ type: 'equal', value: 7 });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    it('should deduplicate buckets in "in" query', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 15 });

      // Values 3, 5, 7 all map to bucket 0
      const result = index.retrieve({ type: 'in', values: [3, 5, 7, 15] });
      expect([...result].sort()).toEqual(['1', '2']);
    });
  });

  describe('range queries', () => {
    let index: QuantizedNavigableIndex<string, TimestampedEvent, number>;

    beforeEach(() => {
      index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      // Add values: 5->0, 15->10, 25->20, 35->30, 45->40
      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 15 });
      index.add('3', { id: '3', timestamp: 0, value: 25 });
      index.add('4', { id: '4', timestamp: 0, value: 35 });
      index.add('5', { id: '5', timestamp: 0, value: 45 });
    });

    it('should retrieve gt using quantized value', () => {
      // gt 20 -> gt bucket 20 -> buckets 30, 40
      const result = index.retrieve({ type: 'gt', value: 20 });
      expect([...result].sort()).toEqual(['4', '5']); // values 35, 45
    });

    it('should retrieve gte using quantized value', () => {
      // gte 20 -> gte bucket 20 -> buckets 20, 30, 40
      const result = index.retrieve({ type: 'gte', value: 25 });
      expect([...result].sort()).toEqual(['3', '4', '5']); // values 25, 35, 45
    });

    it('should retrieve lt using quantized value', () => {
      // lt 20 -> lt bucket 20 -> buckets 0, 10
      const result = index.retrieve({ type: 'lt', value: 25 });
      expect([...result].sort()).toEqual(['1', '2']); // values 5, 15
    });

    it('should retrieve between using quantized values', () => {
      // between 10 and 35 -> buckets 10, 20, 30 (depending on inclusivity)
      const result = index.retrieve({
        type: 'between',
        from: 15,
        to: 35,
        fromInclusive: true,
        toInclusive: true,
      });
      expect([...result].sort()).toEqual(['2', '3', '4']); // values 15, 25, 35
    });
  });

  describe('timestamp quantization use case', () => {
    it('should group events by minute', () => {
      const index = new QuantizedNavigableIndex(
        timestampAttr,
        Quantizers.timestampInterval(60000) // 1 minute
      );

      const baseTime = 1704067200000;

      // Add events at various times
      index.add('1', { id: '1', timestamp: baseTime, value: 1 }); // minute 0
      index.add('2', { id: '2', timestamp: baseTime + 30000, value: 2 }); // still minute 0
      index.add('3', { id: '3', timestamp: baseTime + 60000, value: 3 }); // minute 1
      index.add('4', { id: '4', timestamp: baseTime + 90000, value: 4 }); // still minute 1
      index.add('5', { id: '5', timestamp: baseTime + 120000, value: 5 }); // minute 2

      // Should have 3 buckets (3 minutes)
      expect(index.getBucketCount()).toBe(3);

      // Query for events in first minute
      const firstMinute = index.retrieve({ type: 'equal', value: baseTime + 15000 });
      expect([...firstMinute].sort()).toEqual(['1', '2']);

      // Query for events >= minute 1
      const laterMinutes = index.retrieve({
        type: 'gte',
        value: baseTime + 60000,
      });
      expect([...laterMinutes].sort()).toEqual(['3', '4', '5']);
    });
  });

  describe('add/remove/update', () => {
    it('should add records with quantized values', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 8 });

      expect(index.getStats().totalEntries).toBe(2);
      expect(index.getBucketCount()).toBe(1); // Both in bucket 0
    });

    it('should remove records correctly', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      const event = { id: '1', timestamp: 0, value: 5 };
      index.add('1', event);
      index.remove('1', event);

      expect(index.getStats().totalEntries).toBe(0);
      expect(index.getBucketCount()).toBe(0);
    });

    it('should update when quantized value changes', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      const event1 = { id: '1', timestamp: 0, value: 5 }; // bucket 0
      const event2 = { id: '1', timestamp: 0, value: 15 }; // bucket 10

      index.add('1', event1);
      index.update('1', event1, event2);

      expect([...index.retrieve({ type: 'equal', value: 5 })]).toEqual([]);
      expect([...index.retrieve({ type: 'equal', value: 15 })]).toEqual(['1']);
    });

    it('should skip update when quantized value unchanged', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      const event1 = { id: '1', timestamp: 0, value: 5 }; // bucket 0
      const event2 = { id: '1', timestamp: 0, value: 8 }; // still bucket 0

      index.add('1', event1);
      const statsBefore = index.getStats();

      index.update('1', event1, event2);

      expect(index.getStats()).toEqual(statsBefore);
    });
  });

  describe('clear', () => {
    it('should clear all entries', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 15 });

      index.clear();

      expect(index.getStats().totalEntries).toBe(0);
      expect(index.getBucketCount()).toBe(0);
    });
  });

  describe('has query', () => {
    it('should return all indexed keys', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      index.add('1', { id: '1', timestamp: 0, value: 5 });
      index.add('2', { id: '2', timestamp: 0, value: 15 });
      index.add('3', { id: '3', timestamp: 0, value: 25 });

      const result = index.retrieve({ type: 'has' });
      expect([...result].sort()).toEqual(['1', '2', '3']);
    });
  });

  describe('edge cases', () => {
    it('should handle empty index', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      expect([...index.retrieve({ type: 'equal', value: 10 })]).toEqual([]);
      expect([...index.retrieve({ type: 'gt', value: 10 })]).toEqual([]);
      expect([...index.retrieve({ type: 'has' })]).toEqual([]);
    });

    it('should throw for unsupported query type', () => {
      const index = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(10)
      );

      expect(() => {
        index.retrieve({ type: 'contains' as 'equal', value: 10 });
      }).toThrow('QuantizedNavigableIndex does not support query type: contains');
    });
  });

  describe('performance', () => {
    it('should reduce bucket count with quantization', () => {
      const regularIndex = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(1) // No quantization
      );

      const quantizedIndex = new QuantizedNavigableIndex(
        valueAttr,
        Quantizers.integerMultiple(100)
      );

      // Add 1000 events with sequential values
      for (let i = 0; i < 1000; i++) {
        const event = { id: String(i), timestamp: 0, value: i };
        regularIndex.add(String(i), event);
        quantizedIndex.add(String(i), event);
      }

      // Regular: 1000 buckets, Quantized: 10 buckets
      expect(regularIndex.getBucketCount()).toBe(1000);
      expect(quantizedIndex.getBucketCount()).toBe(10);
    });
  });
});
