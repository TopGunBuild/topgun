import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts', 'src/start-server.ts'],
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  external: ['@topgunbuild/native', 'isolated-vm'],
});
