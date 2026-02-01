/**
 * Tests for IndexAdvisor
 */

import { IndexAdvisor } from '../IndexAdvisor';
import { QueryPatternTracker } from '../QueryPatternTracker';

describe('IndexAdvisor', () => {
  let tracker: QueryPatternTracker;
  let advisor: IndexAdvisor;

  beforeEach(() => {
    tracker = new QueryPatternTracker();
    advisor = new IndexAdvisor(tracker);
  });

  describe('getSuggestions', () => {
    it('returns empty array when no queries recorded', () => {
      const suggestions = advisor.getSuggestions();
      expect(suggestions).toEqual([]);
    });

    it('suggests hash index for equality queries', () => {
      // Record enough queries to trigger suggestion (high priority requires > 100 queries AND > 10ms cost)
      for (let i = 0; i < 150; i++) {
        tracker.recordQuery('category', 'eq', 15.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions).toHaveLength(1);
      expect(suggestions[0].attribute).toBe('category');
      expect(suggestions[0].indexType).toBe('hash');
      expect(suggestions[0].priority).toBe('high');
    });

    it('suggests navigable index for range queries', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('price', 'gt', 15.0, 200, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions).toHaveLength(1);
      expect(suggestions[0].attribute).toBe('price');
      expect(suggestions[0].indexType).toBe('navigable');
    });

    it('suggests inverted index for text search queries', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('description', 'contains', 20.0, 50, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions).toHaveLength(1);
      expect(suggestions[0].attribute).toBe('description');
      expect(suggestions[0].indexType).toBe('inverted');
    });

    it('respects minQueryCount threshold', () => {
      // Record 5 queries (below default threshold of 10)
      for (let i = 0; i < 5; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, false);
      }

      const suggestions = advisor.getSuggestions({ minQueryCount: 10 });
      expect(suggestions).toHaveLength(0);

      // Now it should pass with lower threshold
      const suggestionsWith5 = advisor.getSuggestions({ minQueryCount: 5 });
      expect(suggestionsWith5).toHaveLength(1);
    });

    it('respects minAverageCost threshold', () => {
      // Fast queries (0.5ms average)
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 0.5, 100, false);
      }

      // With default minAverageCost of 1ms, should not suggest
      const suggestions = advisor.getSuggestions({ minAverageCost: 1 });
      expect(suggestions).toHaveLength(0);

      // Should suggest with lower threshold
      const suggestionsWithLower = advisor.getSuggestions({ minAverageCost: 0.1 });
      expect(suggestionsWithLower).toHaveLength(1);
    });

    it('excludes already indexed attributes', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, true); // hasIndex = true
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions).toHaveLength(0);

      // Should include with excludeExistingIndexes = false
      const suggestionsIncluding = advisor.getSuggestions({ excludeExistingIndexes: false });
      expect(suggestionsIncluding).toHaveLength(1);
    });

    it('limits suggestions count', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('a', 'eq', 10.0, 100, false);
        tracker.recordQuery('b', 'eq', 10.0, 100, false);
        tracker.recordQuery('c', 'eq', 10.0, 100, false);
      }

      const suggestions = advisor.getSuggestions({ maxSuggestions: 2 });
      expect(suggestions).toHaveLength(2);
    });

    it('sorts suggestions by priority and benefit', () => {
      // High priority: many queries, high cost
      for (let i = 0; i < 200; i++) {
        tracker.recordQuery('highPriority', 'eq', 20.0, 100, false);
      }

      // Medium priority: moderate queries
      for (let i = 0; i < 60; i++) {
        tracker.recordQuery('mediumPriority', 'eq', 5.0, 100, false);
      }

      // Low priority: few queries, low cost
      for (let i = 0; i < 15; i++) {
        tracker.recordQuery('lowPriority', 'eq', 2.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions[0].attribute).toBe('highPriority');
      expect(suggestions[0].priority).toBe('high');
      expect(suggestions[1].attribute).toBe('mediumPriority');
      expect(suggestions[1].priority).toBe('medium');
      expect(suggestions[2].attribute).toBe('lowPriority');
      expect(suggestions[2].priority).toBe('low');
    });

    it('includes estimated benefit and cost', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 1000, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions[0].estimatedBenefit).toBeGreaterThan(0);
      expect(suggestions[0].estimatedCost).toBeGreaterThan(0);
      expect(suggestions[0].reason).toContain('Queried');
    });

    it('deduplicates suggestions per attribute', () => {
      // Record multiple query types for same attribute
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, false);
        tracker.recordQuery('category', 'in', 12.0, 80, false);
      }

      const suggestions = advisor.getSuggestions();
      // Should only have one suggestion for 'category'
      expect(suggestions.filter(s => s.attribute === 'category')).toHaveLength(1);
    });
  });

  describe('getSuggestionForAttribute', () => {
    it('returns suggestion for specific attribute', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, false);
        tracker.recordQuery('status', 'eq', 10.0, 100, false);
      }

      const suggestion = advisor.getSuggestionForAttribute('category');
      expect(suggestion).not.toBeNull();
      expect(suggestion?.attribute).toBe('category');
    });

    it('returns null for non-queried attribute', () => {
      const suggestion = advisor.getSuggestionForAttribute('unknown');
      expect(suggestion).toBeNull();
    });

    it('returns null for already indexed attribute', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, true);
      }

      const suggestion = advisor.getSuggestionForAttribute('category');
      expect(suggestion).toBeNull();
    });
  });

  describe('shouldIndex', () => {
    it('returns true when threshold is reached', () => {
      for (let i = 0; i < 10; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, false);
      }

      expect(advisor.shouldIndex('category', 10)).toBe(true);
    });

    it('returns false when below threshold', () => {
      for (let i = 0; i < 5; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, false);
      }

      expect(advisor.shouldIndex('category', 10)).toBe(false);
    });

    it('returns false for indexed attribute', () => {
      for (let i = 0; i < 20; i++) {
        tracker.recordQuery('category', 'eq', 10.0, 100, true);
      }

      expect(advisor.shouldIndex('category', 10)).toBe(false);
    });

    it('returns false for unknown attribute', () => {
      expect(advisor.shouldIndex('unknown', 10)).toBe(false);
    });
  });

  describe('getRecommendedIndexType', () => {
    it('recommends hash for equality queries', () => {
      expect(advisor.getRecommendedIndexType('eq')).toBe('hash');
      expect(advisor.getRecommendedIndexType('neq')).toBe('hash');
      expect(advisor.getRecommendedIndexType('in')).toBe('hash');
      expect(advisor.getRecommendedIndexType('has')).toBe('hash');
    });

    it('recommends navigable for range queries', () => {
      expect(advisor.getRecommendedIndexType('gt')).toBe('navigable');
      expect(advisor.getRecommendedIndexType('gte')).toBe('navigable');
      expect(advisor.getRecommendedIndexType('lt')).toBe('navigable');
      expect(advisor.getRecommendedIndexType('lte')).toBe('navigable');
      expect(advisor.getRecommendedIndexType('between')).toBe('navigable');
    });

    it('recommends inverted for text search queries', () => {
      expect(advisor.getRecommendedIndexType('contains')).toBe('inverted');
      expect(advisor.getRecommendedIndexType('containsAll')).toBe('inverted');
      expect(advisor.getRecommendedIndexType('containsAny')).toBe('inverted');
    });
  });

  describe('priority calculation', () => {
    it('assigns high priority for frequently queried expensive fields', () => {
      for (let i = 0; i < 150; i++) {
        tracker.recordQuery('hotField', 'eq', 15.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions[0].priority).toBe('high');
    });

    it('assigns medium priority for moderate usage', () => {
      for (let i = 0; i < 60; i++) {
        tracker.recordQuery('warmField', 'eq', 5.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions[0].priority).toBe('medium');
    });

    it('assigns low priority for occasional queries', () => {
      for (let i = 0; i < 12; i++) {
        tracker.recordQuery('coldField', 'eq', 2.0, 100, false);
      }

      const suggestions = advisor.getSuggestions();
      expect(suggestions[0].priority).toBe('low');
    });
  });
});
