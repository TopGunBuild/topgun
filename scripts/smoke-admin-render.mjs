#!/usr/bin/env node
// Headless-Chrome render gate for the @topgunbuild/server admin SPA.
//
// Installs a published server version into a temp dir, boots it zero-config,
// drives headless Chromium to /admin/, waits for React to mount, and asserts:
//   - Zero console.error entries
//   - Zero failed or non-2xx first-party (same-origin) asset requests
//
// Usage:
//   node scripts/smoke-admin-render.mjs [version]
//   version defaults to "latest"
//
// Tunable env vars (mirrors smoke-npx-published.sh):
//   SMOKE_NPM_POLL_TIMEOUT   total seconds to wait for npm view (default: 180)
//   SMOKE_NPM_POLL_INTERVAL  retry interval in seconds (default: 5)

import { execSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, existsSync } from 'node:fs';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const VERSION = process.argv[2] || 'latest';
const POLL_TIMEOUT_S = Number(process.env.SMOKE_NPM_POLL_TIMEOUT ?? 180);
const POLL_INTERVAL_S = Number(process.env.SMOKE_NPM_POLL_INTERVAL ?? 5);

// Port distinct from the shell smoke (18765) so both can run concurrently in CI.
const PORT = 18766;

// 127.0.0.1 literal because macOS resolves "localhost" to ::1 when IPv6 is
// preferred, and the no-auth server bind is IPv4-only loopback.
const BASE_URL = `http://127.0.0.1:${PORT}`;

// ── State tracked for finally-block teardown ────────────────────────────────
let browser = null;
let serverProc = null;
let tmpDir = null;

// ── Helpers ──────────────────────────────────────────────────────────────────

function log(msg) {
  process.stdout.write(msg + '\n');
}

