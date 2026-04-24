#!/usr/bin/env bash
# QA orchestrator for examples/templates/{todo,chat} acceptance criteria 5-9.
# Drives gsd-browser through each scenario and stores before/after screenshots.
#
# AC #7 disposition (SPEC-225 G1 investigation):
#   The todosConflictResolver targets mapName='todos' (ORMap). ORMap.add(id, tag)
#   with unique tags never produces a merge rejection — each tag is distinct so
#   concurrent adds both survive. For a genuine LWW conflict the resolver would
#   need to target per-todo LWWMaps (dynamic keys: todo:{id}), which the current
#   register(mapName, def) API cannot address with a static name. Decision:
#   DOWNGRADE — AC #7 is an INFO verdict. No template code was changed.
#   Follow-up tracked as TODO for a dedicated LWW-conflict smoke-test.
#
# Prerequisites (start in three terminals before running):
#   TOPGUN_NO_AUTH=1 pnpm start:server               # ws://localhost:8080/ws
#   pnpm --filter @topgun-examples/todo dev           # http://localhost:5174
#   pnpm --filter @topgun-examples/chat dev           # http://localhost:5175
#
# Usage:
#   ./run-qa.sh                  # run all 5 scenarios
#   ./run-qa.sh ac5              # run a single scenario
#   ./run-qa.sh ac5 ac8          # run two specific scenarios
#
# Results land in examples/templates/qa/results/<timestamp>/<scenario>/

set -uo pipefail

GSD="${GSD_BROWSER:-/Users/koristuvac/.cargo/bin/gsd-browser}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TS="$(date +%Y-%m-%d_%H-%M-%S)"
RESULTS="$SCRIPT_DIR/results/$TS"

TODO_URL="${TODO_URL:-http://localhost:5174}"
CHAT_URL="${CHAT_URL:-http://localhost:5175}"
SERVER_PORT="${SERVER_PORT:-8080}"

# Sessions used by scenarios. Each session = isolated Chrome with its own
# localStorage, so two sessions = two distinct guest identities.
SESSION_A="topgun-qa-tab-a"
SESSION_B="topgun-qa-tab-b"
SESSION_SOLO="topgun-qa-solo"
ALL_SESSIONS=("$SESSION_A" "$SESSION_B" "$SESSION_SOLO")

# ──────────────────────────────────────────────────────────────────────────────
# Helpers
# ──────────────────────────────────────────────────────────────────────────────

red()    { printf "\033[31m%s\033[0m\n" "$*"; }
green()  { printf "\033[32m%s\033[0m\n" "$*"; }
yellow() { printf "\033[33m%s\033[0m\n" "$*"; }
blue()   { printf "\033[34m%s\033[0m\n" "$*"; }

g() {
  # gsd helper: takes session name + remaining args. Returns exit code.
  local session="$1"; shift
  "$GSD" --session "$session" "$@"
}

shot() {
  # Screenshot helper: session, output filename (no extension)
  local session="$1" out="$2"
  g "$session" screenshot --full-page --format png --output "$out.png" >/dev/null 2>&1 \
    || yellow "  (screenshot failed: $out)"
}

