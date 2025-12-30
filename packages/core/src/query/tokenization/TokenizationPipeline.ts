/**
 * Tokenization Pipeline
 *
 * Chains a tokenizer with multiple filters for text processing.
 * Provides factory methods for common configurations.
 *
 * @module query/tokenization/TokenizationPipeline
 */

import type { Tokenizer } from './Tokenizer';
import { WordBoundaryTokenizer } from './Tokenizer';
import type { TokenFilter } from './TokenFilter';
import { LowercaseFilter, MinLengthFilter, StopWordFilter } from './TokenFilter';

/**
 * Pipeline configuration options.
 */
export interface TokenizationPipelineOptions {
  /** Tokenizer to use */
  tokenizer: Tokenizer;
  /** Filters to apply (in order) */
  filters?: TokenFilter[];
}

/**
 * Tokenization pipeline that chains a tokenizer with filters.
 *
 * Processing order:
 * 1. Tokenizer splits text into tokens
 * 2. Each filter transforms the token array in sequence
 *
 * Example:
 * ```typescript
 * const pipeline = TokenizationPipeline.simple();
 * pipeline.process("Hello World!"); // ["hello", "world"]
 * ```
 */
export class TokenizationPipeline {
  private readonly tokenizer: Tokenizer;
  private readonly filters: TokenFilter[];

  /**
   * Create a tokenization pipeline.
   *
   * @param options - Pipeline configuration
   */
  constructor(options: TokenizationPipelineOptions) {
    this.tokenizer = options.tokenizer;
    this.filters = options.filters ?? [];
  }

  /**
   * Process text through the pipeline.
   *
   * @param text - Text to process
   * @returns Array of processed tokens
   */
  process(text: string): string[] {
    if (!text || typeof text !== 'string') {
      return [];
    }

    // Step 1: Tokenize
    let tokens = this.tokenizer.tokenize(text);

    // Step 2: Apply filters in order
    for (const filter of this.filters) {
      tokens = filter.apply(tokens);
    }

    return tokens;
  }

  /**
   * Get the tokenizer.
   */
  getTokenizer(): Tokenizer {
    return this.tokenizer;
  }

  /**
   * Get the filters.
   */
  getFilters(): TokenFilter[] {
    return [...this.filters];
  }

  // ==================== Factory Methods ====================

  /**
   * Create a simple pipeline with common defaults.
   * Uses word boundary tokenizer with lowercase and minimum length filters.
   *
   * Configuration:
   * - Tokenizer: WordBoundaryTokenizer
   * - Filters: LowercaseFilter, MinLengthFilter(2)
   *
   * @returns Simple tokenization pipeline
   */
  static simple(): TokenizationPipeline {
    return new TokenizationPipeline({
      tokenizer: new WordBoundaryTokenizer(),
      filters: [new LowercaseFilter(), new MinLengthFilter(2)],
    });
  }

  /**
   * Create a pipeline optimized for search.
   * Includes stop word removal for better search relevance.
   *
   * Configuration:
   * - Tokenizer: WordBoundaryTokenizer
   * - Filters: LowercaseFilter, MinLengthFilter(2), StopWordFilter
   *
   * @returns Search-optimized tokenization pipeline
   */
  static search(): TokenizationPipeline {
    return new TokenizationPipeline({
      tokenizer: new WordBoundaryTokenizer(),
      filters: [new LowercaseFilter(), new MinLengthFilter(2), new StopWordFilter()],
    });
  }

  /**
   * Create a minimal pipeline with just tokenization and lowercase.
   * No filtering - preserves all tokens.
   *
   * Configuration:
   * - Tokenizer: WordBoundaryTokenizer
   * - Filters: LowercaseFilter
   *
   * @returns Minimal tokenization pipeline
   */
  static minimal(): TokenizationPipeline {
    return new TokenizationPipeline({
      tokenizer: new WordBoundaryTokenizer(),
      filters: [new LowercaseFilter()],
    });
  }

  /**
   * Create a custom pipeline from a tokenizer and filters.
   *
   * @param tokenizer - Tokenizer to use
   * @param filters - Filters to apply
   * @returns Custom tokenization pipeline
   */
  static custom(tokenizer: Tokenizer, filters: TokenFilter[] = []): TokenizationPipeline {
    return new TokenizationPipeline({ tokenizer, filters });
  }
}
