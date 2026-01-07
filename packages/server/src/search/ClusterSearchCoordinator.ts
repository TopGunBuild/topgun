/**
 * ClusterSearchCoordinator - Distributed Search across Cluster Nodes
 *
 * Implements Scatter-Gather architecture for distributed full-text search.
 * Broadcasts search queries to all nodes, collects local results, and merges
 * them using Reciprocal Rank Fusion (RRF) for unified ranking.
 *
 * Key features:
 * - Scatter-Gather search across all cluster nodes
 * - RRF-based result merging (handles distributed IDF problem)
 * - Cursor-based pagination for efficient deep results
 * - Graceful degradation on node failures
 * - Single-node optimization (skip broadcast when all data is local)
 *
 * @module search/ClusterSearchCoordinator
 */

import { EventEmitter } from 'events';
import {
  ReciprocalRankFusion,
  SearchCursor,
  type RankedResult,
  type SearchCursorData,
  type ClusterSearchReqPayload,
  type ClusterSearchRespPayload,
  type SearchOptions,
} from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { PartitionService } from '../cluster/PartitionService';
import { SearchCoordinator, type ServerSearchResult } from './SearchCoordinator';
import { logger } from '../utils/logger';

/**
 * Options for distributed search.
 */
export interface DistributedSearchOptions {
  /** Maximum results to return */
  limit: number;
  /** Cursor for pagination (opaque string) */
  cursor?: string;
  /** Timeout for waiting on node responses (ms) */
  timeoutMs?: number;
  /** Minimum number of nodes that must respond (0 = all) */
  minResponses?: number;
  /** Minimum score threshold */
  minScore?: number;
  /** Field boost weights */
  boost?: Record<string, number>;
}

/**
 * Result of a distributed search operation.
 */
export interface DistributedSearchResult {
  /** Merged and ranked results */
  results: ServerSearchResult[];
  /** Total hits across all nodes */
  totalHits: number;
  /** Cursor for next page (if more results available) */
  nextCursor?: string;
  /** Nodes that responded successfully */
  respondedNodes: string[];
  /** Nodes that failed or timed out */
  failedNodes: string[];
  /** Total execution time (ms) */
  executionTimeMs: number;
}

/**
 * Configuration for ClusterSearchCoordinator.
 */
export interface ClusterSearchConfig {
  /** RRF constant k (default: 60) */
  rrfK?: number;
  /** Default timeout for node responses (ms) */
  defaultTimeoutMs?: number;
  /** Minimum nodes required to respond (default: 0 = all available) */
  defaultMinResponses?: number;
}

/**
 * Internal pending request state.
 */
interface PendingRequest {
  resolve: (result: DistributedSearchResult) => void;
  reject: (error: Error) => void;
  responses: Map<string, ClusterSearchRespPayload>;
  expectedNodes: Set<string>;
  startTime: number;
  timeoutHandle: NodeJS.Timeout;
  options: DistributedSearchOptions;
  mapName: string;
  query: string;
}

/**
 * Default configuration values.
 */
const DEFAULT_CONFIG: Required<ClusterSearchConfig> = {
  rrfK: 60,
  defaultTimeoutMs: 5000,
  defaultMinResponses: 0,
};

/**
 * ClusterSearchCoordinator manages distributed search across cluster nodes.
 *
 * @example
 * ```typescript
 * const coordinator = new ClusterSearchCoordinator(
 *   clusterManager,
 *   partitionService,
 *   searchCoordinator
 * );
 *
 * const result = await coordinator.search('articles', 'machine learning', {
 *   limit: 10,
 *   timeoutMs: 3000,
 * });
 *
 * console.log(`Found ${result.totalHits} results across ${result.respondedNodes.length} nodes`);
 * ```
 */
export class ClusterSearchCoordinator extends EventEmitter {
  private readonly clusterManager: ClusterManager;
  private readonly partitionService: PartitionService;
  private readonly localSearchCoordinator: SearchCoordinator;
  private readonly rrf: ReciprocalRankFusion;
  private readonly config: Required<ClusterSearchConfig>;

  /** Pending requests awaiting responses */
  private readonly pendingRequests: Map<string, PendingRequest> = new Map();

  constructor(
    clusterManager: ClusterManager,
    partitionService: PartitionService,
    localSearchCoordinator: SearchCoordinator,
    config?: ClusterSearchConfig
  ) {
    super();
    this.clusterManager = clusterManager;
    this.partitionService = partitionService;
    this.localSearchCoordinator = localSearchCoordinator;
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.rrf = new ReciprocalRankFusion({ k: this.config.rrfK });

    // Register handler for incoming cluster messages
    this.clusterManager.on('message', this.handleClusterMessage.bind(this));
  }

