/**
 * Full-Text Index
 *
 * High-level integration class that combines Tokenizer, InvertedIndex,
 * and BM25Scorer for complete full-text search functionality.
 * Designed to integrate with TopGun's CRDT maps.
 *
 * @module fts/FullTextIndex
 */

import type {
  FullTextIndexConfig,
  SearchOptions,
  ScoredDocument,
  TokenizerOptions,
  BM25Options,
  SearchResult,
  SerializedIndex,
} from './types';
import { BM25Tokenizer } from './Tokenizer';
import { BM25InvertedIndex } from './BM25InvertedIndex';
import { BM25Scorer } from './BM25Scorer';
import { IndexSerializer } from './IndexSerializer';

/**
 * Full-Text Index for TopGun
 *
 * Provides BM25-based full-text search across document fields.
 * Supports incremental updates (add/update/remove) for real-time sync.
 *
 * @example
 * ```typescript
 * const index = new FullTextIndex({
 *   fields: ['title', 'body'],
 *   tokenizer: { minLength: 2 },
 *   bm25: { k1: 1.2, b: 0.75 }
 * });
 *
 * index.onSet('doc1', { title: 'Hello World', body: 'Test content' });
 * const results = index.search('hello');
 * // [{ docId: 'doc1', score: 0.5, matchedTerms: ['hello'] }]
 * ```
 */
export class FullTextIndex {
  /** Fields to index from documents */
  private readonly fields: string[];

  /** Tokenizer for text processing */
  private readonly tokenizer: BM25Tokenizer;

  /** BM25 scorer for relevance ranking */
  private readonly scorer: BM25Scorer;

  /** Per-field inverted indexes for field boosting */
  private readonly fieldIndexes: Map<string, BM25InvertedIndex>;

  /** Combined index for all fields */
  private combinedIndex: BM25InvertedIndex;

  /** Track indexed documents */
  private readonly indexedDocs: Set<string>;

  /** Serializer for persistence */
  private readonly serializer: IndexSerializer;

  /**
   * Cache of document tokens for fast single-document scoring.
   * Maps docId → tokenized terms from all indexed fields.
   */
  private readonly documentTokensCache: Map<string, string[]>;

  /**
   * Create a new FullTextIndex.
   *
   * @param config - Index configuration
   */
  constructor(config: FullTextIndexConfig) {
    this.fields = config.fields;
    this.tokenizer = new BM25Tokenizer(config.tokenizer);
    this.scorer = new BM25Scorer(config.bm25);
    this.fieldIndexes = new Map();
    this.combinedIndex = new BM25InvertedIndex();
    this.indexedDocs = new Set();
    this.serializer = new IndexSerializer();
    this.documentTokensCache = new Map();

    // Create per-field indexes
    for (const field of this.fields) {
      this.fieldIndexes.set(field, new BM25InvertedIndex());
    }
  }

  /**
   * Index a document (add or update).
   * Called when a document is set in the CRDT map.
   *
   * @param docId - Document identifier
   * @param document - Document data containing fields to index
   */
  onSet(docId: string, document: Record<string, unknown> | null | undefined): void {
    // Handle null/undefined documents
    if (!document || typeof document !== 'object') {
      // Clear cache for null/undefined document
      this.documentTokensCache.delete(docId);
      return;
    }

    // If document already exists, remove it first
    if (this.indexedDocs.has(docId)) {
      this.removeFromIndexes(docId);
    }

    // Collect all tokens for combined index
    const allTokens: string[] = [];

    // Index each field
    for (const field of this.fields) {
      const value = document[field];

      // Only index string values
      if (typeof value !== 'string') {
        continue;
      }

      const tokens = this.tokenizer.tokenize(value);

      if (tokens.length > 0) {
        // Add to field-specific index
        const fieldIndex = this.fieldIndexes.get(field)!;
        fieldIndex.addDocument(docId, tokens);

        // Collect for combined index
        allTokens.push(...tokens);
      }
    }

    // Add to combined index if any tokens were found
    if (allTokens.length > 0) {
      this.combinedIndex.addDocument(docId, allTokens);
      this.indexedDocs.add(docId);
      // Cache tokens for scoreSingleDocument
      this.documentTokensCache.set(docId, allTokens);
    } else {
      // No tokens - clear cache entry
      this.documentTokensCache.delete(docId);
    }
  }

