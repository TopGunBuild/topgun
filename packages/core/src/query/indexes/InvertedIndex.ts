/**
 * InvertedIndex Implementation
 *
 * Full-text search index using inverted index structure.
 * Supports: contains, containsAll, containsAny, has queries.
 *
 * Structure:
 *   tokenIndex: Map<Token, Set<RecordKey>>
 *   reverseIndex: Map<RecordKey, Set<Token>>
 *
 * Retrieval cost: 50 (between NavigableIndex:40 and FallbackIndex)
 *
 * @module query/indexes/InvertedIndex
 */

import type { Attribute } from '../Attribute';
import type { Index, IndexQuery, IndexStats } from './types';
import type { ResultSet } from '../resultset/ResultSet';
import { SetResultSet } from '../resultset/SetResultSet';
import { TokenizationPipeline } from '../tokenization';

/**
 * Extended statistics for InvertedIndex.
 */
export interface InvertedIndexStats extends IndexStats {
  /** Total unique tokens in the index */
  totalTokens: number;
  /** Average number of tokens per indexed document */
  avgTokensPerDocument: number;
  /** Maximum documents for any single token */
  maxDocumentsPerToken: number;
}

/**
 * Inverted index for full-text search.
 * Maps tokens to sets of document keys for O(K) query performance.
 *
 * K = record key type, V = record value type, A = attribute value type (should be string)
 */
export class InvertedIndex<K, V, A extends string = string> implements Index<K, V, A> {
  readonly type = 'inverted' as const;

  /** Token → Set of keys */
  private tokenIndex: Map<string, Set<K>> = new Map();

  /** Key → Set of tokens (for efficient removal/update) */
  private reverseIndex: Map<K, Set<string>> = new Map();

  /** All keys with indexed content */
  private allKeys: Set<K> = new Set();

  private static readonly RETRIEVAL_COST = 50;
  private static readonly SUPPORTED_QUERIES = ['contains', 'containsAll', 'containsAny', 'has'];

  /**
   * Create an InvertedIndex.
   *
   * @param attribute - Attribute to index (should return string values)
   * @param pipeline - Tokenization pipeline (default: simple pipeline)
   */
  constructor(
    readonly attribute: Attribute<V, A>,
    private readonly pipeline: TokenizationPipeline = TokenizationPipeline.simple()
  ) {}

  getRetrievalCost(): number {
    return InvertedIndex.RETRIEVAL_COST;
  }

  supportsQuery(queryType: string): boolean {
    return InvertedIndex.SUPPORTED_QUERIES.includes(queryType);
  }

  retrieve(query: IndexQuery<A>): ResultSet<K> {
    switch (query.type) {
      case 'contains':
        return this.retrieveContains(query.value as A);
      case 'containsAll':
        return this.retrieveContainsAll(query.values as A[]);
      case 'containsAny':
        return this.retrieveContainsAny(query.values as A[]);
      case 'has':
        return this.retrieveHas();
      default:
        throw new Error(`InvertedIndex does not support query type: ${query.type}`);
    }
  }

