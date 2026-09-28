#!/usr/bin/env bash
#
# Host preflight for the Linux bench host (topgun-bench). Writes one sidecar
# log, spec376-preflight-<YYYYMMDDTHHMMSSZ>.log, holding every command and its
# output, whose LAST line is exactly one of
#   PREFLIGHT=PASS PREFLIGHT_AT=<YYYY-MM-DDTHH:MM:SSZ>
#   PREFLIGHT=FAIL failed=<comma list> PREFLIGHT_AT=<YYYY-MM-DDTHH:MM:SSZ>
# and nothing after it. The cal chain accepts the lexicographically newest log
# (the stamp sorts in time order) only if that line is PASS and at most 60 min
# old. Exit 0 on PASS, 1 on FAIL, 2 on the Linux-only guard or usage.
#
# usage: spec376-preflight.sh [--apply]
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
#   SPEC376_PREFLIGHT_SYS_ROOT  prefix for the host files this script reads or
#                               writes under /sys (THP knobs) and the systemd
#                               unit directories; default empty = the real /.
#   SPEC376_PREFLIGHT_CMDLOG    if set, mutate() appends each command to this
#                               file before running it.
#   SPEC376_PREFLIGHT_LOG_DIR   where the log is written; default the evidence
#                               dir (this script's dir), so the cal-phase log
#                               travels with the artifacts.
#
# Every check follows the absence rule: a command that fails, prints nothing,
# or prints a value that does not parse is a FAIL named in failed=, never a
# pass.
set -uo pipefail
export LC_ALL=C

# The checks below read /proc, systemd and sysfs; on any other OS they would
# fail or, worse, read something else. Refuse before any file exists.
if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "FATAL: spec376-preflight.sh runs on Linux only (uname -s = '$(uname -s 2>/dev/null || true)')" >&2
  exit 2
fi

APPLY=0
case "$#:${1:-}" in
  0:) ;;
  1:--apply) APPLY=1 ;;
  *) echo "usage: spec376-preflight.sh [--apply]" >&2; exit 2 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="${SPEC376_PREFLIGHT_SYS_ROOT:-}"
CMDLOG="${SPEC376_PREFLIGHT_CMDLOG:-}"
LOG_DIR="${SPEC376_PREFLIGHT_LOG_DIR:-$SCRIPT_DIR}"
THP_DIR="${ROOT}/sys/kernel/mm/transparent_hugepage"
TIMERS="apt-daily.timer apt-daily-upgrade.timer man-db.timer e2scrub_all.timer fstrim.timer unattended-upgrades.service"
TOOLS="curl strings shasum sha256sum pgrep ss original-awk python3 git cargo rustc"
MIN_AVAIL_KB=14680064    # 14 GiB
MIN_DISK_KB=41943040     # 40 GiB

[ -d "$LOG_DIR" ] || { echo "FATAL: log dir ${LOG_DIR} does not exist" >&2; exit 2; }
LOG="$LOG_DIR/spec376-preflight-$(date -u +%Y%m%dT%H%M%SZ).log"
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
check() {   # $1 = name, $2 = PASS|FAIL, $3 = detail
  log "CHECK $1=$2 $3"
  echo "CHECK $1=$2 $3"
  [ "$2" = "PASS" ] || FAILED="${FAILED:+${FAILED},}$1"
}
finish() {
  local line
  if [ -z "$FAILED" ]; then
    line="PREFLIGHT=PASS PREFLIGHT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
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

log "== spec376-preflight start $(date -u +%Y-%m-%dT%H:%M:%SZ) apply=${APPLY} sys_root='${ROOT}' script=${SCRIPT_DIR}/spec376-preflight.sh"

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
rec sh -c 'ps -eo pcpu,comm --sort=-pcpu | head -6'
log "awk resolves to: '$(command -v awk 2>/dev/null)'"
rec sh -c 'original-awk -version 2>&1 | head -1'
rec dpkg-query -W original-awk

finish
