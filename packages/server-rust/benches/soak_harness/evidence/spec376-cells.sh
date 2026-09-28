#!/usr/bin/env bash
#
# Linux soak cell runner (topgun-bench) -- a copy of
# spec373b-cells.sh, which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT RUN ON LINUX. Its per-row
# memory breakdown calls /usr/bin/footprint, which does not exist there, and
# the parent's footprint_row() turns that absence into five SILENTLY EMPTY
# cells on every row -- a blind sampler that surfaces only at the predicates,
# hours into a run. Its pid resolution needs lsof, which Debian does not ship.
# And the cells this series runs (smoke admission + a calibration pair + a
# portability control) exist in no parent table. The difference list against
# spec373b-cells.sh is CLOSED at exactly nine items:
#
#   1. LINUX ONLY. `uname -s` must print Linux, checked before any other side
#      effect; anything else is FATAL, exit 2. No macOS code path remains.
#   2. THE MEMORY SAMPLER. footprint_row() is gone; the row's memory cells come
#      from procmem_row() in the sourced spec376-procmem.sh, read from
#      /proc/<pid>/smaps_rollup + /proc/<pid>/status (+ `ps -o rss=` for
#      rss_mb). The read takes the place of the parent's `ps -o rss=` read in
#      emit_row, so a pid gone at that point still ends sampling exactly as the
#      parent's empty-RSS path did, and no written row carries an empty memory
#      cell. A source or field missing on a LIVE pid is a SAMPLER FATAL, never
#      an empty cell. The CSV header is the parent's with five substitutions
#      (7 fp_equiv_mb, 8 hwm_rss_mb, 9 lazyfree_mb, 10 swap_mb, 41 file_mb) and
#      six appended columns (56-61 anon_mb, private_dirty_mb, pss_mb,
#      anon_huge_mb, smaps_rss_mb, smaps_read_ms). Post-run, the eleven memory
#      columns are checked for emptiness by header name, and the console ends
#      with steal_pct=, post_mortem_mem_reads= and
#      mem_invariant_violations=<n> (rows breaking 0 <= LazyFree <= Anonymous
#      or Anonymous - LazyFree + Swap <= Rss + Swap; such a row is still
#      written).
#   3. FLAVOURS CA|DH|JE|SYS|MI. The marker assertion adds MI (the mimalloc
#      literal only) and SYS (none of the four literals), the rule
#      spec372-allocdiag.sh already states.
#   4. PID RESOLUTION: `ss -Hltnp` first, `pgrep -f` second; lsof is not
#      used. The pre-launch port check also refuses a listener ss reports
#      without a pid.
#   5. THE MATRIX RECORDS THE HOST: /etc/os-release, kernel, CPU model and
#      count, glibc, rustc host triple, THP enabled/defrag, swap, overcommit,
#      max_map_count, the resolved awk and its version banner.
#   6. HOST SIDECAR FROM /proc: the matrix carries /proc/loadavg and the first
#      8 lines of /proc/meminfo at launch; /proc/stat steal ticks are read at
#      T0 and at the end and printed as steal_pct=<%.4f> (n/a when a read
#      fails or the total delta is 0).
#   7. THE FREEZE LITERAL IS CAL_PIN, the commit the series' .rs tree must
#      equal; the three refusal guards are unchanged. The server code-state
#      gate takes each cell's "server built at" commit: CAL_PIN, the 373b pin
#      b166719d, or the 373b freeze e85adb1f.
#   8. THE CELL TABLE, ENV NAMES, BASENAME, PORT. Smoke cells sc spb sdh sje
#      ssy smi (120 s, cadence 20 s; sdh SIGTERM so dhat flushes) and
#      calibration cells c1 c2 pb pa (900 s, cadence 60 s, SIGKILL). A smoke
#      cell refuses the tracked evidence dir as its artifact dir. Env names
#      SPEC376_HARNESS_BIN, SPEC376_CHAIN_START_EPOCH, SPEC376_SERVER_COMMIT
#      (the SPEC365_* overrides and the smoke knobs keep their parent names);
#      basename spec376-<cell>, data dir target/spec376-<cell>-data, port
#      47376.
#   9. This header, the usage text, the matrix banner and the comments and
#      messages that name the parent's platform tools.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE NINE ITEMS IS A DEFECT. The
# pre-registered manifest carries the hunk-to-item map. The departure is
# enumerable with:
#   diff spec373b-cells.sh spec376-cells.sh
#
# Everything from here on is spec373b-cells.sh's own header, kept verbatim
# so the lineage stays readable.
#
# Carve 9c part b (SPEC-373b) cell runner -- a copy of spec373a-cells.sh,
# which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT RUN THESE CELLS. They launch
# servers built from a different pin (b166719d, SPEC-373a's merge) and a
# different branch head, they bring the jemalloc flavour back for the recorded
# 4 h pair, and their artifacts must not overwrite the parent's committed
# evidence. The difference list against spec373a-cells.sh is CLOSED at exactly
# six items:
#
#   1. THE CELL TABLE. de (DH, 300 s) and dl (DH, 900 s) -- this spec's own
#      diagnostic dhat pair ("d-e" / "d-l"), SIGTERM teardown so dhat flushes,
#      server = the pin; b1 b2 (CA, pin) and a1 a2 (CA, freeze), 900 s, SIGKILL
#      teardown; jb (JE, pin) and ja (JE, freeze), 14400 s, SIGKILL teardown,
#      recorded only. Every cell: cadence 60 s, crash-interval 0, live-copy
#      census at 300 s, log directive armed, journal default. The header, usage
#      text, flavour column doc and matrix banner that name the cells are filed
#      under this item.
#   2. ENV NAMES. SPEC373B_CODE_FREEZE (and its placeholder),
#      SPEC373B_CHAIN_START_EPOCH, SPEC373B_HARNESS_BIN, SPEC373B_SERVER_COMMIT.
#      The SPEC365_* overrides and the smoke knobs keep their parent names.
#   3. THE FREEZE LITERAL is the branch commit whose .rs tree every cell's
#      checkout must equal (the last .rs commit of the carve). The three
#      refusal guards are unchanged.
#   4. ARTIFACT BASENAME spec373b-<cell>, DATA DIR target/spec373b-<cell>-data
#      (+ .meta sibling), port 47360 (distinct from 47355/47357/47358/47359).
#   5. THE FLAVOUR-MARKER ASSERTION covers CA, DH and JE: JE has the je_probe
#      literal only -- the grandparent spec372-allocdiag.sh's own JE line,
#      restored verbatim.
#   6. THE PIN. The "pin" code state is b166719d (was 61f84658), in the server
#      code-state assertion and its comment.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE SIX ITEMS IS A DEFECT. The
# pre-registered manifest carries the hunk-to-item map. The departure is
# enumerable with:
#   diff spec373a-cells.sh spec373b-cells.sh
#
# Everything from here on is spec373a-cells.sh's own header, kept verbatim
# so the lineage stays readable.
#
# Carve 9c part a (SPEC-373a) cell runner -- a copy of spec372-allocdiag.sh,
# which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT RUN THESE CELLS. They launch
# servers built from TWO code states (the pin 61f84658 and the branch head),
# they bring the dhat flavour back beside count-alloc, and their artifacts
# must not overwrite the parent's committed evidence. The difference list
# against spec372-allocdiag.sh is CLOSED at exactly seven items:
#
#   1. THE CELL TABLE. d3e (DH, 300 s) and d3l (DH, 900 s) -- the diagnostic
#      dhat pair, SIGTERM teardown so dhat flushes; b1 b2 (CA, pin) and a1 a2
#      (CA, branch head), 900 s, SIGKILL teardown. Every cell: cadence 60 s,
#      crash-interval 0, live-copy census at 300 s, log directive armed,
#      journal default. SPEC372_A2J_FLAVOUR and its a2j arm are gone. The
#      header, usage text and matrix banner that name the cells are filed
#      under this item.
#   2. ENV NAMES. SPEC373A_CODE_FREEZE (and its placeholder),
#      SPEC373A_CHAIN_START_EPOCH, SPEC373A_HARNESS_BIN. The SPEC365_*
#      overrides and the smoke knobs keep their parent names.
#   3. THE FREEZE LITERAL is the branch commit whose .rs tree every cell's
#      checkout must equal (the last .rs commit of the carve). The three
#      refusal guards are unchanged. It pins the CHECKOUT the runner and the
#      harness come from; the server a cell launches is item 7's business.
#   4. ARTIFACT BASENAME spec373a-<cell>, DATA DIR target/spec373a-<cell>-data
#      (+ .meta sibling), port 47359 (distinct from 47355/47357/47358).
#   5. THE FLAVOUR-MARKER ASSERTION covers CA and DH: CA has the alloc_probe
#      literal only, DH has the DHAT_OUT literal only (neither has je_probe
#      or the mimalloc literal).
#   6. CONSOLE LINE 1 carries flavour=<CA|DH> (the value set changes; the
#      line's shape does not). No hunk: the line is built from FLAVOUR.
#   7. THE SERVER CODE STATE. SPEC373A_SERVER_COMMIT is REQUIRED and must
#      resolve to the commit the cell names -- 61f84658 for d3e/d3l/b1/b2, the
#      freeze literal for a1/a2 -- asserted before the clock and echoed in the
#      matrix. The chain builds each server from a clean checkout at that
#      commit and records the binary's sha256; identity is still the sha256 on
#      console line 1, matched to spec373a-builds.txt downstream.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE SEVEN ITEMS IS A DEFECT. The
# pre-registered manifest carries the hunk-to-item map. The departure is
# enumerable with:
#   diff spec372-allocdiag.sh spec373a-cells.sh
#
# Everything from here on is spec372-allocdiag.sh's own header, kept verbatim
# so the lineage stays readable.
#
# Allocator-evaluation cell runner -- a copy of spec371-memdiag.sh, which is NOT
# edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT RUN THESE CELLS. They launch
# server builds under candidate global allocators (jemalloc, mimalloc) that the
# parent's flavour table does not know, they need the jemalloc stats probe and
# the probes' cumulative byte counters carried into the CSV, and their
# artifacts must not overwrite the parent's committed evidence. The difference
# list against spec371-memdiag.sh is CLOSED at exactly nine items:
#
#   1. THE CELL TABLE. Stage 1: s1a/s1b (SYS), j1a/j1b (JE), m1a/m1b (MI), k1
#      (CA), all 900 s. Stage 2: s2/j2/m2 (SYS/JE/MI, 14400 s) and a2j (900 s,
#      journal OFF, flavour JE or MI as supplied by the chain in
#      SPEC372_A2J_FLAVOUR). Every cell SIGKILL teardown, live-copy census at
#      300 s. The header, usage text and matrix banner that name the cells
#      are filed under this item.
#   2. ENV NAMES. SPEC372_CODE_FREEZE (and its placeholder),
#      SPEC372_CHAIN_START_EPOCH, SPEC372_HARNESS_BIN, SPEC372_A2J_FLAVOUR.
#      The SPEC365_* overrides and the smoke knobs SPEC362B_SMOKE_DURATION,
#      SPEC365_SMOKE_SAMPLE_INTERVAL and SPEC371_SMOKE_LIVE_CENSUS keep their
#      parent names, because the smoke is driven through them.
#   3. THE FREEZE LITERAL is this carve's code commit (the allocator features
#      and probes). The three refusal guards are unchanged.
#   4. ARTIFACT BASENAME spec372-<cell>, DATA DIR target/spec372-<cell>-data
#      (+ .meta sibling), port 47358 (distinct from the parent's 47357).
#   5. THE CSV HEADER gains ten columns after the parent's 45: the je_probe
#      fields je_allocated, je_active, je_resident, je_retained, je_mapped,
#      je_metadata, je_probe_elapsed_s, je_probe_seq (from the last
#      "[server] je_probe" console line), then bytes_alloc, bytes_dealloc
#      (from the last "[server] alloc_probe" line). A field the probe printed
#      as n/a, or a probe never seen, is an EMPTY cell.
#   6. THE FLAVOUR-MARKER ASSERTION covers SYS/JE/MI/CA: SYS has none of the
#      je_probe, alloc_probe, DHAT_OUT or mimalloc literals; JE has the
#      je_probe literal only; MI has the mimalloc literal only; CA has the
#      alloc_probe literal only.
#   7. CONSOLE LINE 1 carries flavour=<SYS|JE|MI|CA> (the value set changes;
#      the line's shape does not).
#   8. The parent's dead READOUT_OUT assignment is REMOVED.
#   9. PE's row count is derived from matrix.txt (N = D/cadence + 1). PE lives
#      in spec372-predicates.sh, so this item has NO hunk in this file; it is
#      listed so the closed list reads the same everywhere.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE NINE ITEMS IS A DEFECT. The
# pre-registered manifest carries the hunk-to-item map. The departure is
# enumerable with:
#   diff spec371-memdiag.sh spec372-allocdiag.sh
#
# Everything from here on is spec371-memdiag.sh's own header, kept verbatim
# so the lineage stays readable.
#
# Memory-diagnosis cell runner -- a copy of spec370-plateau4h.sh, which is NOT
# edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT RUN THESE CELLS. They launch
# six different server builds (plain release, count-alloc, dhat), none of which
# the parent may build, they need the count-alloc probe carried into the CSV,
# they vary the event journal and the teardown signal per cell, and their
# artifacts must not overwrite the parent's committed evidence. The difference
# list against spec370-plateau4h.sh is CLOSED at exactly sixteen items:
#
#   1. THE CELL TABLE. Six cells -- r0 (plain release), c0/c1/c2 (count-alloc;
#      c1 turns the event journal off), c3e/c3l (dhat, 300 s and 900 s). Every
#      cell is 900 s at a 60 s cadence (c3e: 300 s), crash-interval 0, width
#      unset, the live-copy census ARMED at 300 s, the log directive armed.
#      Each row names its server FLAVOUR (R|CA|DH). r0 is a conductor scope
#      choice: a same-chain release reference slope and release-regime AMP.
#   2. ENV NAMES. The freeze variable becomes SPEC371_CODE_FREEZE; the chain
#      supplies SPEC371_CHAIN_START_EPOCH and SPEC371_HARNESS_BIN. The
#      SPEC365_OUT_DIR / SPEC365_DATA_DIR / SPEC365_FORCE overrides and the two
#      smoke knobs SPEC362B_SMOKE_DURATION / SPEC365_SMOKE_SAMPLE_INTERVAL keep
#      their parent names, because the smoke is driven through them.
#   3. THE FREEZE LITERAL is the commit that lets a cell turn the journal off
#      (the harness env passthrough). The three refusal guards are unchanged.
#   4. DATA DIR target/spec371-<cell>-data (+ .meta sibling); port 47357.
#   5. EVERY CELL IS A PROVENANCE CELL AND THE RUNNER BUILDS NOTHING.
#      SOAK_SERVER_BINARY and SPEC371_HARNESS_BIN are REQUIRED; both freshness
#      clauses (b) compare the binary's mtime with SPEC371_CHAIN_START_EPOCH
#      instead of this invocation's start, because the chain builds once for
#      six cells. The "UNPROVEN override" wording is gone: identity is the
#      sha256 on console line 1, matched to spec371-builds.txt downstream.
#   6. CONSOLE LINE 1 carries flavour=<R|CA|DH> after the server sha; the
#      matrix echoes the flavour, both shas and the per-cell env values.
#   7. THE CSV HEADER gains four columns after the parent's 41:
#      alloc_live_bytes, alloc_live_mb, alloc_probe_elapsed_s, alloc_probe_seq,
#      filled from the last "[server] alloc_probe" console line (empty when no
#      probe has been seen -- always, on the R and DH builds).
#   8. POST-RUN: a dhat cell gzips its profile into the artifact dir; the
#      parent's spec365-readout.sh invocation is DROPPED.
#   9. This header and the usage text.
#  10. THE PER-CELL ENV BLOCK: TOPGUN_JOURNAL_ENABLED (false on c1, unset
#      elsewhere), TOPGUN_SOAK_GRACEFUL_SHUTDOWN (1 on c3e/c3l, unset
#      elsewhere), DHAT_OUT (absolute, dhat cells only) -- replacing the
#      parent's unconditional unset of the graceful switch.
#  11. SPEC370_BASE_SUFFIX is removed; it or SPEC371_BASE_SUFFIX being set is
#      REFUSED.
#  12. A FLAVOUR-MARKER ASSERTION on SOAK_SERVER_BINARY before the clock: R has
#      neither the probe literal nor DHAT_OUT, CA has the probe literal, DH has
#      DHAT_OUT; and the harness must carry the journal-echo literal.
#  13. A MISSING OR EMPTY dhat profile after a dhat cell is an INSTRUMENT
#      DEFECT.
#  14. The parent's BUILD_GAP warning (and the two mtime assignments feeding
#      it) is REMOVED; the "soak binary:"/"server binary:" echoes stay.
#  15. A SMOKE-ONLY live-census override, SPEC371_SMOKE_LIVE_CENSUS, honoured
#      only in smoke mode and refused otherwise.
#  16. export LC_ALL=C right after `set -euo pipefail`: the sampler's awk and
#      printf "%.3f" must not follow a comma-decimal shell locale.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE SIXTEEN ITEMS IS A DEFECT. The
# pre-registered manifest carries the hunk-to-item map. The departure is
# enumerable with:
#   diff spec370-plateau4h.sh spec371-memdiag.sh
#
# Everything from here on is spec370-plateau4h.sh's own header, kept verbatim
# so the lineage stays readable.
#
# Level-ceiling cell runner -- a copy of spec368-plateau4h.sh, which is NOT
# edited.
#
# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT BE RUN FOR THIS CELL. Its
# freeze literal names another commit, the harness it launches must now carry
# the level-ceiling tombstone gate, and its artifact basename names files that
# are committed evidence a re-run must not overwrite. This file is therefore a
# COPY, and the difference list against spec368-plateau4h.sh is CLOSED at
# exactly seven items:
#
#   1. THE ARTIFACT BASENAME IS spec370-plateau4h, extended by the value of
#      SPEC370_BASE_SUFFIX. With that variable unset -- the only supported
#      state for a first sample -- the basename resolves to
#      spec370-plateau4h. A replicate exports the suffix as -r2, the basename
#      resolves to spec370-plateau4h-r2, and a second sample therefore needs
#      NO edit to this file. This runner does not validate the suffix; the
#      chain that launches it does.
#   2. THE ENV NAMES THIS LINEAGE OWNS ARE RENAMED: the freeze variable
#      becomes SPEC370_CODE_FREEZE, its placeholder string is renamed with it,
#      and the suffix variable becomes SPEC370_BASE_SUFFIX, at every site. THE
#      SPEC365_* OVERRIDE NAMES AND SPEC362B_SMOKE_DURATION STAY VERBATIM: the
#      readout below is the UNCHANGED spec365-readout.sh and resolves its own
#      OUT_DIR from one of them, so renaming them would point the runner and
#      the readout at DIFFERENT directories whenever a scratch out dir is used.
#   3. THE FREEZE LITERAL IS THIS CELL'S OWN FREEZE COMMIT, b13afaed -- the
#      last commit that touches a .rs file, where the level-ceiling gate is
#      wired into the harness. The three refusal guards -- the placeholder
#      check, the .rs diff against the freeze commit, the dirty .rs working
#      tree -- are unchanged, and none has an override.
#   4. THE DEFAULT DATA DIR is target/spec370-<cell>-data with its sibling
#      .meta dir, so a run of this file can neither land in nor collide with
#      the parent cell's data dir. The cell id stays plateau4h.
#   5. this header and the usage text, naming this runner, its cell and this
#      list.
#   6. HARNESS PROVENANCE. One block, inserted immediately after the soak
#      binary is resolved, hashes that binary and refuses to start unless (a)
#      it carries the level-ceiling gate's own message literal, and (b) it was
#      built by this invocation -- clause (b) is waived only under the
#      SPEC365_SOAK_BIN override, which the matrix already marks UNPROVEN. The
#      block then extends the console's provenance line with the harness
#      sha256, its build time and the gate marker, so console line 1 carries
#      BOTH launched binaries' identities.
#   7. THE MATRIX ECHOES THE GATE: two added lines name the tombstone gate
#      this harness applies and repeat the harness sha256, which the
#      predicates compare with console line 1.
#
# EVERYTHING ELSE IS BYTE-IDENTICAL to spec368-plateau4h.sh: the four-hour
# duration, the 60s cadence, crash-interval 0, the width left unset, the log
# directive, the armed prune record, the environment-discipline block, the
# server provenance clauses, the fresh-data-dir guard, the artifact-overwrite
# refusal, scrape persistence, the CSV header, the harness invocation (memory
# gate still neutralized), the post-run checks and fits, and the readout
# invocation. The departure is enumerable with:
#   diff spec368-plateau4h.sh spec370-plateau4h.sh
#
# A DIFF HUNK THAT MAPS TO NONE OF THE SEVEN ITEMS IS A DEFECT, not a
# footnote: the pre-registered manifest carries this diff and a hunk-to-item
# map, and a runner whose departure cannot be enumerated cannot be filed under
# a freeze. spec368-plateau4h.sh's own six-item list against its parent stays
# readable in that file.
#
# Everything from here on is spec365-conjuncts.sh's own header, kept verbatim
# so the lineage back to spec362b-durable.sh stays readable.
#
# Derived from the committed spec362b-durable.sh. THE RATE-, SHAPE- AND
# DURATION-DETERMINING MATRIX IS IDENTICAL to that file's 4-hour deciding
# cell, and the ONLY permitted differences are the ones enumerated below.
# No parent runner is edited; the difference list against spec362b-durable.sh
# is CLOSED:
#
#   (a) IDENTITY. Artifact prefix spec365-conj900 and, with it, the renamed
#       SPEC365_* env-var override surface -- the COMPLETE renamed variable
#       surface, the three dead provenance-branch echo variables included --
#       the run's data dir, and the matrix banner. ONE override keeps the
#       parent's literal name rather than being renamed: SPEC362B_SMOKE_DURATION
#       (see (c) below). The parent's own commit-pin variable is not merely
#       renamed but restructured; see (g).
#   (b) THE CELL. A single label, conj900: 900s duration, 60s CSV cadence,
#       width unset (production default), every other matrix literal is the
#       parent's 4-hour-cell literal. --durable-reading stays armed, as on
#       every cell this lineage ships.
#   (c) THE INSTRUMENT IS ARMED. TOPGUN_PRUNE_RECORD is exported true, and the
#       target-scoped log directive widens from the parent's one target
#       (removal) to three (removal, settlement, conjunct), armed together so
#       a reconciliation reader sees all three kinds of line from one run. The
#       harness's own log-passthrough switch is set so those lines land in the
#       committed console log this file's readout reads. The residency target
#       is deliberately NOT armed here: it fires once per OR write, and this
#       cell does not need a per-write log. For the short local smoke this
#       runner's own conj900 cell is not built to carry, the smoke override
#       knob keeps the parent's literal name (SPEC362B_SMOKE_DURATION) rather
#       than defining a new one, so smoke tooling written against the parent
#       needs no relearning.
#   (d) ONE SCRAPE PER ROW. The sampler's one HTTP GET against /metrics now
#       supplies both the inherited tombstone-bytes gauge and every new
#       counter/gauge column this file adds, parsed from that single response
#       body. A name absent from the body renders an EMPTY cell, never a
#       zero -- a gap in a series must stay visible as a gap.
#   (e) A PER-ROW PROCESS MEMORY BREAKDOWN. After the existing ps-based RSS
#       read, one additional per-process sample is taken and parsed into four
#       more columns. Its parser was written and exercised against a
#       committed real capture before this runner was written, so field-name
#       drift in that external tool's output is caught before the CSV header
#       promises columns nothing can fill.
#   (f) THE HEADER WIDENS. The CSV gains new columns to the right of the
#       inherited six; the inherited six stay byte-identical in content and
#       position.
#   (g) THE FREEZE GATE REPLACES THE PIN. The parent's commit-pin literal
#       becomes SPEC370_CODE_FREEZE here, with one added behaviour the parent
#       never needed: while this literal still reads its own placeholder
#       value (because the code it would pin does not exist yet), the runner
#       refuses immediately, before any git call that would otherwise need to
#       resolve a nonsense revision. Once a real commit lands in its place,
#       the gate behaves exactly like the parent's pin: a fail-closed
#       pre-flight against a dirty or drifted .rs tree, asserted before any
#       build.
#   (h) A READOUT RUN. After a sound instrument run, this file additionally
#       invokes the companion mechanical readout on this cell's own
#       artifacts.
#
# NO OTHER RATE- OR SHAPE-DETERMINING LITERAL CHANGES. The server port, the
# environment-discipline block, the build-from-HEAD step and its dirty-.rs
# guard, the artifact-overwrite refusal, the smoke-mode refusal into the
# tracked evidence dir, the six inherited CSV columns' sampler logic, the
# post-run instrument checks and the spec349c2-fit.awk fits are all carried
# over unchanged in shape, from spec362b-durable.sh. The departure is
# enumerable with:
#   diff spec362b-durable.sh spec365-conjuncts.sh
#
# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC370_CODE_FREEZE names the commit
# at which the instrument this file samples was complete and the full gate
# matrix green. The runner refuses to start if the literal ever reads its
# placeholder again, or if any .rs file at HEAD differs from that commit, so
# the series it records can only come from the frozen instrument.
#
# THE MATRIX IS EXECUTED, NOT TRANSCRIBED. Every knob is a literal in this
# file, so the record of what was run is this committed script rather than an
# operator's memory of a command line.
#
# Bash 3.2 (macOS system bash) compatible: no mapfile, no associative arrays,
# no ${x@Q}.
#
# Env overrides (all documented, all logged loudly when active):
#   SPEC365_DATA_DIR                data dir for this run (default: target/)
#   SPEC365_OUT_DIR                 artifact dir     (default: this script's dir)
#   SPEC365_SOAK_BIN                prebuilt soak_harness bench binary
#   SPEC362B_SMOKE_DURATION         SMOKE ONLY: override --duration (seconds);
#                                    kept under the parent's literal name, see (c)
#   SPEC365_SMOKE_SAMPLE_INTERVAL   SMOKE ONLY: override the CSV cadence
#   SPEC365_FORCE=1                 overwrite pre-existing artifacts
#   SOAK_SERVER_BINARY              REQUIRED on a provenance cell, REFUSED
#                                    elsewhere (see section 0b); no cell in
#                                    this file's table is a provenance cell
#
set -euo pipefail
export LC_ALL=C

