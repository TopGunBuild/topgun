/**
 * Tokenizer Interface and Implementations
 *
 * Provides text tokenization for InvertedIndex full-text search.
 * Tokenizers split text into searchable tokens.
 *
 * @module query/tokenization/Tokenizer
 */

/**
 * Interface for text tokenizers.
 * Tokenizers split text into an array of tokens (words, n-grams, etc.)
 */
export interface Tokenizer {
  /**
   * Split text into tokens.
   *
   * @param text - Text to tokenize
   * @returns Array of tokens
   */
  tokenize(text: string): string[];
}

/**
 * Tokenizer that splits on whitespace.
 * Simplest tokenizer - splits on any whitespace characters.
 *
 * Example: "hello world" → ["hello", "world"]
 */
export class WhitespaceTokenizer implements Tokenizer {
  tokenize(text: string): string[] {
    if (!text || typeof text !== 'string') {
      return [];
    }
    return text.split(/\s+/).filter((t) => t.length > 0);
  }
}

/**
 * Tokenizer that splits on word boundaries.
 * Splits on non-word characters (anything not [a-zA-Z0-9_]).
 *
 * Example: "hello-world! test123" → ["hello", "world", "test123"]
 */
export class WordBoundaryTokenizer implements Tokenizer {
  tokenize(text: string): string[] {
    if (!text || typeof text !== 'string') {
      return [];
    }
    return text.split(/\W+/).filter((t) => t.length > 0);
  }
}

/**
 * N-gram tokenizer for substring matching.
 * Creates overlapping character sequences of length n.
 *
 * Example (n=3): "hello" → ["hel", "ell", "llo"]
 *
 * Use cases:
 * - Fuzzy search (typo tolerance)
 * - Substring matching (contains anywhere)
 * - Partial word matching
 */
export class NGramTokenizer implements Tokenizer {
  /**
   * Create an N-gram tokenizer.
   *
   * @param n - Length of each n-gram (default: 3)
   */
  constructor(private readonly n: number = 3) {
    if (n < 1) {
      throw new Error('N-gram size must be at least 1');
    }
  }

  tokenize(text: string): string[] {
    if (!text || typeof text !== 'string') {
      return [];
    }

    // Remove whitespace for n-gram generation
    const normalized = text.replace(/\s+/g, ' ').trim();

    if (normalized.length < this.n) {
      // Text is shorter than n-gram size - return text itself if non-empty
      return normalized.length > 0 ? [normalized] : [];
    }

    const tokens: string[] = [];
    for (let i = 0; i <= normalized.length - this.n; i++) {
      tokens.push(normalized.substring(i, i + this.n));
    }

    return tokens;
  }

  /**
   * Get the n-gram size.
   */
  get size(): number {
    return this.n;
  }
}
