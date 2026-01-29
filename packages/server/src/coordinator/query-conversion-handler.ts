/**
 * QueryConversionHandler - Handles query conversion and cluster query finalization
 *
 * This handler manages:
 * - Converting server Query format to core Query format
 * - Executing queries on local maps with indexed/fallback execution
 * - Finalizing cluster queries with deduplication, sorting, and pagination
 * - Cursor-based pagination support
 *
 * Extracted from ServerCoordinator as part of SPEC-003d refactoring.
 */

import { LWWMap, ORMap, IndexedLWWMap, IndexedORMap, QueryCursor, DEFAULT_QUERY_CURSOR_MAX_AGE_MS, type QueryExpression as CoreQuery } from '@topgunbuild/core';
import { executeQuery, type Query } from '../query/Matcher';
import { logger } from '../utils/logger';
import type { IQueryConversionHandler, QueryConversionHandlerConfig } from './types';
import type { Subscription } from '../query/QueryRegistry';

export class QueryConversionHandler implements IQueryConversionHandler {
    private readonly config: QueryConversionHandlerConfig;

    constructor(config: QueryConversionHandlerConfig) {
        this.config = config;
    }

    /**
     * Execute query on local map.
     * Uses indexed execution if available, fallback to full scan.
     */
    async executeLocalQuery(mapName: string, query: Query): Promise<any[]> {
        // Wait for map to be fully loaded from storage before querying
        const map = await this.config.getMapAsync(mapName);

        // Fix: Do not apply cursor/limit locally for cluster queries.
        // They will be applied in finalizeClusterQuery after aggregation.
        const localQuery = { ...query };
        delete localQuery.cursor; // Phase 14.1: replaces offset
        delete localQuery.limit;

        // Use indexed query execution if available (O(1) to O(log N))
        if (map instanceof IndexedLWWMap) {
            // Convert Query to core query format for indexed execution
            const coreQuery = this.convertToCoreQuery(localQuery);
            if (coreQuery) {
                const entries = map.queryEntries(coreQuery);
                return entries.map(([key, value]) => {
                    const record = map.getRecord(key);
                    return { key, value, timestamp: record?.timestamp };
                });
            }
        }

        if (map instanceof IndexedORMap) {
            const coreQuery = this.convertToCoreQuery(localQuery);
            if (coreQuery) {
                const results = map.query(coreQuery);
                return results.map(({ key, value }) => ({ key, value }));
            }
        }

        // Fallback to full scan for non-indexed maps
        const records = new Map<string, any>();

        if (map instanceof LWWMap) {
            for (const key of map.allKeys()) {
                const rec = map.getRecord(key);
                if (rec && rec.value !== null) {
                    records.set(key, rec);
                }
            }
        } else if (map instanceof ORMap) {
            // For ORMap, we flatten values. A key matches if ANY of its values match?
            // Or we expose the array of values?
            // For now, we expose { key, value: [v1, v2, ...] }
            const items = (map as any).items as Map<string, any>;
            for (const key of items.keys()) {
                const values = map.get(key);
                if (values.length > 0) {
                    records.set(key, { value: values });
                }
            }
        }

        return executeQuery(records, localQuery);
    }

    /**
     * Convert server Query format to core Query format for indexed execution.
     * Returns null if conversion is not possible (complex queries).
     */
    convertToCoreQuery(query: Query): CoreQuery | null {
        // Handle predicate-based queries (core format)
        if (query.predicate) {
            return this.predicateToCoreQuery(query.predicate);
        }

        // Handle where-based queries (server format)
        if (query.where) {
            const conditions: CoreQuery[] = [];

            for (const [attribute, condition] of Object.entries(query.where)) {
                if (typeof condition !== 'object' || condition === null) {
                    // Simple equality: { status: 'active' }
                    conditions.push({ type: 'eq', attribute, value: condition });
                } else {
                    // Operator-based: { age: { $gte: 18 } }
                    for (const [op, value] of Object.entries(condition)) {
                        const coreOp = this.convertOperator(op);
                        if (coreOp) {
                            conditions.push({ type: coreOp, attribute, value } as CoreQuery);
                        }
                    }
                }
            }

            if (conditions.length === 0) return null;
            if (conditions.length === 1) return conditions[0];
            return { type: 'and', children: conditions };
        }

        return null;
    }

    /**
     * Convert predicate node to core Query format.
     */
    predicateToCoreQuery(predicate: any): CoreQuery | null {
        if (!predicate || !predicate.op) return null;

        switch (predicate.op) {
            case 'eq':
            case 'neq':
            case 'gt':
            case 'gte':
            case 'lt':
            case 'lte':
                return {
                    type: predicate.op,
                    attribute: predicate.attribute,
                    value: predicate.value,
                } as CoreQuery;

            case 'and':
            case 'or':
                if (predicate.children && Array.isArray(predicate.children)) {
                    const children = predicate.children
                        .map((c: any) => this.predicateToCoreQuery(c))
                        .filter((c: any): c is CoreQuery => c !== null);
                    if (children.length === 0) return null;
                    if (children.length === 1) return children[0];
                    return { type: predicate.op, children };
                }
                return null;

            case 'not':
                if (predicate.children && predicate.children[0]) {
                    const child = this.predicateToCoreQuery(predicate.children[0]);
                    if (child) {
                        return { type: 'not', child } as CoreQuery;
                    }
                }
                return null;

            default:
                return null;
        }
    }

