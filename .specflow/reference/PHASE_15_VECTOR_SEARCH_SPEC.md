# Phase 15: Vector Search - Technical Specification

**Version:** 1.4
**Date:** 2026-01-05
**Status:** Approved (Ready for implementation)
**Dependencies:** Phase 12 (Hybrid Search), Phase 13 (MCP Server), Phase 14 (Distributed Search)
**Reference:** transformers.js, usearch, Graphiti MCP
**Priority:** HIGH (AI Memory capability, competitive differentiation)
**Package:** `@topgunbuild/vector` (separate, optional)

---

## Executive Summary

Phase 15 introduces **Semantic Vector Search** to TopGun, enabling AI-powered similarity search across documents. This transforms TopGun from a "database with search" into an **"AI Memory"** system capable of understanding the *meaning* of content, not just keywords.

### What is Vector Search?

Vector search (semantic search) converts text into mathematical vectors (embeddings) that capture semantic meaning. Similar concepts cluster together in vector space, enabling:

- "Find documents similar to this one"
- "Search by meaning, not keywords"
- Retrieval Augmented Generation (RAG) for AI assistants

### Current State (Phase 12)

- ✅ BM25 full-text search (keyword-based)
- ✅ Exact match search
- ✅ Hybrid ranking with RRF
- ❌ No semantic understanding
- ❌ "authentication" won't find "OAuth2" or "login"

### Target State (Phase 15)

- ✅ Local embedding generation (transformers.js)
- ✅ Vector storage as CRDT data (synced)
- ✅ HNSW index for fast KNN search
- ✅ Tri-hybrid search: Exact + BM25 + Semantic
- ✅ MCP tool: `topgun_search` with `method: 'semantic'`
- ✅ Works offline, no API keys required

---

## Architecture Decision: Sync Vectors (Variant A)

**Decision:** Vectors are generated on ONE node and synchronized to ALL nodes as data.

### Rationale

1. **Energy Efficiency**
   - Download 1.5 KB < Generate embedding (100-500ms CPU)
   - 10,000 docs = 15 MB (trivial vs. generating on each device)

2. **Determinism & Consistency**
   - Same embedding = same search results everywhere
   - No model version drift between devices
   - Predictable UX

3. **Thin Client Support**
   - Old phones, IoT, web clients don't need ONNX runtime
   - Just receive and index pre-computed vectors

### Trade-off

| Aspect | Sync Vectors (A) | Local Gen (B) |
|--------|-----------------|---------------|
| Network | +15 MB / 10K docs | 0 |
| CPU per client | Index only | Generate + Index |
| Battery | Low | High |
| Consistency | Guaranteed | Potential drift |
| Thin clients | Supported | Not supported |

**Winner: Variant A (Sync Vectors)**

---

## Strategic Decision: Separate Package

### Why a Separate Package?

Vector search adds significant complexity and bundle size that not all users need:

| Impact | Without Vector | With Vector |
|--------|---------------|-------------|
| Bundle size | ~50 KB | +50-100 MB |
| Dependencies | 0 native | ONNX, WASM |
| Complexity | Low | High (build config) |

**Decision:** Implement as `@topgunbuild/vector` — an **optional plugin** that extends the core.

### Package Architecture

```
@topgunbuild/core (unchanged, ~50 KB)
    ↓
@topgunbuild/client (unchanged)
    ↓
@topgunbuild/vector (NEW, optional)
    ├── IEmbeddingProvider + LocalEmbeddingProvider
    ├── IVectorIndex + VoyVectorIndex + UsearchVectorIndex
    ├── TriHybridSearchEngine
    ├── EmbeddingHook, EmbeddingObserver
    ├── vectorToBase64, base64ToVector
    └── withVectorSearch() wrapper
```

### Usage Pattern

```typescript
// Basic TopGun (no vector search)
import { TopGunClient } from '@topgunbuild/client';
const client = new TopGunClient({ serverUrl: 'ws://localhost:8080' });

// With vector search (optional import)
import { TopGunClient } from '@topgunbuild/client';
import { withVectorSearch } from '@topgunbuild/vector';

const client = withVectorSearch(
  new TopGunClient({ serverUrl: 'ws://localhost:8080' }),
  {
    maps: ['notes', 'documents'],
    fields: {
      notes: ['title', 'content'],
      documents: ['title', 'body'],
    },
  }
);

// Now semantic search is available
const results = await client.search('notes', 'meaning of life', {
  methods: ['semantic', 'fulltext'],
});
```

### Competitive Advantage

TopGun's unique value proposition with vector search:

| Feature | TopGun | Chroma/Pinecone | SQLite-vss | RxDB |
|---------|--------|-----------------|------------|------|
| Offline-first | ✅ | ❌ | ✅ | ✅ |
| Auto-sync | ✅ | ❌ | ❌ | ✅ |
| P2P capable | ✅ | ❌ | ❌ | ❌ |
| Zero config | ✅ | ❌ | ❌ | ❌ |
| Local embeddings | ✅ | ❌ | ❌ | Plugin |

**Target niche:** "Apple Notes on steroids for developers" — local AI search that syncs everywhere.

---

## Important Implementation Notes (Audit Feedback)

### 1. Vector Serialization (Critical)

**Problem:** `Float32Array` serialized to JSON becomes `{ "0": 0.1, "1": 0.2, ... }` — extremely inefficient.

**Solution:** Always serialize vectors as Base64-encoded binary:

```typescript
// packages/vector/src/serialization.ts

/**
 * Serialize Float32Array to Base64 string for JSON storage
 * 384 floats × 4 bytes = 1536 bytes → ~2KB Base64
 */
export function vectorToBase64(vector: Float32Array): string {
  return Buffer.from(vector.buffer).toString('base64');
}

/**
 * Deserialize Base64 string back to Float32Array
 */
export function base64ToVector(base64: string): Float32Array {
  return new Float32Array(Buffer.from(base64, 'base64').buffer);
}

// Browser alternative (no Buffer)
export function vectorToBase64Browser(vector: Float32Array): string {
  const bytes = new Uint8Array(vector.buffer);
  let binary = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export function base64ToVectorBrowser(base64: string): Float32Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return new Float32Array(bytes.buffer);
}
```

