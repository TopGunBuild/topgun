import type { APIContext } from 'astro';
import { getCollection } from 'astro:content';
import { generateOgImage } from '../../lib/og-image';

// Pre-generate all OG images at build time
export async function getStaticPaths() {
  const docs = await getCollection('docs');
  const blog = await getCollection('blog');

  const paths = [
    // Default homepage
    { params: { slug: undefined }, props: { title: 'TopGun', description: 'The Hybrid Offline-First In-Memory Data Grid', type: 'default' as const } },
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
