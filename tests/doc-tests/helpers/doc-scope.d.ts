/**
 * Ambient "doc vocabulary" for the typecheck tier.
 *
 * Documentation fragments routinely use a handful of canonical identifiers
 * WITHOUT an import line — most commonly `client` (an instantiated
 * `TopGunClient`) and the bare value exports the docs lean on (`Predicates`,
 * `HLC`, …). Declaring those here, with their REAL types pulled from the
 * published packages, lets the typecheck tier verify member access on them:
 *
 *   client.searchSubscribe('a', 'b')   // ✅ checked against the real TopGunClient
 *   client.searchSubscrib('a', 'b')    // ❌ TS2339 — caught as API drift
 *
 * This is NOT an allowlist of snippets — it is a typed glossary of the names
 * the docs treat as ambient. Snippet-local symbols the docs invent on the spot
 * (`Article`, `todos`, `hlc`, …) stay undeclared; the typecheck tier ignores
 * "cannot find name" for those so illustrative fragments don't fail falsely,
 * while still catching misuse of the REAL API surface above.
 *
 * Keep this in sync with the public package exports. If a name here stops being
 * exported, the import below fails to compile and the whole doc-test suite goes
 * red — which is the correct signal.
 */

import type { TopGunClient } from '@topgunbuild/client';
import type { Predicates } from '@topgunbuild/client';
import type { HLC, LWWMap, ORMap, IndexedORMap, IndexedLWWMap } from '@topgunbuild/core';

declare global {
  /** The canonical instantiated client the docs reference ambiently. */
  const client: TopGunClient;

  // Bare value-exports the docs use without an import line. Typed as their real
  // shapes so member access is verified, surfacing drift if an export is
  // renamed or its signature changes.
  const Predicates: typeof import('@topgunbuild/client').Predicates;
  const HLC: typeof import('@topgunbuild/core').HLC;
  const LWWMap: typeof import('@topgunbuild/core').LWWMap;
  const ORMap: typeof import('@topgunbuild/core').ORMap;
  const IndexedORMap: typeof import('@topgunbuild/core').IndexedORMap;
  const IndexedLWWMap: typeof import('@topgunbuild/core').IndexedLWWMap;

  // Silence references to these unavoidable globals in run-tier snippets without
  // weakening API checks (they are environment, not TopGun API).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  type DocAny = any;
}

// Reference the imported types so they are not elided, keeping the module form
// required for `declare global` to take effect.
export type _DocScopeTypes = TopGunClient | Predicates | HLC | LWWMap | ORMap | IndexedORMap | IndexedLWWMap;
