#!/bin/bash
# .devcontainer/post-create.sh

set -e

echo "Setting up TopGun development environment..."

# Install dependencies
echo "Installing dependencies..."
pnpm install

# Build packages
echo "Building packages..."
pnpm build

echo ""
echo "Setup complete!"
echo ""
echo "Quick commands:"
echo "  pnpm dev           - Start development server"
echo "  pnpm test          - Run tests"
echo "  npx topgun dev     - Start with CLI"
echo "  npx topgun doctor  - Check environment"
echo ""
