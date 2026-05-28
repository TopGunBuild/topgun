# @topgunbuild/cli

Unified developer CLI for [TopGun](https://topgun.build) — an offline-first real-time data platform.

## Install

```bash
# Global install
npm install -g @topgunbuild/cli

# Or run without install
npx @topgunbuild/cli <command>
```

## Quick Start

```bash
# Check your environment
topgun doctor

# Interactive project setup (writes .env with zero-config defaults)
topgun setup

# Start the development server (requires topgun-server binary)
topgun dev
```

## Commands

| Command | Description |
|---------|-------------|
| `topgun doctor` | Check environment setup and dependencies |
| `topgun setup [-y\|--yes]` | Interactive project setup (writes `.env`) |
| `topgun dev [--no-db] [-p port] [--debug]` | Start development server |
| `topgun test [scope] [--coverage]` | Run tests (core, client, server, e2e, k6:smoke) |
| `topgun config [--storage type] [--show]` | Manage configuration |
| `topgun codegen [--schema path] [--out-dir dir]` | Generate types from schema file |
| `topgun cluster:start` | Start local cluster via Docker Compose |
| `topgun cluster:stop` | Stop local cluster |
| `topgun cluster:status` | Show cluster status |
| `topgun docker:start [--with profiles]` | Start Docker services |
| `topgun docker:stop` | Stop all Docker services |
| `topgun docker:status` | Show Docker service status |
| `topgun docker:logs [-s service] [-f]` | Show Docker logs |
| `topgun debug:crdt <action>` | CRDT debugging tools |
| `topgun search:explain` | Explain search score breakdown |

## Setup Flow

After running `topgun setup`, a `.env` file is written with:

- `TOPGUN_NO_AUTH=1` — local dev runs without auth tokens; set `JWT_SECRET` for any networked/production deployment
- `STORAGE_BACKEND=redb` — zero-config embedded storage (no Postgres, no Docker required)

Then `topgun dev` starts the server using those settings.

## Storage Options

| Value | Description |
|-------|-------------|
| `redb` | Embedded storage (default, zero dependencies) |
| `postgres` | PostgreSQL (requires `DATABASE_URL`) |
| `null` | Ephemeral in-memory only |

## Note on `topgun dev`

The `dev` command starts a prebuilt `target/release/topgun-server` binary from the current working directory. This binary is built from the [TopGun monorepo](https://github.com/TopGunBuild/topgun). A future release will ship prebuilt server binaries (see TODO-365).

## License

Apache-2.0