  /**
   * Execute a distributed search across the cluster.
   *
   * @param mapName - Name of the map to search
   * @param query - Search query text
   * @param options - Search options
   * @returns Promise resolving to merged search results
   */
  async search(
    mapName: string,
    query: string,
    options: DistributedSearchOptions = { limit: 10 }
  ): Promise<DistributedSearchResult> {
    const startTime = performance.now();
    const requestId = this.generateRequestId();
    const timeoutMs = options.timeoutMs ?? this.config.defaultTimeoutMs;

    // Get all nodes in the cluster
    const allNodes = new Set(this.clusterManager.getMembers());
    const myNodeId = this.clusterManager.config.nodeId;

    // If single node cluster, execute locally only
    if (allNodes.size === 1 && allNodes.has(myNodeId)) {
      return this.executeLocalSearch(mapName, query, options, startTime);
    }

    // Calculate per-node limit (need more for RRF merge quality)
    const perNodeLimit = this.calculatePerNodeLimit(options.limit, options.cursor);

    // Decode cursor if provided
    let cursorData: SearchCursorData | null = null;
    if (options.cursor) {
      cursorData = SearchCursor.decode(options.cursor);
      if (cursorData && !SearchCursor.isValid(cursorData, query)) {
        cursorData = null; // Invalid or expired cursor
        logger.warn({ requestId }, 'Invalid or expired cursor, ignoring');
      }
    }

    // Create promise for this request
    const promise = new Promise<DistributedSearchResult>((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        this.resolvePartialResults(requestId);
      }, timeoutMs);

      this.pendingRequests.set(requestId, {
        resolve,
        reject,
        responses: new Map(),
        expectedNodes: allNodes,
        startTime,
        timeoutHandle,
        options,
        mapName,
        query,
      });
    });

    // Build search request payload
    const payload: ClusterSearchReqPayload = {
      requestId,
      mapName,
      query,
      options: {
        limit: perNodeLimit,
        minScore: options.minScore,
        boost: options.boost,
        includeMatchedTerms: true,
        // Add cursor position if available
        ...(cursorData ? {
          afterScore: cursorData.nodeScores[myNodeId],
          afterKey: cursorData.nodeKeys[myNodeId],
        } : {}),
      },
      timeoutMs,
    };

    // Send to all nodes (including self via local execution)
    for (const nodeId of allNodes) {
      if (nodeId === myNodeId) {
        // Execute locally and add to responses
        this.executeLocalAndRespond(requestId, mapName, query, perNodeLimit, cursorData);
      } else {
        // Send to remote node
        this.clusterManager.send(nodeId, 'CLUSTER_SEARCH_REQ' as any, payload);
      }
    }

    return promise;
  }

  /**
   * Handle incoming cluster messages.
   */
  private handleClusterMessage(msg: { type: string; senderId: string; payload: any }): void {
    switch (msg.type) {
      case 'CLUSTER_SEARCH_REQ':
        this.handleSearchRequest(msg.senderId, msg.payload);
        break;
      case 'CLUSTER_SEARCH_RESP':
        this.handleSearchResponse(msg.senderId, msg.payload);
        break;
    }
  }

  /**
   * Handle incoming search request from another node.
   */
  private async handleSearchRequest(
    senderId: string,
    payload: ClusterSearchReqPayload
  ): Promise<void> {
    const startTime = performance.now();
    const myNodeId = this.clusterManager.config.nodeId;

    try {
      // Execute local search
      const localResult = this.localSearchCoordinator.search(
        payload.mapName,
        payload.query,
        {
          limit: payload.options.limit,
          minScore: payload.options.minScore,
          boost: payload.options.boost,
        }
      );

      // Filter by cursor if provided
      let results = localResult.results;
      if (payload.options.afterScore !== undefined) {
        results = results.filter(r => {
          if (r.score < payload.options.afterScore!) {
            return true;
          }
          if (r.score === payload.options.afterScore && payload.options.afterKey) {
            return r.key > payload.options.afterKey;
          }
          return false;
        });
      }

      // Send response back
      const response: ClusterSearchRespPayload = {
        requestId: payload.requestId,
        nodeId: myNodeId,
        results: results.map(r => ({
          key: r.key,
          value: r.value,
          score: r.score,
          matchedTerms: r.matchedTerms,
        })),
        totalHits: localResult.totalCount ?? results.length,
        executionTimeMs: performance.now() - startTime,
      };

      this.clusterManager.send(senderId, 'CLUSTER_SEARCH_RESP' as any, response);
    } catch (error) {
      // Send error response
      this.clusterManager.send(senderId, 'CLUSTER_SEARCH_RESP' as any, {
        requestId: payload.requestId,
        nodeId: myNodeId,
        results: [],
        totalHits: 0,
        executionTimeMs: performance.now() - startTime,
        error: error instanceof Error ? error.message : 'Unknown error',
      });
    }
  }

  /**
   * Handle search response from a node.
   */
  private handleSearchResponse(
    _senderId: string,
    payload: ClusterSearchRespPayload
  ): void {
    const pending = this.pendingRequests.get(payload.requestId);
    if (!pending) {
      logger.warn({ requestId: payload.requestId }, 'Received response for unknown request');
      return;
    }

    // Store response
    pending.responses.set(payload.nodeId, payload);

    // Check if we have enough responses
    const minResponses = pending.options.minResponses ?? this.config.defaultMinResponses;
    const requiredResponses = minResponses > 0 ? minResponses : pending.expectedNodes.size;

    if (pending.responses.size >= requiredResponses) {
      clearTimeout(pending.timeoutHandle);
      this.mergeAndResolve(payload.requestId);
    }
  }

  /**
   * Merge results from all nodes using RRF and resolve the promise.
   */
  private mergeAndResolve(requestId: string): void {
    const pending = this.pendingRequests.get(requestId);
    if (!pending) return;

    const resultSets: RankedResult[][] = [];
    const respondedNodes: string[] = [];
    const failedNodes: string[] = [];
    let totalHits = 0;

    // Track results with their source nodes for cursor generation
    const resultsWithNodes: Array<{ key: string; score: number; nodeId: string; value: unknown; matchedTerms: string[] }> = [];

    for (const [nodeId, response] of pending.responses) {
      if (response.error) {
        failedNodes.push(nodeId);
        logger.warn({ nodeId, error: response.error }, 'Node returned error for search');
      } else {
        respondedNodes.push(nodeId);
        totalHits += response.totalHits;

        // Convert to RankedResult format for RRF
        const rankedResults: RankedResult[] = response.results.map((r) => ({
          docId: r.key,
          score: r.score,
          source: nodeId,
        }));
        resultSets.push(rankedResults);

        // Track full results for final output
        for (const r of response.results) {
          resultsWithNodes.push({
            key: r.key,
            score: r.score,
            nodeId,
            value: r.value,
            matchedTerms: r.matchedTerms || [],
          });
        }
      }
    }

    // Check for nodes that didn't respond
    for (const nodeId of pending.expectedNodes) {
      if (!pending.responses.has(nodeId)) {
        failedNodes.push(nodeId);
      }
    }

    // Merge using RRF
    const merged = this.rrf.merge(resultSets);

    // Build final results with values, applying limit
    const limit = pending.options.limit;
    const results: ServerSearchResult[] = [];
    const cursorResults: Array<{ key: string; score: number; nodeId: string }> = [];

    for (const mergedResult of merged.slice(0, limit)) {
      // Find the original result data
      const original = resultsWithNodes.find(r => r.key === mergedResult.docId);
      if (original) {
        results.push({
          key: original.key,
          value: original.value,
          score: mergedResult.score, // Use RRF score
          matchedTerms: original.matchedTerms,
        });
        cursorResults.push({
          key: original.key,
          score: original.score, // Use original score for cursor
          nodeId: original.nodeId,
        });
      }
    }

    // Generate cursor for next page if there are more results
    let nextCursor: string | undefined;
    if (merged.length > limit && cursorResults.length > 0) {
      nextCursor = SearchCursor.fromResults(cursorResults, pending.query);
    }

    const executionTimeMs = performance.now() - pending.startTime;

    pending.resolve({
      results,
      totalHits,
      nextCursor,
      respondedNodes,
      failedNodes,
      executionTimeMs,
    });

    this.pendingRequests.delete(requestId);

    logger.debug({
      requestId,
      mapName: pending.mapName,
      query: pending.query,
      resultCount: results.length,
      totalHits,
      respondedNodes: respondedNodes.length,
      failedNodes: failedNodes.length,
      executionTimeMs,
    }, 'Distributed search completed');
  }

  /**
   * Resolve with partial results when timeout occurs.
   */
  private resolvePartialResults(requestId: string): void {
    const pending = this.pendingRequests.get(requestId);
    if (!pending) return;

    logger.warn(
      {
        requestId,
        received: pending.responses.size,
        expected: pending.expectedNodes.size,
      },
      'Search request timed out, returning partial results'
    );

    this.mergeAndResolve(requestId);
  }

  /**
   * Execute local search and add response to pending request.
   */
  private async executeLocalAndRespond(
    requestId: string,
    mapName: string,
    query: string,
    limit: number,
    cursorData: SearchCursorData | null
  ): Promise<void> {
    const startTime = performance.now();
    const myNodeId = this.clusterManager.config.nodeId;
    const pending = this.pendingRequests.get(requestId);

    if (!pending) return;

    try {
      const localResult = this.localSearchCoordinator.search(mapName, query, {
        limit,
        minScore: pending.options.minScore,
        boost: pending.options.boost,
      });

      // Filter by cursor if provided
      let results = localResult.results;
      if (cursorData) {
        const position = SearchCursor.getNodePosition(cursorData, myNodeId);
        if (position) {
          results = results.filter(r => {
            if (r.score < position.afterScore) {
              return true;
            }
            if (r.score === position.afterScore) {
              return r.key > position.afterKey;
            }
            return false;
          });
        }
      }

      const response: ClusterSearchRespPayload = {
        requestId,
        nodeId: myNodeId,
        results: results.map(r => ({
          key: r.key,
          value: r.value,
          score: r.score,
          matchedTerms: r.matchedTerms,
        })),
        totalHits: localResult.totalCount ?? results.length,
        executionTimeMs: performance.now() - startTime,
      };

      // Directly add to pending responses (no network)
      this.handleSearchResponse(myNodeId, response);
    } catch (error) {
      this.handleSearchResponse(myNodeId, {
        requestId,
        nodeId: myNodeId,
        results: [],
        totalHits: 0,
        executionTimeMs: performance.now() - startTime,
        error: error instanceof Error ? error.message : 'Unknown error',
      });
    }
  }

  /**
   * Execute search locally only (single-node optimization).
   */
  private async executeLocalSearch(
    mapName: string,
    query: string,
    options: DistributedSearchOptions,
    startTime: number
  ): Promise<DistributedSearchResult> {
    const myNodeId = this.clusterManager.config.nodeId;

    const localResult = this.localSearchCoordinator.search(mapName, query, {
      limit: options.limit,
      minScore: options.minScore,
      boost: options.boost,
    });

    // Handle cursor-based filtering for single node
    let results = localResult.results;
    if (options.cursor) {
      const cursorData = SearchCursor.decode(options.cursor);
      if (cursorData && SearchCursor.isValid(cursorData, query)) {
        const position = SearchCursor.getNodePosition(cursorData, myNodeId);
        if (position) {
          results = results.filter(r => {
            if (r.score < position.afterScore) {
              return true;
            }
            if (r.score === position.afterScore) {
              return r.key > position.afterKey;
            }
            return false;
          });
        }
      }
    }

    // Apply limit after cursor filtering
    results = results.slice(0, options.limit);

    // Generate cursor if more results available
    // Check totalCount (if available) or results length
    const totalCount = localResult.totalCount ?? localResult.results.length;
    let nextCursor: string | undefined;
    if (totalCount > options.limit && results.length > 0) {
      const lastResult = results[results.length - 1];
      nextCursor = SearchCursor.fromResults(
        [{ key: lastResult.key, score: lastResult.score, nodeId: myNodeId }],
        query
      );
    }

    return {
      results,
      totalHits: localResult.totalCount ?? results.length,
      nextCursor,
      respondedNodes: [myNodeId],
      failedNodes: [],
      executionTimeMs: performance.now() - startTime,
    };
  }

  /**
   * Calculate per-node limit for distributed query.
   *
   * For quality RRF merge, each node should return more than the final limit.
   * Rule of thumb: 2x limit for good merge quality.
   */
  private calculatePerNodeLimit(limit: number, cursor?: string): number {
    if (cursor) {
      // Cursor-based: each node just needs limit
      return limit;
    }
    // No cursor: return more for better RRF quality
    return Math.min(limit * 2, 1000);
  }

  /**
   * Generate unique request ID.
   */
  private generateRequestId(): string {
    return `search-${Date.now()}-${Math.random().toString(36).slice(2, 11)}`;
  }

  /**
   * Get RRF constant k.
   */
  getRrfK(): number {
    return this.config.rrfK;
  }

  /**
   * Clean up resources.
   */
  destroy(): void {
    // Clear all pending requests
    for (const [requestId, pending] of this.pendingRequests) {
      clearTimeout(pending.timeoutHandle);
      pending.reject(new Error('ClusterSearchCoordinator destroyed'));
    }
    this.pendingRequests.clear();
    this.removeAllListeners();
  }
}
