/**
 * JournalHandler - Handles JOURNAL_SUBSCRIBE, JOURNAL_UNSUBSCRIBE, JOURNAL_READ messages
 *
 * This handler manages event journal subscriptions and reading.
 *
 * Extracted from ServerCoordinator .
 */

import { logger } from '../utils/logger';
import type { IJournalHandler, ClientConnection, JournalHandlerConfig } from './types';

export class JournalHandler implements IJournalHandler {
    private readonly config: JournalHandlerConfig;

    constructor(config: JournalHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle JOURNAL_SUBSCRIBE message.
     * Subscribes client to event journal notifications.
     */
    handleJournalSubscribe(client: ClientConnection, message: any): void {
        if (!this.config.eventJournalService) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 503, message: 'Event journal not enabled' }
            }, true);
            return;
        }

        const { requestId, fromSequence, mapName, types } = message;
        const subscriptionId = requestId;

        // Store subscription metadata
        this.config.journalSubscriptions.set(subscriptionId, {
            clientId: client.id,
            mapName,
            types,
        });

        // Subscribe to journal events
        const unsubscribe = this.config.eventJournalService.subscribe(
            (event) => {
                // Apply filters
                if (mapName && event.mapName !== mapName) return;
                if (types && types.length > 0 && !types.includes(event.type)) return;

                // Check if client still connected
                const clientConn = this.config.getClient(client.id);
                if (!clientConn) {
                    unsubscribe();
                    this.config.journalSubscriptions.delete(subscriptionId);
                    return;
                }

                // Send event to client
                clientConn.writer.write({
                    type: 'JOURNAL_EVENT',
                    event: {
                        sequence: event.sequence.toString(),
                        type: event.type,
                        mapName: event.mapName,
                        key: event.key,
                        value: event.value,
                        previousValue: event.previousValue,
                        timestamp: event.timestamp,
                        nodeId: event.nodeId,
                        metadata: event.metadata,
                    },
                });
            },
            fromSequence ? BigInt(fromSequence) : undefined
        );

        logger.info({ clientId: client.id, subscriptionId, mapName }, 'Journal subscription created');
    }

    /**
     * Handle JOURNAL_UNSUBSCRIBE message.
     * Removes a journal subscription.
     */
    handleJournalUnsubscribe(client: ClientConnection, message: any): void {
        const { subscriptionId } = message;
        this.config.journalSubscriptions.delete(subscriptionId);
        logger.info({ clientId: client.id, subscriptionId }, 'Journal subscription removed');
    }

    /**
     * Handle JOURNAL_READ message.
     * Reads events from the journal starting at a sequence number.
     */
    handleJournalRead(client: ClientConnection, message: any): void {
        if (!this.config.eventJournalService) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 503, message: 'Event journal not enabled' }
            }, true);
            return;
        }

        const { requestId: readReqId, fromSequence: readFromSeq, limit, mapName: readMapName } = message;
        const startSeq = BigInt(readFromSeq);
        const eventLimit = limit ?? 100;

        let events = this.config.eventJournalService.readFrom(startSeq, eventLimit);

        // Filter by map name if provided
        if (readMapName) {
            events = events.filter(e => e.mapName === readMapName);
        }

        // Serialize events
        const serializedEvents = events.map(e => ({
            sequence: e.sequence.toString(),
            type: e.type,
            mapName: e.mapName,
            key: e.key,
            value: e.value,
            previousValue: e.previousValue,
            timestamp: e.timestamp,
            nodeId: e.nodeId,
            metadata: e.metadata,
        }));

        client.writer.write({
            type: 'JOURNAL_READ_RESPONSE',
            requestId: readReqId,
            events: serializedEvents,
            hasMore: events.length === eventLimit,
        });
    }
}
