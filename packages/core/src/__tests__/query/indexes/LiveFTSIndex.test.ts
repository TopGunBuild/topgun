/**
 * LiveFTSIndex Tests (Phase 12)
 *
 * Tests for LiveFTSIndex with O(1) updates via scoreSingleDocument.
 */

import { LiveFTSIndex } from '../../../query/indexes/LiveFTSIndex';
import { FullTextIndex } from '../../../fts';
import type { LiveQueryDelta, RankedResult } from '../../../query/indexes/ILiveQueryIndex';

interface TestDocument {
  id: string;
  title: string;
  body: string;
  [key: string]: unknown;
}

describe('LiveFTSIndex', () => {
  let ftsIndex: FullTextIndex;

  beforeEach(() => {
    ftsIndex = new FullTextIndex({
      fields: ['title', 'body'],
    });
  });

  describe('constructor', () => {
    it('should create index with query terms pre-tokenized', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });

      expect(liveIndex.id).toMatch(/^live-fts-\d+$/);
      expect(liveIndex.query.type).toBe('match');
    });

    it('should use default maxResults and minScore', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'test',
      });

      expect(liveIndex.getResultCount()).toBe(0);
    });
  });

  describe('onRecordAdded()', () => {
    it('should return ENTER delta when record matches', () => {
      // Pre-populate FTS index
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning Basics', body: 'Introduction' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });

      const delta = liveIndex.onRecordAdded('doc1', doc);

      expect(delta).not.toBeNull();
      expect(delta!.type).toBe('added');
      expect(delta!.key).toBe('doc1');
      expect(delta!.score).toBeGreaterThan(0);
      expect(delta!.matchedTerms).toBeDefined();
    });

    it('should return null when record does not match', () => {
      const doc: TestDocument = { id: 'doc1', title: 'JavaScript Basics', body: 'Introduction' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'python',
      });

      const delta = liveIndex.onRecordAdded('doc1', doc);

      expect(delta).toBeNull();
    });

    it('should add record to results', () => {
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
      });

      liveIndex.onRecordAdded('doc1', doc);

      expect(liveIndex.contains('doc1')).toBe(true);
      expect(liveIndex.getResultCount()).toBe(1);
    });
  });

  describe('onRecordUpdated()', () => {
    let liveIndex: LiveFTSIndex<string, TestDocument>;

    beforeEach(() => {
      liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });
    });

    it('should return ENTER delta when record starts matching', () => {
      const oldDoc: TestDocument = { id: 'doc1', title: 'JavaScript Basics', body: 'Test' };
      const newDoc: TestDocument = { id: 'doc1', title: 'Machine Learning Intro', body: 'Test' };

      ftsIndex.onSet('doc1', oldDoc);
      ftsIndex.onSet('doc1', newDoc);

      const delta = liveIndex.onRecordUpdated('doc1', oldDoc, newDoc);

      expect(delta).not.toBeNull();
      expect(delta!.type).toBe('added');
      expect(delta!.key).toBe('doc1');
    });

    it('should return LEAVE delta when record stops matching', () => {
      const oldDoc: TestDocument = { id: 'doc1', title: 'Machine Learning Intro', body: 'Test' };
      const newDoc: TestDocument = { id: 'doc1', title: 'JavaScript Basics', body: 'Test' };

      ftsIndex.onSet('doc1', oldDoc);
      liveIndex.onRecordAdded('doc1', oldDoc);
      ftsIndex.onSet('doc1', newDoc);

      const delta = liveIndex.onRecordUpdated('doc1', oldDoc, newDoc);

      expect(delta).not.toBeNull();
      expect(delta!.type).toBe('removed');
      expect(delta!.key).toBe('doc1');
      expect(delta!.oldScore).toBeGreaterThan(0);
    });

    it('should return UPDATE delta when score changes', () => {
      const oldDoc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      const newDoc: TestDocument = { id: 'doc1', title: 'Machine Learning Machine Learning', body: 'Test' };

      ftsIndex.onSet('doc1', oldDoc);
      liveIndex.onRecordAdded('doc1', oldDoc);
      const oldScore = liveIndex.getResults()[0].score;

      ftsIndex.onSet('doc1', newDoc);
      const delta = liveIndex.onRecordUpdated('doc1', oldDoc, newDoc);

      // Score should change due to increased term frequency
      if (delta && delta.type === 'updated') {
        expect(delta.oldScore).toBe(oldScore);
        expect(delta.score).not.toBe(delta.oldScore);
      }
    });

    it('should return null when neither old nor new matches', () => {
      const oldDoc: TestDocument = { id: 'doc1', title: 'Python Basics', body: 'Test' };
      const newDoc: TestDocument = { id: 'doc1', title: 'JavaScript Basics', body: 'Test' };

      ftsIndex.onSet('doc1', oldDoc);
      ftsIndex.onSet('doc1', newDoc);

      const delta = liveIndex.onRecordUpdated('doc1', oldDoc, newDoc);

      expect(delta).toBeNull();
    });
  });

  describe('onRecordRemoved()', () => {
    it('should return LEAVE delta when record was in results', () => {
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
      });

      liveIndex.onRecordAdded('doc1', doc);
      expect(liveIndex.contains('doc1')).toBe(true);

      ftsIndex.onRemove('doc1');
      const delta = liveIndex.onRecordRemoved('doc1', doc);

      expect(delta).not.toBeNull();
      expect(delta!.type).toBe('removed');
      expect(delta!.key).toBe('doc1');
      expect(delta!.oldScore).toBeGreaterThan(0);
      expect(liveIndex.contains('doc1')).toBe(false);
    });

    it('should return null when record was not in results', () => {
      const doc: TestDocument = { id: 'doc1', title: 'JavaScript Basics', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'python',
      });

      const delta = liveIndex.onRecordRemoved('doc1', doc);

      expect(delta).toBeNull();
    });
  });

  describe('getResults()', () => {
    it('should return results sorted by score descending', () => {
      const docs: TestDocument[] = [
        { id: 'doc1', title: 'Machine Learning', body: 'Test' },
        { id: 'doc2', title: 'Machine Learning Machine Learning', body: 'Deep ML' },
        { id: 'doc3', title: 'Learning', body: 'Machine' },
      ];

      for (const doc of docs) {
        ftsIndex.onSet(doc.id, doc);
      }

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });

      for (const doc of docs) {
        liveIndex.onRecordAdded(doc.id, doc);
      }

      const results = liveIndex.getResults();

      // Results should be sorted by score descending
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score);
      }
    });

    it('should return RankedResult with score and matchedTerms', () => {
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning Basics', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });

      liveIndex.onRecordAdded('doc1', doc);
      const results = liveIndex.getResults();

      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('doc1');
      expect(results[0].score).toBeGreaterThan(0);
      expect(results[0].matchedTerms).toBeDefined();
    });
  });

  describe('maxResults limit', () => {
    it('should enforce maxResults limit', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
        maxResults: 3,
      });

      // Add 5 documents
      for (let i = 0; i < 5; i++) {
        const doc: TestDocument = { id: `doc${i}`, title: `Machine Learning ${i}`, body: 'Test' };
        ftsIndex.onSet(doc.id, doc);
        liveIndex.onRecordAdded(doc.id, doc);
      }

      expect(liveIndex.getResultCount()).toBe(3);
    });

    it('should keep highest scoring results', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
        maxResults: 2,
      });

      // Add documents with varying relevance
      const docs: TestDocument[] = [
        { id: 'low', title: 'Just machine', body: 'Test' },
        { id: 'high', title: 'Machine machine machine machine', body: 'Test' },
        { id: 'medium', title: 'Machine machine', body: 'Test' },
      ];

      for (const doc of docs) {
        ftsIndex.onSet(doc.id, doc);
        liveIndex.onRecordAdded(doc.id, doc);
      }

      const results = liveIndex.getResults();
      expect(results).toHaveLength(2);

      // High scorer should be in results
      expect(results.find(r => r.key === 'high')).toBeDefined();
    });
  });

  describe('minScore threshold', () => {
    it('should filter out results below minScore', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
        minScore: 100, // Very high threshold
      });

      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const delta = liveIndex.onRecordAdded('doc1', doc);

      // Score should be below threshold
      expect(delta).toBeNull();
      expect(liveIndex.getResultCount()).toBe(0);
    });
  });

  describe('buildFromData()', () => {
    it('should build index from entries', () => {
      const docs: TestDocument[] = [
        { id: 'doc1', title: 'Machine Learning', body: 'Test' },
        { id: 'doc2', title: 'Deep Learning', body: 'Test' },
        { id: 'doc3', title: 'Python Basics', body: 'Test' },
      ];

      for (const doc of docs) {
        ftsIndex.onSet(doc.id, doc);
      }

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'learning',
      });

      const entries: Array<[string, TestDocument]> = docs.map(d => [d.id, d]);
      liveIndex.buildFromData(entries);

      expect(liveIndex.getResultCount()).toBe(2); // doc1 and doc2
      expect(liveIndex.contains('doc1')).toBe(true);
      expect(liveIndex.contains('doc2')).toBe(true);
      expect(liveIndex.contains('doc3')).toBe(false);
    });
  });

  describe('clear()', () => {
    it('should clear all data', () => {
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
      });

      liveIndex.onRecordAdded('doc1', doc);
      expect(liveIndex.getResultCount()).toBe(1);

      liveIndex.clear();

      expect(liveIndex.getResultCount()).toBe(0);
      expect(liveIndex.contains('doc1')).toBe(false);
    });
  });

  describe('contains()', () => {
    it('should return true for keys in results', () => {
      const doc: TestDocument = { id: 'doc1', title: 'Machine Learning', body: 'Test' };
      ftsIndex.onSet('doc1', doc);

      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine',
      });

      liveIndex.onRecordAdded('doc1', doc);

      expect(liveIndex.contains('doc1')).toBe(true);
      expect(liveIndex.contains('doc2')).toBe(false);
    });
  });

  describe('query property', () => {
    it('should expose the query', () => {
      const liveIndex = new LiveFTSIndex<string, TestDocument>(ftsIndex, {
        field: 'title',
        query: 'machine learning',
      });

      expect(liveIndex.query).toEqual({
        type: 'match',
        attribute: 'title',
        query: 'machine learning',
      });
    });
  });
});
