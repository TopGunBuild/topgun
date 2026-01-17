/**
 * BM25 scoring debug information.
 */
export interface BM25DebugInfo {
  score: number;
  matchedTerms: string[];
  tf: Record<string, number>; // Term frequency per term
  idf: Record<string, number>; // Inverse document frequency per term
  fieldWeights: Record<string, number>;
  avgDocLength: number;
  docLength: number;
  k1: number;
  b: number;
}

/**
 * Exact match debug information.
 */
export interface ExactMatchDebugInfo {
  score: number;
  matchedFields: string[];
  boostApplied: number;
}

/**
 * Reciprocal Rank Fusion (RRF) debug information.
 */
export interface RRFDebugInfo {
  rank: number;
  score: number;
  k: number; // RRF constant (default 60)
  contributingRanks: {
    source: string; // 'bm25', 'exact', 'vector'
    rank: number;
  }[];
}

/**
 * Vector similarity debug information.
 */
export interface VectorDebugInfo {
  score: number;
  distance: number;
  similarity: 'cosine' | 'euclidean' | 'dot';
}

/**
 * Debug information for a single search result.
 */
export interface SearchResultDebug {
  docId: string;
  finalScore: number;
  scoreBreakdown: {
    bm25?: BM25DebugInfo;
    exact?: ExactMatchDebugInfo;
    rrf?: RRFDebugInfo;
    vector?: VectorDebugInfo;
  };
  matchedDocument?: Record<string, unknown>;
}

/**
 * Index statistics for a search operation.
 */
export interface SearchIndexStats {
  indexType: string;
  indexSize: number;
  termsSearched: number;
}

/**
 * Timing information for search operations.
 */
export interface SearchTiming {
  tokenization: number;
  indexLookup: number;
  scoring: number;
  ranking: number;
  fusion?: number;
  total: number;
}

/**
 * Complete debug information for a search query.
 */
export interface SearchDebugInfo {
  query: string;
  queryTokens: string[];
  mapId: string;
  searchType: 'bm25' | 'exact' | 'hybrid' | 'vector';
  totalDocuments: number;
  matchingDocuments: number;
  results: SearchResultDebug[];
  timing: SearchTiming;
  indexStats: SearchIndexStats;
}

/**
 * SearchDebugger - Records and analyzes search operations for debugging.
 *
 * Features:
 * - Record search queries with full debug info
 * - BM25 score breakdown per term
 * - RRF fusion explanation
 * - Timing breakdown
 * - Query history
 *
 * @see PHASE_14C_OBSERVABILITY.md for specification
 */
export class SearchDebugger {
  private enabled: boolean;
  private lastQuery: SearchDebugInfo | null = null;
  private history: SearchDebugInfo[] = [];
  private maxHistory: number;

  constructor(options: { enabled?: boolean; maxHistory?: number } = {}) {
    this.enabled = options.enabled ?? process.env.TOPGUN_DEBUG === 'true';
    this.maxHistory = options.maxHistory || 100;
  }

  // ============================================================================
  // Control
  // ============================================================================

  isEnabled(): boolean {
    return this.enabled;
  }

  enable(): void {
    this.enabled = true;
  }

  disable(): void {
    this.enabled = false;
  }

  // ============================================================================
  // Recording
  // ============================================================================

  recordSearch(debugInfo: SearchDebugInfo): void {
    if (!this.enabled) return;

    this.lastQuery = debugInfo;
    this.history.push(debugInfo);

    if (this.history.length > this.maxHistory) {
      this.history = this.history.slice(-this.maxHistory);
    }
  }

  // ============================================================================
  // Querying
  // ============================================================================

  getLastQuery(): SearchDebugInfo | null {
    return this.lastQuery;
  }

  getHistory(): SearchDebugInfo[] {
    return this.history;
  }

  getHistoryByMap(mapId: string): SearchDebugInfo[] {
    return this.history.filter((q) => q.mapId === mapId);
  }

  explainResult(docId: string): SearchResultDebug | undefined {
    return this.lastQuery?.results.find((r) => r.docId === docId);
  }

  // ============================================================================
  // Formatting
  // ============================================================================

  formatExplanation(docId: string): string {
    const result = this.explainResult(docId);
    if (!result) return 'No debug info available for this document.';

    const lines: string[] = [];

    lines.push(`Score Breakdown for ${docId}`);
    lines.push(`Final Score: ${result.finalScore.toFixed(4)}`);
    lines.push('');

    // BM25 breakdown
    if (result.scoreBreakdown.bm25) {
      const bm25 = result.scoreBreakdown.bm25;
      lines.push('BM25 Full-Text Search:');
      lines.push(`  Score: ${bm25.score.toFixed(4)}`);
      lines.push(
        `  Document length: ${bm25.docLength} (avg: ${bm25.avgDocLength.toFixed(1)})`
      );
      lines.push(`  Parameters: k1=${bm25.k1}, b=${bm25.b}`);
      lines.push('  Term contributions:');

      for (const term of bm25.matchedTerms) {
        const tf = bm25.tf[term] || 0;
        const idf = bm25.idf[term] || 0;
        const contribution = tf * idf;
        lines.push(
          `    "${term}": TF=${tf.toFixed(3)}, IDF=${idf.toFixed(3)}, contribution=${contribution.toFixed(4)}`
        );
      }
      lines.push('');
    }

    // Exact match breakdown
    if (result.scoreBreakdown.exact) {
      const exact = result.scoreBreakdown.exact;
      lines.push('Exact Match:');
      lines.push(`  Score: ${exact.score.toFixed(4)}`);
      lines.push(`  Matched fields: ${exact.matchedFields.join(', ')}`);
      lines.push(`  Boost applied: ${exact.boostApplied}x`);
      lines.push('');
    }

    // RRF breakdown
    if (result.scoreBreakdown.rrf) {
      const rrf = result.scoreBreakdown.rrf;
      lines.push('Reciprocal Rank Fusion (RRF):');
      lines.push(`  Final rank: ${rrf.rank}`);
      lines.push(`  RRF score: ${rrf.score.toFixed(4)}`);
      lines.push(`  k parameter: ${rrf.k}`);
      lines.push('  Contributing ranks:');

      for (const contrib of rrf.contributingRanks) {
        lines.push(`    ${contrib.source}: rank ${contrib.rank}`);
      }
      lines.push('');
    }

    // Vector breakdown
    if (result.scoreBreakdown.vector) {
      const vector = result.scoreBreakdown.vector;
      lines.push('Vector Similarity:');
      lines.push(`  Score: ${vector.score.toFixed(4)}`);
      lines.push(`  Distance: ${vector.distance.toFixed(4)}`);
      lines.push(`  Metric: ${vector.similarity}`);
      lines.push('');
    }

    return lines.join('\n');
  }

