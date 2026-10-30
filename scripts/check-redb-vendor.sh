#!/usr/bin/env bash
# Checks the vendored redb (packages/server-rust/vendor/redb-2.6.3):
#
#   1. The workspace resolves `redb` to the vendored crate and to nothing else
#      (`cargo metadata --locked`), so `[patch.crates-io]` is in force.
#   2. The crates.io `redb-2.6.3.crate` has the pinned sha256.
#   3. The two patched files equal the crate's with scripts/redb-vendor.patch
#      applied, byte for byte.
#   4. Every other file of the vendored tree equals the crate byte for byte;
#      the only other differences allowed are TOPGUN-VENDOR.md (added) and the
#      files listed in OMITTED (left out, so a fresh checkout equals the tree).
#
# `--resolve-only` runs check 1 alone; the binary builds use it, so a build
# without the vendored redb fails before it compiles anything.
#
# Any failure, including a failed download, exits non-zero.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd -P)
VENDOR="packages/server-rust/vendor/redb-2.6.3"
CRATE_URL="https://static.crates.io/crates/redb/redb-2.6.3.crate"
CRATE_SHA256="8eca1e9d98d5a7e9002d0013e18d5a9b000aee942eb134883a82f06ebffb6c01"
PATCH="scripts/redb-vendor.patch"
PATCHED=(
  src/tree_store/page_store/cached_file.rs
  src/tree_store/page_store/page_manager.rs
)
OMITTED=(.cargo_vcs_info.json .cargo .github .gitignore .editorconfig)

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# 1. Every package whose manifest sits in a `redb-<version>` directory is a redb
#    package; exactly one may exist, and it must be the vendored one.
metadata=$(cargo metadata --format-version 1 --locked) ||
  fail "cargo metadata --locked failed (is the [patch.crates-io] redb entry in Cargo.toml and Cargo.lock?)"
resolved=$(printf '%s' "$metadata" | grep -o '"manifest_path":"[^"]*/redb-[0-9][^"/]*/Cargo.toml"' || true)
expected="\"manifest_path\":\"$ROOT/$VENDOR/Cargo.toml\""
[ "$resolved" = "$expected" ] ||
  fail "redb does not resolve to $VENDOR only; resolved manifests: ${resolved:-none}"
echo "OK: redb resolves to $VENDOR"

if [ "${1:-}" = "--resolve-only" ]; then
  exit 0
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# 2. The crate as published.
curl -fsSL --retry 3 -o "$work/redb.crate" "$CRATE_URL" || fail "download of $CRATE_URL failed"
actual=$(sha256 "$work/redb.crate")
[ "$actual" = "$CRATE_SHA256" ] || fail "redb-2.6.3.crate has sha256 $actual, expected $CRATE_SHA256"
tar -xzf "$work/redb.crate" -C "$work"
crate="$work/redb-2.6.3"
[ -d "$crate" ] || fail "the crate does not unpack to redb-2.6.3/"
echo "OK: redb-2.6.3.crate sha256 $CRATE_SHA256"

# 3. The patched files are the crate's with the committed patch applied, byte
#    for byte. Applying the patch, rather than comparing diff output, keeps the
#    check independent of the diff implementation of the host.
# The ceiling keeps git from treating an enclosing repository as the one to
# patch, which would resolve the patch's paths against that repository's root.
(cd "$crate" && GIT_CEILING_DIRECTORIES="$work" git apply --whitespace=nowarn "$ROOT/$PATCH") ||
  fail "$PATCH does not apply to the crate"
for file in "${PATCHED[@]}"; do
  cmp -s "$crate/$file" "$VENDOR/$file" || {
    diff -u "$crate/$file" "$VENDOR/$file" >&2 || true
    fail "$VENDOR/$file is not the crate's file with $PATCH applied"
  }
done
echo "OK: ${PATCHED[*]} are the crate's files with $PATCH applied"

# 4. The rest of the tree equals the crate.
for omitted in "${OMITTED[@]}"; do
  [ -e "$crate/$omitted" ] || fail "$omitted is not in the crate; update OMITTED"
  [ ! -e "$VENDOR/$omitted" ] && [ ! -L "$VENDOR/$omitted" ] ||
    fail "$VENDOR/$omitted must not be vendored"
  rm -rf "${crate:?}/$omitted"
done
[ -f "$VENDOR/TOPGUN-VENDOR.md" ] || fail "$VENDOR/TOPGUN-VENDOR.md is missing"
cp "$VENDOR/TOPGUN-VENDOR.md" "$crate/TOPGUN-VENDOR.md"
diff -r "$crate" "$VENDOR" >&2 ||
  fail "$VENDOR differs from the crate outside the allowed difference"
echo "OK: $VENDOR equals the crate apart from the allowed difference"