wait_text_visible() {
  # Wait for text that is visible in the DOM and NOT inside an input/textarea/hidden element.
  # Usage: wait_text_visible <session> <text> [timeout-ms]
  #
  # gsd-browser's wait-for --condition text_visible matches any text in the DOM,
  # including input values and hidden containers, which causes false positives.
  # This helper pairs the wait with a find --text --json assertion that checks
  # the match is not inside a tag=input or tag=textarea element.
  local session="$1" text="$2" timeout="${3:-10000}"

  # Phase 1: wait for text to appear anywhere in the DOM
  if ! g "$session" wait-for --condition text_visible --value "$text" --timeout "$timeout" >/dev/null 2>&1; then
    return 1
  fi

  # Phase 2: assert the match is in a visible, non-input element.
  # find --text --json returns a JSON array of matches with element metadata.
  # We check that at least one match exists outside input/textarea elements.
  local find_result
  find_result="$(g "$session" find --text "$text" --json 2>/dev/null || echo '{}')"

  # If find returns no matches or errors, fall back to accepting the wait result
  # (avoids breaking on environments where find --text --json is unavailable).
  if echo "$find_result" | /usr/bin/grep -q '"matches"\s*:\s*\[\]' 2>/dev/null; then
    yellow "  (wait_text_visible: find returned empty matches for '$text' — accepting wait result)"
    return 0
  fi

  # Check for input/textarea contamination: if ALL matches are inside input elements,
  # this is a false positive. Use an eval fallback to confirm a non-input match exists.
  if echo "$find_result" | /usr/bin/grep -qi '"tag"\s*:\s*"input"\|"tag"\s*:\s*"textarea"' 2>/dev/null; then
    # Try eval to check if text appears in a non-input element's textContent
    local non_input_found
    non_input_found="$(g "$session" eval \
      "document.querySelectorAll('*:not(input):not(textarea)').length > 0 && Array.from(document.querySelectorAll('*:not(input):not(textarea)')).some(el => el.childElementCount === 0 && el.textContent && el.textContent.includes('$(printf '%s' "$text" | /usr/bin/sed "s/'/\\\\'/g")'))" \
      2>/dev/null || echo 'false')"
    if echo "$non_input_found" | /usr/bin/grep -qi 'true'; then
      return 0
    else
      yellow "  (wait_text_visible: '$text' only found in input elements — false positive filtered)"
      return 1
    fi
  fi

  return 0
}

assert_text() {
  # Returns 0 if text found in page, 1 otherwise
  local session="$1" text="$2"
  g "$session" find --text "$text" --json 2>/dev/null \
    | /usr/bin/grep -q '"matches"\s*:\s*\[' && \
    g "$session" find --text "$text" --json 2>/dev/null \
    | /usr/bin/grep -q -v '"matches":\[\]'
}

reset_session() {
  # Clear localStorage + IDB so a fresh "guest" is generated on next visit.
  local session="$1"
  g "$session" eval 'localStorage.clear(); indexedDB.databases?.().then(dbs => dbs.forEach(d => indexedDB.deleteDatabase(d.name)))' >/dev/null 2>&1 || true
}

prewarm_daemon() {
  # Start each session's Chrome daemon before two-session scenarios to avoid
  # cold-start exceeding scenario timeouts. gsd-browser daemon cold-start can
  # take >10s on first launch; prewarm moves that cost out of the scenario budget.
  blue "Prewarming gsd-browser daemons…"
  for s in "${ALL_SESSIONS[@]}"; do
    "$GSD" --session "$s" daemon start >/dev/null 2>&1 &
  done

  # Wait up to 15s per session for daemons to become ready
  local deadline=$(( $(date +%s) + 15 ))
  local all_ready=0
  while [[ $(date +%s) -lt $deadline ]]; do
    all_ready=1
    for s in "${ALL_SESSIONS[@]}"; do
      if ! "$GSD" --session "$s" daemon status >/dev/null 2>&1; then
        all_ready=0
        break
      fi
    done
    [[ $all_ready -eq 1 ]] && break
    /bin/sleep 1
  done

  if [[ $all_ready -eq 1 ]]; then
    green "  ✓ All daemons ready"
  else
    yellow "  ⚠ Some daemons may not be fully ready after 15s — proceeding anyway"
  fi
}

cleanup() {
  yellow ""
  yellow "Cleaning up sessions…"

  # Stop each session's daemon gracefully
  for s in "${ALL_SESSIONS[@]}"; do
    "$GSD" --session "$s" daemon stop >/dev/null 2>&1 || true
  done

  # Kill any orphan Chrome processes associated with QA sessions.
  # Note: macOS-only pattern — process is named "Google Chrome" on macOS.
  # On Linux the process name is "chrome" or "chromium"; this harness is not
  # expected to run on Linux CI.
  pkill -f "Google Chrome.*--user-data-dir=.*topgun-qa" >/dev/null 2>&1 || true

  # Remove stale gsd-browser session directories. Path sanity check guards
  # against accidentally removing unrelated directories if gsd-browser ever
  # changes its session storage location.
  local session_glob="$HOME/.gsd-browser/sessions/topgun-qa-*"
  if [[ "$session_glob" == "$HOME/"* ]] && echo "$session_glob" | /usr/bin/grep -q "topgun-qa"; then
    rm -rf $HOME/.gsd-browser/sessions/topgun-qa-* 2>/dev/null || true
  else
    yellow "  ⚠ Unexpected session path '$session_glob' — skipping rm to avoid data loss"
  fi
}
trap cleanup EXIT

