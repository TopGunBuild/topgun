# admin-dashboard

The TopGun admin dashboard — a Vite + React 19 SPA that connects to a running TopGun server for operator workflows: browsing maps, inspecting cluster health, viewing recent operations, and reading metrics.

Private package — not published to npm. Shipped as a Docker image bundled with the server's `docker compose --profile admin` deploy.

## Quick start

```bash
# From the repo root
pnpm install
pnpm --filter admin-dashboard dev
```

Vite serves the SPA at `http://localhost:5173`. It expects a TopGun server reachable at `ws://localhost:8080` by default; override via `VITE_TOPGUN_SERVER_URL` for a remote server, and `VITE_TOPGUN_METRICS_URL` for the Prometheus endpoint.

## Build

```bash
pnpm --filter admin-dashboard build      # static bundle into dist/
pnpm --filter admin-dashboard preview    # serve the built bundle locally
pnpm --filter admin-dashboard typecheck
```

## Deploy via Docker Compose

```bash
# Boots admin-ui (port 3001) + server (8080) + postgres
docker compose --profile admin up
```

The `Dockerfile` does a two-stage build (Node → nginx) and serves the bundle at port 3000 in the container, mapped to host 3001 in `docker-compose.yml`. Set `VITE_TOPGUN_SERVER_URL` as a build-time arg if the server URL differs from the in-container default.

## Layout

```
src/
├── App.tsx                # Top-level shell + Server-Unavailable overlay
├── features/              # Feature folders (maps, cluster, metrics, setup wizard)
├── components/            # Reusable UI
└── lib/                   # Client wiring + helpers
```

## Contributing

See the repo-root [`CONTRIBUTING.md`](../../CONTRIBUTING.md).
