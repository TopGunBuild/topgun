#!/usr/bin/env node

/**
 * Generates apps/docs-astro/public/llms-full.txt — a single concatenated markdown
 * file containing the canonical TopGun documentation, suitable for LLM context windows.
 *
 * WHY this file exists:
 *   AI coding agents (Cursor, Claude Code, Codex, GitHub Copilot Chat) probe /llms.txt
 *   on a project's documentation site to discover machine-readable docs. When /llms-full.txt
 *   is present, agents can load the entire canonical doc set in one request rather than
 *   scraping MDX-rendered HTML (which is slow and lossy). This closes the AI-agent
 *   discoverability gap for TopGun's post-cutoff API redesign.
 *
 * ALLOWLIST RULE (for future additions):
 *   Add a page here only if (a) it teaches the canonical API, (b) an agent could not
 *   reconstruct it from other allowlisted pages, and (c) total output stays under 200 KB.
 *
 * CONTENT-TYPE NOTE:
 *   Served by Cloudflare Pages as text/plain; charset=utf-8 — desired behavior.
 *   Do not add HTML routing or asset-pipeline transforms that would force text/markdown.
 *
 * IDEMPOTENCY CONTRACT:
 *   Running this script twice with no source changes produces byte-identical output.
 *   No timestamps in the output, deterministic order via ALLOWLIST (not directory traversal).
 */

import { readFile, writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

// Root of the apps/docs-astro package — resolved relative to this script so the
// script works correctly whether invoked from the workspace root or from apps/docs-astro/.
const DOCS_ROOT = join(__dirname, '..');

const CONTENT_DOCS = join(DOCS_ROOT, 'src', 'content', 'docs');
const OUTPUT_PATH = join(DOCS_ROOT, 'public', 'llms-full.txt');

// Curated allowlist of MDX paths relative to src/content/docs/.
// Ordered for logical reading progression: orientation → installation → concepts → guides → reference → comparison.
// Excluded by design: migration guides (long, opinionated, agents rarely need them),
// tutorial walkthroughs (quick-start covers this ground), advanced ops guides
// (deployment/observability/performance — irrelevant to "build a todo app").
const ALLOWLIST = [
  'intro.mdx',
  'quickstart.mdx',
  'installation.mdx',
  'concepts/index.mdx',
  'concepts/local-first.mdx',
  'concepts/crdt-hlc.mdx',
  'concepts/sync-protocol.mdx',
  'concepts/data-structures.mdx',
  'guides/schema.mdx',
  'guides/building-with-ai.mdx',     // Linked from llms.txt — agents should find it via their RAG
  'guides/authentication.mdx',
  'reference/client.mdx',
  'reference/react.mdx',
  'reference/core.mdx',
  'comparison.mdx',
];

// 200 KB budget — typical limit LLM agents reserve for project context.
const BUDGET_BYTES = 200 * 1024;

const strictMode = process.argv.includes('--strict');

/**
 * Strips YAML frontmatter from the top of an MDX file and extracts
 * the title and description fields. Returns { title, description, body }.
 */
function parseFrontmatter(source) {
  // Frontmatter is delimited by --- on its own line at the start of the file.
  const match = source.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  if (!match) {
    return { title: '', description: '', body: source };
  }

  const frontmatterBlock = match[1];
  const body = match[2];

  // Extract title and description from the YAML block (simple key: value parsing —
  // sufficient for our controlled frontmatter which never uses multi-line YAML values).
  const titleMatch = frontmatterBlock.match(/^title:\s*(.+)$/m);
  const descMatch = frontmatterBlock.match(/^description:\s*(.+)$/m);

  const title = titleMatch ? titleMatch[1].trim() : '';
  const description = descMatch ? descMatch[1].trim() : '';

  return { title, description, body };
}

/**
 * Removes MDX-only constructs that are meaningless in a plain-text markdown context:
 * - Component import lines
 * - export const blocks (single-line and multi-line)
 * - Block-level self-closing JSX tags (<Foo ... /> on their own line)
 * - Block-level opening/closing JSX tags (<Foo ...> / </Foo> on their own line)
 *
 * Inline JSX inside markdown paragraphs is left intact — agents handle stray unknown
 * HTML tags without issue, and removing them risks corrupting surrounding text.
 */
function stripMdxConstructs(body) {
  const lines = body.split('\n');
  const result = [];
  let inExportBlock = false;
  let braceDepth = 0;
  let backtickDepth = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Track multi-line export const blocks (e.g., export const foo = `...` or export const foo = {...})
    if (inExportBlock) {
      // Count backtick pairs for template literal blocks
      for (const ch of line) {
        if (ch === '`') backtickDepth = backtickDepth === 0 ? 1 : 0;
        if (ch === '{') braceDepth++;
        if (ch === '}') braceDepth--;
      }
      // Block ends when both backticks and braces are balanced (depth back to 0)
      if (backtickDepth === 0 && braceDepth === 0) {
        inExportBlock = false;
      }
      continue; // Skip all lines in the export block
    }

    // Single-line import statements: import Foo from 'bar'; or import { Foo } from 'bar'
    if (/^import\s+.*from\s+['"][^'"]+['"]\s*;?\s*$/.test(line)) {
      continue;
    }

    // export const blocks — may be single-line or start a multi-line block
    if (/^export\s+const\s+\w+/.test(line)) {
      // Check if this is a self-contained single line (no open delimiters)
      let localBrace = 0;
      let localBacktick = 0;
      for (const ch of line) {
        if (ch === '`') localBacktick = localBacktick === 0 ? 1 : 0;
        if (ch === '{') localBrace++;
        if (ch === '}') localBrace--;
      }
      if (localBacktick === 0 && localBrace === 0) {
        // Self-contained single line — skip it and continue
        continue;
      } else {
        // Multi-line block — enter tracking mode
        inExportBlock = true;
        braceDepth = localBrace;
        backtickDepth = localBacktick;
        continue;
      }
    }

    // Block-level self-closing JSX tags: <ComponentName ... /> on their own line.
    // Match a line that is purely a JSX self-closing tag (starts with <UpperCase).
    if (/^\s*<[A-Z][A-Za-z0-9.]*(\s[^>]*)?\s*\/>\s*$/.test(line)) {
      continue;
    }

    // Block-level opening JSX tags: <ComponentName ...> on their own line.
    if (/^\s*<[A-Z][A-Za-z0-9.]*(\s[^>]*)?\s*>\s*$/.test(line)) {
      continue;
    }

    // Block-level closing JSX tags: </ComponentName> on their own line.
    if (/^\s*<\/[A-Z][A-Za-z0-9.]*>\s*$/.test(line)) {
      continue;
    }

    // Block-level HTML div/span with JSX className props (breadcrumb nav divs used in many pages)
    if (/^\s*<div\s+className=/.test(line)) {
      continue;
    }
    if (/^\s*<\/div>\s*$/.test(line)) {
      continue;
    }

    result.push(line);
  }

  return result.join('\n');
}

