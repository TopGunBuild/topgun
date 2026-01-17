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
})
