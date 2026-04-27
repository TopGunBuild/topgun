import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  outDir: 'dist',
  clean: true,
  // Preserve #!/usr/bin/env node shebang for the CLI entry point.
  banner: {
    js: '#!/usr/bin/env node',
  },
});
