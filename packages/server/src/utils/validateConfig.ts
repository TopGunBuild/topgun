/**
 * Configuration validation utilities for production safety.
 */

/**
 * Default JWT secret used in development/test environments.
 * This value is publicly known and must never be used in production.
 */
export const DEFAULT_JWT_SECRET = 'topgun-secret-dev';

/**
 * Validates JWT secret configuration for production safety.
 *
 * In production mode (NODE_ENV === 'production'):
 * - Throws if no secret is provided (neither config nor env var)
 * - Throws if the default development secret is used
 *
 * In development mode:
 * - Falls back to 'topgun-secret-dev' if no secret provided
 *
 * @param configSecret - Secret from config object
 * @param envSecret - Secret from environment variable
 * @returns The effective JWT secret to use
 * @throws Error if validation fails in production mode
 */
export function validateJwtSecret(
    configSecret: string | undefined,
    envSecret: string | undefined
): string {
    const isProduction = process.env.NODE_ENV === 'production';
    const effectiveSecret = configSecret || envSecret;

    if (isProduction) {
        if (!effectiveSecret) {
            throw new Error(
                'SECURITY ERROR: JWT_SECRET is required in production mode.\n' +
                'Set the JWT_SECRET environment variable or pass jwtSecret in the config.\n' +
                'Example: JWT_SECRET=$(openssl rand -base64 32) node server.js'
            );
        }

        if (effectiveSecret === DEFAULT_JWT_SECRET) {
            throw new Error(
                'SECURITY ERROR: Default JWT_SECRET cannot be used in production mode.\n' +
                'The default secret "topgun-secret-dev" is publicly known and insecure.\n' +
                'Generate a secure secret: openssl rand -base64 32'
            );
        }
    }

    // Development fallback
    return effectiveSecret || DEFAULT_JWT_SECRET;
}