**Storage format in CRDT:**
```typescript
interface VectorDocument {
  title: string;
  body: string;
  // Vector stored as Base64, NOT Float32Array
  _embedding?: string; // Base64-encoded Float32Array
  _embeddingModel?: string;
  _embeddingVersion?: number;
}
```

### 2. Build Configuration (transformers.js + ONNX)

**Problem:** ONNX runtime binaries must be correctly bundled for Node.js/Electron.

**Solution:** Configure tsup to handle native modules:

```typescript
// packages/vector/tsup.config.ts

import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['cjs', 'esm'],
  dts: true,
  external: [
    // Mark ONNX runtime as external - will be loaded at runtime
    'onnxruntime-node',
    'onnxruntime-web',
    '@xenova/transformers',
    // Native modules
    'usearch',
  ],
  noExternal: [],
  // Copy ONNX binaries for Electron builds
  esbuildPlugins: [],
});
```

**For Electron builds:**
```bash
# Post-install script to copy ONNX binaries
cp node_modules/onnxruntime-node/bin/napi-v3/*/onnxruntime_binding.node ./dist/
```

### 3. Vector Index: usearch vs voy (Fallback Strategy)

**Problem:** `usearch` requires `node-gyp` compilation, which can fail on some systems.

**Solution:** Implement factory with automatic fallback:

```typescript
// packages/vector/src/createVectorIndex.ts

import type { IVectorIndex } from './types';

export type VectorIndexBackend = 'usearch' | 'voy' | 'auto';

export async function createVectorIndex(
  dimension: number,
  backend: VectorIndexBackend = 'auto'
): Promise<IVectorIndex> {
  if (backend === 'usearch' || backend === 'auto') {
    try {
      const { UsearchVectorIndex } = await import('./UsearchVectorIndex');
      return new UsearchVectorIndex(dimension);
    } catch (e) {
      if (backend === 'usearch') {
        throw new Error(`usearch not available: ${e}`);
      }
      // Fall through to voy
      console.warn('usearch not available, falling back to voy (WASM)');
    }
  }

  // Fallback to voy (WASM, works everywhere)
  const { VoyVectorIndex } = await import('./VoyVectorIndex');
  return new VoyVectorIndex(dimension);
}
```

**Usage recommendation:**
- **Server/Desktop (Node.js, Electron):** Use `usearch` for maximum performance
- **Browser/CI/Cross-compile issues:** Use `voy` (WASM, no native deps)
- **Default:** `auto` - tries usearch, falls back to voy

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    TOPGUN VECTOR SEARCH (Phase 15)                       │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                    Embedding Provider (Adapter)                    │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐               │  │
│  │  │ Transformers│  │ Ollama      │  │ OpenAI      │               │  │
│  │  │ .js (Local) │  │ (Local LLM) │  │ (Cloud)     │               │  │
│  │  │ [DEFAULT]   │  │ [Optional]  │  │ [Optional]  │               │  │
│  │  └─────────────┘  └─────────────┘  └─────────────┘               │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                    │                                     │
│                                    ▼                                     │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                      Vector Storage (CRDT)                        │  │
│  │  ┌─────────────────────────────────────────────────────────────┐ │  │
│  │  │ Document: { title, body, _embedding: Float32Array(384) }    │ │  │
│  │  │            ↑                                                 │ │  │
│  │  │     Synced via existing TopGun protocol                      │ │  │
│  │  └─────────────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                    │                                     │
│                                    ▼                                     │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                    Vector Index (In-Memory HNSW)                  │  │
│  │  ┌─────────────┐                                                  │  │
│  │  │ usearch     │ ← Loaded from CRDT on startup                   │  │
│  │  │ or voy      │ ← Rebuilt on vector updates                     │  │
│  │  └─────────────┘                                                  │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                    │                                     │
│                                    ▼                                     │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                    Tri-Hybrid Search Engine                       │  │
│  │  ┌───────────┐  ┌───────────┐  ┌───────────┐                     │  │
│  │  │ Exact     │  │ BM25      │  │ Semantic  │                     │  │
│  │  │ Match     │  │ Full-Text │  │ Vector    │                     │  │
│  │  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘                     │  │
│  │        └──────────────┼──────────────┘                            │  │
│  │                       ▼                                            │  │
│  │              ┌─────────────────┐                                   │  │
│  │              │ RRF Fusion      │                                   │  │
│  │              │ (k=60)          │                                   │  │
│  │              └─────────────────┘                                   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Component Specifications

### 15.01: Embedding Provider Interface

**Priority:** Critical
**Effort:** 2 days
**Package:** `@topgunbuild/core` or `@topgunbuild/vector`

#### Interface Definition

```typescript
// packages/vector/src/types.ts

/**
 * Embedding provider interface
 * Allows pluggable embedding backends
 */
export interface IEmbeddingProvider {
  /** Provider name for logging/config */
  readonly name: string;

  /** Vector dimension (e.g., 384 for MiniLM) */
  readonly dimension: number;

  /** Generate embedding for single text */
  embed(text: string): Promise<Float32Array>;

  /** Batch embed multiple texts (more efficient) */
  batchEmbed(texts: string[]): Promise<Float32Array[]>;

  /** Cleanup resources (unload model) */
  dispose(): Promise<void>;
}

/**
 * Embedding provider configuration
 */
export interface EmbeddingConfig {
  provider: 'local' | 'ollama' | 'openai';
  model?: string;
  /** For Ollama/OpenAI */
  baseUrl?: string;
  apiKey?: string;
}
```

#### Default Implementation (transformers.js)

