#!/usr/bin/env bash
# Captures flamegraphs for fire-and-forget and fire-and-wait workloads.
# Must be run as root or via sudo on macOS (dtrace requires elevated privileges).
#
# Usage:
#   sudo ./packages/server-rust/scripts/flamegraph-capture.sh
#
# Prerequisites:
#   cargo install flamegraph --version 0.6.0  (or later)

set -euo pipefail

# ---------------------------------------------------------------------------
# Prerequisite check: cargo-flamegraph must be installed (v0.6.0+)
# ---------------------------------------------------------------------------
if ! cargo flamegraph --version &>/dev/null; then
    echo "ERROR: cargo-flamegraph is not installed or not found in PATH." >&2
    echo "" >&2
    echo "Install it with:" >&2
    echo "  cargo install flamegraph" >&2
    echo "" >&2
    echo "Minimum required version: 0.6.0 (supports --profile and --bench flags)." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# macOS compatibility: SDKROOT required for the linker
# ---------------------------------------------------------------------------
export SDKROOT
SDKROOT=$(xcrun --sdk macosx --show-sdk-path)

# ---------------------------------------------------------------------------
# Output directory
# ---------------------------------------------------------------------------
PROFILING_DIR="packages/server-rust/docs/profiling"
mkdir -p "${PROFILING_DIR}"

SVG_FAF="${PROFILING_DIR}/flamegraph-fire-and-forget.svg"
SVG_FAW="${PROFILING_DIR}/flamegraph-fire-and-wait.svg"

# ---------------------------------------------------------------------------
# Helper: post-process collapsed stacks to produce a server-only SVG
# Filters to server-rt threads, which isolates server-side hot paths from
# client-side load harness overhead present in the merged flamegraph.
# ---------------------------------------------------------------------------
filter_server_only() {
    local primary_svg="$1"
    local server_only_svg="${primary_svg%.svg}-server-only.svg"

    # cargo flamegraph on macOS produces flamegraph.stacks in the CWD.
    # Fall back to globbing for *.stacks if that file is absent.
    local stacks_file=""
    if [ -f "flamegraph.stacks" ]; then
        stacks_file="flamegraph.stacks"
    else
        stacks_file=$(ls ./*.stacks 2>/dev/null | head -n 1 || true)
    fi

    if [ -z "${stacks_file}" ]; then
        echo "WARNING: No collapsed stacks file found; skipping server-only SVG generation." >&2
        return
    fi

    if ! command -v inferno-flamegraph &>/dev/null; then
        echo "WARNING: inferno-flamegraph is not installed; skipping server-only SVG." >&2
        echo "  Install: cargo install inferno" >&2
        return
    fi

    grep 'server-rt' "${stacks_file}" | inferno-flamegraph > "${server_only_svg}"
    echo "  Server-only SVG: ${server_only_svg}"
}

# ---------------------------------------------------------------------------
# Capture 1: fire-and-forget
# ---------------------------------------------------------------------------
echo "==> Capturing fire-and-forget flamegraph (60 seconds)..."
echo "    This requires sudo/root for dtrace on macOS."

cargo flamegraph --bench load_harness --profile release-with-debug \
    -o "${SVG_FAF}" \
    -- --connections 200 --interval 0 --duration 60 --fire-and-forget

echo "  Primary SVG: ${SVG_FAF}"
filter_server_only "${SVG_FAF}"

# ---------------------------------------------------------------------------
# Capture 2: fire-and-wait
# ---------------------------------------------------------------------------
echo "==> Capturing fire-and-wait flamegraph (60 seconds)..."

cargo flamegraph --bench load_harness --profile release-with-debug \
    -o "${SVG_FAW}" \
    -- --connections 200 --interval 0 --duration 60

echo "  Primary SVG: ${SVG_FAW}"
filter_server_only "${SVG_FAW}"

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
echo ""
echo "Flamegraph capture complete."
echo "  Fire-and-forget: ${SVG_FAF}"
echo "  Fire-and-wait:   ${SVG_FAW}"
echo ""
echo "Open each SVG in a browser to inspect hot paths."
echo "Use the -server-only variants for server-side analysis (server-rt threads only)."
echo "Fill in docs/profiling/FLAMEGRAPH_ANALYSIS.md with findings."
