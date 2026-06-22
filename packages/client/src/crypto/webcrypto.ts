/**
 * Resolves the Web Crypto API (`SubtleCrypto`) across browser and Node.
 *
 * Browsers expose it on `window.crypto` / `globalThis.crypto`; Node ≥ 20, Deno,
 * Bun, and edge runtimes expose it on `globalThis.crypto`. We resolve lazily (at
 * call time, not module load) so bundlers never need a Node built-in and SSR /
 * edge runtimes pick up their own global. Throws a clear, actionable error when
 * no implementation is available rather than failing later inside `subtle.*`.
 */
export function getWebCrypto(): Crypto {
  const candidate =
    (typeof globalThis !== 'undefined' && globalThis.crypto) ||
    (typeof window !== 'undefined' && window.crypto) ||
    undefined;

  if (!candidate?.subtle) {
    throw new Error(
      'Web Crypto API unavailable: EncryptedStorageAdapter requires ' +
        'globalThis.crypto.subtle (browsers, Node ≥ 20, Deno, Bun, or a ' +
        'WebCrypto polyfill installed on globalThis.crypto).',
    );
  }

  return candidate;
}