```typescript
// packages/vector/src/LocalEmbeddingProvider.ts

import { pipeline, env } from '@xenova/transformers';

export class LocalEmbeddingProvider implements IEmbeddingProvider {
  readonly name = 'transformers.js';
  readonly dimension = 384; // MiniLM-L6-v2

  private pipeline: any = null;
  private modelId: string;

  constructor(modelId = 'Xenova/all-MiniLM-L6-v2') {
    this.modelId = modelId;
    // Disable remote model loading for offline-first
    env.allowRemoteModels = false;
    env.localModelPath = './models';
  }

  private async ensureLoaded(): Promise<void> {
    if (!this.pipeline) {
      this.pipeline = await pipeline('feature-extraction', this.modelId, {
        quantized: true, // Use quantized model for speed
      });
    }
  }

  async embed(text: string): Promise<Float32Array> {
    await this.ensureLoaded();

    const output = await this.pipeline(text, {
      pooling: 'mean',
      normalize: true,
    });

    return new Float32Array(output.data);
  }

  async batchEmbed(texts: string[]): Promise<Float32Array[]> {
    await this.ensureLoaded();

    // Process in batches of 32 for memory efficiency
    const batchSize = 32;
    const results: Float32Array[] = [];

    for (let i = 0; i < texts.length; i += batchSize) {
      const batch = texts.slice(i, i + batchSize);
      const outputs = await this.pipeline(batch, {
        pooling: 'mean',
        normalize: true,
      });

      // Extract each embedding from batch output
      for (let j = 0; j < batch.length; j++) {
        const start = j * this.dimension;
        const end = start + this.dimension;
        results.push(new Float32Array(outputs.data.slice(start, end)));
      }
    }

    return results;
  }

  async dispose(): Promise<void> {
    this.pipeline = null;
  }
}
```

#### Optional: Ollama Provider

```typescript
// packages/vector/src/OllamaEmbeddingProvider.ts

export class OllamaEmbeddingProvider implements IEmbeddingProvider {
  readonly name = 'ollama';
  readonly dimension: number;

  private baseUrl: string;
  private model: string;

  constructor(config: { baseUrl?: string; model?: string; dimension?: number }) {
    this.baseUrl = config.baseUrl || 'http://localhost:11434';
    this.model = config.model || 'nomic-embed-text';
    this.dimension = config.dimension || 768;
  }

  async embed(text: string): Promise<Float32Array> {
    const response = await fetch(`${this.baseUrl}/api/embeddings`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: this.model, prompt: text }),
    });

    const data = await response.json();
    return new Float32Array(data.embedding);
  }

  async batchEmbed(texts: string[]): Promise<Float32Array[]> {
    // Ollama doesn't support batch, so process sequentially
    return Promise.all(texts.map(t => this.embed(t)));
  }

  async dispose(): Promise<void> {
    // No cleanup needed
  }
}
```

---

### 15.02: Vector Index (HNSW)

**Priority:** Critical
**Effort:** 2 days
**Recommendation:** `usearch` (fastest, C++ bindings) or `voy` (WASM, browser-compatible)

#### Interface Definition

```typescript
// packages/vector/src/types.ts

export interface IVectorIndex {
  /** Add vector with ID */
  add(id: string, vector: Float32Array): void;

  /** Remove vector by ID */
  remove(id: string): void;

  /** Find k nearest neighbors */
  search(query: Float32Array, k: number): Array<{
    id: string;
    score: number; // Cosine similarity (0-1)
  }>;

  /** Number of vectors in index */
  readonly size: number;

  /** Rebuild index (after bulk updates) */
  rebuild(): void;

  /** Serialize index to bytes */
  serialize(): Uint8Array;

  /** Load index from bytes */
  deserialize(data: Uint8Array): void;
}
```

#### Implementation with usearch

```typescript
// packages/vector/src/UsearchVectorIndex.ts

import { Index, MetricKind } from 'usearch';

export class UsearchVectorIndex implements IVectorIndex {
  private index: Index;
  private idToLabel: Map<string, number> = new Map();
  private labelToId: Map<number, string> = new Map();
  private nextLabel = 0;
  private dimension: number;

  constructor(dimension: number, capacity = 100000) {
    this.dimension = dimension;
    this.index = new Index({
      metric: MetricKind.Cosine,
      dimensions: dimension,
      capacity,
    });
  }

  add(id: string, vector: Float32Array): void {
    // Check if ID already exists
    if (this.idToLabel.has(id)) {
      this.remove(id);
    }

    const label = this.nextLabel++;
    this.idToLabel.set(id, label);
    this.labelToId.set(label, id);

    this.index.add(label, vector);
  }

  remove(id: string): void {
    const label = this.idToLabel.get(id);
    if (label !== undefined) {
      this.index.remove(label);
      this.idToLabel.delete(id);
      this.labelToId.delete(label);
    }
  }

  search(query: Float32Array, k: number): Array<{ id: string; score: number }> {
    const results = this.index.search(query, k);

    return results.keys.map((label: number, i: number) => ({
      id: this.labelToId.get(label) || '',
      score: 1 - results.distances[i], // Convert distance to similarity
    })).filter(r => r.id !== '');
  }

  get size(): number {
    return this.idToLabel.size;
  }

  rebuild(): void {
    // usearch handles this automatically
  }

  serialize(): Uint8Array {
    // Export index to buffer
    const indexData = this.index.save();
    const metadata = JSON.stringify({
      idToLabel: Array.from(this.idToLabel.entries()),
      nextLabel: this.nextLabel,
    });

    // Combine metadata + index
    const metadataBytes = new TextEncoder().encode(metadata);
    const combined = new Uint8Array(4 + metadataBytes.length + indexData.length);
    new DataView(combined.buffer).setUint32(0, metadataBytes.length, true);
    combined.set(metadataBytes, 4);
    combined.set(new Uint8Array(indexData), 4 + metadataBytes.length);

    return combined;
  }

  deserialize(data: Uint8Array): void {
    const metadataLength = new DataView(data.buffer).getUint32(0, true);
    const metadataBytes = data.slice(4, 4 + metadataLength);
    const indexData = data.slice(4 + metadataLength);

    const metadata = JSON.parse(new TextDecoder().decode(metadataBytes));
    this.idToLabel = new Map(metadata.idToLabel);
    this.labelToId = new Map(metadata.idToLabel.map(([k, v]: [string, number]) => [v, k]));
    this.nextLabel = metadata.nextLabel;

    this.index.load(indexData.buffer);
  }
}
```