# Item 1: the memory sampler, pid resolution and host sidecar below read
# /proc and Linux tools; on any other kernel they would run blind, so refuse
# before anything is created.
if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "FATAL: spec376-cells.sh runs on Linux only (uname -s = '$(uname -s 2>/dev/null || true)')" >&2
  exit 2
fi

# ---------------------------------------------------------------------------
# 0. Argument: the CELL id. Every output path, the width, the duration and the
#    provenance discipline are derived from it. There is no free-form knob: a
#    cell that is not in this table cannot be run.
# ---------------------------------------------------------------------------
CELL="${1:-}"

usage() {
  cat >&2 <<'EOF'
usage: spec376-cells.sh <cell>

  A COPY of spec373b-cells.sh, which is not edited. The difference list
  against that file is CLOSED, has exactly nine items, and is enumerated in
  the header block above. Linux only.

  Every cell: crash-interval 0, live-census ARMED at 300s, log directive
  ARMED, journal default.
  Smoke (admission), 120s at csv cadence 20s:
    sc     flavour CA (count-alloc), server CAL_PIN,  SIGKILL teardown
    spb    flavour CA,               server b166719d, SIGKILL teardown
    sdh    flavour DH (dhat),        server b166719d, SIGTERM teardown
    sje    flavour JE (jemalloc),    server CAL_PIN,  SIGKILL teardown
    ssy    flavour SYS (glibc),      server CAL_PIN,  SIGKILL teardown
    smi    flavour MI (mimalloc),    server CAL_PIN,  SIGKILL teardown
  Calibration, 900s at csv cadence 60s, SIGKILL teardown:
    c1 c2  flavour CA, server CAL_PIN   (the same-binary pair)
    pb     flavour CA, server b166719d  (portability control, before)
    pa     flavour CA, server e85adb1f  (portability control, after)
  Every other matrix literal is spec373b-cells.sh's own.

  Any other argument falls through to this text and exits 2.

  The run REFUSES TO START, before any clock, if CAL_PIN still reads its
  own placeholder value, or unless the .rs tree at HEAD is identical to the
  commit that literal names, or unless the .rs working tree is clean. Three
  guards, three distinct messages; none has an override. It also refuses,
  before any clock, a soak harness binary that lacks the level-ceiling gate
  or the journal echo, or that predates the chain start.

  EVERY cell is a provenance cell: SOAK_SERVER_BINARY (the cell's flavour),
  SPEC376_SERVER_COMMIT (the code state it was built from),
  SPEC376_HARNESS_BIN and SPEC376_CHAIN_START_EPOCH are REQUIRED, and are
  exported by spec376-chain.sh. This runner builds nothing.
EOF
  exit 2
}

# The per-cell literals. Columns:
#   width      -- "" means UNSET, i.e. the PRODUCTION default (1000)
#   duration   -- seconds
#   cadence    -- CSV sample interval, seconds
#   provenance -- yes|no
#   crash      -- --crash-interval literal (0 => None: no kill -9 during the run)
#   live       -- the live-copy census sampler's cadence in seconds; 0 leaves
#                 that sampler DISARMED
#   armlog     -- yes|no: SOAK_SERVER_LOG carries the removal/settlement/conjunct
#                 target directive
#   extra      -- extra harness flags
#   base       -- artifact basename
#   flavour    -- CA|DH|JE|SYS|MI: the server build the cell must launch
#   journal    -- off|default: TOPGUN_JOURNAL_ENABLED=false, or left unset
#   graceful   -- yes|no: SIGTERM teardown (dhat flushes on a clean exit)
if [ -n "${SPEC370_BASE_SUFFIX:-}" ] || [ -n "${SPEC371_BASE_SUFFIX:-}" ]; then
  echo "FATAL: a base-name suffix is set; this runner's basenames are fixed per" >&2
  echo "       cell and a smoke writes into a scratch SPEC365_OUT_DIR instead." >&2
  exit 2
fi
WIDTH=""; SAMPLE_INTERVAL=60; PROVENANCE=yes; CELL_CRASH_INTERVAL=0; ARM_LOG=yes
CELL_LIVE_CENSUS=300; EXTRA_FLAGS=""
# CELL_SERVER is the code state the cell's server must be built from (item 7):
# "cal" = CAL_PIN, "pin" = b166719d, "frz" = e85adb1f.
# CELL_PHASE smoke cells may never write into the tracked evidence dir.
case "$CELL" in
  sc)    DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=CA;  CELL_GRACEFUL=no;  CELL_SERVER=cal; CELL_PHASE=smoke ;;
  spb)   DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=CA;  CELL_GRACEFUL=no;  CELL_SERVER=pin; CELL_PHASE=smoke ;;
  sdh)   DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=DH;  CELL_GRACEFUL=yes; CELL_SERVER=pin; CELL_PHASE=smoke ;;
  sje)   DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=JE;  CELL_GRACEFUL=no;  CELL_SERVER=cal; CELL_PHASE=smoke ;;
  ssy)   DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=SYS; CELL_GRACEFUL=no;  CELL_SERVER=cal; CELL_PHASE=smoke ;;
  smi)   DURATION=120; SAMPLE_INTERVAL=20; FLAVOUR=MI;  CELL_GRACEFUL=no;  CELL_SERVER=cal; CELL_PHASE=smoke ;;
  c1|c2) DURATION=900; FLAVOUR=CA; CELL_GRACEFUL=no; CELL_SERVER=cal; CELL_PHASE=cal ;;
  pb)    DURATION=900; FLAVOUR=CA; CELL_GRACEFUL=no; CELL_SERVER=pin; CELL_PHASE=cal ;;
  pa)    DURATION=900; FLAVOUR=CA; CELL_GRACEFUL=no; CELL_SERVER=frz; CELL_PHASE=cal ;;
  *)    usage ;;
