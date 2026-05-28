import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/topgun.ts'],
  format: ['cjs'],
  dts: false,
  clean: true,
  sourcemap: false,
  splitting: false,
  treeshake: true,
  minify: false,
  target: 'node18',
  outDir: 'dist',
  shims: true,
  banner: {
    js: '#!/usr/bin/env node',
  },
});
