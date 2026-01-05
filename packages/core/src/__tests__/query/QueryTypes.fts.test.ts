/**
 * FTS Query Types Tests (Phase 12)
 *
 * Tests for FTS query node types and type guards.
 */

import {
  Query,
  MatchQueryNode,
  MatchPhraseQueryNode,
  MatchPrefixQueryNode,
  MatchQueryOptions,
  FTSQueryNode,
  isSimpleQuery,
  isLogicalQuery,
  isFTSQuery,
  isMatchQuery,
  isMatchPhraseQuery,
  isMatchPrefixQuery,
} from '../../query/QueryTypes';

describe('FTS Query Types', () => {
  describe('MatchQueryNode', () => {
    it('should create a valid match query node', () => {
      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'title',
        query: 'machine learning',
      };

      expect(query.type).toBe('match');
      expect(query.attribute).toBe('title');
      expect(query.query).toBe('machine learning');
    });

    it('should support options', () => {
      const options: MatchQueryOptions = {
        minScore: 1.0,
        boost: 2.0,
        operator: 'and',
        fuzziness: 1,
      };

      const query: MatchQueryNode = {
        type: 'match',
        attribute: 'body',
        query: 'neural networks',
        options,
      };

      expect(query.options).toEqual(options);
    });
  });

  describe('MatchPhraseQueryNode', () => {
    it('should create a valid match phrase query node', () => {
      const query: MatchPhraseQueryNode = {
        type: 'matchPhrase',
        attribute: 'body',
        query: 'machine learning',
      };

      expect(query.type).toBe('matchPhrase');
      expect(query.query).toBe('machine learning');
    });

    it('should support slop', () => {
      const query: MatchPhraseQueryNode = {
        type: 'matchPhrase',
        attribute: 'body',
        query: 'machine learning',
        slop: 2,
      };

      expect(query.slop).toBe(2);
    });
  });

  describe('MatchPrefixQueryNode', () => {
    it('should create a valid match prefix query node', () => {
      const query: MatchPrefixQueryNode = {
        type: 'matchPrefix',
        attribute: 'title',
        prefix: 'mach',
      };

      expect(query.type).toBe('matchPrefix');
      expect(query.prefix).toBe('mach');
    });

    it('should support maxExpansions', () => {
      const query: MatchPrefixQueryNode = {
        type: 'matchPrefix',
        attribute: 'title',
        prefix: 'mach',
        maxExpansions: 50,
      };

      expect(query.maxExpansions).toBe(50);
    });
  });

  describe('Type Guards', () => {
    describe('isFTSQuery()', () => {
      it('should return true for match query', () => {
        const query: Query = { type: 'match', attribute: 'title', query: 'test' };
        expect(isFTSQuery(query)).toBe(true);
      });

      it('should return true for matchPhrase query', () => {
        const query: Query = { type: 'matchPhrase', attribute: 'body', query: 'test phrase' };
        expect(isFTSQuery(query)).toBe(true);
      });

      it('should return true for matchPrefix query', () => {
        const query: Query = { type: 'matchPrefix', attribute: 'name', prefix: 'pre' };
        expect(isFTSQuery(query)).toBe(true);
      });

      it('should return false for simple query', () => {
        const query: Query = { type: 'eq', attribute: 'status', value: 'active' };
        expect(isFTSQuery(query)).toBe(false);
      });

      it('should return false for logical query', () => {
        const query: Query = { type: 'and', children: [] };
        expect(isFTSQuery(query)).toBe(false);
      });
    });

    describe('isMatchQuery()', () => {
      it('should return true for match query', () => {
        const query: Query = { type: 'match', attribute: 'title', query: 'test' };
        expect(isMatchQuery(query)).toBe(true);
      });

      it('should return false for matchPhrase query', () => {
        const query: Query = { type: 'matchPhrase', attribute: 'body', query: 'test' };
        expect(isMatchQuery(query)).toBe(false);
      });

      it('should return false for non-FTS query', () => {
        const query: Query = { type: 'eq', attribute: 'x', value: 1 };
        expect(isMatchQuery(query)).toBe(false);
      });
    });

    describe('isMatchPhraseQuery()', () => {
      it('should return true for matchPhrase query', () => {
        const query: Query = { type: 'matchPhrase', attribute: 'body', query: 'test phrase' };
        expect(isMatchPhraseQuery(query)).toBe(true);
      });

      it('should return false for match query', () => {
        const query: Query = { type: 'match', attribute: 'title', query: 'test' };
        expect(isMatchPhraseQuery(query)).toBe(false);
      });
    });

    describe('isMatchPrefixQuery()', () => {
      it('should return true for matchPrefix query', () => {
        const query: Query = { type: 'matchPrefix', attribute: 'name', prefix: 'pre' };
        expect(isMatchPrefixQuery(query)).toBe(true);
      });

      it('should return false for match query', () => {
        const query: Query = { type: 'match', attribute: 'title', query: 'test' };
        expect(isMatchPrefixQuery(query)).toBe(false);
      });
    });

    describe('isSimpleQuery()', () => {
      it('should return false for FTS queries', () => {
        const matchQuery: Query = { type: 'match', attribute: 'title', query: 'test' };
        const matchPhraseQuery: Query = { type: 'matchPhrase', attribute: 'body', query: 'test' };
        const matchPrefixQuery: Query = { type: 'matchPrefix', attribute: 'name', prefix: 'pre' };

        expect(isSimpleQuery(matchQuery)).toBe(false);
        expect(isSimpleQuery(matchPhraseQuery)).toBe(false);
        expect(isSimpleQuery(matchPrefixQuery)).toBe(false);
      });

      it('should return true for simple queries', () => {
        const eqQuery: Query = { type: 'eq', attribute: 'status', value: 'active' };
        expect(isSimpleQuery(eqQuery)).toBe(true);
      });
    });

    describe('isLogicalQuery()', () => {
      it('should return false for FTS queries', () => {
        const query: Query = { type: 'match', attribute: 'title', query: 'test' };
        expect(isLogicalQuery(query)).toBe(false);
      });
    });
  });

  describe('Query union type', () => {
    it('should accept FTS queries as Query type', () => {
      const queries: Query[] = [
        { type: 'eq', attribute: 'status', value: 'active' },
        { type: 'match', attribute: 'title', query: 'machine learning' },
        { type: 'matchPhrase', attribute: 'body', query: 'exact phrase' },
        { type: 'matchPrefix', attribute: 'name', prefix: 'pre' },
        { type: 'and', children: [] },
      ];

      expect(queries).toHaveLength(5);
    });

    it('should work in logical combinations', () => {
      const hybridQuery: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'published' },
          { type: 'match', attribute: 'body', query: 'machine learning' },
        ],
      };

      expect(hybridQuery.type).toBe('and');
      expect(hybridQuery.children).toHaveLength(2);
    });
  });
});
