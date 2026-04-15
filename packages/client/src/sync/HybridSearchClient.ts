/**
 * HybridSearchClient - Handles tri-hybrid search request/response for SyncEngine
 *
 * Responsibilities:
 * - One-shot HYBRID_SEARCH requests combining exact, fullText, and/or semantic methods via RRF
 * - Handle HYBRID_SEARCH_RESP responses from server
 * - Timeout handling for pending requests
 * - Cleanup on close
 *
 * Mirrors VectorSearchClient exactly, adapted for the hybrid search wire protocol.
 */

import { vectorToBytes } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type {
  IHybridSearchClient,
  HybridSearchClientConfig,
  HybridSearchClientOptions,
  HybridSearchClientResult,
  HybridSearchMethod,
} from './types';

/**
 * Default timeout for hybrid search requests (ms).
 */
const DEFAULT_HYBRID_SEARCH_TIMEOUT = 30000;

/**
 * Default number of fused results to return.
 */
const DEFAULT_K = 10;

/**
 * Default methods to use when caller omits the option.
 * Matches the existing search() API default (fullText-only) to avoid surprises.
 */
const DEFAULT_METHODS: HybridSearchMethod[] = ['fullText'];

/**
 * Pending hybrid search request state.
 */
interface PendingHybridSearchRequest {
  resolve: (results: HybridSearchClientResult[]) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

/**
 * HybridSearchClient implements IHybridSearchClient.
 *
 * Manages tri-hybrid RRF search with support for:
 * - One-shot HYBRID_SEARCH queries with configurable timeout
 * - Request/response pattern matched by requestId
 * - Optional Float32Array -> little-endian Uint8Array conversion for the semantic leg
 */
export class HybridSearchClient implements IHybridSearchClient {
  private readonly config: HybridSearchClientConfig;
  private readonly timeoutMs: number;

  // Pending hybrid search requests by requestId
  private pendingRequests: Map<string, PendingHybridSearchRequest> = new Map();

  constructor(config: HybridSearchClientConfig) {
    this.config = config;
    this.timeoutMs = config.timeoutMs ?? DEFAULT_HYBRID_SEARCH_TIMEOUT;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Perform a tri-hybrid search on the server.
   *
   * When queryVector is provided, converts the developer-facing Float32Array to the
   * wire-format little-endian Uint8Array before sending.
   *
   * @param mapName - Name of the map to search
   * @param queryText - Search query text
   * @param options - Search options (methods, k, queryVector, predicate, etc.)
   * @returns Promise resolving to ranked HybridSearchClientResult[]
   */
  public async hybridSearch(
    mapName: string,
    queryText: string,
    options?: HybridSearchClientOptions
  ): Promise<HybridSearchClientResult[]> {
    if (!this.config.isAuthenticated()) {
      throw new Error('Not connected to server');
    }

    const requestId = crypto.randomUUID();
    const methods = options?.methods ?? DEFAULT_METHODS;
    const k = options?.k ?? DEFAULT_K;

    // Convert queryVector to wire-format Uint8Array when provided for the semantic leg
    const queryVector =
      options?.queryVector !== undefined ? vectorToBytes(options.queryVector) : undefined;

    return new Promise((resolve, reject) => {
      // Set timeout to avoid hanging promises on server-side failures
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error('Hybrid search request timed out'));
      }, this.timeoutMs);

      // Store pending request
      this.pendingRequests.set(requestId, {
        resolve: (results) => {
          clearTimeout(timeout);
          resolve(results);
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
        timeout,
      });

      // Build payload using ternary-spread pattern so optional fields are omitted
      // entirely when undefined (not spread as {}) — matches VectorSearchClient convention.
      // Do NOT write ...(queryVector ?? {}) which would spread the Uint8Array's own enumerable
      // properties when defined rather than including it under the 'queryVector' key.
      const payload = {
        requestId,
        mapName,
        queryText,
        methods,
        k,
        ...(queryVector !== undefined ? { queryVector } : {}),
        ...(options?.predicate !== undefined ? { predicate: options.predicate } : {}),
        ...(options?.includeValue !== undefined ? { includeValue: options.includeValue } : {}),
        ...(options?.minScore !== undefined ? { minScore: options.minScore } : {}),
      };

      // Send HYBRID_SEARCH request to server
      const sent = this.config.sendMessage({
        type: 'HYBRID_SEARCH',
        payload,
      });

      if (!sent) {
        this.pendingRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send hybrid search request'));
      }
    });
  }

  /**
   * Handle HYBRID_SEARCH_RESP message from server.
   * Called by the message router on receipt of HYBRID_SEARCH_RESP.
   */
  public handleResponse(payload: {
    requestId: string;
    results: Array<{
      key: string;
      score: number;
      methodScores: Partial<Record<HybridSearchMethod, number>>;
      value?: unknown;
    }>;
    searchTimeMs: number;
    error?: string;
  }): void {
    const pending = this.pendingRequests.get(payload.requestId);
    if (pending) {
      this.pendingRequests.delete(payload.requestId);

      if (payload.error) {
        pending.reject(new Error(payload.error));
      } else {
        // Server sends methodScores as a plain object keyed by method name — pass through as-is
        const results: HybridSearchClientResult[] = payload.results.map((r) => ({
          key: r.key,
          score: r.score,
          methodScores: r.methodScores,
          ...(r.value !== undefined && { value: r.value }),
        }));
        pending.resolve(results);
      }
    } else {
      logger.warn(
        { requestId: payload.requestId },
        'Received HYBRID_SEARCH_RESP for unknown request id'
      );
    }
  }

  /**
   * Clean up resources.
   * Clears pending timeouts without rejecting promises to match VectorSearchClient behaviour.
   * Pass an Error to also reject all pending promises (e.g. on hard disconnect).
   */
  public close(error?: Error): void {
    for (const [, pending] of this.pendingRequests.entries()) {
      clearTimeout(pending.timeout);
      if (error) {
        pending.reject(error);
      }
    }
    this.pendingRequests.clear();
  }
}