  formatQuerySummary(): string {
    if (!this.lastQuery) return 'No query recorded.';

    const q = this.lastQuery;
    const lines: string[] = [];

    lines.push(`Query: "${q.query}"`);
    lines.push(`Tokens: ${q.queryTokens.join(', ')}`);
    lines.push(`Type: ${q.searchType}`);
    lines.push(`Results: ${q.matchingDocuments} of ${q.totalDocuments} documents`);
    lines.push('');
    lines.push('Timing:');
    lines.push(`  Tokenization: ${q.timing.tokenization.toFixed(2)}ms`);
    lines.push(`  Index lookup: ${q.timing.indexLookup.toFixed(2)}ms`);
    lines.push(`  Scoring: ${q.timing.scoring.toFixed(2)}ms`);
    lines.push(`  Ranking: ${q.timing.ranking.toFixed(2)}ms`);
    if (q.timing.fusion !== undefined) {
      lines.push(`  Fusion: ${q.timing.fusion.toFixed(2)}ms`);
    }
    lines.push(`  Total: ${q.timing.total.toFixed(2)}ms`);
    lines.push('');
    lines.push('Index stats:');
    lines.push(`  Type: ${q.indexStats.indexType}`);
    lines.push(`  Size: ${q.indexStats.indexSize} entries`);
    lines.push(`  Terms searched: ${q.indexStats.termsSearched}`);

    return lines.join('\n');
  }

  formatAllResults(): string {
    if (!this.lastQuery) return 'No query recorded.';

    const lines: string[] = [];
    lines.push(this.formatQuerySummary());
    lines.push('');
    lines.push('Results:');
    lines.push('');

    for (let i = 0; i < this.lastQuery.results.length; i++) {
      const result = this.lastQuery.results[i];
      lines.push(`${i + 1}. ${result.docId}`);
      lines.push(`   Final score: ${result.finalScore.toFixed(4)}`);

      if (result.scoreBreakdown.bm25) {
        const bm25 = result.scoreBreakdown.bm25;
        lines.push(`   BM25: ${bm25.score.toFixed(4)}`);
        const topTerms = bm25.matchedTerms.slice(0, 3);
        for (const term of topTerms) {
          lines.push(
            `     - "${term}": TF=${bm25.tf[term]?.toFixed(3)}, IDF=${bm25.idf[term]?.toFixed(3)}`
          );
        }
      }

      if (result.scoreBreakdown.exact) {
        lines.push(
          `   Exact: ${result.scoreBreakdown.exact.score.toFixed(4)} (${result.scoreBreakdown.exact.matchedFields.join(', ')})`
        );
      }

      if (result.scoreBreakdown.rrf) {
        lines.push(`   RRF rank: ${result.scoreBreakdown.rrf.rank}`);
      }

      lines.push('');
    }

    return lines.join('\n');
  }

  // ============================================================================
  // Export
  // ============================================================================

  exportDebugInfo(): string {
    return JSON.stringify(this.lastQuery, null, 2);
  }

  exportHistory(): string {
    return JSON.stringify(
      {
        version: '1.0',
        exportedAt: new Date().toISOString(),
        queryCount: this.history.length,
        queries: this.history,
      },
      null,
      2
    );
  }

  // ============================================================================
  // Analysis
  // ============================================================================

  getSearchStats(): {
    totalQueries: number;
    averageResultCount: number;
    averageLatencyMs: number;
    queryTypeBreakdown: Record<string, number>;
  } {
    if (this.history.length === 0) {
      return {
        totalQueries: 0,
        averageResultCount: 0,
        averageLatencyMs: 0,
        queryTypeBreakdown: {},
      };
    }

    const queryTypeBreakdown: Record<string, number> = {};
    let totalResults = 0;
    let totalLatency = 0;

    for (const q of this.history) {
      queryTypeBreakdown[q.searchType] =
        (queryTypeBreakdown[q.searchType] || 0) + 1;
      totalResults += q.matchingDocuments;
      totalLatency += q.timing.total;
    }

    return {
      totalQueries: this.history.length,
      averageResultCount: totalResults / this.history.length,
      averageLatencyMs: totalLatency / this.history.length,
      queryTypeBreakdown,
    };
  }

  // ============================================================================
  // Utilities
  // ============================================================================

  clear(): void {
    this.lastQuery = null;
    this.history = [];
  }
}

// Singleton
let globalSearchDebugger: SearchDebugger | null = null;

export function getSearchDebugger(): SearchDebugger {
  if (!globalSearchDebugger) {
    globalSearchDebugger = new SearchDebugger();
  }
  return globalSearchDebugger;
}

export function resetSearchDebugger(): void {
  globalSearchDebugger = null;
}
