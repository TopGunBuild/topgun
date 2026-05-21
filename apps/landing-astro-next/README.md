# landing-astro-next

Experimental redesign of the TopGun landing page. **Temporary** — when the new
design is approved, the result migrates into `apps/docs-astro` and this
directory is deleted.

Stack: Astro 5, Tailwind 4, Inter + JetBrains Mono. No React, no MDX — keep the
prototype static and fast.

## Run

```sh
pnpm install
pnpm --filter apps-landing-astro-next dev
```

Dev server runs on port `4322` to avoid clashing with `apps/docs-astro` on
`4321`.

## Design system

- Theme: dark, single-accent (signal cyan) — see `src/styles/global.css`
- Atmosphere: subtle dot grid + animated sync pulses (CSS-only, respects
  `prefers-reduced-motion`)
- Section labels: `~ section.name` (Unix-path style, mono)
- Two-clause H2 with the second clause in accent color
- Contrast: all text passes WCAG AA on the dark background
