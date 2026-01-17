import type { IncomingMessage, ServerResponse } from 'http';
import type { LWWMap, ORMap } from '@topgunbuild/core';
import {
  getCRDTDebugger,
  getSearchDebugger,
  type CRDTSnapshot,
  type SearchDebugInfo,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

/**
 * Configuration for debug endpoints.
 */
export interface DebugEndpointsConfig {
  /** Whether debug endpoints are enabled */
  enabled: boolean;
  /** Map accessor function */
  getMaps: () => Map<string, LWWMap<string, unknown> | ORMap<string, unknown>>;
}

/**
 * DebugEndpoints - HTTP request handlers for CRDT and Search debugging.
 *
 * Endpoints:
 * - POST /debug/crdt/export - Export CRDT operation history
 * - POST /debug/crdt/stats - Get CRDT statistics
 * - POST /debug/crdt/conflicts - Get resolved conflicts
 * - POST /debug/crdt/operations - Query operations
 * - POST /debug/search/explain - Execute search with debug info
 * - GET /debug/search/stats - Get search statistics
 * - GET /health - Health check
 * - GET /ready - Readiness check
 *
 * @see PHASE_14C_OBSERVABILITY.md for specification
 */
export class DebugEndpoints {
  private config: DebugEndpointsConfig;

  constructor(config: DebugEndpointsConfig) {
    this.config = config;
  }

  /**
   * Handle an incoming HTTP request.
   * Returns true if the request was handled, false otherwise.
   */
  async handle(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);
    const pathname = url.pathname;

    // Health and readiness endpoints (always enabled)
    if (pathname === '/health') {
      return this.handleHealth(res);
    }

    if (pathname === '/ready') {
      return this.handleReady(res);
    }

    // Debug endpoints require explicit enablement
    if (!this.config.enabled) {
      return false;
    }

    // CRDT debug endpoints
    if (pathname === '/debug/crdt/export') {
      return this.handleCrdtExport(req, res);
    }

    if (pathname === '/debug/crdt/stats') {
      return this.handleCrdtStats(req, res);
    }

    if (pathname === '/debug/crdt/conflicts') {
      return this.handleCrdtConflicts(req, res);
    }

    if (pathname === '/debug/crdt/operations') {
      return this.handleCrdtOperations(req, res);
    }

    if (pathname === '/debug/crdt/timeline') {
      return this.handleCrdtTimeline(req, res);
    }

    // Search debug endpoints
    if (pathname === '/debug/search/explain') {
      return this.handleSearchExplain(req, res);
    }

    if (pathname === '/debug/search/stats') {
      return this.handleSearchStats(res);
    }

    if (pathname === '/debug/search/history') {
      return this.handleSearchHistory(req, res);
    }

    return false;
  }

  // ============================================================================
  // Health Endpoints
  // ============================================================================

  private handleHealth(res: ServerResponse): boolean {
    res.setHeader('Content-Type', 'application/json');
    res.end(
      JSON.stringify({
        status: 'ok',
        timestamp: new Date().toISOString(),
      })
    );
    return true;
  }

  private handleReady(res: ServerResponse): boolean {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ ready: true }));
    return true;
  }

  // ============================================================================
  // CRDT Debug Endpoints
  // ============================================================================

  private async handleCrdtExport(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getCRDTDebugger();

      const format = (body.format as 'json' | 'csv' | 'ndjson') || 'json';
      const data = debugger_.exportHistory(format);

      const contentType =
        format === 'csv'
          ? 'text/csv'
          : format === 'ndjson'
            ? 'application/x-ndjson'
            : 'application/json';

      res.setHeader('Content-Type', contentType);
      res.end(data);
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  private async handleCrdtStats(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getCRDTDebugger();

      const stats = debugger_.getStatistics(body.mapId as string | undefined);

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(stats, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  private async handleCrdtConflicts(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getCRDTDebugger();

      const conflicts = debugger_.getConflicts(body.mapId as string | undefined);

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(conflicts, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  private async handleCrdtOperations(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getCRDTDebugger();

      const operations = debugger_.getOperations({
        mapId: body.mapId as string | undefined,
        nodeId: body.nodeId as string | undefined,
        operation: body.operation as string | undefined,
        limit: body.limit as number | undefined,
      });

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(operations, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  private async handleCrdtTimeline(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getCRDTDebugger();

      const intervalMs = (body.intervalMs as number) || 1000;
      const timeline = debugger_.getTimeline(
        intervalMs,
        body.mapId as string | undefined
      );

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(timeline, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  // ============================================================================
  // Search Debug Endpoints
  // ============================================================================

  private async handleSearchExplain(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getSearchDebugger();

      // If query is provided, we should execute a search with debug enabled
      // For now, return the last recorded search or an error if none
      if (body.query) {
        // Note: The actual search execution should be done by the caller
        // This endpoint just returns whatever was recorded
        const lastQuery = debugger_.getLastQuery();
        if (lastQuery && lastQuery.query === body.query) {
          res.setHeader('Content-Type', 'application/json');
          res.end(JSON.stringify(lastQuery, null, 2));
          return true;
        }

        // No matching query found
        res.setHeader('Content-Type', 'application/json');
        res.end(
          JSON.stringify({
            error: 'No search history available. Execute a search query via client first, then use this endpoint to inspect results.',
            query: body.query,
          })
        );
        return true;
      }

      // Return last query if no specific query requested
      const lastQuery = debugger_.getLastQuery();
      if (lastQuery) {
        res.setHeader('Content-Type', 'application/json');
        res.end(JSON.stringify(lastQuery, null, 2));
        return true;
      }

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify({ error: 'No search recorded yet' }));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  private handleSearchStats(res: ServerResponse): boolean {
    try {
      const debugger_ = getSearchDebugger();
      const stats = debugger_.getSearchStats();

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(stats, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error as Error);
    }
  }

  private async handleSearchHistory(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const debugger_ = getSearchDebugger();

      let history: SearchDebugInfo[];
      if (body.mapId) {
        history = debugger_.getHistoryByMap(body.mapId as string);
      } else {
        history = debugger_.getHistory();
      }

      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(history, null, 2));
      return true;
    } catch (error) {
      return this.handleError(res, error);
    }
  }

  // ============================================================================
  // Utilities
  // ============================================================================

  private async parseBody(req: IncomingMessage): Promise<Record<string, unknown>> {
    return new Promise((resolve, reject) => {
      if (req.method === 'GET') {
        // Parse query string for GET requests
        const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);
        const params: Record<string, unknown> = {};
        for (const [key, value] of url.searchParams) {
          params[key] = value;
        }
        resolve(params);
        return;
      }

      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        if (!body) {
          resolve({});
          return;
        }
        try {
          resolve(JSON.parse(body));
        } catch {
          resolve({});
        }
      });
      req.on('error', reject);
    });
  }

  private handleError(res: ServerResponse, error: unknown): boolean {
    logger.error({ error }, 'Debug endpoint error');
    res.statusCode = 500;
    res.setHeader('Content-Type', 'application/json');
    res.end(
      JSON.stringify({
        error: 'Internal Server Error',
        message: error instanceof Error ? error.message : String(error),
      })
    );
    return true;
  }
}

/**
 * Create debug endpoints handler.
 */
export function createDebugEndpoints(
  config: DebugEndpointsConfig
): DebugEndpoints {
  return new DebugEndpoints(config);
}
