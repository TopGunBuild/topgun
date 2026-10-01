#!/usr/bin/env bash
#
# Host preflight for the Linux allocator series (topgun-bench) -- a copy of
# spec376-preflight.sh, which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT CANNOT SEE AN ALLOCATOR TREATMENT CHANGE
# THAT LIVES ON THE HOST: a /etc/_rjem_malloc.conf symlink configures this
# build's _rjem_-prefixed jemalloc, /etc/malloc.conf an unprefixed one, and
# /etc/ld.so.preload interposes a different allocator into every arm -- none
# of them visible in any env var the cells prove. The difference list against
# spec376-preflight.sh is CLOSED at exactly four items:
#
#   1. THE ALLOC_CONF ROW (a gate). /etc/_rjem_malloc.conf and
#      /etc/malloc.conf exist neither as a file nor as a symlink, and
#      /etc/ld.so.preload is absent or blank; an unreadable /etc is FAIL,
#      never "absent". Plus the jemalloc archive the JE build linked
#      (libjemalloc_pic.a under target/spec377-JE-ser, exactly one): `nm`
#      must read it (rc 0, non-empty), must show jemalloc's own WEAK default
#      for _rjem_malloc_conf (the positive control that nm sees the symbol at
#      all) and no other definition of it (a strong one would replace the
#      empty default). The release binary itself is stripped, so the
#      archive is what nm can read; the runtime confirm_conf printout of the
#      JE cells is the proof for the linked whole. Before the builds exist
#      (no target/spec377-run/spec377-builds.txt) the archive part cannot be
#      read: the row prints PENDING, which is not PASS -- the last line
#      then carries pending=alloc_conf and the series chain refuses it.
#   2. RECORDED, not gates: every /proc/sys/vm/overcommit_* value (THP stays
#      a gate); `nm` joins the required tools.
#   3. NAMES: the log is spec377-preflight-<stamp>.log and the test hooks are
#      SPEC377_PREFLIGHT_* (plus SPEC377_PREFLIGHT_T_ROOT, the target dir the
#      builds file and the JE archive are read from); the last line may carry
#      pending=<list> between PREFLIGHT=PASS and PREFLIGHT_AT=.
#   4. This header, the usage text and the messages.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE FOUR ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec376-preflight.sh spec377-preflight.sh).
#
# Everything from here on is spec376-preflight.sh's own header, adjusted only
# for the names above.
#
# Host preflight for the Linux bench host (topgun-bench). Writes one sidecar
# log, spec377-preflight-<YYYYMMDDTHHMMSSZ>.log, holding every command and its
# output, whose LAST line is exactly one of
#   PREFLIGHT=PASS [pending=<comma list>] PREFLIGHT_AT=<YYYY-MM-DDTHH:MM:SSZ>
#   PREFLIGHT=FAIL failed=<comma list> PREFLIGHT_AT=<YYYY-MM-DDTHH:MM:SSZ>
# and nothing after it. The series chain accepts the lexicographically newest
# log (the stamp sorts in time order) only if that line is PASS with no
# pending=, at most 60 min old, and the log carries CHECK alloc_conf=PASS.
# Exit 0 on PASS, 1 on FAIL, 2 on the Linux-only guard or usage.
#
# usage: spec377-preflight.sh [--apply]
#   without --apply: checks only.
#   --apply: after identity and isolation PASS, stops the listed timers and
#   unattended-upgrades and writes madvise to both THP knobs, then checks.
#
# ORDER IS THE SAFETY PROPERTY. The Linux-only guard runs before any file is
# created. Then identity (hostname topgun-bench, 4 CPUs) and isolation (no
# dokploy / traefik / dockerd process or systemd unit; ports 8080 and 47376
# not listening) run BEFORE anything is changed: on the wrong host -- the demo
# box above all -- --apply stops nothing and writes nothing, logs the refusal
# and exits. Every state change goes through mutate(), the only place a
# command that changes the host is issued.
#
# Environment (test hooks; the bench host sets none of them):
#   SPEC377_PREFLIGHT_SYS_ROOT  prefix for the host files this script reads or
#                               writes under /sys (THP knobs), /etc (the
#                               alloc_conf files) and the systemd unit
#                               directories; default empty = the real /.
#   SPEC377_PREFLIGHT_CMDLOG    if set, mutate() appends each command to this
#                               file before running it.
#   SPEC377_PREFLIGHT_LOG_DIR   where the log is written; default the evidence
#                               dir (this script's dir), so the series-phase
#                               log travels with the artifacts.
#   SPEC377_PREFLIGHT_T_ROOT    the target dir holding spec377-run/ and
#                               spec377-JE-ser/; default <repo>/target.
#
# Every check follows the absence rule: a command that fails, prints nothing,
# or prints a value that does not parse is a FAIL named in failed=, never a
# pass.
set -uo pipefail
export LC_ALL=C

