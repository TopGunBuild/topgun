/**
 * Full-Text Search Module
 *
 * Provides BM25-based full-text search capabilities for TopGun.
 * Features:
 * - Tokenizer with Porter stemming and stopwords filtering
 * - Inverted index for fast term lookup
 * - BM25 scorer for relevance ranking
 *
 * @module fts
 */

// Types
export type {
  TokenizerOptions,
  TermInfo,
  Posting,
  BM25Options,
  ScoredDocument,
  FullTextIndexConfig,
  SearchOptions,
  SearchResult,
  SerializedIndex,
} from './types';

// Tokenizer
export { BM25Tokenizer as Tokenizer, ENGLISH_STOPWORDS, porterStem } from './Tokenizer';

// Inverted Index
export { BM25InvertedIndex as InvertedIndex } from './BM25InvertedIndex';

// BM25 Scorer
export { BM25Scorer } from './BM25Scorer';

// Full-Text Index (high-level integration)
export { FullTextIndex } from './FullTextIndex';
