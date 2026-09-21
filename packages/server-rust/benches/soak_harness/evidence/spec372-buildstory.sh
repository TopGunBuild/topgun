#!/usr/bin/env bash
#
# spec372 build story: a literal result, per allocator arm, for every place the
# server binary is built, plus the compile-time proof that the allocator cfg
# lattice is total. Writes spec372-buildstory.txt.
#
#   LATTICE  cargo check of the server binary and the load harness under each
#            of the 16 subsets of {count-alloc, dhat-heap, alloc-jemalloc,
#            alloc-mimalloc}. ALLOCATOR_NAME is declared once per lattice row,
#            so an overlap fails as a duplicate and a gap fails to resolve:
#            16 exit codes of 0 prove EXACTLY one allocator per subset.
#   ITEM 1   darwin-arm64 host build: cargo build --release [--features <arm>]
#   ITEM 2   linux-x64, the npm path: cargo zigbuild --release [--features <arm>]
#            --target x86_64-unknown-linux-gnu, SYS included, so every size is
#            measured. SIZE_<arm> = arm - SYS on linux-x64 (bytes, stripped by
#            the release profile); SIZE_HEADROOM_<arm> = TIGHT within 1 MiB of
#            the 20 MiB ceiling scripts/build-server-binaries.sh enforces.
#   ITEM 3   docker build of deploy/Dockerfile.server with the feature on: a
#            derived copy in a scratch dir whose one cargo line gains the
#            feature. The committed Dockerfile is never edited. The build
#            context is a scratch copy of exactly what the Dockerfile COPYs
#            (plus the repo's .dockerignore): the repo root's target/ is not
#            dockerignored, so `docker build .` would ship all of it. Built for
#            the host's native platform; the platform is recorded.
#   ITEM 4   clippy -D warnings: --all-targets --all-features (what CI runs,
#            under which count-alloc wins the lattice), and --all-targets
#            --features <arm> for each arm (the blocks CI never compiles).
#   ITEM 5   cargo audit --ignore RUSTSEC-2023-0071 (CI's exact command).
#   ITEM 6   the CI cost, recorded: cold `cargo clippy --all-targets
#            --all-features -- -D warnings` at the pin and at HEAD, each in its
#            own fresh target dir. CI's test job runs `cargo test --all-targets`
#            on default features, which never resolves the allocator crates.
#
# BUILD_<arm>: OK iff items 1, 2 and 3 build and items 4 and 5 exit 0; FAIL
# iff item 1 fails; PARTIAL otherwise. Item 6 never moves it.
#
# SPEC372_SMOKE=1 writes into SPEC365_OUT_DIR (a scratch dir); same work.
set -uo pipefail
export LC_ALL=C

PIN=f59a61ec
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
SMOKE="${SPEC372_SMOKE:-0}"
if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  [ -n "$OUT" ] || { echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  [ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; }
else
  OUT="$SCRIPT_DIR"
fi
TXT="$OUT/spec372-buildstory.txt"
RAW="$OUT/.spec372-buildstory-raw"; rm -rf "$RAW"; mkdir -p "$RAW"
: > "$TXT"
say() { echo "$*" | tee -a "$TXT"; }
if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi
T_ROOT="${REPO_ROOT}/target"
say "buildstory start: $(date -u +%Y-%m-%dT%H:%M:%SZ) smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) pin=${PIN}"
say "toolchain: $(rustc --version) / $(cargo --version) / $(cargo-zigbuild --version 2>/dev/null | head -1) / zig $(zig version 2>/dev/null)"

timed() {   # $1 = label, $2 = log; runs the rest; prints "<rc> <seconds>"
  local s rc; s="$(date +%s)"; shift 2
  "$@"; rc=$?
  echo "$rc $(( $(date +%s) - s ))"
}
feat() { case "$1" in SYS) echo "" ;; JE) echo "alloc-jemalloc" ;; MI) echo "alloc-mimalloc" ;; esac; }
fargs() { [ -n "$(feat "$1")" ] && echo "--features $(feat "$1")"; }

# ----------------------------------------------------------------- LATTICE
LAT_OK=1
for mask in $(seq 0 15); do
  fs=""
  [ $((mask & 1)) -ne 0 ] && fs="${fs},count-alloc"
  [ $((mask & 2)) -ne 0 ] && fs="${fs},dhat-heap"
  [ $((mask & 4)) -ne 0 ] && fs="${fs},alloc-jemalloc"
  [ $((mask & 8)) -ne 0 ] && fs="${fs},alloc-mimalloc"
  fs="${fs#,}"
  if [ -n "$fs" ]; then fa=(--features "$fs"); else fa=(); fi
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$T_ROOT/spec372-bs-lattice" cargo check --bin topgun-server --bench load_harness ${fa[@]+"${fa[@]}"} ) > "$RAW/lattice-$mask.log" 2>&1
  rc=$?; [ "$rc" -eq 0 ] || LAT_OK=0
  say "LATTICE subset=${mask} features=[${fs}] check_rc=${rc}"
