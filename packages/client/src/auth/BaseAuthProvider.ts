import type { AuthProvider, AuthEvent, AuthEventType, TokenExchangeConfig, TokenExchangeResponse } from './types';
import type { IStorageAdapter } from '../IStorageAdapter';

export interface BaseAuthProviderConfig {
  tokenExchangeConfig?: TokenExchangeConfig;
  refreshMarginMs?: number;
  storageAdapter?: IStorageAdapter;
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
  private refreshTokenValue: string | null = null;
  private readonly storageAdapter?: IStorageAdapter;
  private restored = false;

  protected readonly tokenExchangeConfig?: TokenExchangeConfig;
  protected readonly refreshMarginMs: number;

  constructor(config?: BaseAuthProviderConfig) {
    this.tokenExchangeConfig = config?.tokenExchangeConfig;
    this.refreshMarginMs = config?.refreshMarginMs ?? 60_000;
    this.storageAdapter = config?.storageAdapter;
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

  private async restoreRefreshToken(): Promise<void> {
    if (!this.storageAdapter || !this.tokenExchangeConfig?.enableRefresh) return;
    try {
      const stored = await this.storageAdapter.getMeta('topgun:refreshToken');
      if (typeof stored === 'string' && stored.length > 0) {
        this.refreshTokenValue = stored;
      }
    } catch {
      // Storage read failure is non-fatal
    }
  }

  private async refreshToken(): Promise<string | null> {
    try {
      // Restore refresh token from storage on first call
      if (!this.restored) {
        this.restored = true;
        await this.restoreRefreshToken();
      }

      // Attempt server-issued refresh before calling the external provider.
      // This avoids a round-trip to the external provider when a refresh token
      // from a previous exchange is still valid.
      if (
        this.refreshTokenValue &&
        this.tokenExchangeConfig?.enableRefresh
      ) {
        const refreshed = await this.attemptServerRefresh(this.refreshTokenValue);
        if (refreshed) return refreshed;
        // Fall through to external token fetch if server refresh fails.
      }

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
   * Attempt to renew the access token using the stored server refresh token.
   *
   * On success: updates cachedToken, cachedTokenExpiry, and refreshTokenValue,
   * then returns the new access token.
   * On failure (401, network error, or missing server URL): clears the stored
   * refresh token and returns null so the caller falls through to fetchExternalToken().
   */
  private async attemptServerRefresh(refreshToken: string): Promise<string | null> {
    if (!this.tokenExchangeConfig) return null;

    const { serverUrl } = this.tokenExchangeConfig;
    const url = `${serverUrl.replace(/\/$/, '')}/api/auth/refresh`;

    try {
      const response = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refreshToken }),
      });

      if (!response.ok) {
        // Clear the stale refresh token so the next attempt uses external fetch.
        this.refreshTokenValue = null;
        this.storageAdapter?.setMeta('topgun:refreshToken', null).catch(() => {});
        return null;
      }

      const data = await response.json();
      const newAccessToken: string | null = data.token ?? null;
      const newRefreshToken: string | null = data.refreshToken ?? null;

      if (!newAccessToken) {
        this.refreshTokenValue = null;
        this.storageAdapter?.setMeta('topgun:refreshToken', null).catch(() => {});
        return null;
      }

      // Update cached state with the new token pair.
      this.cachedToken = newAccessToken;
      this.cachedTokenExpiry = this.extractExpiry(newAccessToken);
      this.refreshTokenValue = newRefreshToken;
      this.storageAdapter?.setMeta('topgun:refreshToken', newRefreshToken).catch(() => {});

      this.scheduleRefresh();
      this.emit({ type: 'token:refreshed' });

      return newAccessToken;
    } catch {
      // Network error or JSON parse failure -- fall back to external fetch.
      this.refreshTokenValue = null;
      this.storageAdapter?.setMeta('topgun:refreshToken', null).catch(() => {});
      return null;
    }
  }

  /**
   * Exchange an external provider token for a TopGun JWT via POST /api/auth/token.
   *
   * When the server response includes a refresh token (and enableRefresh is true),
   * stores it for use on the next token renewal.
   */
  private async exchangeToken(externalToken: string): Promise<string | null> {
    const { serverUrl, providerName, enableRefresh } = this.tokenExchangeConfig!;
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

    const data: TokenExchangeResponse = await response.json();

    // Store the refresh token when the server returns one and refresh is enabled.
    if (enableRefresh && data.refreshToken) {
      this.refreshTokenValue = data.refreshToken;
      this.storageAdapter?.setMeta('topgun:refreshToken', data.refreshToken).catch(() => {});
    }

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

  /**
   * Clear the cached token, expiry, and stored refresh token so the next
   * getToken() call fetches fresh from the external provider.
   * Subclasses call this when the external session changes (e.g., Firebase onIdTokenChanged).
   */
  protected invalidateCache(): void {
    this.cachedToken = null;
    this.cachedTokenExpiry = 0;
    this.refreshTokenValue = null;
    this.storageAdapter?.setMeta('topgun:refreshToken', null).catch(() => {});
    this.clearRefreshTimer();
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
    this.refreshTokenValue = null;
    this.storageAdapter?.setMeta('topgun:refreshToken', null).catch(() => {});
    this.inflightPromise = null;
  }
}
