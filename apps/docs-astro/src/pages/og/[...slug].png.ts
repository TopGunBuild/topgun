import type { APIContext } from 'astro';
import { getCollection } from 'astro:content';
import { generateOgImage } from '../../lib/og-image';

// Pre-generate all OG images at build time
export async function getStaticPaths() {
  const docs = await getCollection('docs');
  const blog = await getCollection('blog');

  // Homepage default OG image lives at src/pages/og.png.ts (a dedicated route).
  // Emitting it here as `slug: undefined` produced the dotfile `/og/.png`, which
  // is unreachable as `/og.png` and is served with the wrong Content-Type.
  const paths = [
    // Standalone pages (not in content collections)
    {
      params: { slug: 'blog' },
      props: {
        title: 'TopGun Blog',
        description: 'Engineering posts on offline-first architecture, CRDTs, and the Rust server.',
        type: 'blog' as const,
      },
    },
    {
      params: { slug: 'whitepaper' },
      props: {
        title: 'TopGun v2 Whitepaper',
        description:
          'Architecture, design decisions, and benchmarks behind the offline-first data grid.',
        type: 'docs' as const,
      },
    },
    {
      params: { slug: '404' },
      props: {
        title: 'Page not found',
        description: 'The page you are looking for has moved or never existed.',
        type: 'default' as const,
      },
    },
    // Docs pages
    ...docs.map((entry) => ({
      params: { slug: `docs/${entry.slug}` },
      props: {
        title: entry.data.title,
        description: entry.data.description,
        type: 'docs' as const,
      },
    })),
    // Blog pages
    ...blog.map((entry) => ({
      params: { slug: `blog/${entry.slug}` },
      props: {
        title: entry.data.title,
        description: entry.data.excerpt,
        type: 'blog' as const,
      },
    })),
  ];

  return paths;
}

export async function GET({ props }: APIContext) {
  const { title, description, type } = props as {
    title: string;
    description?: string;
    type: 'default' | 'blog' | 'docs';
  };

  const png = await generateOgImage({ title, description, type });

  return new Response(png, {
    headers: {
      'Content-Type': 'image/png',
      'Cache-Control': 'public, max-age=31536000, immutable',
    },
  });
}
