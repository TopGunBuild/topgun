/**
 * Typecheck tier — compiles documentation snippets against the REAL published
 * package types and reports only the diagnostics that indicate genuine API
 * drift (the dominant docs-overpromise vector).
 *
 * A doc fragment is not a complete program: it references symbols it never
 * declares (`Article`, `todos`, `hlc`) and imports modules that don't exist
 * outside the snippet's imagined file (`./types`). Failing on those would make
 * every illustrative fragment red and teach authors to disable the gate. So the
 * compiler runs in a deliberately split mode:
 *
 *   IGNORE  — "you referenced something you didn't declare here" (TS2304 et al).
 *             That is the nature of a fragment, not a doc bug.
 *   FAIL    — "you used the REAL API wrong": a method that doesn't exist on a
 *             TopGun type, a wrong argument, an import of a non-exported member.
 *             That is exactly the drift the gate exists to catch.
 *
 * Self-containment detection reuses the same compile: a snippet that produces
 * ZERO diagnostics of ANY kind (including the ignored fragment codes) and pulls
 * in at least one @topgunbuild import is a complete program and is eligible for
 * the run tier.
 */

import * as ts from 'typescript';
import { join } from 'path';

import { REPO_ROOT } from './extract';

/**
 * Diagnostic codes that signal real API misuse / drift. These FAIL the gate.
 * Everything else (notably "cannot find name/module/namespace") is treated as
 * the expected shape of an illustrative fragment and ignored.
 */
export const DRIFT_CODES = new Set<number>([
  2305, // Module '"X"' has no exported member 'Y'.        — import renamed/removed
  2614, // Module '"X"' has no exported member 'Y'. (namespace import variant)
  2724, // '"X"' has no exported member named 'Y'. Did you mean 'Z'?
  2339, // Property 'x' does not exist on type 'Y'.          — method renamed/removed
  2551, // Property 'x' does not exist on type 'Y'. Did you mean 'z'?
  2345, // Argument of type 'A' is not assignable to parameter of type 'B'.
  2554, // Expected N arguments, but got M.
  2555, // Expected at least N arguments, but got M.
  2353, // Object literal may only specify known properties.
  2559, // Type 'A' has no properties in common with type 'B'.
  2769, // No overload matches this call.
  2741, // Property 'x' is missing in type 'A' but required in type 'B'.
]);

/**
 * Codes that are syntax-level: a snippet producing these is not parseable as a
 * standalone TS module (e.g. a bare object-literal fragment). Such snippets are
 * auto-classified `skip` (illustrative, surfaced in the manifest) rather than
 * failed — unless the author forced a tier.
 */
export function isSyntaxError(code: number): boolean {
  return code >= 1000 && code < 2000;
}

const PRELUDE = `/// <reference path="./doc-scope.d.ts" />\n`;
const PRELUDE_LINES = PRELUDE.split('\n').length - 1;

export interface CheckOptions {
  /**
   * When true, the doc-scope ambient (`client`, `Predicates`, …) is in scope.
   * Use this for DRIFT detection — fragments that reference ambient `client`
   * get their real-API member access checked.
   *
   * When false, the snippet is compiled with no ambient crutch. Use this for
   * SELF-CONTAINMENT: a snippet is runnable standalone only if it compiles
   * clean WITHOUT the ambient (i.e. it declares its own `client` etc.). A
   * fragment that leans on ambient `client` is not standalone-runnable — the
   * run tier has no ambient — so it must stay typecheck-only.
   */
  ambient: boolean;
}

export interface SnippetInput {
  id: string;
  code: string;
}

export interface SnippetDiagnostics {
  id: string;
  /** Drift diagnostics — these must be empty for a typecheck PASS. */
  drift: string[];
  /** Syntax errors — snippet is not standalone-parseable. */
  syntax: string[];
  /** True when the snippet compiled with ZERO diagnostics of any kind. */
  selfContained: boolean;
  /** True when the snippet imports at least one @topgunbuild package. */
  importsTopgun: boolean;
}

function compilerOptions(): ts.CompilerOptions {
  const pkg = (name: string) => join(REPO_ROOT, 'packages', name, 'src', 'index.ts');
  return {
    target: ts.ScriptTarget.ESNext,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    jsx: ts.JsxEmit.ReactJSX,
    lib: ['lib.esnext.d.ts', 'lib.dom.d.ts'],
    strict: false,
    noImplicitAny: false,
    skipLibCheck: true,
    noEmit: true,
    allowJs: true,
    esModuleInterop: true,
    allowImportingTsExtensions: true,
    types: [],
    baseUrl: REPO_ROOT,
    paths: {
      '@topgunbuild/client': [pkg('client')],
      '@topgunbuild/core': [pkg('core')],
      '@topgunbuild/react': [pkg('react')],
      '@topgunbuild/adapters': [pkg('adapters')],
      '@topgunbuild/mcp-server': [pkg('mcp-server')],
      '@topgunbuild/schema': [pkg('schema')],
    },
  };
}

