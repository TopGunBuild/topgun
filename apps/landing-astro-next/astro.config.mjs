// @ts-check
import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  site: 'https://topgun.build',
  trailingSlash: 'never',
  build: {
    format: 'file',
  },
  integrations: [sitemap()],
  vite: {
    plugins: [tailwindcss()],
  },
});