esac
CELL_JOURNAL=default
BASE="spec376-${CELL}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"     # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd)"

# Item 2: the memory sampler is sourced before any clock, so a missing or
# broken helper refuses the cell instead of blinding it mid-run.
PROCMEM_LIB="${SCRIPT_DIR}/spec376-procmem.sh"
if [ ! -f "$PROCMEM_LIB" ]; then
  echo "FATAL: memory sampler helper not found: $PROCMEM_LIB" >&2
  exit 1
fi
# shellcheck source=spec376-procmem.sh
. "$PROCMEM_LIB"
if ! declare -F procmem_row > /dev/null; then
  echo "FATAL: $PROCMEM_LIB does not define procmem_row" >&2
  exit 1
fi

OUT_DIR="${SPEC365_OUT_DIR:-$EVIDENCE_DIR}"
CSV_OUT="${OUT_DIR}/${BASE}.csv"
JSON_OUT="${OUT_DIR}/${BASE}.soak.json"
PROGRESS_OUT="${OUT_DIR}/${BASE}.progress.jsonl"
# The effective-matrix echo, committed beside the series it describes.
MATRIX_OUT="${OUT_DIR}/${BASE}.matrix.txt"
# The harness derives the mechanism report's path from --json-output via Rust's
# `Path::with_extension`, which replaces the LAST extension only: it writes
# "<base>.soak.mechanism.json". The ledger for these runs names
# "<base>.mechanism.json", so the runner normalizes the name after the run
# rather than leaving the committed tree disagreeing with the ledger.
MECH_RAW="${OUT_DIR}/${BASE}.soak.mechanism.json"
MECH_OUT="${OUT_DIR}/${BASE}.mechanism.json"
# The durable reading's sibling artifact, derived the same way. It is
# DELIBERATELY NOT given the symmetric rename the mechanism report gets: the
# readout reads this raw basename, and a normalize hunk here would silently
# break that.
DURABLE_OUT="${OUT_DIR}/${BASE}.soak.durable.json"
# The harness's own stdout, committed IN THE EVIDENCE DIRECTORY.
CONSOLE_OUT="${OUT_DIR}/${BASE}.harness-console.log"

# Item 5: this invocation's start, recorded BEFORE anything is built. The
# provenance clause below compares the server binary's mtime against it, so the
# timestamp has to predate the build or the comparison proves nothing.
RUN_START_EPOCH="$(date +%s)"
RUN_START_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
# Every /metrics body this run scrapes, kept verbatim, one file per sample.
# P6/P7 are decided on a RAW body taken outside every open prune window, and no
# CSV row can carry one: the row is 31 numbers awk-ed out of a body that today
# is discarded.
SCRAPES_DIR="${OUT_DIR}/${BASE}.scrapes"

# ---------------------------------------------------------------------------
# 0b. FAIL-CLOSED SERVER-BINARY RESOLUTION -- the provenance guard.
#
#     This is a REFUSAL, not a knob: it determines nothing about rate, shape or
#     duration. It exists because the Rust resolver fails OPEN. With
#     SOAK_SERVER_BINARY absent from the bench process's environment,
#     `resolve_server_binary()` silently returns the bench's compile-time
#     default server path with no existence check and no warning -- so a
#     variable exported in the wrong subshell, or a worktree whose
#     `cargo build` landed in the shared target/, turns the provenance cell
#     into a second cell A while every console line still looks correct.
#
#     A cell that silently measures the wrong binary is the one failure that
#     looks most like success, so on the provenance path this runner refuses
#     to start unless the variable is set to an existing executable file.
#     There is deliberately NO fallback here.
# ---------------------------------------------------------------------------
if [ "$PROVENANCE" = "yes" ]; then
  if [ -z "${SOAK_SERVER_BINARY:-}" ]; then
    echo "FATAL: cell '${CELL}' is a PROVENANCE cell and SOAK_SERVER_BINARY is unset." >&2
    echo "       Refusing to run: the Rust resolver fails OPEN and would silently" >&2
    echo "       use the HEAD binary the bench was compiled beside, making this" >&2
    echo "       cell a second cell A." >&2
    echo "       Export SOAK_SERVER_BINARY=<worktree>/target/release/topgun-server" >&2
    exit 3
  fi
  if [ ! -f "$SOAK_SERVER_BINARY" ] || [ ! -x "$SOAK_SERVER_BINARY" ]; then
    echo "FATAL: SOAK_SERVER_BINARY does not name an existing executable file:" >&2
    echo "       '${SOAK_SERVER_BINARY}'" >&2
    exit 3
  fi
  export SOAK_SERVER_BINARY
  PROV_BIN="$SOAK_SERVER_BINARY"
else
  if [ -n "${SOAK_SERVER_BINARY:-}" ]; then
    echo "WARNING: SOAK_SERVER_BINARY was exported ('${SOAK_SERVER_BINARY}') but cell" >&2
    echo "         '${CELL}' is NOT a provenance cell. Unsetting it: only a" >&2
    echo "         provenance cell may vary the server binary." >&2
  fi
  unset SOAK_SERVER_BINARY || true
  PROV_BIN=""
fi

DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec376-${CELL}-data}"
META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
CONSOLE_LOG="${META_DIR}/harness-console.log"
STOP_FILE="${META_DIR}/sampler.stop"
PM_FILE="${META_DIR}/sampler.post_mortem_rows"   # item 17: the sampler runs in a
                                                 # subshell, so its counter lives in a file
FAIL_FILE="${META_DIR}/sampler.fail"
# Item 2: the memory sampler's two counters, files for the same subshell
# reason as PM_FILE.
PROCMEM_INV_FILE="${META_DIR}/sampler.mem_invariant_violations"
PROCMEM_PM_FILE="${META_DIR}/sampler.post_mortem_mem_reads"

# ---------------------------------------------------------------------------
# 2. Smoke override -- only ever changes the duration (and, with it, the CSV
#    cadence, which would otherwise yield 3 rows). The real run must not be
#    able to take this path by accident, so the defaults are the per-cell
#    literals above, the override is loud, and it is REFUSED if the artifacts
#    would land in the tracked evidence directory.
# ---------------------------------------------------------------------------
SMOKE=0
if [ -n "${SPEC362B_SMOKE_DURATION:-}" ]; then
  SMOKE=1
  DURATION="$SPEC362B_SMOKE_DURATION"
  if [ -n "${SPEC365_SMOKE_SAMPLE_INTERVAL:-}" ]; then
    SAMPLE_INTERVAL="$SPEC365_SMOKE_SAMPLE_INTERVAL"
  fi
  echo "############################################################"
  echo "## SMOKE MODE -- THIS IS NOT A CHARACTERIZATION RUN       ##"
  echo "##   --duration      = ${DURATION}s (override)            "
  echo "##   CSV cadence     = ${SAMPLE_INTERVAL}s                 "
  echo "## Its artifacts MUST NOT be committed as evidence.       ##"
  echo "############################################################"
  if [ "$OUT_DIR" = "$EVIDENCE_DIR" ]; then
    echo "REFUSING: smoke mode would write into the tracked evidence dir" >&2
    echo "  $EVIDENCE_DIR" >&2
    echo "Set SPEC365_OUT_DIR to a scratch directory." >&2
    exit 2
  fi
elif [ -n "${SPEC365_SMOKE_SAMPLE_INTERVAL:-}" ]; then
  echo "WARNING: SPEC365_SMOKE_SAMPLE_INTERVAL ignored outside smoke mode;" >&2
  echo "         this cell's CSV cadence is pinned at ${SAMPLE_INTERVAL}s." >&2
fi
if [ -n "${SPEC371_SMOKE_LIVE_CENSUS:-}" ]; then
  if [ "$SMOKE" != "1" ]; then
    echo "FATAL: SPEC371_SMOKE_LIVE_CENSUS is a smoke-only knob; the committed cells" >&2
    echo "       always run --live-census-interval ${CELL_LIVE_CENSUS}." >&2
    exit 2
  fi
  CELL_LIVE_CENSUS="$SPEC371_SMOKE_LIVE_CENSUS"
fi

if [ "$CELL_PHASE" = "smoke" ] && [ "$OUT_DIR" = "$EVIDENCE_DIR" ]; then
  echo "REFUSING: smoke cell ${CELL} would write into the tracked evidence dir" >&2
  echo "  $EVIDENCE_DIR" >&2
  echo "Set SPEC365_OUT_DIR to the chain's smoke OUT dir." >&2
  exit 2
fi
if [ "$OUT_DIR" != "$EVIDENCE_DIR" ]; then
  echo "WARNING: artifact dir overridden to $OUT_DIR"
  echo "         A characterization run's artifacts belong in $EVIDENCE_DIR"
  echo "         (git-tracked); artifacts written elsewhere are not evidence."
fi

# ---------------------------------------------------------------------------
# 3. Environment discipline.
#
#    The harness spawns the real topgun-server as a child and sets a fixed
#    block of env on it, WITHOUT env_clear() -- so the child also inherits this
#    shell's environment. Anything below that were already exported in an
#    operator's shell would silently change what this run measures, so they
#    are ACTIVELY UNSET rather than merely not set.
# ---------------------------------------------------------------------------
if [ -n "$WIDTH" ]; then
  export TOPGUN_EPOCH_WIDTH="$WIDTH"
else
  # Unset => the server applies the PRODUCTION default epoch width (1000).
  unset TOPGUN_EPOCH_WIDTH || true
fi
# Unset => the harness supplies its own 100ms / 5000 write-behind cadence, which
# is the instrument-identity choice shared with every other soak run.
unset TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS || true
unset TOPGUN_WRITEBEHIND_BATCH_SIZE || true
# The target-scoped log directive: `warn` for everything, `info` for the
# removal, settlement and conjunct targets, all three armed together so one
# run's console log carries every line kind the readout joins.
ORIGIN_LOG_DIRECTIVE='warn,topgun_server::tombstone_frontier::removal=info,topgun_server::tombstone_frontier::settlement=info,topgun_server::tombstone_frontier::conjunct=info'
if [ "$ARM_LOG" = "yes" ]; then
  export SOAK_SERVER_LOG="$ORIGIN_LOG_DIRECTIVE"
  # The harness mirrors matched lines to its own stderr only under this
  # switch, and the harness's stdout+stderr are both redirected into
  # CONSOLE_LOG below -- so this is what makes the [server] -prefixed lines
  # the readout parses actually land in the committed console log.
  export SOAK_SERVER_LOG_PASSTHROUGH=1
else
  unset SOAK_SERVER_LOG || true
  unset SOAK_SERVER_LOG_PASSTHROUGH || true
fi
# Arms the scrape-time snapshot's metric family. The family's own default is
# already armed when this is unset, but the run states it explicitly rather
# than leaning on that default, matching every other line in this block.
export TOPGUN_PRUNE_RECORD=true
# Unset => the fmt layer renders the human format the origin parser reads.
unset TOPGUN_LOG_FORMAT || true
# Unset => teardown is SIGKILL, as every other soak run's is.
if [ "$CELL_GRACEFUL" = "yes" ]; then
  export TOPGUN_SOAK_GRACEFUL_SHUTDOWN=1
else
  unset TOPGUN_SOAK_GRACEFUL_SHUTDOWN || true
fi
if [ "$CELL_JOURNAL" = "off" ]; then
  export TOPGUN_JOURNAL_ENABLED=false
else
  unset TOPGUN_JOURNAL_ENABLED || true
fi
if [ "$FLAVOUR" = "DH" ]; then
  export DHAT_OUT="${META_DIR}/${BASE}.dhat.json"
else
  unset DHAT_OUT || true
fi
unset TOPGUN_JOURNAL_CAPACITY || true
# Unset => production memory ceiling and eviction water marks. Eviction cadence
# is not a thing this cell varies.
unset TOPGUN_MAX_RAM_MB || true
unset TOPGUN_EVICTION_HIGH_PCT || true
unset TOPGUN_EVICTION_LOW_PCT || true
unset TOPGUN_EVICTION_INTERVAL_MS || true
# Unset => the emitter is ARMED, the shipped default.
unset TOPGUN_OR_DELTA_WAL || true

# NOTE: TOPGUN_WAL_FSYNC_POLICY is deliberately NOT managed here. The harness
# OVERWRITES it on the child unconditionally from --wal-fsync, so exporting it
# does nothing at all; the policy comes from the flag below and is recorded in
# soak.json as `walFsync`.
if [ -n "${TOPGUN_WAL_FSYNC_POLICY:-}" ]; then
  echo "note: TOPGUN_WAL_FSYNC_POLICY='${TOPGUN_WAL_FSYNC_POLICY}' is inherited but IGNORED"
  echo "      (the harness overwrites it on the child from --wal-fsync)"
fi

# ---------------------------------------------------------------------------
# 1. The pinned matrix. Byte-identical to spec362b-durable.sh's long4h cell,
#    except CRASH_INTERVAL, which is per-cell and 0 on both files' one
#    shipped cell anyway.
# ---------------------------------------------------------------------------
CHURN_CLIENTS=6
KEYSPACE=200            # LWW keyspace
OR_CHURN=true
OR_KEYSPACE=48          # 48 % 6 == 0 satisfies the harness's single-writer assert
OR_EVERY=5
WRITE_INTERVAL_MS=20
WRITES_PER_LIFE=200
OFFLINE_KEYS=3
CONFIRM_INTERVAL=2      # keeps the low-water-mark advancing so the prune fires
CRASH_INTERVAL="$CELL_CRASH_INTERVAL"
STEADY_INTERVAL=300     # ~1% of the run spent quiesced instead of ~10%
QUIESCE=3
MEM_SAMPLE_INTERVAL=5
WAL_FSYNC=batched
# The harness's live memory gate is NEUTRALIZED, not sharpened: this cell's
# verdict is read from committed artifacts by a separate mechanical readout,
# and at the shipped defaults the live gate fires on every run in this regime.
MEM_MIN_GROWTH_MB=1000000
MEM_THRESHOLD_MB_PER_HOUR=1000000
MEM_CEILING_MB=1000000
SERVER_PORT=47376       # fixed, distinct from every macOS runner's port
                         # (47355, 47357-47360) so no two runners' cells
                         # collide on one host
JITTER_SEED=20260831

# ---------------------------------------------------------------------------
# 4. Pre-flight. Fail closed BEFORE the clock starts.
# ---------------------------------------------------------------------------
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
  if [ -d "${REPO_ROOT}/target/release" ]; then
    TARGET_DIR="${REPO_ROOT}/target"
  else
    TARGET_DIR="${SERVER_ROOT}/target"
  fi
fi

# ---------------------------------------------------------------------------
# 4a. THE FREEZE GATE. This spec's readout is pre-registered before data, and
#     the runner and readout are not edited after that pre-registration. So
#     the commit at which the instrument this file samples is complete and
#     green is recorded here as a literal, exactly as the parent recorded its
#     own commit pin -- with one behaviour the parent never needed: while this
#     literal still names its own placeholder, the instrument does not exist
#     yet, and the run refuses immediately rather than attempting to diff
#     against a value that is not a revision.
# ---------------------------------------------------------------------------
CAL_PIN=bee21fcd
if [ "$CAL_PIN" = "PENDING_CAL_PIN" ]; then
  echo "FATAL: CAL_PIN still reads its placeholder value." >&2
  echo "       The prune-conjunct instrument this runner samples is not yet" >&2
  echo "       committed under a named freeze commit. Refusing to start." >&2
  exit 2
fi
SOAK_BIN_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
if [ -z "$SOAK_BIN_COMMIT" ]; then
  echo "FATAL: $REPO_ROOT is not a git checkout, so this run cannot be pinned" >&2
  echo "       to the commit that produced it. Refusing to start." >&2
  exit 1
fi
if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$CAL_PIN"..HEAD -- '*.rs' 2>&1)"; then
  echo "FATAL: the .rs diff against the freeze commit ${CAL_PIN} could" >&2
  echo "       not be computed, so the freeze cannot be asserted:" >&2
  printf '%s\n' "$FREEZE_RS_DIFF" >&2
  echo "       Refusing to start." >&2
  exit 1
fi
if [ -n "$FREEZE_RS_DIFF" ]; then
  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${CAL_PIN}; this run would not be filed under it" >&2
  printf '%s\n' "$FREEZE_RS_DIFF" >&2
  exit 1
fi
FREEZE_DIFF_STATE="EMPTY (asserted before the build)"
RS_DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- '*.rs' 2>/dev/null || true)"
if [ -n "$RS_DIRTY" ]; then
  echo "FATAL: the .rs working tree is DIRTY, so the binary this run would" >&2
  echo "       build is not the commit it would be filed under:" >&2
  printf '%s\n' "$RS_DIRTY" >&2
  echo "       Commit or stash the .rs changes and re-run." >&2
  exit 1
fi
RS_TREE_STATE="CLEAN (asserted before the build)"

# This runner builds nothing: spec376-chain.sh builds every flavour once, into
# its own target dir, and hands the paths over. Both are required.
if [ -z "${SPEC376_HARNESS_BIN:-}" ] || [ ! -x "${SPEC376_HARNESS_BIN}" ]; then
  echo "FATAL: SPEC376_HARNESS_BIN must name the chain-built soak_harness binary." >&2
  exit 3
fi
case "${SPEC376_CHAIN_START_EPOCH:-}" in
  ''|*[!0-9]*)
    echo "FATAL: SPEC376_CHAIN_START_EPOCH must be the chain's integer start epoch." >&2
    exit 3
    ;;
