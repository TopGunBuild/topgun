# redb 2.6.3, vendored with one upstream fix

This directory is the crates.io crate `redb` 2.6.3 (`redb-2.6.3.crate`, sha256
`8eca1e9d98d5a7e9002d0013e18d5a9b000aee942eb134883a82f06ebffb6c01`), unpacked, with one change
backported from upstream redb. The workspace uses it through `[patch.crates-io]` in the root
`Cargo.toml`. It is not a workspace member.

## The change

Upstream commit `fd82ced` ("Make file growth durable to avoid an unopenable database after a
crash", first released in redb 4.2.0), with upstream's own comments:

- `src/tree_store/page_store/cached_file.rs:302-308` — `PagedCachedFile::sync_file()`, which syncs
  the backing file without flushing the write buffer. redb 2.x's `StorageBackend::sync_data` takes
  an `eventual` flag; the call passes `false`.
- `src/tree_store/page_store/page_manager.rs:1215-1224` — `TransactionalMemory::grow()` calls it
  right after `resize`. On an error it sets `needs_recovery`, as 2.6.3 already does when `resize`
  fails.

The exact change is `scripts/redb-vendor.patch`.

**Why.** redb 2.6.3 grows the file with `set_len` and orders that only by the commit's final
fsync. A crash could persist a header naming the larger layout and lose the growth; every later
open then panicked at `page_manager.rs:243`, though the durable state was intact. With the sync,
the file is never shorter than the layout its header names. It costs one extra `fsync` per
growth, and the file doubles while it is small.

## What differs from the crate

Exactly this, checked by `scripts/check-redb-vendor.sh` (also in CI):

1. the change above;
2. this file, added (the upstream `README.md` is untouched);
3. left out: `.cargo_vcs_info.json`, `.cargo/`, `.github/`, `.gitignore`, `.editorconfig` — the
   crate's `.gitignore` would keep its `Cargo.lock` out of git, and a fresh checkout must equal
   what the check compares.

Do not edit anything else here, and do not "fix" this source: `cargo clippy` and `cargo test`
print 39 warnings from redb, which are expected. Do not run cargo from inside this directory: the
crate's own `rust-toolchain`, `rustfmt.toml` and `clippy.toml` would apply. Run it from the
repository root.

## What guards it

- `scripts/check-redb-vendor.sh` fails when `[patch.crates-io]` is gone (`cargo metadata` must
  resolve `redb` to this directory), when the crate's sha256 differs, and when this tree differs
  from the crate by anything but the list above. The Docker image and
  `scripts/build-server-binaries.sh` run its `--resolve-only` part before they build.
- `k08_redb_crashes_while_the_file_grows_recover_a_prefix_that_holds_every_durable_batch` in
  `packages/server-rust/src/storage/datastores/kv_crash_tests.rs` runs in the default test suite
  and fails without the change, so removing it turns CI red — in 24 of 25 runs measured, not
  every run: redb iterates hash sets in a per-process random order, so which crash images panic
  varies from run to run. The vendor check above is the deterministic guard.

## Security advisories

`cargo audit` matches only registry crates against RUSTSEC. A path crate is not matched, so
`cargo audit` no longer reports advisories for redb. Check redb 2.6.3 advisories by hand
(https://rustsec.org/packages/redb.html) when you touch this directory.

## Removing it

TODO-889: when the server moves to a redb release that carries the fix (4.2.0 or later), or to
another engine, delete this directory, `scripts/check-redb-vendor.sh`,
`scripts/redb-vendor.patch`, the `[patch.crates-io]` entry, the check's steps in
`.github/workflows/rust.yml`, `deploy/Dockerfile.server` and `scripts/build-server-binaries.sh`, and
the redb entry in `NOTICE`.
