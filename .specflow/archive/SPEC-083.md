> **SPLIT:** This specification was decomposed into:
> - SPEC-083a: Reference Documentation Rewrite for Rust Server
> - SPEC-083b: New Guides, Comparison Update, and Code Snippet Audit
> - SPEC-083c: Sync-Lab Session Isolation and Example Cleanup
>
> See child specifications for implementation.

---
id: SPEC-083
type: docs
status: split
priority: P2
complexity: large
created: 2026-03-08
---

# Update Documentation Content for Rust Server

## Context

The Rust server migration is functionally complete (Phase 3 done, all integration tests passing). The docs site (`apps/docs-astro/`) still reflects the TypeScript server API, CLI, and architecture. TODO-096 (Adoption Path + Security Model) is complete and its content needs to be incorporated. The comparison page is missing key competitors (Replicache/Zero, Cloudflare Durable Objects). The `examples/collaborative-tasks/` app should be replaced with a concise Tier 1 code snippet in docs. The `sync-lab` demo needs session isolation before it can be embedded on the homepage.

**Source TODO:** TODO-106
**Dependencies:** TODO-096 (done), TODO-068 (done)

## Goal Analysis

**Goal Statement:** Documentation accurately reflects the Rust server, provides clear adoption guidance, and the sync-lab demo is safe for public homepage embedding.

**Observable Truths:**
1. A developer reading `reference/server.mdx` sees Rust server startup, configuration, and API -- not TypeScript `ServerFactory`
2. A developer reading `reference/cli.mdx` sees the Rust binary CLI commands (`topgun-server` or equivalent)
3. `reference/protocol.mdx` accurately describes the MsgPack binary wire format used by the Rust server
4. `guides/security.mdx` contains a Security Model section covering JWT auth, TLS, mTLS, RBAC
5. A new Adoption Path page explains the 3-tier adoption model with a "TopGun + Your PostgreSQL" guide
6. `comparison.mdx` includes Replicache/Zero and Cloudflare Durable Objects columns
7. All code snippets in docs compile/run against the current SDK
8. The `examples/collaborative-tasks/` directory no longer exists; its essence is a 20-line Tier 1 snippet in docs
9. The `sync-lab` demo uses per-session namespaces, preventing cross-visitor state pollution

**Required Artifacts:**
- `apps/docs-astro/src/content/docs/reference/server.mdx` (rewrite)
- `apps/docs-astro/src/content/docs/reference/cli.mdx` (rewrite)
- `apps/docs-astro/src/content/docs/reference/protocol.mdx` (update)
- `apps/docs-astro/src/content/docs/guides/security.mdx` (extend)
- `apps/docs-astro/src/content/docs/guides/adoption-path.mdx` (new)
- `apps/docs-astro/src/content/docs/guides/postgresql.mdx` (new)
- `apps/docs-astro/src/content/docs/comparison.mdx` (extend)
- `apps/docs-astro/src/content/docs/quick-start.mdx` (update with Tier 1 snippet)
- `examples/collaborative-tasks/` (delete entire directory)
- `examples/sync-lab/src/hooks/useDeviceClient.ts` (session namespace)
- `examples/sync-lab/src/App.tsx` (session isolation + optional share button)
- `examples/sync-lab/src/lib/device-manager.ts` (session namespace)
