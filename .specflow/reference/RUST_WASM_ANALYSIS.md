# Rust + WASM Integration Strategy for TopGun

> **Version:** 2.0 (Consolidated)
> **Updated:** 2026-01-12
> **Status:** Roadmap - Research Complete
> **Dependencies:** None (can be executed independently)
> **Priority:** Medium-Low (performance optimization, not feature)

---

## Executive Summary

| Question | Answer |
|----------|--------|
| **Should we use Rust+WASM?** | Yes, with phased approach |
| **Development complexity?** | Medium-High initially, pays off long-term |
| **Where in monorepo?** | `packages/core-rust/` + `packages/core-wasm/` |
| **Best candidates?** | MerkleTree, CRDT Merge, DAG Executor, SQL Parser |
| **When to start?** | After validating with PoC (SQL Parser) |
| **Estimated effort?** | 4-6 weeks total |
| **Expected speedup?** | 2-3x on CPU-intensive operations |

**Verdict:** Recommended for CPU-bound hot paths. Start with SQL Parser PoC to validate pipeline.

**Key Finding:** Architecture audit confirms zero blockers for WASM integration.

---

## Table of Contents

1. [Background & Motivation](#1-background--motivation)
2. [Current Architecture Analysis](#2-current-architecture-analysis)
3. [Complexity Analysis](#3-complexity-analysis)
4. [Monorepo Structure](#4-monorepo-structure)
5. [Hot Paths & Migration Candidates](#5-hot-paths--migration-candidates)
6. [Implementation Phases](#6-implementation-phases)
7. [Practical Guidelines](#7-practical-guidelines)
8. [Runtime-Specific Loading Strategy](#8-runtime-specific-loading-strategy)
9. [Serverless & Edge Compatibility](#9-serverless--edge-compatibility)
10. [Backward Compatibility](#10-backward-compatibility)
11. [Performance Targets & Success Criteria](#11-performance-targets--success-criteria)
12. [Risk Assessment](#12-risk-assessment)
13. [Decision Criteria](#13-decision-criteria)
14. [Conclusion](#14-conclusion)

---

## 1. Background & Motivation

### 1.1 Why Consider Rust/WASM?

| Problem | Node.js Limitation | Rust/WASM Solution |
|---------|-------------------|-------------------|
| GC pauses | V8 GC on large heaps (10+ GB) causes p99 latency spikes | No GC, predictable performance |
| CPU-bound ops | Single-threaded event loop blocks during merge | Native parallelism, SIMD |
| Offline recovery | Mass merge after long offline is slow | 10-100x speedup (Automerge experience) |
| Serialization | msgpackr overhead | Native binary handling |

### 1.2 What Similar Projects Do

| Project | Stack | Approach |
|---------|-------|----------|
| Gun.js | Node.js | Works, but known scaling issues |
| Yjs | Node.js | Successful, document-focused |
| Automerge | Rust + WASM | Started JS, migrated core to Rust - **10-100x speedup** |
| Liveblocks | Node.js + Rust | Hybrid approach |

---

## 2. Current Architecture Analysis

### 2.1 Why TopGun is Ready

**1. Worker interfaces are DTO-based (not class-based)**

```typescript
// packages/server/src/workers/worker-scripts/crdt.worker.ts
interface LWWMergePayload {
  mapName: string;
  records: Array<{
    key: string;
    value: unknown;
    timestamp: { millis: number; counter: number; nodeId: string };
    ttlMs?: number;
  }>;
  existingState: Array<{ key: string; timestamp; value?; ttlMs? }>;
}
```

Workers receive plain objects, not JS class instances. This is **ideal for WASM** - no custom serialization needed.

**2. Worker pool abstraction exists**

`CRDTMergeWorker` decides whether to process inline (small batches) or in worker thread (large batches). Business logic in `ServerCoordinator` is decoupled from implementation.

**3. Serialization uses msgpackr**

Binary format supported by Rust (`rmp-serde` crate). Data already flows in WASM-friendly format.

**4. Native bindings pattern exists**

`@topgunbuild/native` package provides optional xxHash64 binding with JS fallback. Same pattern can be extended for WASM.

### 2.2 Data Flow Verification

```
Client -> WRITE_SET -> ServerCoordinator
    |
Creates merge payload (plain objects)
    |
CRDTMergeWorker.mergeLWW()
    +- if batch < 10: mergeLWWInline() (main thread)
    +- if batch >= 10: WorkerPool.submit() (worker thread)
    |
Worker computes toApply[] (what changed)
    |
Main thread applies via map.merge()
```

**Verdict:** Clean separation. Worker can be replaced with WASM without affecting data flow.

---

## 3. Complexity Analysis

### 3.1 Development Complexity

| Aspect | Complexity | Notes |
|--------|------------|-------|
| **Rust learning curve** | High | Ownership/borrowing requires 2-4 weeks for productivity |
| **WASM boundary** | Medium | Each JS<->WASM call has ~100-500ns overhead |
| **Debugging** | Medium-High | Source maps work inconsistently, often debug in native Rust |
| **Tooling** | Low | Cargo + wasm-pack + clippy - excellent ecosystem |
| **TypeScript integration** | Low | wasm-pack generates TS types automatically |

### 3.2 Maintenance Complexity

**Pros:**
- Rust code is more stable (fewer runtime errors)
- Strict type system catches bugs at compile time
- Excellent documentation via `cargo doc`
- Memory safety guarantees (no data races in CRDT algorithms)

**Cons:**
- Need developers with Rust knowledge (or training time)
- Two toolchains in CI (Node.js + Rust)
- Versioning between TS and Rust packages
- Larger bundle size (+200KB-2MB for WASM module)

### 3.3 Build & Integration

```bash
# Typical workflow
cd packages/core-rust
wasm-pack build --target web --release

# Output in pkg/
+-- core_rust_bg.wasm      # ~200KB-2MB depending on code
+-- core_rust_bg.wasm.d.ts
+-- core_rust.js           # JS glue code
+-- core_rust.d.ts         # TypeScript types (auto-generated!)
```

**Integration:** Nearly seamless with modern bundlers (Vite, Webpack). WASM modules are imported as async dependencies.

---

## 4. Monorepo Structure

### 4.1 Recommended Layout

```
topgun/
+-- packages/
|   +-- core/              # Existing TS core
|   +-- client/            # Existing TS client
|   +-- server/            # Existing TS server
|   +-- native/            # Already exists! (N-API for xxhash)
|   |
|   +-- core-rust/         # NEW: Rust crate
|   |   +-- Cargo.toml
|   |   +-- src/
|   |   |   +-- lib.rs
|   |   |   +-- merkle.rs
|   |   |   +-- crdt.rs
|   |   |   +-- dag.rs
|   |   |   +-- sql_parser.rs
|   |   |   +-- timestamp.rs
|   |   |   +-- hash.rs
|   |   +-- pkg/           # wasm-pack output
|   |
|   +-- core-wasm/         # NEW: TS wrapper with fallback
|       +-- package.json
|       +-- src/
|       |   +-- index.ts   # Re-exports + fallback logic
|       |   +-- fallback/  # Pure JS fallback implementations
|       +-- wasm/          # Symlink to core-rust/pkg
|
+-- Cargo.toml             # Workspace root
+-- rust-toolchain.toml    # Pin Rust version
```

### 4.2 Why This Structure?

1. **TopGun already has the pattern** - `@topgunbuild/native` demonstrates native addon + JS fallback
2. **Clear separation** - Rust code isolated, TS wrapper provides clean API
3. **Fallback support** - Environments without WASM still work
4. **Single source of truth** - Types generated from Rust, no manual sync

### 4.3 Package Dependencies

```
@topgunbuild/core-wasm
+-- depends on: core-rust/pkg (WASM binary)
+-- provides: JS API + fallback

@topgunbuild/core
+-- optionally depends on: @topgunbuild/core-wasm
+-- falls back to: pure JS implementations

@topgunbuild/server
+-- uses: @topgunbuild/core-wasm (if available)

@topgunbuild/client
+-- uses: @topgunbuild/core-wasm (browser WASM)
```

### 4.4 Migration Path

```
Current State
+-- Core CRDT logic: packages/core/src/LWWMap.ts (JavaScript)
+-- Worker merge: packages/server/src/workers/crdt.worker.ts (JavaScript)
+-- Native hash: @topgunbuild/native (optional, Node.js only)

Phase 1
+-- Core CRDT logic: packages/core/src/LWWMap.ts (JavaScript - unchanged)
+-- Worker merge: packages/server/src/workers/crdt.worker.ts -> calls WASM
+-- WASM core: @topgunbuild/wasm (Rust)

Phase 2+ (Optional)
+-- Core CRDT logic: packages/core/src/LWWMap.ts -> thin wrapper over WASM
+-- Worker merge: Uses same WASM
+-- WASM core: @topgunbuild/wasm (Rust)
```

**Key insight:** Phase 1 provides benefits without changing core package APIs.

---

## 5. Hot Paths & Migration Candidates

### 5.1 Priority Matrix

| Priority | Component | Location | Speedup | Effort |
|----------|-----------|----------|---------|--------|
| 1 | **Merkle Tree Hash/Diff** | `MerkleTree.ts` + `merkle.worker.ts` | 50-60% | Medium |
| 2 | **CRDT Batch Merge** | `CRDTMergeWorker.ts` + `crdt.worker.ts` | 30-40% | Medium |
| 3 | **DAG Executor** | Not implemented | 2-5x | High |
| 4 | **SQL Parser** | Not implemented | N/A (new feature) | Medium |
| 5 | **Serialization** | `SerializationWorker.ts` | 50-70% | Medium |
| 6 | **Hash Function** | `utils/hash.ts` (FNV-1a) | 20-30% | Low |
| 7 | **Timestamp Compare** | Inline in both files | 30-50% | Low |

### 5.2 Detailed Analysis

#### MerkleTree (Highest Impact)

**Current state:** `MerkleWorker.ts` in worker threads

**Problem:** With large datasets (100K+ records), Merkle diff takes 100-500ms

**Operations:** `merkle-hash`, `merkle-diff`, `merkle-rebuild`

**Rust solution:**
```rust
// packages/core-rust/src/merkle.rs
use wasm_bindgen::prelude::*;
use sha2::{Sha256, Digest};

#[wasm_bindgen]
pub struct MerkleTree {
    buckets: Vec<[u8; 32]>,
    depth: usize,
}

#[wasm_bindgen]
impl MerkleTree {
    #[wasm_bindgen(constructor)]
    pub fn new(depth: usize) -> MerkleTree {
        let bucket_count = 1 << depth;
        MerkleTree {
            buckets: vec![[0u8; 32]; bucket_count],
            depth,
        }
    }

    pub fn update(&mut self, key: &[u8], value: &[u8]) {
        let bucket_idx = self.get_bucket_index(key);
        let mut hasher = Sha256::new();
        hasher.update(&self.buckets[bucket_idx]);
        hasher.update(key);
        hasher.update(value);
        self.buckets[bucket_idx] = hasher.finalize().into();
    }

    pub fn diff(&self, other: &MerkleTree) -> Vec<u32> {
        self.buckets.iter()
            .zip(other.buckets.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i as u32)
            .collect()
    }

    fn get_bucket_index(&self, key: &[u8]) -> usize {
        let hash = xxhash_rust::xxh64::xxh64(key, 0);
        (hash as usize) % self.buckets.len()
    }
}
```

**Expected result:** 50-60% speedup, predictable latency (no GC pauses)

#### CRDT Batch Merge (High Priority)

**Current state:** `CRDTMergeWorker.ts` processes merges in worker

**Threshold:** Uses worker thread if `batchSize >= 10`

**Problem:** GC pauses during large batch merges

**Rust solution:**
```rust
// packages/core-rust/src/crdt.rs
use wasm_bindgen::prelude::*;
use rmp_serde::{decode, encode};

#[wasm_bindgen]
pub fn merge_lww_batch(
    local_msgpack: &[u8],
    remote_msgpack: &[u8],
) -> Vec<u8> {
    let local: HashMap<String, LWWRecord> = decode::from_slice(local_msgpack).unwrap();
    let remote: HashMap<String, LWWRecord> = decode::from_slice(remote_msgpack).unwrap();

    let mut result = local;
    for (key, remote_record) in remote {
        match result.get(&key) {
            Some(local_record) if local_record.timestamp >= remote_record.timestamp => {
                // Keep local
            }
            _ => {
                result.insert(key, remote_record);
            }
        }
    }

    encode::to_vec(&result).unwrap()
}

#[derive(Serialize, Deserialize)]
struct LWWRecord {
    value: serde_json::Value,
    timestamp: u64,
    node_id: String,
}
```

#### DAG Executor (High Priority - if implementing)

Ideal Rust candidate:
- CPU-bound batch processing
- Predictable latency is critical
- Can work in browser (client-side queries)

```rust
// packages/core-rust/src/dag.rs
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct DAG {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
}

#[wasm_bindgen]
impl DAG {
    pub fn execute(&self, input: &[u8]) -> Vec<u8> {
        // Process through DAG vertices
        // Return results as msgpack
    }
}
```

#### SQL Parser (Good PoC - Medium Priority)

Using `sqlparser-rs` - excellent entry point for validation:

```rust
// packages/core-rust/src/sql_parser.rs
use wasm_bindgen::prelude::*;
use sqlparser::parser::Parser;
use sqlparser::dialect::GenericDialect;

#[wasm_bindgen]
pub fn parse_sql(sql: &str) -> JsValue {
    let dialect = GenericDialect {};
    match Parser::parse_sql(&dialect, sql) {
        Ok(ast) => serde_wasm_bindgen::to_value(&ast).unwrap(),
        Err(e) => {
            let error = format!("Parse error: {}", e);
            serde_wasm_bindgen::to_value(&error).unwrap()
        }
    }
}

#[wasm_bindgen]
pub fn sql_to_query_dsl(sql: &str) -> JsValue {
    // Convert SQL AST to TopGun Query DSL
    // Returns JSON that matches existing Query type
}
```

### 5.3 What NOT to Migrate

| Component | Why Keep in TypeScript |
|-----------|------------------------|
| Cluster Management | I/O bound, complex async logic |
| Networking | Node.js net/dgram excellent, WASM can't do sockets |
| WebSocket handling | I/O bound |
| Query Registry | Mostly Map operations, JS is fine |
| Configuration | Simple object manipulation |

---

## 6. Implementation Phases

### Phase 0: Proof of Concept (1-2 weeks)

```
Goal: Validate pipeline without risk
Component: SQL Parser (sqlparser-rs)
Result: client.sql("SELECT * FROM users") works
```

**Why SQL Parser:**
- Isolated component (doesn't touch existing code)
- Easy to measure success
- `sqlparser-rs` is ready to use
- Provides new feature (SQL support)

**Deliverables:**
- [ ] `packages/core-rust/` scaffolding
- [ ] SQL Parser implementation
- [ ] `packages/core-wasm/` wrapper
- [ ] Integration test
- [ ] Bundle size measurement

### Phase 1: Create `@topgunbuild/wasm` Package (1-2 weeks)

**Package structure:**

```
packages/wasm/
+-- src/
|   +-- lib.rs           # Rust entry point
|   +-- merkle.rs        # Merkle tree operations
|   +-- crdt.rs          # CRDT merge logic
|   +-- timestamp.rs     # HLC comparison
|   +-- hash.rs          # xxHash64 / FNV-1a
+-- pkg/                  # wasm-pack output
+-- Cargo.toml
+-- package.json
+-- README.md
```

**Build toolchain:**
- `wasm-pack` for Rust -> WASM compilation
- `wasm-bindgen` for JS bindings
- Optional: `wasm-opt` for size optimization

### Phase 2: MerkleTree Migration (2-3 weeks)

```
Goal: Speed up slowest path
Component: MerkleTree diff
Result: 50%+ speedup for delta sync
```

**Replace:**
```typescript
// Current: packages/server/src/workers/worker-scripts/merkle.worker.ts
registerHandler('merkle-hash', (payload) => {
  // JavaScript implementation
});
```

**With:**
```typescript
// New: WASM binding
import { merkle_hash } from '@topgunbuild/wasm';

registerHandler('merkle-hash', (payload) => {
  return merkle_hash(payload);
});
```

**API remains identical** - only implementation changes.

**Deliverables:**
- [ ] MerkleTree Rust implementation
- [ ] Benchmark: before vs after
- [ ] Fallback JS implementation
- [ ] Integration with MerkleWorker

### Phase 3: CRDT Batch Merge (3-4 weeks)

```
Goal: Eliminate GC pauses
Component: LWW/OR merge + batch operations
Result: Predictable p99 latency
```

**Deliverables:**
- [ ] LWWMap merge in Rust
- [ ] ORMap merge in Rust
- [ ] Batch operation support
- [ ] Integration with CRDTMergeWorker

### Phase 4: DAG Executor (4-6 weeks)

```
Goal: Distributed queries
Component: Full DAG runtime in Rust
Result: 2-5x speedup for complex queries
```

**Deliverables:**
- [ ] DAG structure in Rust
- [ ] Processor implementations
- [ ] WASM bindings
- [ ] Integration with cluster

### Phase 5: Optional Serialization Upgrade (1-2 weeks)

**Options:**
1. Replace Base64 with SharedArrayBuffer (if browser support sufficient)
2. Use WASM-based msgpack codec
3. Keep current approach (acceptable overhead)

---

## 7. Practical Guidelines

### 7.1 Minimize Boundary Crossings

```typescript
// BAD: many small calls
for (const item of items) {
  wasmModule.process(item);
}

// GOOD: single batch call
const serialized = msgpack.encode(items);
const result = wasmModule.processBatch(serialized);
```

### 7.2 Use SharedArrayBuffer Where Possible

```typescript
// For large data - zero-copy via SharedArrayBuffer
const sharedBuffer = new SharedArrayBuffer(1024 * 1024);
const view = new Uint8Array(sharedBuffer);
// Fill view with data
wasmModule.processInPlace(sharedBuffer);
```

### 7.3 Fallback for Environments Without WASM

```typescript
// packages/core-wasm/src/index.ts
let wasmModule: WasmModule | null = null;

export async function getMerkleTree(): Promise<typeof MerkleTree> {
  if (wasmModule === null) {
    try {
      wasmModule = await import('./wasm/core_rust');
    } catch {
      // Fallback to JS implementation
      wasmModule = await import('./fallback/merkle');
    }
  }
  return wasmModule.MerkleTree;
}

// Usage
const MerkleTree = await getMerkleTree();
const tree = new MerkleTree(3);
```

### 7.4 CI/CD Configuration

```yaml
# .github/workflows/build.yml
jobs:
  build-wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          target: wasm32-unknown-unknown

      - name: Cache Cargo
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            packages/core-rust/target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: Install wasm-pack
        run: cargo install wasm-pack

      - name: Build WASM
        run: |
          cd packages/core-rust
          wasm-pack build --target web --release

      - name: Upload WASM artifact
        uses: actions/upload-artifact@v3
        with:
          name: wasm-bundle
          path: packages/core-rust/pkg/
```

### 7.5 Cargo.toml Template

```toml
# packages/core-rust/Cargo.toml
[package]
name = "topgun-core-rust"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde-wasm-bindgen = "0.6"
rmp-serde = "1.1"
sha2 = "0.10"
xxhash-rust = { version = "0.8", features = ["xxh64"] }

# Optional: SQL Parser
sqlparser = { version = "0.40", optional = true }

[features]
default = []
sql = ["sqlparser"]

[profile.release]
opt-level = "z"      # Optimize for size
lto = true           # Link-time optimization
codegen-units = 1    # Better optimization
```

---

## 8. Runtime-Specific Loading Strategy

### 8.1 The Key Insight: Different Runtimes, Different Needs

TypeScript code serves as both **fallback** and **primary implementation** depending on environment:

| Runtime | Recommended Implementation | Bundle Size | Reason |
|---------|---------------------------|-------------|--------|
| **Node.js Server** | Rust/WASM | +200KB-2MB | Performance critical, bundle size irrelevant |
| **Browser** | TypeScript | 0 (already bundled) | Bundle size critical, JS fast enough |
| **Edge/Serverless** | Rust/WASM | +200KB-2MB | Cold start matters, WASM initializes faster |

### 8.2 Conditional Loading Implementation

```typescript
// packages/core-wasm/src/index.ts

type Runtime = 'node' | 'browser' | 'edge' | 'unknown';

function detectRuntime(): Runtime {
  if (typeof process !== 'undefined' && process.versions?.node) {
    return 'node';
  }
  if (typeof window !== 'undefined') {
    return 'browser';
  }
  if (typeof globalThis.EdgeRuntime !== 'undefined') {
    return 'edge';  // Vercel Edge, Cloudflare Workers
  }
  return 'unknown';
}

export async function getMerkleTree() {
  const runtime = detectRuntime();

  // Strategy: WASM for server/edge, JS for browser
  if (runtime === 'browser') {
    // Browser: use TypeScript (smaller bundle)
    const { MerkleTree } = await import('@topgunbuild/core/merkle');
    return MerkleTree;
  }

  // Server/Edge: try WASM, fallback to JS
  try {
    const wasm = await import('./wasm/core_rust');
    return wasm.MerkleTree;
  } catch {
    const { MerkleTree } = await import('@topgunbuild/core/merkle');
    return MerkleTree;
  }
}
```

### 8.3 Bundler Configuration for Tree-Shaking

```typescript
// vite.config.ts (browser build)
export default defineConfig({
  define: {
    'process.env.USE_WASM': JSON.stringify(false),
  },
  // WASM will be tree-shaken out
});

// In code:
if (process.env.USE_WASM) {
  // This entire branch is removed in browser build
  return import('./wasm/core_rust');
}
```

### 8.4 User-Configurable Strategy

```typescript
// packages/core-wasm/src/index.ts

export interface LoaderConfig {
  // Override automatic detection
  forceWasm?: boolean;
  forceJs?: boolean;

  // Per-runtime preferences
  browser?: 'wasm' | 'js' | 'auto';
  node?: 'wasm' | 'js' | 'auto';
  edge?: 'wasm' | 'js' | 'auto';
}

const defaultConfig: LoaderConfig = {
  browser: 'js',    // Smaller bundle by default
  node: 'wasm',     // Performance by default
  edge: 'wasm',     // Performance by default
};

export function configure(config: Partial<LoaderConfig>): void {
  Object.assign(defaultConfig, config);
}

// Usage:
import { configure, getMerkleTree } from '@topgunbuild/core-wasm';

// Power user wants WASM even in browser
configure({ browser: 'wasm' });

const MerkleTree = await getMerkleTree();
```

### 8.5 Bundle Size Impact

| Scenario | Browser Bundle | Server Bundle |
|----------|----------------|---------------|
| **No WASM** | ~50KB (core) | ~50KB (core) |
| **WASM everywhere** | ~250KB-2MB (+400%) | ~250KB-2MB |
| **Conditional loading** | ~50KB (TS only) | ~250KB-2MB (WASM) |

**Recommendation:** Conditional loading is the optimal approach. Browser gets small bundle, server gets performance.

---

## 9. Serverless & Edge Compatibility

### 9.1 Short Answer: WASM Works Almost Everywhere

WASM **expands** deployment options rather than limiting them.

### 9.2 Compatibility Matrix

| Environment | WASM Support | Notes |
|-------------|--------------|-------|
| **Node.js** | Full | Native support since v12+ |
| **Deno** | Full | Native support |
| **Bun** | Full | Native support |
| **Browser** | Full | All modern browsers (95%+ market share) |
| **Cloudflare Workers** | Full | V8 isolates, excellent support |
| **Vercel Edge** | Full | V8-based |
| **AWS Lambda** | Full | Node.js runtime supports WASM |
| **AWS Lambda@Edge** | Full | Node.js runtime |
| **Google Cloud Functions** | Full | Node.js runtime |
| **Azure Functions** | Full | Node.js runtime |
| **Fastly Compute@Edge** | Full | Native WASM runtime |
| **Netlify Edge** | Full | Deno-based |
| **React Native** | Partial | Via JSI, requires setup |
| **Expo** | Partial | Depends on configuration |

### 9.3 WASM Limitations (Not Runtime-Related)

| Limitation | Description | Workaround |
|------------|-------------|------------|
| **No sockets** | WASM cannot open network connections | Network code stays in JS |
| **No filesystem** | WASM has no FS access | Data passed through JS |
| **No threads** | WASM threads limited (SharedArrayBuffer) | Use Worker threads in JS |
| **Sync calls** | JS<->WASM calls are synchronous | Batch operations |

### 9.4 Platform-Specific Examples

#### Cloudflare Workers (Excellent Support!)

```typescript
// Cloudflare Workers natively support WASM
import wasmModule from './core_rust_bg.wasm';

export default {
  async fetch(request: Request): Promise<Response> {
    const { MerkleTree } = await import('./wasm/core_rust');
    const tree = new MerkleTree(3);
    // ... use WASM module
  }
};
```

**WASM advantages in Cloudflare Workers:**
- Fast cold start (~5ms vs ~50ms for JS)
- Predictable performance
- Lower memory consumption

#### AWS Lambda

```typescript
// AWS Lambda handler
import { getMerkleTree } from '@topgunbuild/core-wasm';

export const handler = async (event: any) => {
  const MerkleTree = await getMerkleTree();
  const tree = new MerkleTree(3);
  // ... WASM works in Lambda
};
```

#### Vercel Edge Functions

```typescript
// pages/api/sync.ts (Edge runtime)
export const config = {
  runtime: 'edge',
};

export default async function handler(req: Request) {
  // WASM loads and works in Edge runtime
  const { merge_lww_batch } = await import('@topgunbuild/core-wasm');
  // ...
}
```

### 9.5 Architecture for Maximum Flexibility

```
@topgunbuild/core-wasm
+-- src/
|   +-- index.ts           # Smart loader (detects runtime)
|   +-- wasm/              # WASM binaries
|   |   +-- core_rust_bg.wasm
|   |   +-- core_rust.js
|   +-- fallback/          # Pure TS implementations
|   |   +-- merkle.ts
|   |   +-- crdt.ts
|   +-- strategies/
|       +-- node.ts        # Always use WASM
|       +-- browser.ts     # Always use TS (or configurable)
|       +-- edge.ts        # Always use WASM
+-- package.json
```

### 9.6 Summary: Runtime Strategy

```
1. Development:
   - Rust code for hot paths
   - TypeScript fallback (always)
   - Tests on both implementations

2. Build:
   - WASM: wasm-pack build --target web
   - TS: standard build

3. Distribution:
   - npm package contains both WASM and JS
   - Smart loader chooses implementation

4. Runtime:
   - Browser -> JS (smaller bundle)
   - Server -> WASM (performance)
   - Edge -> WASM (cold start)
   - Fallback -> JS (always works)
```

---

## 10. Backward Compatibility

### 10.1 JavaScript Fallback Pattern

```typescript
// packages/wasm/src/index.ts
let wasmModule: typeof import('./pkg') | null = null;

export async function init() {
  try {
    wasmModule = await import('./pkg');
  } catch {
    console.warn('WASM not available, using JS fallback');
  }
}

export function merkleHash(data: Uint8Array): string {
  if (wasmModule) {
    return wasmModule.merkle_hash(data);
  }
  // JavaScript fallback
  return jsMerkleHash(data);
}
```

### 10.2 Factory Pattern for Instantiation

```typescript
// packages/core-wasm/src/index.ts

export async function createMerkleTree(options: MerkleTreeOptions) {
  if (options.useWasm !== false && await isWasmSupported()) {
    return WasmMerkleTree.create(options);
  }
  return new JsMerkleTree(options);
}
```

### 10.3 Benefits

- Works in environments without WASM support
- Graceful degradation
- No breaking changes to existing APIs
- 100% feature parity

---

## 11. Performance Targets & Success Criteria

### 11.1 Performance Targets

| Operation | Current (JS) | Target (WASM) | Speedup |
|-----------|--------------|---------------|---------|
| Merkle diff (10K nodes) | 50ms | 20ms | 2.5x |
| Batch merge (1K records) | 10ms | 3ms | 3x |
| Hash (1MB data) | 5ms | 1ms | 5x |
| Serialization (10K objects) | 100ms | 30ms | 3x |

**Note:** These are estimates based on Automerge experience. Actual results depend on data patterns.

### 11.2 Success Criteria

| Metric | Target |
|--------|--------|
| Merkle sync speedup | >=2x |
| Batch merge speedup | >=2x |
| Bundle size increase | <500KB |
| Memory usage | No regression |
| Test coverage | 100% parity with JS |
| Fallback coverage | 100% (all features work without WASM) |
| CRDT merge p99 | <10ms (no GC spikes) |

---

## 12. Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| WASM size (bundle bloat) | Medium | Tree-shaking, lazy loading, conditional loading |
| Build complexity | Medium | CI/CD automation, clear docs |
| Browser compatibility | Low | Feature detection + JS fallback |
| Maintenance overhead | Medium | Shared test suite JS/WASM |
| Debug difficulty | Medium | Source maps, logging |
| Team Rust knowledge | Medium | Training, documentation, start with PoC |

---

## 13. Decision Criteria

### 13.1 When to Execute Rust+WASM Migration

Start when ANY of these conditions are met:

1. **p99 latency spikes** - GC pauses visible in production
2. **Large dataset sync >5 seconds** - Delta sync too slow
3. **Memory pressure >4GB heap** - Server under memory stress
4. **100K+ records per map** - Scale requires optimization
5. **SQL feature requested** - Natural PoC opportunity
6. **10+ concurrent heavy sync operations**
7. **Users reporting slow merge after offline**

### 13.2 When NOT to Use Rust+WASM

| Scenario | Why Not WASM |
|----------|--------------|
| I/O bound code | Node.js async handles well |
| Simple logic | Overhead doesn't pay off |
| Frequently changing code | Development cycle slower |
| Small datasets (<1K records) | JS is fast enough |

### 13.3 Resource Requirements

- Rust expertise available (or training time: 2-4 weeks)
- 4-6 weeks development time
- Testing infrastructure ready

**Current recommendation:** Document and defer. Execute when scale demands it.

---

## 14. Conclusion

**Rust + WASM is recommended for TopGun** with these caveats:

1. **Start with PoC** - SQL Parser to validate pipeline
2. **Focus on hot paths** - MerkleTree, CRDT merge, DAG
3. **Always provide fallback** - JS implementations for non-WASM environments
4. **Measure everything** - Benchmarks before and after each module
5. **Don't migrate everything** - I/O bound code stays in TypeScript

**Expected outcome:**
- 50-60% speedup for MerkleTree operations
- Predictable p99 latency (no GC pauses)
- SQL support as bonus feature
- Same code works in Node.js, browser, and edge

---

## 15. References

### Internal

- `@topgunbuild/native` - Existing native addon pattern
- `packages/server/src/workers/` - Worker abstraction (DTO-based)

### External

- [wasm-pack documentation](https://rustwasm.github.io/wasm-pack/)
- [wasm-bindgen guide](https://rustwasm.github.io/wasm-bindgen/)
- [sqlparser-rs](https://github.com/sqlparser-rs/sqlparser-rs)
- [Automerge Rust migration](https://automerge.org/blog/automerge-2/) - Case study (10-100x speedup)
- [Rust msgpack (rmp-serde)](https://docs.rs/rmp-serde/)

### Source Audits

- `/Users/koristuvac/.gemini/antigravity/brain/94d9d173-e6c9-4be6-a30f-2ecd8928be86/RUST_WASM_STRATEGY.md.resolved`

---

## Appendix A: Architecture Audit

### Worker Interface Verification

**Location:** `packages/server/src/workers/worker-scripts/crdt.worker.ts`

```typescript
// Lines 43-129: LWW merge handler
registerHandler('lww-merge', (payload: unknown): LWWMergeResult => {
  const { records, existingState } = payload as LWWMergePayload;
  // Pure computation on plain objects
  // No class instances, no external state
});
```

**Verdict:** DTO-based, WASM-ready.

### Serialization Verification

**Location:** `packages/core/src/serializer.ts`

- Uses msgpackr (binary format)
- Rust equivalent: `rmp-serde` crate
- No custom serialization logic

**Verdict:** Compatible with Rust/WASM.

---

## Appendix B: Integration Points

### With Existing Phases

| Phase | Integration |
|-------|-------------|
| Phase 3 (Native Addons) | WASM can replace/complement native bindings |
| Phase 6 (Native Benchmarks) | Add WASM benchmarks to harness |
| Phase 10 (Cluster) | Faster sync benefits cluster performance |

---

**Document Status:** Complete (Consolidated from PHASE_16 and RUST_WASM_ANALYSIS)
**Next Step:** Monitor performance metrics, execute when scale demands
**Estimated Total Effort:** 4-6 weeks
**Expected Speedup:** 2-3x on CPU-intensive operations