esac
# Item 7: the server's code state. The chain builds every server from a clean
# checkout at a named commit; this cell refuses one built from anything else.
case "$CELL_SERVER" in
  cal)    EXPECT_SERVER_COMMIT="$CAL_PIN" ;;
  pin)    EXPECT_SERVER_COMMIT=b166719d ;;
  frz)    EXPECT_SERVER_COMMIT=e85adb1f ;;
esac
EXPECT_SERVER_SHA="$(git -C "$REPO_ROOT" rev-parse --verify "${EXPECT_SERVER_COMMIT}^{commit}" 2>/dev/null || true)"
GOT_SERVER_SHA="$(git -C "$REPO_ROOT" rev-parse --verify "${SPEC376_SERVER_COMMIT:-none}^{commit}" 2>/dev/null || true)"
if [ -z "$EXPECT_SERVER_SHA" ] || [ "$GOT_SERVER_SHA" != "$EXPECT_SERVER_SHA" ]; then
  echo "FATAL: cell ${CELL} needs a server built at ${EXPECT_SERVER_COMMIT}; SPEC376_SERVER_COMMIT='${SPEC376_SERVER_COMMIT:-}'." >&2
  exit 3
fi

if [ "$PROVENANCE" = "yes" ]; then
  SERVER_BIN="$PROV_BIN"
else
  SERVER_BIN="${TARGET_DIR}/release/topgun-server"
  if [ ! -x "$SERVER_BIN" ]; then
    echo "FATAL: release server binary not found at $SERVER_BIN" >&2
    echo "  build: (cd $SERVER_ROOT && cargo build --release --bin topgun-server --bench soak_harness)" >&2
    exit 1
  fi
fi
SERVER_BIN_SHA256="$(shasum -a 256 "$SERVER_BIN" 2>/dev/null | awk '{print $1}')"

# ---------------------------------------------------------------------------
# Item 5 -- PRE-CLOCK PROVENANCE ASSERTION.
#
# Cell attempt 1 ran a server built from the PIN and produced a complete,
# plausible artifact set: a readout, a CSV, 16 scrapes, and predicates that
# simply read FALSE. Nothing in the run said the measured binary was the wrong
# one. These two clauses are what turn that silent failure into a refusal, and
# they run BEFORE T0 so a bad binary costs nothing but a restart.
# ---------------------------------------------------------------------------
PROV_COUNTER="topgun_or_prune_restored_cancelled_total"

# The server must be the flavour this cell names. `grep -c`, not `grep -q`,
# for the SIGPIPE reason given at clause (a) below.
# MI's literal was read off the first MI build, never predicted; SYS is
# identified by the ABSENCE of every other flavour's literal.
MI_MARKER_LITERAL='mimalloc: warning: '
PROBE_HITS="$(strings "$SERVER_BIN" | grep -c 'alloc_probe elapsed_s=' || true)"
DHAT_HITS="$(strings "$SERVER_BIN" | grep -c 'DHAT_OUT' || true)"
JE_HITS="$(strings "$SERVER_BIN" | grep -c 'je_probe elapsed_s=' || true)"
MI_HITS="$(strings "$SERVER_BIN" | grep -cF "$MI_MARKER_LITERAL" || true)"
case "$FLAVOUR" in
  JE)  [ "${JE_HITS:-0}" -gt 0 ] && [ "${PROBE_HITS:-0}" -eq 0 ] && [ "${DHAT_HITS:-0}" -eq 0 ] && [ "${MI_HITS:-0}" -eq 0 ] ;;
  CA)  [ "${PROBE_HITS:-0}" -gt 0 ] && [ "${DHAT_HITS:-0}" -eq 0 ] && [ "${JE_HITS:-0}" -eq 0 ] && [ "${MI_HITS:-0}" -eq 0 ] ;;
  DH)  [ "${DHAT_HITS:-0}" -gt 0 ] && [ "${PROBE_HITS:-0}" -eq 0 ] && [ "${JE_HITS:-0}" -eq 0 ] && [ "${MI_HITS:-0}" -eq 0 ] ;;
  MI)  [ "${MI_HITS:-0}" -gt 0 ] && [ "${PROBE_HITS:-0}" -eq 0 ] && [ "${DHAT_HITS:-0}" -eq 0 ] && [ "${JE_HITS:-0}" -eq 0 ] ;;
  SYS) [ "${PROBE_HITS:-0}" -eq 0 ] && [ "${DHAT_HITS:-0}" -eq 0 ] && [ "${JE_HITS:-0}" -eq 0 ] && [ "${MI_HITS:-0}" -eq 0 ] ;;
  *)   false ;;
esac || {
  echo "FATAL: SOAK_SERVER_BINARY is not a ${FLAVOUR} build (alloc_probe hits=${PROBE_HITS:-0}, DHAT_OUT hits=${DHAT_HITS:-0}, je_probe hits=${JE_HITS:-0}, mimalloc hits=${MI_HITS:-0})." >&2
  echo "       binary: $SERVER_BIN" >&2
  exit 1
}

# (a) The binary must carry a symbol this branch introduces. A binary built
#     from any earlier source simply does not contain the string.
#     NOT `grep -q`: this runner is `set -euo pipefail`, and `grep -q` exits at
#     the first match, so `strings` takes SIGPIPE and the PIPELINE reports 141
#     even when the string IS present -- measured rc=141 against a binary that
#     contains it. That would fail the assertion on a CORRECT binary and refuse
#     every run. `grep -c` consumes all of its input, so the status reflects the
#     match count rather than a broken pipe.
PROV_HITS="$(strings "$SERVER_BIN" | grep -c "$PROV_COUNTER" || true)"
if [ "${PROV_HITS:-0}" -eq 0 ]; then
  echo "FATAL: the server binary does not contain '${PROV_COUNTER}'." >&2
  echo "       binary: $SERVER_BIN" >&2
  echo "       built:  $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')" >&2
  echo "       sha256: ${SERVER_BIN_SHA256:-<unavailable>}" >&2
  echo "       This counter is emitted by the branch under test, so a binary" >&2
  echo "       without it was built from other sources. Attempt 1 ran exactly" >&2
  echo "       such a binary and the cell was worthless." >&2
  exit 1
fi

# (b) It must have been produced by THIS CHAIN. A binary older than the chain's
#     start was inherited from somewhere else, which is precisely how a stale
#     artifact survives an intervening `cargo build` that judged it fresh.
SERVER_MTIME_EPOCH="$(date -r "$SERVER_BIN" '+%s')"
if [ "$SERVER_MTIME_EPOCH" -lt "$SPEC376_CHAIN_START_EPOCH" ]; then
  echo "FATAL: stale artifact -- not built by this chain." >&2
  echo "       binary: $SERVER_BIN" >&2
  echo "       built:  $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')" >&2
  echo "       run started: ${RUN_START_UTC}" >&2
  echo "       Remove it and let this runner rebuild it; do not reuse a binary" >&2
  echo "       from another tree or another run." >&2
  exit 1
fi

# (c) The identity of the measured binary travels WITH the artifacts: the same
#     sha256 the matrix records is repeated as the console log's first line.
#     The predicate awks skip any line without a timestamp prefix, so this is
#     inert to them.
PROV_LINE="provenance: server sha256=${SERVER_BIN_SHA256} flavour=${FLAVOUR} built=$(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ') run_start=${RUN_START_UTC} ${PROV_COUNTER}=present"
echo "$PROV_LINE"

# Every path that publishes the console artifact goes through this, so the
# provenance line cannot be lost on an early-exit path.
write_console_out() {
  { printf '%s\n' "$PROV_LINE"; cat "$CONSOLE_LOG"; } > "$CONSOLE_OUT" 2>/dev/null || true
}

SOAK_BIN="$SPEC376_HARNESS_BIN"

# Harness provenance. The server clauses above prove which server runs; this
# block proves the same for the soak harness, whose tombstone gate is the one
# this cell evaluates. A harness built from earlier sources would still run and
# still write a soak.json -- with the retired slope verdict and without the
# ceiling fields -- so its absence must be a refusal, not a surprise at readout.
SOAK_BIN_SHA256="$(shasum -a 256 "$SOAK_BIN" 2>/dev/null | awk '{print $1}')"
SOAK_BIN_BUILT="$(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
# (a) The harness must carry the level-ceiling gate's own message literal.
#     `grep -c`, not `grep -q`, for the SIGPIPE reason given at clause (a) above.
HARNESS_GATE_LITERAL="tombstone-byte level ceiling breached"
HARNESS_HITS="$(strings "$SOAK_BIN" | grep -c "$HARNESS_GATE_LITERAL" || true)"
if [ "${HARNESS_HITS:-0}" -eq 0 ]; then
  echo "FATAL: the soak harness binary does not contain '${HARNESS_GATE_LITERAL}'." >&2
  echo "       binary: $SOAK_BIN" >&2
  echo "       built:  ${SOAK_BIN_BUILT}" >&2
  echo "       sha256: ${SOAK_BIN_SHA256:-<unavailable>}" >&2
  echo "       The level-ceiling gate is what this cell evaluates, so a harness" >&2
  echo "       without it was built from other sources. Refusing to start." >&2
  exit 1
fi
HARNESS_JOURNAL_LITERAL="soak: child TOPGUN_JOURNAL_ENABLED="
HARNESS_J_HITS="$(strings "$SOAK_BIN" | grep -c "$HARNESS_JOURNAL_LITERAL" || true)"
if [ "${HARNESS_J_HITS:-0}" -eq 0 ]; then
  echo "FATAL: the soak harness binary does not contain '${HARNESS_JOURNAL_LITERAL}'," >&2
  echo "       so it cannot pass a journal-off cell through to the server." >&2
  exit 1
fi
# (b) It must have been produced by THIS CHAIN.
if [ "$(date -r "$SOAK_BIN" '+%s')" -lt "$SPEC376_CHAIN_START_EPOCH" ]; then
  echo "FATAL: stale soak harness binary -- not built by this chain." >&2
  echo "       binary: $SOAK_BIN" >&2
  echo "       built:  ${SOAK_BIN_BUILT}" >&2
  echo "       run started: ${RUN_START_UTC}" >&2
  exit 1
fi
# (c) Both launched binaries' identities travel on console line 1.
PROV_LINE="${PROV_LINE} harness sha256=${SOAK_BIN_SHA256} harness_built=${SOAK_BIN_BUILT} tombstone_level_ceiling_gate=present"
echo "$PROV_LINE"

echo "soak binary:   $SOAK_BIN"
echo "  built:       $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "server binary: $SERVER_BIN"
echo "  built:       $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"

# The census is only sound over a fresh corpus.
if [ -e "$DATA_DIR" ]; then
  if [ ! -d "$DATA_DIR" ]; then
    echo "FATAL: data dir path exists and is not a directory: $DATA_DIR" >&2
    exit 1
  fi
  if [ -n "$(ls -A "$DATA_DIR" 2>/dev/null || true)" ]; then
    echo "FATAL: data dir is NOT empty: $DATA_DIR" >&2
    echo "       Each run needs its own fresh, empty dir." >&2
    exit 1
  fi
fi
mkdir -p "$DATA_DIR" "$META_DIR" "$OUT_DIR"
rm -f "$STOP_FILE" "$FAIL_FILE" "$PM_FILE" "$PROCMEM_INV_FILE" "$PROCMEM_PM_FILE"

# Refuse to silently overwrite artifacts: a re-run that clobbers a recorded
# series destroys the only copy of a measurement.
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MECH_OUT" "$MECH_RAW" "$DURABLE_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  if [ -e "$f" ] && [ "${SPEC365_FORCE:-0}" != "1" ]; then
    echo "FATAL: artifact already exists: $f" >&2
    echo "       Move it aside, or re-run with SPEC365_FORCE=1 to overwrite." >&2
    exit 1
  fi
done
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  d="$(dirname "$f")"
  if [ ! -w "$d" ]; then
    echo "FATAL: artifact directory is not writable: $d" >&2
    exit 1
  fi
  rm -f "$f"
  if ! : > "$f" 2>/dev/null; then
    echo "FATAL: cannot write artifact: $f" >&2
    exit 1
  fi
  rm -f "$f"
done

# The per-sample scrape directory is an ARTIFACT and joins the refusal above.
# The loop enumerates NAMED FILES, so a directory beside them would sit outside
# it, and a second run would then MIX its scrapes with the surviving ones --
# corrupting the decision-scrape selection silently rather than failing loudly.
if [ -e "$SCRAPES_DIR" ] && [ "${SPEC365_FORCE:-0}" != "1" ]; then
  if [ ! -d "$SCRAPES_DIR" ] || [ -n "$(ls -A "$SCRAPES_DIR" 2>/dev/null || true)" ]; then
    echo "FATAL: artifact already exists: $SCRAPES_DIR" >&2
    echo "       Move it aside, or re-run with SPEC365_FORCE=1 to overwrite." >&2
    exit 1
  fi
