/**
 * Error surfaced when the server requires authentication (sends AUTH_REQUIRED)
 * but the client has no token and no token provider configured.
 *
 * Without this, the SyncEngine would park in AUTHENTICATING forever — visible
 * only as an info-level log line. AuthRequiredError gives integrators a typed
 * hook (via TopGunClientConfig.onAuthRequired) to detect the deadlock and react
 * (prompt for login, call setAuthToken, redirect, etc.).
 */
export class AuthRequiredError extends Error {
  public readonly name = 'AuthRequiredError';
  public readonly code = 'AUTH_REQUIRED_NO_TOKEN';

  constructor() {
    super(
      'Server requires authentication but no token is configured. ' +
        'Call client.setAuthToken(token), client.setAuthTokenProvider(fn), or pass `auth` to TopGunClient.'
    );

    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, AuthRequiredError);
    }
  }
}
