# bin/

This directory contains the prebuilt `topgun-server` binary for darwin arm64 (Apple Silicon M-series).

The binary is a build artifact — it is **not committed to git**. It is produced by running:

```bash
bash scripts/build-server-binaries.sh
```

from the repository root. That script cross-compiles `topgun-server` (default features: `redb` embedded storage)
for `aarch64-apple-darwin` using `cargo-zigbuild`, strips the result, and places it here as `bin/topgun-server`.

To use a prebuilt binary without building from source:

```bash
npm install @topgunbuild/server
npx @topgunbuild/server
```
