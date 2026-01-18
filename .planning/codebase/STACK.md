# Technology Stack

**Analysis Date:** 2026-01-18

## Languages

**Primary:**
- TypeScript 5.x - All source code across all packages

**Secondary:**
- C++ - Native xxHash64 implementation in `packages/native/src/*.cc`
- JavaScript - k6 load tests in `tests/k6/scenarios/*.js`

## Runtime

**Environment:**
- Node.js >= 18.0.0 (enforced in all `package.json` engines)
- Production Dockerfile uses Node.js 20 Alpine: `node:20-alpine`

**Package Manager:**
- pnpm 10.13.1 (specified in root `package.json` packageManager field)
- Lockfile: `pnpm-lock.yaml` (present)
- Workspace configuration: `pnpm-workspace.yaml`

## Frameworks

**Core:**
- No traditional framework - Custom WebSocket server architecture
- React >= 16.8.0 - React bindings for client integration (`packages/react`)

**Testing:**
- Jest 29.7.0 - Unit/integration tests across all packages
- Vitest 4.x - Benchmarks in `packages/core`
- k6 - Load testing with xk6-msgpack extension

**Build/Dev:**
- tsup 8.5.x - TypeScript bundler (outputs CJS + ESM + .d.ts)
- Vite 7.x - Admin dashboard and example apps
- Astro 5.x - Documentation site (`apps/docs-astro`)
- tsc - Type checking and native package compilation

## Key Dependencies

**Critical (packages/core):**
- `msgpackr` ^1.11.8 - Binary serialization for wire protocol
- `zod` ^4.1.13 - Runtime schema validation for messages
- `sorted-btree` ^2.1.0 - Efficient sorted data structure for indexes

**Critical (packages/client):**
- `idb` ^8.0.3 - IndexedDB wrapper for browser storage
- `pino` ^10.1.0 - Structured logging

**Critical (packages/server):**
- `ws` ^8.18.3 - WebSocket server implementation
- `pg` ^8.11.0 - PostgreSQL client
- `jsonwebtoken` ^9.0.2 - JWT authentication
- `prom-client` ^15.1.0 - Prometheus metrics

**Optional (packages/server):**
- `better-sqlite3` ^11.7.0 - SQLite for development mode
- `isolated-vm` ^4.7.2 - Sandboxed JavaScript execution

**Optional (packages/native):**
- `node-addon-api` ^8.3.0 - N-API bindings for native xxHash64

**Infrastructure:**
- `@modelcontextprotocol/sdk` ^1.0.0 - MCP server for AI assistant integration
- `better-auth` ^1.0.0 - Authentication framework adapter

## Configuration

**Environment:**
- Configuration via `.env` files (dotenv ^16.4.5)
- Key vars: `STORAGE_MODE`, `DATABASE_URL`, `SERVER_PORT`, `JWT_SECRET`
- Docker secrets for production passwords: `/run/secrets/admin_password`

**Build:**
- Root `tsconfig.json` - Shared TypeScript config with path aliases
- Per-package `tsup.config.ts` - Bundle configuration
- Per-package `jest.config.js` - Test configuration

**TypeScript Settings:**
- Target: ES2020
- Module: ESNext
- Module Resolution: bundler
- Strict mode enabled

## Platform Requirements

**Development:**
- Node.js >= 18
- pnpm 10.x
- Optional: Docker for PostgreSQL, Prometheus, Grafana

**Production:**
- Node.js 20 (Docker image)
- PostgreSQL 16 (recommended) or SQLite (single-node only)
- Docker Compose for orchestration

## Monorepo Structure

**Workspaces:**
```
packages/
  core/           # CRDTs, HLC, MerkleTree, schemas
  client/         # Browser/Node SDK
  server/         # WebSocket server, clustering
  react/          # React hooks and provider
  adapters/       # Storage implementations (IndexedDB)
  native/         # Native xxHash64 (optional)
  adapter-better-auth/  # BetterAuth integration
  mcp-server/     # Model Context Protocol server

apps/
  admin-dashboard/  # React admin UI (Vite)
  docs-astro/       # Documentation (Astro)

examples/
  notes-app/        # Demo React app
  todo-app/         # Demo React app
```

**Package Dependencies:**
```
@topgunbuild/core (no internal deps)
    |
    +-- @topgunbuild/client
    |       |
    |       +-- @topgunbuild/adapters
    |       +-- @topgunbuild/react
    |       +-- @topgunbuild/adapter-better-auth
    |       +-- @topgunbuild/mcp-server
    |
    +-- @topgunbuild/server
            |
            +-- @topgunbuild/native (optional)
```

---

*Stack analysis: 2026-01-18*