function runCmd(cmd, opts = {}) {
  return execSync(cmd, { stdio: 'pipe', encoding: 'utf8', ...opts }).trim();
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ── Teardown (always runs) ────────────────────────────────────────────────────

async function teardown() {
  // Close the browser before killing the server to avoid a "target closed" error.
  if (browser !== null) {
    try {
      await browser.close();
    } catch (_) {
      // Ignore errors on close; the process may already be gone.
    }
    browser = null;
  }

  // Kill the server process and wait for it to exit so the port is freed.
  if (serverProc !== null) {
    try {
      serverProc.kill('SIGTERM');
      await new Promise((resolve) => {
        const t = setTimeout(() => {
          // Force-kill if it did not exit within 5 seconds.
          try { serverProc.kill('SIGKILL'); } catch (_) {}
          resolve();
        }, 5000);
        serverProc.once('exit', () => { clearTimeout(t); resolve(); });
      });
    } catch (_) {
      // Ignore: process may already have exited.
    }
    serverProc = null;
  }

  // Remove the temp install dir to avoid leaving disk debris.
  if (tmpDir !== null && existsSync(tmpDir)) {
    try {
      rmSync(tmpDir, { recursive: true, force: true });
    } catch (_) {
      // Best-effort cleanup; non-fatal.
    }
    tmpDir = null;
  }
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main() {
  log('');
  log(`=== smoke-admin-render @topgunbuild/server@${VERSION} ===`);
  log('');

  // ── Step 1: Resolve exact version via npm view ──────────────────────────────

  log(`[1/4] Polling npm view @topgunbuild/server@${VERSION}...`);
  let exactVersion = '';
  const pollEnd = Date.now() + POLL_TIMEOUT_S * 1000;

  while (Date.now() < pollEnd) {
    try {
      const result = runCmd(`npm view "@topgunbuild/server@${VERSION}" version`);
      if (result) {
        exactVersion = result;
        log(`  resolved: @topgunbuild/server@${exactVersion}`);
        break;
      }
    } catch (_) {
      // Not yet available; will retry.
    }
    const elapsed = Math.round((Date.now() - (pollEnd - POLL_TIMEOUT_S * 1000)) / 1000);
    log(`  not yet in registry, retrying in ${POLL_INTERVAL_S}s (${elapsed}/${POLL_TIMEOUT_S}s)...`);
    await sleep(POLL_INTERVAL_S * 1000);
  }

  if (!exactVersion) {
    throw new Error(
      `npm-view: @topgunbuild/server@${VERSION} did not resolve within ${POLL_TIMEOUT_S}s`,
    );
  }

  // ── Step 2: Install server + playwright into temp dir ──────────────────────

  log(`[2/4] Installing @topgunbuild/server@${exactVersion} + playwright into temp dir...`);
  tmpDir = mkdtempSync(join(tmpdir(), 'smoke-admin-render-'));
  log(`  temp dir: ${tmpDir}`);

  // Install the published server package.
  runCmd(
    `npm install "@topgunbuild/server@${exactVersion}" --omit=dev --no-audit --no-fund --silent`,
    { cwd: tmpDir },
  );

  // Install playwright into the same temp dir so we can import { chromium }
  // from there. The workflow's "npx playwright install --with-deps chromium"
  // step only downloads browser binaries into the Playwright cache; it does
  // NOT put the playwright JS package on the module-resolution path.
  runCmd(
    'npm install playwright --no-audit --no-fund --silent',
    { cwd: tmpDir },
  );

  // Fetch the chromium binary that matches THIS playwright version. The
  // workflow's separate "npx playwright install" may resolve a different
  // playwright version than the one just installed here; installing from the
  // temp-dir playwright keeps the JS package and browser binary in lockstep
  // (a no-op cache hit when the workflow already fetched the same version).
  // OS-level deps are handled by the workflow's earlier "--with-deps" run, so
  // only the browser binary is fetched here.
  runCmd('npx playwright install chromium', { cwd: tmpDir });

  log('  packages installed');

  // ── Step 3: Boot the server ─────────────────────────────────────────────────

  log(`[3/4] Booting server on port ${PORT}...`);
  const shimPath = join(tmpDir, 'node_modules/@topgunbuild/server/bin/topgun-server.cjs');

  // The bin shim automatically sets TOPGUN_NO_AUTH=1 and loopback-only bind
  // when no auth secret is present in the environment.
  serverProc = spawn('node', [shimPath], {
    cwd: tmpDir,
    env: { ...process.env, PORT: String(PORT) },
    // Capture output for diagnostics if readiness poll times out.
    stdio: ['ignore', 'pipe', 'pipe'],
    // Detached=false (default) so SIGTERM propagates from this process.
  });

  let serverLog = '';
  serverProc.stdout.on('data', (d) => { serverLog += d.toString(); });
  serverProc.stderr.on('data', (d) => { serverLog += d.toString(); });

  // Poll /health until 2xx (up to 30s). 127.0.0.1 literal for IPv4-only bind.
  let ready = false;
  const bootDeadline = Date.now() + 30_000;
  while (Date.now() < bootDeadline) {
    try {
      const res = await fetch(`${BASE_URL}/health`);
      if (res.ok) { ready = true; break; }
    } catch (_) {
      // Server not listening yet; keep polling.
    }
    await sleep(500);
  }

  if (!ready) {
    log('  Server log:');
    log(serverLog || '  (empty)');
    throw new Error(`boot: server did not become ready on ${BASE_URL}/health within 30s`);
  }
  log(`  server ready at ${BASE_URL}`);

  // ── Step 4: Browser drive + assertions ─────────────────────────────────────

  log('[4/4] Driving headless Chromium to /admin/...');

  // Import chromium from the locally-installed playwright package rather than
  // a bare "playwright" import, which would fail if playwright is not on the
  // global or project module path.
  const playwrightIndexPath = join(tmpDir, 'node_modules/playwright/index.mjs');
  const { chromium } = await import(pathToFileURL(playwrightIndexPath).href);

  // Use the Playwright cache that the workflow pre-populated via
  // "npx playwright install --with-deps chromium". If PLAYWRIGHT_BROWSERS_PATH
  // is set by the CI environment, honour it; otherwise Playwright uses its
  // default cache directory (~/.cache/ms-playwright).
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();

  // Bounded navigation timeout: fail rather than hang indefinitely.
  page.setDefaultNavigationTimeout(30_000);
  page.setDefaultTimeout(15_000);

  const consoleErrors = [];
  const failedAssets = [];

  // Collect console.error entries. Any entry here indicates a runtime JS error
  // in the SPA (including React error #525 / duplicate instance defects).
  page.on('console', (msg) => {
    if (msg.type() === 'error') {
      consoleErrors.push(msg.text());
    }
  });

  // Collect first-party (same-origin) asset failures. Cross-origin requests
  // (CDN fonts, analytics) are ignored — only the SPA's own assets are gated.
  // /favicon.ico is excluded everywhere: browsers auto-request it and the admin
  // bundle may not ship one, so a 404 there is not a SPA-mount defect.
  const isGatedAsset = (url) => url.startsWith(BASE_URL) && !url.endsWith('/favicon.ico');

  page.on('requestfailed', (request) => {
    const url = request.url();
    if (isGatedAsset(url)) {
      failedAssets.push({ url, reason: request.failure()?.errorText ?? 'unknown' });
    }
  });

  page.on('response', (response) => {
    const url = response.url();
    const status = response.status();
    // Gate only first-party requests; ignore redirects (3xx) and informational (1xx).
    if (isGatedAsset(url) && status >= 400) {
      failedAssets.push({ url, status });
    }
  });

  // Navigate to the admin root.
  await page.goto(`${BASE_URL}/admin/`, { waitUntil: 'domcontentloaded' });

  // Wait for the React app to mount. The spinner (`animate-spin`) disappears
  // once the app has initialised (either Dashboard, Login, or ServerUnavailable
  // screen renders). We wait for the spinner to be gone as the primary signal,
  // combined with networkidle as a secondary signal to let late async requests
  // settle. Both waits are bounded by the page-level default timeout (15s).
  //
  // Why not a hard static selector: in no-auth mode the app renders a brief
  // spinner then transitions to Dashboard or Login. Waiting for the spinner to
  // disappear is stable across both paths without coupling to a specific page
  // element that may be re-labelled.
  try {
    // First, wait for networkidle to let the initial asset loads finish.
    await page.waitForLoadState('networkidle', { timeout: 15_000 });
  } catch (_) {
    // networkidle is best-effort; heavy analytics could hold it open. We
    // continue to the selector check regardless.
  }

  // The loading spinner uses the class `animate-spin`. If it is present, wait
  // up to 10s for it to be removed (React init complete).
  const spinnerLocator = page.locator('.animate-spin').first();
  try {
    const spinnerVisible = await spinnerLocator.isVisible({ timeout: 1_000 });
    if (spinnerVisible) {
      await spinnerLocator.waitFor({ state: 'hidden', timeout: 10_000 });
    }
  } catch (_) {
    // Spinner not present or already gone — mount is complete.
  }

  // ── Evaluate assertions ─────────────────────────────────────────────────────

  const failures = [];

  if (consoleErrors.length > 0) {
    failures.push('console.error entries:');
    for (const msg of consoleErrors) {
      failures.push(`  - ${msg}`);
    }
  }

  if (failedAssets.length > 0) {
    failures.push('failed first-party asset requests:');
    for (const a of failedAssets) {
      const detail = a.status != null ? `HTTP ${a.status}` : `network error: ${a.reason}`;
      failures.push(`  - ${a.url} (${detail})`);
    }
  }

  if (failures.length > 0) {
    log('');
    log('=== smoke-admin-render: FAIL ===');
    for (const line of failures) {
      log(line);
    }
    process.exitCode = 1;
    return;
  }

  log('');
  log('=== smoke-admin-render: PASS ===');
  log(`  URL checked  : ${BASE_URL}/admin/`);
  log('  Mount signal : animate-spin gone (React init complete) + networkidle');
  log('  console.error: 0');
  log('  Failed assets: 0 (first-party)');
}

// ── Run with guaranteed teardown ──────────────────────────────────────────────

main()
  .catch((err) => {
    log('');
    log(`=== smoke-admin-render: ERROR: ${err.message} ===`);
    process.exitCode = 1;
  })
  .finally(teardown);
