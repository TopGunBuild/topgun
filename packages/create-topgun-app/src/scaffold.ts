// fs-extra is a CommonJS module — use default import and destructure to avoid
// named-export resolution failures in ESM contexts (Node.js v22+).
import fsExtra from 'fs-extra';
const { copy, readJson, writeJson, pathExists, readdir } = fsExtra;
import { join } from 'node:path';
import { readFile, rename, writeFile } from 'node:fs/promises';

/**
 * Slugify an app name to a valid npm package name.
 * Lowercases and replaces spaces/underscores/dots with hyphens.
 */
function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[\s_.]+/g, '-')
    .replace(/[^a-z0-9-]/g, '')
    .replace(/^-+|-+$/g, '');
}

export interface ScaffoldOptions {
  appName: string;
  targetDir: string;
  /** Path to the template directory. Defaults to ../template relative to this file at runtime. */
  templateDir?: string;
}

/**
 * Scaffold a new TopGun app into targetDir.
 *
 * Steps:
 *  1. Copy all template files recursively.
 *  2. Rewrite package.json with the slugified appName and strip private field.
 *  3. Rename .gitignore.template → .gitignore (npm strips bare .gitignore on publish).
 */
export async function scaffold({ appName, targetDir, templateDir }: ScaffoldOptions): Promise<void> {
  // Guard: target directory must not be non-empty.
  if (await pathExists(targetDir)) {
    const entries = await readdir(targetDir);
    if (entries.length > 0) {
      throw new Error(
        `Target directory "${targetDir}" already exists and is not empty. Choose a different name or remove the directory first.`
      );
    }
  }

  // 1. Copy template → targetDir (fs-extra handles recursive copy + dir creation).
  await copy(templateDir!, targetDir, { overwrite: false, errorOnExist: false });

  const slug = slugify(appName);

  // 2. Rewrite package.json name and strip private field.
  const pkgPath = join(targetDir, 'package.json');
  const pkg = await readJson(pkgPath);
  pkg.name = slug;
  delete pkg.private;
  await writeJson(pkgPath, pkg, { spaces: 2 });

  // 3. Substitute {{appName}} mustache token in README.md so onboarding `cd`
  //    instructions match the actual scaffolded directory name.
  const readmePath = join(targetDir, 'README.md');
  if (await pathExists(readmePath)) {
    const readme = await readFile(readmePath, 'utf-8');
    await writeFile(readmePath, readme.replace(/\{\{appName\}\}/g, slug), 'utf-8');
  }

  // 4. Rename .gitignore.template → .gitignore.
  //    npm strips bare .gitignore files when publishing — .template suffix is the workaround.
  const gitignoreTemplatePath = join(targetDir, '.gitignore.template');
  const gitignorePath = join(targetDir, '.gitignore');
  if (await pathExists(gitignoreTemplatePath)) {
    await rename(gitignoreTemplatePath, gitignorePath);
  }
}
