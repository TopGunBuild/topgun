import type { WebSocket } from 'ws';
import type { HLC, Principal, Timestamp } from '@topgunbuild/core';
import type { CoalescingWriter, CoalescingWriterOptions } from '../utils/CoalescingWriter';

/**
 * Represents a connected client with its WebSocket and state.
 */
export interface ClientConnection {
    id: string;
    socket: WebSocket;
    writer: CoalescingWriter;
    principal?: Principal;
    isAuthenticated: boolean;
    subscriptions: Set<string>;
    lastActiveHlc: Timestamp;
    lastPingReceived: number;
}

/**
 * Interface for managing client connections.
 * ConnectionManager is the single owner of the clients Map.
 */
export interface IConnectionManager {
    /** Get all connected clients (read-only access) */
    getClients(): Map<string, ClientConnection>;

    /** Get a specific client by ID */
    getClient(clientId: string): ClientConnection | undefined;

    /** Register a new client connection */
    registerClient(clientId: string, socket: WebSocket, writer: CoalescingWriter): ClientConnection;

    /** Remove client and return the removed connection (for cleanup) */
    removeClient(clientId: string): ClientConnection | undefined;

    /** Update client's authenticated state */
    setClientAuthenticated(clientId: string, principal: Principal): void;

    /** Broadcast message to all clients (optionally excluding one) */
    broadcast(message: any, excludeClientId?: string): void;

    /** Broadcast batch of events */
    broadcastBatch(events: any[], excludeClientId?: string): void;

    /** Check if client is alive based on heartbeat */
    isClientAlive(clientId: string): boolean;

    /** Get client idle time in ms */
    getClientIdleTime(clientId: string): number;

    /** Update client's last ping timestamp */
    updateLastPing(clientId: string): void;

    /** Get total client count */
    getClientCount(): number;
}

/**
 * Configuration for ConnectionManager.
 */
export interface ConnectionManagerConfig {
    hlc: HLC;
    writeCoalescingEnabled: boolean;
    writeCoalescingOptions: Partial<CoalescingWriterOptions>;
    /** Client heartbeat timeout in ms (default: 20000) */
    clientHeartbeatTimeoutMs?: number;
    /** Callback when a client is registered */
    onClientRegistered?: (client: ClientConnection) => void;
    /** Callback when a client is removed */
    onClientRemoved?: (clientId: string) => void;
}

// ============================================================================
// AuthHandler Types
// ============================================================================

/**
 * Result of an authentication attempt.
 */
export interface AuthResult {
    success: boolean;
    principal?: Principal;
    error?: string;
}

/**
 * Configuration for the AuthHandler.
 */
export interface AuthHandlerConfig {
    jwtSecret: string;
    onAuthSuccess?: (clientId: string, principal: Principal) => void;
    onAuthFailure?: (clientId: string, error: string) => void;
}

/**
 * Interface for authentication handling.
 * AuthHandler is stateless - it only processes tokens and updates client state.
 */
export interface IAuthHandler {
    /**
     * Verify a JWT token and return the principal.
     * @param token The JWT token to verify
     * @returns The decoded principal
     * @throws Error if token is invalid
     */
    verifyToken(token: string): Principal;

    /**
     * Handle an AUTH message from a client.
     * Updates client.principal and client.isAuthenticated on success.
     * @param client The client connection to authenticate
     * @param token The JWT token
     * @returns AuthResult with success status and principal or error
     */
    handleAuth(client: ClientConnection, token: string): Promise<AuthResult>;
}
