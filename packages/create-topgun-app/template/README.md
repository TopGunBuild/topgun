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

## How to connect to a server (optional)

By default, this app stores all data locally in IndexedDB — no server, no internet, no setup. To sync data across devices or share state with other users:

1. In `src/App.tsx`, uncomment the `serverUrl` line:
   ```ts
   const client = new TopGunClient({
     storage: new IDBAdapter(),
     serverUrl: 'ws://localhost:8080',
   });
   ```
2. Point `serverUrl` at any running TopGun server. For a quick local server, see [topgun.build/docs/intro](https://topgun.build/docs/intro).
3. Restart `pnpm dev`.

The app keeps working offline either way — writes apply instantly, and any backlog syncs when the connection returns.

## Where to learn more

- [Getting started guide](https://topgun.build/docs/intro)
- [Migrating from Firebase](https://topgun.build/docs/guides/migrating-from-firebase)
- [GitHub Discussions](https://github.com/topgunbuild/topgun/discussions) — questions, ideas, feedback
