/**
 * Full-Text Search Types
 *
 * Type definitions for the FTS (Full-Text Search) module.
 * This module provides BM25-based keyword search capabilities.
 *
 * @module fts/types
 */

/**
 * Options for configuring the FTS Tokenizer.
 */
export interface TokenizerOptions {
  /**
   * Convert all text to lowercase before tokenization.
   * @default true
   */
  lowercase?: boolean;

  /**
   * Set of words to exclude from tokenization (e.g., "the", "and", "is").
   * @default ENGLISH_STOPWORDS (174 words)
   */
  stopwords?: Set<string>;

  /**
   * Function to reduce words to their root form.
   * @default Porter stemmer
   */
  stemmer?: (word: string) => string;

  /**
   * Minimum token length to include in results.
   * @default 2
   */
  minLength?: number;

  /**
   * Maximum token length to include in results.
   * @default 40
   */
  maxLength?: number;
}

/**
 * Information about a term's occurrence in a document.
 */
export interface TermInfo {
  /** Document ID where the term appears */
  docId: string;

  /** Number of times the term appears in the document */
  termFrequency: number;

  /** Optional: positions of the term for phrase search (future) */
  fieldPositions?: number[];
}

/**
 * Posting list entry for the inverted index.
 */
export interface Posting {
  /** Document ID */
  docId: string;

  /** Term frequency in this document */
  termFrequency: number;

  /** Optional: positions for phrase search */
  positions?: number[];
}

/**
 * BM25 algorithm configuration options.
 */
export interface BM25Options {
  /**
   * Term frequency saturation parameter.
   * Higher values give more weight to repeated terms.
   * @default 1.2
   */
  k1?: number;

  /**
   * Document length normalization parameter.
   * 0 = no length normalization, 1 = full normalization.
   * @default 0.75
   */
  b?: number;
}

/**
 * A document with its BM25 relevance score.
 */
export interface ScoredDocument {
  /** Document ID */
  docId: string;

  /** BM25 relevance score */
  score: number;

  /** Terms from the query that matched this document */
  matchedTerms: string[];
}

/**
 * Configuration for a FullTextIndex.
 */
export interface FullTextIndexConfig {
  /** Fields to index for full-text search (e.g., ['title', 'body']) */
  fields: string[];

  /** Tokenizer configuration */
  tokenizer?: TokenizerOptions;

  /** BM25 scoring parameters */
  bm25?: BM25Options;
}

/**
 * Options for search queries.
 */
export interface SearchOptions {
  /** Maximum number of results to return */
  limit?: number;

  /** Minimum BM25 score threshold */
  minScore?: number;

  /** Restrict search to specific fields */
  fields?: string[];

  /** Field boost weights (e.g., { title: 2.0, body: 1.0 }) */
  boost?: Record<string, number>;
}

/**
 * Search result with full details.
 */
export interface SearchResult {
  /** Document ID */
  docId: string;

  /** BM25 relevance score */
  score: number;

  /** Source of the match (for hybrid search) */
  source: 'exact' | 'fulltext' | 'vector' | 'bfs';

  /** Original document data (if requested) */
  data?: unknown;

  /** Highlighted text snippet (if requested) */
  highlightedText?: string;

  /** Terms that matched */
  matchedTerms?: string[];

  /** Debug information */
  debug?: {
    exactScore?: number;
    fulltextScore?: number;
    vectorScore?: number;
    rangeScore?: number;
  };
}

/**
 * Serialized format for index persistence.
 */
export interface SerializedIndex {
  /** Schema version for backwards compatibility */
  version: number;

  /** Index metadata */
  metadata: {
    totalDocs: number;
    avgDocLength: number;
    createdAt: number;
    lastModified: number;
  };

  /** Serialized term data */
  terms: Array<{
    term: string;
    idf: number;
    postings: Array<{
      docId: string;
      termFrequency: number;
      positions?: number[];
    }>;
  }>;

  /** Document lengths for BM25 normalization */
  docLengths: Record<string, number>;
}
