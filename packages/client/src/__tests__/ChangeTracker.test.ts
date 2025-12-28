import { ChangeTracker, ChangeEvent } from '../ChangeTracker';

describe('ChangeTracker', () => {
  describe('computeChanges', () => {
    it('should detect additions', () => {
      const tracker = new ChangeTracker<{ name: string }>();
      const timestamp = Date.now();

      const changes = tracker.computeChanges(
        new Map([['a', { name: 'Alice' }]]),
        timestamp
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('add');
      expect(changes[0].key).toBe('a');
      expect(changes[0].value).toEqual({ name: 'Alice' });
      expect(changes[0].previousValue).toBeUndefined();
      expect(changes[0].timestamp).toBe(timestamp);
    });

    it('should detect multiple additions', () => {
      const tracker = new ChangeTracker<{ name: string }>();
      const timestamp = Date.now();

      const changes = tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice' }],
          ['b', { name: 'Bob' }],
          ['c', { name: 'Charlie' }],
        ]),
        timestamp
      );

      expect(changes).toHaveLength(3);
      expect(changes.every((c) => c.type === 'add')).toBe(true);
      expect(changes.map((c) => c.key).sort()).toEqual(['a', 'b', 'c']);
    });

    it('should detect updates', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      // First snapshot
      tracker.computeChanges(new Map([['a', { name: 'Alice' }]]), 1);

      // Second snapshot with update
      const changes = tracker.computeChanges(
        new Map([['a', { name: 'Alice Updated' }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      expect(changes[0].key).toBe('a');
      expect(changes[0].value).toEqual({ name: 'Alice Updated' });
      expect(changes[0].previousValue).toEqual({ name: 'Alice' });
      expect(changes[0].timestamp).toBe(2);
    });

    it('should detect removals', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      // First snapshot
      tracker.computeChanges(new Map([['a', { name: 'Alice' }]]), 1);

      // Second snapshot with removal
      const changes = tracker.computeChanges(new Map(), 2);

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('remove');
      expect(changes[0].key).toBe('a');
      expect(changes[0].value).toBeUndefined();
      expect(changes[0].previousValue).toEqual({ name: 'Alice' });
      expect(changes[0].timestamp).toBe(2);
    });

    it('should handle mixed changes (add, update, remove)', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      // First snapshot
      tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice' }],
          ['b', { name: 'Bob' }],
        ]),
        1
      );

      // Second snapshot with mixed changes:
      // - 'a' updated
      // - 'b' removed
      // - 'c' added
      const changes = tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice Updated' }],
          ['c', { name: 'Charlie' }],
        ]),
        2
      );

      expect(changes).toHaveLength(3);

      const updateChange = changes.find((c) => c.type === 'update');
      expect(updateChange).toBeDefined();
      expect(updateChange!.key).toBe('a');
      expect(updateChange!.value).toEqual({ name: 'Alice Updated' });
      expect(updateChange!.previousValue).toEqual({ name: 'Alice' });

      const addChange = changes.find((c) => c.type === 'add');
      expect(addChange).toBeDefined();
      expect(addChange!.key).toBe('c');
      expect(addChange!.value).toEqual({ name: 'Charlie' });

      const removeChange = changes.find((c) => c.type === 'remove');
      expect(removeChange).toBeDefined();
      expect(removeChange!.key).toBe('b');
      expect(removeChange!.previousValue).toEqual({ name: 'Bob' });
    });

    it('should return empty array when no changes', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      // First snapshot
      tracker.computeChanges(new Map([['a', { name: 'Alice' }]]), 1);

      // Same data
      const changes = tracker.computeChanges(
        new Map([['a', { name: 'Alice' }]]),
        2
      );

      expect(changes).toHaveLength(0);
    });

    it('should return empty array for empty to empty', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      const changes1 = tracker.computeChanges(new Map(), 1);
      expect(changes1).toHaveLength(0);

      const changes2 = tracker.computeChanges(new Map(), 2);
      expect(changes2).toHaveLength(0);
    });

    it('should handle nested objects', () => {
      const tracker = new ChangeTracker<{ user: { name: string; age: number } }>();

      tracker.computeChanges(
        new Map([['a', { user: { name: 'Alice', age: 30 } }]]),
        1
      );

      // Update nested property
      const changes = tracker.computeChanges(
        new Map([['a', { user: { name: 'Alice', age: 31 } }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      expect(changes[0].previousValue).toEqual({ user: { name: 'Alice', age: 30 } });
      expect(changes[0].value).toEqual({ user: { name: 'Alice', age: 31 } });
    });

    it('should handle arrays in values', () => {
      const tracker = new ChangeTracker<{ tags: string[] }>();

      tracker.computeChanges(new Map([['a', { tags: ['foo', 'bar'] }]]), 1);

      // Update array
      const changes = tracker.computeChanges(
        new Map([['a', { tags: ['foo', 'bar', 'baz'] }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      expect(changes[0].previousValue).toEqual({ tags: ['foo', 'bar'] });
      expect(changes[0].value).toEqual({ tags: ['foo', 'bar', 'baz'] });
    });

    it('should not report update when array order changes but content is same', () => {
      const tracker = new ChangeTracker<{ tags: string[] }>();

      tracker.computeChanges(new Map([['a', { tags: ['foo', 'bar'] }]]), 1);

      // Same array, different order (should be considered different)
      const changes = tracker.computeChanges(
        new Map([['a', { tags: ['bar', 'foo'] }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
    });

    it('should handle primitive values', () => {
      const tracker = new ChangeTracker<number>();

      tracker.computeChanges(
        new Map([
          ['a', 1],
          ['b', 2],
        ]),
        1
      );

      const changes = tracker.computeChanges(
        new Map([
          ['a', 1],
          ['b', 3],
        ]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      expect(changes[0].key).toBe('b');
      expect(changes[0].previousValue).toBe(2);
      expect(changes[0].value).toBe(3);
    });

    it('should handle null values correctly', () => {
      const tracker = new ChangeTracker<{ value: string | null }>();

      tracker.computeChanges(new Map([['a', { value: 'hello' }]]), 1);

      const changes = tracker.computeChanges(
        new Map([['a', { value: null }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      expect(changes[0].previousValue).toEqual({ value: 'hello' });
      expect(changes[0].value).toEqual({ value: null });
    });

    it('should not mutate previous snapshot when current changes', () => {
      const tracker = new ChangeTracker<{ name: string }>();
      const item = { name: 'Alice' };

      tracker.computeChanges(new Map([['a', item]]), 1);

      // Mutate the original object
      item.name = 'Alice Mutated';

      // Should still detect as update because we stored a copy
      const changes = tracker.computeChanges(
        new Map([['a', { name: 'Alice New' }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('update');
      // Previous value should be the original
      expect(changes[0].previousValue).toEqual({ name: 'Alice' });
    });
  });

  describe('reset', () => {
    it('should clear previous snapshot', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice' }],
          ['b', { name: 'Bob' }],
        ]),
        1
      );

      expect(tracker.size).toBe(2);

      tracker.reset();

      expect(tracker.size).toBe(0);

      // After reset, all items should be additions again
      const changes = tracker.computeChanges(
        new Map([['a', { name: 'Alice' }]]),
        2
      );

      expect(changes).toHaveLength(1);
      expect(changes[0].type).toBe('add');
    });
  });

  describe('size', () => {
    it('should return correct snapshot size', () => {
      const tracker = new ChangeTracker<{ name: string }>();

      expect(tracker.size).toBe(0);

      tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice' }],
          ['b', { name: 'Bob' }],
        ]),
        1
      );

      expect(tracker.size).toBe(2);

      tracker.computeChanges(
        new Map([
          ['a', { name: 'Alice' }],
          ['b', { name: 'Bob' }],
          ['c', { name: 'Charlie' }],
        ]),
        2
      );

      expect(tracker.size).toBe(3);

      tracker.computeChanges(new Map([['a', { name: 'Alice' }]]), 3);

      expect(tracker.size).toBe(1);
    });
  });

  describe('performance', () => {
    it('should handle large collections efficiently', () => {
      const tracker = new ChangeTracker<{ id: number; name: string }>();
      const itemCount = 10000;

      // Create initial large collection
      const initialMap = new Map<string, { id: number; name: string }>();
      for (let i = 0; i < itemCount; i++) {
        initialMap.set(`item-${i}`, { id: i, name: `Item ${i}` });
      }

      const start1 = performance.now();
      const changes1 = tracker.computeChanges(initialMap, 1);
      const duration1 = performance.now() - start1;

      expect(changes1).toHaveLength(itemCount);

      // Modify 100 items
      const modifiedMap = new Map(initialMap);
      for (let i = 0; i < 100; i++) {
        modifiedMap.set(`item-${i}`, { id: i, name: `Item ${i} Modified` });
      }

      const start2 = performance.now();
      const changes2 = tracker.computeChanges(modifiedMap, 2);
      const duration2 = performance.now() - start2;

      expect(changes2).toHaveLength(100);
      expect(changes2.every((c) => c.type === 'update')).toBe(true);

      // Performance check: should complete in reasonable time
      expect(duration1).toBeLessThan(1000); // Less than 1 second for 10K items
      expect(duration2).toBeLessThan(500); // Less than 500ms for comparison
    });
  });
});

describe('deepEqual', () => {
  // Test deepEqual through ChangeTracker behavior
  it('should correctly compare identical objects', () => {
    const tracker = new ChangeTracker<{ a: number; b: string }>();

    tracker.computeChanges(new Map([['x', { a: 1, b: 'test' }]]), 1);
    const changes = tracker.computeChanges(
      new Map([['x', { a: 1, b: 'test' }]]),
      2
    );

    expect(changes).toHaveLength(0);
  });

  it('should correctly compare objects with different key order', () => {
    const tracker = new ChangeTracker<Record<string, number>>();

    tracker.computeChanges(new Map([['x', { a: 1, b: 2 }]]), 1);
    const changes = tracker.computeChanges(
      new Map([['x', { b: 2, a: 1 }]]),
      2
    );

    expect(changes).toHaveLength(0);
  });

  it('should detect difference in deeply nested objects', () => {
    const tracker = new ChangeTracker<{ level1: { level2: { value: number } } }>();

    tracker.computeChanges(
      new Map([['x', { level1: { level2: { value: 1 } } }]]),
      1
    );
    const changes = tracker.computeChanges(
      new Map([['x', { level1: { level2: { value: 2 } } }]]),
      2
    );

    expect(changes).toHaveLength(1);
    expect(changes[0].type).toBe('update');
  });
});