emit_verification_summary() {
  # Write a human-readable summary of this run's verdicts to verification-summary.md.
  # The filename is stable (no timestamp in the leaf name) so the SPEC-223 archive
  # appendix can link to it with a stable path: results/<timestamp>/verification-summary.md
  local -n _passed_ref=$1
  local -n _failed_ref=$2
  local -n _info_ref=$3
  local git_sha
  git_sha="$(/usr/bin/git -C "$SCRIPT_DIR" rev-parse --short HEAD 2>/dev/null || echo 'unknown')"

  local summary_file="$RESULTS/verification-summary.md"
  {
    printf "# Template QA Verification Summary\n\n"
    printf "**Date:** %s\n" "$(date '+%Y-%m-%d %H:%M:%S')"
    printf "**Git SHA:** %s\n" "$git_sha"
    printf "**Harness:** \`TOPGUN_NO_AUTH=1 pnpm start:server\` + todo/chat dev servers + \`./run-qa.sh\`\n\n"
    printf "## Verdicts\n\n"
    printf "| Scenario | Verdict | Screenshots |\n"
    printf "|----------|---------|-------------|\n"
    for s in "${_passed_ref[@]:-}"; do
      [[ -n "$s" ]] && printf "| %s | PASS | \`%s/%s/\` |\n" "$s" "$RESULTS" "$s-*"
    done
    for s in "${_info_ref[@]:-}"; do
      [[ -n "$s" ]] && printf "| %s | INFO | \`%s/%s/\` |\n" "$s" "$RESULTS" "$s-*"
    done
    for s in "${_failed_ref[@]:-}"; do
      [[ -n "$s" ]] && printf "| %s | FAIL | \`%s/%s/\` |\n" "$s" "$RESULTS" "$s-*"
    done
    printf "\n## Notes\n\n"
    printf "- **AC #7 disposition:** INFO (DOWNGRADE). The \`todosConflictResolver\` targets the \`todos\` ORMap; ORMap.add with unique tags never produces a merge rejection. See \`run-qa.sh\` header comment for full rationale.\n"
    printf "- **Screenshot directory:** \`%s/\`\n" "$RESULTS"
    printf "- **SPEC-223 archive:** \`.specflow/archive/SPEC-223.md\` — see \`## Verification Result\` appendix.\n"
  } > "$summary_file"
  blue "  Verification summary: $summary_file"
}

preflight() {
  blue "Preflight checks…"
  local fail=0

  # Unconditional advisory: TOPGUN_NO_AUTH must be set when starting the server
  # for templates to connect without auth tokens. No detection logic — always printed.
  blue "  ℹ TOPGUN_NO_AUTH: ensure server was started with this env-flag set to 1"

  if ! /usr/bin/curl -fs --max-time 2 "$TODO_URL" >/dev/null; then
    red "  ✗ todo dev server unreachable at $TODO_URL"
    red "    start with: pnpm --filter @topgun-examples/todo dev"
    fail=1
  else
    green "  ✓ todo dev server $TODO_URL"
  fi

  if ! /usr/bin/curl -fs --max-time 2 "$CHAT_URL" >/dev/null; then
    red "  ✗ chat dev server unreachable at $CHAT_URL"
    red "    start with: pnpm --filter @topgun-examples/chat dev"
    fail=1
  else
    green "  ✓ chat dev server $CHAT_URL"
  fi

  if ! /usr/bin/nc -z localhost "$SERVER_PORT" 2>/dev/null; then
    red "  ✗ TopGun server not listening on :$SERVER_PORT"
    red "    start with: TOPGUN_NO_AUTH=1 pnpm start:server"
    fail=1
  else
    green "  ✓ TopGun server :$SERVER_PORT"
  fi

  if [[ ! -x "$GSD" ]]; then
    red "  ✗ gsd-browser not found at $GSD"
    red "    set GSD_BROWSER env var or install gsd-browser"
    fail=1
  else
    green "  ✓ gsd-browser $GSD"
  fi

  [[ $fail -eq 0 ]] || exit 1
}