fi
rm -rf "$SCRAPES_DIR"
if ! mkdir -p "$SCRAPES_DIR"; then
  echo "FATAL: cannot create artifact directory: $SCRAPES_DIR" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 5. PID resolution. The thing being sampled is the CHILD topgun-server the
#    harness spawns, NOT the harness itself.
# ---------------------------------------------------------------------------
resolve_server_pid() {   # prints the pid, or nothing; never fails the shell
  local pids
  pids="$(ss -Hltnp "sport = :${SERVER_PORT}" 2>/dev/null | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u || true)"
  if [ -z "$pids" ]; then
    pids="$(pgrep -f "topgun-server --port ${SERVER_PORT}" 2>/dev/null | sort -u || true)"
  fi
  local count
  count="$(printf '%s\n' "$pids" | grep -c '[0-9]' || true)"
  if [ "$count" = "1" ]; then
    printf '%s' "$(printf '%s\n' "$pids" | grep '[0-9]' | head -1)"
  fi
}

if [ -n "$(resolve_server_pid)" ] || [ -n "$(ss -Hltn "sport = :${SERVER_PORT}" 2>/dev/null || true)" ]; then
  echo "FATAL: something is already listening on port ${SERVER_PORT}" >&2
  echo "       The sampler resolves the server by that port; a stranger there" >&2
  echo "       would be sampled instead of this run's server." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 6. Report the effective matrix before starting. `tee`d into a COMMITTED
#    artifact, not merely printed.
# ---------------------------------------------------------------------------
{
  echo
  echo "=== spec376 Linux run: cell ${CELL} (flavour ${FLAVOUR}, phase ${CELL_PHASE}) ==="
  echo "  repo HEAD:      $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo '<not a git checkout>')"
  echo "  dirty tree:     $(test -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" && echo yes || echo no)"
  echo "  host/OS:        $(uname -a)"
  echo "  lineage:        chain-built harness + chain-built ${FLAVOUR} server"
  echo "  chain start:    ${SPEC376_CHAIN_START_EPOCH}"
  echo "  data dir:       $DATA_DIR"
  echo "  csv:            $CSV_OUT"
  echo "  soak.json:      $JSON_OUT"
  echo "  mechanism.json: $MECH_OUT (harness writes $(basename "$MECH_RAW"); renamed after the run)"
  echo "  durable.json:   $DURABLE_OUT (raw basename: NOT renamed, by design)"
  echo "  progress.jsonl: $PROGRESS_OUT"
  echo "  console log:    $CONSOLE_OUT"
  echo "  matrix:         $MATRIX_OUT"
  echo "  soak binary:    $SOAK_BIN"
  echo "  soak binary commit: ${SOAK_BIN_COMMIT} (HEAD; the binary itself is identified by its sha256)"
  echo "  .rs working tree:   ${RS_TREE_STATE}"
  echo "  code freeze:            ${CAL_PIN}"
  echo "  code freeze diff (.rs): ${FREEZE_DIFF_STATE}"
  echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  server binary:  $SERVER_BIN"
  echo "    built:        $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "    sha256:       ${SERVER_BIN_SHA256:-<unavailable>}"
  echo "  --- varied knobs; everything else is the spec362b-durable.sh long4h"
  echo "      literal ---"
  echo "  TOPGUN_EPOCH_WIDTH:  ${TOPGUN_EPOCH_WIDTH:-<unset: production default 1000>}"
  echo "  duration:            ${DURATION}s$( [ "$SMOKE" = "1" ] && echo '  <-- SMOKE OVERRIDE')"
  echo "  csv cadence:         ${SAMPLE_INTERVAL}s"
  echo "  extra harness flags: ${EXTRA_FLAGS:-<none>}"
  echo "  --durable-reading:   ARMED"
  echo "  --live-census-interval: ${CELL_LIVE_CENSUS}$( [ "$CELL_LIVE_CENSUS" = "0" ] && echo ' (DISARMED)' || echo ' (ARMED: the live-copy census)' )"
  echo "  --sampler-jitter-seed:  ${JITTER_SEED}"
  echo "  SOAK_SERVER_LOG:     ${SOAK_SERVER_LOG:-<unset>}"
  echo "  SOAK_SERVER_LOG_PASSTHROUGH: ${SOAK_SERVER_LOG_PASSTHROUGH:-<unset>}"
  echo "  TOPGUN_PRUNE_RECORD: ${TOPGUN_PRUNE_RECORD:-<unset>}"
  echo "  TOPGUN_LOG_FORMAT:   ${TOPGUN_LOG_FORMAT:-<unset: human fmt, which is what the origin parser reads>}"
  echo "  server port:         ${SERVER_PORT}"
  echo "  SOAK_SERVER_BINARY:  ${SOAK_SERVER_BINARY:-<unset: HEAD binary, fail-closed guard not on this path>}"
  echo "  --- pinned matrix (identical to spec362b-durable.sh's long4h cell;"
  echo "      crash-interval is per-cell and is 0 on the one cell this runner"
  echo "      ships) ---"
  echo "  churn-clients ${CHURN_CLIENTS}; keyspace ${KEYSPACE}; or-churn ${OR_CHURN};"
  echo "  or-keyspace ${OR_KEYSPACE}; or-every ${OR_EVERY}; write-interval-ms ${WRITE_INTERVAL_MS};"
  echo "  writes-per-life ${WRITES_PER_LIFE}; offline-keys ${OFFLINE_KEYS};"
  echo "  confirm-interval ${CONFIRM_INTERVAL}; crash-interval ${CRASH_INTERVAL};"
  echo "  steady-interval ${STEADY_INTERVAL}; quiesce ${QUIESCE};"
  echo "  mem-sample-interval ${MEM_SAMPLE_INTERVAL}; wal-fsync ${WAL_FSYNC};"
  echo "  memory gate NEUTRALIZED (${MEM_MIN_GROWTH_MB}/${MEM_THRESHOLD_MB_PER_HOUR}/${MEM_CEILING_MB})"
  echo "  tombstone gate: level ceiling K=2+ceil(S_A/W) epochs × W × b_max (S_A, b_max measured by the harness; A = TOPGUN_WAL_WATERMARK_STALL_BOUND_MS, not set by this runner); level stability 10 %; slope report-only"
  echo "  harness sha256: ${SOAK_BIN_SHA256}"
  echo "  server flavour: ${FLAVOUR}"
  echo "  server code:    ${CELL_SERVER} = ${EXPECT_SERVER_COMMIT} (SPEC376_SERVER_COMMIT=${SPEC376_SERVER_COMMIT}, resolves ${GOT_SERVER_SHA})"
  echo "  TOPGUN_JOURNAL_ENABLED:        ${TOPGUN_JOURNAL_ENABLED:-<unset: harness passes true>}"
  echo "  TOPGUN_SOAK_GRACEFUL_SHUTDOWN: ${TOPGUN_SOAK_GRACEFUL_SHUTDOWN:-<unset: SIGKILL teardown>}"
  echo "  DHAT_OUT:                      ${DHAT_OUT:-<unset>}"
  echo "  post-mortem rows:              counted at the end of the run (item 17)"
  echo "  --- host (Linux) ---"
  echo "  os-release:          $(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-<unset>}" || echo '<unavailable>')"
  echo "  kernel:              $(uname -r)"
  echo "  cpu model:           $(lscpu 2>/dev/null | awk -F: '/^Model name/ { sub(/^[ \t]+/, "", $2); print $2; exit }')"
  echo "  nproc:               $(nproc 2>/dev/null || echo '<unavailable>')"
  echo "  glibc:               $(ldd --version 2>&1 | head -1)"
  echo "  rustc host:          $(rustc -vV 2>/dev/null | awk '/^host:/ { print $2 }' || true)"
  echo "  THP enabled:         $(cat /sys/kernel/mm/transparent_hugepage/enabled 2>/dev/null || echo '<unavailable>')"
  echo "  THP defrag:          $(cat /sys/kernel/mm/transparent_hugepage/defrag 2>/dev/null || echo '<unavailable>')"
  echo "  swapon --show:       $(test -n "$(swapon --show 2>/dev/null || true)" && echo 'NON-EMPTY (swap active)' || echo 'empty (no swap)')"
  echo "  vm.overcommit_memory: $(cat /proc/sys/vm/overcommit_memory 2>/dev/null || echo '<unavailable>')"
  echo "  vm.max_map_count:    $(cat /proc/sys/vm/max_map_count 2>/dev/null || echo '<unavailable>')"
  echo "  awk:                 $(command -v awk || echo '<none>')"
  echo "  awk -version:        $(awk -version 2>&1 | head -1)"
  echo "  memory sampler:      $(basename "$PROCMEM_LIB") sha256=$(shasum -a 256 "$PROCMEM_LIB" 2>/dev/null | awk '{print $1}')"
  echo "  memory proc root:    /proc (fixed; procmem_row is called without a root, fixture roots belong to the synthetic suite only)"
  echo "  /proc/loadavg:       $(cat /proc/loadavg 2>/dev/null || echo '<unavailable>')"
  echo "  /proc/meminfo (first 8 lines):"
  head -8 /proc/meminfo 2>/dev/null | sed 's/^/    /' || true
  echo
} | tee "$MATRIX_OUT"

# ---------------------------------------------------------------------------
# 7. Launch.
# ---------------------------------------------------------------------------
HARNESS_PID=""
SAMPLER_PID=""
cleanup() {
  if [ -n "$SAMPLER_PID" ] && kill -0 "$SAMPLER_PID" 2>/dev/null; then
    kill "$SAMPLER_PID" 2>/dev/null || true
  fi
  if [ -n "$HARNESS_PID" ] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# EXTRA_FLAGS is deliberately unquoted: it is a fixed literal from this file's
# own case table (always empty here), never operator input.
# shellcheck disable=SC2086
"$SOAK_BIN" \
  --duration "$DURATION" \
  --churn-clients "$CHURN_CLIENTS" \
  --keyspace "$KEYSPACE" \
  --or-churn "$OR_CHURN" \
  --or-keyspace "$OR_KEYSPACE" \
  --or-every "$OR_EVERY" \
  --write-interval-ms "$WRITE_INTERVAL_MS" \
  --writes-per-life "$WRITES_PER_LIFE" \
  --offline-keys "$OFFLINE_KEYS" \
  --confirm-interval "$CONFIRM_INTERVAL" \
  --crash-interval "$CRASH_INTERVAL" \
  --steady-interval "$STEADY_INTERVAL" \
  --quiesce "$QUIESCE" \
  --mem-sample-interval "$MEM_SAMPLE_INTERVAL" \
  --mem-min-growth-mb "$MEM_MIN_GROWTH_MB" \
  --mem-threshold-mb-per-hour "$MEM_THRESHOLD_MB_PER_HOUR" \
  --mem-ceiling-mb "$MEM_CEILING_MB" \
  --wal-fsync "$WAL_FSYNC" \
  --server-port "$SERVER_PORT" \
  --data-dir "$DATA_DIR" \
  --json-output "$JSON_OUT" \
  --progress-output "$PROGRESS_OUT" \
  --mechanism-report \
  --durable-reading \
  --live-census-interval "$CELL_LIVE_CENSUS" \
  --sampler-jitter-seed "$JITTER_SEED" \
  $EXTRA_FLAGS \
  > "$CONSOLE_LOG" 2>&1 &
HARNESS_PID=$!
echo "harness pid $HARNESS_PID; follow with: tail -f $CONSOLE_LOG"

# ---------------------------------------------------------------------------
# 8. Wait for server-ready. This is the CSV's time origin.
# ---------------------------------------------------------------------------
READY_TIMEOUT=180
SERVER_PID=""
waited=0
while [ "$waited" -lt "$READY_TIMEOUT" ]; do
  SERVER_PID="$(resolve_server_pid)"
  [ -n "$SERVER_PID" ] && break
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    echo "FATAL: harness exited before the server became ready" >&2
    tail -40 "$CONSOLE_LOG" >&2 || true
    write_console_out
    exit 1
  fi
  sleep 1
  waited=$((waited + 1))
done
if [ -z "$SERVER_PID" ]; then
  echo "FATAL: no single listener on port ${SERVER_PORT} after ${READY_TIMEOUT}s" >&2
  tail -40 "$CONSOLE_LOG" >&2 || true
  write_console_out
  exit 1
fi
T0="$(date +%s)"
echo "server ready: pid $SERVER_PID (t0 = server-ready)"

# Item 6: steal ticks and total ticks of the aggregate cpu line, "steal total"
# or nothing. A dedicated-vCPU host should read ~0; a noisy neighbour shows up
# here and nowhere else.
proc_stat_steal() {
  awk '$1 == "cpu" && NF >= 9 {
         t = 0; for (i = 2; i <= 9; i++) { if ($i !~ /^[0-9]+$/) exit; t += $i }
         printf "%s %s\n", $9, t; exit
       }' /proc/stat 2>/dev/null || true
}
STEAL_T0="$(proc_stat_steal)"

