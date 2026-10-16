#!/usr/bin/env bash
# check-server-binary-size.sh <binary> [label]
#
# Prints the size of a topgun-server binary in bytes and MiB, and exits 1 when it
# exceeds MAX_SIZE_BYTES. The ceiling lives only here: the release build script
# (scripts/build-server-binaries.sh) and CI (.github/workflows/integration-rust.yml)
# both call this script, so changing the number changes every check at once.

set -euo pipefail

MAX_SIZE_BYTES=$((20 * 1024 * 1024))  # 20 MiB

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 <binary> [label]" >&2
  exit 2
fi

path="$1"
label="${2:-$1}"

if [ ! -f "${path}" ]; then
  echo "ERROR: ${label}: no file at ${path}" >&2
  exit 2
fi

size=$(wc -c < "${path}" | tr -d ' ')
# LC_ALL=C keeps the decimal point a dot under comma-decimal locales.
mib=$(LC_ALL=C awk -v s="${size}" 'BEGIN { printf "%.2f", s / 1048576 }')
max_mib=$(LC_ALL=C awk -v s="${MAX_SIZE_BYTES}" 'BEGIN { printf "%.2f", s / 1048576 }')

echo "  ${label}: ${size} bytes (${mib} MiB); ceiling ${MAX_SIZE_BYTES} bytes (${max_mib} MiB)"

if [ "${size}" -gt "${MAX_SIZE_BYTES}" ]; then
  echo "ERROR: ${label} binary exceeds the size ceiling by $((size - MAX_SIZE_BYTES)) bytes." >&2
  echo "  Investigate with: cargo bloat --release --bin topgun-server --crates" >&2
  exit 1
fi
