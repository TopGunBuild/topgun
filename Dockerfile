FROM node:18-alpine

# Install pnpm
RUN corepack enable && corepack prepare pnpm@10.13.1 --activate

WORKDIR /app

# Copy pnpm workspace configuration
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json ./

# Copy workspace package definitions
COPY packages/core/package.json packages/core/package.json
COPY packages/server/package.json packages/server/package.json
COPY packages/client/package.json packages/client/package.json
COPY packages/adapters/package.json packages/adapters/package.json
COPY examples/todo-app/package.json examples/todo-app/package.json

# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy source code
COPY packages ./packages
COPY examples ./examples
COPY tsconfig.json ./

# Build all packages
RUN pnpm build

# Expose the WebSocket port
EXPOSE 8080

# Start the server using the example runner
CMD ["pnpm", "start:server"]
