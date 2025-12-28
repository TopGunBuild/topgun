import { Ringbuffer } from '../Ringbuffer';

describe('Ringbuffer', () => {
  describe('constructor', () => {
    it('should create buffer with specified capacity', () => {
      const buffer = new Ringbuffer<string>(10);
      expect(buffer.getCapacity()).toBe(10);
      expect(buffer.size()).toBe(0);
    });

    it('should throw error for capacity < 1', () => {
      expect(() => new Ringbuffer<string>(0)).toThrow('Capacity must be >= 1');
      expect(() => new Ringbuffer<string>(-1)).toThrow('Capacity must be >= 1');
    });
  });

  describe('add', () => {
    it('should add items and return sequence numbers', () => {
      const buffer = new Ringbuffer<string>(5);

      const seq0 = buffer.add('a');
      const seq1 = buffer.add('b');
      const seq2 = buffer.add('c');

      expect(seq0).toBe(0n);
      expect(seq1).toBe(1n);
      expect(seq2).toBe(2n);
      expect(buffer.size()).toBe(3);
    });

    it('should overwrite oldest items when capacity is reached', () => {
      const buffer = new Ringbuffer<string>(3);

      buffer.add('a'); // seq 0
      buffer.add('b'); // seq 1
      buffer.add('c'); // seq 2
      buffer.add('d'); // seq 3, overwrites 'a'
      buffer.add('e'); // seq 4, overwrites 'b'

      expect(buffer.size()).toBe(3);
      expect(buffer.getHeadSequence()).toBe(2n);
      expect(buffer.getTailSequence()).toBe(5n);

      expect(buffer.read(0n)).toBeUndefined(); // evicted
      expect(buffer.read(1n)).toBeUndefined(); // evicted
      expect(buffer.read(2n)).toBe('c');
      expect(buffer.read(3n)).toBe('d');
      expect(buffer.read(4n)).toBe('e');
    });
  });

  describe('read', () => {
    it('should read item at sequence', () => {
      const buffer = new Ringbuffer<string>(5);
      buffer.add('a');
      buffer.add('b');
      buffer.add('c');

      expect(buffer.read(0n)).toBe('a');
      expect(buffer.read(1n)).toBe('b');
      expect(buffer.read(2n)).toBe('c');
    });

    it('should return undefined for out of range sequence', () => {
      const buffer = new Ringbuffer<string>(5);
      buffer.add('a');
      buffer.add('b');

      expect(buffer.read(-1n)).toBeUndefined();
      expect(buffer.read(2n)).toBeUndefined();
      expect(buffer.read(100n)).toBeUndefined();
    });
  });

  describe('readRange', () => {
    it('should read range of items', () => {
      const buffer = new Ringbuffer<string>(10);
      buffer.add('a');
      buffer.add('b');
      buffer.add('c');
      buffer.add('d');
      buffer.add('e');

      const items = buffer.readRange(1n, 3n);
      expect(items).toEqual(['b', 'c', 'd']);
    });

    it('should clamp to available range', () => {
      const buffer = new Ringbuffer<string>(5);
      buffer.add('a');
      buffer.add('b');
      buffer.add('c');

      // Request beyond available
      const items = buffer.readRange(0n, 100n);
      expect(items).toEqual(['a', 'b', 'c']);

      // Request before available
      const items2 = buffer.readRange(-10n, 1n);
      expect(items2).toEqual(['a', 'b']);
    });

    it('should return empty for completely out of range', () => {
      const buffer = new Ringbuffer<string>(5);
      buffer.add('a');
      buffer.add('b');

      const items = buffer.readRange(10n, 20n);
      expect(items).toEqual([]);
    });
  });

  describe('readFrom', () => {
    it('should read from sequence with limit', () => {
      const buffer = new Ringbuffer<string>(10);
      for (let i = 0; i < 10; i++) {
        buffer.add(`item${i}`);
      }

      const items = buffer.readFrom(3n, 3);
      expect(items).toEqual(['item3', 'item4', 'item5']);
    });

    it('should use default limit of 100', () => {
      const buffer = new Ringbuffer<string>(200);
      for (let i = 0; i < 150; i++) {
        buffer.add(`item${i}`);
      }

      const items = buffer.readFrom(0n);
      expect(items.length).toBe(100);
    });
  });

  describe('sequence tracking', () => {
    it('should track head and tail sequences', () => {
      const buffer = new Ringbuffer<number>(3);

      expect(buffer.getHeadSequence()).toBe(0n);
      expect(buffer.getTailSequence()).toBe(0n);

      buffer.add(1);
      expect(buffer.getHeadSequence()).toBe(0n);
      expect(buffer.getTailSequence()).toBe(1n);

      buffer.add(2);
      buffer.add(3);
      expect(buffer.getHeadSequence()).toBe(0n);
      expect(buffer.getTailSequence()).toBe(3n);

      buffer.add(4); // Evicts seq 0
      expect(buffer.getHeadSequence()).toBe(1n);
      expect(buffer.getTailSequence()).toBe(4n);
    });
  });

  describe('isAvailable', () => {
    it('should check if sequence is available', () => {
      const buffer = new Ringbuffer<string>(3);
      buffer.add('a');
      buffer.add('b');
      buffer.add('c');
      buffer.add('d'); // evicts 'a'

      expect(buffer.isAvailable(0n)).toBe(false); // evicted
      expect(buffer.isAvailable(1n)).toBe(true);
      expect(buffer.isAvailable(2n)).toBe(true);
      expect(buffer.isAvailable(3n)).toBe(true);
      expect(buffer.isAvailable(4n)).toBe(false); // not added yet
    });
  });

  describe('remainingCapacity', () => {
    it('should return remaining capacity', () => {
      const buffer = new Ringbuffer<string>(5);

      expect(buffer.remainingCapacity()).toBe(5);

      buffer.add('a');
      expect(buffer.remainingCapacity()).toBe(4);

      buffer.add('b');
      buffer.add('c');
      expect(buffer.remainingCapacity()).toBe(2);

      buffer.add('d');
      buffer.add('e');
      expect(buffer.remainingCapacity()).toBe(0);

      // After overflow, remaining capacity stays 0
      buffer.add('f');
      expect(buffer.remainingCapacity()).toBe(0);
    });
  });

  describe('clear', () => {
    it('should clear all items and reset sequences', () => {
      const buffer = new Ringbuffer<string>(5);
      buffer.add('a');
      buffer.add('b');
      buffer.add('c');

      buffer.clear();

      expect(buffer.size()).toBe(0);
      expect(buffer.getHeadSequence()).toBe(0n);
      expect(buffer.getTailSequence()).toBe(0n);
      expect(buffer.read(0n)).toBeUndefined();
    });
  });

  describe('various types', () => {
    it('should work with objects', () => {
      const buffer = new Ringbuffer<{ id: number; name: string }>(3);

      buffer.add({ id: 1, name: 'Alice' });
      buffer.add({ id: 2, name: 'Bob' });

      const item = buffer.read(0n);
      expect(item).toEqual({ id: 1, name: 'Alice' });
    });

    it('should work with numbers', () => {
      const buffer = new Ringbuffer<number>(3);

      buffer.add(1);
      buffer.add(2);
      buffer.add(3);

      expect(buffer.readRange(0n, 2n)).toEqual([1, 2, 3]);
    });
  });

  describe('edge cases', () => {
    it('should handle capacity of 1', () => {
      const buffer = new Ringbuffer<string>(1);

      buffer.add('a');
      expect(buffer.read(0n)).toBe('a');

      buffer.add('b');
      expect(buffer.read(0n)).toBeUndefined();
      expect(buffer.read(1n)).toBe('b');
      expect(buffer.size()).toBe(1);
    });

    it('should handle large number of operations', () => {
      const buffer = new Ringbuffer<number>(100);

      for (let i = 0; i < 10000; i++) {
        buffer.add(i);
      }

      expect(buffer.size()).toBe(100);
      expect(buffer.getHeadSequence()).toBe(9900n);
      expect(buffer.getTailSequence()).toBe(10000n);

      // Should be able to read the last 100 items
      const items = buffer.readFrom(9900n, 100);
      expect(items.length).toBe(100);
      expect(items[0]).toBe(9900);
      expect(items[99]).toBe(9999);
    });
  });
});
