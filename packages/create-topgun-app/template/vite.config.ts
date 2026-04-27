import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Standard Vite config for a scaffolded TopGun app.
// @topgunbuild/* packages resolve via node_modules after `pnpm install`.
// No workspace-symlink resolve.alias needed — this app runs outside the monorepo.
export default defineConfig({
  plugins: [react()],
});
