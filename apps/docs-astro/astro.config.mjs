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