done
say "LATTICE_TOTAL=$([ "$LAT_OK" -eq 1 ] && echo TRUE || echo FALSE) subsets=16"

# ----------------------------------------------------------------- ITEM 1 + ITEM 2
for arm in SYS JE MI; do
  # shellcheck disable=SC2046
  r="$(timed item1 x bash -c "cd '$SERVER_ROOT' && CARGO_TARGET_DIR='$T_ROOT/spec372-bs-darwin-$arm' cargo build --release $(fargs $arm) --bin topgun-server > '$RAW/item1-$arm.log' 2>&1")"
  b="$T_ROOT/spec372-bs-darwin-$arm/release/topgun-server"
  sz="$( [ -f "$b" ] && wc -c < "$b" | tr -d ' ' || echo n/a)"
  say "ITEM1 arm=${arm} target=darwin-arm64 rc=${r% *} wall_s=${r#* } size_bytes=${sz}"
  r="$(timed item2 x bash -c "cd '$REPO_ROOT' && CARGO_TARGET_DIR='$T_ROOT/spec372-bs-linux-$arm' cargo zigbuild --release -p topgun-server $(fargs $arm) --bin topgun-server --target x86_64-unknown-linux-gnu > '$RAW/item2-$arm.log' 2>&1")"
  b="$T_ROOT/spec372-bs-linux-$arm/x86_64-unknown-linux-gnu/release/topgun-server"
  sz="$( [ -f "$b" ] && wc -c < "$b" | tr -d ' ' || echo n/a)"
  say "ITEM2 arm=${arm} target=linux-x64 rc=${r% *} wall_s=${r#* } size_bytes=${sz} file=$( [ -f "$b" ] && file -b "$b" | cut -d, -f1-2 || echo n/a)"
  [ "${r% *}" -ne 0 ] && tail -30 "$RAW/item2-$arm.log" | sed "s/^/ITEM2-LOG arm=${arm}: /" >> "$TXT"
done

# ----------------------------------------------------------------- ITEM 3
DOCKER_UP=0
if docker info >/dev/null 2>&1; then DOCKER_UP=1
else
  open -a Docker >/dev/null 2>&1 || true
  for _ in $(seq 1 36); do sleep 5; if docker info >/dev/null 2>&1; then DOCKER_UP=1; break; fi; done
fi
say "docker daemon: $([ "$DOCKER_UP" -eq 1 ] && docker info --format '{{.ServerVersion}} {{.OSType}}/{{.Architecture}}' || echo unavailable)"
CTX="$RAW/docker-context"
mkdir -p "$CTX/packages"
cp "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/Cargo.lock" "$REPO_ROOT/.dockerignore" "$CTX/"
for pkg in core-rust server-rust; do
  rsync -a --exclude target --exclude node_modules "$REPO_ROOT/packages/$pkg/" "$CTX/packages/$pkg/"
done
say "docker context: $(du -sh "$CTX" | cut -f1) (Cargo.toml, Cargo.lock, .dockerignore, packages/core-rust, packages/server-rust)"
for arm in JE MI; do
  if [ "$DOCKER_UP" -ne 1 ]; then say "ITEM3 arm=${arm} rc=n/a reason=docker_daemon_unavailable"; continue; fi
  df="$RAW/Dockerfile.$arm"
  awk -v f="$(feat $arm)" '/^RUN cargo build --release --bin topgun-server$/ { print "RUN cargo build --release --features " f " --bin topgun-server"; n++; next } { print } END { if (n != 1) exit 3 }' \
      "$REPO_ROOT/deploy/Dockerfile.server" > "$df" || { say "ITEM3 arm=${arm} rc=n/a reason=cargo_line_not_found_exactly_once"; continue; }
  r="$(timed item3 x bash -c "cd '$CTX' && docker build --no-cache -f '$df' -t topgun-spec372-$(echo $arm | tr A-Z a-z) . > '$RAW/item3-$arm.log' 2>&1")"
  say "ITEM3 arm=${arm} rc=${r% *} wall_s=${r#* } platform=$(docker info --format '{{.OSType}}/{{.Architecture}}') new_apt_packages=none (derived Dockerfile differs from the committed one only in its cargo line)"
  [ "${r% *}" -ne 0 ] && tail -30 "$RAW/item3-$arm.log" | sed "s/^/ITEM3-LOG arm=${arm}: /" >> "$TXT"
done

# ----------------------------------------------------------------- ITEM 4 + ITEM 6
rm -rf "$T_ROOT/spec372-bs-ci-head" "$T_ROOT/spec372-bs-ci-pin" "$RAW/pin-worktree"
r="$(timed item4 x bash -c "cd '$REPO_ROOT' && CARGO_TARGET_DIR='$T_ROOT/spec372-bs-ci-head' cargo clippy --all-targets --all-features -- -D warnings > '$RAW/item4-all.log' 2>&1")"
CLIPPY_ALL_RC="${r% *}"; HEAD_S="${r#* }"
say "ITEM4 clippy=all-features rc=${CLIPPY_ALL_RC} cold_wall_s=${HEAD_S}"
for arm in JE MI; do
  r="$(timed item4 x bash -c "cd '$REPO_ROOT' && CARGO_TARGET_DIR='$T_ROOT/spec372-bs-ci-head' cargo clippy --all-targets -p topgun-server --features $(feat $arm) -- -D warnings > '$RAW/item4-$arm.log' 2>&1")"
  say "ITEM4 clippy=${arm} rc=${r% *} wall_s=${r#* }"
