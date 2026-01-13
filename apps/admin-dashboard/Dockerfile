# apps/admin-dashboard/Dockerfile
FROM node:20-alpine AS builder

WORKDIR /app

# Install pnpm
RUN npm install -g pnpm@10.13.1

# Copy workspace configuration
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/core/package.json ./packages/core/
COPY packages/client/package.json ./packages/client/
COPY packages/react/package.json ./packages/react/
COPY apps/admin-dashboard/package.json ./apps/admin-dashboard/

# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy source code
COPY packages/core ./packages/core
COPY packages/client ./packages/client
COPY packages/react ./packages/react
COPY apps/admin-dashboard ./apps/admin-dashboard
COPY tsconfig.json ./

# Build packages
RUN pnpm --filter @topgunbuild/core build
RUN pnpm --filter @topgunbuild/client build
RUN pnpm --filter @topgunbuild/react build
RUN pnpm --filter admin-dashboard build

# Production image - serve static files with nginx
FROM nginx:alpine

# Copy built assets
COPY --from=builder /app/apps/admin-dashboard/dist /usr/share/nginx/html

# Copy nginx config for SPA routing
RUN echo 'server { \
    listen 3000; \
    root /usr/share/nginx/html; \
    index index.html; \
    location / { \
        try_files $uri $uri/ /index.html; \
    } \
}' > /etc/nginx/conf.d/default.conf

EXPOSE 3000

CMD ["nginx", "-g", "daemon off;"]
