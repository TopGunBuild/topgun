# bin/

This directory contains the prebuilt `topgun-server` binary for linux x64.

The binary is a build artifact — it is **not committed to git**. It is produced by running:

```bash
bash scripts/build-server-binaries.sh
```

from the repository root. That script cross-compiles `topgun-server` (default features: `redb` embedded storage)
for `x86_64-unknown-linux-gnu` using `cargo-zigbuild`, strips the result, and places it here as `bin/topgun-server`.

To use a prebuilt binary without building from source:

```bash
npm install @topgunbuild/server
npx topgun-server
```
