import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'
import path from 'path'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    react(),
    VitePWA({
      registerType: 'autoUpdate',
      includeAssets: ['icon-192.svg', 'icon-512.svg'],
      manifest: {
        name: 'TopGun Notes',
        short_name: 'Notes',
        description: 'Offline-first real-time notes with TopGun',
        theme_color: '#3B82F6',
        background_color: '#ffffff',
        display: 'standalone',
        start_url: '/',
        icons: [
          {
            src: 'icon-192.svg',
            sizes: '192x192',
            type: 'image/svg+xml',
            purpose: 'any maskable'
          },
          {
            src: 'icon-512.svg',
            sizes: '512x512',
            type: 'image/svg+xml',
            purpose: 'any maskable'
          }
        ]
      },
      workbox: {
        globPatterns: ['**/*.{js,css,html,ico,png,svg,woff2}'],
        // Import custom push notification handlers
        importScripts: ['sw-push.js'],
        runtimeCaching: [
          {
            urlPattern: /^https:\/\/.*\.clerk\.accounts\.dev\/.*/i,
            handler: 'NetworkFirst',
            options: {
              cacheName: 'clerk-cache',
              expiration: {
                maxEntries: 10,
                maxAgeSeconds: 60 * 60 * 24 // 24 hours
              }
            }
          }
        ]
      }
    })
  ],
  resolve: {
    alias: {
      '@topgunbuild/client': path.resolve(__dirname, '../../packages/client/src/index.ts'),
      '@topgunbuild/core': path.resolve(__dirname, '../../packages/core/src/index.ts'),
      '@topgunbuild/react': path.resolve(__dirname, '../../packages/react/src/index.ts'),
      '@topgunbuild/adapters': path.resolve(__dirname, '../../packages/adapters/src/index.ts')
    }
  },
  optimizeDeps: {
    exclude: ['@topgunbuild/client', '@topgunbuild/core', '@topgunbuild/react', '@topgunbuild/adapters']
  }
})
