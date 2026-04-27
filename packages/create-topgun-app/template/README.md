# My TopGun App

This app was scaffolded by `create-topgun-app`. It runs in offline-first local mode by default — no server required for the demo.

## Next 5 commands

```sh
1. cd {{appName}}
2. pnpm install   (or npm install)
3. pnpm dev
4. Open http://localhost:5173
5. Edit src/App.tsx and watch live reload
```

## How to connect to a server

By default, the app operates fully offline using IndexedDB for local storage. To sync data with a TopGun server:

1. Start a TopGun server (see [topgun.build/docs/intro](https://topgun.build/docs/intro))
2. In `src/App.tsx`, uncomment the `serverUrl` line:
   ```ts
   const client = new TopGunClient({
     storage: new IDBAdapter('topgun-app'),
     serverUrl: 'ws://localhost:8080',
   });
   ```
3. Restart `pnpm dev`

## Where to learn more

- [Getting started guide](https://topgun.build/docs/intro)
- [Migrating from Firebase](https://topgun.build/docs/guides/migrating-from-firebase)
- [GitHub Discussions](https://github.com/topgunbuild/topgun/discussions) — questions, ideas, feedback
