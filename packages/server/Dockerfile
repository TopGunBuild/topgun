# syntax=docker/dockerfile:1

FROM node:20-alpine AS builder

# Install pnpm
RUN corepack enable && corepack prepare pnpm@10.13.1 --activate

WORKDIR /app

# Copy root configuration and workspace files
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json tsconfig.json ./

# Create directory for TLS certificates
RUN mkdir -p /etc/topgun/tls

# Copy packages source code
COPY packages packages

# Install dependencies
RUN pnpm install --frozen-lockfile

# Build core first (dependency)
RUN pnpm --filter @topgunbuild/core build

# Build server
RUN pnpm --filter @topgunbuild/server build

# --- Production Stage ---
FROM node:20-alpine
WORKDIR /app

# Create a non-root user (if not already present in alpine node image) or use existing 'node' user
# node:20-alpine comes with a 'node' user (uid 1000)
USER node

ENV NODE_ENV=production

# Copy root workspace files and node_modules
COPY --from=builder /app/package.json ./package.json
COPY --from=builder /app/pnpm-workspace.yaml ./pnpm-workspace.yaml
COPY --from=builder /app/node_modules ./node_modules

# Copy @topgunbuild/core package (workspace dependency)
COPY --from=builder /app/packages/core ./packages/core

# Copy @topgunbuild/server package
COPY --from=builder /app/packages/server ./packages/server

# Expose ports
# HTTP
EXPOSE 8080
# Cluster
EXPOSE 9080

# Healthcheck: supports both HTTP and HTTPS modes
# Uses wget with --no-check-certificate for self-signed certs in TLS mode
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD sh -c 'if [ "$TOPGUN_TLS_ENABLED" = "true" ]; then wget -q --no-check-certificate --spider https://localhost:${TOPGUN_PORT:-8080}/; else wget -q --spider http://localhost:${TOPGUN_PORT:-8080}/; fi' || exit 1

CMD ["node", "packages/server/dist/start-server.js"]
