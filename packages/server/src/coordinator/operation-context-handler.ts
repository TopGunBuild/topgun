/**
 * OperationContextHandler - Handles operation context and interceptors
 *
 * This handler manages:
 * - Building operation context from client connections
 * - Running before/after interceptors
 * - Handling lock granted notifications (local and cluster)
 *
 * Extracted from ServerCoordinator as part of SPEC-003d refactoring.
 */

import { logger } from '../utils/logger';
import type { IOperationContextHandler, OperationContextHandlerConfig } from './types';
import type { OpContext, ServerOp, IInterceptor } from '../interceptor/IInterceptor';

export class OperationContextHandler implements IOperationContextHandler {
    private readonly config: OperationContextHandlerConfig;

    constructor(config: OperationContextHandlerConfig) {
        this.config = config;
    }

    /**
     * Build operation context from clientId.
     * Resolves authenticated state and principal if client exists.
     */
    buildOpContext(clientId: string, fromCluster: boolean): OpContext {
        let context: OpContext = {
            clientId,
            isAuthenticated: false,
            fromCluster,
            originalSenderId: clientId
        };

        if (!fromCluster) {
            const client = this.config.connectionManager.getClient(clientId);
            if (client) {
                context = {
                    clientId: client.id,
                    socket: client.socket,
                    isAuthenticated: client.isAuthenticated,
                    principal: client.principal,
                    fromCluster,
                    originalSenderId: clientId
                };
            }
        }

        return context;
    }

    /**
     * Run onBeforeOp interceptors.
     * Returns modified operation or null if silently dropped.
     */
    async runBeforeInterceptors(op: any, context: OpContext): Promise<any | null> {
        let currentOp: ServerOp | null = op;

        for (const interceptor of this.config.interceptors as IInterceptor[]) {
            if (interceptor.onBeforeOp && currentOp) {
                currentOp = await interceptor.onBeforeOp(currentOp, context);
                if (!currentOp) {
                    logger.debug({ interceptor: interceptor.name, opId: op.id }, 'Interceptor silently dropped op');
                    return null;
                }
            }
        }

        return currentOp;
    }

    /**
     * Run onAfterOp interceptors (fire-and-forget).
     */
    runAfterInterceptors(op: any, context: OpContext): void {
        for (const interceptor of this.config.interceptors as IInterceptor[]) {
            if (interceptor.onAfterOp) {
                interceptor.onAfterOp(op, context).catch(err => {
                    logger.error({ err }, 'Error in onAfterOp');
                });
            }
        }
    }

    /**
     * Handle lock granted notification.
     * Routes to local client or remote node via cluster.
     */
    handleLockGranted({ clientId, requestId, name, fencingToken }: { clientId: string, requestId: string, name: string, fencingToken: number }): void {
        // Check if local client
        const client = this.config.connectionManager.getClient(clientId);
        if (client) {
            client.writer.write({
                type: 'LOCK_GRANTED',
                payload: { requestId, name, fencingToken }
            });
            return;
        }

        // Check if remote client (composite ID: "nodeId:realClientId")
        const parts = clientId.split(':');
        if (parts.length === 2) {
            const [nodeId, realClientId] = parts;
            // Verify nodeId is not self (loopback check, though split should handle it)
            if (nodeId !== this.config.cluster.config.nodeId) {
                this.config.cluster.send(nodeId, 'CLUSTER_LOCK_GRANTED', {
                    clientId: realClientId,
                    requestId,
                    name,
                    fencingToken
                });
                return;
            }
        }

        logger.warn({ clientId, name, fencingToken }, 'Could not route LOCK_GRANTED to client');
    }
}
