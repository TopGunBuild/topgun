/**
 * Tests for Compound Index Auto-Detection
 *
 * Tests compound query pattern tracking and suggestions:
 * - QueryPatternTracker compound query recording
 * - IndexAdvisor compound index suggestions
 * - Cost/benefit analysis for compound indexes
 */

import { QueryPatternTracker } from '../../../query/adaptive/QueryPatternTracker';
import { IndexAdvisor } from '../../../query/adaptive/IndexAdvisor';

describe('Compound Query Pattern Tracking', () => {
  describe('recordCompoundQuery', () => {
    it('should track compound queries with 2 attributes', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(1);
      expect(stats[0].attributes).toEqual(['category', 'status']); // Sorted
      expect(stats[0].queryCount).toBe(1);
      expect(stats[0].averageCost).toBe(5.0);
    });

    it('should track compound queries with 3+ attributes', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['region', 'status', 'category'], 10.0, 50, false);

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(1);
      expect(stats[0].attributes).toEqual(['category', 'region', 'status']); // Sorted
    });

    it('should ignore single-attribute queries', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status'], 5.0, 100, false);

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(0);
    });

    it('should aggregate multiple queries for same combination', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 4.0, 100, false);
      tracker.recordCompoundQuery(['status', 'category'], 6.0, 100, false);
      tracker.recordCompoundQuery(['category', 'status'], 5.0, 100, false); // Same, different order

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(1);
      expect(stats[0].queryCount).toBe(3);
      expect(stats[0].averageCost).toBe(5.0); // (4 + 6 + 5) / 3
    });

    it('should track different combinations separately', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);
      tracker.recordCompoundQuery(['status', 'region'], 6.0, 50, false);

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(2);
    });

    it('should track hasCompoundIndex flag', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 0.5, 100, true);

      const stats = tracker.getCompoundStatistics();
      expect(stats[0].hasCompoundIndex).toBe(true);
    });
  });

  describe('getCompoundStats', () => {
    it('should retrieve specific compound stats', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);

      const stat = tracker.getCompoundStats(['status', 'category']);
      expect(stat).toBeDefined();
      expect(stat?.queryCount).toBe(1);

      // Order shouldn't matter for retrieval
      const stat2 = tracker.getCompoundStats(['category', 'status']);
      expect(stat2).toBeDefined();
    });

    it('should return undefined for non-existent combination', () => {
      const tracker = new QueryPatternTracker();

      const stat = tracker.getCompoundStats(['status', 'region']);
      expect(stat).toBeUndefined();
    });
  });

  describe('isInCompoundPattern', () => {
    it('should detect attributes in compound patterns', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);

      expect(tracker.isInCompoundPattern('status')).toBe(true);
      expect(tracker.isInCompoundPattern('category')).toBe(true);
      expect(tracker.isInCompoundPattern('region')).toBe(false);
    });
  });

  describe('updateCompoundIndexStatus', () => {
    it('should update compound index status', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);
      expect(tracker.getCompoundStats(['status', 'category'])?.hasCompoundIndex).toBe(false);

      tracker.updateCompoundIndexStatus(['status', 'category'], true);
      expect(tracker.getCompoundStats(['status', 'category'])?.hasCompoundIndex).toBe(true);
    });
  });

  describe('clear', () => {
    it('should clear compound statistics', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);
      tracker.clear();

      const stats = tracker.getCompoundStatistics();
      expect(stats.length).toBe(0);
    });
  });

  describe('getTrackingInfo', () => {
    it('should include compound patterns count', () => {
      const tracker = new QueryPatternTracker();

      tracker.recordCompoundQuery(['status', 'category'], 5.0, 100, false);
      tracker.recordCompoundQuery(['status', 'region'], 5.0, 100, false);

      const info = tracker.getTrackingInfo();
      expect(info.compoundPatternsTracked).toBe(2);
    });
  });
});

