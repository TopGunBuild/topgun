import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@topgunbuild/core': path.resolve(__dirname, '../../packages/core/src/index.ts'),
      '@topgunbuild/client': path.resolve(__dirname, '../../packages/client/src/index.ts'),
      '@topgunbuild/react': path.resolve(__dirname, '../../packages/react/src/index.ts'),
      '@': path.resolve(__dirname, './src'),
    },
  },
  optimizeDeps: {
    exclude: ['@topgunbuild/client', '@topgunbuild/core', '@topgunbuild/react'],
  },
})
