/**
 * Bash tier.
 *
 * TopGun's documented shell commands are almost entirely side-effecting infra
 * (install, scaffold, `cargo run`, `docker pull/run`, env exports, curl against
 * placeholder hosts). A doc-test sandbox must not execute those — they are
 * validated by the CI jobs named in each snippet's skip reason (node,
 * create-topgun-app, post-publish-smoke, docker, rust). So the classifier marks
 * them EXPLICIT skips with a categorized reason (visible in the manifest), and
 * only runs a snippet whose every command is hermetic-safe.
 *
 * This suite (a) executes any run-tier bash in a throwaway temp dir, and
 * (b) asserts every bash snippet is classified — run, or skip WITH a reason.
 * New bash snippets are picked up automatically; an unclassifiable one fails.
 */
import { execSync } from 'child_process';
import { mkdtempSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

import { extractAll, BASH_LANGS } from './helpers/extract';
import { classifyAll } from './helpers/classify';

const classified = classifyAll(extractAll());
const bash = classified.filter((c) => BASH_LANGS.has(c.snippet.lang));
const runBash = bash.filter((c) => c.verdict === 'run');

describe('bash tier', () => {
  it('classifies every bash snippet (run, or skip with a reason)', () => {
    const bad = bash.filter((c) => c.verdict !== 'run' && (!c.reason || c.reason.trim() === ''));
    if (bad.length) {
      throw new Error(
        `bash snippets skipped without a reason:\n${bad.map((c) => '  ' + c.snippet.id).join('\n')}`,
      );
    }
    expect(bash.length).toBeGreaterThan(0);
  });

  if (runBash.length === 0) {
    it('has no hermetic-safe bash to execute (all are infra — skipped with reasons)', () => {
      // Documented expectation, not a silent gap: the manifest lists each skip.
      expect(runBash).toHaveLength(0);
    });
  }

  for (const c of runBash) {
    it(`runs — ${c.snippet.id}`, () => {
      const dir = mkdtempSync(join(tmpdir(), 'topgun-doc-bash-'));
      try {
        execSync(c.snippet.code, { cwd: dir, timeout: 15_000, stdio: 'pipe' });
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    });
  }
});
