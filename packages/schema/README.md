# @topgunbuild/schema

[![npm](https://img.shields.io/npm/v/@topgunbuild/schema)](https://www.npmjs.com/package/@topgunbuild/schema) [![License](https://img.shields.io/npm/l/@topgunbuild/schema)](https://github.com/TopGunBuild/topgun/blob/main/LICENSE)

Schema definition DSL and code generation for [TopGun](https://topgun.build). Define your map schemas in TypeScript, generate JSON schemas and TypeScript types for both client and server.

## Install

```bash
npm install @topgunbuild/schema
```

## Overview

This package provides:

- **DSL (`defineMap`, `t`)** — Fluent API for defining map schemas with field types and constraints
- **SchemaRegistry** — Global registry for accumulating schema definitions
- **Codegen (`runCodegen`)** — Generate JSON schemas and TypeScript types from your definitions

## Quickstart

### 1. Define your schemas

Create a `topgun.schema.ts` file in your project:

```typescript
import { defineMap, t, schema } from '@topgunbuild/schema';

// Define a "users" map
schema.register(
  defineMap('users', { strict: true, version: 1 })
    .field('email', t.string(), { required: true, pattern: '^[^@]+@[^@]+$' })
    .field('name', t.string(), { required: true, minLength: 1, maxLength: 100 })
    .field('age', t.int(), { minValue: 0, maxValue: 150 })
    .field('roles', t.array(t.string()), { required: true })
    .field('metadata', t.map())
);

// Define a "posts" map
schema.register(
  defineMap('posts', { strict: false, version: 1 })
    .field('title', t.string(), { required: true, maxLength: 200 })
    .field('content', t.string(), { required: true })
    .field('published', t.bool())
    .field('tags', t.array(t.string()))
    .field('createdAt', t.timestamp(), { required: true })
);
```

### 2. Run codegen

```typescript
import { runCodegen } from '@topgunbuild/schema';

runCodegen({
  schemaPath: './topgun.schema.ts',
  outDir: './generated',
  typescript: true,
  json: true,
});
```

This generates:
- `generated/topgun.schema.json` — JSON schema for server-side validation
- `generated/topgun.types.ts` — TypeScript interfaces for type-safe client code

### 3. Use generated types

```typescript
import type { UserRecord, PostRecord, MapSchemas } from './generated/topgun.types';

const user: UserRecord = {
  email: 'alice@example.com',
  name: 'Alice',
  age: 30,
  roles: ['admin'],
};
```

## API Reference

### Field Types (`t`)

| Method | Output Type | TypeScript Type |
|--------|-------------|-----------------|
| `t.string()` | `"String"` | `string` |
| `t.int()` | `"Int"` | `number` |
| `t.float()` | `"Float"` | `number` |
| `t.bool()` | `"Bool"` | `boolean` |
| `t.binary()` | `"Binary"` | `Uint8Array` |
| `t.timestamp()` | `"Timestamp"` | `number` |
| `t.array(inner)` | `{ Array: inner }` | `T[]` |
| `t.map()` | `"Map"` | `Record<string, unknown>` |
| `t.any()` | `"Any"` | `unknown` |

### Field Constraints

| Option | Applies to | Description |
|--------|-----------|-------------|
| `required` | All types | Field must be present |
| `minLength` | String, Array | Minimum length |
| `maxLength` | String, Array | Maximum length |
| `minValue` | Int, Float | Minimum numeric value |
| `maxValue` | Int, Float | Maximum numeric value |
| `pattern` | String | Regex pattern for validation |
| `enumValues` | String | Allowed string values |

### `defineMap(name, options?)`

Create a new map schema builder.

```typescript
defineMap('users', { strict: true, version: 1 })
  .field('email', t.string(), { required: true })
  .field('name', t.string())
```

Options:
- `strict` (default: `false`) — Reject unknown fields
- `version` (default: `1`) — Schema version for migrations

### `schema.register(builder)`

Register a map schema with the global registry.

### `runCodegen(options?)`

Generate output files from registered schemas.

Options:
- `schemaPath` (default: `'./topgun.schema.ts'`) — Path to schema definition file
- `outDir` (default: `'./generated'`) — Output directory
- `typescript` (default: `true`) — Generate TypeScript types
- `json` (default: `true`) — Generate JSON schema

## Integration with Server

The generated `topgun.schema.json` file is used by the Rust server for runtime validation. Place it in your server's config directory or embed it in your deployment.

## Documentation

- Full docs: [topgun.build/docs](https://topgun.build/docs)
- GitHub: [TopGunBuild/topgun](https://github.com/TopGunBuild/topgun)

## License

Apache-2.0