# The checks below read /proc, systemd and sysfs; on any other OS they would
# fail or, worse, read something else. Refuse before any file exists.
if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "FATAL: spec377-preflight.sh runs on Linux only (uname -s = '$(uname -s 2>/dev/null || true)')" >&2
  exit 2
fi

APPLY=0
case "$#:${1:-}" in
  0:) ;;
  1:--apply) APPLY=1 ;;
  *) echo "usage: spec377-preflight.sh [--apply]" >&2; exit 2 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="${SPEC377_PREFLIGHT_SYS_ROOT:-}"
CMDLOG="${SPEC377_PREFLIGHT_CMDLOG:-}"
LOG_DIR="${SPEC377_PREFLIGHT_LOG_DIR:-$SCRIPT_DIR}"
T_ROOT="${SPEC377_PREFLIGHT_T_ROOT:-$(cd "$SCRIPT_DIR/../../../../.." && pwd -P)/target}"
THP_DIR="${ROOT}/sys/kernel/mm/transparent_hugepage"
TIMERS="apt-daily.timer apt-daily-upgrade.timer man-db.timer e2scrub_all.timer fstrim.timer unattended-upgrades.service"
TOOLS="curl strings nm shasum sha256sum pgrep ss original-awk python3 git cargo rustc"
MIN_AVAIL_KB=14680064    # 14 GiB
MIN_DISK_KB=41943040     # 40 GiB

[ -d "$LOG_DIR" ] || { echo "FATAL: log dir ${LOG_DIR} does not exist" >&2; exit 2; }
LOG="$LOG_DIR/spec377-preflight-$(date -u +%Y%m%dT%H%M%SZ).log"
# A second run in the same second must not overwrite the first one's log.
( set -C; : > "$LOG" ) 2>/dev/null || { echo "FATAL: ${LOG} already exists; re-run in a second" >&2; exit 2; }

# Logging uses redirection only (never tee): tee is a mutating command here
# (the THP write), and a test that records every tee call must see none
# unless the host is being changed.
log() { printf '%s\n' "$*" >> "$LOG"; }
# Records a read-only command and its output; returns the command's status.
rec() {
  local rc
  log "\$ $*"
  "$@" >> "$LOG" 2>&1
  rc=$?
  log "  (rc=${rc})"
  return "$rc"
}
# The only path by which this script changes the host.
mutate() {
  [ -n "$CMDLOG" ] && printf '%s\n' "$*" >> "$CMDLOG"
  log "MUTATE: $*"
  "$@" >> "$LOG" 2>&1
}
FAILED=""
PENDING=""
check() {   # $1 = name, $2 = PASS|FAIL|PENDING, $3 = detail
  log "CHECK $1=$2 $3"
  echo "CHECK $1=$2 $3"
  case "$2" in
    PASS) ;;
    # Not failed and not passed: named on the last line, and never read as
    # PASS by the series chain.
    PENDING) PENDING="${PENDING:+${PENDING},}$1" ;;
    *) FAILED="${FAILED:+${FAILED},}$1" ;;
  esac
}
finish() {
  local line
  if [ -z "$FAILED" ]; then
    line="PREFLIGHT=PASS${PENDING:+ pending=${PENDING}} PREFLIGHT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  else
    line="PREFLIGHT=FAIL failed=${FAILED} PREFLIGHT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  fi
  log "$line"
  echo "log: ${LOG}"
  echo "$line"
  [ -z "$FAILED" ]
  exit $?
}
is_uint() { case "$1" in ''|*[!0-9]*) return 1 ;; *) return 0 ;; esac; }
# The bracketed value of a THP knob, e.g. "always [madvise] never" -> madvise;
# empty if the file is unreadable or carries no single bracket.
thp_value() {
  [ -r "$1" ] || return 0
  sed -n 's/.*\[\([a-z+]*\)\].*/\1/p' "$1" | head -1
}

