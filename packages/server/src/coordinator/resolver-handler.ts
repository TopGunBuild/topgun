/**
 * ResolverHandler - Handles REGISTER_RESOLVER, UNREGISTER_RESOLVER, LIST_RESOLVERS messages
 *
 * This handler manages custom conflict resolver registration and listing.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { logger } from '../utils/logger';
import type { IResolverHandler, ClientConnection, ResolverHandlerConfig } from './types';

export class ResolverHandler implements IResolverHandler {
    private readonly config: ResolverHandlerConfig;

    constructor(config: ResolverHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle REGISTER_RESOLVER message.
     * Registers a custom conflict resolver for a map.
     */
    handleRegisterResolver(client: ClientConnection, message: any): void {
        const { requestId, mapName, resolver } = message;

        // Check PUT permission (resolver registration is a privileged operation)
        if (!this.config.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
            client.writer.write({
                type: 'REGISTER_RESOLVER_RESPONSE',
                requestId,
                success: false,
                error: `Access Denied for map ${mapName}`,
            }, true);
            return;
        }

        try {
            this.config.conflictResolverHandler.registerResolver(
                mapName,
                {
                    name: resolver.name,
                    code: resolver.code,
                    priority: resolver.priority,
                    keyPattern: resolver.keyPattern,
                },
                client.id,
            );

            client.writer.write({
                type: 'REGISTER_RESOLVER_RESPONSE',
                requestId,
                success: true,
            });

            logger.info({
                clientId: client.id,
                mapName,
                resolverName: resolver.name,
                priority: resolver.priority,
            }, 'Conflict resolver registered');
        } catch (err) {
            const errorMessage = err instanceof Error ? err.message : String(err);
            client.writer.write({
                type: 'REGISTER_RESOLVER_RESPONSE',
                requestId,
                success: false,
                error: errorMessage,
            }, true);
            logger.warn({
                clientId: client.id,
                mapName,
                error: errorMessage,
            }, 'Failed to register conflict resolver');
        }
    }

    /**
     * Handle UNREGISTER_RESOLVER message.
     * Unregisters a conflict resolver for a map.
     */
    handleUnregisterResolver(client: ClientConnection, message: any): void {
        const { requestId, mapName, resolverName } = message;

        // Check PUT permission
        if (!this.config.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
            client.writer.write({
                type: 'UNREGISTER_RESOLVER_RESPONSE',
                requestId,
                success: false,
                error: `Access Denied for map ${mapName}`,
            }, true);
            return;
        }

        const removed = this.config.conflictResolverHandler.unregisterResolver(
            mapName,
            resolverName,
            client.id,
        );

        client.writer.write({
            type: 'UNREGISTER_RESOLVER_RESPONSE',
            requestId,
            success: removed,
            error: removed ? undefined : 'Resolver not found or not owned by this client',
        });

        if (removed) {
            logger.info({
                clientId: client.id,
                mapName,
                resolverName,
            }, 'Conflict resolver unregistered');
        }
    }

    /**
     * Handle LIST_RESOLVERS message.
     * Lists all registered resolvers, optionally filtered by map.
     */
    handleListResolvers(client: ClientConnection, message: any): void {
        const { requestId, mapName } = message;

        // Check READ permission if mapName specified
        if (mapName && !this.config.securityManager.checkPermission(client.principal!, mapName, 'READ')) {
            client.writer.write({
                type: 'LIST_RESOLVERS_RESPONSE',
                requestId,
                resolvers: [],
            });
            return;
        }

        const resolvers = this.config.conflictResolverHandler.listResolvers(mapName);

        client.writer.write({
            type: 'LIST_RESOLVERS_RESPONSE',
            requestId,
            resolvers,
        });
    }
}
