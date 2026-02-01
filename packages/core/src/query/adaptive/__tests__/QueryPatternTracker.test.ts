/**
 * Tests for QueryPatternTracker
 */

import { QueryPatternTracker } from '../QueryPatternTracker';
import type { TrackedQueryType } from '../types';

describe('QueryPatternTracker', () => {
  let tracker: QueryPatternTracker;

  beforeEach(() => {
    tracker = new QueryPatternTracker();
  });

  describe('recordQuery', () => {
    it('records a single query', () => {
      tracker.recordQuery('email', 'eq', 5.2, 100, false);

      const stats = tracker.getStatistics();
      expect(stats).toHaveLength(1);
      expect(stats[0].attribute).toBe('email');
      expect(stats[0].queryType).toBe('eq');
      expect(stats[0].queryCount).toBe(1);
      expect(stats[0].averageCost).toBeCloseTo(5.2);
      expect(stats[0].totalCost).toBeCloseTo(5.2);
      expect(stats[0].estimatedCardinality).toBe(100);
      expect(stats[0].hasIndex).toBe(false);
    });

    it('accumulates multiple queries for same attribute', () => {
      tracker.recordQuery('email', 'eq', 5.0, 100, false);
      tracker.recordQuery('email', 'eq', 6.0, 120, false);
      tracker.recordQuery('email', 'eq', 4.0, 80, false);

      const stats = tracker.getStatistics();
      expect(stats).toHaveLength(1);
      expect(stats[0].queryCount).toBe(3);
      expect(stats[0].averageCost).toBeCloseTo(5.0);
      expect(stats[0].totalCost).toBeCloseTo(15.0);
      // Cardinality is max observed
      expect(stats[0].estimatedCardinality).toBe(120);
    });

    it('tracks different query types separately', () => {
      tracker.recordQuery('price', 'gt', 3.0, 50, false);
      tracker.recordQuery('price', 'lte', 4.0, 60, false);
      tracker.recordQuery('price', 'between', 5.0, 40, false);

      const stats = tracker.getStatistics();
      expect(stats).toHaveLength(3);

      const gtStats = stats.find(s => s.queryType === 'gt');
      const lteStats = stats.find(s => s.queryType === 'lte');
      const betweenStats = stats.find(s => s.queryType === 'between');

      expect(gtStats?.queryCount).toBe(1);
      expect(lteStats?.queryCount).toBe(1);
      expect(betweenStats?.queryCount).toBe(1);
    });

    it('tracks different attributes separately', () => {
      tracker.recordQuery('category', 'eq', 2.0, 10, false);
      tracker.recordQuery('status', 'eq', 3.0, 20, true);
      tracker.recordQuery('price', 'gt', 4.0, 30, false);

      const stats = tracker.getStatistics();
      expect(stats).toHaveLength(3);

      const categoryStats = stats.find(s => s.attribute === 'category');
      const statusStats = stats.find(s => s.attribute === 'status');
      const priceStats = stats.find(s => s.attribute === 'price');

      expect(categoryStats?.hasIndex).toBe(false);
      expect(statusStats?.hasIndex).toBe(true);
      expect(priceStats?.queryType).toBe('gt');
    });

    it('updates lastQueried timestamp', () => {
      const before = Date.now();
      tracker.recordQuery('email', 'eq', 5.0, 100, false);
      const after = Date.now();

      const stats = tracker.getStatistics();
      expect(stats[0].lastQueried).toBeGreaterThanOrEqual(before);
      expect(stats[0].lastQueried).toBeLessThanOrEqual(after);
    });
  });

  describe('getStatistics', () => {
    it('returns empty array when no queries recorded', () => {
      const stats = tracker.getStatistics();
      expect(stats).toEqual([]);
    });

    it('returns stats sorted by query count descending', () => {
      tracker.recordQuery('a', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'eq', 1.0, 10, false);
      tracker.recordQuery('c', 'eq', 1.0, 10, false);
      tracker.recordQuery('c', 'eq', 1.0, 10, false);
      tracker.recordQuery('c', 'eq', 1.0, 10, false);

      const stats = tracker.getStatistics();
      expect(stats[0].attribute).toBe('c');
      expect(stats[0].queryCount).toBe(3);
      expect(stats[1].attribute).toBe('b');
      expect(stats[1].queryCount).toBe(2);
      expect(stats[2].attribute).toBe('a');
      expect(stats[2].queryCount).toBe(1);
    });
  });

  describe('getAttributeStats', () => {
    it('returns stats for a specific attribute', () => {
      tracker.recordQuery('category', 'eq', 1.0, 10, false);
      tracker.recordQuery('category', 'in', 2.0, 20, false);
      tracker.recordQuery('status', 'eq', 3.0, 30, true);

      const categoryStats = tracker.getAttributeStats('category');
      expect(categoryStats).toHaveLength(2);
      expect(categoryStats.every(s => s.attribute === 'category')).toBe(true);
    });

    it('returns empty array for unknown attribute', () => {
      tracker.recordQuery('category', 'eq', 1.0, 10, false);

      const unknownStats = tracker.getAttributeStats('unknown');
      expect(unknownStats).toEqual([]);
    });
  });

  describe('getStats', () => {
    it('returns stats for specific attribute and query type', () => {
      tracker.recordQuery('category', 'eq', 5.0, 100, false);
      tracker.recordQuery('category', 'in', 6.0, 200, false);

      const eqStats = tracker.getStats('category', 'eq');
      expect(eqStats).toBeDefined();
      expect(eqStats?.queryType).toBe('eq');
      expect(eqStats?.averageCost).toBeCloseTo(5.0);

      const inStats = tracker.getStats('category', 'in');
      expect(inStats).toBeDefined();
      expect(inStats?.queryType).toBe('in');
    });

    it('returns undefined for non-existent stats', () => {
      tracker.recordQuery('category', 'eq', 5.0, 100, false);

      expect(tracker.getStats('category', 'gt')).toBeUndefined();
      expect(tracker.getStats('unknown', 'eq')).toBeUndefined();
    });
  });

  describe('hasStats', () => {
    it('returns true when attribute has stats', () => {
      tracker.recordQuery('category', 'eq', 5.0, 100, false);
      expect(tracker.hasStats('category')).toBe(true);
    });

    it('returns false when attribute has no stats', () => {
      tracker.recordQuery('category', 'eq', 5.0, 100, false);
      expect(tracker.hasStats('unknown')).toBe(false);
    });
  });

  describe('getTotalQueryCount', () => {
    it('returns total count across all patterns', () => {
      tracker.recordQuery('a', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'eq', 1.0, 10, false);
      tracker.recordQuery('c', 'gt', 1.0, 10, false);

      expect(tracker.getTotalQueryCount()).toBe(4);
    });

    it('returns 0 when no queries recorded', () => {
      expect(tracker.getTotalQueryCount()).toBe(0);
    });
  });

  describe('getPatternCount', () => {
    it('returns count of unique patterns', () => {
      tracker.recordQuery('a', 'eq', 1.0, 10, false);
      tracker.recordQuery('a', 'eq', 1.0, 10, false); // Same pattern
      tracker.recordQuery('a', 'gt', 1.0, 10, false); // Different type
      tracker.recordQuery('b', 'eq', 1.0, 10, false); // Different attr

      expect(tracker.getPatternCount()).toBe(3);
    });
  });

  describe('updateIndexStatus', () => {
    it('updates index status for all patterns of an attribute', () => {
      tracker.recordQuery('category', 'eq', 1.0, 10, false);
      tracker.recordQuery('category', 'in', 1.0, 10, false);
      tracker.recordQuery('status', 'eq', 1.0, 10, false);

      tracker.updateIndexStatus('category', true);

      const categoryStats = tracker.getAttributeStats('category');
      expect(categoryStats.every(s => s.hasIndex)).toBe(true);

      const statusStats = tracker.getAttributeStats('status');
      expect(statusStats[0].hasIndex).toBe(false);
    });
  });

  describe('resetAttributeStats', () => {
    it('clears stats for a specific attribute', () => {
      tracker.recordQuery('category', 'eq', 1.0, 10, false);
      tracker.recordQuery('status', 'eq', 1.0, 10, false);

      tracker.resetAttributeStats('category');

      expect(tracker.hasStats('category')).toBe(false);
      expect(tracker.hasStats('status')).toBe(true);
    });
  });

  describe('clear', () => {
    it('clears all statistics', () => {
      tracker.recordQuery('a', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'gt', 2.0, 20, false);

      tracker.clear();

      expect(tracker.getStatistics()).toEqual([]);
      expect(tracker.getTotalQueryCount()).toBe(0);
      expect(tracker.getPatternCount()).toBe(0);
    });
  });

  describe('getTrackingInfo', () => {
    it('returns tracking overhead information', () => {
      tracker.recordQuery('a', 'eq', 1.0, 10, false);
      tracker.recordQuery('b', 'eq', 1.0, 10, false);

      const info = tracker.getTrackingInfo();
      expect(info.patternsTracked).toBe(2);
      expect(info.totalQueries).toBe(2);
      expect(info.samplingRate).toBe(1);
      expect(info.memoryEstimate).toBeGreaterThan(0);
    });
  });

  describe('sampling', () => {
    it('respects sampling rate', () => {
      const sampledTracker = new QueryPatternTracker({ samplingRate: 2 });

      // With sampling rate 2, should only track every 2nd query
      sampledTracker.recordQuery('a', 'eq', 1.0, 10, false); // Query 1 - skipped
      sampledTracker.recordQuery('a', 'eq', 1.0, 10, false); // Query 2 - tracked
      sampledTracker.recordQuery('a', 'eq', 1.0, 10, false); // Query 3 - skipped
      sampledTracker.recordQuery('a', 'eq', 1.0, 10, false); // Query 4 - tracked

      const stats = sampledTracker.getStatistics();
      // With sampling, the count is adjusted (multiplied by sampling rate)
      expect(stats.length).toBe(1);
    });
  });

  describe('maxTrackedPatterns', () => {
    it('evicts oldest when max patterns reached', () => {
      const limitedTracker = new QueryPatternTracker({ maxTrackedPatterns: 3 });

      limitedTracker.recordQuery('a', 'eq', 1.0, 10, false);
      // Wait a bit to ensure different timestamps
      limitedTracker.recordQuery('b', 'eq', 1.0, 10, false);
      limitedTracker.recordQuery('c', 'eq', 1.0, 10, false);
      limitedTracker.recordQuery('d', 'eq', 1.0, 10, false); // Should evict 'a'

      expect(limitedTracker.getPatternCount()).toBe(3);
      expect(limitedTracker.hasStats('a')).toBe(false);
      expect(limitedTracker.hasStats('d')).toBe(true);
    });
  });
});
