/**
 * ServerTestHarness - Test harness for ServerCoordinator integration tests.
 *
 * Provides controlled access to internal handlers without modifying production code.
 * This pattern centralizes all test-specific access to internals in one place,
 * making tests more maintainable and less coupled to implementation details.
 *
 * Created during modularization to fix 23 integration tests that broke after
 * handler extraction during handler extraction.
 */

import type { ServerCoordinator } from '../../ServerCoordinator';
import type { ClusterManager } from '../../cluster/ClusterManager';
import type { PartitionService } from '../../cluster/PartitionService';
import type { ReplicationPipeline } from '../../cluster/ReplicationPipeline';
import type { SearchCoordinator } from '../../search/SearchCoordinator';
import type { QueryRegistry } from '../../query/QueryRegistry';
import type { HLC } from '@topgunbuild/core';
import type {
    ClientConnection,
    IConnectionManager,
    IWebSocketHandler,
    IHeartbeatHandler,
    IGCHandler,
    IOperationHandler,
    IBroadcastHandler,
} from '../../coordinator/types';

/**
 * Test harness for ServerCoordinator integration tests.
 * Provides controlled access to internal handlers without modifying production code.
 */
export class ServerTestHarness {
    private readonly server: ServerCoordinator;

    constructor(server: ServerCoordinator) {
        this.server = server;
    }

    // ============================================================================
    // Handler Accessors
    // ============================================================================

    /**
     * Access the WebSocketHandler for message handling tests.
     */
    get webSocketHandler(): IWebSocketHandler {
        return (this.server as any).webSocketHandler;
    }

    /**
     * Access the HeartbeatHandler for heartbeat tests.
     */
    get heartbeatHandler(): IHeartbeatHandler {
        return (this.server as any).heartbeatHandler;
    }

    /**
     * Access the ConnectionManager for client manipulation.
     * Returns as any for test flexibility with partial mock clients.
     */
    get connectionManager(): any {
        return (this.server as any).connectionManager;
    }

    /**
     * Access the GCHandler for garbage collection tests.
     */
    get gcHandler(): IGCHandler {
        return (this.server as any).gcHandler;
    }

    /**
     * Access the OperationHandler for operation processing tests.
     */
    get operationHandler(): IOperationHandler {
        return (this.server as any).operationHandler;
    }

    /**
     * Access the BroadcastHandler for broadcast tests.
     */
    get broadcastHandler(): IBroadcastHandler {
        return (this.server as any).broadcastHandler;
    }

    /**
     * Access the ClusterManager for cluster tests.
     * Used by DistributedGC.test.ts, ClusterE2E.test.ts, etc.
     */
    get cluster(): ClusterManager {
        return (this.server as any).cluster;
    }

    /**
     * Access the QueryRegistry for subscription tests.
     * Note: queryRegistry is now accessed through queryConversionHandler's config
     * after handler extraction removed it from ServerCoordinator.
     */
    get queryRegistry(): QueryRegistry {
        // Try direct access first (legacy path)
        const direct = (this.server as any).queryRegistry;
        if (direct) return direct;

        // Access through queryConversionHandler's config (post-refactoring path)
        const handler = (this.server as any).queryConversionHandler;
        return handler?.config?.queryRegistry;
    }

    /**
     * Access the PartitionService for partition tests.
     */
    get partitionService(): PartitionService {
        return (this.server as any).partitionService;
    }

    /**
     * Access the HLC for timestamp tests.
     */
    get hlc(): HLC {
        return (this.server as any).hlc;
    }

    /**
     * Access the SearchCoordinator for FTS tests.
     */
    get searchCoordinator(): SearchCoordinator {
        return (this.server as any).searchCoordinator;
    }

    /**
     * Access the ReplicationPipeline for replication tests.
     */
    get replicationPipeline(): ReplicationPipeline | undefined {
        return (this.server as any).replicationPipeline;
    }

    // ============================================================================
    // Message Handling
    // ============================================================================

    /**
     * Simulate receiving a message from a client.
     * Delegates to WebSocketHandler.handleMessage().
     * Accepts any client-like object for test flexibility (cast to full type internally).
     */
    async handleMessage(client: { id: string; [key: string]: any }, message: any): Promise<void> {
        return this.webSocketHandler.handleMessage(client as ClientConnection, message);
    }

    // ============================================================================
    // Heartbeat / Client Eviction
    // ============================================================================

    /**
     * Trigger dead client eviction manually.
     * Delegates to HeartbeatHandler.evictDeadClients() (private method).
     */
    evictDeadClients(): void {
        // HeartbeatHandler.evictDeadClients is private, expose via test access
        (this.heartbeatHandler as any).evictDeadClients();
    }

    // ============================================================================
    // Client Management
    // ============================================================================

    /**
     * Register a mock client connection for testing.
     * Accepts any client-like object for test flexibility (cast to full type internally).
     */
    registerMockClient(client: { id: string; [key: string]: any }): void {
        this.connectionManager.getClients().set(client.id, client as ClientConnection);
    }

    /**
     * Remove a mock client connection.
     */
    removeMockClient(clientId: string): void {
        this.connectionManager.getClients().delete(clientId);
    }

    /**
     * Get all connected clients.
     * Returns as Map<string, any> for test flexibility when working with partial mock clients.
     */
    getClients(): Map<string, any> {
        return this.connectionManager.getClients() as Map<string, any>;
    }

    // ============================================================================
    // Cluster Operations
    // ============================================================================

    /**
     * Report local HLC for cluster synchronization tests.
     * Used by DistributedGC.test.ts.
     */
    reportLocalHlc(): void {
        (this.server as any).reportLocalHlc?.();
    }

    /**
     * Get cluster members.
     */
    getClusterMembers(): string[] {
        return this.cluster?.getMembers() || [];
    }

    // ============================================================================
    // Operation Processing
    // ============================================================================

    /**
     * Process a local operation directly.
     * Used for tests that need to bypass client message handling.
     */
    async processLocalOp(op: any, fromCluster: boolean, senderId?: string): Promise<void> {
        return (this.server as any).processLocalOp?.(op, fromCluster, senderId) ??
               this.operationHandler.processLocalOp(op, fromCluster, senderId);
    }

    // ============================================================================
    // Broadcast
    // ============================================================================

    /**
     * Broadcast a message to all clients.
     */
    broadcast(message: any, excludeClientId?: string): void {
        (this.server as any).broadcast?.(message, excludeClientId) ??
        this.broadcastHandler.broadcast(message, excludeClientId);
    }
}

/**
 * Create a test harness for the given server.
 * Convenience factory function.
 */
export function createTestHarness(server: ServerCoordinator): ServerTestHarness {
    return new ServerTestHarness(server);
}
