---
id: SPEC-129
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-19
source: TODO-129
---

# SPEC-129: TypeScript Schema DSL and Codegen Toolchain

## Context

TODO-069 (Schema System) slice 3 of 4. SPEC-127 delivered the Rust schema types (`FieldType`, `FieldConstraint`, `MapSchema`, `FieldDef`) and validation engine in `core-rust`. SPEC-128 wired schema validation into the CrdtService write path. The server now validates incoming writes against registered `MapSchema` instances, but there is no developer-facing way to define schemas.

Currently, schemas must be constructed as raw `MapSchema` JSON objects and registered via `SchemaProvider::register_schema` at runtime. This is error-prone, provides no TypeScript autocompletion for client code, and requires manual synchronization between Rust types and TS types.

The project's stated strategy (PROJECT.md) is: "TypeScript-first schema strategy (developers define Zod schemas, build step generates Rust)." This spec delivers the toolchain that makes that strategy concrete.

## Goal Analysis

**Goal Statement:** Developers define schemas once in TypeScript and a build step produces both Rust-compatible schema registration payloads and TypeScript client types with full autocompletion.

**Observable Truths:**
1. Developer creates a `topgun.schema.ts` file using a fluent builder DSL that expresses all 9 `FieldType` variants and all 6 `FieldConstraint` options
2. Running `topgun codegen` reads the schema file, extracts schema definitions, and writes output files
3. A JSON intermediate format (`topgun.schema.json`) is produced that exactly matches the Rust `MapSchema` serde shape, enabling any language to consume it
4. A generated TypeScript file provides typed interfaces per map (e.g., `UserRecord`) with field types matching the DSL definition, enabling autocompletion in client code
5. A generated Rust file (or JSON asset) provides schema registration code that can be loaded at server startup, calling `SchemaProvider::register_schema` for each map
6. Round-trip fidelity: the JSON intermediate format deserializes into Rust `MapSchema` via `serde_json` without transformation

**Required Artifacts:**
- `packages/schema/` — new TS package containing DSL builder and codegen logic
- `packages/schema/src/dsl.ts` — builder API (`defineMap`, `field`, `string`, `int`, `float`, etc.)
- `packages/schema/src/codegen.ts` — reads schema definitions, writes JSON + TS + Rust outputs
- `packages/schema/src/types.ts` — internal types mirroring Rust schema structs
- `packages/schema/src/__tests__/dsl.test.ts` — DSL builder tests
- `packages/schema/src/__tests__/codegen.test.ts` — codegen output tests
- `bin/commands/codegen.js` — CLI command wiring
- Example `topgun.schema.ts` — demonstrates usage

**Key Links:**
- DSL builder types map 1:1 to `FieldType` enum variants in `core-rust/src/schema.rs`
- JSON intermediate format must match `MapSchema` serde shape (snake_case for `field_type`, camelCase for `FieldConstraint` fields)
- Generated TS types are consumed by `@topgunbuild/client` users for autocompletion
- Generated JSON is consumed by Rust server at startup via `serde_json::from_str::<MapSchema>()`

## Task

1. Create new `packages/schema/` TypeScript package with `package.json`, `tsconfig.json`, `tsup.config.ts`
2. Implement a fluent builder DSL in `packages/schema/src/dsl.ts` that mirrors all `FieldType` variants and `FieldConstraint` options
3. Define an internal JSON schema format in `packages/schema/src/types.ts` that matches the Rust `MapSchema` serde representation exactly
4. Implement codegen logic in `packages/schema/src/codegen.ts` that:
   a. Loads a `topgun.schema.ts` file by importing it (requires ts-node or tsx)
   b. Serializes schema definitions to `topgun.schema.json` (intermediate format)
   c. Generates a TypeScript types file with interfaces per map
   d. Generates a Rust-compatible JSON asset file that the server can load at startup
5. Add `topgun codegen` command to `bin/topgun.js`
6. Write unit tests for DSL builder and codegen output correctness

## Requirements

### File: `packages/schema/package.json` (create)

```json
{
  "name": "@topgunbuild/schema",
  "version": "0.12.0",
  "main": "./dist/index.js",
  "module": "./dist/index.mjs",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "import": "./dist/index.mjs",
      "require": "./dist/index.js"
    }
  },
  "files": [
    "dist"
  ],
  "sideEffects": false,
  "publishConfig": {
    "access": "public"
  },
  "scripts": {
    "build": "tsup",
    "test": "jest"
  },
  "devDependencies": {
    "typescript": "^5.0.0"
  },
  "engines": {
    "node": ">=18"
  },
  "license": "Apache-2.0",
  "repository": {
    "type": "git",
    "url": "https://github.com/TopGunBuild/topgun.git",
    "directory": "packages/schema"
  }
}
```

No dependency on `ts-morph`. The codegen imports the schema file directly via `tsx` / `ts-node` (already a project devDependency). This avoids a heavy AST parsing dependency.

### File: `packages/schema/src/types.ts` (create)

Internal types that mirror the Rust serde representation exactly:

**`JsonFieldType`** — union type matching Rust `FieldType` enum serde output:
- `"String"` | `"Int"` | `"Float"` | `"Bool"` | `"Binary"` | `"Timestamp"` | `{ Array: JsonFieldType }` | `"Map"` | `"Any"`

