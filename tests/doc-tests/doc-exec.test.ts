/**
 * Run tier — executes documentation snippets against a real Rust server.
 *
 * This is the heart of the G2 gate: a snippet the docs present as runnable is
 * actually run. The server is the same binary the integration-rust suite uses
 * (prebuilt via RUST_SERVER_BINARY in CI, cargo locally), booted once with the
 * doc profile (no-auth + deterministic embeddings). Per page, `doctest setup`
 * blocks accumulate into a preamble so a "create the client" block can feed a
 * later "write" block; each run block executes as (page-setup ++ block).
 *
 * Verbatim except for one rewrite: the documented `localhost:8080` authority is
 * swapped for the live ephemeral port (the path the doc wrote is preserved, so
 * a wrong documented WS path is still caught).
 */
import { extractAll } from './helpers/extract';
import { classifyAll } from './helpers/classify';
import { assembleRunModule, executeModule } from './helpers/assemble';
import { startDocServer, DocServer } from './helpers/server';
import { __doctestAwaitConnected, __doctestResetClients } from './helpers/client-shim';

const PER_SNIPPET_TIMEOUT_MS = 15_000;
const CONNECT_TIMEOUT_MS = 10_000;

const classified = classifyAll(extractAll());
const runnable = classified.filter((c) => c.verdict === 'run' && c.snippet.kind === 'ts');

/** Per page, the forced-setup blocks that form the shared preamble. */
function setupBlocksFor(file: string): string[] {
  return classified
    .filter(
      (c) =>
        c.snippet.file === file &&
        c.snippet.kind === 'ts' &&
        c.snippet.directive.setup &&
        c.verdict === 'run',
    )
    .map((c) => c.snippet.code);
}

describe('run tier — docs execute against a live Rust server', () => {
  let server: DocServer;

  beforeAll(async () => {
    server = await startDocServer();
  }, 120_000);

  afterAll(async () => {
    if (server) await server.cleanup();
  });

  if (runnable.length === 0) {
    it('has runnable snippets', () => {
      throw new Error('no run-tier snippets found — classification regressed');
    });
  }

  for (const c of runnable) {
    const label = c.snippet.directive.setup ? `${c.snippet.id} (setup)` : c.snippet.id;
    it(
      `runs — ${label}`,
      async () => {
        // A setup block on its own page is executed in-line with each run block,
        // so running it standalone here would double-execute; skip the redundant
        // standalone run when it is purely a preamble for siblings.
        const setupBlocks = setupBlocksFor(c.snippet.file).filter(
          (code) => code !== c.snippet.code,
        );
        const assembled = assembleRunModule(setupBlocks, c.snippet.code, {
          authority: server.authority,
        });
        try {
          await executeModule(assembled, require, PER_SNIPPET_TIMEOUT_MS);
          // If the snippet wired up a client, prove it actually handshook with the
          // live server — this is what makes "runs against a real server" real
          // rather than just "constructs an object". No-ops for local-only / pure
          // snippets that never opened a connection.
          await __doctestAwaitConnected(CONNECT_TIMEOUT_MS);
        } catch (err) {
          throw new Error(
            `run-tier snippet failed: ${c.snippet.id} (${c.snippet.file})\n` +
              `${(err as Error).message}\n\nSnippet:\n${c.snippet.code}`,
          );
        } finally {
          // Tear every client down so reconnect timers don't keep Jest alive and
          // snippets stay isolated.
          await __doctestResetClients();
        }
      },
      PER_SNIPPET_TIMEOUT_MS + 5_000,
    );
  }
});
