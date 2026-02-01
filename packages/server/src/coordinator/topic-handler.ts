/**
 * TopicHandler - Handles TOPIC_SUB, TOPIC_UNSUB, TOPIC_PUB messages
 *
 * This handler manages topic pub/sub functionality with access control.
 *
 * Extracted from ServerCoordinator .
 */

import { logger } from '../utils/logger';
import type { ITopicHandler, ClientConnection, TopicHandlerConfig } from './types';

export class TopicHandler implements ITopicHandler {
    private readonly config: TopicHandlerConfig;

    constructor(config: TopicHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle TOPIC_SUB message.
     * Subscribes client to a topic after permission check.
     */
    handleTopicSub(client: ClientConnection, message: any): void {
        const { topic } = message.payload;

        // C1: Access Control
        // We treat topics as resources.
        // Policy check: action 'READ' on resource `topic:${topic}`
        if (!this.config.securityManager.checkPermission(client.principal!, `topic:${topic}`, 'READ')) {
            logger.warn({ clientId: client.id, topic }, 'Access Denied: TOPIC_SUB');
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for topic ${topic}` }
            }, true);
            return;
        }

        try {
            this.config.topicManager.subscribe(client.id, topic);
        } catch (e: any) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 400, message: e.message }
            }, true);
        }
    }

    /**
     * Handle TOPIC_UNSUB message.
     * Unsubscribes client from a topic.
     */
    handleTopicUnsub(client: ClientConnection, message: any): void {
        const { topic } = message.payload;
        this.config.topicManager.unsubscribe(client.id, topic);
    }

    /**
     * Handle TOPIC_PUB message.
     * Publishes data to a topic after permission check.
     */
    handleTopicPub(client: ClientConnection, message: any): void {
        const { topic, data } = message.payload;

        // C1: Access Control
        // Policy check: action 'PUT' (publish) on resource `topic:${topic}`
        if (!this.config.securityManager.checkPermission(client.principal!, `topic:${topic}`, 'PUT')) {
            logger.warn({ clientId: client.id, topic }, 'Access Denied: TOPIC_PUB');
            // No error sent back? Fire and forget usually implies silent drop or async error.
            // But for security violations, an error is useful during dev.
            // Spec says fire-and-forget delivery, but security rejection should ideally notify.
            // Let's send error.
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for topic ${topic}` }
            }, true);
            return;
        }

        try {
            this.config.topicManager.publish(topic, data, client.id);
        } catch (e: any) {
            // Invalid topic name etc
            client.writer.write({
                type: 'ERROR',
                payload: { code: 400, message: e.message }
            }, true);
        }
    }
}
