import { CustomAuthProvider } from '../CustomAuthProvider';
import { ClerkAuthProvider } from '../ClerkAuthProvider';
import { FirebaseAuthProvider } from '../FirebaseAuthProvider';
import { BetterAuthProvider } from '../BetterAuthProvider';
import type { AuthEvent } from '../types';
import type { IStorageAdapter } from '../../IStorageAdapter';

function createMockStorageAdapter(): IStorageAdapter & { store: Map<string, any> } {
  const store = new Map<string, any>();
  return {
    store,
    initialize: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    delete: jest.fn().mockResolvedValue(undefined),
    getAll: jest.fn().mockResolvedValue([]),
    query: jest.fn().mockResolvedValue([]),
    getMeta: jest.fn().mockImplementation((key: string) => Promise.resolve(store.get(key))),
    setMeta: jest.fn().mockImplementation((key: string, value: any) => {
      if (value === null || value === undefined) {
        store.delete(key);
      } else {
        store.set(key, value);
      }
      return Promise.resolve();
    }),
    appendOpLog: jest.fn().mockResolvedValue(undefined),
    getOpLog: jest.fn().mockResolvedValue([]),
    clearOpLog: jest.fn().mockResolvedValue(undefined),
  } as any;
}

/**
 * Create a mock JWT with the given expiry time (seconds since epoch).
 * No signature verification is performed -- this is just for testing expiry parsing.
 */
function createMockJwt(exp: number): string {
  const header = btoa(JSON.stringify({ alg: 'HS256', typ: 'JWT' }));
  const payload = btoa(JSON.stringify({ sub: 'user-123', exp }));
  const signature = 'mock-signature';
  return `${header}.${payload}.${signature}`;
}