log "== spec377-preflight start $(date -u +%Y-%m-%dT%H:%M:%SZ) apply=${APPLY} sys_root='${ROOT}' t_root='${T_ROOT}' script=${SCRIPT_DIR}/spec377-preflight.sh"

# ------------------------------------------------ identity (before any change)
log "== identity"
H="$(hostname 2>/dev/null)"; log "\$ hostname -> '${H}'"
N="$(nproc 2>/dev/null)";    log "\$ nproc -> '${N}'"
if [ "$H" = "topgun-bench" ] && [ "$N" = "4" ]; then
  check identity PASS "hostname=${H} nproc=${N}"
else
  check identity FAIL "hostname=${H:-absent} nproc=${N:-absent} (need topgun-bench / 4)"
fi

# ----------------------------------------------- isolation (before any change)
log "== isolation"
ISO=""
PROCS="$(pgrep -f -- 'dokploy|traefik|dockerd' 2>/dev/null)"; PRC=$?
log "\$ pgrep -f 'dokploy|traefik|dockerd' -> rc=${PRC} pids='$(printf '%s' "$PROCS" | tr '\n' ' ')'"
case "$PRC" in
  0) ISO="${ISO} processes=$(printf '%s' "$PROCS" | tr '\n' '/')"
     for p in $PROCS; do log "  $(ps -o pid=,args= -p "$p" 2>/dev/null)"; done ;;
  1) ;;
  *) ISO="${ISO} pgrep_rc=${PRC}" ;;
esac
# Unit files are read from the unit directories, not through systemctl, so
# that this check stays a pure read under every test stub. At least one
# directory must exist: a host with none cannot be shown to have no units.
UNIT_DIRS=0; UNITS=""
for d in "${ROOT}/etc/systemd/system" "${ROOT}/lib/systemd/system" "${ROOT}/usr/lib/systemd/system" "${ROOT}/run/systemd/system"; do
  [ -d "$d" ] || continue
  UNIT_DIRS=$((UNIT_DIRS + 1))
  UNITS="${UNITS}$(find "$d" -maxdepth 2 \( -name 'dokploy*' -o -name 'traefik*' -o -name 'docker*' \) 2>/dev/null)"
done
log "unit dirs present=${UNIT_DIRS}; matching units='$(printf '%s' "$UNITS" | tr '\n' ' ')'"
[ "$UNIT_DIRS" -gt 0 ] || ISO="${ISO} unit_dirs=absent"
[ -z "$UNITS" ] || ISO="${ISO} units=$(printf '%s' "$UNITS" | tr '\n' '/')"
if command -v ss >/dev/null 2>&1; then
  LST="$(ss -Hltn 2>&1)"; SRC=$?
  log "\$ ss -Hltn (rc=${SRC})"; printf '%s\n' "$LST" | sed 's/^/  /' >> "$LOG"
  if [ "$SRC" -ne 0 ]; then
    ISO="${ISO} ss_rc=${SRC}"
  else
    PORTS="$(printf '%s\n' "$LST" | awk '{ n = split($4, a, ":"); if (a[n] == "8080" || a[n] == "47376") print a[n] }' | sort -u | tr '\n' '/')"
    [ -z "$PORTS" ] || ISO="${ISO} listening=${PORTS}"
  fi
else
  ISO="${ISO} ss=absent"
fi
if [ -z "$ISO" ]; then check isolation PASS "no dokploy/traefik/dockerd process or unit; 8080 and 47376 free"
else check isolation FAIL "${ISO# }"; fi

if [ -n "$FAILED" ]; then
  log "REFUSED: identity/isolation failed; nothing was changed and no further check ran"
  echo "refused: identity/isolation failed; nothing changed"
  finish
fi