/**
 * Builds the page-boundary heading that separates pages in the concatenated output.
 * Using --- (thematic break) + H1 + blockquote description keeps the output valid markdown
 * and lets agents identify page boundaries unambiguously.
 */
function buildPageBoundary(title, description) {
  const parts = ['\n\n---\n', `\n# ${title}\n`];
  if (description) {
    parts.push(`\n> ${description}\n`);
  }
  parts.push('\n');
  return parts.join('');
}

async function main() {
  const header =
    '# TopGun — Full Documentation\n\n' +
    '> Generated by apps/docs-astro/scripts/build-llms-full.mjs from a curated allowlist\n' +
    '> of canonical pages. For the short index see /llms.txt.\n' +
    '> Source: https://github.com/TopGunBuild/topgun\n\n';

  const pageParts = [];

  for (const relativePath of ALLOWLIST) {
    const filePath = join(CONTENT_DOCS, relativePath);

    let source;
    try {
      source = await readFile(filePath, 'utf8');
    } catch (err) {
      console.error(`[build-llms-full] ERROR: could not read ${relativePath}: ${err.message}`);
      process.exit(1);
    }

    const { title, description, body } = parseFrontmatter(source);
    const cleanBody = stripMdxConstructs(body);
    const boundary = buildPageBoundary(title, description);

    const pageContribution = boundary + cleanBody;
    const pageBytes = Buffer.byteLength(pageContribution, 'utf8');

    console.log(`[build-llms-full] ${relativePath}: ${pageBytes} bytes`);

    pageParts.push(pageContribution);
  }

  const output = header + pageParts.join('');
  const outputBytes = Buffer.byteLength(output, 'utf8');

  try {
    await writeFile(OUTPUT_PATH, output, 'utf8');
  } catch (err) {
    console.error(`[build-llms-full] ERROR: could not write ${OUTPUT_PATH}: ${err.message}`);
    process.exit(1);
  }

  console.log(
    `[build-llms-full] wrote ${outputBytes} bytes from ${ALLOWLIST.length} pages to ${OUTPUT_PATH}`,
  );

  if (outputBytes > BUDGET_BYTES) {
    console.log(
      `[build-llms-full] WARN: llms-full.txt is ${outputBytes} bytes (>200 KB) — review allowlist and trim to stay within the agent context budget`,
    );
    if (strictMode) {
      process.exit(1);
    }
  }
}

main();
