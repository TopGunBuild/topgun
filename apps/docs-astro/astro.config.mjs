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