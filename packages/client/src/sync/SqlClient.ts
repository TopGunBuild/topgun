/**
 * SqlClient - Handles server-side SQL query execution for SyncEngine
 *
 * Responsibilities:
 * - One-shot SQL query requests routed to the server's DataFusion backend
 * - Handle SQL query responses from server
 * - Timeout handling for pending requests
 * - Cleanup on close
 */

import { logger } from '../utils/logger';
import type { ISqlClient, SqlClientConfig, SqlQueryResult } from './types';

/**
 * Default timeout for SQL query requests (ms).
 */
const DEFAULT_SQL_TIMEOUT = 30000;

/**
 * Pending SQL query request state.
 */
interface PendingSqlRequest {
  resolve: (result: SqlQueryResult) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

/**
 * SqlClient implements ISqlClient.
 *
 * Manages server-side SQL query execution with support for:
 * - One-shot DataFusion queries with configurable timeout
 * - Request/response pattern matched by queryId
 */
export class SqlClient implements ISqlClient {
  private readonly config: SqlClientConfig;
  private readonly timeoutMs: number;

  // Pending SQL requests by queryId
  private pendingSqlRequests: Map<string, PendingSqlRequest> = new Map();

  constructor(config: SqlClientConfig) {
    this.config = config;
    this.timeoutMs = config.timeoutMs ?? DEFAULT_SQL_TIMEOUT;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Execute a SQL query on the server via DataFusion.
   *
   * @param query SQL query string
   * @returns Promise resolving to { columns, rows }
   */
  public async sql(query: string): Promise<SqlQueryResult> {
    if (!this.config.isAuthenticated()) {
      throw new Error('Not connected to server');
    }

    const queryId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout to avoid hanging promises on server-side failures
      const timeout = setTimeout(() => {
        this.pendingSqlRequests.delete(queryId);
        reject(new Error('SQL query request timed out'));
      }, this.timeoutMs);

      // Store pending request
      this.pendingSqlRequests.set(queryId, {
        resolve: (result) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
        timeout,
      });

      // Send SQL query request to server
      const sent = this.config.sendMessage({
        type: 'SQL_QUERY',
        payload: {
          sql: query,
          queryId,
        },
      });

      if (!sent) {
        this.pendingSqlRequests.delete(queryId);
        clearTimeout(timeout);
        reject(new Error('Failed to send SQL query'));
      }
    });
  }

  /**
   * Handle SQL query response from server.
   * Called by SyncEngine for SQL_QUERY_RESP messages.
   */
  public handleSqlQueryResponse(payload: {
    queryId: string;
    columns: string[];
    rows: unknown[][];
    error?: string;
  }): void {
    const pending = this.pendingSqlRequests.get(payload.queryId);
    if (pending) {
      this.pendingSqlRequests.delete(payload.queryId);

      if (payload.error) {
        pending.reject(new Error(payload.error));
      } else {
        pending.resolve({ columns: payload.columns, rows: payload.rows });
      }
    } else {
      logger.warn({ queryId: payload.queryId }, 'Received SQL_QUERY_RESP for unknown queryId');
    }
  }

  /**
   * Clean up resources.
   * Clears pending timeouts without rejecting promises to match SearchClient behavior.
   */
  public close(error?: Error): void {
    // Only clear timeouts, don't reject promises to avoid unhandled rejections in tests
    for (const [, pending] of this.pendingSqlRequests.entries()) {
      clearTimeout(pending.timeout);
    }
    this.pendingSqlRequests.clear();
  }
}