const HELPERS_DIR = __dirname;

/**
 * Compiles a batch of snippets in a single program and returns per-snippet
 * diagnostics. Batching shares the lib + package-types load across all
 * snippets, so checking ~200 fragments costs roughly one program build.
 */
export function checkSnippets(
  snippets: SnippetInput[],
  opts: CheckOptions = { ambient: true },
): Map<string, SnippetDiagnostics> {
  const options = compilerOptions();
  const prelude = opts.ambient ? PRELUDE : '';
  const preludeLines = opts.ambient ? PRELUDE_LINES : 0;
  const virtualName = (i: number) => join(HELPERS_DIR, `__doc_snippet_${i}.tsx`);
  const sources = new Map<string, string>();
  snippets.forEach((s, i) => {
    // Force MODULE scope with a trailing `export {}`. Without it, a snippet that
    // has no import/export is a SCRIPT, and its top-level `const client = …`
    // leaks as a program-wide GLOBAL — contaminating sibling snippets (bare
    // `client` would wrongly resolve, masking both drift and self-containment).
    // As a module, every snippet's top-level bindings are isolated.
    sources.set(virtualName(i), prelude + s.code + '\nexport {};\n');
  });

  const host = ts.createCompilerHost(options, true);
  const originalGetSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (fileName, languageVersion, onError, shouldCreate) => {
    if (sources.has(fileName)) {
      return ts.createSourceFile(
        fileName,
        sources.get(fileName)!,
        languageVersion,
        true,
        ts.ScriptKind.TSX,
      );
    }
    return originalGetSourceFile(fileName, languageVersion, onError, shouldCreate);
  };
  const originalFileExists = host.fileExists.bind(host);
  host.fileExists = (fileName) => sources.has(fileName) || originalFileExists(fileName);
  const originalReadFile = host.readFile.bind(host);
  host.readFile = (fileName) =>
    sources.has(fileName) ? sources.get(fileName) : originalReadFile(fileName);

  const program = ts.createProgram([...sources.keys()], options, host);

  const result = new Map<string, SnippetDiagnostics>();
  snippets.forEach((s, i) => {
    const file = program.getSourceFile(virtualName(i));
    const diags = file
      ? [...program.getSyntacticDiagnostics(file), ...program.getSemanticDiagnostics(file)]
      : [];

    const drift: string[] = [];
    const syntax: string[] = [];
    let anyDiag = false;
    for (const d of diags) {
      anyDiag = true;
      const msg = formatDiag(d, preludeLines);
      if (isSyntaxError(d.code)) syntax.push(msg);
      else if (DRIFT_CODES.has(d.code) && !isFragmentNoise(d)) drift.push(msg);
    }

    result.set(s.id, {
      id: s.id,
      drift,
      syntax,
      selfContained: !anyDiag,
      importsTopgun: /from\s+['"]@topgunbuild\//.test(s.code),
    });
  });

  return result;
}

/**
 * Property-access diagnostics (2339/2551) are only meaningful when the target
 * is a REAL package type. When the target is `unknown` (a loosely-typed
 * callback param the fragment never annotated) or an inline object literal the
 * snippet invented locally, the access says nothing about TopGun's API — it is
 * fragment noise, not drift.
 */
function isFragmentNoise(d: ts.Diagnostic): boolean {
  if (d.code !== 2339 && d.code !== 2551) return false;
  const text = ts.flattenDiagnosticMessageText(d.messageText, '\n');
  // "...does not exist on type 'unknown'" — untyped local, not the public API.
  if (/type '(unknown|any|never|\{)/.test(text)) return true;
  return false;
}

function formatDiag(d: ts.Diagnostic, preludeLines: number): string {
  const text = ts.flattenDiagnosticMessageText(d.messageText, '\n');
  if (d.file && d.start != null) {
    const { line, character } = d.file.getLineAndCharacterOfPosition(d.start);
    // Subtract the injected prelude so the line maps to the snippet body.
    const snippetLine = line - preludeLines + 1;
    return `TS${d.code} (snippet line ${snippetLine}, col ${character + 1}): ${text}`;
  }
  return `TS${d.code}: ${text}`;
}
