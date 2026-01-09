/**
 * Universal Base64URL encoding/decoding utilities.
 *
 * Works in both Node.js and browser environments.
 * Used by QueryCursor and SearchCursor for opaque cursor encoding.
 *
 * @module utils/base64url
 */

/**
 * Encode a string to base64url format.
 * URL-safe, no padding characters.
 *
 * @param str - UTF-8 string to encode
 * @returns Base64url encoded string
 */
export function encodeBase64Url(str: string): string {
  // Node.js environment
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(str, 'utf8').toString('base64url');
  }

  // Browser environment
  const base64 = btoa(unescape(encodeURIComponent(str)));
  return base64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/**
 * Decode a base64url string back to UTF-8.
 *
 * @param encoded - Base64url encoded string
 * @returns Decoded UTF-8 string
 * @throws Error if decoding fails
 */
export function decodeBase64Url(encoded: string): string {
  // Node.js environment
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(encoded, 'base64url').toString('utf8');
  }

  // Browser environment
  let base64 = encoded.replace(/-/g, '+').replace(/_/g, '/');

  // Add padding if needed
  const pad = base64.length % 4;
  if (pad) {
    base64 += '='.repeat(4 - pad);
  }

  return decodeURIComponent(escape(atob(base64)));
}
