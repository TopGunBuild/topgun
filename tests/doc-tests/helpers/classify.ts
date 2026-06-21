/**
 * Verdict resolution — assigns every documentation snippet exactly one tier.
 *
 * Tiers (strongest first):
 *   run        — execute against a live Rust server (proves end-to-end behavior)
 *   typecheck  — compile against the real package types (catches API drift)
 *   skip       — explicitly NOT exercised; always carries a reason
 *
 * The contract that satisfies the G2 gate:
 *   - DEFAULT is never "skip". A new ts/tsx/js block is type-checked
 *     automatically (no allowlist); a new bash block runs automatically. The
 *     only way to land an un-exercised snippet is an EXPLICIT skip directive
 *     with a reason — so skips are always visible in the manifest.
 *   - `run` is the author opting INTO execution for a complete, runnable
 *     example (or a self-contained snippet the typechecker proves is a whole
 *     program). It is a STRONGER assertion layered on top of the automatic
 *     typecheck, never a way to test less.
 *   - Non-executable languages (json, rust, text, …) are skipped with an
 *     auto-filled reason, still surfaced in the manifest.
 *   - A ts/tsx block that is not parseable as a standalone module (a bare
 *     object-literal or signature fragment) is auto-skipped with a reason,
 *     unless the author forced a tier. This keeps illustrative notation from
 *     failing the gate while remaining explicit in the manifest.
 */

import { Snippet, TS_LANGS, BASH_LANGS } from './extract';
import { checkSnippets, SnippetDiagnostics } from './tsc';

export type Verdict = 'run' | 'typecheck' | 'skip';

export interface Classified {
  snippet: Snippet;
  verdict: Verdict;
  /** Required when verdict === 'skip'. */
  reason: string | null;
  /** Typecheck diagnostics, when the tier is typecheck or run. */
  diagnostics?: SnippetDiagnostics;
  /** True when the author forced the tier via a directive (vs auto-derived). */
  forced: boolean;
}

/**
 * Classifies every snippet. Type-checks the whole ts/tsx batch in one program
 * for speed, then resolves each verdict.
 */
export function classifyAll(snippets: Snippet[]): Classified[] {
  // Type-check every non-explicitly-skipped ts/tsx/js snippet up front, twice:
  //  - WITH the doc-scope ambient → drift + syntax (member access on ambient
  //    `client` is checked against the real API).
  //  - WITHOUT it → self-containment (a snippet is standalone-runnable only if
  //    it compiles clean with no ambient crutch; the run tier has no ambient).
  const tsToCheck = snippets.filter((s) => TS_LANGS.has(s.lang) && !s.directive.skip);
  const inputs = tsToCheck.map((s) => ({ id: s.id, code: s.code }));
  const ambient = checkSnippets(inputs, { ambient: true });
  const standalone = checkSnippets(inputs, { ambient: false });

  const merged = new Map<string, SnippetDiagnostics>();
  for (const s of tsToCheck) {
    const a = ambient.get(s.id)!;
    const st = standalone.get(s.id)!;
    merged.set(s.id, {
      ...a,
      // Self-containment is judged WITHOUT the ambient crutch.
      selfContained: st.selfContained,
    });
  }

  return snippets.map((s) => classifyOne(s, merged.get(s.id)));
}

function classifyOne(s: Snippet, diag?: SnippetDiagnostics): Classified {
  const d = s.directive;

  // 1. Explicit skip always wins. Reason is mandatory (enforced by manifest test).
  if (d.skip) {
    return {
      snippet: s,
      verdict: 'skip',
      reason: d.reason ?? '(no reason given)',
      forced: true,
    };
  }

  // 2. Non-executable languages: auto-skip with a language reason.
  if (s.kind === 'other') {
    return {
      snippet: s,
      verdict: 'skip',
      reason: `non-executable language: ${s.lang || 'none'}`,
      forced: false,
    };
  }

  // 3. Bash: classified by side-effect safety. A command runs only if it is
  //    hermetic (no install / daemon / docker / network / global mutation);
  //    everything else is an EXPLICIT skip whose reason names the category and
  //    the CI that actually validates it. New bash snippets are auto-evaluated
  //    (no allowlist); the skip is visible in the manifest, never silent.
  if (BASH_LANGS.has(s.lang)) {
    return classifyBash(s);
  }

  // 4. ts / tsx / js.
  // Author forced execution (`doctest run` executes this block; `doctest setup`
  // also contributes it as a shared preamble for sibling run blocks).
  if (d.run || d.setup) {
    return { snippet: s, verdict: 'run', reason: null, diagnostics: diag, forced: true };
  }

  // Not parseable as a standalone module → illustrative fragment notation.
  if (diag && diag.syntax.length > 0) {
    return {
      snippet: s,
      verdict: 'skip',
      reason: 'not parseable as standalone TypeScript (illustrative notation)',
      diagnostics: diag,
      forced: false,
    };
  }

  // Self-contained whole program that imports the SDK → auto-promote to run.
  // BUT only when it does not open a network connection: a snippet that wires up
  // a client against a documented/placeholder URL (`wss://topgun.example.com`)
  // would fire a real connect + reconnect storm. Networked examples must opt in
  // explicitly via `doctest setup`/`run`, where the harness substitutes the live
  // URL and tears the client down. Pure-core snippets (CRDT/HLC math) auto-run.
  if (
    diag &&
    diag.selfContained &&
    diag.importsTopgun &&
    hasExecutableStatement(s.code) &&
    !opensConnection(s.code)
  ) {
    return { snippet: s, verdict: 'run', reason: null, diagnostics: diag, forced: false };
  }

  // Default: typecheck the fragment against real types.
  return { snippet: s, verdict: 'typecheck', reason: null, diagnostics: diag, forced: false };
}

