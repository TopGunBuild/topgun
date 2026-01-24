/**
 * CounterHandlerAdapter - Adapts COUNTER_REQUEST, COUNTER_SYNC messages
 * to the existing CounterHandler
 *
 * This is a thin adapter that delegates to the existing CounterHandler
 * from handlers/CounterHandler.ts.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { WebSocket } from 'ws';
import { logger } from '../utils/logger';
import type { ICounterHandlerAdapter, ClientConnection, CounterHandlerAdapterConfig } from './types';

export class CounterHandlerAdapter implements ICounterHandlerAdapter {
    private readonly config: CounterHandlerAdapterConfig;

    constructor(config: CounterHandlerAdapterConfig) {
        this.config = config;
    }

    /**
     * Handle COUNTER_REQUEST message.
     * Delegates to CounterHandler.handleCounterRequest.
     */
    handleCounterRequest(client: ClientConnection, message: any): void {
        const { name } = message.payload;
        const response = this.config.counterHandler.handleCounterRequest(client.id, name);
        client.writer.write(response);
        logger.debug({ clientId: client.id, name }, 'Counter request handled');
    }

    /**
     * Handle COUNTER_SYNC message.
     * Delegates to CounterHandler.handleCounterSync and broadcasts to subscribers.
     */
    handleCounterSync(client: ClientConnection, message: any): void {
        const { name, state } = message.payload;
        const result = this.config.counterHandler.handleCounterSync(client.id, name, state);

        // Send response to the syncing client
        client.writer.write(result.response);

        // Broadcast to other subscribed clients
        for (const targetClientId of result.broadcastTo) {
            const targetClient = this.config.getClient(targetClientId);
            if (targetClient && targetClient.socket.readyState === WebSocket.OPEN) {
                targetClient.writer.write(result.broadcastMessage);
            }
        }
        logger.debug({ clientId: client.id, name, broadcastCount: result.broadcastTo.length }, 'Counter sync handled');
    }
}
