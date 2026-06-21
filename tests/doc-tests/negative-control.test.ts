/**
 * Negative control — proves the gate actually fails when documentation breaks.
 *
 * A green doc-test suite is only meaningful if a broken snippet would turn it
 * red. Rather than commit a broken doc (which would block the PR), these tests
 * feed deliberately-broken snippets through the SAME extraction / classification
 * / typecheck / execution pipeline the real suites use, and assert the harness
 * reports a failure. If any of these stops failing, the gate has gone blind.
 *
 * To verify by hand: introduce a typo in any doc snippet (e.g. `client.getMap`
 * → `client.getMapp`) and the typecheck suite reddens on that snippet.
 */
import { checkSnippets } from './helpers/tsc';
import { classifyAll } from './helpers/classify';
import { assembleRunModule, executeModule } from './helpers/assemble';
import { Snippet } from './helpers/extract';
import { __doctestResetClients } from './helpers/client-shim';

function synthetic(code: string, overrides: Partial<Snippet> = {}): Snippet {
  return {
    file: 'negative-control',
    line: 1,
    lang: 'ts',
    meta: '',
    code,
    kind: 'ts',
    directive: {
      skip: false,
      setup: false,
      run: false,
      vector: false,
      expectError: false,
      reason: null,
      source: 'none',
    },
    id: 'negative-control:1',
    ...overrides,
  };
}

describe('negative control — the gate fails on broken docs', () => {
  it('TYPECHECK catches a renamed method on a real type (TS2339)', () => {
    const d = checkSnippets([{ id: 'x', code: `client.getMapTYPO('todos').set('a', {});` }], {
      ambient: true,
    }).get('x')!;
    expect(d.drift.length).toBeGreaterThan(0);
    expect(d.drift.join('\n')).toMatch(/getMapTYPO|does not exist/);
  });

  it('TYPECHECK catches an import of a non-existent export (TS2305)', () => {
    const d = checkSnippets(
      [
        {
          id: 'x',
          code: `import { NotARealExport } from '@topgunbuild/client';\nconsole.log(NotARealExport);`,
        },
      ],
      { ambient: true },
    ).get('x')!;
    expect(d.drift.length).toBeGreaterThan(0);
    expect(d.drift.join('\n')).toMatch(/no exported member|NotARealExport/);
  });

  it('TYPECHECK catches a wrong argument count on a real API (TS2554)', () => {
    const d = checkSnippets(
      [
        {
          id: 'x',
          code: `import { HLC } from '@topgunbuild/core';\nnew HLC('n', {}, 'extra-arg');`,
        },
      ],
      { ambient: true },
    ).get('x')!;
    expect(d.drift.length).toBeGreaterThan(0);
  });

  it('a CORRECT snippet produces no drift (control for the controls)', () => {
    const d = checkSnippets(
      [
        {
          id: 'x',
          code: `import { HLC } from '@topgunbuild/core';\nconst h = new HLC('node-1');\nh.now();`,
        },
      ],
      { ambient: true },
    ).get('x')!;
    expect(d.drift).toHaveLength(0);
  });

  it('RUN catches a snippet that throws at runtime', async () => {
    const assembled = assembleRunModule([], `throw new Error('intentional doc-test failure');`, {
      authority: 'localhost:0',
    });
    await expect(executeModule(assembled, require, 5_000)).rejects.toThrow(
      /intentional doc-test failure/,
    );
    await __doctestResetClients();
  });

  it('MANIFEST refuses an explicit skip with no reason', () => {
    const s = synthetic(`const a = 1;`, {
      lang: 'ts',
      directive: {
        skip: true,
        setup: false,
        run: false,
        vector: false,
        expectError: false,
        reason: null,
        source: 'fence',
      },
    });
    const [classified] = classifyAll([s]);
    expect(classified.verdict).toBe('skip');
    // A reasonless skip surfaces the sentinel the manifest integrity test rejects.
    expect(classified.reason).toBe('(no reason given)');
  });
});
