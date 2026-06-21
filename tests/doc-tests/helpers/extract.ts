/**
 * Documentation snippet extractor — the source of truth for the G2 doc-test gate.
 *
 * Walks every published documentation source (README + apps/docs-astro MDX) and
 * yields one structured record per fenced code block, together with any
 * `doctest` directive that governs how the block is treated. The runner
 * (manifest.test.ts) consumes these records and either EXECUTES the snippet
 * against a live Rust server or records an EXPLICIT skip — never a silent one.
 *
 * WHY extract from MDX sources and not the rendered site or llms-full.txt:
 *   - llms-full.txt is a GENERATED artifact (scripts/build-llms-full.mjs) whose
 *     code blocks are a strict subset of the MDX it is built from. Testing the
 *     MDX sources subsumes it; re-extracting the derived file would double-count
 *     the same snippets and drift the moment the allowlist changes.
 *   - The rendered HTML loses the fence info-string and the authoring comments
 *     that carry doctest directives.
 *
 * Directive model (see tests/doc-tests/README.md for the authoring contract):
 *   - DEFAULT = RUN. Every ts / tsx / typescript / js / javascript / bash / sh
 *     block is executable by default, so a newly-added snippet is picked up
 *     automatically — there is no allowlist of "tested" snippets.
 *   - To opt a block OUT, the author writes an explicit directive carrying a
 *     reason. Skips are therefore always visible in the manifest, never silent.
 *   - Non-executable languages (json, text, rust, http, nginx, …) are
 *     classified `skip` with an auto-filled reason, again surfaced in the
 *     manifest.
 *
 * Directive carriers (any of, checked in this order):
 *   1. A comment line IMMEDIATELY above the fence (one optional blank line
 *      allowed). Render-invisible in both flavors:
 *        MDX:  {/* doctest skip reason="needs a browser DOM" *␣/}
 *        md:   <!-- doctest skip reason="needs a browser DOM" -->
 *   2. Tokens appended to the fence info string:
 *        ```ts doctest-skip reason="needs a browser DOM"
 *
 * Grammar: the keyword `doctest` followed by space-separated tokens. Recognised
 * tokens: `skip`, `setup`, `vector`, `expect-error`, and `reason="..."`
 * (`reason='...'` and bare trailing text after `skip:` also accepted).
 */

import { readFileSync, readdirSync, statSync } from 'fs';
import { join, relative } from 'path';

/** Repository root — three levels up from tests/doc-tests/helpers/. */
export const REPO_ROOT = join(__dirname, '..', '..', '..');

/** Languages whose blocks the harness can execute. */
export const TS_LANGS = new Set(['ts', 'tsx', 'typescript', 'js', 'javascript', 'jsx']);
export const BASH_LANGS = new Set(['bash', 'sh', 'shell']);

export type SnippetKind = 'ts' | 'bash' | 'other';

export interface DoctestDirective {
  /** Author explicitly marked the block illustrative — do not execute. */
  skip: boolean;
  /** Block establishes shared page scope consumed by later blocks on the page. */
  setup: boolean;
  /** Block is forced into the run tier (executed; not a shared preamble). */
  run: boolean;
  /** Block requires the vector/embedding-enabled server profile. */
  vector: boolean;
  /** Block is a negative example: executing it is expected to throw. */
  expectError: boolean;
  /** Human-readable reason (required when skip is true). */
  reason: string | null;
  /** Where the directive came from, for diagnostics. */
  source: 'comment' | 'fence' | 'none';
}

export interface Snippet {
  /** Repo-relative source path. */
  file: string;
  /** 1-based line number of the opening fence. */
  line: number;
  /** Lower-cased fence language (`` ``` `` with no language → ''). */
  lang: string;
  /** Raw fence info string after the language token. */
  meta: string;
  /** The code between the fences, verbatim. */
  code: string;
  /** Coarse routing bucket. */
  kind: SnippetKind;
  /** Parsed doctest directive (defaults to run). */
  directive: DoctestDirective;
  /** Stable identifier: `${file}:${line}`. */
  id: string;
}

const NO_DIRECTIVE: DoctestDirective = {
  skip: false,
  setup: false,
  run: false,
  vector: false,
  expectError: false,
  reason: null,
  source: 'none',
};

/** Default doc roots, relative to the repo root. */
export const DOC_SOURCES = {
  readme: 'README.md',
  docsContent: join('apps', 'docs-astro', 'src', 'content'),
};

function langKind(lang: string): SnippetKind {
  if (TS_LANGS.has(lang)) return 'ts';
  if (BASH_LANGS.has(lang)) return 'bash';
  return 'other';
}

/** Recursively collect .md / .mdx files under a directory. */
function collectMarkdown(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      collectMarkdown(full, out);
    } else if (entry.endsWith('.md') || entry.endsWith('.mdx')) {
      out.push(full);
    }
  }
}

/**
 * Returns the absolute paths of every documentation source file, in a stable
 * (sorted) order so the manifest is deterministic.
 */
export function docFiles(): string[] {
  const files: string[] = [join(REPO_ROOT, DOC_SOURCES.readme)];
  collectMarkdown(join(REPO_ROOT, DOC_SOURCES.docsContent), files);
  return files.sort();
}

