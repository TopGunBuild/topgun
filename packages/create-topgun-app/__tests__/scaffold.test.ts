import { join, resolve } from 'node:path';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import fsExtra from 'fs-extra';
const { pathExists, readJson } = fsExtra;
import { scaffold } from '../src/scaffold.js';

// Resolve the template directory relative to the package root.
// __dirname in CJS Jest context = packages/create-topgun-app/__tests__
// → walk up one level to package root, then into template/
const TEMPLATE_DIR = resolve(__dirname, '..', 'template');

let tmpDir: string;

beforeEach(async () => {
  tmpDir = await mkdtemp(join(tmpdir(), 'create-topgun-app-test-'));
});

afterEach(async () => {
  if (tmpDir) {
    await rm(tmpDir, { recursive: true, force: true });
  }
});

describe('scaffold', () => {
  test('Test 1: scaffolds to tmp dir and rewrites package.json name', async () => {
    const appName = 'my-test-app';
    const targetDir = join(tmpDir, appName);

    await scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR });

    const pkgPath = join(targetDir, 'package.json');
    expect(await pathExists(pkgPath)).toBe(true);

    const pkg = await readJson(pkgPath);
    expect(pkg.name).toBe('my-test-app');
  });

  test('Test 2: scaffolds .gitignore (renamed from .gitignore.template)', async () => {
    const appName = 'my-test-app-2';
    const targetDir = join(tmpDir, appName);

    await scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR });

    expect(await pathExists(join(targetDir, '.gitignore'))).toBe(true);
    // The .gitignore.template source file should no longer exist after rename.
    expect(await pathExists(join(targetDir, '.gitignore.template'))).toBe(false);
  });

  test('Test 3: throws when target directory is non-empty', async () => {
    const appName = 'existing-app';
    const targetDir = join(tmpDir, appName);

    // Scaffold once to populate the directory.
    await scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR });

    // Scaffolding into the same (now non-empty) directory must throw.
    await expect(scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR })).rejects.toThrow(
      /already exists and is not empty/
    );
  });

  test('Test 4: scaffolded package.json has only expected runtime dependencies', async () => {
    const appName = 'dep-check-app';
    const targetDir = join(tmpDir, appName);

    await scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR });

    const pkg = await readJson(join(targetDir, 'package.json'));
    const runtimeDeps = Object.keys(pkg.dependencies ?? {});

    const allowedRuntimeDeps = new Set([
      '@topgunbuild/client',
      '@topgunbuild/adapters',
      '@topgunbuild/core',
      '@topgunbuild/react',
      'react',
      'react-dom',
    ]);

    for (const dep of runtimeDeps) {
      expect(allowedRuntimeDeps.has(dep)).toBe(true);
    }

    // Ensure vite and plugin-react are NOT in runtime dependencies (they belong in devDependencies).
    expect(runtimeDeps).not.toContain('vite');
    expect(runtimeDeps).not.toContain('@vitejs/plugin-react');

    // Verify no literal "topgun-app" name remains in the scaffolded package.json
    // (the template name "topgun-app" must have been replaced with the slugified appName).
    expect(pkg.name).not.toBe('topgun-app');
    expect(pkg.name).toBe('dep-check-app');
  });

  test('Test 5: README {{appName}} placeholder is substituted with slugified name', async () => {
    const appName = 'My Cool App';
    const targetDir = join(tmpDir, 'my-cool-app');

    await scaffold({ appName, targetDir, templateDir: TEMPLATE_DIR });

    const readme = await readFile(join(targetDir, 'README.md'), 'utf-8');

    // Slugified name should be substituted into the cd instruction.
    expect(readme).toContain('cd my-cool-app');
    // Raw token should not survive scaffolding.
    expect(readme).not.toContain('{{appName}}');
  });
});
