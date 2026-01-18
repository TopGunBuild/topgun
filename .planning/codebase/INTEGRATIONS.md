# External Integrations

**Analysis Date:** 2026-01-18

## APIs & External Services

**Model Context Protocol (MCP):**
- Package: `packages/mcp-server`
- SDK: `@modelcontextprotocol/sdk` ^1.0.0
- Purpose: Enables AI assistants (Claude, Cursor) to interact with TopGun databases
- Config env vars: `TOPGUN_MCP_ENABLED`, `TOPGUN_MCP_PORT`, `TOPGUN_MCP_TOKEN`

**No other external APIs integrated** - TopGun is designed as a self-hosted data grid.

## Data Storage

**PostgreSQL (Production):**
- Client: `pg` ^8.11.0
- Adapter: `packages/server/src/storage/PostgresAdapter.ts`
- Connection: `DATABASE_URL` env var or individual `DB_HOST`, `DB_PORT`, `DB_USER`, `DB_PASSWORD`, `DB_NAME`
- Docker image: `postgres:16-alpine`
- Table schema: `topgun_maps` with JSONB values and HLC timestamps

**SQLite (Development):**
- Client: `better-sqlite3` ^11.7.0 (optional dependency)
- Adapter: `packages/server/src/storage/BetterSqlite3Adapter.ts`
- Connection: `DB_PATH` env var (default: `./topgun.db`)
- Features: WAL mode, 64MB cache, prepared statements
- Limitations: Single-node only, no clustering support

**In-Memory (Testing):**
- No external dependency
- Adapter: Memory-based storage
- Config: `STORAGE_MODE=memory`

**IndexedDB (Browser):**
- Wrapper: `idb` ^8.0.3
- Adapter: `packages/adapters/src/IDBAdapter.ts`
- Purpose: Client-side offline storage for CRDT data

## Authentication & Identity

**JWT Authentication (Built-in):**
- Library: `jsonwebtoken` ^9.0.2
- Implementation: Custom JWT verification in `packages/server/src/ServerCoordinator.ts`
- Config: `JWT_SECRET`, `JWT_EXPIRY` env vars
- Usage: WebSocket connection authentication

**BetterAuth Integration:**
- Package: `packages/adapter-better-auth`
- Library: `better-auth` ^1.0.0
- Adapter: `packages/adapter-better-auth/src/TopGunAdapter.ts`
- Purpose: Use TopGun as the database backend for BetterAuth
- Model mapping: `auth_user`, `auth_session`, etc.

**Bootstrap Admin Auth:**
- Location: `packages/server/src/bootstrap/BootstrapController.ts`
- Credentials: `TOPGUN_ADMIN_USER`, `TOPGUN_ADMIN_PASSWORD` (or `_FILE`)
- Storage: Admin credentials stored in SQLite/PostgreSQL during setup

## Monitoring & Observability

**Prometheus Metrics:**
- Library: `prom-client` ^15.1.0
- Service: `packages/server/src/monitoring/MetricsService.ts`
- Port: `METRICS_PORT` (default: 9091)
- Metrics exposed:
  - `topgun_connected_clients` - Active WebSocket connections
  - `topgun_ops_total` - Operation counts by type
  - `topgun_memory_usage_bytes` - Heap usage
  - `topgun_cluster_members` - Cluster node count
  - `topgun_distributed_search_*` - Search performance
  - `topgun_distributed_sub_*` - Subscription metrics
  - `topgun_event_queue_*` - Queue metrics
  - `topgun_backpressure_*` - Backpressure metrics

**Grafana Dashboards:**
- Provisioning: `deploy/grafana/provisioning/`
- Dashboards: `deploy/grafana/dashboards/`
- Docker: `grafana/grafana:latest`

**Logging:**
- Library: `pino` ^10.1.0
- Pretty printing: `pino-pretty` ^13.1.2 (dev)
- Config: `LOG_LEVEL` env var (fatal, error, warn, info, debug, trace)

## CI/CD & Deployment

**Container Registry:**
- Dockerfile: `deploy/Dockerfile.server`
- Base image: `node:20-alpine`
- Multi-stage build for minimal production image

**Docker Compose Profiles:**
- Default: `postgres`, `server`
- `admin`: Admin dashboard UI
- `monitoring`: Prometheus + Grafana
- `dbtools`: DbGate database browser
- `k6`: Load testing container
- `cluster`: 3-node cluster for testing
- `auto-setup`: Zero-touch automated configuration

**Health Checks:**
- HTTP endpoint: `GET /health`
- Docker: `wget -q --spider http://localhost:8080/health`

## Environment Configuration

**Required env vars (production):**
```
DATABASE_URL=postgres://user:pass@host:5432/db
JWT_SECRET=your-secret-key-min-32-chars-long
```

**Optional env vars:**
```
STORAGE_MODE=sqlite|postgres|memory
SERVER_PORT=8080
METRICS_PORT=9091
LOG_LEVEL=info
NODE_ID=node-1

# Clustering
CLUSTER_ENABLED=true
CLUSTER_SEEDS=ws://node-2:9080,ws://node-3:9080
CLUSTER_PORT=9080
TOPGUN_PEERS=node-2:9080,node-3:9080

# Auto-setup
TOPGUN_AUTO_SETUP=true
TOPGUN_DEPLOYMENT_MODE=standalone|cluster
TOPGUN_STORAGE_TYPE=sqlite|postgres|memory
TOPGUN_ADMIN_USER=admin
TOPGUN_ADMIN_PASSWORD=password
TOPGUN_ADMIN_PASSWORD_FILE=/run/secrets/admin_password

# Integrations
TOPGUN_MCP_ENABLED=false
TOPGUN_VECTOR_ENABLED=false
```

**Secrets location:**
- Development: `.env` file (gitignored)
- Production: Docker secrets at `/run/secrets/`
- Example files: `.env.example`, `.env.auto-setup.example`

## Webhooks & Callbacks

**Incoming:**
- None - TopGun uses WebSocket for real-time communication

**Outgoing:**
- None - TopGun does not call external webhooks

## WebSocket Protocol

**Client Connections:**
- Port: `SERVER_PORT` (default: 8080)
- Protocol: Custom binary protocol using msgpackr
- Auth: JWT token in connection handshake

**Cluster Communication:**
- Port: `CLUSTER_PORT` (default: 9080)
- Mesh topology: Each node connects to all peers
- Library: `ws` WebSocketServer

## Native Bindings

**xxHash64 (Optional):**
- Package: `packages/native`
- Purpose: High-performance hashing for Merkle tree
- Fallback: Pure JavaScript implementation if native build fails
- Platforms: darwin, linux, win32 (x64, arm64)
- Build: node-gyp via `binding.gyp`

---

*Integration audit: 2026-01-18*