**`JsonFieldConstraint`** — interface matching Rust `FieldConstraint` (camelCase):
- `minLength?: number`
- `maxLength?: number`
- `minValue?: number`
- `maxValue?: number`
- `pattern?: string`
- `enumValues?: string[]`

Note: `minValue` and `maxValue` correspond to Rust `i64` fields. JavaScript `number` is a 64-bit float with safe integer range up to `Number.MAX_SAFE_INTEGER` (2^53 - 1). Values outside this range silently lose precision. Schema constraints are expected to stay well within this range in practice; this is a known limitation documented here for completeness.

**`JsonFieldDef`** — interface matching Rust `FieldDef` (snake_case):
- `name: string`
- `required: boolean`
- `field_type: JsonFieldType`
- `constraints?: JsonFieldConstraint`

**`JsonMapSchema`** — interface matching Rust `MapSchema` (snake_case):
- `version: number`
- `fields: JsonFieldDef[]`
- `strict: boolean`

**`JsonSchemaFile`** — top-level codegen output:
- `maps: Record<string, JsonMapSchema>` — keyed by map name

### File: `packages/schema/src/dsl.ts` (create)

Fluent builder API. Public exports:

**`defineMap(name: string, options?: { strict?: boolean; version?: number })`** — returns a `MapBuilder` instance

**`MapBuilder` class:**
- `field(name: string, type: JsonFieldType, options?: FieldOptions): this` — add a field definition
- `build(): { name: string; schema: JsonMapSchema }` — finalize and return the map name + JSON schema

**`FieldOptions` interface:**
- `required?: boolean` (default: `false`)
- `minLength?: number`
- `maxLength?: number`
- `minValue?: number`
- `maxValue?: number`
- `pattern?: string`
- `enumValues?: string[]`

**`t` namespace — factory functions (one per FieldType variant):**
- `t.string()` — returns `"String"` (`JsonFieldType`)
- `t.int()` — returns `"Int"` (`JsonFieldType`)
- `t.float()` — returns `"Float"` (`JsonFieldType`)
- `t.bool()` — returns `"Bool"` (`JsonFieldType`)
- `t.binary()` — returns `"Binary"` (`JsonFieldType`)
- `t.timestamp()` — returns `"Timestamp"` (`JsonFieldType`)
- `t.array(inner: JsonFieldType)` — returns `{ Array: inner }` (`JsonFieldType`)
- `t.map()` — returns `"Map"` (`JsonFieldType`)
- `t.any()` — returns `"Any"` (`JsonFieldType`)

The `t` namespace is exported as a named export for ergonomic schema definitions.

**`SchemaRegistry` class:**
- `register(builder: MapBuilder): void` — adds a built map schema to the registry
- `getSchemas(): Record<string, JsonMapSchema>` — returns all registered schemas
- Static `global` instance for use by `topgun.schema.ts` files

**Example `topgun.schema.ts`:**
```typescript
import { defineMap, t, schema } from '@topgunbuild/schema';

schema.register(
  defineMap('users', { strict: true, version: 1 })
    .field('name', t.string(), { required: true, maxLength: 100 })
    .field('email', t.string(), { required: true, pattern: '^[^@]+@[^@]+$' })
    .field('age', t.int(), { minValue: 0, maxValue: 150 })
    .field('tags', t.array(t.string()), { maxLength: 10 })
    .field('metadata', t.map())
);

schema.register(
  defineMap('posts', { version: 1 })
    .field('title', t.string(), { required: true, maxLength: 200 })
    .field('body', t.string(), { required: true })
    .field('authorId', t.string(), { required: true })
    .field('createdAt', t.timestamp(), { required: true })
    .field('published', t.bool())
);
```

### File: `packages/schema/src/codegen.ts` (create)

**`runCodegen(options: CodegenOptions): void`**

`CodegenOptions`:
- `schemaPath: string` — path to `topgun.schema.ts` (default: `./topgun.schema.ts`)
- `outDir: string` — output directory (default: `./generated`)
- `typescript: boolean` — generate TS types file (default: `true`)
- `json: boolean` — generate JSON intermediate file (default: `true`)

**Codegen steps:**
1. Import the schema file using `require()` (the file must be pre-compiled or run via ts-node/tsx registered in the CLI)
2. Access `SchemaRegistry.global.getSchemas()` to get all registered schemas
3. If `json`: write `topgun.schema.json` containing `JsonSchemaFile` (the intermediate format)
4. If `typescript`: write `topgun.types.ts` containing:
   - One interface per map, named by the following rule: strip a trailing `s` (if present) to singularize, apply PascalCase, then append `Record` — e.g., map name `users` → `UserRecord`, `posts` → `PostRecord`, `metadata` → `MetadataRecord` (no trailing `s` to strip). For irregular plurals that do not end in `s`, the map name is used as-is in PascalCase (e.g., `people` → `PeopleRecord`).
   - Field types mapped: `String` -> `string`, `Int` -> `number`, `Float` -> `number`, `Bool` -> `boolean`, `Binary` -> `Uint8Array`, `Timestamp` -> `number`, `Array(T)` -> `T[]` (where T is the mapped TypeScript type of the inner element, e.g., `Array(String)` -> `string[]`, `Array(Array(Int))` -> `number[][]`), `Map` -> `Record<string, unknown>`, `Any` -> `unknown`
   - Required fields are non-optional; non-required fields use `?`
   - A `MapSchemas` type mapping map names to record types

