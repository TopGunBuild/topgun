import pc from 'picocolors';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { scaffold } from './scaffold.js';

const appName = process.argv[2];

if (!appName) {
  console.error(pc.red('Usage: npx create-topgun-app <project-name>'));
  process.exit(1);
}

// Resolve the template directory relative to this CLI entry file.
// dist/index.js → package root → template/
const packageRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const templateDir = join(packageRoot, 'template');
const targetDir = join(process.cwd(), appName);

async function main() {
  try {
    await scaffold({ appName, targetDir, templateDir });
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    console.error(pc.red(`Error: ${message}`));
    process.exit(1);
  }

  console.log();
  console.log(pc.green(`Done! Next steps:`));
  console.log();
  console.log(`  1. ${pc.cyan(`cd ${appName}`)}`);
  console.log(`  2. ${pc.cyan('pnpm install')}   ${pc.dim('(or npm install)')}`);
  console.log(`  3. ${pc.cyan('pnpm dev')}`);
  console.log(`  4. Open ${pc.cyan('http://localhost:5173')}`);
  console.log(`  5. Edit ${pc.cyan('src/App.tsx')} and watch live reload`);
  console.log();
}

main();