describe('IndexAdvisor Compound Suggestions', () => {
  describe('getCompoundSuggestions', () => {
    it('should suggest compound index for frequent AND queries', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // Record many compound queries
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions.length).toBe(1);
      expect(suggestions[0].indexType).toBe('compound');
      expect(suggestions[0].compoundAttributes).toEqual(['category', 'status']);
    });

    it('should not suggest when below query threshold', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // Record fewer queries than threshold
      for (let i = 0; i < 5; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions({ minQueryCount: 10 });
      expect(suggestions.length).toBe(0);
    });

    it('should not suggest when below cost threshold', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // Record queries with low cost
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 0.1, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions({ minAverageCost: 1 });
      expect(suggestions.length).toBe(0);
    });

    it('should exclude already indexed combinations', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // Record queries with existing compound index
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, true);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions.length).toBe(0);
    });

    it('should include indexed when excludeExistingIndexes is false', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, true);
      }

      const suggestions = advisor.getCompoundSuggestions({ excludeExistingIndexes: false });
      expect(suggestions.length).toBe(1);
    });
  });

  describe('getSuggestions (combined)', () => {
    it('should include compound suggestions in main suggestions', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // Record compound queries
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      const compoundSuggestions = suggestions.filter((s) => s.indexType === 'compound');
      expect(compoundSuggestions.length).toBe(1);
    });

    it('should sort compound suggestions with others by priority', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // High-priority compound query
      for (let i = 0; i < 200; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 15.0, 100, false);
      }

      // Lower-priority single attribute query
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('region', 'eq', 2.0, 50, false);
      }

      const suggestions = advisor.getSuggestions();
      // Compound should be high priority and come first
      expect(suggestions[0].indexType).toBe('compound');
    });
  });

  describe('getCompoundSuggestionFor', () => {
    it('should get suggestion for specific combination', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestion = advisor.getCompoundSuggestionFor(['status', 'category']);
      expect(suggestion).not.toBeNull();
      expect(suggestion?.indexType).toBe('compound');
    });

    it('should return null for non-tracked combination', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      const suggestion = advisor.getCompoundSuggestionFor(['status', 'region']);
      expect(suggestion).toBeNull();
    });
  });

  describe('shouldCreateCompoundIndex', () => {
    it('should return true when threshold exceeded', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 15; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      expect(advisor.shouldCreateCompoundIndex(['status', 'category'], 10)).toBe(true);
    });

    it('should return false when below threshold', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 5; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      expect(advisor.shouldCreateCompoundIndex(['status', 'category'], 10)).toBe(false);
    });

    it('should return false when already indexed', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 15; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, true);
      }

      expect(advisor.shouldCreateCompoundIndex(['status', 'category'], 10)).toBe(false);
    });
  });

  describe('suggestion properties', () => {
    it('should include compound attributes in suggestion', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category', 'region'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions[0].compoundAttributes).toEqual(['category', 'region', 'status']);
    });

    it('should use compound key as attribute name', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions[0].attribute).toBe('category+status'); // Sorted key
    });

    it('should estimate higher benefit for more attributes', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      // 2-attribute compound
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      // 3-attribute compound
      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category', 'region'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      const twoAttr = suggestions.find((s) => s.compoundAttributes?.length === 2);
      const threeAttr = suggestions.find((s) => s.compoundAttributes?.length === 3);

      expect(threeAttr!.estimatedBenefit).toBeGreaterThan(twoAttr!.estimatedBenefit);
    });

    it('should generate meaningful reason', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 20; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 10.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions[0].reason).toContain('Compound AND query');
      expect(suggestions[0].reason).toContain('category, status');
      expect(suggestions[0].reason).toContain('intersection');
    });
  });

  describe('priority calculation', () => {
    it('should assign high priority for frequent expensive queries', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 200; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 15.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions[0].priority).toBe('high');
    });

    it('should assign medium priority for moderate patterns', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 60; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 3.0, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions();
      expect(suggestions[0].priority).toBe('medium');
    });

    it('should assign low priority for infrequent cheap queries', () => {
      const tracker = new QueryPatternTracker();
      const advisor = new IndexAdvisor(tracker);

      for (let i = 0; i < 15; i++) {
        tracker.recordCompoundQuery(['status', 'category'], 0.5, 100, false);
      }

      const suggestions = advisor.getCompoundSuggestions({ minAverageCost: 0.1 });
      expect(suggestions[0].priority).toBe('low');
    });
  });
});