**TS type mapping table:**

| FieldType | TypeScript Type |
|-----------|----------------|
| String | `string` |
| Int | `number` |
| Float | `number` |
| Bool | `boolean` |
| Binary | `Uint8Array` |
| Timestamp | `number` |
| Array(T) | `T[]` (concrete mapped type, e.g., `string[]`, `number[][]`) |
| Map | `Record<string, unknown>` |
| Any | `unknown` |

### File: `packages/schema/src/index.ts` (create)

Re-exports public API: `defineMap`, `t`, `schema` (the global `SchemaRegistry`), `MapBuilder`, `FieldOptions`, codegen types.

### File: `packages/schema/src/__tests__/dsl.test.ts` (create)

Tests:
- `defineMap` creates a builder; `build()` returns correct `JsonMapSchema`
- All 9 field type builders produce correct `JsonFieldType` values
- `t.array(t.string())` produces `{ Array: "String" }`
- Nested arrays: `t.array(t.array(t.int()))` produces `{ Array: { Array: "Int" } }`
- Field options map to correct `JsonFieldConstraint` fields
- `required: true` sets `required: true` in `JsonFieldDef`
- Default `strict` is `false`, default `version` is `1`
- `SchemaRegistry.register` accumulates schemas; `getSchemas` returns all

### File: `packages/schema/src/__tests__/codegen.test.ts` (create)

Tests:
- JSON output matches expected `JsonSchemaFile` structure
- JSON output deserializes to the exact serde shape that Rust expects (field names, casing)
- TypeScript output contains correct interface definitions
- Required fields are non-optional; optional fields use `?`
- `Binary` maps to `Uint8Array`
- `Timestamp` maps to `number`
- `Array(String)` maps to `string[]`
- Empty schema produces empty output

### File: `bin/commands/codegen.js` (create)

CLI command handler:
- Reads `--schema` flag (default: `./topgun.schema.ts`)
- Reads `--out-dir` flag (default: `./generated`)
- Reads `--no-typescript` flag to skip TS generation
- Reads `--no-json` flag to skip JSON generation
- Registers `ts-node` or `tsx` for TypeScript execution if needed
- Calls `runCodegen()` from `@topgunbuild/schema`
- Prints summary of generated files

### File: `bin/topgun.js` (modify)

Add the `codegen` command:
```javascript
program
  .command('codegen')
  .description('Generate types and schema files from topgun.schema.ts')
  .option('--schema <path>', 'Path to schema file', './topgun.schema.ts')
  .option('--out-dir <dir>', 'Output directory', './generated')
  .option('--no-typescript', 'Skip TypeScript type generation')
  .option('--no-json', 'Skip JSON schema generation')
  .action(require('./commands/codegen'));
```

## Acceptance Criteria

1. `@topgunbuild/schema` package exists with `package.json`, builds via `pnpm build`, and exports `defineMap`, `t`, `schema`
2. All 9 `FieldType` variants are expressible via `t.*()` functions: `t.string()`, `t.int()`, `t.float()`, `t.bool()`, `t.binary()`, `t.timestamp()`, `t.array(inner)`, `t.map()`, `t.any()`
3. All 6 `FieldConstraint` options are expressible via `FieldOptions`: `minLength`, `maxLength`, `minValue`, `maxValue`, `pattern`, `enumValues`
4. `MapBuilder.build()` returns a `JsonMapSchema` whose JSON representation round-trips through `serde_json::from_str::<MapSchema>()` in Rust without error
5. `topgun codegen` CLI command reads a `topgun.schema.ts` file and generates output files
6. Generated `topgun.schema.json` contains `JsonSchemaFile` with correct structure: `field_type` in snake_case, `FieldConstraint` fields in camelCase, matching the Rust serde shape exactly
7. Generated `topgun.types.ts` contains TypeScript interfaces with correct field types and optionality
8. `required: true` fields produce non-optional TS properties; `required: false` (or omitted) produce optional (`?`) properties
9. `FieldType::Binary` maps to `Uint8Array` in generated TS types
10. `FieldType::Array(String)` maps to `string[]` in generated TS types
11. Nested arrays (`t.array(t.array(t.int()))`) produce `number[][]` in generated TS types
12. `topgun codegen --schema ./path/to/schema.ts --out-dir ./output` works with custom paths
13. Unit tests pass for DSL builder (all type builders, constraints, registry)
14. Unit tests pass for codegen (JSON output shape, TS output correctness)

## Validation Checklist

1. Create a `topgun.schema.ts` with the example users/posts schema above; run `topgun codegen` — produces `topgun.schema.json` and `topgun.types.ts` in `./generated/`
2. Copy the generated `topgun.schema.json` to a Rust test, deserialize each map value via `serde_json::from_str::<MapSchema>()` — all succeed without error
3. Open the generated `topgun.types.ts` in an IDE — `UserRecord.name` is `string` (non-optional), `UserRecord.age` is `age?: number` (optional), `UserRecord.tags` is `tags?: string[]`
4. Run `pnpm --filter @topgunbuild/schema test` — all tests pass
5. Run `pnpm build` — `@topgunbuild/schema` builds successfully

