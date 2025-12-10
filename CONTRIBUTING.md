# Contributing to TopGun

Thank you for your interest in contributing to TopGun! This document provides guidelines and instructions for contributing.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Node.js** >= 18.0.0
- **pnpm** 10.13.1 or later (package manager)
- **Git**

### Installing pnpm

```bash
npm install -g pnpm@10.13.1
```

Or using corepack:

```bash
corepack enable
corepack prepare pnpm@10.13.1 --activate
```

## Getting Started

### 1. Fork and Clone

```bash
git clone https://github.com/YOUR_USERNAME/topgun.git
cd topgun
```

### 2. Install Dependencies

```bash
pnpm install
```

### 3. Build All Packages

```bash
pnpm build
```

This builds all packages in the monorepo in the correct order.

## Project Structure

```
topgun/
├── packages/           # Core packages (npm publishable)
│   ├── core/          # @topgunbuild/core - CRDT, types, utilities
│   ├── client/        # @topgunbuild/client - Browser/Node client
│   ├── server/        # @topgunbuild/server - WebSocket server
│   ├── adapters/      # @topgunbuild/adapters - Storage adapters
│   ├── react/         # @topgunbuild/react - React bindings
│   └── adapter-better-auth/  # @topgunbuild/adapter-better-auth
│
├── apps/              # Applications
│   ├── docs-astro/    # Documentation site
│   └── admin-dashboard/
│
├── examples/          # Example applications
│   ├── notes-app/     # PWA notes app with offline sync
│   ├── todo-app/      # Todo app example
│   └── ...
│
└── tests/             # Integration tests
    ├── e2e/           # End-to-end tests
    └── load/          # Load/performance tests
```

### Package Dependencies

The packages have the following dependency hierarchy:

```
@topgunbuild/core (no internal deps)
    ↓
@topgunbuild/client, @topgunbuild/server (depend on core)
    ↓
@topgunbuild/adapters, @topgunbuild/react (depend on client/server)
```

## Running Tests

### Unit Tests

Run all package tests:

```bash
pnpm test
```

Run tests for a specific package:

```bash
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/client test
pnpm --filter @topgunbuild/server test
```

### Test Coverage

```bash
pnpm test:coverage
```

Or for a specific package:

```bash
pnpm --filter @topgunbuild/core test:coverage
```

### E2E Tests

```bash
pnpm test:e2e
```

### Load Tests

```bash
pnpm test:load
```

## Development Workflow

### Working on a Package

1. Navigate to the package or work from root with filters:

```bash
# Build specific package
pnpm --filter @topgunbuild/core build

# Run tests in watch mode (if configured)
pnpm --filter @topgunbuild/core test
```

2. If your changes affect dependent packages, rebuild them:

```bash
pnpm build
```

### Running Examples

Start the development server:

```bash
pnpm start:server
```

Run an example app:

```bash
cd examples/todo-app
pnpm install
pnpm dev
```

## Pull Request Guidelines

### Before Submitting

1. **Create a feature branch** from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the code style guidelines

3. **Write/update tests** for your changes

4. **Run the full test suite**:
   ```bash
   pnpm test
   ```

5. **Build all packages** to ensure no TypeScript errors:
   ```bash
   pnpm build
   ```

### PR Requirements

- Clear, descriptive title
- Description of what changes were made and why
- Reference any related issues (e.g., "Fixes #123")
- All tests pass
- No TypeScript errors
- New features include tests

### Commit Messages

Use clear, descriptive commit messages:

```
feat(core): add new CRDT merge strategy
fix(client): resolve sync race condition
docs: update API documentation
test(server): add cluster integration tests
chore: update dependencies
```

Format: `type(scope): description`

Types: `feat`, `fix`, `docs`, `test`, `chore`, `refactor`, `perf`

## Code Style

### TypeScript

- Use TypeScript for all source code
- Enable strict mode
- Prefer explicit types over `any`
- Use interfaces for object shapes
- Export types alongside implementations

### General Guidelines

- Keep functions small and focused
- Write self-documenting code with clear naming
- Add comments for complex logic
- Follow existing patterns in the codebase

### Building

All packages use `tsup` for building. Each package outputs:
- CommonJS (`dist/index.js`)
- ESM (`dist/index.mjs`)
- Type declarations (`dist/index.d.ts`)

## Questions and Support

- **GitHub Issues**: For bug reports and feature requests
- **Discussions**: For questions and general discussion

## License

By contributing to TopGun, you agree that your contributions will be licensed under the BSL-1.1 license.