# ------------------------------------------------------------------ --apply
if [ "$APPLY" -eq 1 ]; then
  log "== apply"
  # shellcheck disable=SC2086 # the unit list is a fixed literal
  mutate systemctl stop $TIMERS
  for k in enabled defrag; do
    log "THP ${k} before: '$(cat "$THP_DIR/$k" 2>&1)'"
    printf 'madvise\n' | mutate tee "$THP_DIR/$k"
    log "THP ${k} after:  '$(cat "$THP_DIR/$k" 2>&1)'"
  done
fi

# --------------------------------------------------------------------- os
log "== os"
OS="$(uname -s)"
VID="$(sed -n 's/^VERSION_ID="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release 2>/dev/null)"
rec cat /etc/os-release
if [ "$OS" = "Linux" ] && [ "$VID" = "12" ]; then check os PASS "uname=${OS} VERSION_ID=${VID}"
else check os FAIL "uname=${OS} VERSION_ID=${VID:-absent}"; fi

# ----------------------------------------------------------------- timers
log "== timers"
rec systemctl list-timers --all --no-pager
BAD=""
for u in $TIMERS; do
  s="$(systemctl is-active "$u" 2>/dev/null)"
  log "\$ systemctl is-active ${u} -> '${s}'"
  [ "$s" = "inactive" ] || BAD="${BAD}${BAD:+,}${u}=${s:-absent}"
done
if [ -z "$BAD" ]; then check timers PASS "all inactive: ${TIMERS}"; else check timers FAIL "$BAD"; fi

# ------------------------------------------------------------------- load
log "== load"
load1() { awk 'NR == 1 && $1 ~ /^[0-9]+(\.[0-9]+)?$/ { print $1 }' /proc/loadavg 2>/dev/null; }
below() { awk -v v="$1" 'BEGIN { exit !(v + 0 < 0.5) }'; }
POLLS=1; [ "$APPLY" -eq 1 ] && POLLS=21   # --apply: up to 10 min, every 30 s
i=1; L=""; LOK=0
while [ "$i" -le "$POLLS" ]; do
  L="$(load1)"
  log "poll ${i}: /proc/loadavg = '$(cat /proc/loadavg 2>&1)'"
  if [ -n "$L" ] && below "$L"; then LOK=1; break; fi
  [ "$i" -lt "$POLLS" ] && sleep 30
  i=$((i + 1))
done
if [ "$LOK" -eq 1 ]; then check load PASS "load1=${L} < 0.5 (poll ${i})"
else check load FAIL "load1=${L:-absent} after ${POLLS} poll(s)"; fi

# ------------------------------------------------------------------ steal
log "== steal"
# cpu user nice system idle iowait irq softirq steal: the total is the sum of
# fields 2..9 (guest time is already inside user).
cpu_ticks() { awk '$1 == "cpu" && NF >= 9 { t = 0; for (i = 2; i <= 9; i++) t += $i; printf "%d %d\n", $9, t; n++ } END { if (n != 1) exit 1 }' /proc/stat 2>/dev/null; }
S0="$(cpu_ticks)"; sleep 10; S1="$(cpu_ticks)"
log "/proc/stat steal,total: t0='${S0}' t1='${S1}'"
ST="$(printf '%s %s\n' "$S0" "$S1" | awk 'NF == 4 && $4 > $2 { printf "%.4f\n", ($3 - $1) * 100 / ($4 - $2) }')"
if [ -n "$ST" ] && awk -v v="$ST" 'BEGIN { exit !(v + 0 < 1) }'; then check steal PASS "steal_pct=${ST} < 1 over 10 s"
else check steal FAIL "steal_pct=${ST:-absent}"; fi

# ------------------------------------------------------------------- swap
log "== swap"
SW="$(swapon --show 2>&1)"; SWRC=$?
log "\$ swapon --show (rc=${SWRC}) -> '${SW}'"
STOT="$(awk '$1 == "SwapTotal:" { n++; v = $2 " " $3 } END { if (n == 1) print v }' /proc/meminfo 2>/dev/null)"
log "SwapTotal: ${STOT:-absent}"
if [ "$SWRC" -eq 0 ] && [ -z "$SW" ] && [ "$STOT" = "0 kB" ]; then check swap PASS "swapon empty, SwapTotal: 0 kB"
else check swap FAIL "swapon_rc=${SWRC} swapon='$(printf '%s' "$SW" | tr '\n' ' ')' SwapTotal=${STOT:-absent}"; fi

