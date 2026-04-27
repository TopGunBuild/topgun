# create-topgun-app

Scaffold a new [TopGun](https://topgun.build) offline-first app in seconds.

## Usage

```sh
# npm
npx create-topgun-app my-app

# pnpm
pnpm create topgun-app my-app

# yarn
yarn create topgun-app my-app

# bun
bunx create-topgun-app my-app
```

Then:

```sh
cd my-app
pnpm install
pnpm dev
```

Open [http://localhost:5173](http://localhost:5173) to see the app.

## What you get

A minimal React + TypeScript app pre-wired with:

- `TopGunClient` configured for offline-first local operation
- `IDBAdapter` for IndexedDB persistence (data survives page refresh)
- A working `LWWMap` todo demo — add and toggle tasks with zero network round-trips
- Vite dev server with hot module reload

## Local-only by default

The scaffolded app runs without a server. All reads and writes resolve from local IndexedDB storage. To connect a TopGun server, uncomment the `serverUrl` line in `src/App.tsx`.

Cloud hosting is on the roadmap. See [topgun.build/docs/roadmap](https://topgun.build/docs/roadmap).

## Requirements

- Node.js >= 18
- pnpm, npm, yarn, or bun

## Learn more

- [TopGun docs](https://topgun.build/docs/intro)
- [Migrating from Firebase](https://topgun.build/docs/guides/migrating-from-firebase)
- [GitHub](https://github.com/topgunbuild/topgun)
