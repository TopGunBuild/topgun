import { HLC, LWWMap, PermissionType, Principal } from '@topgunbuild/core';
import type { HttpSyncRequest, HttpSyncResponse } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IAuthHandler, IOperationHandler, IStorageManager, IQueryConversionHandler } from './types';

/**
 * Configuration for HttpSyncHandler, using dependency injection
 * for all server components it needs access to.
 */
export interface HttpSyncHandlerConfig {
  authHandler: IAuthHandler;
  operationHandler: IOperationHandler;
  storageManager: IStorageManager;
  queryConversionHandler: IQueryConversionHandler;
  searchCoordinator: {
    search: (mapName: string, query: string, options?: any) => any;
  };
  hlc: HLC;
  securityManager: {
    checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
  };
}

/**
 * Handles stateless HTTP sync requests for serverless environments.
 *
 * Each request is fully self-contained: the client sends its HLC state,
 * operations, and sync map timestamps, and receives acknowledgments,
 * deltas, and query results in a single response. No per-client
 * server-side state persists between requests.
 */
export class HttpSyncHandler {
  private readonly config: HttpSyncHandlerConfig;

  constructor(config: HttpSyncHandlerConfig) {
    this.config = config;
  }

  /**
   * Process a complete HTTP sync request and return a response.
   *
   * @param request - Parsed and validated HttpSyncRequest body
   * @param authToken - JWT token from Authorization header
   * @returns HttpSyncResponse with acks, deltas, query/search results
   * @throws Error with message starting with '401:' for auth failures
   * @throws Error with message starting with '403:' for permission failures
   */
  async handleSyncRequest(
    request: HttpSyncRequest,
    authToken: string,
  ): Promise<HttpSyncResponse> {
    // Verify authentication
    let principal: Principal;
    try {
      principal = this.config.authHandler.verifyToken(authToken);
    } catch (err: any) {
      throw new Error(`401: Authentication failed: ${err.message}`);
    }

    // Tick the server HLC with the client's timestamp for causality tracking
    this.config.hlc.update({
      millis: request.clientHlc.millis,
      counter: request.clientHlc.counter,
      nodeId: request.clientHlc.nodeId,
    });

    const response: HttpSyncResponse = {
      serverHlc: this.config.hlc.now(),
    };

    const errors: Array<{ code: number; message: string; context?: string }> = [];

    // Process operations
    if (request.operations && request.operations.length > 0) {
      const ackResults = await this.processOperations(request.operations, principal, errors);
      if (ackResults.processedCount > 0) {
        response.ack = {
          lastId: ackResults.lastId,
          results: ackResults.results.length > 0 ? ackResults.results : undefined,
        };
      }
    }

    // Compute deltas for requested sync maps
    if (request.syncMaps && request.syncMaps.length > 0) {
      response.deltas = await this.computeDeltas(request.syncMaps, principal, errors);
    }

    // Execute one-shot queries
    if (request.queries && request.queries.length > 0) {
      response.queryResults = await this.executeQueries(request.queries, principal, errors);
    }

    // Execute one-shot searches
    if (request.searches && request.searches.length > 0) {
      response.searchResults = await this.executeSearches(request.searches, principal, errors);
    }

    // Attach errors if any occurred
    if (errors.length > 0) {
      response.errors = errors;
    }

    // Update serverHlc to reflect any changes from processing
    response.serverHlc = this.config.hlc.now();

    return response;
  }

  /**
   * Process a batch of client operations.
   */
  private async processOperations(
    operations: HttpSyncRequest['operations'] & {},
    principal: Principal,
    errors: Array<{ code: number; message: string; context?: string }>,
  ): Promise<{ processedCount: number; lastId: string; results: any[] }> {
    let lastId = '';
    let processedCount = 0;
    const results: any[] = [];

    for (const op of operations) {
      const opId = op.id || `http-op-${processedCount}`;

      // Check write permission
      const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
      const action: PermissionType = isRemove ? 'REMOVE' : 'PUT';

      if (!this.config.securityManager.checkPermission(principal, op.mapName, action)) {
        errors.push({
          code: 403,
          message: 'Access denied',
          context: `Operation on ${op.mapName}/${op.key}`,
        });
        results.push({
          opId,
          success: false,
          achievedLevel: 'FIRE_AND_FORGET',
          error: 'Access denied',
        });
        continue;
      }

      try {
        const result = await this.config.operationHandler.applyOpToMap(op);
        if (result.rejected) {
          results.push({
            opId,
            success: false,
            achievedLevel: 'FIRE_AND_FORGET',
            error: 'Operation rejected by conflict resolver',
          });
        } else {
          results.push({
            opId,
            success: true,
            achievedLevel: 'MEMORY',
          });
        }
        processedCount++;
        lastId = opId;
      } catch (err: any) {
        logger.error({ err, opId }, 'HTTP sync operation failed');
        errors.push({
          code: 500,
          message: err.message || 'Operation failed',
          context: `Operation ${opId} on ${op.mapName}/${op.key}`,
        });
        results.push({
          opId,
          success: false,
          achievedLevel: 'FIRE_AND_FORGET',
          error: err.message || 'Operation failed',
        });
      }
    }

    return { processedCount, lastId, results };
  }