    /**
     * Convert server operator to core query type.
     */
    convertOperator(op: string): 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | null {
        const mapping: Record<string, 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte'> = {
            '$eq': 'eq',
            '$ne': 'neq',
            '$neq': 'neq',
            '$gt': 'gt',
            '$gte': 'gte',
            '$lt': 'lt',
            '$lte': 'lte',
        };
        return mapping[op] || null;
    }

    /**
     * Finalize cluster query with aggregation, deduplication, sorting, and pagination.
     */
    async finalizeClusterQuery(requestId: string, timeout = false): Promise<void> {
        const pending = this.config.pendingClusterQueries.get(requestId);
        if (!pending) return;

        if (timeout) {
            logger.warn({ requestId, responded: pending.respondedNodes.size, expected: pending.expectedNodes.size }, 'Query timed out. Returning partial results.');
        }

        clearTimeout(pending.timer);
        this.config.pendingClusterQueries.delete(requestId);

        const { client, queryId, mapName, query, results } = pending;

        // Deduplicate results (if backups responded or multiple nodes have same key)
        const uniqueResults = new Map<string, any>();
        for (const res of results) {
            uniqueResults.set(res.key, res);
        }
        const finalResults = Array.from(uniqueResults.values());

        // Re-Apply Sort (Global)
        if (query.sort) {
            finalResults.sort((a, b) => {
                for (const [field, direction] of Object.entries(query.sort!)) {
                    // Handle ORMap array values vs LWW single values?
                    // Assuming LWW for sort logic or array comparison.
                    const valA = a.value[field];
                    const valB = b.value[field];
                    if (valA < valB) return direction === 'asc' ? -1 : 1;
                    if (valA > valB) return direction === 'asc' ? 1 : -1;
                }
                return 0;
            });
        }

        // Phase 14.1: Apply cursor-based pagination
        let slicedResults = finalResults;
        let nextCursor: string | undefined;
        let hasMore = false;
        let cursorStatus: 'valid' | 'expired' | 'invalid' | 'none' = 'none';

        if (query.cursor || query.limit) {
            const sort = query.sort || {};
            const sortEntries = Object.entries(sort);
            const sortField = sortEntries.length > 0 ? sortEntries[0][0] : '_key';

            // Apply cursor filtering and track status
            if (query.cursor) {
                const cursorData = QueryCursor.decode(query.cursor);
                if (!cursorData) {
                    cursorStatus = 'invalid';
                } else if (!QueryCursor.isValid(cursorData, query.predicate ?? query.where, sort)) {
                    // Check if it's specifically expired vs hash mismatch
                    if (Date.now() - cursorData.timestamp > DEFAULT_QUERY_CURSOR_MAX_AGE_MS) {
                        cursorStatus = 'expired';
                    } else {
                        cursorStatus = 'invalid';
                    }
                } else {
                    cursorStatus = 'valid';
                    slicedResults = finalResults.filter((r: any) => {
                        const sortValue = r.value[sortField];
                        return QueryCursor.isAfterCursor(
                            { key: r.key, sortValue },
                            cursorData
                        );
                    });
                }
            }

            // Apply limit and generate next cursor
            if (query.limit) {
                hasMore = slicedResults.length > query.limit;
                slicedResults = slicedResults.slice(0, query.limit);

                if (hasMore && slicedResults.length > 0) {
                    const lastResult = slicedResults[slicedResults.length - 1];
                    const sortValue = lastResult.value[sortField];
                    nextCursor = QueryCursor.fromLastResult(
                        { key: lastResult.key, sortValue },
                        sort,
                        query.predicate ?? query.where
                    );
                }
            }
        }

        // Register Subscription
        const resultKeys = new Set(slicedResults.map((r: any) => r.key));
        const sub: Subscription = {
            id: queryId,
            clientId: client.id,
            mapName,
            query,
            socket: client.socket,
            previousResultKeys: resultKeys,
            interestedFields: 'ALL'
        };

        this.config.queryRegistry.register(sub);
        client.subscriptions.add(queryId);

        // Apply Field Level Security
        const filteredResults = slicedResults.map((res: any) => {
            const filteredValue = this.config.securityManager.filterObject(res.value, client.principal!, mapName);
            return { ...res, value: filteredValue };
        });

        client.writer.write({
            type: 'QUERY_RESP',
            payload: { queryId, results: filteredResults, nextCursor, hasMore, cursorStatus }
        });
    }

    /**
     * Stop handler and clear all pending cluster query timers.
     * Called during server shutdown.
     */
    stop(): void {
        for (const [_requestId, pending] of this.config.pendingClusterQueries) {
            if (pending.timer) {
                clearTimeout(pending.timer);
            }
        }
        this.config.pendingClusterQueries.clear();
    }
}
