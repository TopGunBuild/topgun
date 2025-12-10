import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@topgunbuild/client': path.resolve(__dirname, '../../packages/client/src/index.ts'),
      '@topgunbuild/core': path.resolve(__dirname, '../../packages/core/src/index.ts')
    }
  },
  optimizeDeps: {
    exclude: ['@topgunbuild/client', '@topgunbuild/core']
  }
})
