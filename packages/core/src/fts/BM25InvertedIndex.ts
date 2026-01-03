/**
 * FTS Inverted Index
 *
 * Data structure for full-text search that maps terms to documents.
 * Supports efficient term lookup, document frequency calculation,
 * and IDF (Inverse Document Frequency) for BM25 scoring.
 *
 * @module fts/BM25InvertedIndex
 */

import type { TermInfo } from './types';

/**
 * Inverted Index for Full-Text Search (BM25)
 *
 * Maps terms to the documents containing them, along with term frequency
 * information needed for BM25 scoring.
 *
 * @example
 * ```typescript
 * const index = new BM25InvertedIndex();
 * index.addDocument('doc1', ['hello', 'world']);
 * index.addDocument('doc2', ['hello', 'there']);
 *
 * const docs = index.getDocumentsForTerm('hello');
 * // [{ docId: 'doc1', termFrequency: 1 }, { docId: 'doc2', termFrequency: 1 }]
 * ```
 */
export class BM25InvertedIndex {
  /** term → list of documents containing term */
  private index: Map<string, TermInfo[]>;

  /** document → total term count (for length normalization) */
  private docLengths: Map<string, number>;

  /** document → set of terms (for efficient removal) */
  private docTerms: Map<string, Set<string>>;

  /** Inverse Document Frequency cache */
  private idfCache: Map<string, number>;

  /** Total number of documents */
  private totalDocs: number;

  /** Average document length */
  private avgDocLength: number;

  constructor() {
    this.index = new Map();
    this.docLengths = new Map();
    this.docTerms = new Map();
    this.idfCache = new Map();
    this.totalDocs = 0;
    this.avgDocLength = 0;
  }

  /**
   * Add a document to the index.
   *
   * @param docId - Unique document identifier
   * @param tokens - Array of tokens (already tokenized/stemmed)
   */
  addDocument(docId: string, tokens: string[]): void {
    // Count term frequencies
    const termFreqs = new Map<string, number>();
    const uniqueTerms = new Set<string>();

    for (const token of tokens) {
      termFreqs.set(token, (termFreqs.get(token) || 0) + 1);
      uniqueTerms.add(token);
    }

    // Update inverted index
    for (const [term, freq] of termFreqs) {
      if (!this.index.has(term)) {
        this.index.set(term, []);
      }
      this.index.get(term)!.push({
        docId,
        termFrequency: freq,
      });
    }

    // Store document info
    this.docLengths.set(docId, tokens.length);
    this.docTerms.set(docId, uniqueTerms);

    // Update stats
    this.totalDocs++;
    this.updateAvgDocLength();

    // Invalidate IDF cache
    this.idfCache.clear();
  }

  /**
   * Remove a document from the index.
   *
   * @param docId - Document identifier to remove
   */
  removeDocument(docId: string): void {
    const terms = this.docTerms.get(docId);
    if (!terms) {
      return; // Document doesn't exist
    }

    // Remove from inverted index
    for (const term of terms) {
      const termInfos = this.index.get(term);
      if (termInfos) {
        const filtered = termInfos.filter((info) => info.docId !== docId);
        if (filtered.length === 0) {
          this.index.delete(term);
        } else {
          this.index.set(term, filtered);
        }
      }
    }

    // Remove document info
    this.docLengths.delete(docId);
    this.docTerms.delete(docId);

    // Update stats
    this.totalDocs--;
    this.updateAvgDocLength();

    // Invalidate IDF cache
    this.idfCache.clear();
  }

  /**
   * Get all documents containing a term.
   *
   * @param term - Term to look up
   * @returns Array of TermInfo objects
   */
  getDocumentsForTerm(term: string): TermInfo[] {
    return this.index.get(term) || [];
  }

  /**
   * Calculate IDF (Inverse Document Frequency) for a term.
   *
   * Uses BM25 IDF formula:
   * IDF = log((N - df + 0.5) / (df + 0.5) + 1)
   *
   * Where:
   * - N = total documents
   * - df = document frequency (docs containing term)
   *
   * @param term - Term to calculate IDF for
   * @returns IDF value (0 if term doesn't exist)
   */
  getIDF(term: string): number {
    // Check cache first
    if (this.idfCache.has(term)) {
      return this.idfCache.get(term)!;
    }

    const termInfos = this.index.get(term);
    if (!termInfos || termInfos.length === 0) {
      return 0;
    }

    const docFreq = termInfos.length;

    // BM25 IDF formula
    const idf = Math.log((this.totalDocs - docFreq + 0.5) / (docFreq + 0.5) + 1);

    // Cache the result
    this.idfCache.set(term, idf);

    return idf;
  }

  /**
   * Get the length of a document (number of tokens).
   *
   * @param docId - Document identifier
   * @returns Document length (0 if not found)
   */
  getDocLength(docId: string): number {
    return this.docLengths.get(docId) || 0;
  }

  /**
   * Get the average document length.
   *
   * @returns Average length across all documents
   */
  getAvgDocLength(): number {
    return this.avgDocLength;
  }

  /**
   * Get the total number of documents in the index.
   *
   * @returns Total document count
   */
  getTotalDocs(): number {
    return this.totalDocs;
  }

  /**
   * Get iterator for document lengths (useful for serialization).
   *
   * @returns Iterator of [docId, length] pairs
   */
  getDocLengths(): IterableIterator<[string, number]> {
    return this.docLengths.entries();
  }

  /**
   * Get the number of documents in the index (alias for getTotalDocs).
   *
   * @returns Number of indexed documents
   */
  getSize(): number {
    return this.totalDocs;
  }

  /**
   * Clear all data from the index.
   */
  clear(): void {
    this.index.clear();
    this.docLengths.clear();
    this.docTerms.clear();
    this.idfCache.clear();
    this.totalDocs = 0;
    this.avgDocLength = 0;
  }

  /**
   * Check if a document exists in the index.
   *
   * @param docId - Document identifier
   * @returns True if document exists
   */
  hasDocument(docId: string): boolean {
    return this.docTerms.has(docId);
  }

  /**
   * Get all unique terms in the index.
   *
   * @returns Iterator of all terms
   */
  getTerms(): IterableIterator<string> {
    return this.index.keys();
  }

  /**
   * Get the number of unique terms in the index.
   *
   * @returns Number of unique terms
   */
  getTermCount(): number {
    return this.index.size;
  }

  /**
   * Update the average document length after add/remove.
   */
  private updateAvgDocLength(): void {
    if (this.totalDocs === 0) {
      this.avgDocLength = 0;
      return;
    }

    let sum = 0;
    for (const length of this.docLengths.values()) {
      sum += length;
    }
    this.avgDocLength = sum / this.totalDocs;
  }
}