  /**
   * Retrieve documents containing all tokens from the search text.
   * Uses AND semantics - document must contain ALL tokens.
   */
  private retrieveContains(searchText: A): ResultSet<K> {
    if (!searchText) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    const searchTokens = this.pipeline.process(String(searchText));
    if (searchTokens.length === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    // Sort tokens by frequency (ascending) for efficient intersection
    const sortedTokens = [...searchTokens].sort((a, b) => {
      const sizeA = this.tokenIndex.get(a)?.size ?? 0;
      const sizeB = this.tokenIndex.get(b)?.size ?? 0;
      return sizeA - sizeB;
    });

    // Start with smallest set
    const firstTokenKeys = this.tokenIndex.get(sortedTokens[0]);
    if (!firstTokenKeys || firstTokenKeys.size === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    const result = new Set(firstTokenKeys);

    // Intersect with remaining tokens
    for (let i = 1; i < sortedTokens.length; i++) {
      const tokenKeys = this.tokenIndex.get(sortedTokens[i]);
      if (!tokenKeys || tokenKeys.size === 0) {
        return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
      }

      for (const key of result) {
        if (!tokenKeys.has(key)) {
          result.delete(key);
        }
      }

      if (result.size === 0) {
        return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
      }
    }

    return new SetResultSet(result, InvertedIndex.RETRIEVAL_COST);
  }

  /**
   * Retrieve documents containing ALL specified values.
   * Each value is tokenized, and ALL resulting tokens must match.
   */
  private retrieveContainsAll(values: A[]): ResultSet<K> {
    if (!values || values.length === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    // Collect all tokens from all values
    const allTokens = new Set<string>();
    for (const value of values) {
      const tokens = this.pipeline.process(String(value));
      tokens.forEach((t) => allTokens.add(t));
    }

    if (allTokens.size === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    // Sort tokens by frequency for efficient intersection
    const sortedTokens = [...allTokens].sort((a, b) => {
      const sizeA = this.tokenIndex.get(a)?.size ?? 0;
      const sizeB = this.tokenIndex.get(b)?.size ?? 0;
      return sizeA - sizeB;
    });

    // Start with smallest set
    const firstTokenKeys = this.tokenIndex.get(sortedTokens[0]);
    if (!firstTokenKeys || firstTokenKeys.size === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    const result = new Set(firstTokenKeys);

    // Intersect with remaining tokens
    for (let i = 1; i < sortedTokens.length; i++) {
      const tokenKeys = this.tokenIndex.get(sortedTokens[i]);
      if (!tokenKeys || tokenKeys.size === 0) {
        return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
      }

      for (const key of result) {
        if (!tokenKeys.has(key)) {
          result.delete(key);
        }
      }

      if (result.size === 0) {
        return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
      }
    }

    return new SetResultSet(result, InvertedIndex.RETRIEVAL_COST);
  }

  /**
   * Retrieve documents containing ANY of the specified values.
   * Uses OR semantics - document can contain any token from any value.
   */
  private retrieveContainsAny(values: A[]): ResultSet<K> {
    if (!values || values.length === 0) {
      return new SetResultSet(new Set(), InvertedIndex.RETRIEVAL_COST);
    }

    const result = new Set<K>();

    for (const value of values) {
      const tokens = this.pipeline.process(String(value));
      for (const token of tokens) {
        const keys = this.tokenIndex.get(token);
        if (keys) {
          for (const key of keys) {
            result.add(key);
          }
        }
      }
    }

    return new SetResultSet(result, InvertedIndex.RETRIEVAL_COST);
  }

  /**
   * Retrieve all documents with indexed content.
   */
  private retrieveHas(): ResultSet<K> {
    return new SetResultSet(new Set(this.allKeys), InvertedIndex.RETRIEVAL_COST);
  }

  // ==================== Index Operations ====================

  add(key: K, record: V): void {
    const values = this.attribute.getValues(record);
    if (values.length === 0) return;

    const allTokens = new Set<string>();

    // Tokenize all attribute values
    for (const value of values) {
      if (value === undefined || value === null) continue;
      const tokens = this.pipeline.process(String(value));
      tokens.forEach((t) => allTokens.add(t));
    }

    if (allTokens.size === 0) return;

    // Add to token index
    for (const token of allTokens) {
      let keys = this.tokenIndex.get(token);
      if (!keys) {
        keys = new Set();
        this.tokenIndex.set(token, keys);
      }
      keys.add(key);
    }

    // Add to reverse index
    this.reverseIndex.set(key, allTokens);
    this.allKeys.add(key);
  }

  remove(key: K, _record: V): void {
    const tokens = this.reverseIndex.get(key);
    if (!tokens) return;

    // Remove from token index
    for (const token of tokens) {
      const keys = this.tokenIndex.get(token);
      if (keys) {
        keys.delete(key);
        if (keys.size === 0) {
          this.tokenIndex.delete(token);
        }
      }
    }

    // Remove from reverse index
    this.reverseIndex.delete(key);
    this.allKeys.delete(key);
  }

  update(key: K, oldRecord: V, newRecord: V): void {
    // Get old and new values
    const oldValues = this.attribute.getValues(oldRecord);
    const newValues = this.attribute.getValues(newRecord);

    // Quick check: if values are same, skip
    if (this.valuesEqual(oldValues, newValues)) {
      return;
    }

    // Full re-index
    this.remove(key, oldRecord);
    this.add(key, newRecord);
  }

  clear(): void {
    this.tokenIndex.clear();
    this.reverseIndex.clear();
    this.allKeys.clear();
  }

  // ==================== Statistics ====================

  getStats(): IndexStats {
    return {
      distinctValues: this.tokenIndex.size,
      totalEntries: this.allKeys.size,
      avgEntriesPerValue:
        this.tokenIndex.size > 0 ? this.allKeys.size / this.tokenIndex.size : 0,
    };
  }

  /**
   * Get extended statistics for full-text index.
   */
  getExtendedStats(): InvertedIndexStats {
    let maxDocuments = 0;
    let totalTokensPerDoc = 0;

    for (const keys of this.tokenIndex.values()) {
      if (keys.size > maxDocuments) {
        maxDocuments = keys.size;
      }
    }

    for (const tokens of this.reverseIndex.values()) {
      totalTokensPerDoc += tokens.size;
    }

    return {
      ...this.getStats(),
      totalTokens: this.tokenIndex.size,
      avgTokensPerDocument:
        this.reverseIndex.size > 0 ? totalTokensPerDoc / this.reverseIndex.size : 0,
      maxDocumentsPerToken: maxDocuments,
    };
  }

  /**
   * Get the tokenization pipeline.
   */
  getPipeline(): TokenizationPipeline {
    return this.pipeline;
  }

  /**
   * Check if a specific token exists in the index.
   */
  hasToken(token: string): boolean {
    return this.tokenIndex.has(token);
  }

  /**
   * Get the number of documents for a specific token.
   */
  getTokenDocumentCount(token: string): number {
    return this.tokenIndex.get(token)?.size ?? 0;
  }

  // ==================== Private Helpers ====================

  private valuesEqual(a: A[], b: A[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return false;
    }
    return true;
  }
}