#### Alternative Implementation with voy (WASM fallback)

```typescript
// packages/vector/src/VoyVectorIndex.ts

import { Voy } from 'voy-search';

export class VoyVectorIndex implements IVectorIndex {
  private index: Voy;
  private vectors: Map<string, Float32Array> = new Map();
  private dimension: number;

  constructor(dimension: number) {
    this.dimension = dimension;
    this.index = new Voy({ numDimensions: dimension });
  }

  add(id: string, vector: Float32Array): void {
    this.vectors.set(id, vector);
    this.rebuild();
  }

  remove(id: string): void {
    this.vectors.delete(id);
    this.rebuild();
  }

  search(query: Float32Array, k: number): Array<{ id: string; score: number }> {
    const results = this.index.search(query, k);
    return results.neighbors.map((n: { id: string; distance: number }) => ({
      id: n.id,
      score: 1 - n.distance, // Convert distance to similarity
    }));
  }

  get size(): number {
    return this.vectors.size;
  }

  rebuild(): void {
    // Voy requires rebuilding index from scratch
    const resource = {
      embeddings: Array.from(this.vectors.entries()).map(([id, vector]) => ({
        id,
        embeddings: Array.from(vector),
      })),
    };
    this.index = Voy.index(resource);
  }

  serialize(): Uint8Array {
    const data = {
      vectors: Array.from(this.vectors.entries()).map(([id, vec]) => ({
        id,
        vec: Array.from(vec),
      })),
    };
    return new TextEncoder().encode(JSON.stringify(data));
  }

  deserialize(data: Uint8Array): void {
    const parsed = JSON.parse(new TextDecoder().decode(data));
    this.vectors = new Map(
      parsed.vectors.map((v: { id: string; vec: number[] }) => [v.id, new Float32Array(v.vec)])
    );
    this.rebuild();
  }
}
```

---

### 15.03: Vector Storage in CRDT

**Priority:** Critical
**Effort:** 2 days

#### Schema Extension

Documents with embeddings store them in a reserved `_embedding` field.

> **Important:** Vectors are stored as **Base64-encoded strings**, not raw Float32Array.
> See "Important Implementation Notes" section above.

```typescript
interface VectorDocument {
  // Regular fields
  title: string;
  body: string;
  // ... other fields

  // Reserved vector field (hidden from normal queries)
  // Stored as Base64 string for efficient JSON serialization
  _embedding?: string; // Base64-encoded Float32Array (384 floats → ~2KB)
  _embeddingModel?: string; // e.g., "MiniLM-L6-v2"
  _embeddingVersion?: number; // For re-embedding on model updates
}
```

#### Embedding Hook (Write Path)

```typescript
// packages/vector/src/EmbeddingHook.ts

import type { IEmbeddingProvider } from './types';
import { vectorToBase64 } from './serialization';

export interface EmbeddingHookConfig {
  provider: IEmbeddingProvider;
  /** Fields to embed (concatenated) */
  fields: string[];
  /** Maps to auto-embed */
  maps: string[];
}

export class EmbeddingHook {
  private provider: IEmbeddingProvider;
  private config: EmbeddingHookConfig;

  constructor(config: EmbeddingHookConfig) {
    this.config = config;
    this.provider = config.provider;
  }

  /**
   * Called on document write
   * Generates embedding if needed
   */
  async onPut(
    map: string,
    key: string,
    newValue: Record<string, unknown>,
    oldValue?: Record<string, unknown>
  ): Promise<Record<string, unknown>> {
    // Skip if not a target map
    if (!this.config.maps.includes(map)) {
      return newValue;
    }

    // Skip if embedding already exists and content unchanged
    if (this.hasValidEmbedding(newValue, oldValue)) {
      return newValue;
    }

    // Extract text from configured fields
    const text = this.config.fields
      .map(f => String(newValue[f] || ''))
      .filter(Boolean)
      .join(' ');

    if (!text.trim()) {
      return newValue;
    }

    // Generate embedding and serialize to Base64
    const embedding = await this.provider.embed(text);

    return {
      ...newValue,
      _embedding: vectorToBase64(embedding), // Store as Base64 string
      _embeddingModel: this.provider.name,
      _embeddingVersion: 1,
    };
  }

  private hasValidEmbedding(
    newValue: Record<string, unknown>,
    oldValue?: Record<string, unknown>
  ): boolean {
    if (!oldValue?._embedding) return false;

    // Check if any embedding fields changed
    for (const field of this.config.fields) {
      if (newValue[field] !== oldValue[field]) {
        return false;
      }
    }

    return true;
  }
}
```

#### "Eventual Embedding" Pattern

For thin clients that can't generate embeddings:

```typescript
// packages/vector/src/EmbeddingObserver.ts

import { vectorToBase64 } from './serialization';

class EmbeddingObserver {
  private client: TopGunClient;
  private provider: IEmbeddingProvider;

  constructor(client: TopGunClient, provider: IEmbeddingProvider) {
    this.client = client;
    this.provider = provider;
  }

  /**
   * Watch for documents without embeddings and fill them
   */
  observe(map: string, fields: string[]): void {
    this.client.getMap(map).subscribe((entries) => {
      for (const [key, value] of entries) {
        // Skip if already has embedding
        if (value._embedding) continue;

        // Generate embedding
        const text = fields.map(f => String(value[f] || '')).join(' ');
        if (!text.trim()) continue;

        this.provider.embed(text).then(embedding => {
          // Update document with embedding (as Base64)
          this.client.getMap(map).set(key, {
            ...value,
            _embedding: vectorToBase64(embedding),
            _embeddingModel: this.provider.name,
            _embeddingVersion: 1,
          });
        });
      }
    });
  }
}
```

---

### 15.04: Tri-Hybrid Search Engine

**Priority:** Critical
**Effort:** 2 days

#### Integration with Existing Search