# ---------------------------------------------------------------------------
# 9. The per-minute sampler.
# ---------------------------------------------------------------------------
kib_to_mb() { awk -v k="${1:-0}" 'BEGIN { printf "%.3f", k / 1024 }'; }

# The CSV header literal: the parent's 55 columns with the five macOS
# footprint columns renamed to the Linux quantities that actually fill them
# (7 fp_equiv_mb = Anonymous - LazyFree + Swap, 8 hwm_rss_mb = VmHWM, a peak
# RSS and not a peak footprint, 9 lazyfree_mb, 10 swap_mb, 41 file_mb =
# Private_Clean + Shared_Clean), and six smaps_rollup columns appended as
# 56-61 so 1-55 keep the positions every readout rule cites. The inherited six
# stay byte-identical in content and position; columns 11-31 are the conjunct
# snapshot's counter and 20 gauges, in the order the record's fields are
# defined; columns 32-40 are the pre-existing prune _total/indexed_refs series.
# No column carries a footprint name: a frozen macOS predicate pointed at this
# CSV must fail closed on a missing column, never read a Linux number as an
# Apple one.
CSV_HEADER='elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,fp_equiv_mb,hwm_rss_mb,lazyfree_mb,swap_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,file_mb,alloc_live_bytes,alloc_live_mb,alloc_probe_elapsed_s,alloc_probe_seq,je_allocated,je_active,je_resident,je_retained,je_mapped,je_metadata,je_probe_elapsed_s,je_probe_seq,bytes_alloc,bytes_dealloc,anon_mb,private_dirty_mb,pss_mb,anon_huge_mb,smaps_rss_mb,smaps_read_ms'
# The eleven memory columns, by header name; a written row must fill all of
# them (the sampler is fatal otherwise), and the post-run check asserts it.
MEM_COLUMNS="rss_mb fp_equiv_mb hwm_rss_mb lazyfree_mb swap_mb file_mb anon_mb private_dirty_mb pss_mb anon_huge_mb smaps_rss_mb"

# The ordered metric-name list behind the one /metrics scrape below. Position
# 1 is the inherited tombstone-bytes gauge (CSV column 6); positions 2-31 are
# CSV columns 11-40 in that order. A name this list carries but the scrape
# body does not renders as an EMPTY field, never a zero.
PRUNE_METRIC_NAMES="topgun_ormap_tombstone_bytes topgun_or_prune_conjunct_snapshots_total topgun_or_prune_conjunct_current_epoch topgun_or_prune_conjunct_ceiling topgun_or_prune_conjunct_durable_watermark topgun_or_prune_conjunct_durable_watermark_lag topgun_or_prune_conjunct_claims topgun_or_prune_conjunct_claim_lag_p50 topgun_or_prune_conjunct_claim_lag_p99 topgun_or_prune_conjunct_claim_lag_max topgun_or_prune_conjunct_retained_epochs_claim_only topgun_or_prune_conjunct_retained_epochs_durability_only topgun_or_prune_conjunct_retained_epochs_both topgun_or_prune_conjunct_retained_epochs_neither topgun_or_prune_conjunct_retained_refs_claim_only topgun_or_prune_conjunct_retained_refs_durability_only topgun_or_prune_conjunct_retained_refs_both topgun_or_prune_conjunct_retained_refs_neither topgun_or_prune_conjunct_retained_stamped_bytes topgun_or_prune_conjunct_retained_epochs_unslotted topgun_or_prune_conjunct_retained_refs_open_epoch topgun_or_prune_conjunct_retained_stamped_bytes_open_epoch topgun_or_prune_indexed_refs topgun_or_prune_considered_total topgun_or_prune_dropped_total topgun_or_prune_matched_nothing_total topgun_or_prune_absent_total topgun_or_prune_bytes_freed_total topgun_or_prune_removed_refs_observed_total topgun_or_prune_removed_bytes_observed_total topgun_or_prune_stamped_bytes_total"

# One curl /metrics per row. Prints the 31 values above as one comma-joined
# string, in list order. A metric absent from the body -- including every
# name in this list, if the scrape itself fails -- becomes an empty field.
scrape_prune_metrics() {   # $1 = the server pid this row sampled
  local body stamp
  body="$(curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null)" || body=""
  # POST-MORTEM ROW (item 17). T0 is server-ready, which is later than the
  # harness's own start, so the row due at T0+D always lands after the harness
  # has finished and killed the server. The parent persists an empty file on a
  # failed curl on purpose -- a live server that cannot be read is a named
  # fail-closed reason for P6/P7 -- but a scrape taken after the harness
  # deliberately killed the server observed nothing, and letting it decide
  # P6/P7 would stop a whole chain on a race. So: if the pid is GONE, no scrape
  # file is written for this row and the row is counted; if it is ALIVE, the
  # empty file is written exactly as the parent does.
  # The counter lives in a FILE: this function runs inside a command
  # substitution, so a shell variable incremented here would die with the
  # subshell and the count would always read 0.
  if [ -z "$body" ] && ! kill -0 "$1" 2>/dev/null; then
    local pm
    pm="$(cat "$PM_FILE" 2>/dev/null || echo 0)"
    case "$pm" in ''|*[!0-9]*) pm=0 ;; esac
    echo $((pm + 1)) > "$PM_FILE"
    echo "post-mortem row: the /metrics scrape failed and pid $1 is gone; no scrape file written" >&2
    printf '%s' "" | awk -v names="$PRUNE_METRIC_NAMES" 'BEGIN { n = split(names, want, " "); out = ""; for (i = 1; i <= n; i++) out = (i == 1) ? "" : out ","; print out }'
    return 0
  fi
  # The body is kept WHOLE before it is reduced to 31 numbers, under the same
  # fixed-width UTC RFC 3339 stamp the console prefix uses -- the one time
  # domain the manifest's window rules compare in. A failed curl writes an
  # EMPTY file on purpose: an unreadable scrape is a named fail-closed reason
  # for P6/P7, whereas a missing file would be indistinguishable from a sample
  # that was never due.
  stamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf '%s' "$body" > "${SCRAPES_DIR}/${stamp}.txt" || true
  printf '%s' "$body" | awk -v names="$PRUNE_METRIC_NAMES" '
    BEGIN { n = split(names, want, " ") }
    /^[[:space:]]*#/ { next }
    {
      name = $1
      b = index(name, "{")
      if (b > 0) name = substr(name, 1, b - 1)
      if (!(name in seen)) { val[name] = $2 + 0; seen[name] = 1 }
    }
    END {
      out = ""
      for (i = 1; i <= n; i++) {
        if (want[i] in seen) v = sprintf("%.0f", val[want[i]]); else v = ""
        out = (i == 1) ? v : out "," v
      }
      print out
    }
  '
}

du_kib() {  # $1 = path
  if [ ! -e "$1" ]; then
    printf 'ABSENT'
    return 0
  fi
  local v
  v="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  if [ -z "$v" ]; then
    sleep 1
    v="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  fi
  printf '%s' "$v"
}

sampler_fatal() {   # $1 = reason
  echo "$1" > "$FAIL_FILE"
  echo "SAMPLER FATAL: $1" >&2
  kill "$HARNESS_PID" 2>/dev/null || true
}

run_is_over() {   # $1 = elapsed seconds since server-ready
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    return 0
  fi
  if [ "$1" -ge $((DURATION - 10)) ]; then
    return 0
  fi
  return 1
}

emit_row() {
  local now elapsed pid tries
  now="$(date +%s)"
  elapsed=$((now - T0))

  pid=""
  tries=0
  while [ "$tries" -lt 3 ]; do
    pid="$(resolve_server_pid)"
    [ -n "$pid" ] && break
    if run_is_over "$elapsed"; then
      touch "$STOP_FILE"
      return 0
    fi
    tries=$((tries + 1))
    sleep 2
  done
  if [ -z "$pid" ]; then
    sampler_fatal "server PID on port ${SERVER_PORT} did not resolve to exactly one process at elapsed=${elapsed}s"
    return 1
  fi
  SERVER_PID="$pid"

  # The memory row is read where the parent read `ps -o rss=`: a pid gone
  # here ends sampling exactly as the parent's empty-RSS path did, so no
  # written row carries an empty memory cell. A miss on a LIVE pid is fatal.
  local mem_row mem_rc
  mem_row="$(procmem_row "$pid")" && mem_rc=0 || mem_rc=$?
  case "$mem_rc" in
    0) ;;
    1)
      if run_is_over "$elapsed"; then
        touch "$STOP_FILE"
        return 0
      fi
      sampler_fatal "memory sources unreadable and pid ${pid} gone before the run was over at elapsed=${elapsed}s (blind column)"
      return 1
      ;;
    *)
      sampler_fatal "memory sampler blind on live pid ${pid}: ${mem_row} at elapsed=${elapsed}s"
      return 1
      ;;
  esac
  local m_rss m_fp m_hwm m_lazy m_swap m_file m_tail
  IFS=, read -r m_rss m_fp m_hwm m_lazy m_swap m_file m_tail <<EOF_MEM_ROW
$mem_row
EOF_MEM_ROW

  local total_kib wal_kib redb_kib
  total_kib="$(du_kib "$DATA_DIR")"
  case "$total_kib" in
    ''|ABSENT)
      sampler_fatal "du gave no size for data dir ${DATA_DIR} at elapsed=${elapsed}s"
      return 1
      ;;
  esac
  wal_kib="$(du_kib "${DATA_DIR}/wal")"
  if [ "$wal_kib" = "ABSENT" ]; then
    wal_kib=0
  elif [ -z "$wal_kib" ]; then
    sampler_fatal "du failed on ${DATA_DIR}/wal at elapsed=${elapsed}s"
    return 1
  fi
  redb_kib="$(du_kib "${DATA_DIR}/topgun.redb")"
  if [ "$redb_kib" = "ABSENT" ]; then
    redb_kib=0
  elif [ -z "$redb_kib" ]; then
    sampler_fatal "du failed on ${DATA_DIR}/topgun.redb at elapsed=${elapsed}s"
    return 1
  fi

  local v
  for v in "$wal_kib" "$redb_kib" "$total_kib"; do
    case "$v" in
      ''|*[!0-9]*)
        sampler_fatal "non-integer sample '${v}' at elapsed=${elapsed}s"
        return 1
        ;;
    esac
  done

  # One /metrics scrape per row supplies the inherited tombstone-bytes gauge
  # (first field) and the 30 new metric columns (the remainder), from the
  # same response body.
  local prune_row tomb prune_rest
  prune_row="$(scrape_prune_metrics "$pid")"
  tomb="${prune_row%%,*}"
  prune_rest="${prune_row#*,}"
  case "$tomb" in
    ''|*[!0-9]*) tomb="" ;;
  esac

  # The count-alloc probe, read LAST so the new work cannot eat the margin the
  # lineage's sampler timing relies on: the server prints it on stderr every 30 s and the
  # harness mirrors it as "[server] alloc_probe ...". The last line seen so far
  # fills four columns; none seen (always so on the R and DH builds) leaves
  # them empty. `|| true` because a no-match must not kill a pipefail sampler.
  local probe_line probe_seq probe_cols
  probe_line="$( { grep -a '^\[server\] alloc_probe ' "$CONSOLE_LOG" 2>/dev/null || true; } | tail -1)"
  probe_seq="$(grep -ac '^\[server\] alloc_probe ' "$CONSOLE_LOG" 2>/dev/null || true)"
  local byte_cols
  if [ -n "$probe_line" ]; then
    probe_cols="$(printf '%s\n' "$probe_line" | awk -v seq="${probe_seq:-0}" '
      { for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } }
      END { printf "%s,%.3f,%s,%s", f["live_bytes"], f["live_bytes"] / 1048576, f["elapsed_s"], seq }')"
    byte_cols="$(printf '%s\n' "$probe_line" | awk '
      { for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } }
      END { printf "%s,%s", f["bytes_alloc"], f["bytes_dealloc"] }')"
  else
    probe_cols=",,,"
    byte_cols=","
  fi
  # The jemalloc probe, same transport: "[server] je_probe elapsed_s=... seq=N".
  # A field the server printed as n/a (a failed read) becomes an empty cell, so
  # a gap stays a gap instead of turning into a number.
  local je_line je_cols
  je_line="$( { grep -a '^\[server\] je_probe ' "$CONSOLE_LOG" 2>/dev/null || true; } | tail -1)"
  if [ -n "$je_line" ]; then
    je_cols="$(printf '%s\n' "$je_line" | awk '
      function v(k) { return (f[k] ~ /^[0-9]+$/) ? f[k] : "" }
      { for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } }
      END { printf "%s,%s,%s,%s,%s,%s,%s,%s", v("allocated"), v("active"), v("resident"), v("retained"), v("mapped"), v("metadata"), v("elapsed_s"), v("seq") }')"
  else
    je_cols=",,,,,,,"
  fi


  printf '%d,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$elapsed" \
    "$m_rss" \
    "$(kib_to_mb "$wal_kib")" \
    "$(kib_to_mb "$redb_kib")" \
    "$(kib_to_mb "$total_kib")" \
    "$tomb" \
    "$m_fp" "$m_hwm" "$m_lazy" "$m_swap" \
    "$prune_rest" \
    "$m_file" \
    "$probe_cols" \
    "$je_cols" \
    "$byte_cols" \
    "$m_tail" \
    >> "$CSV_OUT"
}