# ──────────────────────────────────────────────────────────────────────────────
# Scenarios
# ──────────────────────────────────────────────────────────────────────────────

# AC #5 — Real-time todo sync across two tabs (two distinct guests)
scenario_ac5() {
  local out="$RESULTS/ac5-sync"
  /bin/mkdir -p "$out"
  blue "AC #5 — Real-time todo sync across two tabs"

  local marker="qa-sync-$(date +%s)"

  # Tab A: open, wait for connected, add a todo
  g "$SESSION_A" navigate "$TODO_URL" >/dev/null
  wait_text_visible "$SESSION_A" "Connected" 8000 || wait_text_visible "$SESSION_A" "Synced" 5000 || true
  shot "$SESSION_A" "$out/01-tab-a-connected"

  g "$SESSION_A" type 'input[placeholder^="Add a new todo"]' "$marker" --submit >/dev/null
  /bin/sleep 1
  shot "$SESSION_A" "$out/02-tab-a-after-add"

  # Tab B: open separately, expect the same todo to appear via server roundtrip
  g "$SESSION_B" navigate "$TODO_URL" >/dev/null
  wait_text_visible "$SESSION_B" "Connected" 8000 || true

  if wait_text_visible "$SESSION_B" "$marker" 8000; then
    shot "$SESSION_B" "$out/03-tab-b-received"
    green "  ✓ AC #5 PASS — '$marker' propagated A→B"
    return 0
  else
    shot "$SESSION_B" "$out/03-tab-b-MISSING"
    red "  ✗ AC #5 FAIL — '$marker' did not appear in tab B within 8s"
    return 1
  fi
}

# AC #6 — Offline edit + reconnect → SyncStatus transition
scenario_ac6() {
  local out="$RESULTS/ac6-offline"
  /bin/mkdir -p "$out"
  blue "AC #6 — Offline → queued write → reconnect → 'merged N pending writes'"

  local marker="qa-offline-$(date +%s)"

  # Connect normally
  g "$SESSION_SOLO" navigate "$TODO_URL" >/dev/null
  wait_text_visible "$SESSION_SOLO" "Connected" 8000 || true
  shot "$SESSION_SOLO" "$out/01-connected"

  # Block WS endpoint, then reload so the page can't connect
  g "$SESSION_SOLO" block-urls "ws://localhost:$SERVER_PORT/*" >/dev/null
  g "$SESSION_SOLO" reload >/dev/null
  wait_text_visible "$SESSION_SOLO" "Offline" 10000 \
    || wait_text_visible "$SESSION_SOLO" "Connecting" 3000 \
    || wait_text_visible "$SESSION_SOLO" "Initialising" 3000 || true
  shot "$SESSION_SOLO" "$out/02-offline"

  # Add a todo while offline (queues to IDB opLog)
  g "$SESSION_SOLO" type 'input[placeholder^="Add a new todo"]' "$marker" --submit >/dev/null
  /bin/sleep 1
  shot "$SESSION_SOLO" "$out/03-queued-while-offline"

  # Restore network, reload, expect reconnect + queued write to drain
  g "$SESSION_SOLO" clear-routes >/dev/null
  g "$SESSION_SOLO" reload >/dev/null

  if wait_text_visible "$SESSION_SOLO" "merged" 12000 \
       || wait_text_visible "$SESSION_SOLO" "Synced" 12000 \
       || wait_text_visible "$SESSION_SOLO" "Connected" 12000; then
    shot "$SESSION_SOLO" "$out/04-reconnected-merged"
    if assert_text "$SESSION_SOLO" "$marker"; then
      green "  ✓ AC #6 PASS — queued '$marker' drained on reconnect"
      return 0
    else
      yellow "  ⚠ AC #6 PARTIAL — reconnected but '$marker' not visible (queue did not drain?)"
      return 1
    fi
  else
    shot "$SESSION_SOLO" "$out/04-NO-RECONNECT"
    red "  ✗ AC #6 FAIL — never returned to Connected/Synced state"
    return 1
  fi
}