/**
 * Parses a `doctest …` directive body (the text after the `doctest` keyword)
 * into a structured directive. `source` records where it was found.
 */
function parseDirectiveBody(body: string, source: 'comment' | 'fence'): DoctestDirective {
  const directive: DoctestDirective = { ...NO_DIRECTIVE, source };

  // Pull out reason="…" / reason='…' first, then strip it so the remaining
  // tokens parse cleanly.
  const reasonMatch = body.match(/reason\s*=\s*("([^"]*)"|'([^']*)')/);
  if (reasonMatch) {
    directive.reason = (reasonMatch[2] ?? reasonMatch[3] ?? '').trim();
    body = body.replace(reasonMatch[0], ' ');
  }

  // `skip: free text reason` shorthand — everything after the colon is the reason.
  const skipColon = body.match(/\bskip\s*:\s*(.+)$/);
  if (skipColon && !directive.reason) {
    directive.reason = skipColon[1].trim();
  }

  const tokens = body.split(/\s+/).filter(Boolean);
  for (const tok of tokens) {
    const t = tok.replace(/[:].*$/, '').toLowerCase();
    if (t === 'skip' || t === 'doctest-skip') directive.skip = true;
    else if (t === 'setup' || t === 'doctest-setup') directive.setup = true;
    else if (t === 'run' || t === 'doctest-run') directive.run = true;
    else if (t === 'vector' || t === 'doctest-vector') directive.vector = true;
    else if (t === 'expect-error' || t === 'expecterror') directive.expectError = true;
  }

  return directive;
}

/**
 * Looks for a doctest directive in the fence info string. Recognises the
 * `doctest …` keyword form and the dash forms (`doctest-skip`, `doctest-setup`,
 * `doctest-vector`).
 */
function directiveFromMeta(meta: string): DoctestDirective | null {
  if (!/\bdoctest/.test(meta)) return null;
  // Normalise `doctest-skip` → `doctest skip` so a single parser handles both.
  const normalised = meta.replace(/doctest-(\w+)/g, 'doctest $1');
  const m = normalised.match(/\bdoctest\b(.*)$/);
  return parseDirectiveBody(m ? m[1] : '', 'fence');
}

/**
 * Looks backwards from the fence for a directive comment on the nearest
 * preceding non-blank line (one optional blank line allowed between the comment
 * and the fence). Matches both MDX `{/* … *␣/}` and HTML `<!-- … -->` comments
 * that contain a `doctest` keyword.
 */
function directiveFromComment(lines: string[], fenceIndex: number): DoctestDirective | null {
  for (let i = fenceIndex - 1; i >= 0 && i >= fenceIndex - 2; i--) {
    const line = lines[i].trim();
    if (line === '') continue;
    const mdx = line.match(/^\{\/\*\s*(.*?)\s*\*\/\}$/);
    const html = line.match(/^<!--\s*(.*?)\s*-->$/);
    const inner = mdx ? mdx[1] : html ? html[1] : null;
    if (inner && /\bdoctest\b/.test(inner)) {
      const m = inner.match(/\bdoctest\b(.*)$/);
      return parseDirectiveBody(m ? m[1] : '', 'comment');
    }
    // First non-blank, non-directive line ends the search.
    return null;
  }
  return null;
}

/**
 * Extracts every fenced code block from a single file's text.
 */
export function extractFromText(text: string, fileLabel: string): Snippet[] {
  const lines = text.split('\n');
  const snippets: Snippet[] = [];

  let i = 0;
  while (i < lines.length) {
    const open = lines[i].match(/^(\s*)(```+|~~~+)\s*([^\s`]*)\s*(.*)$/);
    if (!open) {
      i++;
      continue;
    }
    const indent = open[1];
    const fence = open[2][0]; // ` or ~
    const fenceLen = open[2].length;
    const lang = open[3].toLowerCase();
    const meta = open[4].trim();
    const openLine = i;

    // Find the matching closing fence (same char, >= length, same-ish indent).
    let j = i + 1;
    const body: string[] = [];
    let closed = false;
    const closeRe = new RegExp(`^\\s*${fence === '`' ? '`' : '~'}{${fenceLen},}\\s*$`);
    while (j < lines.length) {
      if (closeRe.test(lines[j])) {
        closed = true;
        break;
      }
      body.push(lines[j]);
      j++;
    }

    // Strip the common indent of the opening fence from body lines.
    const code = body
      .map((l) => (indent && l.startsWith(indent) ? l.slice(indent.length) : l))
      .join('\n');

    const fenceDirective = directiveFromMeta(meta);
    const commentDirective = fenceDirective ? null : directiveFromComment(lines, openLine);
    const directive = fenceDirective ?? commentDirective ?? { ...NO_DIRECTIVE };

    snippets.push({
      file: fileLabel,
      line: openLine + 1,
      lang,
      meta,
      code,
      kind: langKind(lang),
      directive,
      id: `${fileLabel}:${openLine + 1}`,
    });

    i = closed ? j + 1 : j;
  }

  return snippets;
}

/** Extracts snippets from every documentation source. */
export function extractAll(): Snippet[] {
  const out: Snippet[] = [];
  for (const abs of docFiles()) {
    const label = relative(REPO_ROOT, abs);
    const text = readFileSync(abs, 'utf8');
    out.push(...extractFromText(text, label));
  }
  return out;
}
