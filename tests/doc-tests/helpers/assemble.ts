/**
 * Run-tier assembly + execution.
 *
 * A documentation example is rarely a single self-contained block: a "setup"
 * block creates the client, later blocks write/read against it. The harness
 * mirrors the mdBook / rustdoc model — per page, `doctest setup` blocks
 * accumulate into a shared preamble, and each `run` block is executed as
 * (page-setup ++ that block) so it sees the established scope without colliding
 * with sibling run blocks.
 *
 * Execution is real: the assembled module is transpiled to CommonJS and run in
 * a Node vm context whose `require` is the Jest module registry — so
 * `@topgunbuild/client` resolves to workspace source and `@topgunbuild/adapters`
 * resolves to the in-memory shim (jest moduleNameMapper). The only source
 * rewrite is substituting the documented `ws://localhost:8080` for the live
 * ephemeral test URL; everything else runs verbatim.
 */

import * as vm from 'vm';
import * as ts from 'typescript';

/**
 * The documented authority the docs hard-code. We rewrite ONLY the host:port,
 * never the path — so a snippet's `ws://localhost:8080` vs `ws://localhost:8080/ws`
 * still connects to whatever path it actually wrote (the server dual-mounts WS
 * at `/` and `/ws`). Rewriting the whole URL would let the harness mask a wrong
 * documented path instead of catching it.
 */
const AUTHORITY_PATTERNS = [/localhost:8080/g, /127\.0\.0\.1:8080/g];

export interface AssembledModule {
  /** CommonJS source ready for vm execution. */
  source: string;
}

interface ImportSplit {
  requires: string[];
  body: string;
}

/**
 * Converts the leading `import` statements of a snippet into `require` calls and
 * returns the remaining body. Type-only imports are dropped. Handles the import
 * forms that appear in the docs: named, default, namespace, and side-effect.
 */
function splitImports(code: string): ImportSplit {
  const requires: string[] = [];
  const bodyLines: string[] = [];
  const lines = code.split('\n');

  for (const line of lines) {
    const trimmed = line.trim();

    // Drop type-only imports — they have no runtime form.
    if (/^import\s+type\b/.test(trimmed)) continue;

    // import { A, B as C } from 'x'
    let m = trimmed.match(/^import\s+\{([^}]*)\}\s+from\s+['"]([^'"]+)['"]\s*;?$/);
    if (m) {
      const named = m[1]
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean)
        .map((s) => s.replace(/\s+as\s+/, ': '))
        // Drop inline `type` modifiers in named imports.
        .map((s) => s.replace(/^type\s+/, ''))
        .join(', ');
      requires.push(`const { ${named} } = require(${JSON.stringify(m[2])});`);
      continue;
    }

    // import * as X from 'x'
    m = trimmed.match(/^import\s+\*\s+as\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?$/);
    if (m) {
      requires.push(`const ${m[1]} = require(${JSON.stringify(m[2])});`);
      continue;
    }

    // import X from 'x'  (default)
    m = trimmed.match(/^import\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?$/);
    if (m) {
      requires.push(
        `const ${m[1]} = require(${JSON.stringify(m[2])}).default ?? require(${JSON.stringify(m[2])});`,
      );
      continue;
    }

    // import X, { A } from 'x'  (default + named)
    m = trimmed.match(/^import\s+(\w+)\s*,\s*\{([^}]*)\}\s+from\s+['"]([^'"]+)['"]\s*;?$/);
    if (m) {
      const mod = JSON.stringify(m[3]);
      const named = m[2]
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean)
        .join(', ');
      requires.push(`const ${m[1]} = require(${mod}).default ?? require(${mod});`);
      requires.push(`const { ${named} } = require(${mod});`);
      continue;
    }

    // import 'x'  (side effect)
    m = trimmed.match(/^import\s+['"]([^'"]+)['"]\s*;?$/);
    if (m) {
      requires.push(`require(${JSON.stringify(m[1])});`);
      continue;
    }

    bodyLines.push(line);
  }

  return { requires, body: bodyLines.join('\n') };
}

function injectUrl(code: string, authority: string): string {
  let out = code;
  for (const p of AUTHORITY_PATTERNS) out = out.replace(p, authority);
  return out;
}

/**
 * Assembles a CommonJS module for one run block, prepended with the page's
 * accumulated setup blocks. The body runs inside an async IIFE so top-level
 * await in the docs works and the promise can be awaited by the caller.
 */
export function assembleRunModule(
  setupBlocks: string[],
  block: string,
  opts: { authority: string },
): AssembledModule {
  const all = [...setupBlocks, block].map((c) => injectUrl(c, opts.authority));
  const splits = all.map(splitImports);

  const requires = splits.flatMap((s) => s.requires).join('\n');
  const body = splits.map((s) => s.body).join('\n');

  const tsSource = `${requires}
exports.__run = (async () => {
${body}
})();
`;

  const js = ts.transpileModule(tsSource, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
      esModuleInterop: true,
    },
  }).outputText;

  return { source: js };
}

/**
 * Executes an assembled module in a fresh vm context that delegates module
 * resolution to the provided `require` (the Jest registry). Resolves when the
 * snippet's async body settles; rejects if it throws or times out.
 */
export async function executeModule(
  assembled: AssembledModule,
  requireFn: NodeJS.Require,
  timeoutMs: number,
): Promise<void> {
  const moduleObj: { exports: Record<string, unknown> } = { exports: {} };
  // UI-placeholder no-ops: wiring examples in the docs call render/display-style
  // functions they never define (the reader supplies the UI). Stubbing them lets
  // the surrounding real API calls execute headless. Documented in the harness
  // README so authors know these names are provided, not asserted.
  const uiStub = () => undefined;
  const sandbox: Record<string, unknown> = {
    require: requireFn,
    module: moduleObj,
    exports: moduleObj.exports,
    console,
    process,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    Buffer,
    URL,
    TextEncoder,
    TextDecoder,
    crypto: (globalThis as { crypto?: unknown }).crypto,
    fetch: (globalThis as { fetch?: unknown }).fetch,
    render: uiStub,
    renderTodos: uiStub,
    display: uiStub,
    updateUI: uiStub,
    showLoadingSpinner: uiStub,
    hideLoadingSpinner: uiStub,
  };
  sandbox.global = sandbox;
  sandbox.globalThis = sandbox;

  vm.runInNewContext(assembled.source, sandbox, { timeout: timeoutMs });

  const run = moduleObj.exports.__run as Promise<unknown> | undefined;
  if (!run) return;

  await Promise.race([
    run,
    new Promise((_, reject) =>
      setTimeout(
        () => reject(new Error(`run-tier snippet timed out after ${timeoutMs}ms`)),
        timeoutMs,
      ),
    ),
  ]);
}
