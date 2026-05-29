#!/usr/bin/env bash
# build-server-binaries.sh
#
# Cross-compile topgun-server for darwin-arm64 and linux-x64, strip each binary,
# assert size < 20 MB, place into the per-platform npm package, and run
# pnpm publish --dry-run for all three packages to verify publish-readiness.
#
# Build tool: cargo-zigbuild (preferred — clean cross-compile from macOS M1
# without Docker). If zigbuild is unavailable, install with:
#   cargo install cargo-zigbuild
#   pip3 install ziglang  # or: brew install zig
#
# Fallback: replace the cargo-zigbuild invocations below with `cross build ...`
# if you prefer the Docker-based cross-compilation approach.
#
# Prerequisites:
#   - Rust toolchain (cargo, rustup)
#   - cargo-zigbuild: cargo install cargo-zigbuild
#   - zig: brew install zig  OR  pip3 install ziglang
#   - Rust targets: rustup target add aarch64-apple-darwin x86_64-unknown-linux-gnu
#   - pnpm (for publish dry-run)
#
# Usage (from repo root):
#   bash scripts/build-server-binaries.sh
#
# Output:
#   packages/server-dist/npm/darwin-arm64/bin/topgun-server  (darwin arm64)
#   packages/server-dist/npm/linux-x64/bin/topgun-server     (linux x64)
#   pnpm publish --dry-run for each per-platform package + the meta-package
#
# This script runs `pnpm publish --dry-run` only — it never runs the real publish.
# The real publish (without --dry-run) is a manual operator step. See
# packages/server-dist/README.md for the publish workflow.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="topgun-server"
BIN="topgun-server"
MAX_SIZE_BYTES=$((20 * 1024 * 1024))  # 20 MB

echo ""
echo "TopGun Server — Cross-Compile + Publish-Readiness Check"
echo "========================================================="
echo ""

# ── Verify tools ──────────────────────────────────────────────────────────────

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo not found. Install Rust: https://rustup.rs"
  exit 1
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "ERROR: cargo-zigbuild not found."
  echo "  Install: cargo install cargo-zigbuild"
  echo "  Then:    brew install zig  OR  pip3 install ziglang"
  exit 1
fi

if ! command -v pnpm >/dev/null 2>&1; then
  echo "ERROR: pnpm not found. Install: npm install -g pnpm"
  exit 1
fi

# ── Ensure Rust cross-compile targets are installed ────────────────────────────

echo "Adding Rust targets (idempotent)..."
rustup target add aarch64-apple-darwin x86_64-unknown-linux-gnu

# ── Compile ────────────────────────────────────────────────────────────────────

echo ""
echo "[1/4] Building darwin-arm64 (aarch64-apple-darwin)..."
cd "${REPO_ROOT}"
cargo zigbuild --release --bin "${BIN}" --target aarch64-apple-darwin

DARWIN_ARM64_SRC="${REPO_ROOT}/target/aarch64-apple-darwin/release/${BIN}"

echo ""
echo "[2/4] Building linux-x64 (x86_64-unknown-linux-gnu)..."
cargo zigbuild --release --bin "${BIN}" --target x86_64-unknown-linux-gnu

LINUX_X64_SRC="${REPO_ROOT}/target/x86_64-unknown-linux-gnu/release/${BIN}"

# ── Strip ──────────────────────────────────────────────────────────────────────

echo ""
echo "[3/4] Stripping binaries..."

DARWIN_ARM64_OUT="${REPO_ROOT}/packages/server-dist/npm/darwin-arm64/bin/${BIN}"
LINUX_X64_OUT="${REPO_ROOT}/packages/server-dist/npm/linux-x64/bin/${BIN}"

# Darwin: use system strip with minimal flags (cross-strip via zig is not reliable
# for Mach-O; strip the native arm64 binary directly on macOS).
cp "${DARWIN_ARM64_SRC}" "${DARWIN_ARM64_OUT}"
strip "${DARWIN_ARM64_OUT}" || echo "  Warning: strip failed for darwin-arm64 (binary may be larger)"
chmod 0755 "${DARWIN_ARM64_OUT}"

# Linux: strip --strip-all if available, else basic strip.
cp "${LINUX_X64_SRC}" "${LINUX_X64_OUT}"
if strip --strip-all "${LINUX_X64_OUT}" 2>/dev/null; then
  echo "  linux-x64: stripped (--strip-all)"
else
  strip "${LINUX_X64_OUT}" 2>/dev/null || echo "  Warning: strip failed for linux-x64 (binary may be larger)"
fi
chmod 0755 "${LINUX_X64_OUT}"

# ── Size assertion ─────────────────────────────────────────────────────────────

check_size() {
  local path="$1"
  local label="$2"
  local size
  size=$(wc -c < "${path}")
  local mb
  mb=$(echo "${size} / 1048576" | bc 2>/dev/null || echo "?")
  echo "  ${label}: ${mb} MB (${size} bytes)"
  if [ "${size}" -gt "${MAX_SIZE_BYTES}" ]; then
    echo "ERROR: ${label} binary exceeds 20 MB ceiling (${size} bytes > ${MAX_SIZE_BYTES})."
    echo "  Investigate with: nm -j ${path} | head -40  and  cargo bloat"
    exit 1
  fi
}

echo ""
echo "Binary sizes:"
check_size "${DARWIN_ARM64_OUT}" "darwin-arm64"
check_size "${LINUX_X64_OUT}"    "linux-x64"
echo "  All binaries within 20 MB ceiling."

# ── Publish dry-run ────────────────────────────────────────────────────────────

echo ""
echo "[4/4] Publish dry-run for all three packages..."
echo ""

run_dry_run() {
  local dir="$1"
  local pkg="$2"
  echo "--- pnpm publish --dry-run: ${pkg} ---"
  cd "${dir}"
  # --no-git-checks: required when there are pre-existing uncommitted workspace files
  # (e.g. apps/docs-astro/public/llms-full.txt) that are unrelated to this package.
  # The dry-run still validates the tarball contents and registry access.
  pnpm publish --dry-run --no-git-checks 2>&1 | tail -30
  echo ""
  cd "${REPO_ROOT}"
}

run_dry_run "${REPO_ROOT}/packages/server-dist/npm/darwin-arm64" "@topgunbuild/server-darwin-arm64"
run_dry_run "${REPO_ROOT}/packages/server-dist/npm/linux-x64"    "@topgunbuild/server-linux-x64"
run_dry_run "${REPO_ROOT}/packages/server-dist"                   "@topgunbuild/server"

echo ""
echo "========================================================="
echo "All checks passed. Packages are publish-ready."
echo ""
echo "To publish (manual operator step):"
echo "  cd packages/server-dist/npm/darwin-arm64 && pnpm publish --no-git-checks"
echo "  cd packages/server-dist/npm/linux-x64    && pnpm publish --no-git-checks"
echo "  cd packages/server-dist                   && pnpm publish --no-git-checks"
echo ""
echo "Binaries placed at:"
echo "  ${DARWIN_ARM64_OUT}"
echo "  ${LINUX_X64_OUT}"
echo ""
