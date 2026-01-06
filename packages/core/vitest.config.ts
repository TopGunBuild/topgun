import { defineConfig } from 'vitest/config';

const isCI = process.env.CI === 'true';

export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    exclude: ['node_modules', 'dist'],
    benchmark: {
      include: ['src/__benchmarks__/**/*.bench.ts'],
      // Reduce iterations in CI to speed up builds
      ...(isCI && {
        warmupIterations: 3,
        iterations: 50,
        time: 500,
      }),
    },
  },
});