```typescript
// packages/vector/src/TriHybridSearchEngine.ts

import type { IVectorIndex, IEmbeddingProvider } from './types';
import { base64ToVector } from './serialization';
import type { BM25Scorer } from '@topgunbuild/core';

export interface TriHybridSearchOptions {
  methods: Array<'exact' | 'fulltext' | 'semantic'>;
  limit: number;
  minScore?: number;
  /** Weight for semantic results in RRF (default: 1.0) */
  semanticWeight?: number;
}

export interface SearchResult<T> {
  key: string;
  value: T;
  score: number;
  matchedBy: Array<'exact' | 'fulltext' | 'semantic'>;
}

export class TriHybridSearchEngine<T extends Record<string, unknown>> {
  private bm25: BM25Scorer;
  private vectorIndex: IVectorIndex;
  private embeddingProvider: IEmbeddingProvider;

  constructor(
    bm25: BM25Scorer,
    vectorIndex: IVectorIndex,
    embeddingProvider: IEmbeddingProvider
  ) {
    this.bm25 = bm25;
    this.vectorIndex = vectorIndex;
    this.embeddingProvider = embeddingProvider;
  }

  async search(
    query: string,
    documents: Map<string, T>,
    options: TriHybridSearchOptions
  ): Promise<SearchResult<T>[]> {
    const { methods, limit, semanticWeight = 1.0 } = options;

    // Collect results from each method
    const resultSets: Map<string, { ranks: Map<string, number>; method: string }> = new Map();

    // 1. Exact Match
    if (methods.includes('exact')) {
      const exactResults = this.exactSearch(query, documents);
      resultSets.set('exact', {
        ranks: new Map(exactResults.map((id, i) => [id, i + 1])),
        method: 'exact',
      });
    }

    // 2. BM25 Full-Text
    if (methods.includes('fulltext')) {
      const bm25Results = this.bm25.score(query);
      resultSets.set('fulltext', {
        ranks: new Map(bm25Results.slice(0, 50).map((r, i) => [r.key, i + 1])),
        method: 'fulltext',
      });
    }

    // 3. Semantic Vector Search
    if (methods.includes('semantic')) {
      const queryEmbedding = await this.embeddingProvider.embed(query);
      const vectorResults = this.vectorIndex.search(queryEmbedding, 50);
      resultSets.set('semantic', {
        ranks: new Map(vectorResults.map((r, i) => [r.id, i + 1])),
        method: 'semantic',
      });
    }

    // RRF Fusion
    const k = 60; // Standard RRF constant
    const fusedScores: Map<string, { score: number; methods: string[] }> = new Map();

    for (const [method, { ranks }] of resultSets) {
      const weight = method === 'semantic' ? semanticWeight : 1.0;

      for (const [id, rank] of ranks) {
        const existing = fusedScores.get(id) || { score: 0, methods: [] };
        existing.score += weight / (k + rank);
        existing.methods.push(method);
        fusedScores.set(id, existing);
      }
    }

    // Sort by fused score and return top results
    const sorted = Array.from(fusedScores.entries())
      .sort((a, b) => b[1].score - a[1].score)
      .slice(0, limit);

    return sorted.map(([key, { score, methods: matchedBy }]) => ({
      key,
      value: documents.get(key)!,
      score,
      matchedBy: matchedBy as Array<'exact' | 'fulltext' | 'semantic'>,
    }));
  }

  private exactSearch(query: string, documents: Map<string, T>): string[] {
    const results: string[] = [];
    const lowerQuery = query.toLowerCase();

    for (const [key, value] of documents) {
      const text = Object.values(value)
        .filter(v => typeof v === 'string')
        .join(' ')
        .toLowerCase();

      if (text.includes(lowerQuery)) {
        results.push(key);
      }
    }

    return results;
  }
}
```

---

### 15.05: MCP Tool Update

**Priority:** High
**Effort:** 1 day

#### Updated `topgun_search` Schema

```json
{
  "name": "topgun_search",
  "description": "Perform tri-hybrid search (exact + BM25 + semantic) across a TopGun map",
  "inputSchema": {
    "type": "object",
    "properties": {
      "map": {
        "type": "string",
        "description": "Name of the map to search"
      },
      "query": {
        "type": "string",
        "description": "Search query (keywords, phrases, or natural language)"
      },
      "methods": {
        "type": "array",
        "items": {
          "type": "string",
          "enum": ["exact", "fulltext", "semantic"]
        },
        "default": ["exact", "fulltext", "semantic"],
        "description": "Search methods to use. 'semantic' uses AI embeddings for meaning-based search."
      },
      "limit": {
        "type": "number",
        "default": 10
      },
      "minScore": {
        "type": "number",
        "default": 0
      }
    },
    "required": ["map", "query"]
  }
}
```

#### Example Usage

```
User in Claude: "Find documents about user authentication"

Claude calls topgun_search:
{
  "map": "docs",
  "query": "user authentication",
  "methods": ["semantic", "fulltext"]
}

Response includes:
1. [semantic] "OAuth2 Integration Guide" (score: 0.89)
   - No keyword match, but semantically similar
2. [fulltext+semantic] "Authentication Best Practices" (score: 0.95)
   - Both keyword and semantic match
3. [semantic] "Login Flow Documentation" (score: 0.82)
   - Semantic match to "authentication" → "login"
```

---

### 15.06: Configuration

**Priority:** Medium
**Effort:** 1 day

#### Config Schema

```typescript
// packages/vector/src/types.ts

export interface VectorSearchConfig {
  /** Enable vector search (default: false) */
  enabled: boolean;

  /** Embedding provider (default: 'local') */
  provider: 'local' | 'ollama' | 'openai';

  /** Model identifier */
  model?: string;

  /** Vector dimension (auto-detected from model if not specified) */
  dimension?: number;

  /** For remote providers */
  baseUrl?: string;
  apiKey?: string;

  /** Maps to index for vector search */
  indexedMaps?: string[];

  /** Fields to embed per map */
  embeddingFields?: Record<string, string[]>;
}

// Usage with withVectorSearch wrapper
import { TopGunClient } from '@topgunbuild/client';
import { withVectorSearch } from '@topgunbuild/vector';

const client = withVectorSearch(
  new TopGunClient({ serverUrl: 'ws://localhost:8080' }),
  {
    provider: 'local',
    model: 'Xenova/all-MiniLM-L6-v2',
    maps: ['documents', 'notes'],
    fields: {
      documents: ['title', 'content'],
      notes: ['text'],
    },
  }
);
```