printf '%s\n' "$CSV_HEADER" > "$CSV_OUT"

sampler_loop() {
  local next="$T0"
  while [ ! -f "$STOP_FILE" ]; do
    local now
    now="$(date +%s)"
    if [ "$now" -lt "$next" ]; then
      sleep 1
      continue
    fi
    emit_row || return 1
    next=$((next + SAMPLE_INTERVAL))
    now="$(date +%s)"
    if [ "$next" -le "$now" ]; then
      next=$((now + SAMPLE_INTERVAL))
    fi
  done
}

sampler_loop &
SAMPLER_PID=$!

# ---------------------------------------------------------------------------
# 10. Wait out the run.
# ---------------------------------------------------------------------------
set +e
wait "$HARNESS_PID"
HARNESS_RC=$?
set -e
HARNESS_PID=""
touch "$STOP_FILE"
set +e
wait "$SAMPLER_PID" 2>/dev/null
set -e
SAMPLER_PID=""

STEAL_END="$(proc_stat_steal)"

echo
echo "harness exited with code ${HARNESS_RC}"
tail -25 "$CONSOLE_LOG" || true

# ---------------------------------------------------------------------------
# 11. Post-run: land the console log in its COMMITTED home, normalize the
#     mechanism report's name, then validate the series.
# ---------------------------------------------------------------------------
write_console_out
echo "console log: $CONSOLE_OUT"

if [ -f "$MECH_RAW" ]; then
  mv -f "$MECH_RAW" "$MECH_OUT"
  echo "mechanism report: $MECH_OUT"
fi

INSTRUMENT_OK=1
fail_instrument() { echo "INSTRUMENT DEFECT: $1" >&2; INSTRUMENT_OK=0; }

if [ -s "$FAIL_FILE" ]; then
  fail_instrument "sampler aborted: $(cat "$FAIL_FILE")"
fi

HEADER="$(head -1 "$CSV_OUT" 2>/dev/null || true)"
if [ "$HEADER" != "$CSV_HEADER" ]; then
  fail_instrument "CSV header is '$HEADER'"
fi
HEADER_PREFIX="$(printf '%s' "$HEADER" | cut -d, -f1-6)"
if [ "$HEADER_PREFIX" != 'elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes' ]; then
  fail_instrument "CSV header's first six columns are not byte-identical to the parent's: '$HEADER_PREFIX'"
fi
ROWS="$(( $(wc -l < "$CSV_OUT") - 1 ))"
echo "csv rows: $ROWS"
PM_ROWS="$(cat "$PM_FILE" 2>/dev/null || echo 0)"
case "$PM_ROWS" in ''|*[!0-9]*) PM_ROWS=0 ;; esac
echo "post_mortem_rows=${PM_ROWS}"
printf '  post_mortem_rows: %s\n' "$PM_ROWS" >> "$MATRIX_OUT"
MEM_INV="$(if [ -e "$PROCMEM_INV_FILE" ]; then cat "$PROCMEM_INV_FILE" 2>/dev/null; else echo 0; fi)"
case "$MEM_INV" in ''|*[!0-9]*) MEM_INV=non_numeric ;; esac
MEM_PM="$(if [ -e "$PROCMEM_PM_FILE" ]; then cat "$PROCMEM_PM_FILE" 2>/dev/null; else echo 0; fi)"
case "$MEM_PM" in ''|*[!0-9]*) MEM_PM=non_numeric ;; esac
STEAL_PCT="$(printf '%s %s\n' "${STEAL_T0:-}" "${STEAL_END:-}" | awk '
  NF == 4 && $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ && $4 ~ /^[0-9]+$/ && $4 - $2 > 0 {
    printf "%.4f", ($3 - $1) / ($4 - $2) * 100; ok = 1
  }
  END { if (!ok) printf "n/a" }')"
printf '  mem_invariant_violations: %s\n  post_mortem_mem_reads: %s\n  steal_pct: %s\n' "$MEM_INV" "$MEM_PM" "$STEAL_PCT" >> "$MATRIX_OUT"
# Item 2/6: the console ENDS with these three lines, exactly once each, on
# every path that got past launch; the predicates key on them.
emit_tail() {
  echo "steal_pct=${STEAL_PCT}"
  echo "post_mortem_mem_reads=${MEM_PM}"
  echo "mem_invariant_violations=${MEM_INV}"
}
if [ "$ROWS" -lt 2 ]; then
  fail_instrument "CSV has $ROWS data rows"
fi

col_report() {   # $1 = 1-based column index, $2 = name
  awk -F, -v idx="$1" -v name="$2" '
    NR == 1 { next }
    {
      v = $idx
      gsub(/[ \t\r]/, "", v)
      if (v == "") empty++
      else {
        n++
        if (v + 0 != 0) nonzero++
        if (n == 1 || v + 0 < min) min = v + 0
        if (n == 1 || v + 0 > max) max = v + 0
      }
    }
    END {
      printf "  %-14s n=%d empty=%d nonzero=%d min=%.3f max=%.3f\n",
             name, n, empty + 0, nonzero + 0, min + 0, max + 0
      if (empty > 0 || n == 0 || nonzero == 0) exit 1
    }
  ' "$CSV_OUT"
}
echo "csv columns:"
col_report 2 rss_mb        || fail_instrument "rss_mb column is empty or all-zero (blind sampler)"
col_report 3 wal_mb        || fail_instrument "wal_mb column is empty or all-zero (wrong WAL path?)"
col_report 4 redb_mb       || fail_instrument "redb_mb column is empty or all-zero (wrong redb path?)"
col_report 5 disk_total_mb || fail_instrument "disk_total_mb column is empty or all-zero"

# POPULATION check, not non-zero: a genuinely bounded tombstone corpus may
# legitimately read 0 for a whole run.
tombstone_col_report() {
  awk -F, '
    NR == 1 { next }
    {
      v = $6
      gsub(/[ \t\r]/, "", v)
      if (v == "") empty++
      else {
        n++
        if (n == 1 || v + 0 < min) min = v + 0
        if (n == 1 || v + 0 > max) max = v + 0
      }
    }
    END {
      printf "  %-14s n=%d empty=%d min=%.0f max=%.0f\n",
             "tombstone_bytes", n + 0, empty + 0, min + 0, max + 0
      if (n == 0) exit 1
    }
  ' "$CSV_OUT"
}
tombstone_col_report || fail_instrument "tombstone_bytes column has no readings (blind gauge scrape)"

# Item 2: the eleven memory columns, located by header name. POPULATION, not
# non-zero: swap_mb and lazyfree_mb legitimately read 0 for a whole run on a
# host without swap or before any MADV_FREE. An absent column or an empty cell
# is a blind sampler.
mem_col_report() {   # $1 = column name
  awk -F, -v name="$1" '
    NR == 1 { for (i = 1; i <= NF; i++) if ($i == name) idx = i; if (!idx) { printf "  %-16s MISSING_COLUMN\n", name; exit 1 }; next }
    { v = $idx; gsub(/[ \t\r]/, "", v); if (v == "") empty++; else n++ }
    END {
      if (!idx) exit 1
      printf "  %-16s n=%d empty=%d\n", name, n + 0, empty + 0
      if (empty > 0 || n == 0) exit 1
    }
  ' "$CSV_OUT"
}
for c in $MEM_COLUMNS; do
  mem_col_report "$c" || fail_instrument "memory column ${c} is missing, empty or unfilled (blind sampler)"
done

for f in "$JSON_OUT" "$MECH_OUT" "$DURABLE_OUT"; do
  if [ ! -s "$f" ]; then
    fail_instrument "missing or empty artifact: $f"
  fi
done

if [ "$DURATION" -gt "$STEADY_INTERVAL" ]; then
  if [ ! -s "$PROGRESS_OUT" ]; then
    fail_instrument "missing or empty artifact: $PROGRESS_OUT"
  else
    echo "  progress.jsonl checkpoints: $(wc -l < "$PROGRESS_OUT" | tr -d ' ')"
  fi
else
  echo "  progress.jsonl: not required -- duration ${DURATION}s <= steady interval ${STEADY_INTERVAL}s,"
  echo "                  so the harness reached no checkpoint to snapshot."
fi

if [ -s "$JSON_OUT" ]; then
  for key in walFsync epochWidth crashes churnClients keyspace durationSecsActual; do
    line="$(grep -m1 "\"${key}\"" "$JSON_OUT" || true)"
    if [ -z "$line" ]; then
      fail_instrument "soak.json has no '${key}' key"
    else
      echo "  soak.json $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
    fi
  done
fi
if [ -s "$MECH_OUT" ]; then
  for key in orDeltaFrames orSnapshotFrames; do
    line="$(grep -m1 "\"${key}\"" "$MECH_OUT" || true)"
    if [ -z "$line" ]; then
      fail_instrument "mechanism.json has no '${key}' key"
    else
      echo "  mechanism.json $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
    fi
  done
fi

# ---------------------------------------------------------------------------
# 12. Convenience: the post-hoc fits. Unedited from the parent; it resolves
#     columns by header name, so the widened header does not affect it.
# ---------------------------------------------------------------------------
FIT="${SCRIPT_DIR}/spec349c2-fit.awk"
if [ -f "$FIT" ] && [ "$ROWS" -ge 4 ]; then
  echo
  echo "post-hoc OLS fits (see the SE caveat in $(basename "$FIT")):"
  echo "  units: the field is named slope_mb_per_hour for every column; on the"
  echo "  tombstone_bytes row the fit is identical but the unit is BYTES/hour."
  for w in full last_half; do
    for c in rss_mb wal_mb redb_mb disk_total_mb tombstone_bytes; do
      awk -v col="$c" -v window="$w" -f "$FIT" "$CSV_OUT" 2>&1 | sed 's/^/  /' || true
    done
  done
fi

if [ "$FLAVOUR" = "DH" ]; then
  if [ -s "$DHAT_OUT" ]; then
    gzip -9 -c "$DHAT_OUT" > "${OUT_DIR}/${BASE}.dhat.json.gz"
    echo "dhat profile: ${OUT_DIR}/${BASE}.dhat.json.gz ($(wc -c < "$DHAT_OUT" | tr -d ' ') bytes raw)"
  else
    fail_instrument "dhat profile missing at $DHAT_OUT"
  fi
fi

echo
if [ "$INSTRUMENT_OK" != "1" ]; then
  echo "RESULT: INSTRUMENT DEFECT -- this run's series must not be recorded as evidence." >&2
  emit_tail
  exit 9
fi
echo "RESULT: instrument sound; harness exit code ${HARNESS_RC}."
if [ "$HARNESS_RC" != "0" ]; then
  echo "NOTE: a non-zero harness exit is NOT automatically a failed characterization."
  echo "      The memory gate is neutralized by design for this run, while the"
  echo "      tombstone-byte gate, both blind-monitor clauses, convergence/recovery"
  echo "      and panic capture stay armed. Read 'finishedReason' in $JSON_OUT and"
  echo "      record the attribution, rather than reading the flag as a verdict."
fi

emit_tail
exit "$HARNESS_RC"
