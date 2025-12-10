#!/bin/bash

# Load environment variables from .env if present
if [ -f .env ]; then
  export $(grep -v '^#' .env | xargs)
fi

# Configuration with defaults
# IMPORTANT: Replace these placeholder values with your own credentials

# JWT Secret (Clerk Public Key)
# Get your public key from: https://your-domain.clerk.accounts.dev/.well-known/jwks.json
DEFAULT_JWT_SECRET="-----BEGIN PUBLIC KEY-----
YOUR_CLERK_PUBLIC_KEY_HERE
-----END PUBLIC KEY-----"

export JWT_SECRET="${JWT_SECRET:-$DEFAULT_JWT_SECRET}"

# Database URL
# Format: postgresql://user:password@host:port/database?sslmode=require
DEFAULT_DATABASE_URL="postgresql://user:password@localhost:5432/topgun?sslmode=require"
export DATABASE_URL="${DATABASE_URL:-$DEFAULT_DATABASE_URL}"

# Port Configuration
export PORT="${PORT:-8080}"
export CLUSTER_PORT="${CLUSTER_PORT:-8081}"

# Storage Configuration
# Set to 'postgres' to use PostgreSQL, or 'memory' for in-memory (default is memory)
export STORAGE_MODE="${STORAGE_MODE:-memory}"

echo "----------------------------------------------------------------"
echo "Starting TopGun Server..."
echo "PORT: $PORT"

if [ "$JWT_SECRET" == "$DEFAULT_JWT_SECRET" ]; then
  echo "Auth: WARNING - Using placeholder JWT_SECRET. Set JWT_SECRET env var."
else
  echo "Auth: Using Custom JWT_SECRET."
fi

if [ "$DATABASE_URL" == "$DEFAULT_DATABASE_URL" ]; then
  echo "DB: WARNING - Using placeholder DATABASE_URL. Set DATABASE_URL env var."
else
  echo "DB: Using Custom Database."
fi
echo "Storage: $STORAGE_MODE"
echo "----------------------------------------------------------------"

npm run start:server
