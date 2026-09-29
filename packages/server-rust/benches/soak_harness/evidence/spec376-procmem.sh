#!/usr/bin/env bash
#
# Linux per-process memory sampler, SOURCED by spec376-cells.sh (and by the
# synthetic suite). It replaces the macOS runner's `footprint --swapped` call:
# Apple's phys_footprint has no Linux equivalent under that name, so the row
# carries the kernel's own smaps_rollup / status fields under honest names and
# one derived column, fp_equiv_mb = Anonymous - LazyFree + Swap.
#
# Interface
#   procmem_row <pid> [<proc_root>]
#     Prints ONE line of twelve comma-joined cells, in CSV header order:
#       rss_mb,fp_equiv_mb,hwm_rss_mb,lazyfree_mb,swap_mb,file_mb,
#       anon_mb,private_dirty_mb,pss_mb,anon_huge_mb,smaps_rss_mb,smaps_read_ms
#     The runner places them at header positions 2, 7-10, 41 and 56-61.
#     Return status:
#       0  row complete; every memory cell numeric
#       1  the pid is gone and a source could not be read: every cell empty,
#          the post-mortem memory-row counter incremented
#       2  BLIND ON A LIVE PID: a source or a required field is missing or
#          non-numeric while the pid is alive; stdout carries the reason
#          (the missing source or field) instead of a row. The runner must
#          call sampler_fatal -- an empty memory cell on a live pid is the
#          silent blindness this file exists to prevent.
#       3  usage error (no pid, or a counter-file variable unset); stdout
#          carries the reason
#
#   The caller sets two counter-file paths before the first call. The helper
#   runs inside a command substitution, so a counter kept in a shell variable
#   would die with the subshell and always read 0:
#     PROCMEM_INV_FILE  rows violating the row invariants (below)
#     PROCMEM_PM_FILE   post-mortem memory rows (status 1)
#
# Sources
#   rss_mb        `ps -o rss= -p <pid>` -- the harness's own memory series
#   hwm_rss_mb    <proc_root>/<pid>/status VmHWM (a peak RSS, not a peak
#                 footprint)
#   every other   <proc_root>/<pid>/smaps_rollup: Rss, Pss, Private_Clean,
#                 Shared_Clean, Private_Dirty, Anonymous, LazyFree,
#                 AnonHugePages, Swap
#   smaps_read_ms wall time of the smaps_rollup read, integer ms: the kernel
#                 walks every VMA under the mmap lock for this file, so its
#                 cost is recorded beside the value it bought
#
# Row invariants (integer KiB, exact): 0 <= LazyFree <= Anonymous and
# Anonymous - LazyFree + Swap <= Rss + Swap. A violating row is still printed
# with status 0 -- it is data, not a sampler failure -- and PROCMEM_INV_FILE is
# incremented, so the runner console can report the count.
#
# Fixture mode. A proc root other than /proc selects it, so the synthetic
# suite can drive this file on any host:
#   <root>/<pid>/smaps_rollup, <root>/<pid>/status  stand in for /proc
#   <root>/<pid>.alive   "0" = the pid is gone; any other content, or no
#                        file, = alive. Absence reads as ALIVE on purpose: a
#                        malformed fixture then fails closed (status 2)
#                        instead of producing a silently empty row.
#   <root>/<pid>.ps_rss  the literal `ps -o rss=` output; absent = ps printed
#                        nothing
# The live path (/proc) never reads *.alive or *.ps_rss.
#
# All parsing under LC_ALL=C; all arithmetic in integer KiB before the single
# /1024 conversion. Bash 3.2 compatible, so the fixture cases run on macOS.

# Sourcing sets the parsing locale for the whole caller on purpose: the runner
# already exports it, and a comma-decimal locale would corrupt every %.3f.
export LC_ALL=C

# Microseconds since the epoch. EPOCHREALTIME (bash >= 5) is a builtin and
# adds no process to the timed read; GNU date is the fallback on an older
# bash; perl covers BSD date, whose %N is not a conversion.
procmem_now_us() {
  local t
  if [ -n "${EPOCHREALTIME:-}" ]; then
    t="${EPOCHREALTIME/[.,]/}"
    case "$t" in ''|*[!0-9]*) ;; *) printf '%s' "$t"; return 0 ;; esac
  fi
  t="$(date +%s%6N 2>/dev/null || true)"
  case "$t" in ''|*[!0-9]*) ;; *) printf '%s' "$t"; return 0 ;; esac
  t="$(perl -MTime::HiRes=time -e 'printf "%.0f", time() * 1e6' 2>/dev/null || true)"
  case "$t" in ''|*[!0-9]*) return 1 ;; *) printf '%s' "$t"; return 0 ;; esac
}

procmem_bump() {   # $1 = counter file
  local n
  n="$(cat "$1" 2>/dev/null || echo 0)"
  case "$n" in ''|*[!0-9]*) n=0 ;; esac
  echo $((n + 1)) > "$1"
}

