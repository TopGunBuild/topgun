import {
  SearchDebugger,
  getSearchDebugger,
  resetSearchDebugger,
  SearchDebugInfo,
} from '../../debug/SearchDebugger';

describe('SearchDebugger', () => {
  let debugger_: SearchDebugger;

  const createMockDebugInfo = (query: string): SearchDebugInfo => ({
    query,
    queryTokens: query.toLowerCase().split(' '),
    mapId: 'test-map',
    searchType: 'bm25',
    totalDocuments: 100,
    matchingDocuments: 10,
    results: [
      {
        docId: 'doc1',
        finalScore: 5.5,
        scoreBreakdown: {
          bm25: {
            score: 5.5,
            matchedTerms: ['test'],
            tf: { test: 0.5 },
            idf: { test: 2.0 },
            fieldWeights: { title: 2.0, content: 1.0 },
            avgDocLength: 100,
            docLength: 50,
            k1: 1.2,
            b: 0.75,
          },
        },
      },
      {
        docId: 'doc2',
        finalScore: 3.2,
        scoreBreakdown: {
          bm25: {
            score: 3.2,
            matchedTerms: ['test'],
            tf: { test: 0.3 },
            idf: { test: 2.0 },
            fieldWeights: { content: 1.0 },
            avgDocLength: 100,
            docLength: 150,
            k1: 1.2,
            b: 0.75,
          },
        },
      },
    ],
    timing: {
      tokenization: 0.5,
      indexLookup: 2.0,
      scoring: 5.0,
      ranking: 1.0,
      total: 8.5,
    },
    indexStats: {
      indexType: 'inverted',
      indexSize: 1000,
      termsSearched: 1,
    },
  });

  beforeEach(() => {
    resetSearchDebugger();
    debugger_ = new SearchDebugger({ enabled: true });
  });

  describe('control', () => {
    it('should be disabled by default without env', () => {
      const d = new SearchDebugger();
      expect(d.isEnabled()).toBe(false);
    });

    it('should enable/disable recording', () => {
      debugger_.disable();
      expect(debugger_.isEnabled()).toBe(false);

      debugger_.enable();
      expect(debugger_.isEnabled()).toBe(true);
    });

    it('should not record when disabled', () => {
      debugger_.disable();
      debugger_.recordSearch(createMockDebugInfo('test query'));

      expect(debugger_.getLastQuery()).toBeNull();
    });
  });

  describe('recording', () => {
    it('should record search debug info', () => {
      const info = createMockDebugInfo('test query');
      debugger_.recordSearch(info);

      const lastQuery = debugger_.getLastQuery();
      expect(lastQuery).not.toBeNull();
      expect(lastQuery?.query).toBe('test query');
      expect(lastQuery?.results).toHaveLength(2);
    });

    it('should maintain history', () => {
      debugger_.recordSearch(createMockDebugInfo('query 1'));
      debugger_.recordSearch(createMockDebugInfo('query 2'));
      debugger_.recordSearch(createMockDebugInfo('query 3'));

      const history = debugger_.getHistory();
      expect(history).toHaveLength(3);
    });

    it('should trim history when exceeding max', () => {
      const smallDebugger = new SearchDebugger({ enabled: true, maxHistory: 3 });

      for (let i = 0; i < 5; i++) {
        smallDebugger.recordSearch(createMockDebugInfo(`query ${i}`));
      }

      const history = smallDebugger.getHistory();
      expect(history).toHaveLength(3);
      expect(history[0].query).toBe('query 2');
    });
  });

  describe('querying', () => {
    beforeEach(() => {
      debugger_.recordSearch({
        ...createMockDebugInfo('users query'),
        mapId: 'users',
      });
      debugger_.recordSearch({
        ...createMockDebugInfo('posts query'),
        mapId: 'posts',
      });
      debugger_.recordSearch({
        ...createMockDebugInfo('another users query'),
        mapId: 'users',
      });
    });

    it('should get last query', () => {
      const last = debugger_.getLastQuery();
      expect(last?.query).toBe('another users query');
    });

    it('should filter history by map', () => {
      const userQueries = debugger_.getHistoryByMap('users');
      expect(userQueries).toHaveLength(2);
    });

    it('should explain result by docId', () => {
      const result = debugger_.explainResult('doc1');
      expect(result).toBeDefined();
      expect(result?.finalScore).toBe(5.5);
    });

    it('should return undefined for unknown docId', () => {
      const result = debugger_.explainResult('unknown');
      expect(result).toBeUndefined();
    });
  });

  describe('formatting', () => {
    beforeEach(() => {
      debugger_.recordSearch(createMockDebugInfo('test query'));
    });

    it('should format explanation for document', () => {
      const explanation = debugger_.formatExplanation('doc1');

      expect(explanation).toContain('Score Breakdown for doc1');
      expect(explanation).toContain('Final Score: 5.5000');
      expect(explanation).toContain('BM25 Full-Text Search');
      expect(explanation).toContain('TF=');
      expect(explanation).toContain('IDF=');
    });

    it('should handle unknown document gracefully', () => {
      const explanation = debugger_.formatExplanation('unknown');
      expect(explanation).toBe('No debug info available for this document.');
    });

    it('should format query summary', () => {
      const summary = debugger_.formatQuerySummary();

      expect(summary).toContain('Query: "test query"');
      expect(summary).toContain('Tokens: test, query');
      expect(summary).toContain('Type: bm25');
      expect(summary).toContain('Results: 10 of 100 documents');
      expect(summary).toContain('Timing:');
      expect(summary).toContain('Index stats:');
    });

    it('should format all results', () => {
      const formatted = debugger_.formatAllResults();

      expect(formatted).toContain('1. doc1');
      expect(formatted).toContain('2. doc2');
      expect(formatted).toContain('BM25:');
    });

    it('should handle no query recorded', () => {
      const emptyDebugger = new SearchDebugger({ enabled: true });
      expect(emptyDebugger.formatQuerySummary()).toBe('No query recorded.');
      expect(emptyDebugger.formatAllResults()).toBe('No query recorded.');
    });
  });

  describe('export', () => {
    it('should export debug info as JSON', () => {
      debugger_.recordSearch(createMockDebugInfo('test'));
      const json = debugger_.exportDebugInfo();
      const parsed = JSON.parse(json);

      expect(parsed.query).toBe('test');
    });

    it('should export history', () => {
      debugger_.recordSearch(createMockDebugInfo('query 1'));
      debugger_.recordSearch(createMockDebugInfo('query 2'));

      const json = debugger_.exportHistory();
      const parsed = JSON.parse(json);

      expect(parsed.version).toBe('1.0');
      expect(parsed.queryCount).toBe(2);
      expect(parsed.queries).toHaveLength(2);
    });
  });

  describe('statistics', () => {
    beforeEach(() => {
      debugger_.recordSearch(createMockDebugInfo('bm25 query'));
      debugger_.recordSearch({
        ...createMockDebugInfo('hybrid query'),
        searchType: 'hybrid',
        timing: { ...createMockDebugInfo('').timing, total: 15.0 },
      });
      debugger_.recordSearch({
        ...createMockDebugInfo('exact query'),
        searchType: 'exact',
        matchingDocuments: 5,
        timing: { ...createMockDebugInfo('').timing, total: 3.0 },
      });
    });

    it('should calculate search stats', () => {
      const stats = debugger_.getSearchStats();

      expect(stats.totalQueries).toBe(3);
      expect(stats.queryTypeBreakdown).toEqual({
        bm25: 1,
        hybrid: 1,
        exact: 1,
      });
      expect(stats.averageLatencyMs).toBeCloseTo((8.5 + 15.0 + 3.0) / 3, 1);
      expect(stats.averageResultCount).toBeCloseTo((10 + 10 + 5) / 3, 1);
    });

    it('should handle empty history', () => {
      const emptyDebugger = new SearchDebugger({ enabled: true });
      const stats = emptyDebugger.getSearchStats();

      expect(stats.totalQueries).toBe(0);
      expect(stats.averageResultCount).toBe(0);
      expect(stats.averageLatencyMs).toBe(0);
    });
  });

  describe('RRF and exact match formatting', () => {
    it('should format RRF breakdown', () => {
      const info: SearchDebugInfo = {
        ...createMockDebugInfo('test'),
        searchType: 'hybrid',
        results: [
          {
            docId: 'doc1',
            finalScore: 10.0,
            scoreBreakdown: {
              bm25: {
                score: 5.0,
                matchedTerms: ['test'],
                tf: { test: 0.5 },
                idf: { test: 2.0 },
                fieldWeights: {},
                avgDocLength: 100,
                docLength: 50,
                k1: 1.2,
                b: 0.75,
              },
              rrf: {
                rank: 1,
                score: 0.016,
                k: 60,
                contributingRanks: [
                  { source: 'bm25', rank: 1 },
                  { source: 'exact', rank: 2 },
                ],
              },
            },
          },
        ],
      };

      debugger_.recordSearch(info);
      const explanation = debugger_.formatExplanation('doc1');

      expect(explanation).toContain('Reciprocal Rank Fusion');
      expect(explanation).toContain('Final rank: 1');
      expect(explanation).toContain('k parameter: 60');
      expect(explanation).toContain('bm25: rank 1');
    });

    it('should format exact match breakdown', () => {
      const info: SearchDebugInfo = {
        ...createMockDebugInfo('test'),
        results: [
          {
            docId: 'doc1',
            finalScore: 10.0,
            scoreBreakdown: {
              exact: {
                score: 10.0,
                matchedFields: ['title', 'description'],
                boostApplied: 2.0,
              },
            },
          },
        ],
      };

      debugger_.recordSearch(info);
      const explanation = debugger_.formatExplanation('doc1');

      expect(explanation).toContain('Exact Match');
      expect(explanation).toContain('Score: 10.0000');
      expect(explanation).toContain('title, description');
      expect(explanation).toContain('Boost applied: 2x');
    });

    it('should format vector breakdown', () => {
      const info: SearchDebugInfo = {
        ...createMockDebugInfo('test'),
        searchType: 'vector',
        results: [
          {
            docId: 'doc1',
            finalScore: 0.95,
            scoreBreakdown: {
              vector: {
                score: 0.95,
                distance: 0.05,
                similarity: 'cosine',
              },
            },
          },
        ],
      };

      debugger_.recordSearch(info);
      const explanation = debugger_.formatExplanation('doc1');

      expect(explanation).toContain('Vector Similarity');
      expect(explanation).toContain('Score: 0.9500');
      expect(explanation).toContain('Distance: 0.0500');
      expect(explanation).toContain('Metric: cosine');
    });
  });

  describe('singleton', () => {
    it('should return same instance', () => {
      const d1 = getSearchDebugger();
      const d2 = getSearchDebugger();
      expect(d1).toBe(d2);
    });

    it('should reset singleton', () => {
      const d1 = getSearchDebugger();
      resetSearchDebugger();
      const d2 = getSearchDebugger();
      expect(d1).not.toBe(d2);
    });
  });

  describe('clear', () => {
    it('should clear all data', () => {
      debugger_.recordSearch(createMockDebugInfo('test'));

      debugger_.clear();

      expect(debugger_.getLastQuery()).toBeNull();
      expect(debugger_.getHistory()).toHaveLength(0);
    });
  });
});
