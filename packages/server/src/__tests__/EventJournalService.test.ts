import { newDb } from 'pg-mem';
import { EventJournalService } from '../EventJournalService';
import type { JournalEvent } from '@topgunbuild/core';

describe('EventJournalService (Integration via pg-mem)', () => {
  let service: EventJournalService;
  let db: any;
  let pool: any;

  beforeEach(async () => {
    db = newDb();
    const { Pool } = db.adapters.createPg();
    pool = new Pool();
    service = new EventJournalService({
      pool,
      capacity: 100,
      ttlMs: 0,
      persistent: true,
      tableName: 'test_event_journal',
      persistBatchSize: 10,
      persistIntervalMs: 100,
    });
    await service.initialize();
  });

  afterEach(async () => {
    service.dispose();
    await pool.end();
  });

  describe('initialize', () => {
    it('should create event_journal table', async () => {
      const res = await pool.query(`
        SELECT table_name
        FROM information_schema.tables
        WHERE table_schema = 'public'
        AND table_name = 'test_event_journal'
      `);
      expect(res.rows.length).toBe(1);
    });
  });

  describe('append and read', () => {
    it('should append events with sequence numbers', () => {
      const e1 = service.append({
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      });

      const e2 = service.append({
        type: 'UPDATE',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice Updated' },
        previousValue: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node1' },
        nodeId: 'node1',
      });

      expect(e1.sequence).toBe(0n);
      expect(e2.sequence).toBe(1n);
    });

    it('should read events from sequence', () => {
      for (let i = 0; i < 5; i++) {
        service.append({
          type: 'PUT',
          mapName: 'items',
          key: `item${i}`,
          value: { index: i },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node1' },
          nodeId: 'node1',
        });
      }

      const events = service.readFrom(2n, 2);
      expect(events.length).toBe(2);
      expect(events[0].key).toBe('item2');
      expect(events[1].key).toBe('item3');
    });
  });

  describe('persistToStorage', () => {
    it('should persist events to PostgreSQL', async () => {
      // Append events
      for (let i = 0; i < 15; i++) {
        service.append({
          type: 'PUT',
          mapName: 'items',
          key: `item${i}`,
          value: { index: i },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node1' },
          nodeId: 'node1',
        });
      }

      // Force persist
      await service.persistToStorage();

      // Check database
      const res = await pool.query('SELECT COUNT(*) FROM test_event_journal');
      expect(parseInt(res.rows[0].count)).toBeGreaterThan(0);
    });
  });

  describe('loadFromStorage', () => {
    // Note: Tests that create new EventJournalService instances are skipped because
    // pg-mem doesn't support CHECK constraints in CREATE TABLE.
    // These tests should be run against a real PostgreSQL database.

    it('should load events and prevent double-persist via flag', async () => {
      // Append and persist events
      for (let i = 0; i < 5; i++) {
        service.append({
          type: 'PUT',
          mapName: 'items',
          key: `item${i}`,
          value: { index: i },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node1' },
          nodeId: 'node1',
        });
      }
      await service.persistToStorage();

      // Get initial count
      const res1 = await pool.query('SELECT COUNT(*) FROM test_event_journal');
      const initialCount = parseInt(res1.rows[0].count);

      // Load from storage using the same service (simulates restart)
      // This clears in-memory buffer and reloads from DB
      await service.loadFromStorage();

      // The isLoadingFromStorage flag should prevent re-persisting loaded events
      // Trigger persist to verify no duplicates are created
      await service.persistToStorage();

      // Count should be the same (no duplicates)
      const res2 = await pool.query('SELECT COUNT(*) FROM test_event_journal');
      const finalCount = parseInt(res2.rows[0].count);

      expect(finalCount).toBe(initialCount);
    });
  });

  describe('subscribe', () => {
    it('should notify subscribers of new events', () => {
      const events: JournalEvent[] = [];
      const unsubscribe = service.subscribe((event) => {
        events.push(event);
      });

      service.append({
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      });

      service.append({
        type: 'DELETE',
        mapName: 'users',
        key: 'user1',
        previousValue: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node1' },
        nodeId: 'node1',
      });

      expect(events.length).toBe(2);
      expect(events[0].type).toBe('PUT');
      expect(events[1].type).toBe('DELETE');

      unsubscribe();
    });

    it('should replay events from sequence on subscribe', () => {
      // Append some events first
      for (let i = 0; i < 5; i++) {
        service.append({
          type: 'PUT',
          mapName: 'items',
          key: `item${i}`,
          value: { index: i },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node1' },
          nodeId: 'node1',
        });
      }

      // Subscribe from sequence 2
      const events: JournalEvent[] = [];
      const unsubscribe = service.subscribe(
        (event) => events.push(event),
        2n
      );

      // Should have replayed 3 events (2, 3, 4)
      expect(events.length).toBe(3);
      expect(events[0].key).toBe('item2');
      expect(events[2].key).toBe('item4');

      unsubscribe();
    });
  });

  describe('queryFromStorage', () => {
    it('should query events with filters', async () => {
      // Append and persist events
      service.append({
        type: 'PUT',
        mapName: 'users',
        key: 'user1',
        value: { name: 'Alice' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      });
      service.append({
        type: 'PUT',
        mapName: 'orders',
        key: 'order1',
        value: { total: 100 },
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node1' },
        nodeId: 'node1',
      });
      service.append({
        type: 'DELETE',
        mapName: 'users',
        key: 'user2',
        timestamp: { millis: Date.now(), counter: 2, nodeId: 'node1' },
        nodeId: 'node1',
      });

      await service.persistToStorage();

      // Query by map name
      const userEvents = await service.queryFromStorage({ mapName: 'users' });
      expect(userEvents.length).toBe(2);

      // Query by type
      const deleteEvents = await service.queryFromStorage({ types: ['DELETE'] });
      expect(deleteEvents.length).toBe(1);
      expect(deleteEvents[0].type).toBe('DELETE');
    });
  });

  describe('countFromStorage', () => {
    it('should count events with filters', async () => {
      for (let i = 0; i < 10; i++) {
        service.append({
          type: i % 2 === 0 ? 'PUT' : 'UPDATE',
          mapName: 'items',
          key: `item${i}`,
          value: { index: i },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node1' },
          nodeId: 'node1',
        });
      }

      await service.persistToStorage();

      const totalCount = await service.countFromStorage({});
      expect(totalCount).toBe(10);

      const putCount = await service.countFromStorage({ types: ['PUT'] });
      expect(putCount).toBe(5);
    });
  });

  describe('getCapacity', () => {
    it('should return capacity info', () => {
      service.append({
        type: 'PUT',
        mapName: 'test',
        key: 'key1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      });

      const capacity = service.getCapacity();
      expect(capacity.used).toBe(1);
      expect(capacity.total).toBe(100);
    });
  });

  describe('map filtering', () => {
    it('should filter events by includeMaps', async () => {
      const filteredService = new EventJournalService({
        pool,
        capacity: 100,
        ttlMs: 0,
        persistent: false,
        tableName: 'filtered_journal',
        includeMaps: ['allowed'],
      });

      const e1 = filteredService.append({
        type: 'PUT',
        mapName: 'allowed',
        key: 'key1',
        value: {},
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
        nodeId: 'node1',
      });

      const e2 = filteredService.append({
        type: 'PUT',
        mapName: 'not_allowed',
        key: 'key2',
        value: {},
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node1' },
        nodeId: 'node1',
      });

      expect(e1.sequence).toBe(0n);
      expect(e2.sequence).toBe(-1n); // Filtered out

      const events = filteredService.readFrom(0n, 10);
      expect(events.length).toBe(1);

      filteredService.dispose();
    });
  });
});
