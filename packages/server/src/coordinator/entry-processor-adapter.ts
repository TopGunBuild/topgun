/**
 * EntryProcessorAdapter - Handles ENTRY_PROCESS, ENTRY_PROCESS_BATCH messages
 *
 * This adapter delegates to the existing EntryProcessorHandler
 * from handlers/EntryProcessorHandler.ts.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { LWWMap } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IEntryProcessorAdapter, ClientConnection, EntryProcessorAdapterConfig } from './types';

export class EntryProcessorAdapter implements IEntryProcessorAdapter {
    private readonly config: EntryProcessorAdapterConfig;

    constructor(config: EntryProcessorAdapterConfig) {
        this.config = config;
    }

    /**
     * Handle ENTRY_PROCESS message.
     * Executes an entry processor on a single key.
     */
    async handleEntryProcess(client: ClientConnection, message: any): Promise<void> {
        const { requestId, mapName, key, processor } = message;

        // Check PUT permission (entry processor modifies data)
        if (!this.config.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
            client.writer.write({
                type: 'ENTRY_PROCESS_RESPONSE',
                requestId,
                success: false,
                error: `Access Denied for map ${mapName}`,
            }, true);
            return;
        }

        // Get or create the map
        const entryMap = this.config.getMap(mapName) as LWWMap<string, any>;

        // Execute the processor
        const { result, timestamp } = await this.config.entryProcessorHandler.executeOnKey(
            entryMap,
            key,
            processor,
        );

        // Send response to client
        client.writer.write({
            type: 'ENTRY_PROCESS_RESPONSE',
            requestId,
            success: result.success,
            result: result.result,
            newValue: result.newValue,
            error: result.error,
        });

        // If successful and value changed, notify query subscribers
        if (result.success && timestamp) {
            const record = entryMap.getRecord(key);
            if (record) {
                this.config.queryRegistry.processChange(mapName, entryMap, key, record, undefined);
            }
        }

        logger.debug({
            clientId: client.id,
            mapName,
            key,
            processor: processor.name,
            success: result.success,
        }, 'Entry processor executed');
    }

    /**
     * Handle ENTRY_PROCESS_BATCH message.
     * Executes an entry processor on multiple keys.
     */
    async handleEntryProcessBatch(client: ClientConnection, message: any): Promise<void> {
        const { requestId, mapName, keys, processor } = message;

        // Check PUT permission
        if (!this.config.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
            const errorResults: Record<string, { success: boolean; error: string }> = {};
            for (const key of keys) {
                errorResults[key] = {
                    success: false,
                    error: `Access Denied for map ${mapName}`,
                };
            }
            client.writer.write({
                type: 'ENTRY_PROCESS_BATCH_RESPONSE',
                requestId,
                results: errorResults,
            }, true);
            return;
        }

        // Get or create the map
        const batchMap = this.config.getMap(mapName) as LWWMap<string, any>;

        // Execute the processor on all keys
        const { results, timestamps } = await this.config.entryProcessorHandler.executeOnKeys(
            batchMap,
            keys,
            processor,
        );

        // Convert Map to Record for serialization
        const resultsRecord: Record<string, {
            success: boolean;
            result?: unknown;
            newValue?: unknown;
            error?: string;
        }> = {};

        for (const [key, keyResult] of results) {
            resultsRecord[key] = {
                success: keyResult.success,
                result: keyResult.result,
                newValue: keyResult.newValue,
                error: keyResult.error,
            };
        }

        // Send batch response to client
        client.writer.write({
            type: 'ENTRY_PROCESS_BATCH_RESPONSE',
            requestId,
            results: resultsRecord,
        });

        // Notify query subscribers about changes
        for (const [key] of timestamps) {
            const record = batchMap.getRecord(key);
            if (record) {
                this.config.queryRegistry.processChange(mapName, batchMap, key, record, undefined);
            }
        }

        logger.debug({
            clientId: client.id,
            mapName,
            keyCount: keys.length,
            processor: processor.name,
            successCount: Array.from(results.values()).filter(r => r.success).length,
        }, 'Entry processor batch executed');
    }
}