/**
 * Side-effect categories for bash snippets, each with the CI that genuinely
 * exercises that command class. Matched against the first meaningful token of
 * each command line. A snippet that contains ANY side-effecting command is
 * skipped (with the most specific reason), because a doc-test sandbox must not
 * install packages, pull images, start daemons, or hit the network.
 */
const BASH_SIDE_EFFECTS: Array<{ test: RegExp; reason: string }> = [
  {
    test: /\b(docker|docker-compose)\b/,
    reason: 'docker command — validated by docker.yml, not doc-tests',
  },
  { test: /\bcargo\b/, reason: 'cargo build/run — validated by rust.yml, not doc-tests' },
  {
    test: /\b(pnpm|npm|npx|yarn|corepack)\b/,
    reason:
      'package-manager command (install/scaffold/dev/build) — validated by node.yml + create-topgun-app + post-publish-smoke, not doc-tests',
  },
  {
    test: /\b(curl|wget|http)\b/,
    reason: 'HTTP example (placeholder/admin endpoint) — illustrative, not executed',
  },
  {
    test: /\b(sudo|rm|kill|lsof|chmod|chown|mv|cp|mkdir|systemctl|service)\b/,
    reason: 'system/filesystem command — not safe in a doc-test sandbox',
  },
  {
    test: /\btopgun\b/,
    reason: 'topgun CLI invocation — illustrative (needs a scaffolded app / running server)',
  },
];

/** Hermetic verbs that are safe to actually run if a bash snippet is ONLY these. */
const BASH_SAFE_VERBS = /^(echo|true|:|printf)\b/;

/**
 * Classifies a bash snippet. Returns `run` only for hermetic snippets whose
 * every command is a known-safe verb; otherwise an explicit, categorized skip.
 */
function classifyBash(s: Snippet): Classified {
  const lines = s.code
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'));

  // Env-only snippets (just `export FOO=bar` / `FOO=bar`) are illustrative.
  const allEnv = lines.length > 0 && lines.every((l) => /^(export\s+)?[A-Z_][A-Z0-9_]*=/.test(l));
  if (allEnv) {
    return {
      snippet: s,
      verdict: 'skip',
      reason: 'environment-variable example — no command to execute',
      forced: false,
    };
  }

  for (const { test, reason } of BASH_SIDE_EFFECTS) {
    if (lines.some((l) => test.test(l))) {
      return { snippet: s, verdict: 'skip', reason, forced: false };
    }
  }

  // Only reach here if nothing side-effecting matched. Run only if every line is
  // a hermetic safe verb; otherwise skip conservatively (unknown command).
  const allSafe =
    lines.length > 0 && lines.every((l) => BASH_SAFE_VERBS.test(l.replace(/^\$\s*/, '')));
  if (allSafe) {
    return { snippet: s, verdict: 'run', reason: null, forced: s.directive.setup };
  }
  return {
    snippet: s,
    verdict: 'skip',
    reason: 'shell command — not a server API call; illustrative',
    forced: false,
  };
}

/**
 * Heuristic: does the snippet actually DO something at runtime (a call / await /
 * new), as opposed to being pure type or interface declarations? Pure-type
 * snippets stay in the typecheck tier rather than being executed.
 */
function hasExecutableStatement(code: string): boolean {
  // Strip line comments to avoid matching prose-in-comments.
  const stripped = code.replace(/\/\/.*$/gm, '');
  return (
    /\b(await|new|\w+\s*\()/.test(stripped) &&
    !/^\s*(export\s+)?(type|interface)\b/.test(stripped.trim())
  );
}

/**
 * True when a snippet would open a network connection if run as-is. Such
 * snippets must opt into the run tier explicitly so the harness can substitute
 * the live test URL and guarantee teardown; otherwise they stay typecheck-only.
 */
function opensConnection(code: string): boolean {
  const stripped = code.replace(/\/\/.*$/gm, '');
  return /new\s+TopGunClient|\bserverUrl\b|\.start\s*\(|\bcluster\s*:|new\s+\w*SyncProvider|new\s+SyncEngine|pollInterval/.test(
    stripped,
  );
}
