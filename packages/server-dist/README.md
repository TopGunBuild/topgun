# @topgunbuild/server

Run a TopGun server with a single command — no Rust toolchain required.

```bash
npx @topgunbuild/server
```

This boots a zero-config server on `ws://localhost:8080` with embedded redb storage. Data survives restarts; no Postgres, no Docker, no Rust compiler needed.

## Installation

```bash
npm install @topgunbuild/server
# or
pnpm add @topgunbuild/server
```

Only one platform binary is downloaded — the one matching your OS and CPU. The others are skipped silently at install time.

## Supported Platforms

| Platform | Package |
|----------|---------|
| macOS (Apple Silicon / M1–M4) | `@topgunbuild/server-darwin-arm64` |
| Linux x64 | `@topgunbuild/server-linux-x64` |

Additional platforms (macOS Intel, Linux arm64, Windows) are planned.

## Usage

Start the server:

```bash
npx @topgunbuild/server
```

Or if installed globally / in your project:

```bash
topgun-server
```

The server listens on `ws://localhost:8080` by default. Set the `PORT` environment variable to change the port.

## Admin Dashboard

This package bundles the prebuilt admin dashboard SPA. With the server running, open
**http://localhost:8080/admin/** — the dashboard is served directly by the binary, with
no extra process, build step, or monorepo checkout required. On a zero-config no-auth
server (the `npx` default) it opens straight to the Dashboard, no login needed.

The bin shim resolves the bundled SPA relative to its own location and sets
`TOPGUN_ADMIN_DIR` automatically when you have not set it. Point `TOPGUN_ADMIN_DIR` at a
custom build directory to override; an explicit value is never overwritten.

> **Package size note.** Bundling the SPA grows this meta package from ~9 kB to roughly
> **316 kB packed / 1.1 MB unpacked** (v2.1.0). The increase is the Monaco-editor-heavy
> admin SPA (~991 kB JS + ~40 kB CSS, gzipping to ~298 kB). The SPA ships **only** here in
> the platform-independent meta package — the per-platform binary packages stay
> binary-only and well under 20 MB.

## Storage

The server uses embedded redb storage by default. Data is written to `./topgun.redb` in the current directory. No external database is required.

To use Postgres, set `STORAGE_BACKEND=postgres` and `DATABASE_URL=<your-postgres-url>`.

## Contributor / Monorepo Usage

Inside the TopGun monorepo, `pnpm start:server` either uses a prebuilt binary (if present) or falls back to:

```bash
cargo run --bin topgun-server --release
```

The prebuilt binary is produced by running:

```bash
bash scripts/build-server-binaries.sh
```

## License

Apache-2.0
