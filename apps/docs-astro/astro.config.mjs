// @ts-check
import { defineConfig } from 'astro/config';

import react from '@astrojs/react';
import tailwindcss from '@tailwindcss/vite';
import mdx from '@astrojs/mdx';
import sitemap from '@astrojs/sitemap';

// https://astro.build/config
export default defineConfig({
  site: 'https://topgun.build',
  trailingSlash: 'never',
  redirects: {
    '/docs/quick-start': '/docs/quickstart',
    '/docs/reference/data-structures': '/docs/reference/core',
    '/docs/reference/react-hooks': '/docs/reference/react',
    '/docs/reference/adapter': '/docs/reference/adapters',
    '/docs/reference/cli': '/docs/reference/server',
    '/docs/guides/mcp-server': '/docs/reference/mcp',
    '/docs/guides/deployment': '/docs/deploy/self-host',
    '/docs/guides/postgresql': '/docs/deploy/storage-backends',
    '/docs/guides/performance': '/docs/deploy/performance',
    '/docs/guides/pub-sub': '/docs/guides/live-notifications',
    '/docs/guides/live-queries': '/docs/guides/search-and-live-queries',
    '/docs/guides/full-text-search': '/docs/guides/search-and-live-queries',
    '/docs/guides/pn-counter': '/docs/guides/counters-and-locks',
    '/docs/guides/distributed-locks': '/docs/guides/counters-and-locks',
    '/docs/guides/schema': '/docs/guides/schema-typed-data',
  },
  build: {
    format: 'file',
  },
  integrations: [
    react(),
    mdx(),
    sitemap({
      filter: (page) => {
        // Exclude redirect pages from sitemap
        const excludedPages = [
          'https://topgun.build/docs',
        ];
        return !excludedPages.includes(page.replace(/\/$/, ''));
      },
    }),
  ],

  vite: {
    plugins: [tailwindcss()]
  }
});