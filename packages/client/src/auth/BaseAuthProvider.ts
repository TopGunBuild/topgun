import type { AuthProvider, AuthEvent, AuthEventType, TokenExchangeConfig } from './types';

export interface BaseAuthProviderConfig {
  tokenExchangeConfig?: TokenExchangeConfig;
  refreshMarginMs?: number;
}

/**
 * Abstract base class for auth providers with shared token lifecycle management.
 * Handles caching, proactive refresh, JWT expiry detection, token exchange,
 * event emission, and concurrent getToken() deduplication.
 */
export abstract class BaseAuthProvider implements AuthProvider {
  private cachedToken: string | null = null;
  private cachedTokenExpiry: number = 0;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private listeners: Array<(event: AuthEvent) => void> = [];
  private inflightPromise: Promise<string | null> | null = null;

  protected readonly tokenExchangeConfig?: TokenExchangeConfig;
  protected readonly refreshMarginMs: number;

  constructor(config?: BaseAuthProviderConfig) {
    this.tokenExchangeConfig = config?.tokenExchangeConfig;
    this.refreshMarginMs = config?.refreshMarginMs ?? 60_000;
  }

  /**
   * Subclasses implement this to fetch a token from the external auth provider.
   */
  protected abstract fetchExternalToken(): Promise<string | null>;

  /**
   * Returns a valid token, refreshing if necessary.
   * Concurrent calls while a refresh is in-flight share the same Promise.
   */
  async getToken(): Promise<string | null> {
    // Return cached token if it's still valid (not within refresh margin)
    if (this.cachedToken && Date.now() < this.cachedTokenExpiry - this.refreshMarginMs) {
      return this.cachedToken;
    }

    // Deduplicate concurrent calls: return existing in-flight Promise
    if (this.inflightPromise) {
      return this.inflightPromise;
    }

    // Start a new token fetch
    this.inflightPromise = this.refreshToken();

    try {
      const token = await this.inflightPromise;
      return token;
    } finally {
      this.inflightPromise = null;
    }
  }

  private async refreshToken(): Promise<string | null> {
    try {
      let token = await this.fetchExternalToken();

      if (!token) {
        this.cachedToken = null;
        this.cachedTokenExpiry = 0;
        this.clearRefreshTimer();
        return null;
      }

      // If token exchange is configured, exchange external token for TopGun JWT
      if (this.tokenExchangeConfig) {
        token = await this.exchangeToken(token);
        if (!token) {
          this.cachedToken = null;
          this.cachedTokenExpiry = 0;
          this.clearRefreshTimer();
          return null;
        }
      }

      // Cache the token and extract expiry
      this.cachedToken = token;
      this.cachedTokenExpiry = this.extractExpiry(token);

      // Schedule proactive refresh
      this.scheduleRefresh();

      this.emit({ type: 'token:refreshed' });

      return token;
    } catch (error) {
      this.emit({
        type: 'auth:error',
        error: error instanceof Error ? error : new Error(String(error)),
      });
      return null;
    }
  }

  /**
   * Exchange an external provider token for a TopGun JWT via POST /api/auth/token.
   */
  private async exchangeToken(externalToken: string): Promise<string | null> {
    const { serverUrl, providerName } = this.tokenExchangeConfig!;
    const url = `${serverUrl.replace(/\/$/, '')}/api/auth/token`;

    const response = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        token: externalToken,
        provider: providerName,
      }),
    });

    if (!response.ok) {
      throw new Error(`Token exchange failed: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    return data.token ?? null;
  }

  /**
   * Extract the `exp` claim from a JWT payload (base64 decode, no signature verification).
   * Returns expiry as milliseconds since epoch, or 0 if parsing fails.
   */
  protected extractExpiry(token: string): number {
    try {
      const parts = token.split('.');
      if (parts.length !== 3) return 0;

      // Base64url decode the payload
      const payload = parts[1];
      const padded = payload.replace(/-/g, '+').replace(/_/g, '/');
      const decoded = atob(padded);
      const parsed = JSON.parse(decoded);

      if (typeof parsed.exp === 'number') {
        // JWT exp is in seconds, convert to milliseconds
        return parsed.exp * 1000;
      }

      return 0;
    } catch {
      return 0;
    }
  }

  /**
   * Schedule a proactive token refresh before the cached token expires.
   */
  private scheduleRefresh(): void {
    this.clearRefreshTimer();

    if (this.cachedTokenExpiry <= 0) return;

    const refreshAt = this.cachedTokenExpiry - this.refreshMarginMs;
    const delay = refreshAt - Date.now();

    if (delay <= 0) return;

    this.refreshTimer = setTimeout(() => {
      // Trigger a background refresh; discard the result
      // (next getToken() call will use the refreshed value)
      this.inflightPromise = null;
      this.cachedToken = null;
      this.getToken().catch(() => {
        // Error already emitted via auth:error event
      });
    }, delay);
  }

  private clearRefreshTimer(): void {
    if (this.refreshTimer) {
      clearTimeout(this.refreshTimer);
      this.refreshTimer = null;
    }
  }

  initialize?(): void;

  onAuthEvent(listener: (event: AuthEvent) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  protected emit(event: AuthEvent): void {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch {
        // Listener errors should not break the provider
      }
    }
  }

  destroy(): void {
    this.clearRefreshTimer();
    this.listeners = [];
    this.cachedToken = null;
    this.cachedTokenExpiry = 0;
    this.inflightPromise = null;
  }
}
