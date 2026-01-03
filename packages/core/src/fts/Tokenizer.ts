/**
 * FTS Tokenizer with Porter Stemming and Stopwords
 *
 * Provides text tokenization for BM25 full-text search.
 * Features:
 * - Unicode-aware word boundary detection
 * - English stopwords filtering (174 words)
 * - Porter stemming algorithm for word normalization
 * - Configurable min/max token length
 *
 * @module fts/Tokenizer
 */

import type { Tokenizer as ITokenizer } from '../query/tokenization/Tokenizer';
import type { TokenizerOptions } from './types';
import { ENGLISH_STOPWORDS } from '../query/tokenization/stopwords';
import { porterStem } from '../query/tokenization/porter-stemmer';

export { ENGLISH_STOPWORDS, porterStem };

/**
 * FTS Tokenizer
 *
 * Splits text into searchable tokens with normalization.
 *
 * @example
 * ```typescript
 * const tokenizer = new BM25Tokenizer();
 * const tokens = tokenizer.tokenize('The quick brown foxes');
 * // ['quick', 'brown', 'fox']
 * ```
 */
export class BM25Tokenizer implements ITokenizer {
  private readonly options: Required<TokenizerOptions>;

  /**
   * Create a new BM25Tokenizer.
   *
   * @param options - Configuration options
   */
  constructor(options?: TokenizerOptions) {
    this.options = {
      lowercase: true,
      stopwords: ENGLISH_STOPWORDS,
      stemmer: porterStem,
      minLength: 2,
      maxLength: 40,
      ...options,
    };
  }

  /**
   * Tokenize text into an array of normalized tokens.
   *
   * @param text - Text to tokenize
   * @returns Array of tokens
   */
  tokenize(text: string): string[] {
    // Handle null/undefined/empty
    if (!text || typeof text !== 'string') {
      return [];
    }

    // 1. Lowercase if enabled
    let processed = this.options.lowercase ? text.toLowerCase() : text;

    // 2. Split on non-alphanumeric characters (Unicode-aware)
    // This regex matches Unicode letters and numbers
    const words = processed.split(/[^\p{L}\p{N}]+/u).filter((w) => w.length > 0);

    // 3. Filter and process each word
    const tokens: string[] = [];

    for (const word of words) {
      // Skip if too short before any processing
      if (word.length < this.options.minLength) {
        continue;
      }

      // Skip stopwords (before stemming)
      if (this.options.stopwords.has(word)) {
        continue;
      }

      // Apply stemmer
      const stemmed = this.options.stemmer(word);

      // Skip if too short after stemming
      if (stemmed.length < this.options.minLength) {
        continue;
      }

      // Skip if too long
      if (stemmed.length > this.options.maxLength) {
        continue;
      }

      tokens.push(stemmed);
    }

    return tokens;
  }
}