#### withVectorSearch Wrapper Implementation

```typescript
// packages/vector/src/withVectorSearch.ts

import type { TopGunClient } from '@topgunbuild/client';
import { LocalEmbeddingProvider } from './LocalEmbeddingProvider';
import { createVectorIndex } from './createVectorIndex';
import { EmbeddingHook } from './EmbeddingHook';
import { TriHybridSearchEngine } from './TriHybridSearchEngine';

export interface WithVectorSearchConfig {
  provider?: 'local' | 'ollama' | 'openai';
  model?: string;
  maps: string[];
  fields: Record<string, string[]>;
  indexBackend?: 'usearch' | 'voy' | 'auto';
}

export interface VectorSearchClient extends TopGunClient {
  search(
    map: string,
    query: string,
    options?: { methods?: Array<'exact' | 'fulltext' | 'semantic'>; limit?: number }
  ): Promise<Array<{ key: string; value: unknown; score: number }>>;
}

export async function withVectorSearch(
  client: TopGunClient,
  config: WithVectorSearchConfig
): Promise<VectorSearchClient> {
  // Initialize embedding provider
  const embeddingProvider = new LocalEmbeddingProvider(config.model);

  // Initialize vector index
  const vectorIndex = await createVectorIndex(
    embeddingProvider.dimension,
    config.indexBackend
  );

  // Set up embedding hook for writes
  const embeddingHook = new EmbeddingHook({
    provider: embeddingProvider,
    maps: config.maps,
    fields: config.fields[config.maps[0]] || [], // Simplified
  });

  // Create search engine
  const searchEngine = new TriHybridSearchEngine(
    client.getBM25Scorer(), // Assumes client exposes this
    vectorIndex,
    embeddingProvider
  );

  // Extend client with search method
  const extendedClient = client as VectorSearchClient;

  extendedClient.search = async (map, query, options = {}) => {
    const documents = new Map(client.getMap(map).entries());
    return searchEngine.search(query, documents, {
      methods: options.methods || ['exact', 'fulltext', 'semantic'],
      limit: options.limit || 10,
    });
  };

  return extendedClient;
}
```

#### MCP Server Config

```typescript
// packages/mcp-server/src/types.ts

export interface MCPServerConfig {
  // ... existing config

  vectorSearch?: {
    enabled: boolean;
    provider: 'local' | 'ollama' | 'openai';
    model?: string;
    baseUrl?: string;
    apiKey?: string;
  };
}
```

---

## Implementation Plan

### Phase 15.0: Package Setup

| Task | Effort | Priority |
|------|--------|----------|
| Create `packages/vector` directory structure | 0.5d | Critical |
| Set up package.json, tsconfig, tsup | 0.5d | Critical |
| Add to pnpm workspace | 0.5d | Critical |

### Phase 15.1: Core Infrastructure (Week 1)

| Task | Effort | Priority |
|------|--------|----------|
| 15.01: Embedding Provider Interface | 2d | Critical |
| 15.02: Vector Index (voy first, usearch optional) | 2d | Critical |
| Base64 serialization utilities | 0.5d | Critical |
| Unit tests for providers/index | 1d | High |

### Phase 15.2: Integration (Week 2)

| Task | Effort | Priority |
|------|--------|----------|
| 15.03: CRDT Storage for Vectors | 2d | Critical |
| 15.04: Tri-Hybrid Search Engine | 2d | Critical |
| withVectorSearch() wrapper | 1d | Critical |
| Integration tests | 1d | High |

### Phase 15.3: MCP & Config (Week 3)

| Task | Effort | Priority |
|------|--------|----------|
| 15.05: MCP Tool Update | 1d | High |
| 15.06: Configuration System | 1d | Medium |
| E2E tests with MCP | 1d | High |
| Documentation | 1d | Medium |

### Phase 15.4: Cluster Integration (Week 4)

| Task | Effort | Priority |
|------|--------|----------|
| 15.07: Extend ClusterSearchReqPayload | 0.5d | High |
| 15.07: ClusterSearchCoordinator semantic support | 1d | High |
| 15.07: ClusterSearchHandler local vector search | 1d | High |
| Cluster integration tests | 1d | High |
| Multi-node semantic search E2E test | 0.5d | High |

---

## Dependencies

### @topgunbuild/vector package.json

```json
{
  "name": "@topgunbuild/vector",
  "version": "0.1.0",
  "description": "Optional vector search plugin for TopGun - semantic search with local embeddings",
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
  "peerDependencies": {
    "@topgunbuild/client": "workspace:*",
    "@topgunbuild/core": "workspace:*"
  },
  "dependencies": {
    "@xenova/transformers": "^2.17.0",
    "voy-search": "^0.6.0"
  },
  "optionalDependencies": {
    "usearch": "^2.0.0"
  },
  "keywords": ["topgun", "vector", "search", "embeddings", "ai", "semantic"]
}
```

**Note on dependencies:**
- `voy-search` (WASM) is a required dependency — works everywhere without compilation
- `usearch` (C++ bindings) is optional — provides 2-3x better performance when available
- The `createVectorIndex()` factory automatically selects the best available backend
- `@topgunbuild/client` and `@topgunbuild/core` are peer dependencies (not bundled)

### Model Download

The default model (`Xenova/all-MiniLM-L6-v2`) will be downloaded on first use (~25MB quantized).

For fully offline deployment, models can be pre-bundled:

```bash
# Download model for offline use
npx @xenova/transformers download Xenova/all-MiniLM-L6-v2 --output ./models
```

---

## Success Criteria

### Functionality

- [ ] Local embedding generation works offline
- [ ] Vectors stored in CRDT and synced between nodes
- [ ] HNSW index provides <10ms KNN search
- [ ] Tri-hybrid search returns relevant results
- [ ] MCP tool supports `method: 'semantic'`
- [ ] Thin clients receive and index pre-computed vectors

