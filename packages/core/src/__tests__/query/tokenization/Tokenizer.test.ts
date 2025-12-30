/**
 * Tokenizer Tests
 */

import {
  WhitespaceTokenizer,
  WordBoundaryTokenizer,
  NGramTokenizer,
} from '../../../query/tokenization/Tokenizer';

describe('WhitespaceTokenizer', () => {
  const tokenizer = new WhitespaceTokenizer();

  test('should split on single space', () => {
    expect(tokenizer.tokenize('hello world')).toEqual(['hello', 'world']);
  });

  test('should split on multiple spaces', () => {
    expect(tokenizer.tokenize('hello   world')).toEqual(['hello', 'world']);
  });

  test('should handle tabs and newlines', () => {
    expect(tokenizer.tokenize('hello\tworld\ntest')).toEqual(['hello', 'world', 'test']);
  });

  test('should handle leading and trailing whitespace', () => {
    expect(tokenizer.tokenize('  hello world  ')).toEqual(['hello', 'world']);
  });

  test('should return empty array for empty string', () => {
    expect(tokenizer.tokenize('')).toEqual([]);
  });

  test('should return empty array for whitespace only', () => {
    expect(tokenizer.tokenize('   ')).toEqual([]);
  });

  test('should handle null/undefined', () => {
    expect(tokenizer.tokenize(null as unknown as string)).toEqual([]);
    expect(tokenizer.tokenize(undefined as unknown as string)).toEqual([]);
  });

  test('should preserve punctuation within tokens', () => {
    expect(tokenizer.tokenize('hello-world foo.bar')).toEqual(['hello-world', 'foo.bar']);
  });
});

describe('WordBoundaryTokenizer', () => {
  const tokenizer = new WordBoundaryTokenizer();

  test('should split on word boundaries', () => {
    expect(tokenizer.tokenize('hello world')).toEqual(['hello', 'world']);
  });

  test('should split on punctuation', () => {
    expect(tokenizer.tokenize('hello-world')).toEqual(['hello', 'world']);
  });

  test('should split on multiple punctuation', () => {
    expect(tokenizer.tokenize('hello...world!!!')).toEqual(['hello', 'world']);
  });

  test('should handle mixed separators', () => {
    expect(tokenizer.tokenize('hello, world! test.')).toEqual(['hello', 'world', 'test']);
  });

  test('should preserve numbers in tokens', () => {
    expect(tokenizer.tokenize('test123 hello')).toEqual(['test123', 'hello']);
  });

  test('should preserve underscores in tokens', () => {
    expect(tokenizer.tokenize('hello_world test')).toEqual(['hello_world', 'test']);
  });

  test('should return empty array for empty string', () => {
    expect(tokenizer.tokenize('')).toEqual([]);
  });

  test('should return empty array for punctuation only', () => {
    expect(tokenizer.tokenize('...-!!!')).toEqual([]);
  });

  test('should handle null/undefined', () => {
    expect(tokenizer.tokenize(null as unknown as string)).toEqual([]);
    expect(tokenizer.tokenize(undefined as unknown as string)).toEqual([]);
  });
});

describe('NGramTokenizer', () => {
  test('should create n-grams of specified size', () => {
    const tokenizer = new NGramTokenizer(3);
    expect(tokenizer.tokenize('hello')).toEqual(['hel', 'ell', 'llo']);
  });

  test('should default to trigrams (n=3)', () => {
    const tokenizer = new NGramTokenizer();
    expect(tokenizer.tokenize('hello')).toEqual(['hel', 'ell', 'llo']);
  });

  test('should handle bigrams (n=2)', () => {
    const tokenizer = new NGramTokenizer(2);
    expect(tokenizer.tokenize('hello')).toEqual(['he', 'el', 'll', 'lo']);
  });

  test('should handle text shorter than n', () => {
    const tokenizer = new NGramTokenizer(5);
    expect(tokenizer.tokenize('hi')).toEqual(['hi']);
  });

  test('should handle text equal to n', () => {
    const tokenizer = new NGramTokenizer(5);
    expect(tokenizer.tokenize('hello')).toEqual(['hello']);
  });

  test('should normalize whitespace', () => {
    const tokenizer = new NGramTokenizer(3);
    expect(tokenizer.tokenize('a b')).toEqual(['a b']);
  });

  test('should return empty array for empty string', () => {
    const tokenizer = new NGramTokenizer(3);
    expect(tokenizer.tokenize('')).toEqual([]);
  });

  test('should handle null/undefined', () => {
    const tokenizer = new NGramTokenizer(3);
    expect(tokenizer.tokenize(null as unknown as string)).toEqual([]);
    expect(tokenizer.tokenize(undefined as unknown as string)).toEqual([]);
  });

  test('should throw for invalid n value', () => {
    expect(() => new NGramTokenizer(0)).toThrow('N-gram size must be at least 1');
    expect(() => new NGramTokenizer(-1)).toThrow('N-gram size must be at least 1');
  });

  test('should expose size property', () => {
    const tokenizer = new NGramTokenizer(4);
    expect(tokenizer.size).toBe(4);
  });
});
