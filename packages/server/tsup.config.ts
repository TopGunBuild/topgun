import { defineConfig } from 'tsup';

export default defineConfig([
  // Main entry points
  {
    entry: ['src/index.ts', 'src/start-server.ts'],
    format: ['cjs', 'esm'],
    dts: true,
    clean: true,
    sourcemap: true,
    external: ['@topgunbuild/native', 'isolated-vm'],
  },
  // Worker scripts - CJS only, preserve directory structure
  // Each worker file is self-contained with all handlers bundled
  {
    entry: ['src/workers/worker-scripts/*.worker.ts'],
    format: ['cjs'],
    outDir: 'dist/workers/worker-scripts',
    clean: false, // Don't clean, main build already did
    sourcemap: true,
    external: ['@topgunbuild/native', 'isolated-vm'],
  },
]);