procmem_row() {
  local pid="${1:-}" root="${2:-/proc}"
  case "$pid" in
    ''|*[!0-9]*) printf 'usage: procmem_row <pid> [<proc_root>] (pid=%s)\n' "$pid"; return 3 ;;
  esac
  if [ -z "${PROCMEM_INV_FILE:-}" ] || [ -z "${PROCMEM_PM_FILE:-}" ]; then
    printf 'PROCMEM_INV_FILE and PROCMEM_PM_FILE must be set\n'
    return 3
  fi

  local fixture=0
  [ "$root" != "/proc" ] && fixture=1

  local rss_raw smaps status t0 t1 read_ms
  if [ "$fixture" = "1" ]; then
    rss_raw="$(cat "${root}/${pid}.ps_rss" 2>/dev/null || true)"
  else
    rss_raw="$(ps -o rss= -p "$pid" 2>/dev/null || true)"
  fi
  rss_raw="$(printf '%s' "$rss_raw" | tr -d '[:space:]')"

  t0="$(procmem_now_us || true)"
  smaps="$(cat "${root}/${pid}/smaps_rollup" 2>/dev/null || true)"
  t1="$(procmem_now_us || true)"
  status="$(cat "${root}/${pid}/status" 2>/dev/null || true)"
  read_ms=""
  case "${t0}${t1}" in
    ''|*[!0-9]*) ;;
    *) [ "${#t0}" -gt 0 ] && [ "${#t1}" -gt 0 ] && read_ms=$(( (t1 - t0) / 1000 )) ;;
  esac

  # One awk pass decides completeness and computes every cell. Input: the
  # smaps_rollup text, a separator line, the status text. A field that
  # appears twice, is not an integer, or is not in kB counts as missing --
  # absence and ambiguity are both blindness.
  local parsed
  parsed="$(
    { printf '%s\n' "$smaps"; printf '%s\n' '@@PROCMEM_STATUS@@'; printf '%s\n' "$status"; } |
    LC_ALL=C awk -v rss_raw="$rss_raw" -v read_ms="$read_ms" '
      function take(key, val, unit) {
        if (key in cnt) { cnt[key]++; return }
        cnt[key] = 1
        if (val ~ /^[0-9]+$/ && unit == "kB") v[key] = val
        else bad[key] = 1
      }
      function mb(k) { return sprintf("%.3f", k / 1024) }
      $0 == "@@PROCMEM_STATUS@@" { in_status = 1; next }
      !in_status && $1 ~ /:$/ { k = substr($1, 1, length($1) - 1); take("s." k, $2, $3); next }
      in_status && $1 == "VmHWM:" { take("t.VmHWM", $2, $3); next }
      END {
        n = split("s.Rss s.Pss s.Private_Clean s.Shared_Clean s.Private_Dirty s.Anonymous s.LazyFree s.AnonHugePages s.Swap t.VmHWM", req, " ")
        miss = ""
        if (rss_raw !~ /^[0-9]+$/) miss = "ps_rss"
        for (i = 1; i <= n; i++) {
          f = req[i]
          if (!(f in cnt) || cnt[f] != 1 || (f in bad)) {
            name = substr(f, 3)
            if (!(f in cnt)) why = "absent"; else if (cnt[f] != 1) why = "dup"; else why = "non_numeric"
            miss = miss (miss == "" ? "" : ",") name "=" why
          }
        }
        if (read_ms !~ /^[0-9]+$/) miss = miss (miss == "" ? "" : ",") "smaps_read_ms=unmeasured"
        if (miss != "") { print "MISSING " miss; exit }
        anon = v["s.Anonymous"]; lazy = v["s.LazyFree"]; swap = v["s.Swap"]; rss = v["s.Rss"]
        inv = (lazy <= anon && anon - lazy + swap <= rss + swap) ? "ok" : "violated"
        printf "OK %s %s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n", inv, \
          mb(rss_raw), mb(anon - lazy + swap), mb(v["t.VmHWM"]), mb(lazy), mb(swap), \
          mb(v["s.Private_Clean"] + v["s.Shared_Clean"]), mb(anon), \
          mb(v["s.Private_Dirty"]), mb(v["s.Pss"]), mb(v["s.AnonHugePages"]), mb(rss), read_ms
      }
    '
  )"

  case "$parsed" in
    "OK "*)
      parsed="${parsed#OK }"
      if [ "${parsed%% *}" = "violated" ]; then
        procmem_bump "$PROCMEM_INV_FILE"
      fi
      printf '%s\n' "${parsed#* }"
      return 0
      ;;
  esac

  # Something is missing. Only a GONE pid may yield an empty row; on a live
  # pid the same miss is fatal to the cell.
  local alive=1
  if [ "$fixture" = "1" ]; then
    [ "$(cat "${root}/${pid}.alive" 2>/dev/null | tr -d '[:space:]')" = "0" ] && alive=0
  else
    kill -0 "$pid" 2>/dev/null || alive=0
  fi
  if [ "$alive" = "0" ]; then
    procmem_bump "$PROCMEM_PM_FILE"
    printf ',,,,,,,,,,,\n'
    return 1
  fi
  printf '%s\n' "${parsed#MISSING }"
  return 2
}
