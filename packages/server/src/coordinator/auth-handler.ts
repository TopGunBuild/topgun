import * as jwt from 'jsonwebtoken';
import { Principal } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IAuthHandler, ClientConnection, AuthHandlerConfig, AuthResult } from './types';

/**
 * Handles JWT authentication for client connections.
 *
 * This is a stateless handler - it only processes tokens and updates
 * client state. It does not maintain any connection registry or state.
 *
 * Supports both:
 * - HS256 (symmetric) - standard shared secret
 * - RS256 (asymmetric) - RSA public key (e.g., Clerk)
 */
export class AuthHandler implements IAuthHandler {
    private readonly jwtSecret: string;
    private readonly onAuthSuccess?: (clientId: string, principal: Principal) => void;
    private readonly onAuthFailure?: (clientId: string, error: string) => void;

    constructor(config: AuthHandlerConfig) {
        this.jwtSecret = config.jwtSecret;
        this.onAuthSuccess = config.onAuthSuccess;
        this.onAuthFailure = config.onAuthFailure;
    }

    /**
     * Verify a JWT token and return the principal.
     *
     * Algorithm selection:
     * - RS256 for RSA keys (detected by -----BEGIN prefix)
     * - HS256 for symmetric secrets
     *
     * Principal normalization:
     * - Adds default 'USER' role if roles missing
     * - Maps 'sub' claim to 'userId' if userId missing
     *
     * @param token The JWT token to verify
     * @returns The decoded and normalized principal
     * @throws Error if token is invalid or verification fails
     */
    verifyToken(token: string): Principal {
        const isRSAKey = this.jwtSecret.includes('-----BEGIN');
        const verifyOptions: jwt.VerifyOptions = isRSAKey
            ? { algorithms: ['RS256'] }
            : { algorithms: ['HS256'] };

        const decoded = jwt.verify(token, this.jwtSecret, verifyOptions) as any;

        // Normalize principal - ensure roles exist
        if (!decoded.roles) {
            decoded.roles = ['USER'];
        }

        // Normalize principal - map sub to userId if needed
        if (!decoded.userId && decoded.sub) {
            decoded.userId = decoded.sub;
        }

        return decoded as Principal;
    }

    /**
     * Handle an AUTH message from a client.
     *
     * On success:
     * - Updates client.principal with verified principal
     * - Sets client.isAuthenticated to true
     * - Calls onAuthSuccess callback if configured
     *
     * On failure:
     * - Calls onAuthFailure callback if configured
     * - Returns error message in result
     *
     * @param client The client connection to authenticate
     * @param token The JWT token from the AUTH message
     * @returns AuthResult with success status and principal or error
     */
    async handleAuth(client: ClientConnection, token: string): Promise<AuthResult> {
        try {
            const principal = this.verifyToken(token);

            // Update client state
            client.principal = principal;
            client.isAuthenticated = true;

            logger.info(
                { clientId: client.id, user: principal.userId || 'anon' },
                'Client authenticated'
            );

            // Notify success callback
            this.onAuthSuccess?.(client.id, principal);

            return {
                success: true,
                principal,
            };
        } catch (e) {
            const errorMessage = e instanceof Error ? e.message : 'Invalid token';

            logger.error({ clientId: client.id, err: e }, 'Auth failed');

            // Notify failure callback
            this.onAuthFailure?.(client.id, errorMessage);

            return {
                success: false,
                error: errorMessage,
            };
        }
    }
}
