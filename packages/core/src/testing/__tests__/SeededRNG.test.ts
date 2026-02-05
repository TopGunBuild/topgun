import { SeededRNG } from '../SeededRNG';

describe('SeededRNG', () => {
  describe('constructor', () => {
    test('creates with integer seed', () => {
      const rng = new SeededRNG(12345);
      expect(rng.getSeed()).toBe(12345);
    });

    test('rejects non-integer seed', () => {
      expect(() => new SeededRNG(123.45)).toThrow('Seed must be an integer');
    });
  });

  describe('getSeed()', () => {
    test('returns original seed', () => {
      const rng = new SeededRNG(99999);
      expect(rng.getSeed()).toBe(99999);
    });
  });

  describe('random()', () => {
    test('generates numbers in [0, 1)', () => {
      const rng = new SeededRNG(12345);
      for (let i = 0; i < 100; i++) {
        const value = rng.random();
        expect(value).toBeGreaterThanOrEqual(0);
        expect(value).toBeLessThan(1);
      }
    });

    test('produces deterministic sequence for same seed', () => {
      const rng1 = new SeededRNG(42);
      const sequence1 = [rng1.random(), rng1.random(), rng1.random()];

      const rng2 = new SeededRNG(42);
      const sequence2 = [rng2.random(), rng2.random(), rng2.random()];

      expect(sequence2).toEqual(sequence1);
    });

    test('produces different sequences for different seeds', () => {
      const rng1 = new SeededRNG(42);
      const sequence1 = [rng1.random(), rng1.random(), rng1.random()];

      const rng2 = new SeededRNG(43);
      const sequence2 = [rng2.random(), rng2.random(), rng2.random()];

      expect(sequence2).not.toEqual(sequence1);
    });
  });

  describe('randomInt()', () => {
    test('generates integers in [min, max]', () => {
      const rng = new SeededRNG(12345);
      for (let i = 0; i < 100; i++) {
        const value = rng.randomInt(10, 20);
        expect(value).toBeGreaterThanOrEqual(10);
        expect(value).toBeLessThanOrEqual(20);
        expect(Number.isInteger(value)).toBe(true);
      }
    });

    test('includes both min and max', () => {
      const rng = new SeededRNG(12345);
      const values = new Set<number>();
      for (let i = 0; i < 1000; i++) {
        values.add(rng.randomInt(1, 3));
      }
      expect(values.has(1)).toBe(true);
      expect(values.has(2)).toBe(true);
      expect(values.has(3)).toBe(true);
    });

    test('handles single-value range', () => {
      const rng = new SeededRNG(12345);
      expect(rng.randomInt(5, 5)).toBe(5);
    });

    test('rejects non-integer min/max', () => {
      const rng = new SeededRNG(12345);
      expect(() => rng.randomInt(1.5, 10)).toThrow('must be integers');
      expect(() => rng.randomInt(1, 10.5)).toThrow('must be integers');
    });

    test('rejects min > max', () => {
      const rng = new SeededRNG(12345);
      expect(() => rng.randomInt(10, 5)).toThrow('Min must be less than or equal to max');
    });
  });

  describe('randomBool()', () => {
    test('generates booleans with default 0.5 probability', () => {
      const rng = new SeededRNG(12345);
      let trueCount = 0;
      const iterations = 1000;

      for (let i = 0; i < iterations; i++) {
        if (rng.randomBool()) trueCount++;
      }

      // Should be roughly 50% (allow 40-60% for randomness)
      expect(trueCount).toBeGreaterThan(400);
      expect(trueCount).toBeLessThan(600);
    });

    test('respects custom probability', () => {
      const rng = new SeededRNG(12345);
      let trueCount = 0;
      const iterations = 1000;

      for (let i = 0; i < iterations; i++) {
        if (rng.randomBool(0.8)) trueCount++;
      }

      // Should be roughly 80% (allow 75-85% for randomness)
      expect(trueCount).toBeGreaterThan(750);
      expect(trueCount).toBeLessThan(850);
    });

    test('probability 0 always returns false', () => {
      const rng = new SeededRNG(12345);
      for (let i = 0; i < 100; i++) {
        expect(rng.randomBool(0)).toBe(false);
      }
    });

    test('probability 1 always returns true', () => {
      const rng = new SeededRNG(12345);
      for (let i = 0; i < 100; i++) {
        expect(rng.randomBool(1)).toBe(true);
      }
    });

    test('rejects invalid probability', () => {
      const rng = new SeededRNG(12345);
      expect(() => rng.randomBool(-0.1)).toThrow('must be between 0 and 1');
      expect(() => rng.randomBool(1.1)).toThrow('must be between 0 and 1');
    });
  });

  describe('shuffle()', () => {
    test('shuffles array in place', () => {
      const rng = new SeededRNG(12345);
      const original = [1, 2, 3, 4, 5];
      const array = [...original];
      const result = rng.shuffle(array);

      expect(result).toBe(array); // Same reference
      expect(result.sort()).toEqual(original); // Same elements
    });

    test('produces deterministic shuffle', () => {
      const rng1 = new SeededRNG(42);
      const array1 = [1, 2, 3, 4, 5];
      rng1.shuffle(array1);

      const rng2 = new SeededRNG(42);
      const array2 = [1, 2, 3, 4, 5];
      rng2.shuffle(array2);

      expect(array2).toEqual(array1);
    });

    test('handles empty array', () => {
      const rng = new SeededRNG(12345);
      const array: number[] = [];
      rng.shuffle(array);
      expect(array).toEqual([]);
    });

    test('handles single-element array', () => {
      const rng = new SeededRNG(12345);
      const array = [42];
      rng.shuffle(array);
      expect(array).toEqual([42]);
    });
  });

  describe('pick()', () => {
    test('picks random element from array', () => {
      const rng = new SeededRNG(12345);
      const array = [1, 2, 3, 4, 5];
      const picked = rng.pick(array);

      expect(array).toContain(picked);
    });

    test('produces deterministic pick', () => {
      const array = [1, 2, 3, 4, 5];

      const rng1 = new SeededRNG(42);
      const pick1 = rng1.pick(array);

      const rng2 = new SeededRNG(42);
      const pick2 = rng2.pick(array);

      expect(pick2).toBe(pick1);
    });

    test('returns undefined for empty array', () => {
      const rng = new SeededRNG(12345);
      expect(rng.pick([])).toBeUndefined();
    });

    test('picks all elements over time', () => {
      const rng = new SeededRNG(12345);
      const array = [1, 2, 3];
      const picked = new Set<number>();

      for (let i = 0; i < 100; i++) {
        const value = rng.pick(array);
        if (value !== undefined) picked.add(value);
      }

      expect(picked.size).toBe(3);
    });
  });

  describe('reset()', () => {
    test('resets to original seed', () => {
      const rng = new SeededRNG(12345);
      const sequence1 = [rng.random(), rng.random(), rng.random()];

      rng.reset();
      const sequence2 = [rng.random(), rng.random(), rng.random()];

      expect(sequence2).toEqual(sequence1);
    });
  });

  describe('determinism verification', () => {
    test('same seed produces identical long sequence', () => {
      const rng1 = new SeededRNG(99999);
      const sequence1: number[] = [];
      for (let i = 0; i < 1000; i++) {
        sequence1.push(rng1.random());
      }

      const rng2 = new SeededRNG(99999);
      const sequence2: number[] = [];
      for (let i = 0; i < 1000; i++) {
        sequence2.push(rng2.random());
      }

      expect(sequence2).toEqual(sequence1);
    });
  });
});
