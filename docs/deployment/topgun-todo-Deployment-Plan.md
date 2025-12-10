# TopGun Deployment Plan

## Goal
Deploy the TopGun server to a VPS using Docker and ensure logs are correctly collected and accessible.

## Prerequisites
- **VPS**: Linux server (Ubuntu/Debian recommended) with public IP.
- **Docker**: Installed and running (`docker --version`).
- **Docker Compose**: Installed (`docker-compose --version` or `docker compose version`).
- **Access**: SSH access to the VPS.
- **Source**: Access to the TopGun codebase or pre-built Docker images.

## Logging Strategy
- **Application**: The server uses `pino` (configured in `packages/server/src/utils/logger.ts`).
  - **Production**: Outputs JSON to `stdout` (best for tools like Filebeat, Fluentd, or simple Docker logging).
  - **Development**: Outputs pretty-printed text (if `NODE_ENV != production`).
- **Collection**: Docker Daemon captures `stdout`/`stderr`.
- **Storage**: Docker `json-file` driver (default).
- **Rotation**: **CRITICAL**. Must be configured to prevent disk exhaustion.
- **Access**: 
  - Real-time: `docker-compose logs -f server`
  - History: `docker logs topgun-server`

## Deployment Steps

### 1. Preparation (Local)
- [ ] **Verify Dockerfile**: Ensure `packages/server/Dockerfile` builds correctly.
- [ ] **Prepare Config**: Create a production `.env` file (do NOT commit this).
  ```env
  NODE_ENV=production
  DATABASE_URL=postgres://user:pass@postgres:5432/topgun
  LOG_LEVEL=info
  # Add other secrets here (JWT_SECRET, etc.)
  ```

### 2. Server Setup (VPS)
- [ ] **Install Docker**: If not present.
- [ ] **Create Directory**: `mkdir -p /opt/topgun`
- [ ] **Transfer Files**:
  - Copy `docker-compose.yml` to `/opt/topgun/`.
  - Copy `.env` to `/opt/topgun/`.
  - (Option A - Build on VPS): Copy entire source code.
  - (Option B - Registry): `docker login` and pull image.

### 3. Docker Compose Configuration
Update `docker-compose.yml` to include log rotation and restart policies:

```yaml
version: '3.8'
services:
  server:
    # ... existing config ...
    restart: always
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"
    environment:
      - LOG_LEVEL=info
      - NODE_ENV=production
      # ... other env vars
```

### 4. Execution
- [ ] **Start Services**:
  ```bash
  cd /opt/topgun
  docker-compose up -d
  ```

### 5. Verification
- [ ] **Check Containers**: `docker-compose ps` (should show 'Up').
- [ ] **Check Logs**: `docker-compose logs -f server` (look for "TopGun Server Starting").
- [ ] **Health Check**: `curl http://localhost:8080/` (should return 200 or 404, not connection refused).
- [ ] **Database Connection**: Verify logs show successful DB connection.

## Troubleshooting
- **Container Exits Immediately**: Check logs `docker logs topgun-server`. Likely missing env vars or DB connection failure.
- **DB Connection Failed**: Ensure `postgres` service is healthy and `DATABASE_URL` is correct (host should be `postgres` inside docker network).