  /**
   * Compute deltas by iterating the in-memory LWWMap and filtering records
   * newer than the client's lastSyncTimestamp using HLC.compare().
   */
  private async computeDeltas(
    syncMaps: NonNullable<HttpSyncRequest['syncMaps']>,
    principal: Principal,
    errors: Array<{ code: number; message: string; context?: string }>,
  ): Promise<HttpSyncResponse['deltas']> {
    const deltas: NonNullable<HttpSyncResponse['deltas']> = [];

    for (const { mapName, lastSyncTimestamp } of syncMaps) {
      // Check read permission
      if (!this.config.securityManager.checkPermission(principal, mapName, 'READ')) {
        errors.push({
          code: 403,
          message: 'Access denied',
          context: `Read deltas for ${mapName}`,
        });
        continue;
      }

      try {
        const map = await this.config.storageManager.getMapAsync(mapName);

        // Only support LWWMap deltas (ORMap deltas are out of scope)
        if (!(map instanceof LWWMap)) {
          errors.push({
            code: 400,
            message: 'HTTP sync only supports LWWMap deltas',
            context: `Map ${mapName} is not an LWWMap`,
          });
          continue;
        }

        const records: Array<{
          key: string;
          record: any;
          eventType: 'PUT' | 'REMOVE';
        }> = [];

        // Iterate all keys and filter by timestamp
        for (const key of map.allKeys()) {
          const record = map.getRecord(key);
          if (!record) continue;

          // Include records newer than the client's last sync timestamp
          if (HLC.compare(record.timestamp, lastSyncTimestamp) > 0) {
            records.push({
              key,
              record,
              eventType: record.value === null ? 'REMOVE' : 'PUT',
            });
          }
        }

        const serverSyncTimestamp = this.config.hlc.now();

        deltas.push({
          mapName,
          records,
          serverSyncTimestamp,
        });
      } catch (err: any) {
        logger.error({ err, mapName }, 'HTTP sync delta computation failed');
        errors.push({
          code: 500,
          message: err.message || 'Delta computation failed',
          context: `Map ${mapName}`,
        });
      }
    }

    return deltas.length > 0 ? deltas : undefined;
  }

  /**
   * Execute one-shot queries.
   */
  private async executeQueries(
    queries: NonNullable<HttpSyncRequest['queries']>,
    principal: Principal,
    errors: Array<{ code: number; message: string; context?: string }>,
  ): Promise<HttpSyncResponse['queryResults']> {
    const results: NonNullable<HttpSyncResponse['queryResults']> = [];

    for (const query of queries) {
      // Check read permission
      if (!this.config.securityManager.checkPermission(principal, query.mapName, 'READ')) {
        errors.push({
          code: 403,
          message: 'Access denied',
          context: `Query ${query.queryId} on ${query.mapName}`,
        });
        continue;
      }

      try {
        const coreQuery: any = {
          where: query.filter,
          limit: query.limit,
        };

        const queryResults = await this.config.queryConversionHandler.executeLocalQuery(
          query.mapName,
          coreQuery,
        );

        const hasMore = query.limit ? queryResults.length >= query.limit : false;

        results.push({
          queryId: query.queryId,
          results: queryResults,
          hasMore,
        });
      } catch (err: any) {
        logger.error({ err, queryId: query.queryId }, 'HTTP sync query failed');
        errors.push({
          code: 500,
          message: err.message || 'Query failed',
          context: `Query ${query.queryId} on ${query.mapName}`,
        });
      }
    }

    return results.length > 0 ? results : undefined;
  }

  /**
   * Execute one-shot searches.
   */
  private async executeSearches(
    searches: NonNullable<HttpSyncRequest['searches']>,
    principal: Principal,
    errors: Array<{ code: number; message: string; context?: string }>,
  ): Promise<HttpSyncResponse['searchResults']> {
    const results: NonNullable<HttpSyncResponse['searchResults']> = [];

    for (const search of searches) {
      // Check read permission
      if (!this.config.securityManager.checkPermission(principal, search.mapName, 'READ')) {
        errors.push({
          code: 403,
          message: 'Access denied',
          context: `Search ${search.searchId} on ${search.mapName}`,
        });
        continue;
      }

      try {
        const searchResult = this.config.searchCoordinator.search(
          search.mapName,
          search.query,
          search.options,
        );

        results.push({
          searchId: search.searchId,
          results: searchResult.results || [],
          totalCount: searchResult.totalCount,
        });
      } catch (err: any) {
        logger.error({ err, searchId: search.searchId }, 'HTTP sync search failed');
        errors.push({
          code: 500,
          message: err.message || 'Search failed',
          context: `Search ${search.searchId} on ${search.mapName}`,
        });
      }
    }

    return results.length > 0 ? results : undefined;
  }
}