describe('AuthProviders', () => {
  beforeEach(() => {
    jest.useFakeTimers();
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  describe('CustomAuthProvider', () => {
    it('getToken() returns token from user function', async () => {
      const token = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const getTokenFn = jest.fn().mockResolvedValue(token);
      const provider = new CustomAuthProvider(getTokenFn);

      const result = await provider.getToken();

      expect(result).toBe(token);
      expect(getTokenFn).toHaveBeenCalledTimes(1);
    });

    it('getToken() returns cached token on second call within expiry window', async () => {
      const exp = Math.floor(Date.now() / 1000) + 3600; // 1 hour from now
      const token = createMockJwt(exp);
      const getTokenFn = jest.fn().mockResolvedValue(token);
      const provider = new CustomAuthProvider(getTokenFn);

      const result1 = await provider.getToken();
      const result2 = await provider.getToken();

      expect(result1).toBe(token);
      expect(result2).toBe(token);
      expect(getTokenFn).toHaveBeenCalledTimes(1);
    });

    it('getToken() refreshes when cached token is within refresh margin', async () => {
      const now = Date.now();
      // Token expires in 30 seconds -- within the default 60s refresh margin
      const exp = Math.floor(now / 1000) + 30;
      const token1 = createMockJwt(exp);
      // Second token expires in 1 hour
      const token2 = createMockJwt(Math.floor(now / 1000) + 3600);

      const getTokenFn = jest.fn().mockResolvedValueOnce(token1).mockResolvedValueOnce(token2);
      const provider = new CustomAuthProvider(getTokenFn);

      const result1 = await provider.getToken();
      expect(result1).toBe(token1);

      // Second call should refresh because token is within refresh margin
      const result2 = await provider.getToken();
      expect(result2).toBe(token2);
      expect(getTokenFn).toHaveBeenCalledTimes(2);
    });

    it('getToken() returns null when user function returns null', async () => {
      const getTokenFn = jest.fn().mockResolvedValue(null);
      const provider = new CustomAuthProvider(getTokenFn);

      const result = await provider.getToken();

      expect(result).toBeNull();
    });
  });

  describe('ClerkAuthProvider', () => {
    it('getToken() returns null when session is undefined', async () => {
      const clerkClient = {};
      const provider = new ClerkAuthProvider(clerkClient);

      const result = await provider.getToken();

      expect(result).toBeNull();
    });

    it('getToken() delegates to clerk session.getToken()', async () => {
      const token = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const clerkClient = {
        session: {
          getToken: jest.fn().mockResolvedValue(token),
        },
      };
      const provider = new ClerkAuthProvider(clerkClient);

      const result = await provider.getToken();

      expect(result).toBe(token);
      expect(clerkClient.session.getToken).toHaveBeenCalledTimes(1);
    });
  });

  describe('FirebaseAuthProvider', () => {
    it('initialize() subscribes to onIdTokenChanged', () => {
      const unsubscribe = jest.fn();
      const firebaseAuth = {
        currentUser: { getIdToken: jest.fn().mockResolvedValue('token') },
        onIdTokenChanged: jest.fn().mockReturnValue(unsubscribe),
      };
      const provider = new FirebaseAuthProvider(firebaseAuth);

      provider.initialize();

      expect(firebaseAuth.onIdTokenChanged).toHaveBeenCalledTimes(1);
      expect(firebaseAuth.onIdTokenChanged).toHaveBeenCalledWith(expect.any(Function));
    });

    it('destroy() unsubscribes from onIdTokenChanged', () => {
      const unsubscribe = jest.fn();
      const firebaseAuth = {
        currentUser: { getIdToken: jest.fn().mockResolvedValue('token') },
        onIdTokenChanged: jest.fn().mockReturnValue(unsubscribe),
      };
      const provider = new FirebaseAuthProvider(firebaseAuth);

      provider.initialize();
      provider.destroy();

      expect(unsubscribe).toHaveBeenCalledTimes(1);
    });
  });

  describe('BetterAuthProvider', () => {
    it('getToken() extracts token from session object', async () => {
      const token = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const betterAuthClient = {
        getSession: jest.fn().mockResolvedValue({ token }),
      };
      const provider = new BetterAuthProvider(betterAuthClient);

      const result = await provider.getToken();

      expect(result).toBe(token);
      expect(betterAuthClient.getSession).toHaveBeenCalledTimes(1);
    });
  });

  describe('BaseAuthProvider (via CustomAuthProvider)', () => {
    it('emits auth:error event when fetchExternalToken throws', async () => {
      const error = new Error('Session expired');
      const getTokenFn = jest.fn().mockRejectedValue(error);
      const provider = new CustomAuthProvider(getTokenFn);

      const events: AuthEvent[] = [];
      provider.onAuthEvent((event) => events.push(event));

      const result = await provider.getToken();

      expect(result).toBeNull();
      expect(events).toHaveLength(1);
      expect(events[0].type).toBe('auth:error');
      expect(events[0].error).toBe(error);
    });

    it('token exchange POSTs to /api/auth/token and returns exchanged JWT', async () => {
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          providerName: 'custom',
        },
      });

      // Mock global fetch
      const mockFetch = jest.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken }),
      });
      global.fetch = mockFetch;

      const result = await provider.getToken();

      expect(result).toBe(topgunToken);
      expect(mockFetch).toHaveBeenCalledWith(
        'http://localhost:8080/api/auth/token',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ token: externalToken, provider: 'custom' }),
        }
      );
    });

    it('correctly extracts exp from a JWT payload', async () => {
      const expTime = Math.floor(Date.now() / 1000) + 3600;
      const token = createMockJwt(expTime);
      const getTokenFn = jest.fn().mockResolvedValue(token);
      const provider = new CustomAuthProvider(getTokenFn);

      // Call getToken to trigger caching with expiry extraction
      await provider.getToken();

      // Second call should return cached token (proving expiry was correctly parsed
      // and the token is not within the refresh margin)
      const result = await provider.getToken();
      expect(result).toBe(token);
      expect(getTokenFn).toHaveBeenCalledTimes(1);
    });

    it('concurrent getToken() calls invoke fetchExternalToken exactly once', async () => {
      const token = createMockJwt(Math.floor(Date.now() / 1000) + 3600);

      // Use a delayed resolution to ensure concurrency
      let resolveToken: (value: string) => void;
      const getTokenFn = jest.fn().mockImplementation(
        () =>
          new Promise<string>((resolve) => {
            resolveToken = resolve;
          })
      );
      const provider = new CustomAuthProvider(getTokenFn);

      // Start two concurrent getToken() calls
      const promise1 = provider.getToken();
      const promise2 = provider.getToken();

      // Allow pending microtasks to settle before resolving the token
      await Promise.resolve();
      resolveToken!(token);

      const [result1, result2] = await Promise.all([promise1, promise2]);

      expect(result1).toBe(token);
      expect(result2).toBe(token);
      expect(getTokenFn).toHaveBeenCalledTimes(1);
    });

    // ── Refresh token flow (AC8, AC9) ────────────────────────────────────────

    it('AC8: stores refresh token from token exchange response when enableRefresh is true', async () => {
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);
      const refreshToken = 'a'.repeat(64); // opaque 64-char hex

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
      });

      const mockFetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken, refreshExpiresAt: 9999999 }),
      });
      global.fetch = mockFetch;

      await provider.getToken();

      // Force expiry of cached token so next call triggers a refresh.
      // At this point the stored refresh token should be used before
      // falling back to fetchExternalToken.
      const newAccessToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);
      mockFetch.mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: newAccessToken, refreshToken: 'b'.repeat(64) }),
      });

      // Expire the cached token.
      (provider as any).cachedToken = null;
      (provider as any).cachedTokenExpiry = 0;

      const result = await provider.getToken();
      expect(result).toBe(newAccessToken);
      // fetchExternalToken should NOT have been called a second time because
      // the server refresh succeeded.
      expect(getTokenFn).toHaveBeenCalledTimes(1);
      // Second fetch call was to /api/auth/refresh
      expect(mockFetch).toHaveBeenCalledTimes(2);
      expect(mockFetch.mock.calls[1][0]).toBe('http://localhost:8080/api/auth/refresh');
    });

    it('AC9: falls back to fetchExternalToken when server refresh returns 401', async () => {
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);
      const refreshToken = 'c'.repeat(64);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
      });

      // First call: successful token exchange that stores a refresh token.
      const mockFetch = jest.fn()
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({ token: topgunToken, refreshToken }),
        })
        // Second call: server refresh returns 401 (consumed or expired).
        .mockResolvedValueOnce({ ok: false, status: 401 })
        // Third call: token exchange after fallback to fetchExternalToken.
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({ token: topgunToken }),
        });
      global.fetch = mockFetch;

      await provider.getToken();

      // Expire the cached token.
      (provider as any).cachedToken = null;
      (provider as any).cachedTokenExpiry = 0;

      const result = await provider.getToken();
      expect(result).toBe(topgunToken);
      // fetchExternalToken was called twice: initial + fallback after 401.
      expect(getTokenFn).toHaveBeenCalledTimes(2);
    });

    it('persists refresh token to storage on exchange', async () => {
      const storage = createMockStorageAdapter();
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);
      const refreshToken = 'persist-me-' + 'x'.repeat(53);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken }),
      });

      await provider.getToken();

      expect(storage.setMeta).toHaveBeenCalledWith('topgun:refreshToken', refreshToken);
      expect(storage.store.get('topgun:refreshToken')).toBe(refreshToken);
    });

    it('restores refresh token from storage on first getToken()', async () => {
      const storage = createMockStorageAdapter();
      const storedRefresh = 'stored-refresh-' + 'y'.repeat(49);
      storage.store.set('topgun:refreshToken', storedRefresh);

      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const newAccessToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      // Server refresh succeeds using the restored token
      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: newAccessToken, refreshToken: 'rotated' }),
      });

      const result = await provider.getToken();
      expect(result).toBe(newAccessToken);
      // fetchExternalToken should NOT have been called because server refresh succeeded
      expect(getTokenFn).not.toHaveBeenCalled();
      // The refresh endpoint was called with the stored token
      expect(global.fetch).toHaveBeenCalledWith(
        'http://localhost:8080/api/auth/refresh',
        expect.objectContaining({
          body: JSON.stringify({ refreshToken: storedRefresh }),
        })
      );
    });

    it('clears refresh token from storage on invalidateCache()', async () => {
      const storage = createMockStorageAdapter();
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken: 'to-clear' }),
      });

      await provider.getToken();
      expect(storage.store.has('topgun:refreshToken')).toBe(true);

      // invalidateCache is protected, access via cast
      (provider as any).invalidateCache();
      expect(storage.setMeta).toHaveBeenCalledWith('topgun:refreshToken', null);
    });

    it('clears refresh token from storage on destroy()', async () => {
      const storage = createMockStorageAdapter();
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken: 'to-destroy' }),
      });

      await provider.getToken();
      provider.destroy();
      expect(storage.setMeta).toHaveBeenCalledWith('topgun:refreshToken', null);
    });

    it('clears refresh token from storage on refresh failure', async () => {
      const storage = createMockStorageAdapter();
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      const mockFetch = jest.fn()
        // First call: exchange succeeds with refresh token
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({ token: topgunToken, refreshToken: 'will-fail' }),
        })
        // Second call: refresh returns 401
        .mockResolvedValueOnce({ ok: false, status: 401 })
        // Third call: re-exchange after fallback
        .mockResolvedValueOnce({
          ok: true,
          json: () => Promise.resolve({ token: topgunToken }),
        });
      global.fetch = mockFetch;

      await provider.getToken();
      expect(storage.store.has('topgun:refreshToken')).toBe(true);

      // Expire cached token to force refresh attempt
      (provider as any).cachedToken = null;
      (provider as any).cachedTokenExpiry = 0;

      await provider.getToken();
      // After 401, the stored token should have been cleared
      expect(storage.setMeta).toHaveBeenCalledWith('topgun:refreshToken', null);
    });

    it('storage errors are non-fatal', async () => {
      const storage = createMockStorageAdapter();
      storage.setMeta = jest.fn().mockRejectedValue(new Error('IndexedDB quota exceeded'));
      storage.getMeta = jest.fn().mockRejectedValue(new Error('IndexedDB unavailable'));

      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
        storageAdapter: storage,
      });

      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken: 'wont-persist' }),
      });

      // Should not throw despite storage failures
      const result = await provider.getToken();
      expect(result).toBe(topgunToken);
    });

    it('no adapter means memory-only behavior (existing tests unaffected)', async () => {
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      // No storageAdapter provided
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          enableRefresh: true,
        },
      });

      global.fetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken: 'memory-only' }),
      });

      const result = await provider.getToken();
      expect(result).toBe(topgunToken);
      // No crash, behavior is identical to before
    });

    it('token exchange without enableRefresh does not store refresh token', async () => {
      const externalToken = createMockJwt(Math.floor(Date.now() / 1000) + 3600);
      const topgunToken = createMockJwt(Math.floor(Date.now() / 1000) + 7200);
      const refreshToken = 'd'.repeat(64);

      const getTokenFn = jest.fn().mockResolvedValue(externalToken);
      const provider = new CustomAuthProvider(getTokenFn, {
        tokenExchangeConfig: {
          serverUrl: 'http://localhost:8080',
          // enableRefresh not set (defaults to false/undefined)
        },
      });

      const mockFetch = jest.fn().mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve({ token: topgunToken, refreshToken }),
      });
      global.fetch = mockFetch;

      await provider.getToken();

      // refreshTokenValue should remain null even though server returned one.
      expect((provider as any).refreshTokenValue).toBeNull();
    });
  });
});