# -------------------------------------------------------------------- THP
log "== thp"
TE="$(thp_value "$THP_DIR/enabled")"; TD="$(thp_value "$THP_DIR/defrag")"
log "THP enabled: '$(cat "$THP_DIR/enabled" 2>&1)'"
log "THP defrag:  '$(cat "$THP_DIR/defrag" 2>&1)'"
if [ "$TE" = "madvise" ] && [ "$TD" = "madvise" ]; then check thp PASS "enabled=[madvise] defrag=[madvise]"
else check thp FAIL "enabled=${TE:-absent} defrag=${TD:-absent}"; fi

# ----------------------------------------------------------------- memory
log "== memory"
rec head -8 /proc/meminfo
MA="$(awk '$1 == "MemAvailable:" { n++; v = $2 } END { if (n == 1) print v }' /proc/meminfo 2>/dev/null)"
if is_uint "$MA" && [ "$MA" -ge "$MIN_AVAIL_KB" ]; then check memory PASS "MemAvailable=${MA} kB >= ${MIN_AVAIL_KB}"
else check memory FAIL "MemAvailable=${MA:-absent} kB (need >= ${MIN_AVAIL_KB})"; fi

# ------------------------------------------------------------------- disk
log "== disk"
rec findmnt -no FSTYPE,OPTIONS --target /opt
DF="$(df -Pk /opt 2>/dev/null | awk 'NR == 2 { print $4 }')"
log "\$ df -Pk /opt -> available '${DF}' KiB"
if is_uint "$DF" && [ "$DF" -ge "$MIN_DISK_KB" ]; then check disk PASS "/opt free=${DF} KiB >= ${MIN_DISK_KB}"
else check disk FAIL "/opt free=${DF:-absent} KiB (need >= ${MIN_DISK_KB})"; fi

# ------------------------------------------------------------------ clock
log "== clock"
NTP="$(timedatectl show -p NTPSynchronized 2>/dev/null)"
log "\$ timedatectl show -p NTPSynchronized -> '${NTP}'"
if [ "$NTP" = "NTPSynchronized=yes" ]; then check clock PASS "$NTP"
else check clock FAIL "ntp='${NTP:-absent}'"; fi

# -------------------------------------------------------------- alloc_conf
# Item 1: host-level allocator configuration that no cell env shows. Each
# sub-result is logged; the row passes only if every one of them passes.
log "== alloc_conf"
ETC="${ROOT}/etc"
AC_BAD=""; AC_PENDING=""
ac_bad() { AC_BAD="${AC_BAD:+${AC_BAD},}$1"; }
# "Does not exist" is only a reading when the directory can be listed and
# searched; otherwise every -e test is false for the wrong reason.
if [ -d "$ETC" ] && [ -r "$ETC" ] && [ -x "$ETC" ]; then
  for f in _rjem_malloc.conf malloc.conf; do
    if [ -e "$ETC/$f" ] || [ -L "$ETC/$f" ]; then
      log "/etc/${f}: PRESENT -> '$(ls -l "$ETC/$f" 2>&1)'"
      ac_bad "${f}=present"
    else
      log "/etc/${f}: absent (neither file nor symlink)"
    fi
  done
  P="$ETC/ld.so.preload"
  if [ -e "$P" ] || [ -L "$P" ]; then
    if [ -f "$P" ] && [ -r "$P" ]; then
      NB="$(grep -c '[^[:space:]]' "$P" 2>/dev/null)"; NB="${NB:-unreadable}"
      log "/etc/ld.so.preload: present, non-blank lines=${NB}"
      [ "$NB" = "0" ] || ac_bad "ld.so.preload=non_blank(${NB})"
    else
      log "/etc/ld.so.preload: present but not a readable regular file -> '$(ls -l "$P" 2>&1)'"
      ac_bad "ld.so.preload=unreadable"
    fi
  else
    log "/etc/ld.so.preload: absent"
  fi
else
  log "/etc (${ETC}) cannot be listed and searched"
  ac_bad "etc=unreadable"
