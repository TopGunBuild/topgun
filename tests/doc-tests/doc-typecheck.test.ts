/**
 * Typecheck tier + manifest integrity.
 *
 * - Every documentation snippet is classified into exactly one tier
 *   (run / typecheck / skip). There is no fourth "silently untested" state.
 * - Every `skip` carries a non-empty reason, so the manifest can be audited:
 *   nothing leaves the gate unexercised without an explicit, visible reason.
 * - Every `typecheck`/`run` snippet compiles against the REAL published package
 *   types with zero API-drift diagnostics. A renamed/removed export or method,
 *   a wrong argument — the docs-overpromise class the G2 gate exists to catch —
 *   fails here.
 *
 * This suite needs no server; it is pure static analysis and runs fast.
 */
import { extractAll } from './helpers/extract';
import { classifyAll, Classified } from './helpers/classify';

const snippets = extractAll();
const classified: Classified[] = classifyAll(snippets);

const byVerdict = (v: string) => classified.filter((c) => c.verdict === v);

describe('doc-test manifest integrity', () => {
  it('classifies every snippet into exactly one tier', () => {
    for (const c of classified) {
      expect(['run', 'typecheck', 'skip']).toContain(c.verdict);
    }
    // Sanity: the corpus is non-trivial, so a regression that silently drops
    // extraction (0 snippets) cannot masquerade as "all green".
    expect(classified.length).toBeGreaterThan(100);
  });

  it('never skips silently — every skip has a non-empty reason', () => {
    const offenders = byVerdict('skip').filter(
      (c) => !c.reason || c.reason.trim() === '' || c.reason === '(no reason given)',
    );
    if (offenders.length) {
      const list = offenders.map((c) => `  ${c.snippet.id} (${c.snippet.lang})`).join('\n');
      throw new Error(`Snippets skipped without a reason:\n${list}`);
    }
    expect(offenders).toHaveLength(0);
  });

  it('prints the doc-test manifest summary', () => {
    const run = byVerdict('run').length;
    const tc = byVerdict('typecheck').length;
    const skip = byVerdict('skip').length;
    // eslint-disable-next-line no-console
    console.log(
      `\nDoc-test manifest: ${classified.length} snippets — ` +
        `${run} run · ${tc} typecheck · ${skip} skip\n`,
    );
    expect(run + tc + skip).toBe(classified.length);
  });
});

describe('typecheck tier — docs compile against real package types', () => {
  const checkable = classified.filter(
    (c) => (c.verdict === 'typecheck' || c.verdict === 'run') && c.diagnostics,
  );

  if (checkable.length === 0) {
    it('has checkable snippets', () => {
      throw new Error('no typecheck-tier snippets found — extraction or classification broke');
    });
  }

  for (const c of checkable) {
    it(`no API drift — ${c.snippet.id}`, () => {
      const drift = c.diagnostics!.drift;
      if (drift.length) {
        throw new Error(
          `API drift in ${c.snippet.id} (${c.snippet.file}):\n` +
            drift.map((d) => `  ${d}`).join('\n') +
            `\n\nSnippet:\n${c.snippet.code}`,
        );
      }
      expect(drift).toHaveLength(0);
    });
  }
});