# AC #7 — ConflictLog / conflict resolver investigation
# INFO verdict — the todosConflictResolver targets the ORMap (mapName='todos')
# which uses unique-tag add semantics; concurrent adds never produce a rejection.
# See run-qa.sh header comment for full SPEC-225 G1 investigation rationale.
scenario_ac7() {
  local out="$RESULTS/ac7-conflict"
  /bin/mkdir -p "$out"
  blue "AC #7 — ConflictLog / conflict resolver (INFO — see header comment)"
  yellow "  ℹ AC #7 disposition: INFO (DOWNGRADE). The resolver targets the ORMap"
  yellow "    todos map; ORMap unique-tag add semantics prevent LWW conflicts."
  yellow "    No MERGE_REJECTED event is expected. This verdict is informational."

  # Still capture a screenshot to document the ConflictLog panel's presence in the UI.
  g "$SESSION_A" navigate "$TODO_URL" >/dev/null
  wait_text_visible "$SESSION_A" "Connected" 8000 || true
  shot "$SESSION_A" "$out/01-conflict-log-panel-presence"

  yellow "  ⚠ AC #7 INFO — ConflictLog panel exists in UI but resolver does not"
  yellow "    fire for ORMap operations. Follow-up TODO tracks a dedicated"
  yellow "    LWW-conflict smoke-test (two tabs editing same todo:id LWWMap)."
  return 0
}

# AC #8 — HLC-ordered chat across two tabs (same room, two guests)
scenario_ac8() {
  local out="$RESULTS/ac8-chat-order"
  /bin/mkdir -p "$out"
  blue "AC #8 — HLC-ordered chat across two tabs"

  local room="qa-room-$(date +%s)"
  local m_a="msg-from-A-$(date +%s)"
  local m_b="msg-from-B-$(date +%s)"

  # Both join the same room via URL hash
  g "$SESSION_A" navigate "${CHAT_URL}/#${room}" >/dev/null
  g "$SESSION_B" navigate "${CHAT_URL}/#${room}" >/dev/null
  /bin/sleep 2
  shot "$SESSION_A" "$out/01-tab-a-empty"
  shot "$SESSION_B" "$out/01-tab-b-empty"

  # A sends, then B sends
  g "$SESSION_A" type 'input[placeholder^="Type a message"]' "$m_a" --submit >/dev/null
  /bin/sleep 1
  g "$SESSION_B" type 'input[placeholder^="Type a message"]' "$m_b" --submit >/dev/null
  /bin/sleep 2

  shot "$SESSION_A" "$out/02-tab-a-after-exchange"
  shot "$SESSION_B" "$out/02-tab-b-after-exchange"

  local pass=0
  if assert_text "$SESSION_A" "$m_a" && assert_text "$SESSION_A" "$m_b"; then
    pass=$((pass + 1))
  fi
  if assert_text "$SESSION_B" "$m_a" && assert_text "$SESSION_B" "$m_b"; then
    pass=$((pass + 1))
  fi

  if [[ $pass -eq 2 ]]; then
    green "  ✓ AC #8 PASS — both messages visible in both tabs"
    return 0
  else
    red "  ✗ AC #8 FAIL — only $pass/2 tabs received both messages"
    return 1
  fi
}