  /**
   * Remove a document from the index.
   * Called when a document is deleted from the CRDT map.
   *
   * @param docId - Document identifier to remove
   */
  onRemove(docId: string): void {
    if (!this.indexedDocs.has(docId)) {
      return;
    }

    this.removeFromIndexes(docId);
    this.indexedDocs.delete(docId);
    // Clear cache entry
    this.documentTokensCache.delete(docId);
  }

  /**
   * Search the index with a query.
   *
   * @param query - Search query text
   * @param options - Search options (limit, minScore, boost)
   * @returns Array of search results, sorted by relevance
   */
  search(query: string, options?: SearchOptions): SearchResult[] {
    // Tokenize query
    const queryTerms = this.tokenizer.tokenize(query);

    if (queryTerms.length === 0) {
      return [];
    }

    // Check if field boosting is requested
    const boost = options?.boost;

    let results: ScoredDocument[];

    if (boost && Object.keys(boost).length > 0) {
      // Search with field boosting
      results = this.searchWithBoost(queryTerms, boost);
    } else {
      // Search combined index
      results = this.scorer.score(queryTerms, this.combinedIndex);
    }

    // Apply minScore filter
    if (options?.minScore !== undefined) {
      results = results.filter((r) => r.score >= options.minScore!);
    }

    // Apply limit
    if (options?.limit !== undefined && options.limit > 0) {
      results = results.slice(0, options.limit);
    }

    // Map to SearchResult
    return results.map((r) => ({
      docId: r.docId,
      score: r.score,
      matchedTerms: r.matchedTerms,
      source: 'fulltext' as const,
    }));
  }

  /**
   * Serialize the index state.
   *
   * @returns Serialized index data
   */
  serialize(): SerializedIndex {
    // We only serialize the combined index for now as it's the primary one.
    // If field boosting is required after restore, we'd need to serialize field indexes too.
    // For MVP/Phase 11, combined index serialization covers the main use case.
    return this.serializer.serialize(this.combinedIndex);
  }

  /**
   * Load index from serialized state.
   *
   * @param data - Serialized index data
   */
  load(data: SerializedIndex): void {
    this.combinedIndex = this.serializer.deserialize(data);

    // Rebuild indexedDocs set
    this.indexedDocs.clear();
    // Use private docLengths to rebuild indexedDocs set efficiently
    // This assumes we added getDocLengths to BM25InvertedIndex
    for (const [docId] of this.combinedIndex.getDocLengths()) {
      this.indexedDocs.add(docId);
    }

    // Note: Field indexes are NOT restored from combined index.
    // They would need to be rebuilt from source documents or serialized separately.
    // This is a tradeoff: fast load vs field boosting availability immediately without source docs.
    this.fieldIndexes.clear();
    for (const field of this.fields) {
      this.fieldIndexes.set(field, new BM25InvertedIndex());
    }

    // Clear document tokens cache - tokens must be rebuilt from source documents
    // This is intentional: serialized index doesn't include raw tokens
    this.documentTokensCache.clear();
  }

  /**
   * Build the index from an array of entries.
   * Useful for initial bulk loading.
   *
   * @param entries - Array of [docId, document] tuples
   */
  buildFromEntries(entries: Array<[string, Record<string, unknown> | null]>): void {
    for (const [docId, document] of entries) {
      this.onSet(docId, document);
    }
  }

  /**
   * Clear all data from the index.
   */
  clear(): void {
    this.combinedIndex.clear();
    for (const fieldIndex of this.fieldIndexes.values()) {
      fieldIndex.clear();
    }
    this.indexedDocs.clear();
    this.documentTokensCache.clear();
  }

  /**
   * Get the number of indexed documents.
   *
   * @returns Number of documents in the index
   */
  getSize(): number {
    return this.indexedDocs.size;
  }

