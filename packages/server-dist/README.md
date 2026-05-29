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
