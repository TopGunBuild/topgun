/**
 * Token Filter Interface and Implementations
 *
 * Token filters transform or filter tokens produced by tokenizers.
 * Common uses: lowercase normalization, stop word removal, length filtering.
 *
 * @module query/tokenization/TokenFilter
 */

/**
 * Interface for token filters.
 * Filters transform an array of tokens into another array of tokens.
 */
export interface TokenFilter {
  /**
   * Apply filter to tokens.
   *
   * @param tokens - Input tokens
   * @returns Filtered tokens
   */
  apply(tokens: string[]): string[];
}

/**
 * Filter that converts all tokens to lowercase.
 * Essential for case-insensitive search.
 *
 * Example: ["Hello", "WORLD"] → ["hello", "world"]
 */
export class LowercaseFilter implements TokenFilter {
  apply(tokens: string[]): string[] {
    return tokens.map((t) => t.toLowerCase());
  }
}

/**
 * Default English stop words.
 * Common words that don't add search value.
 */
export const DEFAULT_STOP_WORDS = [
  'a',
  'an',
  'and',
  'are',
  'as',
  'at',
  'be',
  'but',
  'by',
  'for',
  'if',
  'in',
  'into',
  'is',
  'it',
  'no',
  'not',
  'of',
  'on',
  'or',
  'such',
  'that',
  'the',
  'their',
  'then',
  'there',
  'these',
  'they',
  'this',
  'to',
  'was',
  'will',
  'with',
];

/**
 * Filter that removes stop words.
 * Stop words are common words that don't contribute to search relevance.
 *
 * Example: ["the", "quick", "brown", "fox"] → ["quick", "brown", "fox"]
 */
export class StopWordFilter implements TokenFilter {
  private readonly stopWords: Set<string>;

  /**
   * Create a stop word filter.
   *
   * @param stopWords - Array of stop words to remove (default: English stop words)
   */
  constructor(stopWords: string[] = DEFAULT_STOP_WORDS) {
    // Store in lowercase for case-insensitive matching
    this.stopWords = new Set(stopWords.map((w) => w.toLowerCase()));
  }

  apply(tokens: string[]): string[] {
    return tokens.filter((t) => !this.stopWords.has(t.toLowerCase()));
  }

  /**
   * Get the set of stop words.
   */
  getStopWords(): Set<string> {
    return new Set(this.stopWords);
  }
}

/**
 * Filter that removes tokens shorter than a minimum length.
 * Useful for filtering out single characters or very short tokens.
 *
 * Example (minLength=3): ["a", "is", "the", "quick"] → ["the", "quick"]
 */
export class MinLengthFilter implements TokenFilter {
  /**
   * Create a minimum length filter.
   *
   * @param minLength - Minimum token length (default: 2)
   */
  constructor(private readonly minLength: number = 2) {
    if (minLength < 1) {
      throw new Error('Minimum length must be at least 1');
    }
  }

  apply(tokens: string[]): string[] {
    return tokens.filter((t) => t.length >= this.minLength);
  }

  /**
   * Get the minimum length setting.
   */
  getMinLength(): number {
    return this.minLength;
  }
}

/**
 * Filter that removes tokens longer than a maximum length.
 * Useful for preventing very long tokens from being indexed.
 *
 * Example (maxLength=10): ["short", "verylongword"] → ["short"]
 */
export class MaxLengthFilter implements TokenFilter {
  /**
   * Create a maximum length filter.
   *
   * @param maxLength - Maximum token length (default: 50)
   */
  constructor(private readonly maxLength: number = 50) {
    if (maxLength < 1) {
      throw new Error('Maximum length must be at least 1');
    }
  }

  apply(tokens: string[]): string[] {
    return tokens.filter((t) => t.length <= this.maxLength);
  }

  /**
   * Get the maximum length setting.
   */
  getMaxLength(): number {
    return this.maxLength;
  }
}

/**
 * Filter that trims whitespace from tokens.
 * Ensures clean tokens without leading/trailing spaces.
 */
export class TrimFilter implements TokenFilter {
  apply(tokens: string[]): string[] {
    return tokens.map((t) => t.trim()).filter((t) => t.length > 0);
  }
}

/**
 * Filter that removes duplicate tokens.
 * Useful for reducing index size when tokens repeat.
 *
 * Example: ["hello", "world", "hello"] → ["hello", "world"]
 */
export class UniqueFilter implements TokenFilter {
  apply(tokens: string[]): string[] {
    return [...new Set(tokens)];
  }
}
