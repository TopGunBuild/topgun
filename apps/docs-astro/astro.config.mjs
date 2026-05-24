// @ts-check
import { defineConfig } from 'astro/config';

import react from '@astrojs/react';
import tailwindcss from '@tailwindcss/vite';
import mdx from '@astrojs/mdx';
import sitemap from '@astrojs/sitemap';
import rehypeExternalLinks from 'rehype-external-links';

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
    // '/docs/guides/mcp-server' redirect removed 2026-05-24 — the page
    // now exists as a how-to (setup for Claude Desktop / Cursor + 8 tools
    // overview). /docs/reference/mcp stays as the API reference companion.
    '/docs/guides/deployment': '/docs/deploy/self-host',
    '/docs/guides/postgresql': '/docs/deploy/storage-backends',
    '/docs/guides/performance': '/docs/deploy/performance',
    '/docs/guides/pub-sub': '/docs/guides/live-notifications',
    '/docs/guides/live-queries': '/docs/guides/search-and-live-queries',
    '/docs/guides/full-text-search': '/docs/guides/search-and-live-queries',
    '/docs/guides/pn-counter': '/docs/guides/counters-and-locks',
    '/docs/guides/distributed-locks': '/docs/guides/counters-and-locks',
    '/docs/guides/schema': '/docs/guides/schema-typed-data',
    // Removed 2026-05-23 per DOCS_IA_PLAN_2026_05_20.md "What gets deleted" — content
    // status (stable/experimental/planned/not-building) consolidated on /docs/roadmap.
    '/docs/guides/adaptive-indexing': '/docs/roadmap',
    '/docs/guides/indexing': '/docs/roadmap',
    '/docs/guides/interceptors': '/docs/roadmap',
    '/docs/guides/entry-processor': '/docs/roadmap',
    '/docs/guides/conflict-resolvers': '/docs/roadmap',
    '/docs/guides/distributed-queries': '/docs/roadmap',
    '/docs/guides/sql-queries': '/docs/roadmap',
    '/docs/guides/sync-state': '/docs/reference/client',
    '/docs/guides/observability': '/docs/deploy/performance',
    '/docs/guides/event-journal': '/docs/reference/client',
    '/docs/guides/rbac': '/docs/roadmap',
    // '/docs/guides/security' redirect removed 2026-05-24 — the page now
    // exists as a how-to for the security primitives that ship in v2.0
    // (TLS termination, EncryptedStorageAdapter, JWT_SECRET hygiene).
    // RBAC, cluster mTLS, field-level redaction remain Planned and are
    // tracked on /docs/roadmap which the guide cross-links to.
    '/docs/guides/adoption-path': '/docs/quickstart',
    '/docs/guides/cluster-replication': '/docs/roadmap',
    '/docs/guides/cluster-client': '/docs/roadmap',
    '/docs/guides/ttl': '/docs/reference/client',
    '/docs/guides/write-concern': '/docs/roadmap',
  },
  build: {
    format: 'file',
  },
  // rehype-external-links auto-adds target="_blank" + rel="noopener noreferrer"
  // to every external <a> in MDX/Markdown content. This covers demo.topgun.build,
  // npm, GitHub, Cloudflare, and anything else that escapes the site origin.
  // Astro components (.astro / .tsx) keep their explicit attrs.
  markdown: {
    rehypePlugins: [
      [
        rehypeExternalLinks,
        {
          target: '_blank',
          rel: ['noopener', 'noreferrer'],
        },
      ],
    ],
  },
  integrations: [
    react(),
    mdx({
      rehypePlugins: [
        [
          rehypeExternalLinks,
          {
            target: '_blank',
            rel: ['noopener', 'noreferrer'],
          },
        ],
      ],
    }),
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