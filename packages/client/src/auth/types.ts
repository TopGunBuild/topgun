export interface AuthProvider {
  /**
   * Returns a valid token, refreshing if necessary.
   * Returns null if no valid session exists.
   */
  getToken(): Promise<string | null>;

  /**
   * Called once when the provider is attached to a client.
   * Use for subscribing to session change events.
   */
  initialize?(): void;

  /**
   * Called when the client is closed.
   * Use for cleanup (timers, listeners).
   */
  destroy?(): void;

  /**
   * Subscribe to auth lifecycle events.
   * Events: 'token:refreshed', 'auth:error', 'auth:signedOut'
   */
  onAuthEvent?(listener: (event: AuthEvent) => void): () => void;
}

export type AuthEventType = 'token:refreshed' | 'auth:error' | 'auth:signedOut';

export interface AuthEvent {
  type: AuthEventType;
  error?: Error;
}

export interface TokenExchangeConfig {
  /** Server HTTP URL for POST /api/auth/token. Required when using token exchange. */
  serverUrl: string;
  /** Provider name hint sent to server (e.g., 'clerk', 'firebase') */
  providerName?: string;
  /**
   * Enable the server-issued refresh token flow.
   * When true, the provider stores the refresh token returned by /api/auth/token
   * and uses it to renew the access JWT before falling back to fetchExternalToken().
   * The server must have refresh grants configured (PostgresRefreshGrantStore).
   * Default: false.
   */
  enableRefresh?: boolean;
}

/**
 * Shape of the token exchange response body from POST /api/auth/token.
 * The refresh fields are present only when the server has refresh grants enabled.
 */
export interface TokenExchangeResponse {
  token: string;
  expiresAt: number;
  refreshToken?: string;
  refreshExpiresAt?: number;
}
