/**
 * TokenFilter Tests
 */

import {
  LowercaseFilter,
  StopWordFilter,
  MinLengthFilter,
  MaxLengthFilter,
  TrimFilter,
  UniqueFilter,
  DEFAULT_STOP_WORDS,
} from '../../../query/tokenization/TokenFilter';

describe('LowercaseFilter', () => {
  const filter = new LowercaseFilter();

  test('should convert tokens to lowercase', () => {
    expect(filter.apply(['Hello', 'WORLD', 'TeSt'])).toEqual(['hello', 'world', 'test']);
  });

  test('should handle already lowercase tokens', () => {
    expect(filter.apply(['hello', 'world'])).toEqual(['hello', 'world']);
  });

  test('should handle empty array', () => {
    expect(filter.apply([])).toEqual([]);
  });

  test('should handle mixed case with numbers', () => {
    expect(filter.apply(['Test123', 'ABC123'])).toEqual(['test123', 'abc123']);
  });
});

describe('StopWordFilter', () => {
  test('should remove default stop words', () => {
    const filter = new StopWordFilter();
    expect(filter.apply(['the', 'quick', 'brown', 'fox'])).toEqual(['quick', 'brown', 'fox']);
  });

  test('should be case insensitive', () => {
    const filter = new StopWordFilter();
    expect(filter.apply(['THE', 'quick', 'Brown', 'FOX'])).toEqual(['quick', 'Brown', 'FOX']);
  });

  test('should use custom stop words', () => {
    const filter = new StopWordFilter(['custom', 'words']);
    expect(filter.apply(['custom', 'test', 'words', 'hello'])).toEqual(['test', 'hello']);
  });

  test('should handle empty array', () => {
    const filter = new StopWordFilter();
    expect(filter.apply([])).toEqual([]);
  });

  test('should remove all tokens if all are stop words', () => {
    const filter = new StopWordFilter();
    expect(filter.apply(['the', 'a', 'an', 'and'])).toEqual([]);
  });

  test('should expose stop words set', () => {
    const filter = new StopWordFilter(['a', 'b']);
    expect(filter.getStopWords()).toEqual(new Set(['a', 'b']));
  });

  test('DEFAULT_STOP_WORDS should contain common words', () => {
    expect(DEFAULT_STOP_WORDS).toContain('the');
    expect(DEFAULT_STOP_WORDS).toContain('a');
    expect(DEFAULT_STOP_WORDS).toContain('an');
    expect(DEFAULT_STOP_WORDS).toContain('and');
    expect(DEFAULT_STOP_WORDS).toContain('or');
    expect(DEFAULT_STOP_WORDS).toContain('is');
  });
});

describe('MinLengthFilter', () => {
  test('should remove tokens shorter than minLength', () => {
    const filter = new MinLengthFilter(3);
    expect(filter.apply(['a', 'is', 'the', 'quick'])).toEqual(['the', 'quick']);
  });

  test('should default to minLength=2', () => {
    const filter = new MinLengthFilter();
    expect(filter.apply(['a', 'is', 'the'])).toEqual(['is', 'the']);
  });

  test('should handle empty array', () => {
    const filter = new MinLengthFilter();
    expect(filter.apply([])).toEqual([]);
  });

  test('should handle all tokens too short', () => {
    const filter = new MinLengthFilter(5);
    expect(filter.apply(['a', 'is', 'the'])).toEqual([]);
  });

  test('should expose minLength setting', () => {
    const filter = new MinLengthFilter(4);
    expect(filter.getMinLength()).toBe(4);
  });

  test('should throw for invalid minLength', () => {
    expect(() => new MinLengthFilter(0)).toThrow('Minimum length must be at least 1');
    expect(() => new MinLengthFilter(-1)).toThrow('Minimum length must be at least 1');
  });
});

describe('MaxLengthFilter', () => {
  test('should remove tokens longer than maxLength', () => {
    const filter = new MaxLengthFilter(5);
    expect(filter.apply(['short', 'toolong', 'hi'])).toEqual(['short', 'hi']);
  });

  test('should default to maxLength=50', () => {
    const filter = new MaxLengthFilter();
    expect(filter.getMaxLength()).toBe(50);
  });

  test('should handle empty array', () => {
    const filter = new MaxLengthFilter();
    expect(filter.apply([])).toEqual([]);
  });

  test('should expose maxLength setting', () => {
    const filter = new MaxLengthFilter(10);
    expect(filter.getMaxLength()).toBe(10);
  });

  test('should throw for invalid maxLength', () => {
    expect(() => new MaxLengthFilter(0)).toThrow('Maximum length must be at least 1');
    expect(() => new MaxLengthFilter(-1)).toThrow('Maximum length must be at least 1');
  });
});

describe('TrimFilter', () => {
  const filter = new TrimFilter();

  test('should trim whitespace from tokens', () => {
    expect(filter.apply(['  hello  ', '  world  '])).toEqual(['hello', 'world']);
  });

  test('should remove empty tokens after trimming', () => {
    expect(filter.apply(['  ', 'hello', '   '])).toEqual(['hello']);
  });

  test('should handle already trimmed tokens', () => {
    expect(filter.apply(['hello', 'world'])).toEqual(['hello', 'world']);
  });

  test('should handle empty array', () => {
    expect(filter.apply([])).toEqual([]);
  });
});

describe('UniqueFilter', () => {
  const filter = new UniqueFilter();

  test('should remove duplicate tokens', () => {
    expect(filter.apply(['hello', 'world', 'hello', 'test', 'world'])).toEqual([
      'hello',
      'world',
      'test',
    ]);
  });

  test('should preserve first occurrence order', () => {
    const result = filter.apply(['c', 'a', 'b', 'a', 'c']);
    expect(result).toEqual(['c', 'a', 'b']);
  });

  test('should handle all unique tokens', () => {
    expect(filter.apply(['a', 'b', 'c'])).toEqual(['a', 'b', 'c']);
  });

  test('should handle all same tokens', () => {
    expect(filter.apply(['a', 'a', 'a'])).toEqual(['a']);
  });

  test('should handle empty array', () => {
    expect(filter.apply([])).toEqual([]);
  });
});
