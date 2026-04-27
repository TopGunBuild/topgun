# TopGun Docs Site (`apps/docs-astro`)

Source for https://topgun.build — Astro + MDX + React docs site.

## 🚢 Deployment

Production https://topgun.build is hosted on **Cloudflare Pages** with GitHub Git integration. Identified 2026-04-27 via response-header probe (`server: cloudflare`, `cf-ray:`); no in-repo deploy config exists because the project is wired GitHub-side.

- **Trigger:** automatic on push to `main`. No CLI step required.
- **Manual retrigger:** Cloudflare Pages dashboard → project → Deployments → Redeploy.
- **Build source:** this directory (`apps/docs-astro/`); output dir `dist/`.
- **Do not commit** `vercel.json` / `netlify.toml` / `wrangler.toml` — a new in-repo config would conflict with the existing platform integration.

## 🚀 Project Structure

Inside of your Astro project, you'll see the following folders and files:

```text
/
├── public/
├── src/
│   └── pages/
│       └── index.astro
└── package.json
```

Astro looks for `.astro` or `.md` files in the `src/pages/` directory. Each page is exposed as a route based on its file name.

There's nothing special about `src/components/`, but that's where we like to put any Astro/React/Vue/Svelte/Preact components.

Any static assets, like images, can be placed in the `public/` directory.

## 🧞 Commands

All commands are run from the root of the project, from a terminal:

| Command                   | Action                                           |
| :------------------------ | :----------------------------------------------- |
| `npm install`             | Installs dependencies                            |
| `npm run dev`             | Starts local dev server at `localhost:4321`      |
| `npm run build`           | Build your production site to `./dist/`          |
| `npm run preview`         | Preview your build locally, before deploying     |
| `npm run astro ...`       | Run CLI commands like `astro add`, `astro check` |
| `npm run astro -- --help` | Get help using the Astro CLI                     |

## 👀 Want to learn more?

Feel free to check [our documentation](https://docs.astro.build) or jump into our [Discord server](https://astro.build/chat).