## Constraints

- Do NOT use `ts-morph` for schema extraction — import the schema file directly (simpler, faster)
- Do NOT modify any Rust source files — this spec is purely TypeScript
- Do NOT auto-register schemas with the server — the generated JSON is a static asset loaded separately
- Do NOT generate Zod schemas — the DSL is the source of truth, not Zod (PROJECT.md says "TypeScript-first schema strategy")
- Do NOT add runtime validation to the client — validation happens server-side via `SchemaService`
- Do NOT modify `@topgunbuild/core` or `@topgunbuild/client` — this is a new standalone package
- Keep `@topgunbuild/schema` dependency-free (codegen takes a single `--schema` path; no file-discovery step requires additional runtime dependencies)

## Assumptions

- `ts-node` (already a root devDependency) is sufficient for executing `topgun.schema.ts` files during codegen; no need for `tsx` as an additional dependency
- PascalCase `FieldType` variant names in JSON (e.g., `"String"`, `"Int"`) match the Rust serde default for enums without `rename_all` (confirmed: SPEC-127 explicitly does NOT add `rename_all` to `FieldType`)
- `FieldConstraint` fields in JSON use camelCase (e.g., `minLength`, `enumValues`) matching the Rust `#[serde(rename_all = "camelCase")]` on `FieldConstraint`
- `FieldDef` fields in JSON use snake_case (e.g., `field_type`, not `fieldType`) matching Rust's default serde behavior (SPEC-127 does NOT add `rename_all` to `FieldDef`)
- `MapSchema` fields in JSON use snake_case (SPEC-127 does NOT add `rename_all` to `MapSchema`)
- The `topgun.schema.ts` file uses ES module or CommonJS exports; codegen handles both by importing the file and reading the global `SchemaRegistry`
- Generated TypeScript types are plain interfaces (not Zod schemas or runtime validators) — they are for IDE autocompletion only
- The generated JSON file is loaded by the Rust server at startup via a separate mechanism (not part of this spec)
- Version `0.12.0` for the new package matches the current monorepo version

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `packages/schema/` package scaffolding (package.json, tsconfig, tsup config); define `types.ts` with JSON schema types mirroring Rust serde shapes | — | ~10% |
| G2 | 2 | Implement DSL builder in `dsl.ts` (defineMap, MapBuilder, t.* type factories, SchemaRegistry, FieldOptions); implement `index.ts` re-exports | G1 | ~20% |
| G3 | 2 | Implement codegen in `codegen.ts` (schema file loading, JSON serialization, TS type generation); create `bin/commands/codegen.js` and wire into `bin/topgun.js` | G1 | ~25% |
| G4 | 3 | Unit tests: `dsl.test.ts` (builder, type factories, constraints, registry) and `codegen.test.ts` (JSON output shape, TS output correctness, round-trip validation) | G2, G3 | ~15% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2, G3 | Yes | 2 |
| 3 | G4 | No | 1 |

**Total workers needed:** 2 (max in Wave 2)

## Audit History

### Audit v1 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~45% total (within GOOD range)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~45% | <=50% | OK |
| Largest task group | ~25% (G3) | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimension Assessment:**

- Clarity: Strong. Title, context, and task description are specific. The DSL API is fully specified with examples.
- Completeness: All files listed with detailed requirements. Serde shape verified against actual Rust source -- all assumptions about field casing are correct.
- Testability: All 14 acceptance criteria are concrete and measurable.
- Scope: Clear boundaries with explicit constraints. No scope creep.
- Feasibility: Sound approach. Importing schema files via ts-node/require() is a proven pattern.
- Architecture fit: Follows existing TS package conventions (package.json, tsup, Jest). New standalone package with no cross-package coupling.
- Non-duplication: No existing schema DSL in the codebase. This fills a gap.
- Cognitive load: Simple builder pattern + codegen string generation. Straightforward for any TS developer.
- Strategic fit: Aligned with project goals. PROJECT.md states "TypeScript-first schema strategy (developers define Zod schemas, build step generates Rust)."
- Project compliance: Honors PROJECT.md decisions. No Rust modifications. No new runtime deps on other TopGun packages.
- Language profile: N/A (TypeScript package, profile applies only to core-rust/server-rust).

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (DSL expressivity) has artifacts | OK | dsl.ts covers all variants |
| Truth 2 (CLI codegen) has artifacts | OK | codegen.ts + bin/commands/codegen.js |
| Truth 3 (JSON intermediate) has artifacts | OK | codegen.ts JSON output |
| Truth 4 (TS types) has artifacts | OK | codegen.ts TS output |
| Truth 5 (Rust loading) has artifacts | OK | JSON asset file |
| Truth 6 (round-trip fidelity) has artifacts | OK | codegen.test.ts validates shape |
| DSL -> codegen wiring defined | OK | SchemaRegistry.global bridges them |
| codegen -> CLI wiring defined | OK | bin/commands/codegen.js calls runCodegen |

