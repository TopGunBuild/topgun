import { decodeBase64Url } from '@topgunbuild/core';

const BASE64URL = /^[A-Za-z0-9_-]+$/;

/**
 * The `exp` claim (seconds since the epoch) of a JWT, read without verifying
 * the token, or `null` when the token is not a three-part JWT, its payload is
 * not base64url-encoded JSON, or it carries no numeric `exp`.
 *
 * Only scheduling uses it: whether a token is accepted is decided by the
 * server, never by this value.
 */
export function decodeJwtExp(token: string): number | null {
  const parts = token.split('.');
  if (parts.length !== 3 || !BASE64URL.test(parts[1])) return null;
  try {
    const payload: unknown = JSON.parse(decodeBase64Url(parts[1]));
    if (payload === null || typeof payload !== 'object') return null;
    const exp = (payload as { exp?: unknown }).exp;
    return typeof exp === 'number' && Number.isFinite(exp) ? exp : null;
  } catch {
    return null;
  }
}
