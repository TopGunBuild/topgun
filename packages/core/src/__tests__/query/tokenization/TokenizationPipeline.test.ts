/**
 * TokenizationPipeline Tests
 */

import { TokenizationPipeline } from '../../../query/tokenization/TokenizationPipeline';
import {
  WhitespaceTokenizer,
  WordBoundaryTokenizer,
  NGramTokenizer,
} from '../../../query/tokenization/Tokenizer';
import {
  LowercaseFilter,
  StopWordFilter,
  MinLengthFilter,
} from '../../../query/tokenization/TokenFilter';

describe('TokenizationPipeline', () => {
  describe('constructor and basic usage', () => {
    test('should create pipeline with tokenizer only', () => {
      const pipeline = new TokenizationPipeline({
        tokenizer: new WhitespaceTokenizer(),
      });
      expect(pipeline.process('Hello World')).toEqual(['Hello', 'World']);
    });

    test('should create pipeline with tokenizer and filters', () => {
      const pipeline = new TokenizationPipeline({
        tokenizer: new WhitespaceTokenizer(),
        filters: [new LowercaseFilter()],
      });
      expect(pipeline.process('Hello World')).toEqual(['hello', 'world']);
    });

    test('should apply filters in order', () => {
      const pipeline = new TokenizationPipeline({
        tokenizer: new WordBoundaryTokenizer(),
        filters: [new LowercaseFilter(), new MinLengthFilter(3), new StopWordFilter()],
      });
      // "The quick brown fox" → ["the", "quick", "brown", "fox"]
      // → lowercase: ["the", "quick", "brown", "fox"]
      // → minLength(3): ["the", "quick", "brown", "fox"]
      // → stopWords: ["quick", "brown", "fox"]
      expect(pipeline.process('The quick brown fox')).toEqual(['quick', 'brown', 'fox']);
    });
  });

  describe('process method', () => {
    test('should return empty array for empty string', () => {
      const pipeline = TokenizationPipeline.simple();
      expect(pipeline.process('')).toEqual([]);
    });

    test('should return empty array for null/undefined', () => {
      const pipeline = TokenizationPipeline.simple();
      expect(pipeline.process(null as unknown as string)).toEqual([]);
      expect(pipeline.process(undefined as unknown as string)).toEqual([]);
    });

    test('should handle unicode text', () => {
      const pipeline = new TokenizationPipeline({
        tokenizer: new WhitespaceTokenizer(),
        filters: [new LowercaseFilter()],
      });
      expect(pipeline.process('Привет мир')).toEqual(['привет', 'мир']);
    });
  });

  describe('getters', () => {
    test('should expose tokenizer', () => {
      const tokenizer = new WhitespaceTokenizer();
      const pipeline = new TokenizationPipeline({ tokenizer });
      expect(pipeline.getTokenizer()).toBe(tokenizer);
    });

    test('should expose filters copy', () => {
      const filters = [new LowercaseFilter(), new MinLengthFilter()];
      const pipeline = new TokenizationPipeline({
        tokenizer: new WhitespaceTokenizer(),
        filters,
      });
      const result = pipeline.getFilters();
      expect(result).toHaveLength(2);
      expect(result).not.toBe(filters); // Should be a copy
    });
  });

  describe('factory methods', () => {
    describe('simple()', () => {
      test('should create pipeline with WordBoundary + Lowercase + MinLength(2)', () => {
        const pipeline = TokenizationPipeline.simple();
        // "Hello World A" → ["Hello", "World", "A"] → ["hello", "world", "a"] → ["hello", "world"]
        expect(pipeline.process('Hello World A')).toEqual(['hello', 'world']);
      });

      test('should handle punctuation', () => {
        const pipeline = TokenizationPipeline.simple();
        expect(pipeline.process('hello, world!')).toEqual(['hello', 'world']);
      });
    });

    describe('search()', () => {
      test('should create pipeline with stop word removal', () => {
        const pipeline = TokenizationPipeline.search();
        // Removes "the" and "a"
        expect(pipeline.process('the quick brown fox')).toEqual(['quick', 'brown', 'fox']);
      });

      test('should remove common stop words', () => {
        const pipeline = TokenizationPipeline.search();
        expect(pipeline.process('this is a test and it works')).toEqual(['test', 'works']);
      });
    });

    describe('minimal()', () => {
      test('should create pipeline with just lowercase', () => {
        const pipeline = TokenizationPipeline.minimal();
        // Should not remove short tokens or stop words
        expect(pipeline.process('A is B')).toEqual(['a', 'is', 'b']);
      });
    });

    describe('custom()', () => {
      test('should create custom pipeline', () => {
        const tokenizer = new NGramTokenizer(2);
        const filters = [new LowercaseFilter()];
        const pipeline = TokenizationPipeline.custom(tokenizer, filters);
        expect(pipeline.process('HI')).toEqual(['hi']);
      });

      test('should work without filters', () => {
        const tokenizer = new WhitespaceTokenizer();
        const pipeline = TokenizationPipeline.custom(tokenizer);
        expect(pipeline.process('Hello World')).toEqual(['Hello', 'World']);
      });
    });
  });

  describe('real-world examples', () => {
    test('product search tokenization', () => {
      const pipeline = TokenizationPipeline.search();

      expect(pipeline.process('Wireless Gaming Mouse')).toEqual(['wireless', 'gaming', 'mouse']);

      expect(pipeline.process('The Best USB-C Hub for MacBook')).toEqual([
        'best',
        'usb',
        'hub',
        'macbook',
      ]);
    });

    test('user search tokenization', () => {
      const pipeline = TokenizationPipeline.simple();

      expect(pipeline.process('John Smith')).toEqual(['john', 'smith']);
      expect(pipeline.process('Dr. Jane Doe, PhD')).toEqual(['dr', 'jane', 'doe', 'phd']);
    });

    test('log message tokenization', () => {
      const pipeline = TokenizationPipeline.simple();

      expect(pipeline.process('Connection timeout error')).toEqual([
        'connection',
        'timeout',
        'error',
      ]);

      expect(pipeline.process('Failed to connect to 192.168.1.1')).toEqual([
        'failed',
        'to',
        'connect',
        'to',
        '192',
        '168',
      ]);
    });
  });
});