### Performance

- [ ] Embedding generation: <100ms per document (local)
- [ ] Vector index search: <10ms for 100K vectors
- [ ] Memory: ~150MB for 100K vectors (384D)
- [ ] Sync: <2s for 1000 new vectors

### Quality

- [ ] Semantic search finds "OAuth2" when querying "authentication"
- [ ] RRF fusion improves relevance over single-method search
- [ ] No regression in BM25 search quality

### Testing

- [ ] 90%+ unit test coverage for vector components
- [ ] Integration tests for embedding sync
- [ ] E2E tests with MCP Server

### Cluster Mode

- [ ] Coordinator generates embedding once and broadcasts
- [ ] Each node performs local KNN search with received embedding
- [ ] Two-stage RRF merge produces consistent results
- [ ] Graceful degradation when semantic unavailable on some nodes
- [ ] Multi-node semantic search latency <200ms (3-node cluster)

---

### 15.07: Cluster Integration

**Priority:** High
**Effort:** 2 days
**Dependencies:** Phase 14 (Distributed Search)

#### Overview

In cluster mode, TopGun uses a Scatter-Gather pattern for search (Phase 14). Vector search must integrate with `ClusterSearchCoordinator` to enable semantic search across all nodes.

**Key principle:** The coordinator node generates the embedding ONCE and broadcasts it to all partition owners, avoiding redundant embedding computation.

#### Extended Message Types

```typescript
// packages/core/src/messages/cluster-search.ts (Extension from Phase 14)

export interface ClusterSearchReqPayload {
  requestId: string;
  map: string;
  query: string;

  // NEW: Search method selection
  methods: Array<'exact' | 'fulltext' | 'semantic'>;

  // NEW: Pre-computed query embedding (Base64-encoded)
  // Generated ONCE by coordinator, sent to all nodes
  queryEmbedding?: string;

  limit: number;
  minScore?: number;
}

export interface ClusterSearchRespPayload {
  requestId: string;
  results: Array<{
    key: string;
    value: Record<string, unknown>;
    score: number;
    matchedBy: Array<'exact' | 'fulltext' | 'semantic'>;
  }>;
  nodeId: string;

  // NEW: Indicates if semantic search was executed
  semanticExecuted: boolean;
}
```

#### Coordinator Flow

```typescript
// packages/server/src/cluster/ClusterSearchCoordinator.ts (Extension)

import { base64ToVector, vectorToBase64 } from '@topgunbuild/vector';

export class ClusterSearchCoordinator {
  private embeddingProvider?: IEmbeddingProvider;

  async search(
    map: string,
    query: string,
    options: {
      methods: Array<'exact' | 'fulltext' | 'semantic'>;
      limit: number;
      minScore?: number;
    }
  ): Promise<SearchResult[]> {
    const { methods, limit, minScore } = options;

    // Step 1: Generate embedding ONCE on coordinator (if semantic requested)
    let queryEmbedding: string | undefined;
    if (methods.includes('semantic') && this.embeddingProvider) {
      const embedding = await this.embeddingProvider.embed(query);
      queryEmbedding = vectorToBase64(embedding);
    }

    // Step 2: Find partition owners for the map
    const owners = this.partitionService.getPartitionOwnersForMap(map);

    // Step 3: Scatter - send search request to all owners
    const requests = owners.map(nodeId =>
      this.sendSearchRequest(nodeId, {
        requestId: crypto.randomUUID(),
        map,
        query,
        methods,
        queryEmbedding, // Pre-computed, not re-generated on each node
        limit: limit * 2, // Request extra for RRF merge
        minScore,
      })
    );

    // Step 4: Gather - collect results from all nodes
    const responses = await Promise.allSettled(requests);
    const allResults: SearchResult[] = [];

    for (const response of responses) {
      if (response.status === 'fulfilled') {
        allResults.push(...response.value.results);
      }
    }

    // Step 5: Final RRF merge across all node results
    return this.rrfMerge(allResults, limit);
  }

  private rrfMerge(results: SearchResult[], limit: number): SearchResult[] {
    const k = 60;
    const fusedScores: Map<string, { score: number; result: SearchResult }> = new Map();

    // Group results by key and compute RRF scores
    const byKey = new Map<string, SearchResult[]>();
    for (const result of results) {
      const existing = byKey.get(result.key) || [];
      existing.push(result);
      byKey.set(result.key, existing);
    }

    // For each unique key, take the best result and compute fused score
    for (const [key, keyResults] of byKey) {
      // Take result with highest score
      const best = keyResults.reduce((a, b) => (a.score > b.score ? a : b));

      // RRF: sum of 1/(k + rank) for each method that found this result
      let rrfScore = 0;
      const methods = new Set(keyResults.flatMap(r => r.matchedBy));
      for (const result of keyResults) {
        const rank = results.indexOf(result) + 1;
        rrfScore += 1 / (k + rank);
      }

      fusedScores.set(key, { score: rrfScore, result: { ...best, score: rrfScore } });
    }

    return Array.from(fusedScores.values())
      .sort((a, b) => b.score - a.score)
      .slice(0, limit)
      .map(({ result }) => result);
  }
}
```

#### Node Handler (Partition Owner)

