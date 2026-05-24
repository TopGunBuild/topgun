import type { APIContext } from 'astro';
import { generateOgImage } from '../lib/og-image';

// Dedicated homepage OG image route. The catch-all `og/[...slug].png.ts`
// previously emitted this as `/og/.png` (a dotfile that Cloudflare serves as
// application/octet-stream — social scrapers won't render it).
export async function GET(_ctx: APIContext) {
  const png = await generateOgImage({
    title: 'TopGun',
    description: 'Real-time apps that survive offline.',
    type: 'default',
  });

  return new Response(png, {
    headers: {
      'Content-Type': 'image/png',
      'Cache-Control': 'public, max-age=31536000, immutable',
    },
  });
}
