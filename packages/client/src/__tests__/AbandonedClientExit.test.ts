import { spawn } from 'child_process';
import * as path from 'path';

/**
 * The background timers of a client that is built and then abandoned — never
 * closed — must not keep a Node process alive. Jest cannot see this from inside
 * a test (the worker stays up for other reasons and tears timers down itself),
 * so each case runs the client in a child process and checks that the child
 * exits by itself.
 *
 * What may keep the process alive is an open transport, and only that: a real
 * socket (not exercised here, the socket is mocked) or, on HTTP, the polling
 * interval that stands in for one. That case is pinned the other way round.
 */

const PACKAGE_ROOT = path.resolve(__dirname, '..', '..');
const FIXTURE = path.join(__dirname, 'fixtures', 'abandonedClient.child.ts');
const CORE_ENTRY = path.resolve(PACKAGE_ROOT, '..', 'core', 'src', 'index.ts');

// Runs the fixture from source, with @topgunbuild/core taken from source too, so
// the result never depends on a dist/ that may be stale or absent.
const BOOTSTRAP = `
require('ts-node').register({
  transpileOnly: true,
  skipProject: true,
  compilerOptions: { module: 'commonjs', target: 'es2020', esModuleInterop: true },
});
const Module = require('module');
const resolveFilename = Module._resolveFilename;
Module._resolveFilename = function (request, ...rest) {
  return resolveFilename.call(
    this,
    request === '@topgunbuild/core' ? ${JSON.stringify(CORE_ENTRY)} : request,
    ...rest,
  );
};
require(${JSON.stringify(FIXTURE)});
`;

/** How long after RELEASED the process may still be running. */
const EXIT_WITHIN_MS = 3_000;
/** Upper bound on reaching RELEASED, most of which is transpiling the sources. */
const RELEASE_WITHIN_MS = 60_000;

interface ChildOutcome {
  released: boolean;
  /** null when the process had to be killed. */
  exitCode: number | null;
  msFromReleaseToExit: number | null;
  stderr: string;
}

function runScenario(scenario: string, exitWithinMs: number): Promise<ChildOutcome> {
  return new Promise((resolve) => {
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      NODE_ENV: 'test',
      LOG_LEVEL: 'info',
      ABANDONED_CLIENT_SCENARIO: scenario,
    };
    delete env.JEST_WORKER_ID;

    const child = spawn(process.execPath, ['-e', BOOTSTRAP], {
      cwd: PACKAGE_ROOT,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    let stdout = '';
    let stderr = '';
    let releasedAt: number | null = null;
    let killed = false;
    let exitDeadline: ReturnType<typeof setTimeout> | null = null;

    const kill = () => {
      killed = true;
      child.kill('SIGKILL');
    };
    const releaseDeadline = setTimeout(kill, RELEASE_WITHIN_MS);

    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString();
      if (releasedAt === null && stdout.includes('RELEASED')) {
        releasedAt = Date.now();
        clearTimeout(releaseDeadline);
        exitDeadline = setTimeout(kill, exitWithinMs);
      }
    });
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString();
    });

    child.on('close', (code) => {
      clearTimeout(releaseDeadline);
      if (exitDeadline) clearTimeout(exitDeadline);
      resolve({
        released: releasedAt !== null,
        exitCode: killed ? null : code,
        msFromReleaseToExit: releasedAt === null || killed ? null : Date.now() - releasedAt,
        stderr: stderr.split('\n').slice(-25).join('\n'),
      });
    });
  });
}

/** Fails with the child's last log lines, which name the timer that kept firing. */
function expectOutcome(
  outcome: ChildOutcome,
  expected: { released: boolean; exitCode: number | null },
): void {
  const actual = { released: outcome.released, exitCode: outcome.exitCode };
  if (actual.released !== expected.released || actual.exitCode !== expected.exitCode) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)} ` +
        `(exitCode null = had to be killed)\n--- child stderr (tail) ---\n${outcome.stderr}`,
    );
  }
}

describe("a client's background timers do not keep a Node process alive", () => {
  jest.setTimeout(RELEASE_WITHIN_MS + 30_000);

  test.each([
    ['heartbeat', 'connected and idle, heartbeat armed'],
    ['device-credential-grace', 'waiting for the device credential'],
    ['closed-before-open', 'closed before the socket reported open'],
    ['http-closed', 'HTTP transport, closed after polling was armed'],
  ])('%s: %s', async (scenario) => {
    const outcome = await runScenario(scenario, EXIT_WITHIN_MS);

    expectOutcome(outcome, { released: true, exitCode: 0 });
  });

  // The cluster client's connect() waits up to 10 s for a partition map before
  // it falls back; that wait is a one-shot, caller-awaited timeout and is
  // deliberately ref'd. So the bound here is that wait, not EXIT_WITHIN_MS.
  test('cluster: seeds connected, no partition map', async () => {
    const outcome = await runScenario('cluster', 10_000 + EXIT_WITHIN_MS);

    expectOutcome(outcome, { released: true, exitCode: 0 });
  });

  // The one background timer that is ref'd on purpose. An open WebSocket client
  // holds its process through the socket; on HTTP there is no handle between
  // polls, so the polling interval has to do it. The process is killed here.
  test('http-open: an open HTTP client does keep the process alive', async () => {
    const outcome = await runScenario('http-open', EXIT_WITHIN_MS);

    expectOutcome(outcome, { released: true, exitCode: null });
  });

  test("control: a ref'd handle does keep the process alive, so the cases above can fail", async () => {
    const outcome = await runScenario('control-held', EXIT_WITHIN_MS);

    expectOutcome(outcome, { released: true, exitCode: null });
  });
});
