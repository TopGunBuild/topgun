// One-off generator: emits favicon raster set from public/favicon.svg.
// Re-run only when the TG mark changes. Output files are committed to public/.
//
//   pnpm --filter apps-docs-astro exec node scripts/generate-favicons.mjs

import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { readFile, writeFile } from 'node:fs/promises';
import sharp from 'sharp';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PUBLIC = join(__dirname, '..', 'public');

// The committed favicon.svg uses prefers-color-scheme to switch between black
// and white fill. For raster PWA icons we want a single deterministic colour
// scheme: white TG on the brand dark background. Build a one-off SVG that
// hardcodes that — matches what iOS / Android / Chrome will show on the
// browser-tab and home-screen.
const svgSrc = await readFile(join(PUBLIC, 'favicon.svg'), 'utf8');
// Pull everything inside <svg>...</svg>, then drop the <style> block. The
// remainder is the TG glyph as nested <g><path/></g> elements, which we can
// re-wrap with our own <g fill="..."> for color control.
const inner = svgSrc.match(/<svg[^>]*>([\s\S]*?)<\/svg>/)?.[1] ?? '';
const pathData = inner.replace(/<style[\s\S]*?<\/style>/g, '').trim();
if (!pathData.includes('<path')) {
  throw new Error('Could not extract TG path block from favicon.svg');
}

const BG = '#0a0e14';
const FG = '#FFFFFF';

const masterSvg = (size) => `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 128 128">
  <rect width="128" height="128" fill="${BG}" />
  <g fill="${FG}">${pathData}</g>
</svg>`;

const png = (svgString, size) => sharp(Buffer.from(svgString), { density: 384 }).resize(size, size).png().toBuffer();

const out = (name, buf) => writeFile(join(PUBLIC, name), buf);

const tasks = [
  // Tab favicons — emit the brand-dark app-icon variant so the TG is visible
  // on every browser-chrome colour (light Chrome, dark Chrome, Safari pinned
  // tabs, Edge). Modern browsers prefer favicon.svg (which handles dark/light
  // via prefers-color-scheme) and only fall back to these PNGs.
  ['favicon-16.png', await png(masterSvg(64), 16)],
  ['favicon-32.png', await png(masterSvg(128), 32)],
  // iOS add-to-home (no transparency, brand-dark background).
  ['apple-touch-icon.png', await png(masterSvg(360), 180)],
  // Android PWA / Chrome.
  ['icon-192.png', await png(masterSvg(384), 192)],
  ['icon-512.png', await png(masterSvg(1024), 512)],
];

for (const [name, buf] of tasks) {
  await out(name, buf);
  console.log(`  wrote public/${name} (${buf.byteLength} bytes)`);
}

// PWA manifest. Keep it minimal — full PWA install isn't a launch goal, but
// having a manifest unlocks `theme-color` and proper icon resolution on
// Android Chrome's "Add to Home" prompt.
const manifest = {
  name: 'TopGun',
  short_name: 'TopGun',
  description: 'Real-time apps that survive offline.',
  start_url: '/',
  scope: '/',
  display: 'standalone',
  background_color: BG,
  theme_color: BG,
  icons: [
    { src: '/icon-192.png', sizes: '192x192', type: 'image/png', purpose: 'any' },
    { src: '/icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'any' },
    { src: '/favicon.svg', sizes: 'any', type: 'image/svg+xml' },
  ],
};
await writeFile(join(PUBLIC, 'site.webmanifest'), JSON.stringify(manifest, null, 2) + '\n');
console.log('  wrote public/site.webmanifest');