done
git -C "$REPO_ROOT" worktree add --detach "$RAW/pin-worktree" "$PIN" > "$RAW/worktree.log" 2>&1
r="$(timed item6 x bash -c "cd '$RAW/pin-worktree' && CARGO_TARGET_DIR='$T_ROOT/spec372-bs-ci-pin' cargo clippy --all-targets --all-features -- -D warnings > '$RAW/item6-pin.log' 2>&1")"
PIN_S="${r#* }"
say "ITEM6 clippy_all_features_cold pin_rc=${r% *} pin_wall_s=${PIN_S} head_wall_s=${HEAD_S} delta_s=$(( HEAD_S - PIN_S ))"
say "ITEM6 ci_test_job=cargo test --all-targets (default features; does not resolve the allocator crates)"
git -C "$REPO_ROOT" worktree remove --force "$RAW/pin-worktree" >> "$RAW/worktree.log" 2>&1

# ----------------------------------------------------------------- ITEM 5
( cd "$REPO_ROOT" && cargo audit --ignore RUSTSEC-2023-0071 ) > "$RAW/item5.log" 2>&1
AUDIT_RC=$?
say "ITEM5 cargo_audit rc=${AUDIT_RC} $(grep -E '^(error|warning):' "$RAW/item5.log" | head -3 | tr '\n' ' ')"
# Which advisories the allocator crates appear under, and whether any advisory
# names one of them as its own crate.
awk '/^Crate:/ { c = $2; id = "none" } /^ID:/ { id = $2 } /^Warning:/ && id == "none" { id = "warning:" $2 }
     /^Crate:/ && $2 ~ /jemalloc|mimalloc/ { own[$2] = 1 }
     /jemalloc|mimalloc/ && !/^Crate:/ { k = c " " id; if (!(k in seen)) { seen[k] = 1; n++; print "ITEM5-TREE advisory_crate=" c " id=" id " (an allocator crate appears in its dependency tree)" } }
     END { m = 0; for (k in own) { m++; print "ITEM5-OWN crate=" k } print "ITEM5-ADVISORIES_NAMING_AN_ALLOCATOR_CRATE=" m }' "$RAW/item5.log" >> "$TXT"

# ----------------------------------------------------------------- per-arm results
awk -v all_rc="$CLIPPY_ALL_RC" -v audit_rc="$AUDIT_RC" '
  { delete f; for (i = 2; i <= NF; i++) { p = index($i, "="); if (p) f[substr($i, 1, p - 1)] = substr($i, p + 1) } }
  $1 == "ITEM1" { i1[f["arm"]] = f["rc"]; d[f["arm"]] = f["size_bytes"] }
  $1 == "ITEM2" { i2[f["arm"]] = f["rc"]; l[f["arm"]] = f["size_bytes"] }
  $1 == "ITEM3" { i3[f["arm"]] = f["rc"] }
  $1 == "ITEM4" && f["clippy"] != "all-features" { i4[f["clippy"]] = f["rc"] }
  END {
    ceiling = 20 * 1048576
    for (a = 1; a <= 2; a++) {
      arm = (a == 1) ? "JE" : "MI"
      if (l[arm] ~ /^[0-9]+$/ && l["SYS"] ~ /^[0-9]+$/) {
        print "SIZE_" arm "=" (l[arm] - l["SYS"]) " linux_x64_bytes=" l[arm] " sys_linux_x64_bytes=" l["SYS"] " darwin_delta_bytes=" ((d[arm] ~ /^[0-9]+$/ && d["SYS"] ~ /^[0-9]+$/) ? d[arm] - d["SYS"] : "n/a")
        print "SIZE_HEADROOM_" arm "=" ((l[arm] > ceiling - 1048576) ? "TIGHT" : "OK") " headroom_bytes=" (ceiling - l[arm])
      } else { print "SIZE_" arm "=n/a reason=linux_build_missing"; print "SIZE_HEADROOM_" arm "=n/a reason=linux_build_missing" }
      if (i1[arm] != "0") b = "FAIL"
      else if (i2[arm] == "0" && i3[arm] == "0" && all_rc == "0" && i4[arm] == "0" && audit_rc == "0") b = "OK"
      else b = "PARTIAL"
      print "BUILD_" arm "=" b " item1=" i1[arm] " item2=" i2[arm] " item3=" i3[arm] " item4_all=" all_rc " item4_arm=" i4[arm] " item5=" audit_rc
    }
  }' "$TXT" > "$RAW/verdict.txt"
tee -a "$TXT" < "$RAW/verdict.txt"
say "buildstory end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
