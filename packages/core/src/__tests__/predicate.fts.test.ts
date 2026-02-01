/**
 * Full-Text Search Predicate Tests
 *
 * Tests for FTS predicate builders and types.
 */

import { Predicates, PredicateNode, MatchOptions } from '../predicate';

describe('FTS Predicates', () => {
  describe('Predicates.match()', () => {
    it('should create a match predicate with query', () => {
      const predicate = Predicates.match('title', 'machine learning');

      expect(predicate.op).toBe('match');
      expect(predicate.attribute).toBe('title');
      expect(predicate.query).toBe('machine learning');
      expect(predicate.matchOptions).toBeUndefined();
    });

    it('should create a match predicate with options', () => {
      const options: MatchOptions = {
        minScore: 1.5,
        boost: 2.0,
        operator: 'and',
        fuzziness: 1,
      };

      const predicate = Predicates.match('body', 'neural networks', options);

      expect(predicate.op).toBe('match');
      expect(predicate.attribute).toBe('body');
      expect(predicate.query).toBe('neural networks');
      expect(predicate.matchOptions).toEqual(options);
    });

    it('should handle empty query', () => {
      const predicate = Predicates.match('title', '');

      expect(predicate.op).toBe('match');
      expect(predicate.query).toBe('');
    });

    it('should handle partial options', () => {
      const predicate = Predicates.match('title', 'test', { boost: 1.5 });

      expect(predicate.matchOptions).toEqual({ boost: 1.5 });
    });
  });

  describe('Predicates.matchPhrase()', () => {
    it('should create a matchPhrase predicate', () => {
      const predicate = Predicates.matchPhrase('body', 'machine learning');

      expect(predicate.op).toBe('matchPhrase');
      expect(predicate.attribute).toBe('body');
      expect(predicate.query).toBe('machine learning');
      expect(predicate.slop).toBeUndefined();
    });

    it('should create a matchPhrase predicate with slop', () => {
      const predicate = Predicates.matchPhrase('body', 'machine learning', 2);

      expect(predicate.op).toBe('matchPhrase');
      expect(predicate.query).toBe('machine learning');
      expect(predicate.slop).toBe(2);
    });

    it('should handle zero slop', () => {
      const predicate = Predicates.matchPhrase('title', 'exact phrase', 0);

      expect(predicate.slop).toBe(0);
    });
  });

  describe('Predicates.matchPrefix()', () => {
    it('should create a matchPrefix predicate', () => {
      const predicate = Predicates.matchPrefix('title', 'mach');

      expect(predicate.op).toBe('matchPrefix');
      expect(predicate.attribute).toBe('title');
      expect(predicate.prefix).toBe('mach');
      expect(predicate.maxExpansions).toBeUndefined();
    });

    it('should create a matchPrefix predicate with maxExpansions', () => {
      const predicate = Predicates.matchPrefix('title', 'mach', 50);

      expect(predicate.op).toBe('matchPrefix');
      expect(predicate.prefix).toBe('mach');
      expect(predicate.maxExpansions).toBe(50);
    });

    it('should handle single character prefix', () => {
      const predicate = Predicates.matchPrefix('name', 'a');

      expect(predicate.prefix).toBe('a');
    });
  });

  describe('Predicates.multiMatch()', () => {
    it('should create OR composition of match predicates', () => {
      const predicate = Predicates.multiMatch(
        ['title', 'body'],
        'machine learning'
      );

      expect(predicate.op).toBe('or');
      expect(predicate.children).toHaveLength(2);

      const [titleMatch, bodyMatch] = predicate.children!;
      expect(titleMatch.op).toBe('match');
      expect(titleMatch.attribute).toBe('title');
      expect(titleMatch.query).toBe('machine learning');

      expect(bodyMatch.op).toBe('match');
      expect(bodyMatch.attribute).toBe('body');
      expect(bodyMatch.query).toBe('machine learning');
    });

    it('should apply per-field boost', () => {
      const predicate = Predicates.multiMatch(
        ['title', 'body', 'summary'],
        'test query',
        { boost: { title: 2.0, summary: 1.5 } }
      );

      expect(predicate.op).toBe('or');
      expect(predicate.children).toHaveLength(3);

      const [titleMatch, bodyMatch, summaryMatch] = predicate.children!;

      // title has boost 2.0
      expect(titleMatch.matchOptions).toEqual({ boost: 2.0 });

      // body has no boost specified
      expect(bodyMatch.matchOptions).toBeUndefined();

      // summary has boost 1.5
      expect(summaryMatch.matchOptions).toEqual({ boost: 1.5 });
    });

    it('should handle single field', () => {
      const predicate = Predicates.multiMatch(['title'], 'query');

      expect(predicate.op).toBe('or');
      expect(predicate.children).toHaveLength(1);
    });

    it('should handle empty fields array', () => {
      const predicate = Predicates.multiMatch([], 'query');

      expect(predicate.op).toBe('or');
      expect(predicate.children).toHaveLength(0);
    });

    it('should handle no boost options', () => {
      const predicate = Predicates.multiMatch(['title', 'body'], 'query', {});

      expect(predicate.op).toBe('or');
      expect(predicate.children).toHaveLength(2);

      // No boost should be applied
      predicate.children!.forEach((child) => {
        expect(child.matchOptions).toBeUndefined();
      });
    });
  });

  describe('FTS predicates with existing predicates', () => {
    it('should combine FTS with AND', () => {
      const combined = Predicates.and(
        Predicates.equal('status', 'published'),
        Predicates.match('body', 'machine learning')
      );

      expect(combined.op).toBe('and');
      expect(combined.children).toHaveLength(2);
      expect(combined.children![0].op).toBe('eq');
      expect(combined.children![1].op).toBe('match');
    });

    it('should combine FTS with OR', () => {
      const combined = Predicates.or(
        Predicates.match('title', 'machine'),
        Predicates.match('body', 'learning')
      );

      expect(combined.op).toBe('or');
      expect(combined.children).toHaveLength(2);
    });

    it('should negate FTS predicate', () => {
      const negated = Predicates.not(Predicates.match('title', 'spam'));

      expect(negated.op).toBe('not');
      expect(negated.children![0].op).toBe('match');
    });

    it('should create complex hybrid query', () => {
      // WHERE status = 'active' AND (title MATCH 'machine' OR body MATCH 'learning')
      const query = Predicates.and(
        Predicates.equal('status', 'active'),
        Predicates.or(
          Predicates.match('title', 'machine', { boost: 2.0 }),
          Predicates.match('body', 'learning')
        )
      );

      expect(query.op).toBe('and');
      expect(query.children).toHaveLength(2);

      const orClause = query.children![1];
      expect(orClause.op).toBe('or');
      expect(orClause.children).toHaveLength(2);
    });
  });

  describe('PredicateOp type', () => {
    it('should include FTS operators', () => {
      // Type check - these should compile
      const matchNode: PredicateNode = { op: 'match', attribute: 'title', query: 'test' };
      const matchPhraseNode: PredicateNode = { op: 'matchPhrase', attribute: 'body', query: 'test phrase' };
      const matchPrefixNode: PredicateNode = { op: 'matchPrefix', attribute: 'name', prefix: 'pre' };

      expect(matchNode.op).toBe('match');
      expect(matchPhraseNode.op).toBe('matchPhrase');
      expect(matchPrefixNode.op).toBe('matchPrefix');
    });
  });
});
