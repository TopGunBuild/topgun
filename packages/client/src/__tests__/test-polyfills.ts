/**
 * Test polyfills for Node.js environment
 *
 * Provides WebCrypto API polyfill using Node.js crypto module
 */

import * as crypto from 'crypto';

// Extend globalThis for WebCrypto polyfill (Node.js environment)
declare global {
  // eslint-disable-next-line no-var
  var crypto: Crypto;
}

// Polyfill WebCrypto for Node environment if needed
if (!globalThis.crypto) {
  globalThis.crypto = crypto.webcrypto as Crypto;
}

// Create window object if it doesn't exist (Node.js environment)
if (!globalThis.window) {
  Object.defineProperty(globalThis, 'window', {
    value: globalThis,
    writable: true,
    configurable: true,
  });
}

// Set window.crypto if it doesn't exist
if (!window.crypto) {
  Object.defineProperty(window, 'crypto', {
    value: crypto.webcrypto,
    writable: true,
    configurable: true,
  });
}