**Assumptions Verified Against Source:**

| # | Assumption | Verified |
|---|------------|----------|
| A1 | FieldType uses PascalCase (no rename_all) | Confirmed: schema.rs line 13 has no rename_all |
| A2 | FieldConstraint uses camelCase | Confirmed: schema.rs line 46 has rename_all = "camelCase" |
| A3 | FieldDef uses snake_case (no rename_all) | Confirmed: schema.rs line 77 has no rename_all |
| A4 | MapSchema uses snake_case (no rename_all) | Confirmed: schema.rs line 100 has no rename_all |
| A5 | MapSchema.version is u32 | Confirmed: schema.rs line 103 |
| A6 | FieldConstraint.min_value/max_value are i64 | Confirmed: schema.rs lines 53-58 |

**Recommendations:**

1. The `glob` dependency in `package.json` appears unused -- no codegen step mentions file discovery via glob patterns. The codegen takes a single `--schema` path. Consider removing it to keep the package dependency-free, or clarify what it is used for.

2. The `FieldTypeBuilder` naming is misleading -- it is described as "factory functions" but used as a type parameter in `field(name, type: FieldTypeBuilder, ...)`. In practice, `t.string()` returns a `JsonFieldType` value, not a builder. Consider naming the type `FieldTypeValue` or simply using `JsonFieldType` directly to reduce confusion.

3. The interface naming convention for generated types (e.g., `UserRecord` from map name `users`) is not fully specified. The spec says "One interface per map (e.g., `export interface UserRecord { ... }`)" but does not define the transformation rule (singular + PascalCase + "Record" suffix?). Specify the exact naming algorithm (e.g., `capitalize(singularize(mapName)) + "Record"`) or use a simpler rule like `PascalCase(mapName) + "Schema"` to avoid ambiguity with irregular plurals.

4. The `FieldConstraint` types use `number` in TS for fields that are `i64` in Rust (`min_value`, `max_value`). Values exceeding `Number.MAX_SAFE_INTEGER` (2^53 - 1) would silently lose precision. This is unlikely for schema constraints but worth documenting as a known limitation.

5. Validation checklist item 3 mentions `UserRecord.age` is `number | undefined` but standard TS optional properties use `age?: number` syntax (which is `number | undefined` at the type level). The spec's acceptance criteria use `?` notation (correct). The validation checklist wording could be more precise to avoid confusion about whether the generated code uses `?` or explicit `| undefined`.

**Comment:** Well-structured specification with thorough serde shape analysis. All Rust serde assumptions verified correct against source. The DSL API is ergonomic and the codegen approach is sound. Ready for implementation after optionally addressing recommendations.

### Response v1 (2026-03-19)
**Applied:** All 5 recommendations from Audit v1

**Changes:**
1. [✓] Remove unused `glob` dependency — removed `"glob": "^11.0.0"` from `dependencies` in `package.json` section; updated Constraints to explain the package is dependency-free because codegen takes a single `--schema` path
2. [✓] Rename `FieldTypeBuilder` to `JsonFieldType` as parameter type — updated `MapBuilder.field()` signature to `field(name: string, type: JsonFieldType, options?: FieldOptions): this`; updated `t` namespace description to remove the misleading `FieldTypeBuilder` label and show return types explicitly; updated `t.array` parameter type to `JsonFieldType`
3. [✓] Define exact naming transformation algorithm — codegen step 4 now specifies: strip trailing `s` (if present) to singularize, apply PascalCase, append `Record`; irregular plurals not ending in `s` use map name as-is in PascalCase; examples given for `users` → `UserRecord`, `posts` → `PostRecord`, `metadata` → `MetadataRecord`, `people` → `PeopleRecord`
4. [✓] Document `Number.MAX_SAFE_INTEGER` limitation — added a note in `types.ts` section under `JsonFieldConstraint` explaining that `minValue`/`maxValue` are JS `number` corresponding to Rust `i64`, with precision loss beyond `Number.MAX_SAFE_INTEGER` (2^53 - 1) noted as a known limitation
5. [✓] Align validation checklist item 3 to use `?` notation — changed `number | undefined` to `age?: number` and `string[] | undefined` to `tags?: string[]` to match the generated code style and acceptance criteria notation

### Audit v2 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Dimension Assessment:**

