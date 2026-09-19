# spec370-plateau4h — manifest (carve 8f: the tombstone level-ceiling cell)

This cell decides `TG-OR-005` on the re-derived gate: a level ceiling derived from the prune mechanism,
plus level stability, with the slope report-only. §1 and §2 below are **pre-registered**. They are
committed at the pre-registration commit M, before any `spec370-plateau4h` artifact exists, and are never
edited afterwards. §3 is appended after the data commit D, below the `## APPEND-ONLY BELOW` marker at the
end of §2.

## §1 — Freeze and the closed difference list

### 1.1 The freeze

| field | value |
|---|---|
| freeze F (the last `.rs`-touching commit before M) | `b13afaed` |
| full sha | `b13afaedade471c671298dc9f5a69a8f71dd1215` |
| subject | `feat(soak): gate resident tombstone bytes on a level ceiling and level stability` |
| committed | `2026-09-18T15:49:32+03:00` |
| pin / branch base | `86656caa2bc363cb67ca0f2a1c9fccb324d8f3ec` (merge of PR #166) |
| branch | `spec-8f-level-ceiling` (`main` never merged in) |

The `.rs` diff `86656caa..b13afaed` is exactly `benches/soak_harness/monitor.rs`, `main.rs` and
`report.rs`, plus `tests/soak_wal_census.rs`. Only the harness changed; the server source is identical to
the pin. Before the build and before any clock, the runner asserts the freeze with three guards: the
`SPEC370_CODE_FREEZE=b13afaed` literal must not be the placeholder, `git diff b13afaed..HEAD -- '*.rs'`
must be EMPTY, and the `.rs` working tree must be clean. Each guard has its own message, and none has an
override.

### 1.2 Digests

`shasum -a 256`, computed from the files as they stand at this pre-registration commit:

| file | sha256 | role |
|---|---|---|
| `spec370-plateau4h.sh` | `44070d1711f5af909752996a56359f502568f2b13915e27355b40249d1baddac` | the cell runner (§1.3–1.5 enumerate its whole departure) |
| `spec370-pk.awk` | `56b59ee21b65ed35d885735774550d8055a2df451d9f3f22e4713102ce31fe91` | P-K — ceiling epochs + recorded premise rows |
| `spec370-pc.awk` | `94a4c19e594ef051bb6a45e79cb54ed98cd41450e0529fa006ba8ceffa5af5db` | P-C / P-L |
| `spec370-ts.awk` | `c01b8cba561d415bea1934e390ec26de5e5c608fe98043f355018eecdd9ba7d9` | Theil–Sen (recorded) |
| `spec370-ph.awk` | `0c994e15072c2a9a96ef95d01a42d5bf086893ff679bf81f891afce7a040bcd6` | P-H — harness/cell agreement |
| `spec370-decide.awk` | `7c452399996d7e235bc6355a8c3dae14496dd14035398de391b479f257757087` | deciding flags (runs last) |
| `spec370-predicates.sh` | `baa58e8c0b3ddbc1544bc172c3e74859735521fa1b137e2040fcc6fa7512f2de` | orchestration: slicing → fits → STOP → deciding → recorded → decision |
| `spec370-calibrate.sh` | `c8f2b05aa9ac8d0d021eb3a49060d8d5dda1780ae72f4322d4859ce675a7fbc6` | calibration on committed cells |
| `spec370-calibration.txt` | `1b37f53e080a6590b859ad61bcc2a9b732f3ecc92846c4420adef1b4b4ee714a` | calibration output (`cmp`-identical to the spec block, §2.4) |
| `spec368-plateau4h.sh` | `a5c48031827fd38d73dd5f40b829913d95ae53c62cadbde2ad094c5b1ac85290` | FROZEN — the parent the runner is a copy of |
| `spec365-readout.sh` | `5fbfa3e9d9df2a74830edbf89cf4036ae3e376add9171676fe70800b335d7a11` | FROZEN — invoked byte-unchanged by the runner |
| `spec349c2-fit.awk` | `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` | FROZEN — the OLS fitter |
| `spec366-p5.awk` | `2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33` | FROZEN — P5 |
| `spec366-p67.awk` | `ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215` | FROZEN — P6 / P7 |
| `spec368-ps.awk` | `cb84ee55726bed5e7e120cec9eb6468b164df901130d0729de90e5200c3bf20a` | FROZEN — PS-rows |
| `spec368-pf.awk` | `4a1a79c4f8152b2c8edf61913be6b0e89aedc2c45f55b043aa31cff7c933dab3` | FROZEN — P-F (recorded) |
| `spec368-a7.awk` | `cde9e93f4179a3b741c6e787eaaf591ca1cf9ae133d8782e5dd1a69cb06d84bf` | FROZEN — A7 (recorded) |

The eight FROZEN digests equal those in `spec368-manifest.md`: the parent runner at §1.2, the others at
§1.2 and §2. The nine `spec370-*` digests are computed here from the files as written.

### 1.3 The embedded diff

The runner is a copy of `spec368-plateau4h.sh` whose difference list is CLOSED at seven items. Below is
the literal output of

```
cd packages/server-rust/benches/soak_harness/evidence && \
  diff -u spec368-plateau4h.sh spec370-plateau4h.sh | tail -n +3
```

`tail -n +3` drops the two `---`/`+++` header lines, which carry file mtimes.

```diff
@@ -1,58 +1,65 @@
 #!/usr/bin/env bash
 #
-# Plateau readout cell runner -- a copy of spec366-conjuncts.sh, which is NOT
+# Level-ceiling cell runner -- a copy of spec368-plateau4h.sh, which is NOT
 # edited.
 #
 # A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT BE RUN FOR THIS CELL. Its
-# freeze literal names another commit, its single cell is 900s long, and its
-# artifact basename names files that are committed evidence a re-run must not
-# overwrite. This file is therefore a COPY, and the difference list against
-# spec366-conjuncts.sh is CLOSED at exactly six items:
+# freeze literal names another commit, the harness it launches must now carry
+# the level-ceiling tombstone gate, and its artifact basename names files that
+# are committed evidence a re-run must not overwrite. This file is therefore a
+# COPY, and the difference list against spec368-plateau4h.sh is CLOSED at
+# exactly seven items:
 #
-#   1. THE DURATION IS 14400 SECONDS on the single cell line -- four hours,
-#      against the parent's 900s. Nothing else on that line changes apart
-#      from items 2 and 6.
-#   2. THE ARTIFACT BASENAME IS spec368-plateau4h, extended by the value of
-#      SPEC368_BASE_SUFFIX. With that variable unset -- the only supported
+#   1. THE ARTIFACT BASENAME IS spec370-plateau4h, extended by the value of
+#      SPEC370_BASE_SUFFIX. With that variable unset -- the only supported
 #      state for a first sample -- the basename resolves to
-#      spec368-plateau4h. A replicate exports the suffix as -r2, the basename
-#      resolves to spec368-plateau4h-r2, and a second sample therefore needs
-#      NO edit to this file after the first sample's artifacts are committed.
-#      This runner does not validate the suffix; the chain that launches it
-#      does.
-#   3. THE FREEZE VARIABLE IS RENAMED SPEC368_CODE_FREEZE at every site that
-#      reads or names it, and its placeholder string is renamed with it. THE
-#      SPEC365_* ENV OVERRIDE NAMES STAY VERBATIM (they are listed in the
-#      inherited header below): the readout below is the UNCHANGED
-#      spec365-readout.sh and resolves its own OUT_DIR from one of them, so
-#      renaming them would point the runner and the readout at DIFFERENT
-#      directories whenever a scratch out dir is used.
-#   4. THE FREEZE LITERAL IS THIS CELL'S OWN FREEZE COMMIT, d6a3d38f. The
-#      same three refusal guards -- the placeholder check, the .rs diff
-#      against the freeze commit, the dirty .rs working tree -- are otherwise
-#      unchanged, and none has an override. This cell measures a landed
-#      instrument and changes no .rs file, so the freeze commit IS its pin.
-#   5. this header and the usage text, naming this runner and its cell.
-#   6. THE CELL ID IS plateau4h, AND THE DEFAULT DATA DIR DERIVES FROM IT as
-#      target/spec368-<cell>-data with its sibling .meta dir, so a run of
-#      this file can neither land in nor collide with the parent cell's data
-#      dir. The matrix banner, the meta dir and the provenance-cell messages
-#      follow the cell id; the evidence artifact names follow the basename of
-#      item 2, not the cell id, and are unaffected by this item.
+#      spec370-plateau4h. A replicate exports the suffix as -r2, the basename
+#      resolves to spec370-plateau4h-r2, and a second sample therefore needs
+#      NO edit to this file. This runner does not validate the suffix; the
+#      chain that launches it does.
+#   2. THE ENV NAMES THIS LINEAGE OWNS ARE RENAMED: the freeze variable
+#      becomes SPEC370_CODE_FREEZE, its placeholder string is renamed with it,
+#      and the suffix variable becomes SPEC370_BASE_SUFFIX, at every site. THE
+#      SPEC365_* OVERRIDE NAMES AND SPEC362B_SMOKE_DURATION STAY VERBATIM: the
+#      readout below is the UNCHANGED spec365-readout.sh and resolves its own
+#      OUT_DIR from one of them, so renaming them would point the runner and
+#      the readout at DIFFERENT directories whenever a scratch out dir is used.
+#   3. THE FREEZE LITERAL IS THIS CELL'S OWN FREEZE COMMIT, b13afaed -- the
+#      last commit that touches a .rs file, where the level-ceiling gate is
+#      wired into the harness. The three refusal guards -- the placeholder
+#      check, the .rs diff against the freeze commit, the dirty .rs working
+#      tree -- are unchanged, and none has an override.
+#   4. THE DEFAULT DATA DIR is target/spec370-<cell>-data with its sibling
+#      .meta dir, so a run of this file can neither land in nor collide with
+#      the parent cell's data dir. The cell id stays plateau4h.
+#   5. this header and the usage text, naming this runner, its cell and this
+#      list.
+#   6. HARNESS PROVENANCE. One block, inserted immediately after the soak
+#      binary is resolved, hashes that binary and refuses to start unless (a)
+#      it carries the level-ceiling gate's own message literal, and (b) it was
+#      built by this invocation -- clause (b) is waived only under the
+#      SPEC365_SOAK_BIN override, which the matrix already marks UNPROVEN. The
+#      block then extends the console's provenance line with the harness
+#      sha256, its build time and the gate marker, so console line 1 carries
+#      BOTH launched binaries' identities.
+#   7. THE MATRIX ECHOES THE GATE: two added lines name the tombstone gate
+#      this harness applies and repeat the harness sha256, which the
+#      predicates compare with console line 1.
 #
-# EVERYTHING ELSE IS BYTE-IDENTICAL to spec366-conjuncts.sh, and that
-# includes the matrix block, the cell line's remaining literals, the log
-# directive, the environment-discipline block, the pre-clock provenance
-# assertion, the fresh-data-dir guard, the artifact-overwrite refusal and its
-# scrapes-directory clause, scrape persistence, the CSV header, the harness
-# invocation, the post-run checks and fits, and the readout invocation. The
-# readout is the UNCHANGED spec365-readout.sh, invoked with this cell's own
-# basename. The departure is enumerable with:
-#   diff spec366-conjuncts.sh spec368-plateau4h.sh
+# EVERYTHING ELSE IS BYTE-IDENTICAL to spec368-plateau4h.sh: the four-hour
+# duration, the 60s cadence, crash-interval 0, the width left unset, the log
+# directive, the armed prune record, the environment-discipline block, the
+# server provenance clauses, the fresh-data-dir guard, the artifact-overwrite
+# refusal, scrape persistence, the CSV header, the harness invocation (memory
+# gate still neutralized), the post-run checks and fits, and the readout
+# invocation. The departure is enumerable with:
+#   diff spec368-plateau4h.sh spec370-plateau4h.sh
 #
-# A DIFF HUNK THAT MAPS TO NONE OF THE SIX ITEMS IS A DEFECT, not a footnote:
-# the pre-registered manifest carries this diff and a hunk-to-item map, and a
-# runner whose departure cannot be enumerated cannot be filed under a freeze.
+# A DIFF HUNK THAT MAPS TO NONE OF THE SEVEN ITEMS IS A DEFECT, not a
+# footnote: the pre-registered manifest carries this diff and a hunk-to-item
+# map, and a runner whose departure cannot be enumerated cannot be filed under
+# a freeze. spec368-plateau4h.sh's own six-item list against its parent stays
+# readable in that file.
 #
 # Everything from here on is spec365-conjuncts.sh's own header, kept verbatim
 # so the lineage back to spec362b-durable.sh stays readable.
@@ -101,7 +108,7 @@
 #       inherited six; the inherited six stay byte-identical in content and
 #       position.
 #   (g) THE FREEZE GATE REPLACES THE PIN. The parent's commit-pin literal
-#       becomes SPEC368_CODE_FREEZE here, with one added behaviour the parent
+#       becomes SPEC370_CODE_FREEZE here, with one added behaviour the parent
 #       never needed: while this literal still reads its own placeholder
 #       value (because the code it would pin does not exist yet), the runner
 #       refuses immediately, before any git call that would otherwise need to
@@ -122,7 +129,7 @@
 # enumerable with:
 #   diff spec362b-durable.sh spec365-conjuncts.sh
 #
-# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC368_CODE_FREEZE names the commit
+# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC370_CODE_FREEZE names the commit
 # at which the instrument this file samples was complete and the full gate
 # matrix green. The runner refuses to start if the literal ever reads its
 # placeholder again, or if any .rs file at HEAD differs from that commit, so
@@ -158,10 +165,10 @@
 
 usage() {
   cat >&2 <<'EOF'
-usage: spec368-plateau4h.sh <cell>
+usage: spec370-plateau4h.sh <cell>
 
-  A COPY of spec366-conjuncts.sh, which is not edited. The difference list
-  against that file is CLOSED, has exactly six items, and is enumerated in
+  A COPY of spec368-plateau4h.sh, which is not edited. The difference list
+  against that file is CLOSED, has exactly seven items, and is enumerated in
   the header block above.
 
   This runner ships ONE cell:
@@ -170,15 +177,18 @@
                targets (removal, settlement, conjunct), TOPGUN_PRUNE_RECORD
                armed, TOPGUN_EPOCH_WIDTH left unset (production default
                1000). Every other matrix literal is the parent's own
-               4-hour-cell literal, unchanged.
+               4-hour-cell literal, unchanged. The harness it launches gates
+               resident tombstone bytes on a derived level ceiling plus
+               level stability; the slope is report-only.
 
   Any other argument falls through to this text and exits 2.
 
   The run REFUSES TO START, before the build and before any clock, if
-  SPEC368_CODE_FREEZE still reads its own placeholder value, or unless the
+  SPEC370_CODE_FREEZE still reads its own placeholder value, or unless the
   .rs tree at HEAD is identical to the commit that literal names, or unless
   the .rs working tree is clean. Three guards, three distinct messages; none
-  has an override.
+  has an override. It also refuses, before any clock, a soak harness binary
+  that lacks the level-ceiling gate or was not built by this invocation.
 
   A provenance cell REQUIRES SOAK_SERVER_BINARY to be exported and to name an
   existing executable. No cell in this runner's table is one; the guard is
@@ -203,7 +213,7 @@
   plateau4h)  WIDTH="";  DURATION=14400; SAMPLE_INTERVAL=60; PROVENANCE=no
               CELL_CRASH_INTERVAL=0; ARM_LOG=yes
               CELL_LIVE_CENSUS=0
-              EXTRA_FLAGS=""; BASE="spec368-plateau4h${SPEC368_BASE_SUFFIX:-}" ;;
+              EXTRA_FLAGS=""; BASE="spec370-plateau4h${SPEC370_BASE_SUFFIX:-}" ;;
   *)          usage ;;
 esac
 
@@ -289,7 +299,7 @@
   PROV_BIN=""
 fi
 
-DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec368-${CELL}-data}"
+DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec370-${CELL}-data}"
 META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
 CONSOLE_LOG="${META_DIR}/harness-console.log"
 STOP_FILE="${META_DIR}/sampler.stop"
@@ -443,9 +453,9 @@
 #     yet, and the run refuses immediately rather than attempting to diff
 #     against a value that is not a revision.
 # ---------------------------------------------------------------------------
-SPEC368_CODE_FREEZE=d6a3d38f
-if [ "$SPEC368_CODE_FREEZE" = "PENDING_SPEC368_CODE_FREEZE" ]; then
-  echo "FATAL: SPEC368_CODE_FREEZE still reads its placeholder value." >&2
+SPEC370_CODE_FREEZE=b13afaed
+if [ "$SPEC370_CODE_FREEZE" = "PENDING_SPEC370_CODE_FREEZE" ]; then
+  echo "FATAL: SPEC370_CODE_FREEZE still reads its placeholder value." >&2
   echo "       The prune-conjunct instrument this runner samples is not yet" >&2
   echo "       committed under a named freeze commit. Refusing to start." >&2
   exit 2
@@ -456,15 +466,15 @@
   echo "       to the commit that produced it. Refusing to start." >&2
   exit 1
 fi
-if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC368_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
-  echo "FATAL: the .rs diff against the freeze commit ${SPEC368_CODE_FREEZE} could" >&2
+if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC370_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
+  echo "FATAL: the .rs diff against the freeze commit ${SPEC370_CODE_FREEZE} could" >&2
   echo "       not be computed, so the freeze cannot be asserted:" >&2
   printf '%s\n' "$FREEZE_RS_DIFF" >&2
   echo "       Refusing to start." >&2
   exit 1
 fi
 if [ -n "$FREEZE_RS_DIFF" ]; then
-  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC368_CODE_FREEZE}; this run would not be filed under it" >&2
+  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC370_CODE_FREEZE}; this run would not be filed under it" >&2
   printf '%s\n' "$FREEZE_RS_DIFF" >&2
   exit 1
 fi
@@ -588,6 +598,40 @@
   exit 1
 fi
 
+# Harness provenance. The server clauses above prove which server runs; this
+# block proves the same for the soak harness, whose tombstone gate is the one
+# this cell evaluates. A harness built from earlier sources would still run and
+# still write a soak.json -- with the retired slope verdict and without the
+# ceiling fields -- so its absence must be a refusal, not a surprise at readout.
+SOAK_BIN_SHA256="$(shasum -a 256 "$SOAK_BIN" 2>/dev/null | awk '{print $1}')"
+SOAK_BIN_BUILT="$(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
+# (a) The harness must carry the level-ceiling gate's own message literal.
+#     `grep -c`, not `grep -q`, for the SIGPIPE reason given at clause (a) above.
+HARNESS_GATE_LITERAL="tombstone-byte level ceiling breached"
+HARNESS_HITS="$(strings "$SOAK_BIN" | grep -c "$HARNESS_GATE_LITERAL" || true)"
+if [ "${HARNESS_HITS:-0}" -eq 0 ]; then
+  echo "FATAL: the soak harness binary does not contain '${HARNESS_GATE_LITERAL}'." >&2
+  echo "       binary: $SOAK_BIN" >&2
+  echo "       built:  ${SOAK_BIN_BUILT}" >&2
+  echo "       sha256: ${SOAK_BIN_SHA256:-<unavailable>}" >&2
+  echo "       The level-ceiling gate is what this cell evaluates, so a harness" >&2
+  echo "       without it was built from other sources. Refusing to start." >&2
+  exit 1
+fi
+# (b) It must have been produced by THIS invocation, unless the operator named
+#     it with SPEC365_SOAK_BIN -- that override is already recorded as UNPROVEN
+#     in the matrix, and the harness sha256 below still identifies it.
+if [ -z "${SPEC365_SOAK_BIN:-}" ] && [ "$(date -r "$SOAK_BIN" '+%s')" -lt "$RUN_START_EPOCH" ]; then
+  echo "FATAL: stale soak harness binary -- not built by this invocation." >&2
+  echo "       binary: $SOAK_BIN" >&2
+  echo "       built:  ${SOAK_BIN_BUILT}" >&2
+  echo "       run started: ${RUN_START_UTC}" >&2
+  exit 1
+fi
+# (c) Both launched binaries' identities travel on console line 1.
+PROV_LINE="${PROV_LINE} harness sha256=${SOAK_BIN_SHA256} harness_built=${SOAK_BIN_BUILT} tombstone_level_ceiling_gate=present"
+echo "$PROV_LINE"
+
 SOAK_MTIME="$(date -r "$SOAK_BIN" '+%s')"
 SERVER_MTIME="$(date -r "$SERVER_BIN" '+%s')"
 echo "soak binary:   $SOAK_BIN"
@@ -706,7 +750,7 @@
   echo "  soak binary:    $SOAK_BIN"
   echo "  soak binary commit: ${SOAK_BIN_COMMIT}"
   echo "  .rs working tree:   ${RS_TREE_STATE}"
-  echo "  code freeze:            ${SPEC368_CODE_FREEZE}"
+  echo "  code freeze:            ${SPEC370_CODE_FREEZE}"
   echo "  code freeze diff (.rs): ${FREEZE_DIFF_STATE}"
   echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
   echo "  server binary:  $SERVER_BIN"
@@ -737,6 +781,8 @@
   echo "  steady-interval ${STEADY_INTERVAL}; quiesce ${QUIESCE};"
   echo "  mem-sample-interval ${MEM_SAMPLE_INTERVAL}; wal-fsync ${WAL_FSYNC};"
   echo "  memory gate NEUTRALIZED (${MEM_MIN_GROWTH_MB}/${MEM_THRESHOLD_MB_PER_HOUR}/${MEM_CEILING_MB})"
+  echo "  tombstone gate: level ceiling K=2+ceil(S_A/W) epochs × W × b_max (S_A, b_max measured by the harness; A = TOPGUN_WAL_WATERMARK_STALL_BOUND_MS, not set by this runner); level stability 10 %; slope report-only"
+  echo "  harness sha256: ${SOAK_BIN_SHA256}"
   echo
 } | tee "$MATRIX_OUT"
 
```

### 1.4 Hunk → item map

There is one row per `@@` hunk header in the embedded diff above. Each hunk maps to one item of the
closed list, or to a `+`-joined combination when several items share a hunk. **A hunk that mapped to
none of the seven items would be a defect.** There is none.

| hunk | item(s) | what changed, and where it sits in the parent |
|---|---|---|
| `@@ -1,58 +1,65 @@` | **5** | the successor header block, parent `:3-55`, rewritten to describe this runner and this seven-item list; the parent's naming of its own freeze and suffix variables disappears with the rewrite. The parent's closing preamble sentence (`:57-58`, "Everything from here on is spec365-conjuncts.sh's own header") is unchanged trailing context. |
| `@@ -101,7 +108,7 @@` | **2** | parent `:104`, inherited spec365-header prose naming the freeze variable. |
| `@@ -122,7 +129,7 @@` | **2** | parent `:125`, inherited spec365-header prose naming the freeze variable. |
| `@@ -158,10 +165,10 @@` | **5** | the usage heredoc, parent `:161` and `:163-164`: the runner name, and the parent it is a copy of, with "six" → "seven". |
| `@@ -170,15 +177,18 @@` | **2+5** | the usage heredoc, parent `:173-181`: the cell description gains one sentence on the gate (`:173`, item 5), the freeze variable at `:178` is renamed (item 2), and one sentence on the harness refusal is added (`:181`, item 5). |
| `@@ -203,7 +213,7 @@` | **1+2** | the one cell line, parent `:206`: the artifact basename (item 1) and its suffix variable (item 2). `DURATION=14400`, `SAMPLE_INTERVAL=60`, `WIDTH=""`, `PROVENANCE=no` and the remaining literals sit on untouched context lines. |
| `@@ -289,7 +299,7 @@` | **4** | parent `:292`, the default data-dir literal's prefix; the `SPEC365_DATA_DIR` override name is untouched. |
| `@@ -443,9 +453,9 @@` | **2+3** | parent `:446-448`: the freeze literal becomes `b13afaed` (item 3); the variable, its placeholder string and its FATAL message are renamed (item 2). |
| `@@ -456,15 +466,15 @@` | **2** | parent `:459-460` and `:467`, the two remaining freeze guards' reads and messages. |
| `@@ -588,6 +598,40 @@` | **6** | inserted immediately after the soak-binary resolution, which ends at parent `:590`: `SOAK_BIN_SHA256`, clause (a) (the gate literal via `strings | grep -c`, FATAL naming binary / built / sha256), clause (b) (mtime ≥ `RUN_START_EPOCH` unless `SPEC365_SOAK_BIN` is set), and clause (c) (`PROV_LINE` extended and echoed once more). `write_console_out` is untouched and writes the extended line as console line 1. |
| `@@ -706,7 +750,7 @@` | **2** | parent `:709`, the matrix's `code freeze:` line. |
| `@@ -737,6 +781,8 @@` | **7** | two `echo` lines appended to the matrix block after the memory-gate line, parent `:739`: the tombstone-gate description and `  harness sha256: ${SOAK_BIN_SHA256}`. |

**12 hunks, seven items:** item 1 → 1, item 2 → 7, item 3 → 1, item 4 → 1, item 5 → 3, item 6 → 1,
item 7 → 1. These counts sum to more than 12 because three hunks carry two items each.

AC-9 checks on the committed runner:
- `grep -c SPEC368_ spec370-plateau4h.sh` = 0.
- `grep -nE 'DURATION=14400'` matches once, on the cell line.
- `grep -c 'TOPGUN_WAL_WATERMARK_STALL_BOUND_MS='` = 0.
- There is no `SPEC-`/`TODO-` id.
- `export TOPGUN_PRUNE_RECORD=true` is present (parent `:372`, successor `:382`), so the three
  `…MaxObserved` fields that P-K reads are armed.

### 1.5 Byte-identity of the parts that must be identical

Each region was extracted from **both** files by an anchor pattern plus a fixed line count, and the two
extracts were diffed. Every anchor matches exactly once in each file, and all eight regions report
IDENTICAL:

| region | anchor | lines | parent | successor | result |
|---|---|---|---|---|---|
| `CSV_HEADER` (the column literal the frozen readout re-asserts) | `^CSV_HEADER=` | 1 | `:831` | `:877` | IDENTICAL |
| `PRUNE_METRIC_NAMES` (the scrape list) | `^PRUNE_METRIC_NAMES=` | 1 | `:837` | `:883` | IDENTICAL |
| the pinned matrix block, `CHURN_CLIENTS=6` … `JITTER_SEED=20260831` (memory gate still neutralized) | `^CHURN_CLIENTS=6$` | 23 | `:400-422` | `:410-432` | IDENTICAL |
| the environment-discipline block, `if [ -n "$WIDTH" ]` … the inherited `TOPGUN_WAL_FSYNC_POLICY` note | `^if \[ -n "\$WIDTH" \]; then$` | 50 | `:344-393` | `:354-403` | IDENTICAL |
| the server provenance block, `SERVER_BIN_SHA256=` … `write_console_out() {…}` (clauses (a)/(b)/(c)) | `^SERVER_BIN_SHA256=` | 60 | `:517-576` | `:527-586` | IDENTICAL |
| `TOPGUN_PRUNE_RECORD` arming | `^export TOPGUN_PRUNE_RECORD=true$` | 1 | `:372` | `:382` | IDENTICAL |
| the harness invocation, `"$SOAK_BIN" \` … `HARNESS_PID=$!` | `^"\$SOAK_BIN" \\$` | 30 | `:761-790` | `:807-836` | IDENTICAL |
| the readout invocation, `READOUT_SCRIPT=` … `exit "$HARNESS_RC"` | `^READOUT_SCRIPT=` | 10 | `:1241-1250` | `:1287-1296` | IDENTICAL |

Everything outside these regions is covered by the 12-hunk enumeration above. No hunk touches the log
directive, the fresh-data-dir guard, the artifact-overwrite refusal and its `.scrapes/` clause, the matrix
banner line, scrape persistence, `footprint_row`, or the post-run checks and fits.

### 1.6 Consequences of keeping the list CLOSED at seven (recorded, not fixed)

1. **The matrix banner still reads `=== spec365 conjunct-readout run: cell plateau4h ===`.** It is
   byte-unchanged from the parent.
2. **Every `SPEC365_*` override and `SPEC362B_SMOKE_DURATION` keeps its name.** This way the unchanged
   `spec365-readout.sh` and the runner resolve the same `OUT_DIR`.
3. **The inherited spec365 header is verbatim apart from the item-2 renames.** It still contains two
   sentences that describe the grandparent's `conj900` cell. The executed cell line is the authority.
4. **The runner does not validate the basename suffix.** The chain is what refuses any value other than
   empty or `-r2`.
5. **Item 4's effective names** are the data dir `target/spec370-plateau4h-data` and the meta dir
   `target/spec370-plateau4h-data.meta`.
6. **Item 6 (b) needs a fresh harness build.** The harness binary must be newer than `RUN_START_EPOCH`.
   The provenance step before launch, `cargo clean -p topgun-server --release`, removes the bench binary
   along with the server, so the runner's own build is what produces it.

## §2 — Pre-registered predicates

Everything in this section is copied verbatim from the approved spec's normative fenced blocks and
tables: Response v3, plus the one-line pre-M correction v3a, which adds `rm -f` of `$BASE.pk.tmp` to
`spec370-predicates.sh`. Each program is also committed as a file beside this manifest. Each file is
byte-equal to its block here, and its digest is in §1.2.

### 2.1 The ceiling derivation (§D1 formula)

```
H_max = ⌈S_A / W⌉
K     = O + P + H_max = 2 + ⌈S_A / W⌉          (P = 1 and Q = 0 under the A7 premise)
C     = K × W × b_max
```

> **Premise P-A.** No tracked write-behind sequence stays pending longer than `A` =
> `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` (effective value).

The terms:
- `S_A` = `max_count_in_window(remove_attempts, A, Δ)`. The counted window spans `A`, plus one sample gap,
  plus `Δ`.
- `Δ` is the harness's 5 s sample interval. Premise P-Δ: the attempt-to-stamp latency is ≤ Δ.
- `W` = `effective_epoch_width()`.
- `b_max` = `churn_tag_bytes_max`.

`PK-premise=TRUE` iff `held_max_observed ≤ H_max ∧ durable_watermark_lag_max ≤ H_max + 1`. It gates the
`TG-OR-005` flip, not `PLATEAU=`.

### 2.2 The slope decision, P-L and the window guards (§D2)

**Decision.** The deciding conjuncts are **P-C (ceiling)** and **P-L (level stability)**. No slope
statistic decides anything in the harness or in the cell. The last-half Theil–Sen slope is computed by
`spec370-ts.awk` and RECORDED. The harness keeps its OLS slope in `slopeBytesPerHour`, report-only.

**P-L (normative):** with `mh` = mean of `tombstone_bytes` over the last half, `mq` = mean over the last
quarter, and `E = W × b_max`:

```
|mq − mh| ≤ 0.10 × max(mh, E)
```

The ±10 % relative form is SPEC-368's C3, pre-registered before that cell's data existed
(`spec368-pb.awk`). The `E` floor is new. It keeps the tolerance at or above one tenth of an epoch's
bytes, so that a near-zero level (for example, a run with little OR churn) cannot fail on a few bytes.
For SPEC-368 it is inactive (`0.10 × 35,172.9 = 3,517.3 B > 2,300 B`).

**Window guards (normative).** The ceiling clause is evaluated only when the last-half span is
≥ `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS` (120 s, unchanged). That keeps the CI smoke run (~25 s) and
the loaded 150 s run (last-half span ≈ 75 s) exactly as suppressed as they are today. The level clause
is evaluated only when the last-half span is ≥ `DEFAULT_TOMBSTONE_LEVEL_MIN_WINDOW_SECS` (600 s). The
bias of a window mean from an incomplete sawtooth cycle is at most `E / (2 c)` for `c` cycles. At the
default matrix a 300 s last quarter holds c ≈ 11 cycles, so the bias is ≤ 23,000 / 22 ≈ 1,045 B. That
is below the 3,517 B tolerance, and below the `E`-floor tolerance of 2,300 B. A suppressed clause
records its disposition and cannot fail the run. The blind-monitor clause (zero samples) is unchanged.

### 2.3 Predicates and programs

**Missing-line rule.** A missing STOP line (`PM-reconciled=`, `PM-split=`, `P5=`, `P6=`, `P7=`,
`PS-rows=`, `PS-verdict=`, `PV=`, `PR-rows=`, `PR-crashes=`, `PR-class=`) counts as FALSE. A missing
deciding line (`PK=`, `PC=`, `PL=`, `PH=`) counts as INDETERMINATE. `spec370-decide.awk` enforces both.

**Three classes of lines:**
- **STOP predicates** (any not TRUE ⇒ `PLATEAU=STOP`): PM-reconciled, PM-split, P5, P6, P7 (SPEC-368's P-M
  blocks; **A7 moved out of P-M**, per conductor rulings v4 ruling 2 item 3), PS-rows, PS-verdict (SPEC-368
  P-S, unchanged), PV, PR-rows, PR-crashes, PR-class.
- **Deciding predicates:** PK (derivation), PC, PL, PH.
- **Recorded, never gated for PLATEAU:** `PK-recorded`, `PK-crosscheck`, `PK-a7`, `PK-premise` (it gates
  only the `TG-OR-005` flip), the harness recorded JSON line, A7 table and
  per-epoch series, Theil–Sen, `PA-wal_watermark_alarm_lines`, P-F, census, fits, harness attribution.

Every block that is byte-identical to `spec368-predicates.sh` (slicing, fits, the P-M lines, P-S, A7, P-F,
census, attribution) is marked as such in §1's byte-identity section. The full program is pinned below
with no placeholders.

### P-K — `spec370-pk.awk` (pinned program)
Inputs: `-v stamps_window_max=<soak.json .tombstones.stampsInWindowMax> -v width=<soak.json .epochWidth>`.
The CSV supplies the RECORDED server-side rows: `PK-recorded`, `PK-crosscheck`, `PK-a7` and `PK-premise`
(TRUE/FALSE/UNKNOWN). `spec370-predicates.sh` adds `PK-latency` (P-Δ evidence, recorded, never gated)
from soak.json `orRemoveAckLatencyP99Ms`, `orRemoveAckLatencyMaxMs`, `sampleIntervalMs` and
`orRemoveUnackedCount`. `PK=<K>` is `2 + ceil(S / W)`, the same formula as `TombstoneLevelBound::derive`.
```awk
# P-K -- ceiling epochs derived from the fence-age window, and the recorded premise rows.
# -v stamps_window_max=<S from soak.json> -v width=<W>. Reads the CSV for the RECORDED server-side rows.
BEGIN { FS = ","
    if (stamps_window_max !~ /^[0-9]+$/ || width !~ /^[0-9]+$/ || width + 0 < 1) { badin = 1; exit 0 }
    H = int((stamps_window_max + width - 1) / width); K = 2 + H
}
NR == 1 { for (i = 1; i <= NF; i++) c[$i] = i
    if (!c["durable_watermark_lag"] || !c["ret_epochs_durability_only"] || !c["ret_epochs_both"] || !c["ret_epochs_claim_only"] || !c["ret_epochs_neither"] || !c["considered_total"] || !c["indexed_refs"]) { hdr = 1; exit 0 }
    next }
/^[ \t\r]*$/ { next }
{
    held = $c["ret_epochs_durability_only"] + $c["ret_epochs_both"] + $c["ret_epochs_claim_only"]
    if (held > hm) hm = held
    if ($c["ret_epochs_neither"] + 0 > nm) nm = $c["ret_epochs_neither"] + 0
    if ($c["durable_watermark_lag"] + 0 > lm) lm = $c["durable_watermark_lag"] + 0
    st = $c["considered_total"] + $c["indexed_refs"]; t = $1 + 0
    if (seen && t - pt <= 60 && st - ps > sm) sm = st - ps
    ps = st; pt = t; seen = 1
}
END {
    if (badin) { print "PK=INDETERMINATE reason=bad_inputs(stamps_window_max=" stamps_window_max ",width=" width ")"; exit 0 }
    printf("PK-derived stamps_window_max=%d epoch_width=%d held_max_derived=%d ceiling_epochs=%d\n", stamps_window_max, width, H, K)
    if (hdr) { print "PK-recorded reason=missing_columns"; print "PK-crosscheck=UNKNOWN reason=missing_columns"; print "PK-premise=UNKNOWN reason=missing_columns"; print "PK=" K; exit 0 }
    printf("PK-recorded held_max_observed=%d durable_watermark_lag_max=%d neither_max_observed=%d k_eff_expected_under_O2=4 recorded_not_gated\n", hm, lm, nm)
    printf("PK-crosscheck csv_stamps_per_row_max=%d stamps_window_max=%d <=%s recorded_not_gated\n", sm, stamps_window_max, (sm <= stamps_window_max + 0) ? "TRUE" : "FALSE")
    print ((nm == 0) ? "PK-a7 neither_max_observed=0 TRUE recorded_attributed_not_gated" : "PK-a7 neither_max_observed=" nm " FALSE recorded_attributed_not_gated")
    print ((hm <= H && lm <= H + 1) ? "PK-premise=TRUE held_max_observed=" hm " durable_watermark_lag_max=" lm " held_max_derived=" H : "PK-premise=FALSE held_max_observed=" hm " durable_watermark_lag_max=" lm " held_max_derived=" H)
    print "PK=" K
}
```

### P-C / P-L — `spec370-pc.awk` (pinned program)
Inputs: `-v epochs=<PK> -v width=<soak.json .epochWidth> -v tagmax=<soak.json .tombstones.tagBytesMax>`.
`width` ≠ 1000 makes PC FALSE, because the invariant is stated at the production width. The PL line
carries `direction=up|down|none` for the Decision table (pre-audit v1 answer 6).
```awk
# P-C / P-L -- mechanism ceiling on the whole-run maximum, level stability on the last half.
BEGIN { FS = ","
    if (epochs !~ /^[0-9]+$/ || width !~ /^[0-9]+$/ || tagmax !~ /^[0-9]+$/ || epochs + 0 < 1 || width + 0 < 1 || tagmax + 0 < 1) { badin = 1; exit 0 }
}
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{
    v = $yc; gsub(/[ \t\r]/, "", v)
    if (v == "") { skipped++; next }
    if (v !~ /^[0-9]+$/) { nonint++; next }
    t[n] = $1 + 0; y[n++] = v + 0
}
END {
    if (badin) { print "PC=INDETERMINATE reason=bad_inputs(epochs=" epochs ",width=" width ",tagmax=" tagmax ")"; print "PL=INDETERMINATE reason=bad_inputs"; exit 0 }
    if (hdr) { print "PC=INDETERMINATE reason=no_tombstone_bytes_column"; print "PL=INDETERMINATE reason=no_tombstone_bytes_column"; exit 0 }
    if (nonint > 0) { print "PC=INDETERMINATE reason=non_integer_cells(" nonint ")"; print "PL=INDETERMINATE reason=non_integer_cells(" nonint ")"; exit 0 }
    if (n < 8) { print "PC=INDETERMINATE reason=too_few_rows(" n + 0 ")"; print "PL=INDETERMINATE reason=too_few_rows(" n + 0 ")"; exit 0 }
    E = width * tagmax; C = epochs * E
    h = int(n / 2); q = int(3 * n / 4)
    mx = -1; for (i = 0; i < n; i++) if (y[i] > mx) { mx = y[i]; mxt = t[i] }
    lmx = -1; for (i = h; i < n; i++) { sh += y[i]; if (y[i] > lmx) lmx = y[i] }
    for (i = q; i < n; i++) sq += y[i]
    mh = sh / (n - h); mq = sq / (n - q)
    dev = mq - mh; if (dev < 0) dev = -dev
    tol = 0.10 * ((mh > E) ? mh : E)
    pc = (mx <= C); pl = (dev <= tol)
    printf("PC-inputs ceiling_epochs=%d epoch_width=%d tag_bytes_max=%d epoch_bytes=%d ceiling_bytes=%d%s\n", epochs, width, tagmax, E, C, (width + 0 == 1000) ? "" : " width_not_production")
    printf("PC-max n=%d skipped_empty=%d run_max=%d run_max_elapsed=%s last_half_max=%d ceiling=%d run_max_over_ceiling=%.3f <=ceiling=%s\n", n, skipped + 0, mx, mxt, lmx, C, mx / C, pc ? "TRUE" : "FALSE")
    printf("PC-mean last_half_mean=%.3f over_ceiling=%.3f recorded_implied_by_PC-max\n", mh, mh / C)
    dir = (mq > mh) ? "up" : ((mq < mh) ? "down" : "none")
    printf("PL half_start=%d quarter_start=%d last_half_mean=%.3f last_quarter_mean=%.3f deviation_bytes=%.3f tolerance_bytes=%.3f deviation_pct=%s direction=%s <=tolerance=%s\n", h, q, mh, mq, dev, tol, (mh > 0) ? sprintf("%.3f", 100 * dev / mh) : "NA", dir, pl ? "TRUE" : "FALSE")
    if (width + 0 != 1000) { print "PC=FALSE reason=width_not_production(" width ")"; print (pl ? "PL=TRUE" : "PL=FALSE reason=level_deviation_above_tolerance"); exit 0 }
    print (pc ? "PC=TRUE" : "PC=FALSE reason=run_max_above_ceiling")
    print (pl ? "PL=TRUE" : "PL=FALSE reason=level_deviation_above_tolerance")
}
```

### Theil–Sen — `spec370-ts.awk` (pinned program; RECORDED)
```awk
# Theil-Sen slope over the last half of tombstone_bytes (B/h), RECORDED only.
BEGIN { FS = "," }
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{ v = $yc; gsub(/[ \t\r]/, "", v); if (v !~ /^[0-9]+$/) next; t[n] = $1 + 0; y[n++] = v + 0 }
END {
    if (hdr || n < 4) { print "PR-theil_sen=NA reason=" (hdr ? "no_column" : "too_few_rows"); exit 0 }
    h = int(n / 2); m = 0
    for (i = h; i < n; i++) for (j = i + 1; j < n; j++) if (t[j] > t[i]) s[m++] = (y[j] - y[i]) / (t[j] - t[i]) * 3600
    for (k = int(m / 2) - 1; k >= 0; k--) sift(k, m)
    for (e = m - 1; e > 0; e--) { x = s[0]; s[0] = s[e]; s[e] = x; sift(0, e) }
    med = (m % 2) ? s[int(m / 2)] : (s[m / 2 - 1] + s[m / 2]) / 2
    printf("PR-theil_sen last_half_rows=%d pairs=%d slope_bytes_per_hour=%.3f recorded_not_gated\n", n - h, m, med)
}
function sift(r, end,   c, x) { while (2 * r + 1 < end) { c = 2 * r + 1; if (c + 1 < end && s[c + 1] > s[c]) c++; if (s[r] >= s[c]) return; x = s[r]; s[r] = s[c]; s[c] = x; r = c } }
```

### P-H — `spec370-ph.awk` (pinned program)
Compares the harness's own (5 s) ceiling and level verdicts with the cell's PC/PL at the **same K and
b_max**, and prints both (pre-audit v1, "Also required"). **The K inputs are shared**: the harness
K and the cell K both come from soak.json `stampsInWindowMax`/`epochWidth`, so `ceiling_epochs_mismatch`
catches only an arithmetic slip in the harness's `derive`. PH's independent checks are the ceiling
arithmetic (`ceilingBytes = K × W × b`) and the two verdicts (harness 5 s ceiling/level against cell 60 s
PC/PL) (conductor rulings v1, rec 13). Input: one TSV line from the jq in
`spec370-predicates.sh`, with `-v pc= -v pl= -v pk= -v pb= -v pc_ceiling=`.
```awk
# P-H -- the harness (5 s) verdicts agree with the cell's at the same K and b_max.
BEGIN { FS = "\t"; split("ceilingBreached levelBreached ceilingDisposition levelDisposition ceilingEpochs epochWidth tagBytesMax stampsInWindowMax ceilingBytes peakBytes", nm, " ") }
NR == 1 {
    for (i = 1; i <= 10; i++) if ($i == "" || $i == "null") { printf("PH=INDETERMINATE reason=missing_field(%s)\n", nm[i]); done = 1; exit 0 }
    printf("PH-harness ceiling_ok=%s level_ok=%s ceiling_disposition=%s level_disposition=%s ceiling_epochs=%s epoch_width=%s tag_bytes_max=%s stamps_in_window_max=%s ceiling_bytes=%s peak_bytes=%s\n", ($1 == "false") ? "TRUE" : "FALSE", ($2 == "false") ? "TRUE" : "FALSE", $3, $4, $5, $6, $7, $8, $9, $10)
    printf("PH-cell ceiling_ok=%s level_ok=%s ceiling_epochs=%s tag_bytes_max=%s ceiling_bytes=%s\n", pc, pl, pk, pb, pc_ceiling)
    why = ""
    if ($5 + 0 != pk + 0) why = why " ceiling_epochs_mismatch"
    if ($7 + 0 != pb + 0) why = why " tag_bytes_mismatch"
    if ($9 + 0 != $5 * $6 * $7) why = why " ceiling_arithmetic"
    if ($9 + 0 != pc_ceiling + 0) why = why " ceiling_mismatch"
    if ($3 != "EVALUATED") why = why " ceiling_not_evaluated"
    if ($4 != "EVALUATED") why = why " level_not_evaluated"
    if ((($1 == "false") ? "TRUE" : "FALSE") != pc) why = why " ceiling_verdict_disagrees"
    if ((($2 == "false") ? "TRUE" : "FALSE") != pl) why = why " level_verdict_disagrees"
    print (why == "") ? "PH=TRUE" : "PH=FALSE reason=" substr(why, 2)
    done = 1
}
END { if (!done) print "PH=INDETERMINATE reason=no_input" }
```

### Decision — `spec370-decide.awk` (pinned program; runs LAST, over the finished predicates file)
```awk
# Deciding flags -- computed only after every STOP predicate line is already in the file.
BEGIN { ns = split("PM-reconciled PM-split P5 P6 P7 PS-rows PS-verdict PV PR-rows PR-crashes PR-class", stopk, " ") }
{
    for (i = 1; i <= ns; i++) { k = stopk[i]
        if (index($0, k "=") == 1) { seen[k]++; if (index($0, k "=TRUE") == 1) tru[k]++; else fal[k]++ } }
    if ($0 ~ /^PC=/) { pcs++; pc = $0 }
    if ($0 ~ /^PL=/) { pls++; pl = $0 }
    if ($0 ~ /^PH=/) { phs++; ph = $0 }
    if ($0 ~ /^PK=/) { pks++; pk = $0 }
    if ($0 ~ /^PK-premise=/) { prem = $0; sub(/^PK-premise=/, "", prem); sub(/ .*/, "", prem) }
    if ($0 ~ /^PA-wal_watermark_alarm_lines=[0-9]+ /) pa++
    if ($0 ~ /^PH-harness ceiling_ok=/) { hc = $0; sub(/^PH-harness ceiling_ok=/, "", hc); sub(/ .*/, "", hc) }
    if ($0 ~ /^PL half_start=/) { d = $0; sub(/.* deviation_pct=/, "", d); sub(/ .*/, "", d); devpct = d
                                  r = $0; sub(/.* direction=/, "", r); sub(/ .*/, "", r); dir = r }
}
END {
    bad = ""
    for (i = 1; i <= ns; i++) { k = stopk[i]; if (seen[k] != 1 || fal[k] > 0 || tru[k] != 1) bad = bad "," k }
    if (pcs != 1 || pls != 1 || phs != 1 || pks != 1) bad = bad ",deciding_lines"
    if (pc ~ /INDETERMINATE/ || pl ~ /INDETERMINATE/ || ph ~ /INDETERMINATE/ || pk ~ /INDETERMINATE/) bad = bad ",indeterminate"
    if (prem == "") prem = "UNKNOWN"
    if (bad != "") {
        print "DECISION stops=FAILED(" substr(bad, 2) ")"
        print "PREMISE=" prem
        print "PLATEAU=STOP reason=" substr(bad, 2)
        print "REPLICATE=NOT_AUTHORIZED"
        print "TG-OR-005=STAYS_OPEN"
        exit 0
    }
    print "DECISION stops=OK"
    print "PREMISE=" prem
    PC = (pc == "PC=TRUE"); PL = (pl == "PL=TRUE"); PH = (ph == "PH=TRUE")
    if (PC && PL && PH) {
        print "PLATEAU=TRUE"; print "READING=BOUNDED_STEADY"; print "REPLICATE=NOT_NEEDED"
        if (prem == "TRUE" && pa == 1) print "TG-OR-005=FLIP_TO_EVIDENCED"
        else print "TG-OR-005=STAYS_OPEN reason=premise_unverified"
        exit 0
    }
    if (!PC) { print "PLATEAU=FALSE reason=" ((prem == "FALSE") ? "ceiling_premise_violated" : "ceiling"); print "READING=NOT_BOUNDED"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (hc == "FALSE") { print "PLATEAU=FALSE reason=ceiling_5s"; print "READING=NOT_BOUNDED"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (!PH) { print "PLATEAU=FALSE reason=instrument_disagreement"; print "READING=INSTRUMENT_DISAGREEMENT"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (dir == "down") {
        print "PLATEAU=FALSE reason=level_decaying deviation_pct=" devpct; print "READING=BOUNDED_DECAYING"
        print (!replicate ? "REPLICATE=AUTHORIZED" : "REPLICATE=NOT_AUTHORIZED"); print "TG-OR-005=STAYS_OPEN"; exit 0
    }
    near = (devpct != "NA" && devpct + 0 <= 13.0)
    print "PLATEAU=FALSE reason=level_up deviation_pct=" devpct; print "READING=BOUNDED_NOT_STEADY"
    print ((near && !replicate) ? "REPLICATE=AUTHORIZED" : "REPLICATE=NOT_AUTHORIZED"); print "TG-OR-005=STAYS_OPEN"
}
```

### `spec370-predicates.sh <EV_DIR> <BASE>` (pinned program)
```bash
#!/usr/bin/env bash
# Runs the pre-registered slicing, fits, STOP predicates, deciding predicates, recorded readings and,
# LAST, the decision over the finished file, for one cell prefix.
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
EV="${1:-}"; BASE="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then echo "usage: spec370-predicates.sh <EV_DIR> <BASE>" >&2; exit 2; fi
: > "$EV/$BASE.fits.txt"
: > "$EV/$BASE.predicates.txt"
export LC_ALL=C
case "$BASE" in *-r2) REPL=1 ;; *) REPL=0 ;; esac

# Slicing: 8 equal-row windows; each segment file is overwritten.
( cd "$EV" && awk -F, 'NR==1{h=$0; next} {rows[++n]=$0}
  END{seg=int((n+7)/8);
      for(i=1;i<=8;i++){f=sprintf("'"$BASE"'-seg%d.csv",i); print h > f;
        for(j=(i-1)*seg+1; j<=i*seg && j<=n; j++) print rows[j] > f; close(f)}}' "$BASE.csv" )

# Fits: one newline-terminated record per window, FIT_ERROR on a non-zero fitter exit.
fit_record() {
  out="$(awk -f spec349c2-fit.awk -v col="$1" -v window="$2" "$3")"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
( cd "$EV" && for c in tombstone_bytes phys_footprint_mb reclaimable_mb; do
  for i in 1 2 3 4 5 6 7 8; do printf '%s W%s ' "$c" "$i"; fit_record "$c" full "$BASE-seg$i.csv"; done
  printf '%s LH ' "$c"; fit_record "$c" last_half "$BASE.csv"
done ) >> "$EV/$BASE.fits.txt"

{
  echo "== STOP: P-M =="
  awk '/^reconciliation=/ { seen=1; print ($0 ~ /^reconciliation=RECONCILED$/) ? "PM-reconciled=TRUE" : "PM-reconciled=FALSE reason=" $0 } END { if (!seen) print "PM-reconciled=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk '/^reconciliation=/ { seen=1; print (index($0,"split_epochs=")==0) ? "PM-split=TRUE" : "PM-split=FALSE reason=" substr($0, index($0,"split_epochs=")) } END { if (!seen) print "PM-split=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk -f "$EV/spec366-p5.awk" "$EV/$BASE.harness-console.log"
  awk -f "$EV/spec366-p67.awk" -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"

  echo "== STOP: P-S =="
  awk -f "$EV/spec368-ps.awk" "$EV/$BASE.csv"
  awk '/^READOUT: / { seen=1; v=$2; sub(/;$/, "", v); print (v == "O2" || v == "O3") ? "PS-verdict=TRUE verdict=" v : "PS-verdict=FALSE reason=verdict_" v } END { if (!seen) print "PS-verdict=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"

  echo "== STOP: PV =="
  awk 'NR==1 { ok = ($0 ~ /^provenance: server sha256=[0-9a-f]+ built=[^ ]+ run_start=[^ ]+ topgun_or_prune_restored_cancelled_total=present harness sha256=[0-9a-f]+ harness_built=[^ ]+ tombstone_level_ceiling_gate=present$/); s=$0; sub(/^provenance: server sha256=/, "", s); sub(/ .*/, "", s); h="NA"; if (index($0, " harness sha256=") > 0) { h=$0; sub(/.* harness sha256=/, "", h); sub(/ .*/, "", h) }; print "PV-line1 server_sha256=" s " harness_sha256=" h; line1 = ok } END { if (NR == 0) { print "PV=FALSE reason=empty_console_log"; exit 0 } if (!line1) { print "PV=FALSE reason=line1_shape"; exit 0 } print "PV-shape=TRUE" }' "$EV/$BASE.harness-console.log" > "$EV/$BASE.pv.tmp"
  cat "$EV/$BASE.pv.tmp"
  if grep -q '^PV-shape=TRUE$' "$EV/$BASE.pv.tmp"; then
    s1="$(sed -n 's/^PV-line1 server_sha256=\([^ ]*\) .*/\1/p' "$EV/$BASE.pv.tmp")"
    h1="$(sed -n 's/^PV-line1 .* harness_sha256=//p' "$EV/$BASE.pv.tmp")"
    sm="$(grep -E '^    sha256: +[0-9a-f]{64}$' "$EV/$BASE.matrix.txt" | head -1 | awk '{print $2}')"
    hm="$(grep -E '^  harness sha256: +[0-9a-f]{64}$' "$EV/$BASE.matrix.txt" | head -1 | awk '{print $3}')"
    if printf '%s\n%s\n' "$s1" "$h1" | grep -qvE '^[0-9a-f]{64}$'; then echo "PV=FALSE reason=sha_not_64_hex server=$s1 harness=$h1"
    elif [ "$s1" = "$sm" ] && [ "$h1" = "$hm" ]; then echo "PV=TRUE matrix_server_sha256=$sm matrix_harness_sha256=$hm"
    else echo "PV=FALSE reason=sha_mismatch server_line1=$s1 server_matrix=$sm harness_line1=$h1 harness_matrix=$hm"; fi
  fi
  rm -f "$EV/$BASE.pv.tmp"

  echo "== STOP: PR =="
  rows="$(( $(wc -l < "$EV/$BASE.csv") - 1 ))"
  scr="$(ls -1 "$EV/$BASE.scrapes" | grep -cE '^[0-9-]+T[0-9:]+Z\.txt$')"
  if [ "$rows" -ge 229 ] && [ "$rows" -eq "$scr" ]; then echo "PR-rows=TRUE rows=$rows scrapes=$scr"; else echo "PR-rows=FALSE rows=$rows scrapes=$scr"; fi
  cr="$(jq -r '.crashes // "null"' "$EV/$BASE.soak.json")"
  if [ "$cr" = "0" ]; then echo "PR-crashes=TRUE crashes=0 boot_gap_set=empty"; else echo "PR-crashes=FALSE crashes=$cr"; fi
  awk '/^RESULT: instrument sound; harness exit code [0-9]+\.$/ { c = 1 } /^RESULT: INSTRUMENT DEFECT/ { d = 1 } END { print (c && !d) ? "PR-class=TRUE" : "PR-class=FALSE reason=" (d ? "instrument_defect" : "no_sound_result_line") }' "$EV/$BASE.runner-console.log"

  echo "== DECIDING: P-K =="
  W="$(jq -r '.epochWidth // "null"' "$EV/$BASE.soak.json")"
  B="$(jq -r '.tombstones.tagBytesMax // "null"' "$EV/$BASE.soak.json")"
  S="$(jq -r '.tombstones.stampsInWindowMax // "null"' "$EV/$BASE.soak.json")"
  awk -v stamps_window_max="$S" -v width="$W" -f "$EV/spec370-pk.awk" "$EV/$BASE.csv" > "$EV/$BASE.pk.tmp"
  cat "$EV/$BASE.pk.tmp"
  K="$(sed -n 's/^PK=\([0-9][0-9]*\)$/\1/p' "$EV/$BASE.pk.tmp")"
  rm -f "$EV/$BASE.pk.tmp"
  jq -r '[.tombstones.orRemoveAckLatencyP99Ms, .tombstones.orRemoveAckLatencyMaxMs, .tombstones.sampleIntervalMs, .tombstones.orRemoveUnackedCount] | map(if . == null then "null" else tostring end) | @tsv' "$EV/$BASE.soak.json" \
    | awk -F '\t' 'NR == 1 { seen = 1
        if ($1 !~ /^[0-9]+$/ || $3 !~ /^[0-9]+$/) { printf("PK-latency=UNKNOWN p99_ms=%s max_ms=%s delta_ms=%s unacked=%s recorded_not_gated\n", $1, $2, $3, $4); next }
        printf("PK-latency p99_ms=%s max_ms=%s delta_ms=%s unacked=%s p99<=delta=%s max<=delta=%s recorded_not_gated\n", $1, $2, $3, $4, ($1 + 0 <= $3 + 0) ? "TRUE" : "FALSE", ($2 !~ /^[0-9]+$/) ? "UNKNOWN" : (($2 + 0 <= $3 + 0) ? "TRUE" : "FALSE")) }
      END { if (!seen) print "PK-latency=UNKNOWN reason=no_input recorded_not_gated" }'
  echo "== DECIDING: P-C / P-L =="
  awk -v epochs="${K:-x}" -v width="$W" -v tagmax="$B" -f "$EV/spec370-pc.awk" "$EV/$BASE.csv" > "$EV/$BASE.pc.tmp"
  cat "$EV/$BASE.pc.tmp"
  PCC="$(sed -n 's/^PC-inputs .* ceiling_bytes=\([0-9]*\).*/\1/p' "$EV/$BASE.pc.tmp")"
  PCV="$(grep -q '^PC=TRUE$' "$EV/$BASE.pc.tmp" && echo TRUE || echo FALSE)"
  PLV="$(grep -q '^PL=TRUE$' "$EV/$BASE.pc.tmp" && echo TRUE || echo FALSE)"
  rm -f "$EV/$BASE.pc.tmp"

  echo "== DECIDING: P-H =="
  jq -r '[.tombstones.ceilingBreached, .tombstones.levelBreached, .tombstones.ceilingDisposition, .tombstones.levelDisposition, .tombstones.ceilingEpochs, .epochWidth, .tombstones.tagBytesMax, .tombstones.stampsInWindowMax, .tombstones.ceilingBytes, .tombstones.peakBytes] | map(if . == null then "null" else tostring end) | @tsv' "$EV/$BASE.soak.json" \
    | awk -v pc="$PCV" -v pl="$PLV" -v pk="${K:-x}" -v pb="$B" -v pc_ceiling="${PCC:-0}" -f "$EV/spec370-ph.awk"
  jq -c '.tombstones | {kEffExpectedUnderO2, heldEpochsMaxObserved, neitherEpochsMaxObserved, durableWatermarkLagMaxObserved, fenceAgeBoundMs, stampsInWindowMax, slopeBytesPerHour}' "$EV/$BASE.soak.json"

  echo "== recorded: A7 =="
  awk -v t0="$(ls -1 "$EV/$BASE.scrapes" | grep -E '^[0-9-]+T[0-9:]+Z\.txt$' | sort | head -1 | cut -c1-19)" -f "$EV/spec368-a7.awk" "$EV/$BASE.harness-console.log"
  echo "== recorded: Theil-Sen =="
  awk -f "$EV/spec370-ts.awk" "$EV/$BASE.csv"
  echo "== recorded: WAL watermark alarms =="
  n="$(grep -c 'topgun_server::storage::wal_watermark' "$EV/$BASE.harness-console.log")"
  echo "PA-wal_watermark_alarm_lines=${n:-0} recorded_not_gated"
  echo "== recorded: P-F =="
  awk -f "$EV/spec368-pf.awk" "$EV/$BASE.csv"
  for c in phys_footprint_mb reclaimable_mb; do
    awk -v c="$c" '$1 == c && $2 == "LH" { for (i = 3; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { s = substr($i, 19); print "PF-shape " c " last_half_slope_sign=" ((s + 0 > 0) ? "+" : ((s + 0 < 0) ? "-" : "0")) " slope_mb_per_hour=" s; f = 1 } } END { if (!f) print "PF-shape " c " last_half_slope_sign=NA reason=no_LH_line" }' "$EV/$BASE.fits.txt"
  done
  jq -c '.reading, .reason, (.decidingSeries[] | {name, shape, firingEnvelope, lastHalfMean})' "$EV/$BASE.soak.durable.json"

  echo "== recorded: census =="
  jq -c '.censusTerminal | {source, elapsedSecs, keysScanned, keysUndecodable, orMapKeys, tombstoneEntries, tombstoneBytes, tombstoneDupEntries, keysWithTombstones, keysAllDead, maxTombstonesPerKey}' "$EV/$BASE.soak.durable.json"
  jq '.censuses | length' "$EV/$BASE.soak.durable.json"
  jq -c '.tombstoneCorpus' "$EV/$BASE.soak.json"

  echo "== recorded: harness attribution =="
  grep -E '^(RESULT:|RUNNER_EXIT=|csv rows:|harness exited with code)' "$EV/$BASE.runner-console.log"
  jq -r '.passed, .finishedReason, .durationSecsActual, (.pendingGates | length)' "$EV/$BASE.soak.json"
  head -1 "$EV/$BASE.harness-console.log"
  grep -E '^  +(sha256|code freeze|code freeze diff|harness sha256)' "$EV/$BASE.matrix.txt"
} >> "$EV/$BASE.predicates.txt"

# The deciding flags are computed only now, over the finished file, so no flag can be printed before a
# STOP predicate it depends on.
DEC="$(awk -v replicate="$REPL" -f "$EV/spec370-decide.awk" "$EV/$BASE.predicates.txt")"
{ echo "== DECISION =="; printf '%s\n' "$DEC"; } >> "$EV/$BASE.predicates.txt"
```

### `spec370-calibrate.sh <EV_DIR>` (pinned program)
```bash
#!/usr/bin/env bash
# Calibration of the level-ceiling predicates on committed 4 h cells: one expected PASS, two expected FAIL.
set -uo pipefail
EV="${1:-}"
if [ "$#" -ne 1 ] || [ ! -d "$EV" ]; then echo "usage: spec370-calibrate.sh <EV_DIR>" >&2; exit 2; fi
OUT="$EV/spec370-calibration.txt"
: > "$OUT"
export LC_ALL=C
# S_60 from the only calibration CSV that carries the stamp columns; applied to all three (same matrix).
S="$(awk -v stamps_window_max=0 -v width=1000 -f "$EV/spec370-pk.awk" "$EV/spec368-plateau4h.csv" | sed -n 's/^PK-crosscheck csv_stamps_per_row_max=\([0-9]*\) .*/\1/p')"
{ echo "## K derivation (spec368-plateau4h CSV)"; awk -v stamps_window_max="${S:-x}" -v width=1000 -f "$EV/spec370-pk.awk" "$EV/spec368-plateau4h.csv"; } >> "$OUT"
K="$(sed -n 's/^PK=\([0-9][0-9]*\)$/\1/p' "$OUT")"
C=$(( ${K:-0} * 1000 * 23 ))
for b in spec368-plateau4h spec355-w1000 spec362b-long4h; do
  {
    echo "## $b"
    awk -v epochs="${K:-x}" -v width=1000 -v tagmax=23 -f "$EV/spec370-pc.awk" "$EV/$b.csv"
    awk -f "$EV/spec370-ts.awk" "$EV/$b.csv"
    p="$(jq -r '.tombstones.peakBytes' "$EV/$b.soak.json")"
    if [ "$p" -le "$C" ] 2>/dev/null; then r=TRUE; else r=FALSE; fi
    echo "CAL-harness-peak $b peak_bytes=$p ceiling=$C <=ceiling=$r"
  } >> "$OUT"
done
```

### 2.4 Calibration record

1. **Calibration set (committed before this spec):**
   - `spec368-plateau4h` `.csv` + `.soak.json`, expected PASS;
   - `spec355-w1000` and `spec362b-long4h` `.csv` + `.soak.json`, both expected FAIL.

   All three share the matrix (W 1000, 6 churn clients, `OR_EVERY=5`, `WRITE_INTERVAL_MS=20`, no crash,
   4 h), and none carries harness-measured `stampsInWindowMax` / `tagBytesMax`. The calibration therefore
   derives `S_60` from the only CSV that has the stamp columns, `spec368-plateau4h`: `spec370-pk.awk`'s
   `csv_stamps_per_row_max`, 3,389. It applies the resulting K = 6 to all three, and uses b_max = 23
   (Deviation 1). The two references have 13–17 % lower mean OR-remove rates (from
   `totalWrites / 5 / duration`: 31.7 and 30.2 /s against 36.7 /s), so the SPEC-368 S is the conservative
   choice for them.
2. **Program:** `spec370-calibrate.sh` (§2) runs `spec370-pk.awk` on the SPEC-368 CSV (with
   `stamps_window_max=0` first, to read `PK-crosscheck csv_stamps_per_row_max`, then with that S). It then
   runs `spec370-pc.awk` and `spec370-ts.awk` on each CSV with the derived `epochs`, and compares each
   soak.json `tombstones.peakBytes` (5 s peak) with C. Output: `spec370-calibration.txt`, truncated first.
   In calibration `PK-crosscheck` is trivially TRUE, because S is the CSV value itself.
3. **Expectation (normative; mechanical).** `cmp` of the produced `spec370-calibration.txt` against the
   34-line block embedded in R-Exec. Any difference ⇒ S0, STOP before M. Its key lines are `PK=6`,
   `PK-premise=TRUE`, SPEC-368 `PC=TRUE`/`PL=TRUE` with `CAL-harness-peak … peak_bytes=60984 ceiling=138000 <=ceiling=TRUE`,
   and both references `PC=FALSE`/`PL=FALSE` with `<=ceiling=FALSE` (peaks 646,306 and 1,023,353).
4. **Robustness (non-normative):**

   | K (epochs) | C (B) | spec368 run max / C | spec355 / C | spec362b / C | discriminates |
   |---|---|---|---|---|---|
   | 4 (`K_eff`, recorded) | 92,000 | 0.652 | 6.844 | 11.025 | yes |
   | **6 (derived)** | **138,000** | **0.434** | **4.563** | **7.350** | **yes** |
   | 13 (pre-audit default, refuted as a bound) | 299,000 | 0.200 | 2.106 | 3.392 | yes |

   SPEC-368 passes for every K ≥ 3 (59,950 / 23,000 = 2.61), and both references fail for every K ≤ 27
   (629,654 / 23,000 = 27.4). The verdicts depend on the derivation only through that band.
5. **Decision:** only the fresh cell's `PLATEAU=` decides `TG-OR-005`. Calibration is never cited as
   evidence, and a mismatch is never repaired by changing a constant.

**Observed at G3.** `spec370-calibrate.sh` was run on a `mktemp -d` copy holding the programs and the
three calibration `.csv` + `.soak.json` pairs, with the working directory outside the evidence
directory. It exited 0. `cmp <produced> <the spec's 34-line R-Exec block>` exited **0**. The committed
`spec370-calibration.txt` is that output: 34 lines, sha256
`1b37f53e080a6590b859ad61bcc2a9b732f3ecc92846c4420adef1b4b4ee714a`.

```
## K derivation (spec368-plateau4h CSV)
PK-derived stamps_window_max=3389 epoch_width=1000 held_max_derived=4 ceiling_epochs=6
PK-recorded held_max_observed=2 durable_watermark_lag_max=3 neither_max_observed=0 k_eff_expected_under_O2=4 recorded_not_gated
PK-crosscheck csv_stamps_per_row_max=3389 stamps_window_max=3389 <=TRUE recorded_not_gated
PK-a7 neither_max_observed=0 TRUE recorded_attributed_not_gated
PK-premise=TRUE held_max_observed=2 durable_watermark_lag_max=3 held_max_derived=4
PK=6
## spec368-plateau4h
PC-inputs ceiling_epochs=6 epoch_width=1000 tag_bytes_max=23 epoch_bytes=23000 ceiling_bytes=138000
PC-max n=241 skipped_empty=0 run_max=59950 run_max_elapsed=3120 last_half_max=49312 ceiling=138000 run_max_over_ceiling=0.434 <=ceiling=TRUE
PC-mean last_half_mean=35172.893 over_ceiling=0.255 recorded_implied_by_PC-max
PL half_start=120 quarter_start=180 last_half_mean=35172.893 last_quarter_mean=35096.492 deviation_bytes=76.401 tolerance_bytes=3517.289 deviation_pct=0.217 direction=down <=tolerance=TRUE
PC=TRUE
PL=TRUE
PR-theil_sen last_half_rows=121 pairs=7260 slope_bytes_per_hour=745.032 recorded_not_gated
CAL-harness-peak spec368-plateau4h peak_bytes=60984 ceiling=138000 <=ceiling=TRUE
## spec355-w1000
PC-inputs ceiling_epochs=6 epoch_width=1000 tag_bytes_max=23 epoch_bytes=23000 ceiling_bytes=138000
PC-max n=240 skipped_empty=1 run_max=629654 run_max_elapsed=14400 last_half_max=629654 ceiling=138000 run_max_over_ceiling=4.563 <=ceiling=FALSE
PC-mean last_half_mean=490713.300 over_ceiling=3.556 recorded_implied_by_PC-max
PL half_start=120 quarter_start=180 last_half_mean=490713.300 last_quarter_mean=555204.533 deviation_bytes=64491.233 tolerance_bytes=49071.330 deviation_pct=13.142 direction=up <=tolerance=FALSE
PC=FALSE reason=run_max_above_ceiling
PL=FALSE reason=level_deviation_above_tolerance
PR-theil_sen last_half_rows=120 pairs=7140 slope_bytes_per_hour=130576.476 recorded_not_gated
CAL-harness-peak spec355-w1000 peak_bytes=646306 ceiling=138000 <=ceiling=FALSE
## spec362b-long4h
PC-inputs ceiling_epochs=6 epoch_width=1000 tag_bytes_max=23 epoch_bytes=23000 ceiling_bytes=138000
PC-max n=241 skipped_empty=0 run_max=1014337 run_max_elapsed=14400 last_half_max=1014337 ceiling=138000 run_max_over_ceiling=7.350 <=ceiling=FALSE
PC-mean last_half_mean=556957.860 over_ceiling=4.036 recorded_implied_by_PC-max
PL half_start=120 quarter_start=180 last_half_mean=556957.860 last_quarter_mean=714418.885 deviation_bytes=157461.026 tolerance_bytes=55695.786 deviation_pct=28.272 direction=up <=tolerance=FALSE
PC=FALSE reason=run_max_above_ceiling
PL=FALSE reason=level_deviation_above_tolerance
PR-theil_sen last_half_rows=121 pairs=7260 slope_bytes_per_hour=286662.664 recorded_not_gated
CAL-harness-peak spec362b-long4h peak_bytes=1023353 ceiling=138000 <=ceiling=FALSE
```

### 2.5 Decision table (pre-registered)

Rows are evaluated in this order; the first match decides.

| row | condition | `PLATEAU=` | `READING=` | `REPLICATE=` | `TG-OR-005=` | what follows |
|---|---|---|---|---|---|---|
| S0 | `spec370-calibration.txt` not `cmp`-identical to the R-Exec block (before M) | — | — | — | — | STOP before M; report; no constant changes |
| S1 | any STOP line not TRUE, or a deciding line (PK/PC/PL/PH) missing or INDETERMINATE | `STOP` | — | `NOT_AUTHORIZED` | `STAYS_OPEN` | record; no re-run without a ruling (pre-clock FATAL excepted) |
| T | PC ∧ PL ∧ PH, `PK-premise=TRUE`, `PA-wal_watermark_alarm_lines=` present | `TRUE` | `BOUNDED_STEADY` | `NOT_NEEDED` | `FLIP_TO_EVIDENCED` | §3, then the R-Invariant flip commit |
| T-p | PC ∧ PL ∧ PH, but `PK-premise` ≠ TRUE or no `PA-` line | `TRUE` | `BOUNDED_STEADY` | `NOT_NEEDED` | `STAYS_OPEN reason=premise_unverified` | §3 records; one caveat sentence only (R-Invariant); conductor rules |
| F-C | ¬PC | `FALSE reason=ceiling` (`ceiling_premise_violated` when `PK-premise=FALSE`) | `NOT_BOUNDED` | `NOT_AUTHORIZED` | `STAYS_OPEN` | §3 decomposes the run-max row into O/H/P/Q and cross-reads A7; a finding, never replicated |
| F-C5 | PC ∧ `PH-harness ceiling_ok=FALSE` (5 s peak > C while the 60 s max ≤ C) | `FALSE reason=ceiling_5s` | `NOT_BOUNDED` | `NOT_AUTHORIZED` | `STAYS_OPEN` | the bound holds at all times and the 5 s series is the finer instrument (conductor rulings v2, OQ 3); §3 quotes the harness peak |
| F-H | PC ∧ harness ceiling OK ∧ ¬PH (a level verdict disagreeing across cadences, e.g. aliasing, or an arithmetic/disposition check) | `FALSE reason=instrument_disagreement` | `INSTRUMENT_DISAGREEMENT` | `NOT_AUTHORIZED` | `STAYS_OPEN` | §3 names PH's reason; conductor rules (rulings v1 rec 10, v2 OQ 3) |
| F-Ld | PC ∧ PH ∧ ¬PL, direction `down`, first cell | `FALSE reason=level_decaying …` | `BOUNDED_DECAYING` | `AUTHORIZED` | `STAYS_OPEN` | one replicate `-r2` ("not yet steady", not "unbounded"); the replicate's own row decides |
| F-Ld′ | as F-Ld, but the replicate | `FALSE reason=level_decaying …` | `BOUNDED_DECAYING` | `NOT_AUTHORIZED` | `STAYS_OPEN` | §3 records; no third run |
| F-Lu | PC ∧ PH ∧ ¬PL, direction `up`, deviation ≤ 13 %, first cell | `FALSE reason=level_up …` | `BOUNDED_NOT_STEADY` | `AUTHORIZED` | `STAYS_OPEN` | one replicate `-r2`; its own row decides |
| F-Lu′ | as F-Lu, but deviation > 13 % or the replicate | `FALSE reason=level_up …` | `BOUNDED_NOT_STEADY` | `NOT_AUTHORIZED` | `STAYS_OPEN` | §3 records |

A T on `-r2` flips `TG-OR-005` citing both cells, under the same premise condition. `PREMISE=` is printed
on every decision. `PK-crosscheck` and `PK-a7` are recorded and attribute a reading; they never decide
one.

### 2.6 STOP branch

S0/S1 mean: no re-run, no re-tuning of the fixed epochs (2), A, the derivation, b, the 10 % tolerance or
either window, and no edit to §1/§2 or any program. `PLATEAU=FALSE` is a reading, not a STOP. The only
follow-ups are the replicate rows F-Lu and F-Ld.

### 2.7 R-Exec — executability before M, with the observed lines

Every item was executed on `mktemp -d` scratch copies of the files committed here, never with the
evidence directory as the working directory.

- **Calibration (normative, `cmp`):** `CAL_EXIT=0`, `cmp` exit **0** (§2.4).
- **`spec370-pk.awk` fail-closed.**
  - `stamps_window_max=abc` ⇒ `PK=INDETERMINATE reason=bad_inputs(stamps_window_max=abc,width=1000)`.
  - `spec355-w1000.csv` (S 3389) ⇒ `PK-derived stamps_window_max=3389 epoch_width=1000 held_max_derived=4 ceiling_epochs=6`, `PK-recorded reason=missing_columns`, `PK-crosscheck=UNKNOWN reason=missing_columns`, `PK-premise=UNKNOWN reason=missing_columns`, `PK=6`.
- **`spec370-pc.awk` fail-closed** (each prints both lines):
  - no `tombstone_bytes` column ⇒ `PC=INDETERMINATE reason=no_tombstone_bytes_column` / `PL=INDETERMINATE reason=no_tombstone_bytes_column`;
  - non-integer cells ⇒ `PC=INDETERMINATE reason=non_integer_cells(4)` / `PL=INDETERMINATE reason=non_integer_cells(4)`;
  - 7 rows ⇒ `PC=INDETERMINATE reason=too_few_rows(7)` / `PL=INDETERMINATE reason=too_few_rows(7)`;
  - `tagmax=0` ⇒ `PC=INDETERMINATE reason=bad_inputs(epochs=6,width=1000,tagmax=0)` / `PL=INDETERMINATE reason=bad_inputs`;
  - `epochs=x` ⇒ `PC=INDETERMINATE reason=bad_inputs(epochs=x,width=1000,tagmax=23)` / `PL=INDETERMINATE reason=bad_inputs`.
- **`spec370-ph.awk`.** SPEC-368 soak.json (old shape) ⇒ `PH=INDETERMINATE reason=missing_field(ceilingBreached)`.
  Hand-built TSV lines ⇒ `PH=TRUE`; `PH=FALSE reason=ceiling_verdict_disagrees`;
  `PH=FALSE reason=ceiling_epochs_mismatch ceiling_mismatch`.
- **`spec370-decide.awk`**, one synthetic predicates file per row (lines after `DECISION stops=…`):

  | row | printed |
  |---|---|
  | T | `PREMISE=TRUE PLATEAU=TRUE READING=BOUNDED_STEADY REPLICATE=NOT_NEEDED TG-OR-005=FLIP_TO_EVIDENCED` |
  | T-p (`PK-premise=FALSE`) | `PREMISE=FALSE PLATEAU=TRUE READING=BOUNDED_STEADY REPLICATE=NOT_NEEDED TG-OR-005=STAYS_OPEN reason=premise_unverified` |
  | T-p (`PK-premise=UNKNOWN`) | `PREMISE=UNKNOWN PLATEAU=TRUE … TG-OR-005=STAYS_OPEN reason=premise_unverified` |
  | T-p (no `PA-` line) | `PREMISE=TRUE PLATEAU=TRUE … TG-OR-005=STAYS_OPEN reason=premise_unverified` |
  | F-C | `PLATEAU=FALSE reason=ceiling READING=NOT_BOUNDED REPLICATE=NOT_AUTHORIZED TG-OR-005=STAYS_OPEN` |
  | F-C, `PK-premise=FALSE` | `PLATEAU=FALSE reason=ceiling_premise_violated READING=NOT_BOUNDED …` |
  | F-C5 (PL FALSE) | `PLATEAU=FALSE reason=ceiling_5s READING=NOT_BOUNDED REPLICATE=NOT_AUTHORIZED` |
  | F-C5 (PL TRUE, `PH=FALSE reason=ceiling_verdict_disagrees`) | `PLATEAU=FALSE reason=ceiling_5s READING=NOT_BOUNDED REPLICATE=NOT_AUTHORIZED` |
  | F-H | `PLATEAU=FALSE reason=instrument_disagreement READING=INSTRUMENT_DISAGREEMENT REPLICATE=NOT_AUTHORIZED` |
  | F-Lu 11.5 %, first | `PLATEAU=FALSE reason=level_up deviation_pct=11.5 READING=BOUNDED_NOT_STEADY REPLICATE=AUTHORIZED` |
  | F-Lu, `replicate=1` | `… REPLICATE=NOT_AUTHORIZED` |
  | F-Lu′ 14 % | `PLATEAU=FALSE reason=level_up deviation_pct=14 … REPLICATE=NOT_AUTHORIZED` |
  | F-Ld 30 % down, first | `PLATEAU=FALSE reason=level_decaying deviation_pct=30 READING=BOUNDED_DECAYING REPLICATE=AUTHORIZED` |
  | F-Ld′ `replicate=1` | `… READING=BOUNDED_DECAYING REPLICATE=NOT_AUTHORIZED` |
  | S1 `PV=FALSE` | `DECISION stops=FAILED(PV) … PLATEAU=STOP reason=PV REPLICATE=NOT_AUTHORIZED TG-OR-005=STAYS_OPEN` |
  | S1 no `PR-crashes`, `PH=INDETERMINATE` | `DECISION stops=FAILED(PR-crashes,indeterminate)` |

  A synthetic with `PH-harness ceiling_ok=FALSE` and `PH=TRUE` is not a reachable input: `spec370-ph.awk`
  prints `PH=FALSE reason=ceiling_verdict_disagrees` whenever the harness ceiling verdict and PC differ.
- **`spec370-predicates.sh` regeneration** (scratch copy of the `spec368-plateau4h.*` set, with the soak.json
  `tombstones` object extended as the spec pins, including `orRemoveAckLatencyP99Ms:12,
  orRemoveAckLatencyMaxMs:4871, sampleIntervalMs:5000, orRemoveUnackedCount:13`):
  - (a) ⇒ `PV-line1 server_sha256=1f506548…89ab3 harness_sha256=NA`, `PV=FALSE reason=line1_shape`, `PLATEAU=STOP reason=PV`.
  - (b) ⇒ `PV=FALSE reason=sha_not_64_hex server=1f506548…89ab3 harness=abc123`.
  - (c) ⇒ `PV=FALSE reason=sha_mismatch … harness_line1=2d711642…4881 harness_matrix=`.
  - (d) ⇒ `PV=TRUE …`, `PK-premise=TRUE held_max_observed=2 durable_watermark_lag_max=3 held_max_derived=4`,
    `PK-latency p99_ms=12 max_ms=4871 delta_ms=5000 unacked=13 p99<=delta=TRUE max<=delta=TRUE recorded_not_gated`,
    `PA-wal_watermark_alarm_lines=238 recorded_not_gated`, `PLATEAU=TRUE`, `TG-OR-005=FLIP_TO_EVIDENCED`; 623 lines.
  - Two consecutive runs in state (d): `predicates.txt`, `fits.txt` and the 8 segments `cmp`-identical ⇒
    `PREDICATES_REGEN=IDENTICAL`; `*.tmp` left: **0**.
  - `PK-latency` variants: `orRemoveAckLatencyMaxMs:6200` ⇒ `max<=delta=FALSE` with `PLATEAU=TRUE` and
    `TG-OR-005=FLIP_TO_EVIDENCED` unchanged; max `null` ⇒ `max<=delta=UNKNOWN`; p99 `null` ⇒
    `PK-latency=UNKNOWN p99_ms=null max_ms=4871 delta_ms=5000 unacked=13 recorded_not_gated`.

## APPEND-ONLY BELOW

## §3 — the 4 h level-ceiling cell (n=1): reading, regeneration, adjudication

Appended 2026-09-19, after the data commit D = `3ac48cd8a64184c7317012032060d86deb29a424`
(`chore(soak): record the 4 h level-ceiling cell artifacts`, 260 files). This append is the second
commit of the cell; the third, and last, is the `INVARIANTS.md` flip. Nothing above
`## APPEND-ONLY BELOW` changed, and no program, runner, `.rs` file or §1/§2 text changed after M
`024d1b4e`. D landed before any predicate line was read by a person.

**Outcome in one line: `PLATEAU=TRUE`, `READING=BOUNDED_STEADY`, `REPLICATE=NOT_NEEDED`,
`TG-OR-005=FLIP_TO_EVIDENCED`.** Every STOP predicate is TRUE. At K = 5 (C = 115,000 B) the run
maximum is 50,002 B (0.435 C). The level moved 38 B against a 3,469 B tolerance. The harness (5 s) and
the cell (60 s) agree.

### 3.1 Provenance, clean target, and the two refusal demos (AC-10, AC-12)

`cargo clean -p topgun-server --release` removed 52 files (189.7 MiB), and
`rm -f target/release/topgun-server` then left `test ! -e target/release/topgun-server` true. At that
point there was no `target/release/deps/soak_harness-*` and no `target/spec370*`.

**Demo 1: foreign server, clause (a).** This ran with `CARGO_TARGET_DIR` pointing at a scratch
directory whose `release/topgun-server` was a copy of `/usr/bin/true`, and with
`SPEC365_SOAK_BIN=/usr/bin/true`. `DEMO_EXIT=1`:

```
WARNING: SPEC365_SOAK_BIN is set, so this runner did NOT build the bench
         binary from HEAD. The freeze gate is NOT discharged for this run
         and matrix.txt will say so.
FATAL: the server binary does not contain 'topgun_or_prune_restored_cancelled_total'.
       binary: /var/folders/dy/35x7phnx3pz88gmkjf9560sm0000gn/T/tmp.NUhQNNVIGn/release/topgun-server
       built:  2026-09-18T14:30:27Z
       sha256: 875c7eea9c66c826091ede3cc44599311dc6818caf17f9e53d97c54c288842b2
       This counter is emitted by the branch under test, so a binary
       without it was built from other sources. Attempt 1 ran exactly
       such a binary and the cell was worthless.
```

**Demo 2: pin-built harness, item 6 (a).** The harness was built from the pin `86656caa` in a
detached worktree with a scratch `CARGO_TARGET_DIR` (`soak_harness-0379469b8a4255d8`, 0 hits for the
gate literal). The server was built from M in a second scratch target, and that copy carries the
counter. `DEMO2_EXIT=1`:

```
WARNING: SPEC365_SOAK_BIN is set, so this runner did NOT build the bench
         binary from HEAD. The freeze gate is NOT discharged for this run
         and matrix.txt will say so.
provenance: server sha256=ce7671e8f9553ccbab80e8e328971c393b1b876a4efa8c41001257818bf7e1bb built=2026-09-18T15:39:36Z run_start=2026-09-18T14:39:36Z topgun_or_prune_restored_cancelled_total=present
FATAL: the soak harness binary does not contain 'tombstone-byte level ceiling breached'.
       binary: /private/tmp/claude-501/-Users-koristuvac-Projects-topgun-topgun/0dcd65f5-0df0-4019-be9a-26fb80bd053d/scratchpad/demo2/target-pin/release/deps/soak_harness-0379469b8a4255d8
       built:  2026-09-18T14:33:49Z
       sha256: dae9ce3fef2683fc3fe6c50085cd86fa3377766f7b34ce84bfdf6715c386fef9
       The level-ceiling gate is what this cell evaluates, so a harness
       without it was built from other sources. Refusing to start.
```

The server clauses (a) and (b) passed, which is why the `provenance:` line precedes the FATAL. That
means item 6 is the clause that refused.

**Recorded deviation:** the scratch server copy was given a *future* mtime (`touch -t`, now + 1 h).
With a plain `cp`, its mtime would fall before `RUN_START_EPOCH`, clause (b) would refuse first, and
the demo would never reach item 6. The shift shows in the line above: `built=15:39:36Z`,
`run_start=14:39:36Z`. It touched only the scratch copy.

After each demo, `target/spec370-plateau4h-data` and its `.meta` did not exist, the evidence directory
had no untracked or modified file, and `target/release/topgun-server` did not exist.

**The measured binaries** were built by the runner's own invocation, and both identities travel with
the artifacts. Console line 1 of `spec370-plateau4h.harness-console.log` and the matrix:

```
provenance: server sha256=c5089faed37c58571007939eda55290f19f734dd0ab19d1263706dbd381fccf0 built=2026-09-18T14:43:49Z run_start=2026-09-18T14:40:11Z topgun_or_prune_restored_cancelled_total=present harness sha256=82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1 harness_built=2026-09-18T14:42:34Z tombstone_level_ceiling_gate=present
  code freeze:            b13afaed
  code freeze diff (.rs): EMPTY (asserted before the build)
  tombstone gate: level ceiling K=2+ceil(S_A/W) epochs × W × b_max (S_A, b_max measured by the harness; A = TOPGUN_WAL_WATERMARK_STALL_BOUND_MS, not set by this runner); level stability 10 %; slope report-only
  harness sha256: 82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1
```

Both binaries were built after `run_start`, and both shas are 64 hex. Each equals its matrix line,
which `PV=TRUE` checks mechanically (§3.4).

### 3.2 Runner attribution and readout

```
harness exited with code 0
csv rows: 241
RESULT: instrument sound; harness exit code 0.
RUNNER_EXIT=0
READOUT: O2; retained_closed_epochs=1; reconciliation=RECONCILED
```

Class = **reading**: the instrument is sound and the harness passed on its own gates (`passed=true`,
`finishedReason` `duration reached`, `durationSecsActual=14401`). The chain wrapper exited 0
(`### CHAIN COMPLETE`), with no `CHAIN_ABORT` and no FATAL. The harness's own gate line (console
`:4435`):

```
tombstone_level:   ceiling=115000 (5 epochs x width 1000 x tag bytes 23; stamps_in_window=2774) -> EVALUATED | last_half mean=33961.9 max=49841 span=7197s, last_quarter mean=34425.8, deviation=463.9 tolerance=3396.2 -> EVALUATED
```

### 3.3 `predicates.txt`, verbatim

The file has 619 lines (sha256 `05f4b44384e20f0fec8bdd85fd0ce907d7ea8283b72a38e0daa80cfb15aad5d4`).
The block below is that file **with only its 523 `^A7-epoch ` per-epoch data rows removed**
(`grep -v '^A7-epoch '`, 96 lines kept). Every predicate, recorded and decision line is included
unchanged. The per-epoch series is in the committed file itself, and its roll-up is the A7 table and
the `A7-removals` / `A7=TRUE` lines below.

```
== STOP: P-M ==
PM-reconciled=TRUE
PM-split=TRUE
P5=TRUE
P5-observed: removal_rows=523 settlement_rows=523 unsettled=none
P5-zero-return: none observed
DECISION_SCRAPE=2026-09-18T18:43:51Z.txt
P6=TRUE removed_refs_observed_total=523000 considered_total=523000 gap=0
P7=TRUE restored_cancelled_total=0
windows=523 settlement_rows=523 zero_return_removal_rows=0 scrapes=241
== STOP: P-S ==
PS-rows=TRUE rows=241 evaluable=241 exempt=0 first_snapshot_elapsed=0 max(sum-lag)=-1
PS-verdict=TRUE verdict=O2
== STOP: PV ==
PV-line1 server_sha256=c5089faed37c58571007939eda55290f19f734dd0ab19d1263706dbd381fccf0 harness_sha256=82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1
PV-shape=TRUE
PV=TRUE matrix_server_sha256=c5089faed37c58571007939eda55290f19f734dd0ab19d1263706dbd381fccf0 matrix_harness_sha256=82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1
== STOP: PR ==
PR-rows=TRUE rows=241 scrapes=241
PR-crashes=TRUE crashes=0 boot_gap_set=empty
PR-class=TRUE
== DECIDING: P-K ==
PK-derived stamps_window_max=2774 epoch_width=1000 held_max_derived=3 ceiling_epochs=5
PK-recorded held_max_observed=2 durable_watermark_lag_max=3 neither_max_observed=1 k_eff_expected_under_O2=4 recorded_not_gated
PK-crosscheck csv_stamps_per_row_max=3342 stamps_window_max=2774 <=FALSE recorded_not_gated
PK-a7 neither_max_observed=1 FALSE recorded_attributed_not_gated
PK-premise=TRUE held_max_observed=2 durable_watermark_lag_max=3 held_max_derived=3
PK=5
PK-latency p99_ms=23 max_ms=531 delta_ms=5000 unacked=0 p99<=delta=TRUE max<=delta=TRUE recorded_not_gated
== DECIDING: P-C / P-L ==
PC-inputs ceiling_epochs=5 epoch_width=1000 tag_bytes_max=23 epoch_bytes=23000 ceiling_bytes=115000
PC-max n=241 skipped_empty=0 run_max=50002 run_max_elapsed=11820 last_half_max=50002 ceiling=115000 run_max_over_ceiling=0.435 <=ceiling=TRUE
PC-mean last_half_mean=34691.413 over_ceiling=0.302 recorded_implied_by_PC-max
PL half_start=120 quarter_start=180 last_half_mean=34691.413 last_quarter_mean=34729.623 deviation_bytes=38.210 tolerance_bytes=3469.141 deviation_pct=0.110 direction=up <=tolerance=TRUE
PC=TRUE
PL=TRUE
== DECIDING: P-H ==
PH-harness ceiling_ok=TRUE level_ok=TRUE ceiling_disposition=EVALUATED level_disposition=EVALUATED ceiling_epochs=5 epoch_width=1000 tag_bytes_max=23 stamps_in_window_max=2774 ceiling_bytes=115000 peak_bytes=49841
PH-cell ceiling_ok=TRUE level_ok=TRUE ceiling_epochs=5 tag_bytes_max=23 ceiling_bytes=115000
PH=TRUE
{"kEffExpectedUnderO2":4,"heldEpochsMaxObserved":2,"neitherEpochsMaxObserved":1,"durableWatermarkLagMaxObserved":3,"fenceAgeBoundMs":60000,"stampsInWindowMax":2774,"slopeBytesPerHour":768.6789621274689}
== recorded: A7 ==
window | span_s | latencies | max_latency_s | intervals | min_interval_s | ratio
W1 | 0-1800 | 62 | 7.382 | 61 | 15.572 | 0.474
W2 | 1800-3600 | 68 | 6.843 | 68 | 16.050 | 0.426
W3 | 3600-5400 | 64 | 7.912 | 64 | 15.623 | 0.506
W4 | 5400-7200 | 65 | 6.093 | 65 | 15.585 | 0.391
W5 | 7200-9000 | 63 | 7.302 | 63 | 16.051 | 0.455
W6 | 9000-10800 | 65 | 6.312 | 65 | 17.038 | 0.370
W7 | 10800-12600 | 67 | 7.269 | 67 | 16.956 | 0.429
W8 | 12600-14400 | 69 | 6.889 | 69 | 15.399 | 0.447
A7-removals=523 settlements=523 unsettled=none
A7=TRUE
== recorded: Theil-Sen ==
PR-theil_sen last_half_rows=121 pairs=7260 slope_bytes_per_hour=388.152 recorded_not_gated
== recorded: WAL watermark alarms ==
PA-wal_watermark_alarm_lines=0 recorded_not_gated
== recorded: P-F ==
PF-recon2 rows_within_2pct=232/240 MET
PF-recon3 rows_within_2pct=240/240 MET
PF-phys_footprint_mb peak=8236.994 last=8236.994 peak_eq_last=TRUE
PF-reclaimable_mb peak=4066.625 last=1130.297 peak_eq_last=FALSE
PF-phys_footprint_peak_mb last=8236.994
PF-shape phys_footprint_mb last_half_slope_sign=+ slope_mb_per_hour=1688.743672
PF-shape reclaimable_mb last_half_slope_sign=- slope_mb_per_hour=-1460.246699
"PLATEAU_NOT_MET"
"series rss_kib rose and was still rising at the end; firing envelope BOTH"
{"name":"rss_kib","shape":"MONOTONE_RISING","firingEnvelope":"BOTH","lastHalfMean":8776962}
{"name":"redb_bytes","shape":"MONOTONE_RISING","firingEnvelope":"PEAKS","lastHalfMean":236958913}
{"name":"wal_bytes","shape":"RISING_DECELERATING","firingEnvelope":null,"lastHalfMean":1391287}
{"name":"wal_segment_files","shape":"MONOTONE_RISING","firingEnvelope":"PEAKS","lastHalfMean":518}
== recorded: census ==
{"source":"TERMINAL","elapsedSecs":14401.943739042,"keysScanned":96,"keysUndecodable":0,"orMapKeys":96,"tombstoneEntries":1533,"tombstoneBytes":35259,"tombstoneDupEntries":0,"keysWithTombstones":48,"keysAllDead":0,"maxTombstonesPerKey":41}
1
{"scansAttempted":1,"scansFailed":0,"samples":1,"firstBytes":35259,"minBytes":35259,"peakBytes":35259,"lastBytes":35259,"firstHalfPeakBytes":0,"lastHalfPeakBytes":35259,"riseBytes":35259,"spanSecs":0.0,"disposition":"LEVEL_SUPPRESSED","ceilingBytes":null,"passed":true,"reason":null}
== recorded: harness attribution ==
harness exited with code 0
csv rows: 241
RESULT: instrument sound; harness exit code 0.
RUNNER_EXIT=0
true
duration reached
14401
0
provenance: server sha256=c5089faed37c58571007939eda55290f19f734dd0ab19d1263706dbd381fccf0 built=2026-09-18T14:43:49Z run_start=2026-09-18T14:40:11Z topgun_or_prune_restored_cancelled_total=present harness sha256=82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1 harness_built=2026-09-18T14:42:34Z tombstone_level_ceiling_gate=present
  code freeze:            b13afaed
  code freeze diff (.rs): EMPTY (asserted before the build)
    sha256:       c5089faed37c58571007939eda55290f19f734dd0ab19d1263706dbd381fccf0
  harness sha256: 82b50c9c83821f456f90d8822335cbfb41e0b02f6dc4fdf39c38a37aee1f7bc1
== DECISION ==
DECISION stops=OK
PREMISE=TRUE
PLATEAU=TRUE
READING=BOUNDED_STEADY
REPLICATE=NOT_NEEDED
TG-OR-005=FLIP_TO_EVIDENCED
```

### 3.4 STOP predicates — all TRUE

`PM-reconciled`, `PM-split`, `P5`, `P6` (523,000 = 523,000, gap 0), `P7` (0 restored-cancelled),
`PS-rows` (241/241 evaluable), `PS-verdict` (O2), `PV` (both shas 64 hex, each equal to its matrix
line), `PR-rows` (241 rows = 241 scrapes), `PR-crashes` (0) and `PR-class` are all TRUE. The
`DECISION stops=OK` line confirms that `spec370-decide.awk` found every STOP line present and TRUE.

### 3.5 Deciding predicates — PK = 5, PC, PL, PH TRUE

- **P-K:** `stamps_window_max=2774` ⇒ `held_max_derived = ⌈2774/1000⌉ = 3` ⇒ **K = 5**, C = 5 × 1000 ×
  23 = **115,000 B**. The server-side rows are recorded, not gated: `held_max_observed=2`,
  `durable_watermark_lag_max=3`, `neither_max_observed=1`. `PK-premise=TRUE` (2 ≤ 3 and 3 ≤ 4).
- **P-C:** run max 50,002 B at t = 11,820 s = **0.435 C**, and TRUE. The last-half mean of 34,691 B
  (0.302 C) is recorded, and P-C implies it.
- **P-L:** last-half mean 34,691.413 B, last-quarter mean 34,729.623 B, deviation **38.210 B (0.110 %)**
  against a 3,469.141 B tolerance, direction `up`. TRUE.
- **P-H:** the harness (5 s: peak 49,841 B, both dispositions `EVALUATED`) and the cell agree on K, b
  and C, and both clauses are OK. TRUE.
- Recorded: A7 is TRUE in all eight windows (max ratio 0.506, W3). The Theil–Sen last-half slope is
  388 B/h, and the harness's report-only OLS slope is 768.7 B/h; neither gates. There were 0 WAL
  watermark alarm lines.

### 3.6 Which side of the K boundaries S landed on (R-Artifacts (a))

The spec pre-registered the question against the 6→7 boundary at S = 4,000, because the calibration
CSV gave S_60 = 3,389 (K = 6). The cell's `stampsInWindowMax` = **2,774**. That is **1,226 below** the
6→7 boundary. It is also **226 below** the 5→6 boundary at 3,000, so the cell derived and gated on
**K = 5**, one epoch *tighter* than the calibration K. The calibration verdicts (§2.4) were computed
at K = 6 on other data and are not re-read here. This cell's gate is its own K = 5, as the invariant
text requires ("the K the cited cell measured and gated on").

### 3.7 P-Δ (R-Artifacts (b))

*P-Δ is evidenced on acked removes (`PK-latency` p99 and max); the recorded un-acked count bounds what
that evidence cannot see.* In this cell: `PK-latency p99_ms=23 max_ms=531 delta_ms=5000 unacked=0`.
Every attempted remove was acked, and the slowest took 531 ms against a 5,000 ms allowance, so no
attempt fell outside the evidence.

### 3.8 The PK-crosscheck discrepancy — disclosed, tracked, not repaired (conductor rulings v4, Ruling 2)

**The recorded row.** `PK-crosscheck csv_stamps_per_row_max=3342 stamps_window_max=2774 <=FALSE
recorded_not_gated`. This row never gates (§2 "Recorded, never gated"). It is FALSE, and it is
disclosed here with both server-side proxies:

| proxy (per 60 s CSV row) | definition | max | at `elapsed_secs` | vs `S_A = 2,774` (70 s window) |
|---|---|---|---|---|
| `spec370-pk.awk` crosscheck | `Δ(considered_total + indexed_refs)` | **3,342** | 3,180 | +568 (S_A is 17.0 % below) |
| stamped-bytes proxy | `Δ(stamped_bytes_total) / 23` | **3,000** (69,000 B) | 4,800 | +226 (S_A is 7.5 % below) |

Read at face value, both proxies say the harness's S_A under-counts server stamps by 8–20 % (the
conductor's range). 51 of the 240 row deltas of the first proxy exceed 2,774.

**Direction of the error.** A smaller S_A gives a smaller K and a smaller C, so the gate becomes
*stricter*. That is the pre-registered direction (§D1.3, and `max_count_in_window`'s own doc,
`monitor.rs:834-836`): an under-count can produce a false breach, never a false pass. The cell's
PASS therefore does not depend on S_A being exact.

**Recorded row: `C_csv` at the proxy's S.** With S = 3,342: `H = ⌈3342/1000⌉ = 4`, **K = 6**,
`C_csv = 6 × 1000 × 23 = 138,000 B`. The run max of 50,002 B is **≤ C_csv** (0.362 C_csv), and so is
the harness's 5 s peak of 49,841 B. The 3,000 proxy also gives K = 5 (⌈3000/1000⌉ = 3), the same C as
the gate used. Under either proxy, the run max clears the ceiling that proxy implies by more than 2×.

**Best-effort explanation, from the counter definitions only (no code change).**

*What the harness counts.* `or_remove_attempts` (`main.rs:426-429`) is one shared `Arc<SoakMetrics>`
counter for **every** churn client (`main.rs:633`, `:644`). It is incremented **once per tag**, never
per batch. The increment comes after that tag's `or_add` returns `Ok` and before its `or_remove` is
issued (`main.rs:3751-3760`). The harness has exactly one `or_remove` call site (`main.rs:3760`), and
the persist stream (`ork-persist-*`) never removes. The sampler reads the counter once per kept 5 s
byte sample, after the scrape's `elapsed` (`main.rs:815-821`). `max_count_in_window`
(`monitor.rs:838-857`) takes differences against the last sample at or before
`t_{i-1} − A − Δ` (`main.rs:1200-1212`: A = 60 s, Δ = the 5 s sample interval), so each counted
window spans up to ≈ 70 s.

*What the server stamps.* The server stamps at most once per remove: only when the apply reports a
genuinely new tombstone (`crdt.rs:688-697` sets `stamped_new_tombstone`, and `crdt.rs:733-736` calls
`stamp_tombstone`). Each stamp adds one ref to `indexed_refs` (`tombstone_frontier_impl.rs:577`).
From the definitions alone, then, stamps in any window ≤ attempts in a window that reaches back one
latency allowance further, and this cell's `unacked=0` and 531 ms max latency sit well inside that
allowance. The definitions do not by themselves predict a harness under-count.

*What the proxies include, and why each can exceed the in-row stamp count by up to one epoch.*
1. `Δ(stamped_bytes_total)/23`: the exported counter is **not** credited per stamp. It is credited
   **per epoch, at rollover**, with that epoch's whole accumulated total
   (`observe_epoch_entry`, `tombstone_frontier_impl.rs:3186-3197`, fired from the rollover branch at
   `:605` via `publish_epoch_entry`, `:2084`). A row's delta is therefore a sum of whole epochs of up
   to W = 1,000 refs each. Three rollovers inside one 60 s row need only ≥ 2,001 stamps in that row;
   the other ≤ 999 refs of the first credited epoch were stamped in earlier rows. `Δ/23` is a lower
   bound on refs *credited* (tags ≤ 23 B). It is not a lower bound on refs *stamped in that row*. The
   in-row stamps implied by the 3,000 reading are ≥ 2,001, which is consistent with 2,774.
2. `Δ(considered_total + indexed_refs)`: the two terms are published at different instants of one
   prune pass. The drain removes refs from the index and republishes the lower `indexed_refs` gauge
   at the start of the pass (`tombstone_frontier_impl.rs:1026-1027`, `:2274`; called at
   `crdt.rs:1760`). `considered_total` is credited only when the finished pass is observed
   (`crdt.rs:1658` → `tombstone_frontier_impl.rs:3088`). Pass latency was 2.9–7.9 s (the A7 table),
   so a scrape that falls inside a pass sees the sum about one epoch low, and the next row's delta is
   about one epoch high. The rows around the maximum show that sawtooth:
   `Δ = 1,347 (t=3,120) → 3,342 (t=3,180) → 2,300 → 2,298 → 1,058 (t=3,360)`, which averages 2,249 per
   row over 3,180–3,360. Restores would add to this proxy too (`restore` re-adds a ref to
   `indexed_refs`, `tombstone_frontier_impl.rs:1101-1110`, and the drained ref was already counted in
   `considered`), but `P6` shows `considered_total = removed_refs_observed_total = 523,000` and `P7`
   shows 0 restored-cancelled, so restores do not explain this cell's excess.
3. Whole-run context: the first proxy ends at 524,397 over 14,400 s, a mean of 2,185 per 60 s or
   2,549 per 70 s. Over two, three and four rows its maximum per-60 s rate falls to 2,821, 2,664 and
   2,551.

*Reading.* By their definitions, both proxies carry up to one epoch (≈ 1,000 refs) of upward
per-row skew. That alone covers the whole gap (568 and 226 refs), so this cell's data does not show a
real under-count by the harness. It does not exclude one either, because neither proxy is an exact
per-window stamp count. The conductor's 8–20 % figure is therefore recorded as the **upper** estimate
of the under-count, measured against the proxies. Whichever reading holds, the direction argument
above keeps the flip sound. Resolving it (an exact per-window server stamp count, or a proxy without
the skew) is **TODO-690**. No `.rs`, program or §1/§2 text changed here.

### 3.9 Decision against the pre-registered table (AC-14)

`== DECISION ==` is the last section of `predicates.txt`. Checked by hand, in table order:
- **S1** does not match. Every STOP line is TRUE, and all four deciding lines (`PK=5`, `PC=TRUE`,
  `PL=TRUE`, `PH=TRUE`) are present and determinate.
- **T** matches. PC ∧ PL ∧ PH hold, `PK-premise=TRUE`, and `PA-wal_watermark_alarm_lines=0` is
  present.

Row T prescribes `PLATEAU=TRUE`, `READING=BOUNDED_STEADY`, `REPLICATE=NOT_NEEDED` and
`TG-OR-005=FLIP_TO_EVIDENCED`. The file prints exactly those four flags and `PREMISE=TRUE`.

### 3.10 Regeneration, ordering, and the AC roll-up

```
READOUT_REGEN=IDENTICAL
PREDICATES_REGEN=IDENTICAL
tmp_files_left=0
ORDER=OK
```

- **Regeneration (AC-13).** Both ran on `mktemp -d` copies of the D set with `cwd=/tmp`, never with
  `cwd` = evidence. The readout was regenerated from the CSV, the console log and `soak.durable.json`
  by the unchanged `spec365-readout.sh`. `spec370-predicates.sh` covered `fits.txt`, `predicates.txt`
  and all eight segments.
- **Ordering (AC-11).** `ORDER=OK` at D = `3ac48cd8…a424`, and again after this commit. The checks:
  - M = `024d1b4e` ≠ D, M is an ancestor of D, and D is an ancestor of HEAD.
  - The first commit of all ten `spec370-*` files at M is M itself: the seven programs, the
    calibration, the manifest and the runner.
  - M contains no artifact path.
  - `## APPEND-ONLY BELOW` occurs once at M.
  - The text through that marker is identical at M and HEAD
    (`e4d1067b95fc2d4eb099f0508cdfed81e262b12c05a34c4fb76ab9f9c30b74e4`).
- **AC-4.** `git diff --name-only 86656caa..HEAD -- '*.rs'` lists `main.rs`, `monitor.rs`,
  `report.rs` and `tests/soak_wal_census.rs`: 4 of 5.
- **AC-5.** The provenance-id grep over `.rs`/`.sh`/`.awk` finds 0 hits.
- **AC-15.** `ls spec370-plateau4h.scrapes | wc -l` = **241** = CSV data rows. D is the R-Artifacts
  set exactly, and this §3 is a separate, later commit.
- **AC-16.** `git diff 86656caa..D -- INVARIANTS.md` is empty. This commit does not touch
  `INVARIANTS.md` either; the flip is the next commit.
- **AC-17.** `soak.json` `tombstones` carries `kEffExpectedUnderO2: 4`, `heldEpochsMaxObserved: 2`,
  `neitherEpochsMaxObserved: 1` and `durableWatermarkLagMaxObserved: 3`. `PK-recorded` and
  `PK-premise` are present.
- **AC-19.** One tool call was made while the 4.5 h chain ran (a `date`, answering a question about
  remaining time).
- **AC-20.** Each premise row appears exactly once (§3.3).
- **Attestation-only**, because the subject is a one-time machine state: the clean target before the
  demos, and the absence checks after them (§3.1). The reproducible part holds independently: the
  demo 1 FATAL `sha256:` is the sha256 of `/usr/bin/true`.

### 3.11 Was the `TG-OR-005` flip eligible? (R-Artifacts (c))

**Yes.** The decision prints `TG-OR-005=FLIP_TO_EVIDENCED` on row T, and the flip condition holds in
full:
- `PLATEAU=TRUE`;
- `PK-premise=TRUE` (`held_max_observed=2 ≤ held_max_derived=3`, `durable_watermark_lag_max=3 ≤ 4`);
- a `PA-wal_watermark_alarm_lines=` line is present (value 0).

The PK-crosscheck FALSE (§3.8) is a recorded row. It cannot affect eligibility, and its error
direction cannot produce a false PASS. The flip is the next commit
(`docs(invariants): evidence TG-OR-005 from the 4 h level-ceiling cell`). It applies R-Invariant's
text, cites D as evidence and adds the S_A under-count sentence to the caveat. `NAKED_BASELINE` stays
4, because the entry's enforcing-test field stays NAKED (soak-evidenced, not CI-enforced).

### 3.12 Deferred

- **TODO-690.** The S_A-vs-server-stamp cross-check (§3.8): an exact per-window stamp count, or a
  skew-free proxy, before S_A feeds a gate again.
- **TODO-689.** Premise A is alarmed, not enforced: no write-behind sequence pending longer than
  `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS`.
- **TODO-634.** The crash-run ceiling (a recovery re-stamps every live tombstone into one epoch) is
  not derived. Also the memory residue: this cell's durable-layer reading is `PLATEAU_NOT_MET` on
  `rss_kib` (`MONOTONE_RISING`, envelope `BOTH`), and `phys_footprint` reached 8.24 GB with a
  last-half slope of +1,689 MB/h. That is outside this carve; resident tombstone bytes are ~35 KB.

### 3.13 Conductor adjudication (reference/SPEC-370-conductor-rulings-v4.md)

Quoted from the conductor session's local ruling file (not committed). The conductor verified D =
`3ac48cd8` (260 artifact files, 0 `.rs`/`.sh`/`.md`/`.awk`), M `024d1b4e` as an ancestor of D, and the
pre-marker manifest sha as equal at M and HEAD (`e4d1067b…`). The conductor then read
`spec370-plateau4h.predicates.txt` for the first time, after D:

1. **STOP predicates all TRUE.** PM-reconciled, PM-split, P5, P6 (523,000 = 523,000, gap 0), P7 (0),
   PS (241/241, O2), PV (both sha 64 hex, equal to the matrix), PR (241 rows, 0 crashes, class
   reading).
2. **P-K:** `stamps_window_max=2774`, `held_max_derived=3`, **K = 5**, C = 115,000 B. Recorded:
   `held_max_observed=2`, `durable_watermark_lag_max=3`, `neither_max=1`; `PK-premise=TRUE`;
   `PK-latency p99=23 ms, max=531 ms ≤ Δ=5000, unacked=0`.
3. **P-C:** run max 50,002 B at t=11,820 = 0.435 C, TRUE. **P-L:** last-half mean 34,691 B,
   last-quarter 34,730 B, deviation 0.110 % (38 B against a 3,469 B tolerance), TRUE. **P-H:**
   harness and cell agree (harness peak 49,841 B, both dispositions EVALUATED), TRUE. A7 is TRUE in
   all 8 windows (max 0.506). Theil–Sen last-half slope 388 B/h (recorded). WAL watermark alarm
   lines: 0.
4. **DECISION: `PLATEAU=TRUE`, `REPLICATE=NOT_NEEDED`, `TG-OR-005=FLIP_TO_EVIDENCED`.** The harness
   tombstone gate is `ok`, exit 0. The durable-layer reading `PLATEAU_NOT_MET` (rss_kib) is the
   memory residue: recorded, and outside this carve.
5. **Ruling 1 — the flip is AUTHORIZED.** Every pre-registered condition holds
   (`PLATEAU=TRUE` ∧ `PK-premise=TRUE`). `TG-OR-005` becomes evidenced, with D as its evidence and
   the statement per R-Invariant. `NAKED_BASELINE=4` stays, and `check-invariants.sh` must exit 0 on
   the flip commit.
6. **Ruling 2 — the cross-check discrepancy is DISCLOSED in §3 and tracked, not repaired.** Both
   proxies exceed the harness's 2,774 (3,342 and 3,000), "so S_A under-counts server stamps by
   8–20 %". The direction makes the gate stricter, and the run max clears both C(K=5)=115,000 and
   C(K=6)=138,000, "the flip therefore stands on the evidence". Disclosed in §3.8, with the C_csv
   row, the direction and the counter definitions, and deferred to TODO-690.
7. **Ruling 3 — G5 mechanics.** First this §3 commit, then the flip commit (`INVARIANTS.md` only),
   in that order, with `ORDER=OK` re-run after each.

**Executor's note on item 6 (the quote is left as written).** §3.8 finds that, by the counter
definitions, both proxies over-read in-row stamps by up to one epoch. On that reading, "S_A
under-counts by 8–20 %" is the upper estimate against the proxies, not a measured under-count. The
ruling's conclusion is unaffected: under either reading, the error direction tightens the ceiling.
The invariant's caveat sentence is scoped to "the server stamp proxy" and is true as written.

### 3.14 Addendum after review — the `PK-a7` row, and the Δ constant

Appended in the review-fix commit. §3.1–3.13 above are unchanged.

**`PK-a7 neither_max_observed=1 FALSE recorded_attributed_not_gated`.** §3.5 and §3.13 say "A7 is TRUE
in all eight windows". That sentence is about the window-ratio statistic (pass latency ÷ minimum
inter-exit interval). `PK-a7` is a different, server-side row, and §3 did not discuss it. It is the
maximum of `ret_epochs_neither`: closed epochs that were not held and were still indexed when a scrape
landed. That is the derivation's **Q** term ("eligible, queued behind a pass"), which the ceiling
assumes is 0 under the A7 premise.

Exactly one of the 241 rows has Q > 0:

```
elapsed_secs=11100 ret_epochs_neither=1 ret_refs_neither=1000 held(durability_only+both+claim_only)=0 tombstone_bytes=41170
```

At that instant, the whole retained set was at most O + Q + P ≤ 3 epochs, with H = 0 against the K = 5
the gate used. `tombstone_bytes` was 41,170 B = 0.358 C. So the one Q epoch sat in a held slot that
happened to be empty: the premise was momentarily not met, but the bound was not approached. The
row is recorded, and it attributes a reading without deciding one. It does not change the decision.
If Q > 0 recurred together with a full H term, the ceiling would under-provision by one epoch. That
belongs to the same premise family as A7 and is tracked with the ceiling derivation (TODO-634).

**Δ became a constant after the cross-vendor review.** `max_count_in_window`'s latency allowance used
to be `mem_sample_interval`. It is now the constant `DEFAULT_TOMBSTONE_STAMP_LATENCY_ALLOWANCE_SECS`
= 5 s, recorded as `stampLatencyAllowanceMs`, so a coarser sampling cadence can no longer widen the
counted window. The sample interval was 5 s in this cell, and it is 5 s in every pinned test
(`calibration_max_count_in_window`: 4,200 / 4,500 / 2,000). So `stamps_window_max=2774`, K = 5 and every
predicate line above are what the new code computes too. The cell is not re-run.