fi
# The archive the JE build linked. Its _rjem_malloc_conf default is WEAK in
# jemalloc's own source, so seeing that weak definition is the control that
# nm read the right symbol; any other definition would override the default.
if [ -f "$T_ROOT/spec377-run/spec377-builds.txt" ]; then
  ARCH="$(ls "$T_ROOT"/spec377-JE-ser/release/build/tikv-jemalloc-sys-*/out/lib/libjemalloc_pic.a 2>/dev/null)"
  NA="$(printf '%s\n' "$ARCH" | grep -c .)"
  log "JE archive candidates (${NA}): $(printf '%s' "$ARCH" | tr '\n' ' ')"
  if [ "$NA" != "1" ]; then
    ac_bad "je_archive=$( [ "$NA" = "0" ] && echo absent || echo "dup(${NA})")"
  else
    NME="$(mktemp "${TMPDIR:-/tmp}/spec377-nm.XXXXXX")"
    NMO="$(nm "$ARCH" 2>"$NME")"; NRC=$?
    log "\$ nm ${ARCH} (rc=${NRC}, $(printf '%s\n' "$NMO" | grep -c .) lines; stderr: '$(tr '\n' ' ' < "$NME" 2>/dev/null)')"
    rm -f "$NME"
    # nm prints "<value> <type> <name>" for a definition and "U <name>" for a
    # reference, so the type is always the field before the name.
    SYM="$(printf '%s\n' "$NMO" | awk 'NF >= 2 && $NF == "_rjem_malloc_conf" { print }')"
    printf '%s\n' "$SYM" | sed '/^$/d; s/^/  nm: /' >> "$LOG"
    NW="$(printf '%s\n' "$SYM" | awk 'NF >= 2 && $(NF - 1) ~ /^[VvWw]$/ { n++ } END { print n + 0 }')"
    NO="$(printf '%s\n' "$SYM" | awk 'NF >= 2 && $(NF - 1) !~ /^[UVvWw]$/ { n++ } END { print n + 0 }')"
    if [ "$NRC" -ne 0 ]; then ac_bad "nm_rc=${NRC}"
    elif [ -z "$NMO" ]; then ac_bad "nm=empty"
    else
      [ "$NW" -ge 1 ] || ac_bad "nm_weak_default=absent"
      [ "$NO" -eq 0 ] || ac_bad "nm_defined_rjem_malloc_conf=${NO}"
    fi
    log "nm _rjem_malloc_conf: weak_defaults=${NW} other_definitions=${NO}"
  fi
else
  log "no ${T_ROOT}/spec377-run/spec377-builds.txt: the JE archive cannot be read before the builds"
  AC_PENDING="je_archive=not_built"
fi
if [ -n "$AC_BAD" ]; then check alloc_conf FAIL "${AC_BAD}${AC_PENDING:+,${AC_PENDING}}"
elif [ -n "$AC_PENDING" ]; then check alloc_conf PENDING "etc files clean; ${AC_PENDING}"
else check alloc_conf PASS "no /etc/_rjem_malloc.conf or /etc/malloc.conf, ld.so.preload absent or blank, nm: weak default only"; fi

# ------------------------------------------------------------------ tools
log "== tools"
MISS=""
for t in $TOOLS; do
  p="$(command -v "$t" 2>/dev/null)"
  log "command -v ${t} -> '${p}'"
  [ -n "$p" ] || MISS="${MISS}${MISS:+,}${t}"
done
if [ -z "$MISS" ]; then check tools PASS "all present"; else check tools FAIL "missing=${MISS}"; fi

# --------------------------------------------------------- recorded only
log "== recorded (not gates)"
rec uname -r
rec lscpu
rec sh -c 'ldd --version 2>&1 | head -1'
rec sysctl vm.overcommit_memory vm.max_map_count
rec sh -c 'grep -H . /proc/sys/vm/overcommit_*'
rec sh -c 'ps -eo pcpu,comm --sort=-pcpu | head -6'
log "awk resolves to: '$(command -v awk 2>/dev/null)'"
rec sh -c 'original-awk -version 2>&1 | head -1'
rec dpkg-query -W original-awk

finish
