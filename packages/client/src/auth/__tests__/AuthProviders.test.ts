import { CustomAuthProvider } from '../CustomAuthProvider';
import { ClerkAuthProvider } from '../ClerkAuthProvider';
import { FirebaseAuthProvider } from '../FirebaseAuthProvider';
import { BetterAuthProvider } from '../BetterAuthProvider';
import type { AuthEvent } from '../types';

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

      // Resolve the token
      resolveToken!(token);

      const [result1, result2] = await Promise.all([promise1, promise2]);

      expect(result1).toBe(token);
      expect(result2).toBe(token);
      expect(getTokenFn).toHaveBeenCalledTimes(1);
    });
  });
});
