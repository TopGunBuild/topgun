import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@topgunbuild/core': path.resolve(__dirname, '../../packages/core/dist/index.mjs'),
      '@topgunbuild/client': path.resolve(__dirname, '../../packages/client/dist/index.mjs'),
      '@topgunbuild/react': path.resolve(__dirname, '../../packages/react/dist/index.mjs'),
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    proxy: {
      '/api': {
        // Proxy target is configurable so `topgun dev --admin --port <p>` can
        // point the admin's HTTP API (auth-status probe, login) at the same
        // server the WebSocket client connects to. Defaults to the standard port.
        target: process.env.VITE_PROXY_TARGET || 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
  base: '/admin/',
})
