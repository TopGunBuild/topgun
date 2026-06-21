# Doc-test harness (G2 gate)

Every code snippet in the published documentation is extracted and **exercised
against reality** — either executed against a live Rust server, type-checked
against the real published package types, or explicitly skipped with a visible
reason. This is the G2 "executable documentation" gate from the stabilization
program: the mechanism that stops the docs from drifting out of sync with the
code.

```bash
pnpm test:docs           # cargo-builds the server on first run
RUST_SERVER_BINARY=$PWD/target/release/topgun-server pnpm test:docs   # CI: prebuilt
```

## What it covers

Sources walked (sorted, deterministic):

- `README.md`
- every `*.md` / `*.mdx` under `apps/docs-astro/src/content/`

`apps/docs-astro/public/llms-full.txt` is **not** walked separately — it is a
generated artifact (`scripts/build-llms-full.mjs`) whose snippets are a strict
subset of the MDX it is built from. Testing the MDX sources subsumes it.

## The three tiers

Every snippet gets exactly one verdict. There is no fourth "silently untested"
state, and there is **no allowlist** — a newly-added snippet is picked up
automatically.

| Tier | What happens | When |
|------|--------------|------|
| **run** | Executed for real: TS against a live Rust server (`spawnRustServer`, no-auth + deterministic embeddings), bash in a throwaway temp dir. | A self-contained, non-networking program (auto), or any block the author marks `doctest run` / `doctest setup`. Bash that is hermetic-safe. |
| **typecheck** | Compiled against the real `@topgunbuild/*` types. A renamed export/method, a wrong argument, a bad import — i.e. docs-overpromise — fails here. | Default for every TS/TSX fragment. |
| **skip** | Not exercised — but always with a **reason**, surfaced in the manifest. | Explicit `doctest skip`, non-executable languages (json/rust/text/…), non-standalone-parseable notation, and side-effecting shell commands. |

The run tier substitutes the documented `localhost:8080` authority for the live
ephemeral port (the snippet's path is preserved, so a wrong documented WS path
is still caught), maps `@topgunbuild/adapters` → an in-memory adapter (no
IndexedDB in Node), and stubs UI-placeholder calls (`render`, `display`, …) so
wiring examples run headless. A wired-up client must reach `CONNECTED` — that is
what makes "runs against a real server" real and not just "constructs an
object".

## Authoring directives

By default you write nothing — TS is type-checked, runnable TS/bash runs. Opt
out or escalate with a directive, carried either as a render-invisible comment
immediately above the fence, or as tokens on the fence info string:

```mdx
{/* doctest skip reason="illustrative pseudocode" */}
```ts
// ... not executed, but visible in the manifest with the reason ...
```
```

```md
<!-- doctest run reason="flagship example — executed against the live server" -->
```typescript
// ... executed end-to-end ...
```
```

Recognised tokens after `doctest`:

- `skip` — illustrative; **requires** `reason="…"`. The manifest integrity test
  fails on a reasonless skip.
- `run` — force-execute this block (a complete runnable example).
- `setup` — like `run`, and also contribute this block as a shared preamble for
  the other run blocks on the same page (mdBook-style narrative).
- `vector` — needs the embedding-enabled server profile.

`doctest-skip` / `doctest-run` (dash form) work too.

## Negative control

`negative-control.test.ts` proves the gate fails when docs break — it feeds
deliberately-broken snippets (renamed method, bad import, wrong arity, runtime
throw, reasonless skip) through the real pipeline and asserts each is caught. To
verify by hand, introduce a typo in any doc snippet (`client.getMap` →
`client.getMapp`) and watch the typecheck suite redden.

## Files

| File | Role |
|------|------|
| `helpers/extract.ts` | Walk docs, parse fenced blocks + directives |
| `helpers/classify.ts` | Assign the run/typecheck/skip verdict |
| `helpers/tsc.ts` | Typecheck against real types; drift-vs-fragment-noise filter; self-containment |
| `helpers/doc-scope.d.ts` | Typed glossary of ambient doc identifiers (`client`, `Predicates`, …) |
| `helpers/assemble.ts` | Build + execute a run-tier module (import→require, URL inject, vm) |
| `helpers/server.ts` | Boot the Rust server with the doc profile |
| `helpers/client-shim.ts` | Track clients → assert handshake + tear down |
| `helpers/memory-adapter.ts` / `adapters-shim.ts` | In-memory storage for headless Node |
| `doc-typecheck.test.ts` | Manifest integrity + typecheck tier |
| `doc-exec.test.ts` | Run tier against the live server |
| `doc-bash.test.ts` | Bash tier |
| `negative-control.test.ts` | Prove the gate fails on broken docs |
