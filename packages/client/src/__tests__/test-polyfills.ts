/**
 * Test polyfills for Node.js environment
 *
 * Provides WebCrypto API polyfill using Node.js crypto module
 */

import * as crypto from 'crypto';

// Extend globalThis for WebCrypto polyfill
declare global {
  // eslint-disable-next-line no-var
  var crypto: Crypto;
  // eslint-disable-next-line no-var
  var window: typeof globalThis & { crypto: Crypto };
}

// Polyfill WebCrypto for Node environment if needed
if (!globalThis.crypto) {
  globalThis.crypto = crypto.webcrypto as Crypto;
}
if (!globalThis.window) {
  globalThis.window = globalThis as typeof globalThis & { crypto: Crypto };
}
if (!window.crypto) {
  window.crypto = crypto.webcrypto as Crypto;
}