  /**
   * Tokenize a query string using the index's tokenizer.
   * Public method for external use (e.g., SearchCoordinator).
   *
   * @param query - Query text to tokenize
   * @returns Array of tokenized terms
   */
  tokenizeQuery(query: string): string[] {
    return this.tokenizer.tokenize(query);
  }

  /**
   * Score a single document against query terms.
   * O(Q × D) complexity where Q = query terms, D = document tokens.
   *
   * This method is optimized for checking if a single document
   * matches a query, avoiding full index scan.
   *
   * @param docId - Document ID to score
   * @param queryTerms - Pre-tokenized query terms
   * @param document - Optional document data (used if not in cache)
   * @returns SearchResult with score and matched terms, or null if no match
   */
  scoreSingleDocument(
    docId: string,
    queryTerms: string[],
    document?: Record<string, unknown>
  ): SearchResult | null {
    if (queryTerms.length === 0) {
      return null;
    }

    // Get tokens from cache or compute from document
    let docTokens = this.documentTokensCache.get(docId);

    if (!docTokens && document) {
      // Document not in cache - tokenize on the fly
      docTokens = this.tokenizeDocument(document);
    }

    if (!docTokens || docTokens.length === 0) {
      return null;
    }

    // Quick check: any query term matches document?
    const docTokenSet = new Set(docTokens);
    const matchedTerms = queryTerms.filter(term => docTokenSet.has(term));

    if (matchedTerms.length === 0) {
      return null;
    }

    // Calculate BM25 score
    const score = this.scorer.scoreSingleDocument(
      queryTerms,
      docTokens,
      this.combinedIndex
    );

    if (score <= 0) {
      return null;
    }

    return {
      docId,
      score,
      matchedTerms,
      source: 'fulltext' as const,
    };
  }

  /**
   * Tokenize all indexed fields of a document.
   * Internal helper for scoreSingleDocument when document not in cache.
   *
   * @param document - Document data
   * @returns Array of all tokens from indexed fields
   */
  private tokenizeDocument(document: Record<string, unknown>): string[] {
    const allTokens: string[] = [];

    for (const field of this.fields) {
      const value = document[field];
      if (typeof value === 'string') {
        const tokens = this.tokenizer.tokenize(value);
        allTokens.push(...tokens);
      }
    }

    return allTokens;
  }

  /**
   * Get the index name (for debugging/display).
   *
   * @returns Descriptive name including indexed fields
   */
  get name(): string {
    return `FullTextIndex(${this.fields.join(', ')})`;
  }

  /**
   * Remove document from all indexes (internal).
   */
  private removeFromIndexes(docId: string): void {
    this.combinedIndex.removeDocument(docId);
    for (const fieldIndex of this.fieldIndexes.values()) {
      fieldIndex.removeDocument(docId);
    }
  }

  /**
   * Search with field boosting.
   * Scores are computed per-field and combined with boost weights.
   */
  private searchWithBoost(
    queryTerms: string[],
    boost: Record<string, number>
  ): ScoredDocument[] {
    // Accumulate scores per document
    const docScores = new Map<string, { score: number; terms: Set<string> }>();

    for (const field of this.fields) {
      const fieldIndex = this.fieldIndexes.get(field)!;
      const boostWeight = boost[field] ?? 1.0;

      // Score for this field
      const fieldResults = this.scorer.score(queryTerms, fieldIndex);

      for (const result of fieldResults) {
        const current = docScores.get(result.docId) || {
          score: 0,
          terms: new Set(),
        };

        // Apply boost to field score
        current.score += result.score * boostWeight;

        // Collect matched terms
        for (const term of result.matchedTerms) {
          current.terms.add(term);
        }

        docScores.set(result.docId, current);
      }
    }

    // Convert to array and sort
    const results: ScoredDocument[] = [];
    for (const [docId, { score, terms }] of docScores) {
      results.push({
        docId,
        score,
        matchedTerms: Array.from(terms),
      });
    }

    results.sort((a, b) => b.score - a.score);

    return results;
  }
}
