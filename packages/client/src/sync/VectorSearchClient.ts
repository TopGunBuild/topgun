/**
 * VectorSearchClient - Handles ANN vector search request/response for SyncEngine
 *
 * Responsibilities:
 * - One-shot VECTOR_SEARCH requests routed to the server's HNSW backend
 * - Handle VECTOR_SEARCH_RESP responses from server
 * - Timeout handling for pending requests
 * - Cleanup on close
 */

import { vectorToBytes, bytesToVector } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type {
  IVectorSearchClient,
  VectorSearchClientConfig,
  VectorSearchClientOptions,
  VectorSearchClientResult,
} from './types';

/**
 * Default timeout for vector search requests (ms).
 */
const DEFAULT_VECTOR_SEARCH_TIMEOUT = 30000;

/**
 * Default number of nearest neighbours to return.
 */
const DEFAULT_K = 10;

/**
 * Pending vector search request state.
 */
interface PendingVectorSearchRequest {
  resolve: (results: VectorSearchClientResult[]) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

/**
 * VectorSearchClient implements IVectorSearchClient.
 *
 * Manages ANN vector search with support for:
 * - One-shot HNSW queries with configurable timeout
 * - Request/response pattern matched by id
 * - Float32Array <-> Uint8Array conversion at the boundary
 */
export class VectorSearchClient implements IVectorSearchClient {
  private readonly config: VectorSearchClientConfig;
  private readonly timeoutMs: number;

  // Pending vector search requests by request id
  private pendingRequests: Map<string, PendingVectorSearchRequest> = new Map();

  constructor(config: VectorSearchClientConfig) {
    this.config = config;
    this.timeoutMs = config.timeoutMs ?? DEFAULT_VECTOR_SEARCH_TIMEOUT;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Perform an ANN vector search on the server.
   *
   * Converts the developer-facing Float32Array to the wire-format little-endian
   * Uint8Array before sending, and converts the response vector back on receipt.
   *
   * @param mapName - Name of the map / HNSW index to search
   * @param queryVector - Query vector as Float32Array or number[]
   * @param options - Search options (k, efSearch, etc.)
   * @returns Promise resolving to ranked VectorSearchClientResult[]
   */
  public async vectorSearch(
    mapName: string,
    queryVector: Float32Array | number[],
    options?: VectorSearchClientOptions
  ): Promise<VectorSearchClientResult[]> {
    if (!this.config.isAuthenticated()) {
      throw new Error('Not connected to server');
    }

    const id = crypto.randomUUID();
    const queryVectorBytes = vectorToBytes(queryVector);
    const k = options?.k ?? DEFAULT_K;

    return new Promise((resolve, reject) => {
      // Set timeout to avoid hanging promises on server-side failures
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error('Vector search request timed out'));
      }, this.timeoutMs);

      // Store pending request
      this.pendingRequests.set(id, {
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

      // Send VECTOR_SEARCH request to server
      const sent = this.config.sendMessage({
        type: 'VECTOR_SEARCH',
        payload: {
          id,
          mapName,
          queryVector: queryVectorBytes,
          k,
          ...(options?.indexName !== undefined && { indexName: options.indexName }),
          ...(options?.efSearch !== undefined && { efSearch: options.efSearch }),
          ...(options?.includeValue !== undefined ||
            options?.includeVectors !== undefined ||
            options?.minScore !== undefined
            ? {
                options: {
                  ...(options.includeValue !== undefined && { includeValue: options.includeValue }),
                  ...(options.includeVectors !== undefined && { includeVectors: options.includeVectors }),
                  ...(options.minScore !== undefined && { minScore: options.minScore }),
                },
              }
            : {}),
        },
      });

      if (!sent) {
        this.pendingRequests.delete(id);
        clearTimeout(timeout);
        reject(new Error('Failed to send vector search request'));
      }
    });
  }

  /**
   * Handle VECTOR_SEARCH_RESP message from server.
   * Called by the message router on receipt of VECTOR_SEARCH_RESP.
   */
  public handleResponse(payload: {
    id: string;
    results: Array<{ key: string; score: number; value?: unknown; vector?: Uint8Array }>;
    totalCandidates: number;
    searchTimeMs: number;
    error?: string;
  }): void {
    const pending = this.pendingRequests.get(payload.id);
    if (pending) {
      this.pendingRequests.delete(payload.id);

      if (payload.error) {
        pending.reject(new Error(payload.error));
      } else {
        // Convert wire-format Uint8Array vectors back to Float32Array for callers
        const results: VectorSearchClientResult[] = payload.results.map((r) => ({
          key: r.key,
          score: r.score,
          ...(r.value !== undefined && { value: r.value }),
          ...(r.vector !== undefined && { vector: bytesToVector(r.vector) }),
        }));
        pending.resolve(results);
      }
    } else {
      logger.warn({ id: payload.id }, 'Received VECTOR_SEARCH_RESP for unknown request id');
    }
  }

  /**
   * Clean up resources.
   * Clears pending timeouts without rejecting promises to match SqlClient behaviour.
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