- Clarity: All v1 recommendations addressed. Naming algorithm is now explicit. Type parameter naming is consistent. No vague terms remain.
- Completeness: All files listed. Serde shape assumptions remain verified against source. Naming transformation algorithm specified with examples.
- Testability: 14 acceptance criteria are concrete and measurable. No changes needed.
- Scope: Clear boundaries. 7 constraints are well-defined. No scope creep.
- Feasibility: Sound. `ts-node` confirmed as root devDependency (package.json line 72). `require()` pattern for schema loading is proven.
- Architecture fit: Follows existing monorepo conventions. `pnpm-workspace.yaml` uses `packages/*` glob, so `packages/schema/` will be auto-discovered. Package structure mirrors `@topgunbuild/core` (tsup, Jest, same export shape).
- Non-duplication: No existing schema DSL in codebase. Fills the gap between Rust `MapSchema` types (SPEC-127) and developer experience.
- Cognitive load: Simple builder pattern. `t.*()` factories return literal values. Codegen is string concatenation. No complex abstractions.
- Strategic fit: Aligned with PROJECT.md "TypeScript-first schema strategy." Delivers the "build step generates Rust" half of the stated strategy.
- Project compliance: No Rust modifications. No new runtime deps. Language profile N/A (TypeScript package). Monorepo version 0.12.0 confirmed matching (root package.json line 3).

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Developers want a custom DSL rather than raw JSON | DSL is wasted effort; but raw JSON is error-prone and lacks autocompletion, so DSL is justified |
| A2 | ts-node require() works for loading .ts schema files | Codegen fails; but ts-node is already a project devDep and this pattern is used widely |
| A3 | Serde shapes are stable (SPEC-127 won't add rename_all) | JSON output breaks; but schema.rs is freshly written and verified |

Strategic fit: Aligned with project goals. No red flags detected.

**Project Compliance Check:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| TypeScript-first schema strategy | Spec implements this directly | Compliant |
| MsgPack wire protocol | Spec generates JSON for serde, not wire format | Compliant |
| No new runtime dependencies | Package is dependency-free | Compliant |
| Commit format | Not applicable (spec, not code) | Compliant |
| No phase/spec refs in code | Not applicable (new code) | Compliant |

Project compliance: Honors PROJECT.md decisions.

**Recommendations:**

1. [Minor] The `package.json` for `@topgunbuild/schema` is missing fields present in other packages: `files`, `sideEffects`, `publishConfig`, `license`, `repository`, `engines`. Compare with `@topgunbuild/core` package.json. The implementer should add these for consistency, especially `"license": "Apache-2.0"` and `"files": ["dist"]` to avoid publishing source code.

2. [Minor] The TS type mapping table (line 213) shows `Array(T)` mapping to `MappedType<T>[]` but the prose description (line 199) says `Array(T)` -> `T[]`. The table appears to be a placeholder notation. Since the generated types file is plain TypeScript interfaces (not generic type aliases), the prose description (`string[]`, `number[][]`, etc.) is the correct behavior. The table entry should say `<mapped>[]` or simply match the prose to avoid implementer confusion.

**Comment:** All v1 recommendations have been cleanly applied. The spec is well-structured, complete, and ready for implementation. The two minor recommendations above are cosmetic and do not block implementation.

### Response v2 (2026-03-19)
**Applied:** All 2 recommendations from Audit v2

**Changes:**
1. [✓] Add missing `package.json` fields for consistency with `@topgunbuild/core` — added `"files": ["dist"]`, `"sideEffects": false`, `"publishConfig": { "access": "public" }`, `"license": "Apache-2.0"`, `"repository": { "type": "git", "url": "...", "directory": "packages/schema" }`, and `"engines": { "node": ">=18" }` to the `packages/schema/package.json` block
2. [✓] Fix `Array(T)` row in TS type mapping table — changed `MappedType<T>[]` to `T[]` with a clarifying note `(concrete mapped type, e.g., string[], number[][])` in both the table and the prose description to eliminate the generic placeholder notation that contradicted the concrete behavior described elsewhere

### Audit v3 (2026-03-19)
**Status:** APPROVED

**Context Estimate:** ~45% total

**Dimension Assessment:**

- Clarity: Excellent. All API signatures are explicit with types and defaults. Naming algorithm is specified with examples covering regular and irregular plurals. No vague terms.
- Completeness: All 10 files listed (4 source, 2 test, 1 CLI command, 1 CLI modification, plus tsconfig/tsup scaffolding). Serde shape assumptions independently verified against schema.rs source. The `FieldType::Array(Box<FieldType>)` serializes as `{"Array": <inner>}` which matches the spec's `{ Array: JsonFieldType }` type.
- Testability: 14 acceptance criteria are concrete and verifiable. Each maps to specific observable behavior.
- Scope: Well-bounded by 7 explicit constraints. No scope creep into Rust code, client packages, or runtime validation.
- Feasibility: Sound. `bin/topgun.js` uses Commander.js with `require()` for command handlers -- the proposed wiring matches exactly. `pnpm-workspace.yaml` uses `packages/*` glob so `packages/schema/` auto-discovers. `ts-node` is confirmed as root devDependency.
- Architecture fit: Follows established monorepo conventions. Package structure mirrors existing TS packages (tsup build, Jest test, same export shape).
- Non-duplication: No existing schema DSL in the codebase. Fills the gap identified in the TODO chain (069 -> 127 -> 128 -> 129).
- Cognitive load: Simple builder pattern returning literal values. Codegen is string concatenation. No complex abstractions or state management.
- Strategic fit: Directly implements PROJECT.md's "TypeScript-first schema strategy (developers define Zod schemas, build step generates Rust)." The DSL replaces Zod for schema definition (justified: Zod schemas would need a second translation layer to match Rust serde shapes, while this DSL produces the exact JSON format directly).
- Project compliance: No Rust modifications. No new runtime deps. No out-of-scope features. Version 0.12.0 matches monorepo.

**Assumptions Verified Against Source:**

All 6 assumptions from Audit v1 re-verified against current `packages/core-rust/src/schema.rs`:

| # | Assumption | Verified |
|---|------------|----------|
| A1 | FieldType uses PascalCase (no rename_all) | Confirmed: line 13-14 `#[derive(..., Serialize, Deserialize)] pub enum FieldType` with no rename_all |
| A2 | FieldConstraint uses camelCase | Confirmed: line 46 `#[serde(rename_all = "camelCase")]` |
| A3 | FieldDef uses snake_case (no rename_all) | Confirmed: line 77 `#[derive(...)] pub struct FieldDef` with no rename_all |
| A4 | MapSchema uses snake_case (no rename_all) | Confirmed: line 100 `pub struct MapSchema` with no rename_all |
| A5 | MapSchema.version is u32 | Confirmed: line 103 `pub version: u32` |
| A6 | FieldConstraint.min_value/max_value are i64 | Confirmed: lines 56, 58 `pub min_value: Option<i64>`, `pub max_value: Option<i64>` |
| A7 | FieldConstraint.min_length/max_length are u32 | Confirmed: lines 50, 53 `pub min_length: Option<u32>`, `pub max_length: Option<u32>` |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Custom DSL is preferred over raw JSON construction | Low risk -- DSL provides autocompletion and type safety that raw JSON lacks |
| A2 | ts-node require() works for .ts schema files | Low risk -- proven pattern, ts-node is existing devDependency |
| A3 | Serde shapes will remain stable | Low risk -- schema.rs was written in SPEC-127 for this exact purpose |
| A4 | SchemaRegistry.global singleton pattern works across require() | Low risk -- Node.js module cache ensures single instance |

Strategic fit: Aligned with project goals. No red flags.

**Project Compliance Check:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| TypeScript-first schema strategy | Implements directly | Compliant |
| MsgPack wire protocol | JSON for serde only, not wire | Compliant |
| No new runtime dependencies | Package is dependency-free | Compliant |
| Apache-2.0 license | package.json includes license field | Compliant |
| No phase/spec refs in code | New code, convention noted | Compliant |

Project compliance: Honors PROJECT.md decisions.

**Goal-Backward Validation:**

| Check | Status |
|-------|--------|
| Truth 1 (DSL expressivity) -> dsl.ts | OK |
| Truth 2 (CLI codegen) -> codegen.ts + bin/commands/codegen.js | OK |
| Truth 3 (JSON intermediate) -> codegen.ts JSON output | OK |
| Truth 4 (TS types) -> codegen.ts TS output | OK |
| Truth 5 (Rust loading) -> JSON asset file | OK |
| Truth 6 (round-trip fidelity) -> codegen.test.ts | OK |
| All artifacts have purpose | OK |
| All wiring defined | OK |

**Comment:** Spec has been through two revision cycles and all 7 prior recommendations have been cleanly addressed. Independent verification of all Rust serde shape assumptions confirms correctness. The spec is thorough, well-structured, and ready for implementation. No critical issues and no further recommendations.

## Execution Summary

**Executed:** 2026-03-19
**Mode:** orchestrated
**Commits:** 3 (1ac114c, 378cb10, 738e8e2)

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2, G3 | complete |
| 3 | G4 | complete |

### Files Created

- `packages/schema/package.json`
- `packages/schema/tsconfig.json`
- `packages/schema/tsup.config.ts`
- `packages/schema/jest.config.js`
- `packages/schema/src/types.ts`
- `packages/schema/src/dsl.ts`
- `packages/schema/src/index.ts`
- `packages/schema/src/codegen.ts`
- `packages/schema/src/__tests__/dsl.test.ts`
- `packages/schema/src/__tests__/codegen.test.ts`
- `bin/commands/codegen.js`

### Files Modified

- `bin/topgun.js` — added `topgun codegen` command

### Acceptance Criteria Status

- [x] `@topgunbuild/schema` package exists with `package.json`, builds via `pnpm build`, and exports `defineMap`, `t`, `schema`
- [x] All 9 `FieldType` variants expressible via `t.*()` functions
- [x] All 6 `FieldConstraint` options expressible via `FieldOptions`
- [x] `MapBuilder.build()` returns `JsonMapSchema` with correct Rust serde shapes
- [x] `topgun codegen` CLI command reads a `topgun.schema.ts` file and generates output
- [x] Generated `topgun.schema.json` uses `field_type` (snake_case) and `FieldConstraint` fields (camelCase)
- [x] Generated `topgun.types.ts` contains TypeScript interfaces with correct types and optionality
- [x] `required: true` fields produce non-optional TS properties; `required: false` produce optional `?` properties
- [x] `FieldType::Binary` maps to `Uint8Array` in generated TS types
- [x] `FieldType::Array(String)` maps to `string[]` in generated TS types
- [x] Nested arrays `t.array(t.array(t.int()))` produce `number[][]` in generated TS types
- [x] `topgun codegen --schema ./path/to/schema.ts --out-dir ./output` works with custom paths
- [x] Unit tests pass for DSL builder (37 tests)
- [x] Unit tests pass for codegen (22 tests)

### Deviations

- `loadSchemaFile()` does not auto-reset the global registry before loading — this avoids interfering with tests that pre-populate the registry. The registry is user-managed; calling `runCodegen` with a schema file that calls `schema.register(...)` accumulates correctly.

---

## Review History

### Review v1 (2026-03-19 19:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `@topgunbuild/schema` package exists at `packages/schema/package.json` with all required fields (`files`, `sideEffects`, `publishConfig`, `license`, `repository`, `engines`), version 0.12.0 matches monorepo
- [✓] AC2: All 9 `FieldType` variants implemented in `t.*()` — `string`, `int`, `float`, `bool`, `binary`, `timestamp`, `array`, `map`, `any` — verified in `dsl.ts` lines 92-110
- [✓] AC3: All 6 `FieldConstraint` options present in `FieldOptions` — `minLength`, `maxLength`, `minValue`, `maxValue`, `pattern`, `enumValues` — verified in `dsl.ts` lines 7-15
- [✓] AC4: `MapBuilder.build()` returns `JsonMapSchema` with correct Rust serde shapes (`field_type` snake_case, constraint fields camelCase) — verified by codegen tests
- [✓] AC5: `topgun codegen` command wired in `bin/topgun.js` lines 66-72; `bin/commands/codegen.js` correctly registers ts-node and calls `runCodegen()`
- [✓] AC6: JSON output uses `field_type` (snake_case) and camelCase constraint fields — validated by `codegen.test.ts` tests "JSON field names use snake_case" and "FieldConstraint fields use camelCase"
- [✓] AC7-8: TS output generates interfaces with correct optionality — `required: true` produces non-optional, default produces `?` — validated by codegen tests
- [✓] AC9: `Binary` maps to `Uint8Array` — `mapFieldTypeToTs` in `codegen.ts` line 26, confirmed by test
- [✓] AC10: `Array(String)` maps to `string[]` — recursive `mapFieldTypeToTs` confirmed by test
- [✓] AC11: Nested arrays produce `number[][]` — confirmed by test "Nested array Array(Array(Int)) maps to number[][]"
- [✓] AC12: Custom `--schema` and `--out-dir` flags correctly passed through CLI options object in `codegen.js` lines 24-27
- [✓] AC13: 37 DSL builder tests pass
- [✓] AC14: 22 codegen tests pass (59 total across both suites)
- [✓] No Rust files modified — constraint honored
- [✓] No dependencies added beyond `typescript` devDependency — package is dependency-free as required
- [✓] `@topgunbuild/core` and `@topgunbuild/client` not modified — constraint honored
- [✓] `pnpm-workspace.yaml` uses `packages/*` glob — new package auto-discovered
- [✓] Package structure mirrors existing TS packages (tsup, Jest, same export shape) — follows established conventions
- [✓] `index.ts` re-exports all public API: `defineMap`, `t`, `schema`, `MapBuilder`, `FieldOptions`, codegen types
- [✓] `SchemaRegistry.getSchemas()` returns a shallow copy — mutations to returned object do not affect registry (verified by test "getSchemas returns a copy")
- [✓] No spec/phase references in new code comments — WHY-comments used throughout
- [✓] `mapNameToInterfaceName` naming algorithm correctly implements the spec: strip trailing `s`, capitalize first char, append `Record` — matches all four spec examples (`users`→`UserRecord`, `posts`→`PostRecord`, `metadata`→`MetadataRecord`, `people`→`PeopleRecord`)
- [✓] `codegen.test.ts` resets global registry using private field access via type casting (`resetRegistry()`) — pragmatic approach that avoids leaking state between tests
- [✓] Deviation documented: `loadSchemaFile()` does not auto-reset registry before loading — behavior is correct for the CLI use-case where a schema file is loaded once

**Summary:** All 14 acceptance criteria are met. The implementation is clean, minimal, and directly follows the specification. 59 unit tests pass. No critical or major issues found. The package integrates naturally into the monorepo and all architectural constraints are honored.

---

## Completion

**Completed:** 2026-03-19
**Total Commits:** 3
**Review Cycles:** 1

### Outcome

Delivered `@topgunbuild/schema` package with a fluent TypeScript DSL for defining schemas (`defineMap`, `t.*` type factories, `SchemaRegistry`) and a codegen toolchain that produces JSON intermediate format (matching Rust `MapSchema` serde shape exactly) and typed TypeScript interfaces. Wired `topgun codegen` CLI command.

### Key Files

- `packages/schema/src/dsl.ts` — fluent builder DSL with all 9 FieldType variants and 6 FieldConstraint options
- `packages/schema/src/codegen.ts` — generates JSON (Rust-compatible) and TypeScript type files from schema definitions
- `packages/schema/src/types.ts` — internal types mirroring Rust serde shapes (field_type snake_case, constraints camelCase)
- `bin/commands/codegen.js` — CLI command handler for `topgun codegen`

### Patterns Established

- TypeScript-first schema DSL pattern: developers define schemas in `topgun.schema.ts` using builder API, codegen produces artifacts for both Rust server and TS client consumption
- `SchemaRegistry.global` singleton pattern for accumulating schema definitions across a schema file

### Deviations

- `loadSchemaFile()` does not auto-reset the global registry before loading — avoids interfering with tests that pre-populate the registry. Registry is user-managed.
