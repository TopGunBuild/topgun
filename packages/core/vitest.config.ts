import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    exclude: ['node_modules', 'dist'],
    benchmark: {
      include: ['src/__benchmarks__/*.bench.ts'],
    },
  },
});