```typescript
// packages/server/src/cluster/ClusterSearchHandler.ts

import { base64ToVector } from '@topgunbuild/vector';

export class ClusterSearchHandler {
  private localSearchEngine: TriHybridSearchEngine;
  private vectorIndex: IVectorIndex;

  handleSearchRequest(payload: ClusterSearchReqPayload): ClusterSearchRespPayload {
    const { requestId, map, query, methods, queryEmbedding, limit, minScore } = payload;

    // Local search on this node's data
    const localDocs = this.getLocalDocuments(map);
    const results: SearchResult[] = [];

    // 1. Exact search (if requested)
    if (methods.includes('exact')) {
      results.push(...this.exactSearch(query, localDocs));
    }

    // 2. BM25 search (if requested)
    if (methods.includes('fulltext')) {
      results.push(...this.bm25Search(query, localDocs));
    }

    // 3. Semantic search (if requested AND embedding provided)
    let semanticExecuted = false;
    if (methods.includes('semantic') && queryEmbedding) {
      const embedding = base64ToVector(queryEmbedding);
      const vectorResults = this.vectorIndex.search(embedding, limit);

      for (const vr of vectorResults) {
        if (!minScore || vr.score >= minScore) {
          results.push({
            key: vr.id,
            value: localDocs.get(vr.id)!,
            score: vr.score,
            matchedBy: ['semantic'],
          });
        }
      }
      semanticExecuted = true;
    }

    // Local RRF merge before sending back
    const merged = this.localRrfMerge(results, limit);

    return {
      requestId,
      results: merged,
      nodeId: this.nodeId,
      semanticExecuted,
    };
  }
}
```

#### Architecture Diagram (Cluster Mode)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         CLUSTER SEMANTIC SEARCH                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   Client Request: search("notes", "machine learning concepts")               │
│                              │                                               │
│                              ▼                                               │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │                    COORDINATOR NODE                                  │   │
│   │  ┌─────────────────────────────────────────────────────────────┐   │   │
│   │  │ 1. Generate embedding ONCE                                   │   │   │
│   │  │    query → [0.12, -0.34, 0.56, ...] → Base64               │   │   │
│   │  └─────────────────────────────────────────────────────────────┘   │   │
│   │                              │                                       │   │
│   │  ┌───────────────────────────┼───────────────────────────┐         │   │
│   │  │                           │                           │         │   │
│   │  ▼                           ▼                           ▼         │   │
│   │  Node A                   Node B                      Node C       │   │
│   │  (Partitions 0-89)       (Partitions 90-180)        (Partitions 181-270)│
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │                       EACH NODE (Local Search)                       │   │
│   │                                                                      │   │
│   │  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐           │   │
│   │  │ Local BM25    │  │ Local Exact   │  │ Local Vector  │           │   │
│   │  │ Index         │  │ Match         │  │ Index (HNSW)  │           │   │
│   │  └───────┬───────┘  └───────┬───────┘  └───────┬───────┘           │   │
│   │          │                  │                  │                    │   │
│   │          └──────────────────┼──────────────────┘                    │   │
│   │                             ▼                                       │   │
│   │                    Local RRF Merge                                  │   │
│   │                             │                                       │   │
│   │                             ▼                                       │   │
│   │                    Top-K Results → Coordinator                      │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │                    COORDINATOR (Final Merge)                         │   │
│   │                                                                      │   │
│   │  Results from Node A ─┐                                             │   │
│   │  Results from Node B ─┼──► Global RRF Merge ──► Top-K to Client    │   │
│   │  Results from Node C ─┘                                             │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### Key Design Decisions

1. **Single Embedding Generation**
   - Coordinator generates embedding once
   - Base64-encoded and sent to all nodes
   - Avoids redundant computation (N nodes → 1 embedding, not N)

2. **Local Index Per Node**
   - Each node maintains its own HNSW index
   - Index contains only local partition data
   - Vectors synced via CRDT, index rebuilt locally

3. **Two-Stage RRF Merge**
   - Stage 1: Each node does local RRF merge
   - Stage 2: Coordinator does global RRF merge
   - Ensures consistent ranking across cluster

4. **Graceful Degradation**
   - If embedding provider unavailable on coordinator → skip semantic
   - If node has no vector index → return only exact/fulltext
   - `semanticExecuted` flag indicates actual method used

#### Configuration (Cluster Mode)

```typescript
// packages/server/src/types.ts

export interface ServerConfig {
  // ... existing config

  vectorSearch?: {
    enabled: boolean;
    provider: 'local' | 'ollama' | 'openai';
    model?: string;
    // Only coordinator needs embedding capability
    coordinatorOnly?: boolean;
  };
}
```

**Recommendation:** In cluster mode, configure `coordinatorOnly: true` to avoid loading embedding models on all nodes. Only the coordinator needs to generate embeddings.

#### Implementation Notes (Audit Feedback)

1. **Type Definition Order**
   - Update `packages/core` message types (`ClusterSearchReqPayload`) BEFORE implementing server-side handlers
   - This prevents compilation errors from type mismatches across monorepo packages
   - Run `pnpm build` after core changes to ensure dependent packages pick up new types

2. **Graceful Degradation & RRF Scoring**
   - When a node returns results without semantic (e.g., vector index failed), RRF scores will differ
   - RRF is mathematically robust to this, but **log `WARN`** when nodes have capability mismatch:
   ```typescript
   if (!resp.semanticExecuted && methods.includes('semantic')) {
     logger.warn({ nodeId: resp.nodeId }, 'Node did not execute semantic search');
   }
   ```
   - This helps admins identify cluster desync (e.g., one node's index corrupted)

3. **Cold Start on Coordinator Failover**
   - If `coordinatorOnly: true` and coordinator fails, new leader may not have model loaded
   - **Production recommendation:** Either:
     - Load model on ALL potential coordinators (higher RAM usage)
     - Accept 1-2s latency spike on first semantic query after leader election
   - Consider adding `preloadModel: boolean` config for explicit control

---

## Future Enhancements (Out of Scope)

1. **Re-embedding on Model Update**
   - Detect model version changes
   - Background re-embedding of all documents

2. **Selective Sync**
   - Option to NOT sync vectors (generate locally everywhere)
   - Useful for very large datasets

3. **Multi-Modal Embeddings**
   - Image embeddings (CLIP)
   - Audio embeddings (Whisper)

4. **Vector Compression**
   - Product Quantization for 4x storage reduction
   - Binary vectors for 32x reduction

---

## References

1. **transformers.js:** https://huggingface.co/docs/transformers.js
2. **usearch:** https://github.com/unum-cloud/usearch
3. **RRF Paper:** "Reciprocal Rank Fusion outperforms Condorcet and individual Rank Learning Methods"
4. **MiniLM:** https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
5. **Graphiti Vector Search:** Reference implementation in Python