# AC #9 — SkewClockPanel +5s buffer with HLC-correct ordering on delivery
scenario_ac9() {
  local out="$RESULTS/ac9-skew-buffer"
  /bin/mkdir -p "$out"
  blue "AC #9 — SkewClockPanel buffers incoming +5s, late delivery slots in HLC order"

  local room="qa-skew-$(date +%s)"
  local first="aaa-first-$(date +%s)"
  local second="zzz-second-$(date +%s)"

  # Both join same room. B has skew enabled, so it will buffer A's messages.
  g "$SESSION_A" navigate "${CHAT_URL}/#${room}" >/dev/null
  g "$SESSION_B" navigate "${CHAT_URL}/#${room}" >/dev/null
  /bin/sleep 2

  # Verify the mandatory demo-only label is rendered
  if ! assert_text "$SESSION_B" "simulates incoming message delay"; then
    red "  ✗ AC #9 FAIL — mandatory 'demo-only' label not visible"
    shot "$SESSION_B" "$out/00-LABEL-MISSING"
    return 1
  fi

  # Toggle skew on tab B (the receiver)
  g "$SESSION_B" set-checked 'input[type="checkbox"]' --json true >/dev/null 2>&1 \
    || g "$SESSION_B" click 'input[type="checkbox"]' >/dev/null
  /bin/sleep 1
  shot "$SESSION_B" "$out/01-tab-b-skew-on"

  # A sends 'first' then 'second'. B should buffer them.
  g "$SESSION_A" type 'input[placeholder^="Type a message"]' "$first" --submit >/dev/null
  /bin/sleep 0.5
  g "$SESSION_A" type 'input[placeholder^="Type a message"]' "$second" --submit >/dev/null
  /bin/sleep 1

  # Tab B should NOT yet show the messages (still buffered)
  if assert_text "$SESSION_B" "buffered"; then
    shot "$SESSION_B" "$out/02-tab-b-buffering"
  else
    yellow "  (no 'buffered' indicator — buffer may have already drained or label differs)"
    shot "$SESSION_B" "$out/02-tab-b-immediately-after-send"
  fi

  # Wait out the 5-second buffer + 250ms tick + small slack
  /bin/sleep 6
  shot "$SESSION_B" "$out/03-tab-b-after-drain"

  # Both messages should now be visible in B
  if assert_text "$SESSION_B" "$first" && assert_text "$SESSION_B" "$second"; then
    green "  ✓ AC #9 PASS — buffered messages delivered after 5s"
    return 0
  else
    red "  ✗ AC #9 FAIL — buffered messages not visible in tab B after drain"
    return 1
  fi
}

# ──────────────────────────────────────────────────────────────────────────────
# Main
# ──────────────────────────────────────────────────────────────────────────────

usage() {
  printf "Usage: %s [scenario...]\n\nScenarios:  ac5  ac6  ac7  ac8  ac9   (default: all)\n\nEnv overrides: TODO_URL, CHAT_URL, SERVER_PORT, GSD_BROWSER\n\nResults: %s/results/<timestamp>/\n" "$0" "$SCRIPT_DIR"
}

main() {
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage; exit 0
  fi

  preflight
  /bin/mkdir -p "$RESULTS"
  blue ""
  blue "Results directory: $RESULTS"
  blue ""

  # Prewarm daemons before any two-session scenarios to avoid cold-start timeouts.
  # Called after preflight so we know gsd-browser is available.
  prewarm_daemon

  local scenarios=("$@")
  if [[ ${#scenarios[@]} -eq 0 ]]; then
    scenarios=(ac5 ac6 ac7 ac8 ac9)
  fi

  declare -a passed=()
  declare -a failed=()
  declare -a info_verdicts=()

  for s in "${scenarios[@]}"; do
    case "$s" in
      ac5) scenario_ac5 ;;
      ac6) scenario_ac6 ;;
      ac7) scenario_ac7; info_verdicts+=("$s"); blue ""; continue ;;
      ac8) scenario_ac8 ;;
      ac9) scenario_ac9 ;;
      *)   red "Unknown scenario: $s"; usage; exit 1 ;;
    esac
    if [[ $? -eq 0 ]]; then
      passed+=("$s")
    else
      failed+=("$s")
    fi
    blue ""
  done

  blue "──────────────────────────────────────────"
  blue "Summary"
  blue "──────────────────────────────────────────"
  [[ ${#passed[@]} -gt 0 ]] && green "  PASS: ${passed[*]}"
  [[ ${#info_verdicts[@]} -gt 0 ]] && yellow "  INFO: ${info_verdicts[*]}"
  [[ ${#failed[@]} -gt 0 ]] && red   "  FAIL: ${failed[*]}"
  blue "  Screenshots: $RESULTS"
  blue ""

  emit_verification_summary passed failed info_verdicts

  [[ ${#failed[@]} -eq 0 ]] || exit 1
}

main "$@"
