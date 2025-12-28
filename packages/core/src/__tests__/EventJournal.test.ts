import { EventJournalImpl, DEFAULT_EVENT_JOURNAL_CONFIG } from '../EventJournal';
import type { JournalEventInput, JournalEvent } from '../EventJournal';

describe('EventJournalImpl', () => {
  let journal: EventJournalImpl;

  beforeEach(() => {
    journal = new EventJournalImpl({ capacity: 100, persistent: false });
  });

  afterEach(() => {
    journal.dispose();
  });

  describe('constructor', () => {
    it('should create journal with default config', () => {
      const j = new EventJournalImpl();
      expect(j.getConfig().capacity).toBe(DEFAULT_EVENT_JOURNAL_CONFIG.capacity);
      expect(j.getConfig().ttlMs).toBe(DEFAULT_EVENT_JOURNAL_CONFIG.ttlMs);
      j.dispose();
    });

    it('should merge custom config with defaults', () => {
      const j = new EventJournalImpl({ capacity: 500, ttlMs: 60000 });
      expect(j.getConfig().capacity).toBe(500);
      expect(j.getConfig().ttlMs).toBe(60000);
      j.dispose();
    });
  });

  describe('append', () => {
    it('should append event and return with sequence', () => {
      const event: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const result = journal.append(event);

      expect(result.sequence).toBe(0n);
      expect(result.type).toBe('PUT');
      expect(result.mapName).toBe('users');
      expect(result.key).toBe('user1');
      expect(result.value).toEqual({ name: 'Alice' });
    });

    it('should assign incrementing sequences', () => {
      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const e1 = journal.append({ ...baseEvent, key: 'a' });
      const e2 = journal.append({ ...baseEvent, key: 'b' });
      const e3 = journal.append({ ...baseEvent, key: 'c' });

      expect(e1.sequence).toBe(0n);
      expect(e2.sequence).toBe(1n);
      expect(e3.sequence).toBe(2n);
    });

    it('should track previous value for updates', () => {
      const event: JournalEventInput = {
        type: 'UPDATE',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice Updated' },
        previousValue: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const result = journal.append(event);

      expect(result.previousValue).toEqual({ name: 'Alice' });
    });

    it('should not have value for DELETE events', () => {
      const event: JournalEventInput = {
        type: 'DELETE',
        mapName: 'users',
        key: 'user1',
        previousValue: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const result = journal.append(event);

      expect(result.value).toBeUndefined();
      expect(result.previousValue).toEqual({ name: 'Alice' });
    });
  });

  describe('map filtering', () => {
    it('should filter by includeMaps', () => {
      const j = new EventJournalImpl({
        capacity: 100,
        persistent: false,
        includeMaps: ['users', 'orders'],
      });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const e1 = j.append({ ...baseEvent, mapName: 'users' });
      const e2 = j.append({ ...baseEvent, mapName: 'orders' });
      const e3 = j.append({ ...baseEvent, mapName: 'products' }); // excluded

      expect(e1.sequence).toBe(0n);
      expect(e2.sequence).toBe(1n);
      expect(e3.sequence).toBe(-1n); // filtered out

      const events = j.readFrom(0n, 10);
      expect(events.length).toBe(2);

      j.dispose();
    });

    it('should filter by excludeMaps', () => {
      const j = new EventJournalImpl({
        capacity: 100,
        persistent: false,
        excludeMaps: ['internal'],
      });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      const e1 = j.append({ ...baseEvent, mapName: 'users' });
      const e2 = j.append({ ...baseEvent, mapName: 'internal' }); // excluded

      expect(e1.sequence).toBe(0n);
      expect(e2.sequence).toBe(-1n);

      j.dispose();
    });
  });

  describe('readFrom', () => {
    it('should read events from sequence', () => {
      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      journal.append({ ...baseEvent, key: 'a' });
      journal.append({ ...baseEvent, key: 'b' });
      journal.append({ ...baseEvent, key: 'c' });
      journal.append({ ...baseEvent, key: 'd' });
      journal.append({ ...baseEvent, key: 'e' });

      const events = journal.readFrom(2n, 2);

      expect(events.length).toBe(2);
      expect(events[0].key).toBe('c');
      expect(events[1].key).toBe('d');
    });
  });

  describe('readRange', () => {
    it('should read events in range', () => {
      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      for (let i = 0; i < 10; i++) {
        journal.append({ ...baseEvent, key: `key${i}` });
      }

      const events = journal.readRange(3n, 6n);

      expect(events.length).toBe(4);
      expect(events[0].key).toBe('key3');
      expect(events[3].key).toBe('key6');
    });
  });

  describe('sequences', () => {
    it('should track latest and oldest sequences', () => {
      expect(journal.getLatestSequence()).toBe(0n);
      expect(journal.getOldestSequence()).toBe(0n);

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      journal.append({ ...baseEvent, key: 'a' });
      expect(journal.getLatestSequence()).toBe(0n);

      journal.append({ ...baseEvent, key: 'b' });
      journal.append({ ...baseEvent, key: 'c' });
      expect(journal.getLatestSequence()).toBe(2n);
      expect(journal.getOldestSequence()).toBe(0n);
    });
  });

  describe('subscribe', () => {
    it('should notify listeners on new events', () => {
      const events: JournalEvent[] = [];
      const unsubscribe = journal.subscribe((event) => {
        events.push(event);
      });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      journal.append({ ...baseEvent, key: 'a' });
      journal.append({ ...baseEvent, key: 'b' });

      expect(events.length).toBe(2);
      expect(events[0].key).toBe('a');
      expect(events[1].key).toBe('b');

      unsubscribe();
    });

    it('should replay events from sequence', () => {
      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      journal.append({ ...baseEvent, key: 'a' });
      journal.append({ ...baseEvent, key: 'b' });
      journal.append({ ...baseEvent, key: 'c' });

      const events: JournalEvent[] = [];
      const unsubscribe = journal.subscribe(
        (event) => events.push(event),
        1n // Start from sequence 1
      );

      // Should have received replay of b and c
      expect(events.length).toBe(2);
      expect(events[0].key).toBe('b');
      expect(events[1].key).toBe('c');

      unsubscribe();
    });

    it('should unsubscribe correctly', () => {
      const events: JournalEvent[] = [];
      const unsubscribe = journal.subscribe((event) => {
        events.push(event);
      });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      journal.append({ ...baseEvent, key: 'a' });
      expect(events.length).toBe(1);

      unsubscribe();

      journal.append({ ...baseEvent, key: 'b' });
      expect(events.length).toBe(1); // Should not receive new event
    });

    it('should handle errors in listeners gracefully', () => {
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation();

      journal.subscribe(() => {
        throw new Error('Listener error');
      });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      // Should not throw
      expect(() => journal.append(baseEvent)).not.toThrow();

      expect(consoleSpy).toHaveBeenCalledWith(
        'EventJournal listener error:',
        expect.any(Error)
      );

      consoleSpy.mockRestore();
    });
  });

  describe('getCapacity', () => {
    it('should return capacity info', () => {
      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      expect(journal.getCapacity()).toEqual({ used: 0, total: 100 });

      journal.append(baseEvent);
      journal.append(baseEvent);
      journal.append(baseEvent);

      expect(journal.getCapacity()).toEqual({ used: 3, total: 100 });
    });
  });

  describe('dispose', () => {
    it('should clear listeners on dispose', () => {
      journal.subscribe(() => {});
      journal.subscribe(() => {});

      expect(journal.getListenerCount()).toBe(2);

      journal.dispose();

      expect(journal.getListenerCount()).toBe(0);
    });
  });

  describe('metadata', () => {
    it('should store optional metadata', () => {
      const event: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
        metadata: {
          userId: 'admin',
          source: 'api',
        },
      };

      const result = journal.append(event);

      expect(result.metadata).toEqual({
        userId: 'admin',
        source: 'api',
      });

      const events = journal.readFrom(0n, 1);
      expect(events[0].metadata).toEqual({
        userId: 'admin',
        source: 'api',
      });
    });
  });

  describe('ringbuffer eviction', () => {
    it('should evict oldest events when capacity reached', () => {
      const j = new EventJournalImpl({ capacity: 5, persistent: false });

      const baseEvent: JournalEventInput = {
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      };

      // Add 7 events (capacity is 5)
      for (let i = 0; i < 7; i++) {
        j.append({ ...baseEvent, key: `key${i}` });
      }

      expect(j.getCapacity().used).toBe(5);
      expect(j.getOldestSequence()).toBe(2n);
      expect(j.getLatestSequence()).toBe(6n);

      const events = j.readFrom(0n, 10);
      expect(events.length).toBe(5);
      expect(events[0].key).toBe('key2'); // First available
      expect(events[4].key).toBe('key6'); // Last

      j.dispose();
    });
  });
});
