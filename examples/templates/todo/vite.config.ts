import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    // Allow Vite to serve files from the shared directory outside the project root
    fs: {
      allow: ['..'],
    },
  },
  resolve: {
    // Explicitly resolve @topgunbuild/* to the app-local node_modules symlinks.
    // This is NOT the same as sync-lab's alias (which points at pre-built dist/
    // artifacts and requires pnpm build first). These aliases let Vite's bundler
    // find the workspace packages when processing shared files outside the project
    // root, without requiring a prior build step.
    alias: {
      '@topgunbuild/client': path.resolve(__dirname, 'node_modules/@topgunbuild/client'),
      '@topgunbuild/adapters': path.resolve(__dirname, 'node_modules/@topgunbuild/adapters'),
      '@topgunbuild/core': path.resolve(__dirname, 'node_modules/@topgunbuild/core'),
      '@topgunbuild/react': path.resolve(__dirname, 'node_modules/@topgunbuild/react'),
    },
  },
});
