# spec368-plateau4h — manifest (carve 8e: the 4 h plateau cell)

A measurement-only cell: it changes no `.rs` file, no gate, no invariant status. §1 and §2 below are
**pre-registered** — they are committed before any `spec368-plateau4h` artifact exists, and they are
never edited afterwards. §3 is appended, once the cell has run, below the `## APPEND-ONLY BELOW`
marker at the end of §2.

## §1 — Freeze and the closed difference list

### 1.1 The freeze

| field | value |
|---|---|
| freeze / pin / branch base | `d6a3d38f` |
| full sha | `d6a3d38fa238e3d729148742b96ca6a996e9d7ea` |
| subject | `Merge pull request #163 from TopGunBuild/spec-8c-prune-cancel-safe` |
| committed | `2026-09-13T13:09:44+03:00` |
| branch | `spec-8e-plateau4h` |

**The freeze IS the pin.** This cell changes zero `.rs` files, so the tree it measures and the tree it
is filed under are one tree; there is no separate pin commit to reconcile against a later freeze. The
runner asserts it before the build and before any clock: `SPEC368_CODE_FREEZE=d6a3d38f`, a placeholder
refusal, `git diff d6a3d38f..HEAD -- '*.rs'` EMPTY, and a clean `.rs` working tree. Three guards,
three distinct messages, none with an override.

### 1.2 Digests

`shasum -a 256`, computed from the files as they stand at this pre-registration commit:

| file | sha256 | role |
|---|---|---|
| `spec368-plateau4h.sh` | `a5c48031827fd38d73dd5f40b829913d95ae53c62cadbde2ad094c5b1ac85290` | the cell runner (this manifest's §1 enumerates its whole departure) |
| `spec365-readout.sh` | `5fbfa3e9d9df2a74830edbf89cf4036ae3e376add9171676fe70800b335d7a11` | FROZEN — invoked byte-unchanged by the runner's §13 |
| `spec349c2-fit.awk` | `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` | FROZEN — the OLS fitter |
| `spec366-conjuncts.sh` | `e1c283a069f66cc521ece02ae567126844e147a10b130b54ea586d2f2ce92fcb` | FROZEN — the parent the runner is a copy of |
| `spec366-p5.awk` | `2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33` | FROZEN — PM-settlement |
| `spec366-p67.awk` | `ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215` | FROZEN — PM-gap / PM-cancelled |
| `spec368-ps.awk` | `cb84ee55726bed5e7e120cec9eb6468b164df901130d0729de90e5200c3bf20a` | P-S rows |
| `spec368-pb.awk` | `27138d98dc5abf8eef3eba47ee0822a3d78c349871a2ca543ec20b8654c15724` | P-B decay / bound / level |
| `spec368-pf.awk` | `4a1a79c4f8152b2c8edf61913be6b0e89aedc2c45f55b043aa31cff7c933dab3` | P-F footprint (recorded) |
| `spec368-a7.awk` | `cde9e93f4179a3b741c6e787eaaf591ca1cf9ae133d8782e5dd1a69cb06d84bf` | PM-A7 |
| `spec368-predicates.sh` | `02c9e44b73b139ff432d561c20fb126995438e6f3f140465deaff5a788464f7f` | the orchestration: slicing → fits → predicates |

The five FROZEN digests equal the ones the pre-registration pinned in advance; a mismatch on any of
them is a STOP, not a note. The six `spec368-*` digests are **computed here from the files as
written** — none is copied from anywhere, so any ruled edit to a program before this commit is
absorbed by the computation rather than needing a digest correction.

### 1.3 The embedded diff

The runner is a copy of `spec366-conjuncts.sh` with a difference list CLOSED at six items. The literal
output of

```
cd packages/server-rust/benches/soak_harness/evidence && \
  diff -u spec366-conjuncts.sh spec368-plateau4h.sh | tail -n +3
```

(`tail -n +3` drops the two `---`/`+++` header lines, which carry file mtimes and would stop this
embedded copy from being re-checkable byte for byte; `diff` exits 1 when the files differ, which is
expected):
```diff
@@ -1,62 +1,59 @@
 #!/usr/bin/env bash
 #
-# Prune-conjunct readout cell runner -- the SPEC-366 successor of the frozen
-# spec365-conjuncts.sh, which is NOT edited.
+# Plateau readout cell runner -- a copy of spec366-conjuncts.sh, which is NOT
+# edited.
 #
-# A SUCCESSOR EXISTS BECAUSE THE FROZEN RUNNER CANNOT BE RUN FOR THIS SPEC.
-# spec365-conjuncts.sh refuses to start unless `git diff <freeze>..HEAD --
-# '*.rs'` is empty against SPEC365_CODE_FREEZE, and that guard has no override
-# while SPEC-366 changes .rs files by definition; it also hard-codes
-# BASE="spec365-conj900", whose artifacts are committed evidence a re-run must
-# not overwrite. This file is therefore a COPY, and the difference list against
-# spec365-conjuncts.sh is CLOSED at exactly five items:
+# A COPY EXISTS BECAUSE THE PARENT RUNNER CANNOT BE RUN FOR THIS CELL. Its
+# freeze literal names another commit, its single cell is 900s long, and its
+# artifact basename names files that are committed evidence a re-run must not
+# overwrite. This file is therefore a COPY, and the difference list against
+# spec366-conjuncts.sh is CLOSED at exactly six items:
 #
-#   1. THE FREEZE VARIABLE IS RENAMED SPEC366_CODE_FREEZE and its literal is
-#      SPEC-366's own freeze commit. The same three refusal guards -- the
-#      placeholder check, the .rs diff against the freeze commit, the dirty
-#      .rs working tree -- are otherwise unchanged, and none has an override.
-#   2. BASE="spec366-conj900". THE ENV OVERRIDE NAMES STAY VERBATIM
-#      (SPEC365_OUT_DIR, SPEC365_FORCE, SPEC365_DATA_DIR, SPEC365_SOAK_BIN,
-#      SPEC365_SMOKE_SAMPLE_INTERVAL): the readout below is the UNCHANGED
-#      spec365-readout.sh and resolves its own OUT_DIR from SPEC365_OUT_DIR, so
+#   1. THE DURATION IS 14400 SECONDS on the single cell line -- four hours,
+#      against the parent's 900s. Nothing else on that line changes apart
+#      from items 2 and 6.
+#   2. THE ARTIFACT BASENAME IS spec368-plateau4h, extended by the value of
+#      SPEC368_BASE_SUFFIX. With that variable unset -- the only supported
+#      state for a first sample -- the basename resolves to
+#      spec368-plateau4h. A replicate exports the suffix as -r2, the basename
+#      resolves to spec368-plateau4h-r2, and a second sample therefore needs
+#      NO edit to this file after the first sample's artifacts are committed.
+#      This runner does not validate the suffix; the chain that launches it
+#      does.
+#   3. THE FREEZE VARIABLE IS RENAMED SPEC368_CODE_FREEZE at every site that
+#      reads or names it, and its placeholder string is renamed with it. THE
+#      SPEC365_* ENV OVERRIDE NAMES STAY VERBATIM (they are listed in the
+#      inherited header below): the readout below is the UNCHANGED
+#      spec365-readout.sh and resolves its own OUT_DIR from one of them, so
 #      renaming them would point the runner and the readout at DIFFERENT
-#      directories whenever a scratch OUT_DIR is used.
-#   3. EVERY SCRAPE IS PERSISTED. scrape_prune_metrics additionally writes the
-#      raw /metrics body, VERBATIM, to ${OUT_DIR}/${BASE}.scrapes/<RFC3339
-#      UTC>.txt. No cadence change, no CSV change, no change to
-#      PRUNE_METRIC_NAMES: the same single response body is both parsed into
-#      the row and kept whole. A failed curl writes an EMPTY file, so a scrape
-#      that did not happen is a fail-closed input the manifest's P6/P7 can
-#      name, not a silently missing series. The directory JOINS the
-#      artifact-overwrite refusal, because that refusal enumerates named files
-#      and a directory beside them would otherwise let a second run MIX its
-#      scrapes with the first run's -- which corrupts the decision-scrape
-#      selection silently instead of failing loudly.
-#   4. this header and the usage text, naming the successor.
-#   5. A PRE-CLOCK PROVENANCE ASSERTION on the server binary, added after cell
-#      attempt 1 was invalidated. That attempt ran a server built from the PIN,
-#      not from this branch: an earlier experiment had shared one
-#      CARGO_TARGET_DIR between a pin worktree and the main checkout, cargo gave
-#      both source paths the same metadata hash, and `cargo build` then judged
-#      the pin-built binary fresh. The cell produced a full set of artifacts and
-#      a readout, and NOTHING in the run said the measured binary was the wrong
-#      one -- the predicates simply read as FALSE. So, after this runner's own
-#      build and BEFORE T0:
-#        (a) the binary must CONTAIN the string of a counter this branch adds,
-#            `topgun_or_prune_restored_cancelled_total`, else FATAL naming it;
-#        (b) its mtime must be >= this invocation's recorded start, else FATAL
-#            "stale artifact -- not built by this invocation";
-#        (c) its sha256, already on the matrix, is repeated as the FIRST line of
-#            the console log, so every artifact set carries the identity of the
-#            binary that produced it.
-#      A measurement that cannot say which binary it ran is not evidence.
+#      directories whenever a scratch out dir is used.
+#   4. THE FREEZE LITERAL IS THIS CELL'S OWN FREEZE COMMIT, d6a3d38f. The
+#      same three refusal guards -- the placeholder check, the .rs diff
+#      against the freeze commit, the dirty .rs working tree -- are otherwise
+#      unchanged, and none has an override. This cell measures a landed
+#      instrument and changes no .rs file, so the freeze commit IS its pin.
+#   5. this header and the usage text, naming this runner and its cell.
+#   6. THE CELL ID IS plateau4h, AND THE DEFAULT DATA DIR DERIVES FROM IT as
+#      target/spec368-<cell>-data with its sibling .meta dir, so a run of
+#      this file can neither land in nor collide with the parent cell's data
+#      dir. The matrix banner, the meta dir and the provenance-cell messages
+#      follow the cell id; the evidence artifact names follow the basename of
+#      item 2, not the cell id, and are unaffected by this item.
 #
-# The matrix, the cell literals, the harness flags, the log directive and the
-# readout invocation are byte-identical, and the readout is the UNCHANGED
-# spec365-readout.sh invoked with basename spec366-conj900. The departure is
-# enumerable with:
-#   diff spec365-conjuncts.sh spec366-conjuncts.sh
+# EVERYTHING ELSE IS BYTE-IDENTICAL to spec366-conjuncts.sh, and that
+# includes the matrix block, the cell line's remaining literals, the log
+# directive, the environment-discipline block, the pre-clock provenance
+# assertion, the fresh-data-dir guard, the artifact-overwrite refusal and its
+# scrapes-directory clause, scrape persistence, the CSV header, the harness
+# invocation, the post-run checks and fits, and the readout invocation. The
+# readout is the UNCHANGED spec365-readout.sh, invoked with this cell's own
+# basename. The departure is enumerable with:
+#   diff spec366-conjuncts.sh spec368-plateau4h.sh
 #
+# A DIFF HUNK THAT MAPS TO NONE OF THE SIX ITEMS IS A DEFECT, not a footnote:
+# the pre-registered manifest carries this diff and a hunk-to-item map, and a
+# runner whose departure cannot be enumerated cannot be filed under a freeze.
+#
 # Everything from here on is spec365-conjuncts.sh's own header, kept verbatim
 # so the lineage back to spec362b-durable.sh stays readable.
 #
@@ -104,7 +101,7 @@
 #       inherited six; the inherited six stay byte-identical in content and
 #       position.
 #   (g) THE FREEZE GATE REPLACES THE PIN. The parent's commit-pin literal
-#       becomes SPEC366_CODE_FREEZE here, with one added behaviour the parent
+#       becomes SPEC368_CODE_FREEZE here, with one added behaviour the parent
 #       never needed: while this literal still reads its own placeholder
 #       value (because the code it would pin does not exist yet), the runner
 #       refuses immediately, before any git call that would otherwise need to
@@ -125,7 +122,7 @@
 # enumerable with:
 #   diff spec362b-durable.sh spec365-conjuncts.sh
 #
-# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC366_CODE_FREEZE names the commit
+# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC368_CODE_FREEZE names the commit
 # at which the instrument this file samples was complete and the full gate
 # matrix green. The runner refuses to start if the literal ever reads its
 # placeholder again, or if any .rs file at HEAD differs from that commit, so
@@ -161,26 +158,27 @@
 
 usage() {
   cat >&2 <<'EOF'
-usage: spec366-conjuncts.sh <cell>
+usage: spec368-plateau4h.sh <cell>
 
-  A COPY of spec365-conjuncts.sh, which is not edited. The difference list
-  against that file is CLOSED, has exactly five items, and is enumerated in
+  A COPY of spec366-conjuncts.sh, which is not edited. The difference list
+  against that file is CLOSED, has exactly six items, and is enumerated in
   the header block above.
 
   This runner ships ONE cell:
-    conj900    900s, csv cadence 60s, crash-interval 0, live-census DISARMED,
-               log directive ARMED across three targets (removal, settlement,
-               conjunct), TOPGUN_PRUNE_RECORD armed, TOPGUN_EPOCH_WIDTH left
-               unset (production default 1000). Every other matrix literal is
-               the parent's 4-hour-cell literal, unchanged.
+    plateau4h  14400s (four hours), csv cadence 60s, crash-interval 0,
+               live-census DISARMED, log directive ARMED across three
+               targets (removal, settlement, conjunct), TOPGUN_PRUNE_RECORD
+               armed, TOPGUN_EPOCH_WIDTH left unset (production default
+               1000). Every other matrix literal is the parent's own
+               4-hour-cell literal, unchanged.
 
   Any other argument falls through to this text and exits 2.
 
   The run REFUSES TO START, before the build and before any clock, if
-  SPEC366_CODE_FREEZE still reads its own placeholder value, or unless the
-  .rs tree at HEAD is identical to the commit that literal names once it is
-  written, or unless the .rs working tree is clean. Three guards, three
-  distinct messages; none has an override.
+  SPEC368_CODE_FREEZE still reads its own placeholder value, or unless the
+  .rs tree at HEAD is identical to the commit that literal names, or unless
+  the .rs working tree is clean. Three guards, three distinct messages; none
+  has an override.
 
   A provenance cell REQUIRES SOAK_SERVER_BINARY to be exported and to name an
   existing executable. No cell in this runner's table is one; the guard is
@@ -202,10 +200,10 @@
 #   extra      -- extra harness flags
 #   base       -- artifact basename
 case "$CELL" in
-  conj900)    WIDTH="";  DURATION=900; SAMPLE_INTERVAL=60; PROVENANCE=no
+  plateau4h)  WIDTH="";  DURATION=14400; SAMPLE_INTERVAL=60; PROVENANCE=no
               CELL_CRASH_INTERVAL=0; ARM_LOG=yes
               CELL_LIVE_CENSUS=0
-              EXTRA_FLAGS=""; BASE="spec366-conj900" ;;
+              EXTRA_FLAGS=""; BASE="spec368-plateau4h${SPEC368_BASE_SUFFIX:-}" ;;
   *)          usage ;;
 esac
 
@@ -291,7 +289,7 @@
   PROV_BIN=""
 fi
 
-DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec365-${CELL}-data}"
+DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec368-${CELL}-data}"
 META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
 CONSOLE_LOG="${META_DIR}/harness-console.log"
 STOP_FILE="${META_DIR}/sampler.stop"
@@ -445,9 +443,9 @@
 #     yet, and the run refuses immediately rather than attempting to diff
 #     against a value that is not a revision.
 # ---------------------------------------------------------------------------
-SPEC366_CODE_FREEZE=3a009e42
-if [ "$SPEC366_CODE_FREEZE" = "PENDING_SPEC366_CODE_FREEZE" ]; then
-  echo "FATAL: SPEC366_CODE_FREEZE still reads its placeholder value." >&2
+SPEC368_CODE_FREEZE=d6a3d38f
+if [ "$SPEC368_CODE_FREEZE" = "PENDING_SPEC368_CODE_FREEZE" ]; then
+  echo "FATAL: SPEC368_CODE_FREEZE still reads its placeholder value." >&2
   echo "       The prune-conjunct instrument this runner samples is not yet" >&2
   echo "       committed under a named freeze commit. Refusing to start." >&2
   exit 2
@@ -458,15 +456,15 @@
   echo "       to the commit that produced it. Refusing to start." >&2
   exit 1
 fi
-if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC366_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
-  echo "FATAL: the .rs diff against the freeze commit ${SPEC366_CODE_FREEZE} could" >&2
+if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC368_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
+  echo "FATAL: the .rs diff against the freeze commit ${SPEC368_CODE_FREEZE} could" >&2
   echo "       not be computed, so the freeze cannot be asserted:" >&2
   printf '%s\n' "$FREEZE_RS_DIFF" >&2
   echo "       Refusing to start." >&2
   exit 1
 fi
 if [ -n "$FREEZE_RS_DIFF" ]; then
-  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC366_CODE_FREEZE}; this run would not be filed under it" >&2
+  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC368_CODE_FREEZE}; this run would not be filed under it" >&2
   printf '%s\n' "$FREEZE_RS_DIFF" >&2
   exit 1
 fi
@@ -708,7 +706,7 @@
   echo "  soak binary:    $SOAK_BIN"
   echo "  soak binary commit: ${SOAK_BIN_COMMIT}"
   echo "  .rs working tree:   ${RS_TREE_STATE}"
-  echo "  code freeze:            ${SPEC366_CODE_FREEZE}"
+  echo "  code freeze:            ${SPEC368_CODE_FREEZE}"
   echo "  code freeze diff (.rs): ${FREEZE_DIFF_STATE}"
   echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
   echo "  server binary:  $SERVER_BIN"
```

### 1.4 Hunk → item map

One row per `@@` hunk header in the embedded diff above. Every hunk maps to one item of the closed
list, or to a `+`-joined combination where several items share a hunk. **A hunk that mapped to none of
the six items would be a defect, not a footnote**; there is none.

| hunk | item(s) | what changed, and where it sits in the parent |
|---|---|---|
| `@@ -1,62 +1,59 @@` | **5** | the successor header block, parent `:2-59`, rewritten to describe this runner and this six-item list. The parent's `:14` naming of its own freeze variable disappears with the rewrite, which is why the freeze rename (item 3) has no separate site here. The preamble's closing sentence, parent `:60-62`, is unchanged and is this hunk's trailing context. |
| `@@ -104,7 +101,7 @@` | **3** | parent `:107`, inherited-header prose naming the freeze variable. |
| `@@ -125,7 +122,7 @@` | **3** | parent `:128`, inherited-header prose naming the freeze variable. |
| `@@ -161,26 +158,27 @@` | **3+5** | the usage heredoc, parent `:164-187`: the prose is rewritten to name this runner and the `plateau4h` cell at 14400 s (item 5), and the freeze variable at parent `:180` is renamed (item 3). |
| `@@ -202,10 +200,10 @@` | **1+2+6** | the one cell line, parent `:205-208`: `DURATION=900` → `DURATION=14400` (item 1), the artifact basename (item 2), and the case label `conj900)` → `plateau4h)` (item 6). `WIDTH=""`, `SAMPLE_INTERVAL=60`, `PROVENANCE=no`, `CELL_CRASH_INTERVAL=0`, `ARM_LOG=yes`, `CELL_LIVE_CENSUS=0` and `EXTRA_FLAGS=""` are untouched. |
| `@@ -291,7 +289,7 @@` | **6** | parent `:294`, the default data-dir literal's prefix; the `SPEC365_DATA_DIR` override name is untouched. |
| `@@ -445,9 +443,9 @@` | **3+4** | parent `:448-450`: the freeze literal becomes `d6a3d38f` (item 4) and the variable, its placeholder string and its FATAL message are renamed (item 3). |
| `@@ -458,15 +456,15 @@` | **3** | parent `:461`, `:462`, `:469`, the two remaining freeze guards' reads and messages. |
| `@@ -708,7 +706,7 @@` | **3** | parent `:711`, the matrix's `code freeze:` line. |

**9 hunks, six items: item 1 → 1, item 2 → 1, item 3 → 6, item 4 → 1, item 5 → 2, item 6 → 2.**
(The per-item counts sum above nine because three hunks carry more than one item.) Item 6 maps to the
`:205` label hunk and to the `:294` data-dir hunk, as required. Mechanically re-checkable at any time
with the `diff` above.

### 1.5 Byte-identity of the parts that must be identical

Each region was extracted from **both** files by an anchor pattern plus a fixed line count and the two
extracts diffed. All seven report IDENTICAL:

| region | anchor | lines | parent | successor | result |
|---|---|---|---|---|---|
| `CSV_HEADER` (the 41-column literal the frozen readout re-asserts) | `^CSV_HEADER=` | 1 | `:833` | `:831` | IDENTICAL |
| `PRUNE_METRIC_NAMES` (the 31-name scrape list) | `^PRUNE_METRIC_NAMES=` | 1 | `:839` | `:837` | IDENTICAL |
| the pinned matrix block, `CHURN_CLIENTS=6` … `JITTER_SEED=20260831` (so `SERVER_PORT=47356`, `CONFIRM_INTERVAL=2`, `STEADY_INTERVAL=300`, `WAL_FSYNC=batched` are carried) | `^CHURN_CLIENTS=6$` | 23 | `:402-424` | `:400-422` | IDENTICAL |
| the environment-discipline block, `if [ -n "$WIDTH" ]` … the inherited-`TOPGUN_WAL_FSYNC_POLICY` note | `^if \[ -n "\$WIDTH" \]; then$` | 50 | `:346-395` | `:344-393` | IDENTICAL |
| the provenance block, `SERVER_BIN_SHA256=` … `write_console_out() {…}` | `^SERVER_BIN_SHA256=` | 60 | `:519-578` | `:517-576` | IDENTICAL |
| the harness invocation, `"$SOAK_BIN" \` … `HARNESS_PID=$!` (every flag, `--durable-reading` and `--live-census-interval 0` included) | `^"\$SOAK_BIN" \\$` | 30 | `:763-792` | `:761-790` | IDENTICAL |
| the readout invocation, `READOUT_SCRIPT=` … `exit "$HARNESS_RC"` | `^READOUT_SCRIPT=` | 10 | `:1243-1252` | `:1241-1250` | IDENTICAL |

Everything else the pre-registration requires to be byte-identical is covered by the nine-hunk
enumeration above rather than by a separate extract: no hunk touches the log directive (parent `:359`),
the fresh-data-dir guard (`:609-620`), the artifact-overwrite refusal and its `.scrapes/` clause
(`:624-662`), the matrix banner line (`:694`, which renders `=== spec365 conjunct-readout run: cell
plateau4h ===`), scrape persistence (`:844-873`), `footprint_row` (`:887-918`), or the post-run checks
and fits (`:1099-1224`).

### 1.6 Consequences of keeping the list CLOSED at six (recorded, not fixed)

1. **The matrix banner still reads `spec365 conjunct-readout run`** — now `=== spec365
   conjunct-readout run: cell plateau4h ===`. The banner line itself is byte-unchanged; only the
   `${CELL}` it renders differs.
2. **Every env override keeps its `SPEC365_` name** (`SPEC365_DATA_DIR`, `SPEC365_OUT_DIR`,
   `SPEC365_FORCE`, `SPEC365_SOAK_BIN`, `SPEC365_SMOKE_SAMPLE_INTERVAL`), as does
   `SPEC362B_SMOKE_DURATION`. That is deliberate: the readout is the unchanged `spec365-readout.sh`
   and resolves its own `OUT_DIR` from `SPEC365_OUT_DIR`, so one renamed variable would silently split
   runner and readout across two directories under any scratch out dir.
3. **The inherited spec365 header, parent `:63-152`, is verbatim except the item-3 renames at `:107`
   and `:128`.** Inside that verbatim region two sentences are now false prose for this runner and
   stay unedited: `:76` ("A single label, conj900: 900s duration, 60s CSV cadence,") and `:88`
   ("runner's own conj900 cell is not built to carry, the smoke override"). This runner's label is
   `plateau4h` and its duration is 14400 s; the authority on both is the cell line itself, which is
   executed, and the header above it, which is rewritten.
4. **The runner does not validate the basename suffix.** `SPEC368_BASE_SUFFIX` is read, not checked;
   the chain that launches the runner is what refuses anything but the empty value or `-r2`.
5. **Item 6's effective names:** cell id `plateau4h`; data dir `target/spec368-plateau4h-data`; meta
   dir `target/spec368-plateau4h-data.meta` (holding `harness-console.log`, `sampler.stop`,
   `sampler.fail`). The stale `target/spec365-conj900-data{,.meta}` from the parent cell is a
   different path and is not touched. Evidence artifact names derive from the basename, not from the
   cell id, and are unaffected by this item.

## §2 — Pre-registered predicates

Everything in this section is fixed **before the data exists**. Commands are executed from the fenced
blocks only: the tables name, describe and point to blocks; `spec368-predicates.sh` and this section
are copied from those blocks, never from a table cell. Each predicate program's bytes are embedded
here **and** committed as a file beside this manifest, with its sha256 printed next to its block (the
same value as §1.2). The executability check that ran every one of them, before this commit and
against already-committed artifacts, is §2's last subsection.

Throughout: `EV=packages/server-rust/benches/soak_harness/evidence`, `BASE=spec368-plateau4h` (replicate:
`spec368-plateau4h-r2`), all
awk runs under `LC_ALL=C`, BSD awk compatible (no `mktime`, no `{n}` repetition).

**Missing-line rule (applies to every predicate below).** If `<BASE>.predicates.txt` lacks a `P5=`,
`P6=`, `P7=`, `A7=`, `PS-rows=` or `PS-verdict=` line, that predicate is **FALSE**. If it lacks a `PB=`
line, P-B is **INDETERMINATE**. Any of these is a **STOP** (FALSE branch).

**Commands are executed from the fenced blocks only.** Tables in this section name, describe and point
to blocks; the manifest §2 and `spec368-predicates.sh` are copied from the fenced blocks, never from a
table cell.

### One time domain

- CSV series are sliced by **row position** (below) and fitted against `elapsed_secs`.
- Console vs console comparisons (A7) use the **full-precision** console stamp
  `YYYY-MM-DDTHH:MM:SS.ffffffZ`, parsed without `mktime` into seconds via an integer Gregorian day index
  (`daynum()` from `spec366-p5.awk`) plus seconds of day plus the fraction.
- **T0 (wall clock)** is the stamp of the lexicographically first file in `<BASE>.scrapes/`, first 19
  characters. That file belongs to the CSV row at `elapsed_secs=0` (`spec366-conjuncts.sh:853`, and
  scrapes/rows are one-to-one). It lags server-ready by the time `ps`/`du` take inside `emit_row`,
  typically ≤ 2 s. [RULED R9: A7 T0 = the first scrape stamp]

### Slicing — 8 windows (lineage rule, unchanged)

"8 equal windows" means an **equal row-count** split over **all** CSV data rows, including rows whose
value cell is empty, exactly as `spec355-manifest.md:1185-1189`. `seg = int((n+7)/8)` and
`W_i = rows (i-1)*seg+1 .. min(i*seg, n)`, both ends inclusive, 1-based. With 241 rows, `seg = 31`, so
W1–W7 hold 31 rows each and **W8 holds 24** (the 13,020–14,400 s span in both references). The windows
are therefore equal in rows, not in seconds, and W8 is shorter. This is recorded, not corrected
[RULED R12: equal-row slicing 31 × 7 + 24, lineage].

```bash
( cd "$EV" && awk -F, 'NR==1{h=$0; next} {rows[++n]=$0}
  END{seg=int((n+7)/8);
      for(i=1;i<=8;i++){f=sprintf("spec368-plateau4h-seg%d.csv",i); print h > f;
        for(j=(i-1)*seg+1; j<=i*seg && j<=n; j++) print rows[j] > f; close(f)}}' spec368-plateau4h.csv )
```
(`spec368-predicates.sh` substitutes `<BASE>` for the prefix and the CSV name. Nothing else changes.
Each segment file is written with awk's `>`, which overwrites it.)

Fits, written to `<BASE>.fits.txt`, run with `cwd=$EV`. A non-zero fitter exit prints `FIT_ERROR rc=<n>`
in place of the fitter output, and every record is always terminated with exactly one newline:
```bash
fit_record() {
  out="$(awk -f spec349c2-fit.awk -v col="$1" -v window="$2" "$3")"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
for c in tombstone_bytes phys_footprint_mb reclaimable_mb; do
  for i in 1 2 3 4 5 6 7 8; do printf '%s W%s ' "$c" "$i"; fit_record "$c" full "spec368-plateau4h-seg$i.csv"; done
  printf '%s LH ' "$c"; fit_record "$c" last_half spec368-plateau4h.csv
done
```

### P-M — the mechanism holds at 4 h (TRUE iff every PM-* line below is TRUE)

| id | clause | expected line (TRUE rule) | command |
|---|---|---|---|
| PM-reconciled | every exited epoch RECONCILED | `PM-reconciled=TRUE`: the readout line reads exactly `reconciliation=RECONCILED` (`IN_FLIGHT` for the terminal epoch is not a split, `spec365-readout.sh:395-404`) | block PM-reconciled |
| PM-split | `split_epochs` empty | `PM-split=TRUE`: no `split_epochs=` token | block PM-split |
| PM-settlement | every settlement row `restored_cancelled=0`; every non-zero-return removal epoch settled except ≤ 1 terminal in-flight epoch within 15 s | `P5=TRUE` (unchanged program `spec366-p5.awk`) | block PM-settlement |
| PM-gap, PM-cancelled | `removed_refs_observed_total == considered_total` and `topgun_or_prune_restored_cancelled_total == 0`, both on the decision scrape (last scrape outside every open prune window) | `P6=TRUE … gap=0` **and** `P7=TRUE restored_cancelled_total=0`; `DECISION_SCRAPE=OPEN` ⇒ both FALSE (unchanged program `spec366-p67.awk`) | block PM-gap/PM-cancelled |
| PM-A7 | pass latency ÷ inter-exit interval < 1 in every 30-min window | `A7=TRUE` | block PM-A7 |

The commands are the fenced blocks below, one per predicate. **§2 of the manifest and
`spec368-predicates.sh` are copied from these blocks, never from the table.**

Block **PM-reconciled**:
```bash
awk '/^reconciliation=/ { seen=1; print ($0 ~ /^reconciliation=RECONCILED$/) ? "PM-reconciled=TRUE" : "PM-reconciled=FALSE reason=" $0 } END { if (!seen) print "PM-reconciled=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
```

Block **PM-split**:
```bash
awk '/^reconciliation=/ { seen=1; print (index($0,"split_epochs=")==0) ? "PM-split=TRUE" : "PM-split=FALSE reason=" substr($0, index($0,"split_epochs=")) } END { if (!seen) print "PM-split=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
```

Block **PM-settlement**:
```bash
awk -f "$EV/spec366-p5.awk" "$EV/$BASE.harness-console.log"
```

Block **PM-gap/PM-cancelled**:
```bash
awk -f "$EV/spec366-p67.awk" -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"
```

Block **PM-A7**:
```bash
awk -v t0="$(ls -1 "$EV/$BASE.scrapes" | grep -E '^[0-9-]+T[0-9:]+Z\.txt$' | sort | head -1 | cut -c1-19)" -f "$EV/spec368-a7.awk" "$EV/$BASE.harness-console.log"
```

The spec366 programs print `P5=`/`P6=`/`P7=` tokens. §3 maps them to PM-settlement / PM-gap /
PM-cancelled and does not rename them in the output.

**`restored_cancelled_total` and the gap are scrape series, not readout tokens.** The readout carries
only `restored_sum_total` and a *last CSV row* metric check (`spec365-readout.sh:416-417`), and that
check can read MISMATCH on a correct binary when the row is sampled mid-pass (`spec366-manifest.md:870-878`).
It is recorded in §3 and is **not** PM-gap's input.

#### `spec368-a7.awk` (pinned program)

A7 definitions [RULED R9, R10; ruling 2 per `SPEC-366.md:637-638`]:
- **Pass latency** of a non-zero-return `removal` row = (first `settlement` row for the same `epoch`
  strictly after it) − (the removal row), in seconds, console full precision. A removal with no such
  settlement is excluded from latency and listed in the `unsettled=` field of the
  `A7-removals=… unsettled=…` line (P5 decides whether that is admissible).
- **Inter-exit interval** = time between consecutive non-zero-return `removal` rows, in log order.
- **Window** of an event = `int((t − T0)/1800) + 1`, clamped to `[1, 8]`. A latency belongs to its
  removal row's window. An interval belongs to the window of its later removal row. Events at ≥ 14,400 s
  fall into W8.
- **Ratio_k** = max latency in W_k ÷ min interval in W_k. This is the conservative per-window form of
  SPEC-366's `max(latency) ÷ min(inter-exit)` (`spec366-manifest.md:897,1020`).
- **TRUE** iff every W1..W8 has ≥ 1 latency, ≥ 1 interval, min interval > 0 and Ratio_k < 1. Anything
  else is FALSE with named reasons (`empty_window_Wk`, `nonpositive_interval_Wk`, `ratio_ge_1_Wk`,
  `unparsable_console_row(n)`).
- **Per-epoch ratio series (RECORDED beside the window table, not gated; R10):** one `A7-epoch` line per
  non-zero-return removal, in log order: `epoch`, its `window`, `latency_s` (NA if unsettled),
  `next_interval_s` = time to the next non-zero-return removal (NA for the last one), and
  `ratio = latency_s ÷ next_interval_s` (NA if either is NA or the interval is ≤ 0). The verdict is
  decided by the per-window form above only.

```awk
# A7 -- per 30-minute window: max pass latency / min interval between consecutive removals.
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function daynum(t,   y, mo, d, a, yy, mm) {
    y = substr(t, 1, 4) + 0; mo = substr(t, 6, 2) + 0; d = substr(t, 9, 2) + 0
    a = int((14 - mo) / 12); yy = y + 4800 - a; mm = mo + 12 * a - 3
    return d + int((153 * mm + 2) / 5) + 365 * yy + int(yy / 4) - int(yy / 100) + int(yy / 400) - 32045
}
function secs(tok,   z, frac) {
    frac = 0
    if (substr(tok, 20, 1) == ".") { z = index(tok, "Z"); if (z > 21) frac = ("0" substr(tok, 20, z - 20)) + 0 }
    return (daynum(tok) - d0) * 86400 + substr(tok, 12, 2) * 3600 + substr(tok, 15, 2) * 60 + substr(tok, 18, 2) + frac
}
BEGIN {
    ESC = sprintf("%c", 27)
    if (t0 !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]$/) { badt0 = 1; exit 0 }
    d0 = daynum(t0); T0S = secs(t0)
}
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]/) next
    tok = line; sub(/ .*/, "", tok)
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/) { bad++; next }
        if (rr + 0 == 0) next
        nr++; r_ep[nr] = ep; r_t[nr] = secs(tok)
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch")
        if (ep !~ /^[0-9]+$/) { bad++; next }
        ns++; s_ep[ns] = ep; s_t[ns] = secs(tok)
        next
    }
}
END {
    if (badt0) { print "A7=FALSE reason=bad_t0(" t0 ")"; exit 0 }
    if (bad > 0) { print "A7=FALSE reason=unparsable_console_row(" bad ")"; exit 0 }
    unsettled = ""
    for (i = 1; i <= nr; i++) {
        w = int((r_t[i] - T0S) / 1800) + 1; if (w < 1) w = 1; if (w > 8) w = 8
        rw[i] = w; lat_ok[i] = 0
        best = -1
        for (j = 1; j <= ns; j++) if (s_ep[j] == r_ep[i] && s_t[j] > r_t[i] && (best < 0 || s_t[j] < best)) best = s_t[j]
        if (best < 0) unsettled = unsettled (unsettled == "" ? "" : ",") r_ep[i]
        else { L = best - r_t[i]; lat[i] = L; lat_ok[i] = 1; nl[w]++; if (nl[w] == 1 || L > maxl[w]) maxl[w] = L }
        if (i > 1) { I = r_t[i] - r_t[i - 1]; ni[w]++; if (ni[w] == 1 || I < mini[w]) mini[w] = I }
    }
    ok = 1; why = ""
    print "window | span_s | latencies | max_latency_s | intervals | min_interval_s | ratio"
    for (w = 1; w <= 8; w++) {
        if (nl[w] + 0 == 0 || ni[w] + 0 == 0) { ratio = "NA"; ok = 0; why = why " empty_window_W" w }
        else if (mini[w] <= 0) { ratio = "NA"; ok = 0; why = why " nonpositive_interval_W" w }
        else { r = maxl[w] / mini[w]; ratio = sprintf("%.3f", r); if (r >= 1) { ok = 0; why = why " ratio_ge_1_W" w } }
        printf("W%d | %d-%d | %d | %s | %d | %s | %s\n", w, (w - 1) * 1800, w * 1800, nl[w] + 0,
               (nl[w] + 0 ? sprintf("%.3f", maxl[w]) : "NA"), ni[w] + 0, (ni[w] + 0 ? sprintf("%.3f", mini[w]) : "NA"), ratio)
    }
    print "A7-removals=" nr + 0 " settlements=" ns + 0 " unsettled=" (unsettled == "" ? "none" : unsettled)
    for (i = 1; i <= nr; i++) {
        nx_ok = (i < nr); if (nx_ok) nx = r_t[i + 1] - r_t[i]
        er = (lat_ok[i] && nx_ok && nx > 0) ? sprintf("%.3f", lat[i] / nx) : "NA"
        printf("A7-epoch epoch=%s window=%d latency_s=%s next_interval_s=%s ratio=%s\n", r_ep[i], rw[i],
               (lat_ok[i] ? sprintf("%.3f", lat[i]) : "NA"), (nx_ok ? sprintf("%.3f", nx) : "NA"), er)
    }
    print (ok ? "A7=TRUE" : "A7=FALSE reason=" substr(why, 2))
}
```

sha256(`spec368-a7.awk`) = `cde9e93f4179a3b741c6e787eaaf591ca1cf9ae133d8782e5dd1a69cb06d84bf`

### P-S — steady state (replaces SPEC-366's mis-specified P3; TRUE iff PS-rows and PS-verdict are both TRUE)

- **PS-rows** [RULED R11]:
  - **First conjunct snapshot row** = the first CSV data row whose column 11 `conj_snapshots_total` is an
    unsigned integer ≥ 1 (the conjunct snapshot counter, `spec366-conjuncts.sh:830,833`). Rows **before**
    it are **exempt** (their conjunct cells are empty or zero by construction). No such row ⇒ FALSE
    `no_conjunct_snapshot`.
  - **Evaluable rows** = the first snapshot row and every data row after it. At least **95 %** of all
    data rows must be evaluable (`evaluable ≥ 0.95 × rows`), else FALSE `evaluable_below_95pct`.
  - On **every evaluable** row, all four cells `$15, $20, $21, $22` are unsigned integers and
    `ret_epochs_claim_only + ret_epochs_durability_only + ret_epochs_both ≤ durable_watermark_lag + 1`.
    An empty or non-integer cell in those columns on an evaluable row ⇒ FALSE
    `empty_or_non_integer_cell_after_first_snapshot`.
  - A header whose columns 11/15/20/21/22 are not those names is FALSE `header_mismatch`.
- **PS-verdict:** the readout's `READOUT:` verdict is `O2` or `O3`. `O1`, `O4`, `O5`, or no line ⇒ FALSE.
- **Recorded, not gated** (v6 R1's steady-state shape): from `readout.txt`, the SectionA `consistency:`
  line, `retained_closed_epochs`, and SectionD `durable_watermark_lag: max=… last=…`, plus whether
  `retained_closed_epochs ≤ last`.

`spec368-ps.awk`:
```awk
# P-S rows -- retained closed epochs never exceed the durable watermark lag plus the one open epoch.
BEGIN { FS = "," }
NR == 1 {
    if ($11 != "conj_snapshots_total" || $15 != "durable_watermark_lag" || $20 != "ret_epochs_claim_only" || $21 != "ret_epochs_durability_only" || $22 != "ret_epochs_both") { hdr = 1; exit 0 }
    next
}
/^[ \t\r]*$/ { next }
{
    rows++
    if (!started) {
        if ($11 ~ /^[0-9]+$/ && $11 + 0 >= 1) { started = 1; first = $1 }
        else { exempt++; next }
    }
    evaluable++
    if ($15 !~ /^[0-9]+$/ || $20 !~ /^[0-9]+$/ || $21 !~ /^[0-9]+$/ || $22 !~ /^[0-9]+$/) { nbad++; if (nbad <= 5) bl = bl " elapsed=" $1; next }
    s = $20 + $21 + $22
    ex = s - $15
    if (!seen || ex > maxex) { maxex = ex; seen = 1 }
    if (s > $15 + 1) { nv++; if (nv <= 5) vl = vl " elapsed=" $1 "(sum=" s ",lag=" $15 ")" }
}
END {
    if (hdr) { print "PS-rows=FALSE reason=header_mismatch"; exit 0 }
    if (rows + 0 == 0) { print "PS-rows=FALSE reason=no_data_rows"; exit 0 }
    if (!started) { print "PS-rows=FALSE reason=no_conjunct_snapshot rows=" rows; exit 0 }
    if (evaluable < 0.95 * rows) { print "PS-rows=FALSE reason=evaluable_below_95pct evaluable=" evaluable "/" rows " first_snapshot_elapsed=" first; exit 0 }
    if (nbad > 0) { print "PS-rows=FALSE reason=empty_or_non_integer_cell_after_first_snapshot rows=" nbad bl; exit 0 }
    if (nv > 0) { print "PS-rows=FALSE reason=bound_violated rows=" nv vl; exit 0 }
    print "PS-rows=TRUE rows=" rows " evaluable=" evaluable " exempt=" (exempt + 0) " first_snapshot_elapsed=" first " max(sum-lag)=" maxex
}
```

sha256(`spec368-ps.awk`) = `cb84ee55726bed5e7e120cec9eb6468b164df901130d0729de90e5200c3bf20a`
```bash
awk -f "$EV/spec368-ps.awk" "$EV/$BASE.csv"
awk '/^READOUT: / { seen=1; v=$2; sub(/;$/, "", v); print (v == "O2" || v == "O3") ? "PS-verdict=TRUE verdict=" v : "PS-verdict=FALSE reason=verdict_" v } END { if (!seen) print "PS-verdict=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"
```

### P-B — bytes: decay, bound, level (the headline reading)

**Conditions** (all over column `tombstone_bytes`):
- **C1 (decay observation — RECORDED, not a PLATEAU conjunct; RULED R7):** `slope(W8) < slope(W1)`. Each
  slope is the `slope_mb_per_hour` field (unit B/h) from
  `spec349c2-fit.awk -v col=tombstone_bytes -v window=full` on `<BASE>-seg8.csv` / `-seg1.csv`. It only
  separates DECAYING_NOT_BOUND from NOT_MET when the series is not PLATEAU.
- **C2 (bound):** last-half OLS slope ≤ **512** B/h. The slope is `slope_mb_per_hour` from
  `spec349c2-fit.awk -v col=tombstone_bytes -v window=last_half` on the **whole** `<BASE>.csv`, i.e.
  rows `[int(n/2) .. n-1]` of the n non-empty rows (`spec349c2-fit.awk:30-32,128`), the harness's own
  floor-biased split.
- **C3 (level):** over the same n non-empty `tombstone_bytes` values in CSV order (0-based data rows),
  last half `LH` = rows `[int(n/2) .. n-1]` and last quarter `LQ` = rows `[int(3n/4) .. n-1]`
  [RULED R8; W7∪W8 is NOT used]. `dev = |mean(LQ) − mean(LH)| / mean(LH)`.
  TRUE iff `mean(LH) > 0` and `dev ≤ 0.10`. `mean(LH) ≤ 0` ⇒ C3 FALSE `nonpositive_last_half_mean`.

**Decision table (ruling 2 as corrected by pre-audit R7 — flat-from-start is PLATEAU):**

| C2 (bound) | C3 (level) | C1 (`slope(W8) < slope(W1)`) | P-B |
|---|---|---|---|
| TRUE | TRUE | any (recorded as the decay observation) | **PLATEAU** |
| not both TRUE | | TRUE | **DECAYING_NOT_BOUND** |
| not both TRUE | | FALSE | **NOT_MET** |

I.e. PLATEAU ⇔ last-half OLS slope ≤ 512 B/h ∧ last-quarter mean within ±10 % of the last-half mean;
DECAYING_NOT_BOUND ⇔ ¬PLATEAU ∧ `slope(W8) < slope(W1)`; NOT_MET otherwise.

`PB=INDETERMINATE` is printed only when an input is missing: a slope that cannot be read, fewer than 4
non-empty rows, or a non-integer cell. It counts as a STOP (FALSE branch). It is not a verdict. The
row-count reason is printed as `too_few_rows(" n + 0 ")`, so a column from which **no** row parsed reads
`PB=INDETERMINATE reason=too_few_rows(0)` and never the count-free `too_few_rows()`.

**Proximity** [RULED R5: ±3 = percentage POINTS]: `near = (7.0 ≤ dev×100 ≤ 13.0)`, i.e. within ±3
percentage points of the 10 % threshold (printed as `PB-near_threshold`, recorded). **Level near-miss**
(the replicate trigger's proximity clause, R13): `near_miss = (C3 FALSE ∧ mean(LH) > 0 ∧ dev×100 ≤ 13.0)`,
i.e. `10.0 < dev% ≤ 13.0` — the level test missed by ≤ 3 pp (printed as `PB-level_near_miss`).

`spec368-pb.awk` (receives the three slopes via `-v`):
```awk
# P-B -- decay / bound / level decision over tombstone_bytes.
function isslope(s) { return s ~ /^[+-]?[0-9]+(\.[0-9]+)?$/ }
BEGIN { FS = "," }
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{
    v = $yc; gsub(/[ \t\r]/, "", v)
    if (v == "") { skipped++; next }
    if (v !~ /^[0-9]+$/) { nonint++; next }
    y[n++] = v + 0
}
END {
    if (hdr) { print "PB=INDETERMINATE reason=no_tombstone_bytes_column"; exit 0 }
    if (nonint > 0) { print "PB=INDETERMINATE reason=non_integer_cells(" nonint ")"; exit 0 }
    if (!isslope(s1) || !isslope(s8) || !isslope(slh)) { print "PB=INDETERMINATE reason=missing_slope(s1=" s1 ",s8=" s8 ",slh=" slh ")"; exit 0 }
    if (n < 4) { print "PB=INDETERMINATE reason=too_few_rows(" n + 0 ")"; exit 0 }
    h = int(n / 2); q = int(3 * n / 4)
    for (i = h; i < n; i++) sh += y[i]
    for (i = q; i < n; i++) sq += y[i]
    mh = sh / (n - h); mq = sq / (n - q)
    c1 = (s8 + 0 < s1 + 0); c2 = (slh + 0 <= 512)
    if (mh <= 0) { c3 = 0; dpct = "NA"; near = 0; miss = 0; c3why = " reason=nonpositive_last_half_mean" }
    else { dv = (mq - mh) / mh; if (dv < 0) dv = -dv; c3 = (dv <= 0.10); dpct = sprintf("%.3f", dv * 100); near = (dv * 100 >= 7.0 && dv * 100 <= 13.0); miss = (!c3 && dv * 100 <= 13.0); c3why = "" }
    verdict = (c2 && c3) ? "PLATEAU" : (c1 ? "DECAYING_NOT_BOUND" : "NOT_MET")
    printf("PB-C1 slope_W8=%s slope_W1=%s W8<W1=%s recorded_decay_observation\n", s8, s1, c1 ? "TRUE" : "FALSE")
    printf("PB-C2 last_half_slope=%s <=512=%s\n", slh, c2 ? "TRUE" : "FALSE")
    printf("PB-C3 n=%d skipped_empty=%d half_start=%d quarter_start=%d last_half_mean=%.3f last_quarter_mean=%.3f deviation_pct=%s <=10=%s%s\n", n, skipped + 0, h, q, mh, mq, dpct, c3 ? "TRUE" : "FALSE", c3why)
    printf("PB-ratio W8/W1=%s\n", (s1 + 0 != 0) ? sprintf("%.2f", (s8 + 0) / (s1 + 0)) : "NA")
    printf("PB-near_threshold=%s\n", near ? "YES" : "NO")
    printf("PB-level_near_miss=%s\n", miss ? "YES" : "NO")
    printf("PB=%s\n", verdict)
    printf("REPLICATE=%s\n", (verdict == "DECAYING_NOT_BOUND" || (verdict == "NOT_MET" && miss)) ? "AUTHORIZED" : "NOT_AUTHORIZED")
}
```

sha256(`spec368-pb.awk`) = `27138d98dc5abf8eef3eba47ee0822a3d78c349871a2ca543ec20b8654c15724`
```bash
slope() { awk -f "$EV/spec349c2-fit.awk" -v col=tombstone_bytes -v window="$2" "$1" | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { sub(/^slope_mb_per_hour=/, "", $i); print $i } }'; }
awk -v s1="$(slope "$EV/$BASE-seg1.csv" full)" -v s8="$(slope "$EV/$BASE-seg8.csv" full)" \
    -v slh="$(slope "$EV/$BASE.csv" last_half)" -f "$EV/spec368-pb.awk" "$EV/$BASE.csv"
```

§3 also records, beside P-B: the 8-window `tombstone_bytes` table (span, slope, se, r², `skipped_empty`)
next to both reference tables, the CSV peak and last non-empty `tombstone_bytes` with the last
`elapsed_secs`, and the in-process gauge `jq -c '.tombstones | {firstBytes,peakBytes,lastBytes,slopeBytesPerHour}' "$EV/$BASE.soak.json"`.
Channels are not combined, following `spec362-manifest.md:4114-4118`.

#### Reference numbers (placed beside P-B / census in §2 and §3; verified)

| reference | quantity | value | source |
|---|---|---|---|
| `spec355-w1000` | W1 … W8 `tombstone_bytes` slopes (B/h) | 113,657.12 / 244,197.34 / 151,075.57 / 166,532.06 / 118,189.21 / 132,688.50 / 133,598.47 / 155,726.40 | `spec355-manifest.md:1192-1201` |
| `spec355-w1000` | W8/W1 | **1.37×** (155,726.40 / 113,657.12 = 1.3701) | same |
| `spec355-w1000` | CSV peak / last `tombstone_bytes`, last `elapsed_secs` | **629,654 B / 629,654 B / 14400** | `spec362-manifest.md:3560` |
| `spec362b-long4h` | W1 … W8 slopes (B/h) | 95,653.60 / 56,417.64 / 171,702.41 / 154,358.56 / 212,434.31 / 224,270.03 / 484,239.22 / 651,649.15 | `spec362-manifest.md:4281-4288` |
| `spec362b-long4h` | W8/W1 | **6.81×** | `spec362-manifest.md:4253` |
| `spec362b-long4h` | CSV peak / last `tombstone_bytes` at 14,400 s | **1,014,337 B** (both) | `spec362-manifest.md:4233` |
| `spec362b-long4h` | terminal census entries / bytes / max per key | **44,452 / 1,016,398 B / 1,147** | `spec362-manifest.md:4073,4088-4093` |

### P-F — footprint (RECORDED, NOT GATED)

1. 8-window slope tables for `phys_footprint_mb` and `reclaimable_mb` (MB/h), plus the `LH` line, from
   `<BASE>.fits.txt`.
2. Reconstruction, over data rows with `elapsed_secs > 0`. The denominator is all such rows. A row with
   an empty `rss_mb`/`phys_footprint_mb`/`reclaimable_mb`, or `rss_mb = 0`, counts as not within.
   - **PF-recon2 (ruling-2 literal, TODO-664:103-104):** `|rss_mb − (phys_footprint_mb + reclaimable_mb)| / rss_mb ≤ 0.02`.
   - **PF-recon3 (readout SectionE identity, `spec365-readout.sh:576-590`):** the same with `+ clean_mb`.
     This is the identity SPEC-365 amended to after finding the two-term form false by construction
     (`.specflow/archive/SPEC-365.md:1282-1300`). [RULED: accepted deviation — the three-term
     reconstruction is recorded **beside** the two-term one.]
   - Each prints `MET` iff within/total ≥ 0.95, else `NOT_MET`. Neither is a predicate verdict.
3. Peak vs last (last non-empty) for `phys_footprint_mb` and `reclaimable_mb`, each with the boolean
   `peak_eq_last` (numeric equality of the CSV peak and the last non-empty value), and the last non-empty
   `phys_footprint_peak_mb`.
4. **Footprint shape** [RULED: accepted deviation — no `PEAKS`/`MONOTONE` tokens for footprint columns].
   For `phys_footprint_mb` and `reclaimable_mb` the shape is **defined** as the pair
   (`last_half_slope_sign`, `peak_eq_last`): `last_half_slope_sign` is `+`, `-` or `0` by the sign of the
   `slope_mb_per_hour` field on that column's `LH` line in `<BASE>.fits.txt` (the fitter's
   `window=last_half`, rows `[int(n/2) .. n-1]`, R8; `0` iff the printed value is exactly zero), and
   `peak_eq_last` comes from `spec368-pf.awk` (item 3). An `LH` record that reads `FIT_ERROR rc=<n>`
   carries no `slope_mb_per_hour=` field and is treated as `no_LH_line`. Pinned command:
   ```bash
   for c in phys_footprint_mb reclaimable_mb; do
     awk -v c="$c" '$1 == c && $2 == "LH" { for (i = 3; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { s = substr($i, 19); print "PF-shape " c " last_half_slope_sign=" ((s + 0 > 0) ? "+" : ((s + 0 < 0) ? "-" : "0")) " slope_mb_per_hour=" s; f = 1 } } END { if (!f) print "PF-shape " c " last_half_slope_sign=NA reason=no_LH_line" }' "$EV/$BASE.fits.txt"
   done
   ```
   The harness's own tokens are also recorded for its four deciding series only:
   `jq -c '.reading, .reason, (.decidingSeries[] | {name, shape, firingEnvelope, lastHalfMean})' "$EV/$BASE.soak.durable.json"`.
   **No committed tool emits a `PEAKS`/`MONOTONE_RISING` token for `phys_footprint_mb` or
   `reclaimable_mb`**, and §3 says so explicitly.

`spec368-pf.awk`:
```awk
# P-F -- footprint reconstruction (two identities) and peak/last, recorded only.
BEGIN { FS = "," }
NR == 1 {
    if ($1 != "elapsed_secs" || $2 != "rss_mb" || $7 != "phys_footprint_mb" || $8 != "phys_footprint_peak_mb" || $9 != "reclaimable_mb" || $41 != "clean_mb") { hdr = 1; exit 0 }
    next
}
/^[ \t\r]*$/ { next }
{
    if ($7 != "") { if (!pf_seen || $7 + 0 > pf_peak) { pf_peak = $7 + 0; pf_seen = 1 }; pf_last = $7 }
    if ($9 != "") { if (!rc_seen || $9 + 0 > rc_peak) { rc_peak = $9 + 0; rc_seen = 1 }; rc_last = $9 }
    if ($8 != "") pfp_last = $8
    if ($1 + 0 <= 0) next
    total++
    if ($2 == "" || $7 == "" || $9 == "" || $2 + 0 == 0) next
    d2 = $2 - ($7 + $9); if (d2 < 0) d2 = -d2
    if (d2 / $2 <= 0.02) w2++
    if ($41 != "") { d3 = $2 - ($7 + $41 + $9); if (d3 < 0) d3 = -d3; if (d3 / $2 <= 0.02) w3++ }
}
END {
    if (hdr) { print "PF=INSTRUMENT reason=header_mismatch"; exit 0 }
    printf("PF-recon2 rows_within_2pct=%d/%d %s\n", w2 + 0, total + 0, (total > 0 && (w2 + 0) >= 0.95 * total) ? "MET" : "NOT_MET")
    printf("PF-recon3 rows_within_2pct=%d/%d %s\n", w3 + 0, total + 0, (total > 0 && (w3 + 0) >= 0.95 * total) ? "MET" : "NOT_MET")
    printf("PF-phys_footprint_mb peak=%s last=%s peak_eq_last=%s\n", pf_seen ? sprintf("%.3f", pf_peak) : "NA", pf_last == "" ? "NA" : pf_last, pf_seen ? ((pf_peak == pf_last + 0) ? "TRUE" : "FALSE") : "NA")
    printf("PF-reclaimable_mb peak=%s last=%s peak_eq_last=%s\n", rc_seen ? sprintf("%.3f", rc_peak) : "NA", rc_last == "" ? "NA" : rc_last, rc_seen ? ((rc_peak == rc_last + 0) ? "TRUE" : "FALSE") : "NA")
    printf("PF-phys_footprint_peak_mb last=%s\n", pfp_last == "" ? "NA" : pfp_last)
}
```

sha256(`spec368-pf.awk`) = `4a1a79c4f8152b2c8edf61913be6b0e89aedc2c45f55b043aa31cff7c933dab3`
```bash
awk -f "$EV/spec368-pf.awk" "$EV/$BASE.csv"
```
(followed by the item-4 `PF-shape` loop and the `decidingSeries` `jq` line, in that order)

### Terminal durable census (RECORDED; redb scan at teardown by the harness's `--durable-reading`)

```bash
jq -c '.censusTerminal | {source, elapsedSecs, keysScanned, keysUndecodable, orMapKeys, tombstoneEntries, tombstoneBytes, tombstoneDupEntries, keysWithTombstones, keysAllDead, maxTombstonesPerKey}' "$EV/$BASE.soak.durable.json"
jq '.censuses | length' "$EV/$BASE.soak.durable.json"
jq -c '.tombstoneCorpus' "$EV/$BASE.soak.json"
```
§3 places `tombstoneEntries / tombstoneBytes / maxTombstonesPerKey` beside `spec362b-long4h`'s
**44,452 / 1,016,398 / 1,147**. The census is a single end-of-run structural read (crash-interval 0,
live census disarmed), not a series, and no predicate reads it (`spec362-manifest.md:4095-4099`).

### Harness attribution (RECORDED)

```bash
grep -E '^(RESULT:|RUNNER_EXIT=|csv rows:|harness exited with code)' "$EV/$BASE.runner-console.log"
jq -r '.passed, .finishedReason, .durationSecsActual' "$EV/$BASE.soak.json"
head -1 "$EV/$BASE.harness-console.log"
grep -E '^  +(sha256|code freeze|code freeze diff)' "$EV/$BASE.matrix.txt"
```
A non-zero harness exit caused by the tombstone slope gate is **attribution**, not a predicate and not a
repeat trigger (ruling 4; `spec366-conjuncts.sh:1232-1238`).

### `spec368-predicates.sh` (pinned program)

The committed file's bytes equal this block. Its sha256 is computed at G1b and placed next to the block
in the manifest, as for the awk programs. It is assembled from the blocks above, in R-Predicates-Script
order, with `<BASE>` substituted for the literal `spec368-plateau4h` prefix in the slicing and fits
blocks (list-item indentation removed). Beyond those blocks it adds only: argument validation, the two
pinned truncations, `export LC_ALL=C`, the `( cd "$EV" && … ) >> "$EV/$BASE.fits.txt"` wrapper that gives
the fits block its `cwd=$EV`, the `== <name> ==` headings with the `{ … } >> "$EV/$BASE.predicates.txt"`
group, and WHY-comments.

```bash
#!/usr/bin/env bash
# Runs the pre-registered slicing, fits and predicate blocks, in manifest order, for one cell prefix.
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
EV="${1:-}"; BASE="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then echo "usage: spec368-predicates.sh <EV_DIR> <BASE>" >&2; exit 2; fi
: > "$EV/$BASE.fits.txt"
: > "$EV/$BASE.predicates.txt"
export LC_ALL=C

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
  echo "== P-M =="
  awk '/^reconciliation=/ { seen=1; print ($0 ~ /^reconciliation=RECONCILED$/) ? "PM-reconciled=TRUE" : "PM-reconciled=FALSE reason=" $0 } END { if (!seen) print "PM-reconciled=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk '/^reconciliation=/ { seen=1; print (index($0,"split_epochs=")==0) ? "PM-split=TRUE" : "PM-split=FALSE reason=" substr($0, index($0,"split_epochs=")) } END { if (!seen) print "PM-split=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk -f "$EV/spec366-p5.awk" "$EV/$BASE.harness-console.log"
  awk -f "$EV/spec366-p67.awk" -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"
  awk -v t0="$(ls -1 "$EV/$BASE.scrapes" | grep -E '^[0-9-]+T[0-9:]+Z\.txt$' | sort | head -1 | cut -c1-19)" -f "$EV/spec368-a7.awk" "$EV/$BASE.harness-console.log"

  echo "== P-S =="
  awk -f "$EV/spec368-ps.awk" "$EV/$BASE.csv"
  awk '/^READOUT: / { seen=1; v=$2; sub(/;$/, "", v); print (v == "O2" || v == "O3") ? "PS-verdict=TRUE verdict=" v : "PS-verdict=FALSE reason=verdict_" v } END { if (!seen) print "PS-verdict=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"

  echo "== P-B =="
  slope() { awk -f "$EV/spec349c2-fit.awk" -v col=tombstone_bytes -v window="$2" "$1" | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { sub(/^slope_mb_per_hour=/, "", $i); print $i } }'; }
  awk -v s1="$(slope "$EV/$BASE-seg1.csv" full)" -v s8="$(slope "$EV/$BASE-seg8.csv" full)" \
      -v slh="$(slope "$EV/$BASE.csv" last_half)" -f "$EV/spec368-pb.awk" "$EV/$BASE.csv"

  echo "== P-F =="
  awk -f "$EV/spec368-pf.awk" "$EV/$BASE.csv"
  for c in phys_footprint_mb reclaimable_mb; do
    awk -v c="$c" '$1 == c && $2 == "LH" { for (i = 3; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { s = substr($i, 19); print "PF-shape " c " last_half_slope_sign=" ((s + 0 > 0) ? "+" : ((s + 0 < 0) ? "-" : "0")) " slope_mb_per_hour=" s; f = 1 } } END { if (!f) print "PF-shape " c " last_half_slope_sign=NA reason=no_LH_line" }' "$EV/$BASE.fits.txt"
  done
  jq -c '.reading, .reason, (.decidingSeries[] | {name, shape, firingEnvelope, lastHalfMean})' "$EV/$BASE.soak.durable.json"

  echo "== census =="
  jq -c '.censusTerminal | {source, elapsedSecs, keysScanned, keysUndecodable, orMapKeys, tombstoneEntries, tombstoneBytes, tombstoneDupEntries, keysWithTombstones, keysAllDead, maxTombstonesPerKey}' "$EV/$BASE.soak.durable.json"
  jq '.censuses | length' "$EV/$BASE.soak.durable.json"
  jq -c '.tombstoneCorpus' "$EV/$BASE.soak.json"

  echo "== harness attribution =="
  grep -E '^(RESULT:|RUNNER_EXIT=|csv rows:|harness exited with code)' "$EV/$BASE.runner-console.log"
  jq -r '.passed, .finishedReason, .durationSecsActual' "$EV/$BASE.soak.json"
  head -1 "$EV/$BASE.harness-console.log"
  grep -E '^  +(sha256|code freeze|code freeze diff)' "$EV/$BASE.matrix.txt"
} >> "$EV/$BASE.predicates.txt"
```

sha256(`spec368-predicates.sh`) = `02c9e44b73b139ff432d561c20fb126995438e6f3f140465deaff5a788464f7f`

### Replicate policy (ruling 3 as resolved by pre-audit R13, pre-registered)

1. n = 1.
2. **ONE replicate is authorized iff** the n=1 `predicates.txt` reads `REPLICATE=AUTHORIZED`, i.e.:
   a. `PB=DECAYING_NOT_BOUND` (TODO-677 ruling 3, first clause; not revoked by R13), **or**
   b. `PB=NOT_MET` **and** `PB-level_near_miss=YES` (the level test missed by ≤ 3 pp:
      `10.0 < dev% ≤ 13.0`; R13's ±3 pp clause, which applies only to a DECAYING_NOT_BOUND or NOT_MET
      reading — for DECAYING_NOT_BOUND it is already covered by (a)).
3. `PB=PLATEAU` **never** triggers a replicate, whatever `PB-near_threshold` reads; it is reported as-is.
   `PB=NOT_MET` with `PB-level_near_miss=NO` is reported as-is. No replicate.
4. Replicate mechanics [RULED R13]:
   a. The n=1 data commit and §3 (n=1) land first.
   b. **No runner edit.** The runner's `BASE` literal already reads
      `BASE="spec368-plateau4h${SPEC368_BASE_SUFFIX:-}"` (R-Runner item 2). §3 records that
      `git diff <n=1 data commit> HEAD -- packages/server-rust/benches/soak_harness/evidence/spec368-plateau4h.sh`
      is empty and that the runner sha256 equals the §1 digest.
   c. R-Provenance steps 1 and 5 are repeated (clean target; `rm -rf target/spec368-plateau4h-data
      target/spec368-plateau4h-data.meta`, because the data dir derives from the cell id, not from
      `BASE`, and the n=1 run left it populated), but **not** the FATAL demo.
   d. The same `chain-spec368.sh` text is launched once with `SPEC368_BASE_SUFFIX=-r2` in its
      environment, so `BASE=spec368-plateau4h-r2` for the runner, the runner-console name and the
      predicates argument.
   e. The r2 artifacts (`spec368-plateau4h-r2.*`, `spec368-plateau4h-r2-seg{1..8}.csv`,
      `spec368-plateau4h-r2.scrapes/`) get their own data commit and a §3 "Replicate r2" block.
5. **At most one replicate in total. No third run** without a conductor ruling. No pooling or
   combination rule between n=1 and r2 is pre-registered. Both readings are reported side by side for
   the conductor, and r2's own `REPLICATE=` line authorizes nothing.

### FALSE branch / STOP (pre-registered)

If any of P-M, P-S is FALSE, or P-B is `INDETERMINATE`, or the runner class is ABORTED or exit-9: the
executor **STOPS**, commits nothing beyond what the class allows (an exit-9 or ABORTED set is not
committed as evidence), and reports the literal lines and their transports. **No re-run, no re-tuning of
any bound, no widening of any carve-out, no edit to any program or to §1/§2.** P-B = `NOT_MET` or
`DECAYING_NOT_BOUND` is a **reading**, not a FALSE. It is recorded and reported, and the replicate
policy applies. P-F and the census are never STOP triggers.

On this branch, one regeneration result is expected rather than alarming: if `spec366-p67.awk` printed
`no_persisted_scrape_in <path>`, the predicates regeneration reads
`PREDICATES_REGEN_DIFFERS=<BASE>.predicates.txt`, because that reason line embeds `$EV` (relative in the
chain, `mktemp` in the regeneration). §3 records the DIFFERS line with that cause; see *Readout
regeneration*. It changes nothing about the STOP: the same condition is already P-M FALSE.

### Readout regeneration (normative check)

```bash
EV=packages/server-rust/benches/soak_harness/evidence; BASE=spec368-plateau4h
S="$(mktemp -d)"
cp "$EV/$BASE.csv" "$EV/$BASE.harness-console.log" "$EV/$BASE.soak.durable.json" "$S/"
SPEC365_OUT_DIR="$S" bash "$EV/spec365-readout.sh" "$BASE" 2>/dev/null
cmp "$S/$BASE.readout.txt" "$EV/$BASE.readout.txt" && echo READOUT_REGEN=IDENTICAL
```
The check compares the written **file**, not stdout. The same procedure applies to the predicates:
copy `$BASE.*`, the `.scrapes/` directory and every program into a scratch dir, then run
`bash spec368-predicates.sh "$S" "$BASE"`. The regenerated `$BASE.fits.txt`, `$BASE.predicates.txt`
and `$BASE-seg{1..8}.csv` must `cmp` identical to the committed ones. The copied `$BASE.fits.txt` and
`$BASE.predicates.txt` are truncated by the script's first two statements (R-Predicates-Script), so
the copy set needs no exclusion. Predicates block:

```bash
EV=packages/server-rust/benches/soak_harness/evidence; BASE=spec368-plateau4h
S="$(mktemp -d)"
for f in "$EV/$BASE".*; do [ -f "$f" ] && cp "$f" "$S/"; done
cp -R "$EV/$BASE.scrapes" "$S/"
cp "$EV/spec349c2-fit.awk" "$EV/spec366-p5.awk" "$EV/spec366-p67.awk" "$EV/spec368-ps.awk" "$EV/spec368-pb.awk" "$EV/spec368-pf.awk" "$EV/spec368-a7.awk" "$EV/spec368-predicates.sh" "$S/"
bash "$S/spec368-predicates.sh" "$S" "$BASE" 2>/dev/null
ok=1
for f in "$BASE.fits.txt" "$BASE.predicates.txt" "$BASE-seg1.csv" "$BASE-seg2.csv" "$BASE-seg3.csv" "$BASE-seg4.csv" "$BASE-seg5.csv" "$BASE-seg6.csv" "$BASE-seg7.csv" "$BASE-seg8.csv"; do
  cmp -s "$S/$f" "$EV/$f" || { ok=0; echo "PREDICATES_REGEN_DIFFERS=$f"; }
done
[ "$ok" = 1 ] && echo PREDICATES_REGEN=IDENTICAL
```

**The one case where `PREDICATES_REGEN` cannot read IDENTICAL (FALSE branch only; RULED rulings v2
rec 1).** `spec366-p67.awk` prints its no-scrape reason with the directory **path** in it —
`P6=FALSE reason=no_persisted_scrape_in <path>` — and that path is built from `$EV`: relative
(`packages/server-rust/benches/soak_harness/evidence/<BASE>.scrapes`) in the chain, and the `mktemp -d`
scratch directory in the regeneration. So whenever p67 takes that FALSE branch, the regenerated
`predicates.txt` differs from the committed one **by that path alone**, and the block prints
`PREDICATES_REGEN_DIFFERS=<BASE>.predicates.txt` instead of `PREDICATES_REGEN=IDENTICAL`. This is the
**only** admissible non-IDENTICAL result. It is not a new finding and not an AC-16 failure of its own:
the same condition already forces `P6=FALSE` and `P7=FALSE` ⇒ P-M FALSE ⇒ STOP, and fails AC-14
(`scrapes == CSV rows`). §3 records the `PREDICATES_REGEN_DIFFERS=<file>` line **together with that
cause** (the p67 `no_persisted_scrape_in` line quoted beside it). A `PREDICATES_REGEN_DIFFERS` on any
other file, or on `predicates.txt` **without** a p67 `no_persisted_scrape_in` line, is a defect, not this
case.

### R-Exec — executability check (run before this commit, on committed artifacts only)

Every program above was run in a scratch directory — never in `evidence/` — against artifacts that
were already committed, so the check costs this cell no measurement and can be repeated at any time.
BSD awk, bash 3.2, `LC_ALL=C`. Observed lines, literal:

| # | input | what ran | observed |
|---|---|---|---|
| 1 | `spec366-conj900.csv` | `spec368-ps.awk` | `PS-rows=TRUE rows=16 evaluable=16 exempt=0 first_snapshot_elapsed=0 max(sum-lag)=-1` |
| 2 | `spec366-conj900.csv` | `spec368-pf.awk` | `PF-recon2 rows_within_2pct=8/15 NOT_MET` / `PF-recon3 rows_within_2pct=15/15 MET` (recon3 matches the readout's own SectionE, `spec366-conj900.readout.txt:82`) / `PF-phys_footprint_mb peak=133.251 last=90.735 peak_eq_last=FALSE` / `PF-reclaimable_mb peak=736.125 last=736.125 peak_eq_last=TRUE` / `PF-phys_footprint_peak_mb last=139.079` |
| 3 | `spec366-conj900.harness-console.log`, `t0` = first scrape stamp `2026-09-13T08:46:33` | `spec368-a7.awk` | one window table, 32 `A7-epoch` lines, `A7-removals=32 settlements=32 unsettled=none`, and the W1 row / verdict below |
| 4 | the same, via **block PM-A7 copied from its fenced block** (`EV` = a scratch copy holding the console log, `spec366-conj900.scrapes/` and `spec368-a7.awk`; `BASE=spec366-conj900`) | the block | `W1 \| 0-1800 \| 32 \| 6.132 \| 31 \| 17.570 \| 0.349` and `A7=FALSE reason=empty_window_W2 empty_window_W3 empty_window_W4 empty_window_W5 empty_window_W6 empty_window_W7 empty_window_W8` |
| 5 | a scratch copy of every `spec366-conj900.*`, its `.scrapes/` and every program | `spec368-predicates.sh "$S1" spec366-conj900`, then the *Readout regeneration* predicates block against that first run | `PREDICATES_REGEN=IDENTICAL` |
| 6 | a copy of `spec362b-long4h.csv` | the slicing block with that prefix | 8/8 segments `cmp` IDENTICAL against the committed `spec362b-long4h-seg{1..8}.csv` |
| 7 | those segments | the fitter, `col=tombstone_bytes window=full` | `95653.596774 / 56417.641284 / 171702.411290 / 154358.564516 / 212434.306452 / 224270.032258 / 484239.217742 / 651649.151631` — the reference W1…W8 slopes reproduced |
| 8 | `spec362b-long4h` | `spec368-pb.awk` | `PB-C1 slope_W8=651649.151631 slope_W1=95653.596774 W8<W1=FALSE recorded_decay_observation` / `PB-C2 last_half_slope=335132.182145 <=512=FALSE` / `PB=NOT_MET` / `PB-ratio W8/W1=6.81` |
| 9 | the committed `spec355-w1000-seg{1,8}.csv` and `spec355-w1000.csv` | `spec368-pb.awk` | `PB-C3 … deviation_pct=13.142 <=10=FALSE` / `PB=NOT_MET` / `PB-ratio W8/W1=1.37` / `PB-near_threshold=NO` |
| 10 | `spec366-conj900.{csv,harness-console.log,soak.durable.json}` in a scratch out dir | the *Readout regeneration* block | `READOUT_REGEN=IDENTICAL` |
| 11 | a conj900 copy with column 9 (`reclaimable_mb`) blanked | `spec368-predicates.sh` | nine `reclaimable_mb … FIT_ERROR rc=2` records (W1…W8 and LH), the file still ending in exactly one newline, and `PF-shape reclaimable_mb last_half_slope_sign=NA reason=no_LH_line`; the `phys_footprint_mb` shape line is unaffected |

**Note (expected, not an alarm).** The W1 ratio 0.349 in rows 3–4 is computed from full-precision
console stamps; the 0.333 in the parent cell's record came from whole-second stamps. Nothing changed
in the mechanism between the two readings.

**A7 on a 900 s input is FALSE by construction, and that is the point.** 900 s < 1800 s, so W1 is the
only non-empty window and W2…W8 fail closed with `empty_window_Wk`. The check proves the empty-window
path names its reason rather than passing vacuously; it says nothing about the fix.

#### Fail-closed inputs (each must print its named reason)

```
P-B, empty tombstone_bytes column, three valid slopes via -v : PB=INDETERMINATE reason=too_few_rows(0)
P-B, the same input with the slopes absent                   : PB=INDETERMINATE reason=missing_slope(s1=,s8=,slh=)
P-B, one non-integer cell                                    : PB=INDETERMINATE reason=non_integer_cells(1)
P-B, a header without a tombstone_bytes column               : PB=INDETERMINATE reason=no_tombstone_bytes_column
```

The zero-row case prints the count, `too_few_rows(0)`, never a count-free `too_few_rows()`. With the
slopes absent or unparsable the same input reads `missing_slope(…)` instead, because that check
precedes the row-count check in the program.

#### Decision-table inputs (one per row of the P-B table)

```
flat from the start (s1=0 s8=10 slh=0, constant positive column):
  PB-C1 slope_W8=10 slope_W1=0 W8<W1=FALSE recorded_decay_observation
  PB-C2 last_half_slope=0 <=512=TRUE
  PB-C3 n=8 skipped_empty=0 half_start=4 quarter_start=6 last_half_mean=1000.000 last_quarter_mean=1000.000 deviation_pct=0.000 <=10=TRUE
  PB=PLATEAU
  REPLICATE=NOT_AUTHORIZED
decaying but unbound (s1=9000 s8=4000 slh=2000):
  PB-C1 slope_W8=4000 slope_W1=9000 W8<W1=TRUE recorded_decay_observation
  PB-C2 last_half_slope=2000 <=512=FALSE
  PB=DECAYING_NOT_BOUND
  REPLICATE=AUTHORIZED
neither (s1=100 s8=900 slh=2000, level off by more than 13 pp):
  PB-C3 n=8 … deviation_pct=16.667 <=10=FALSE
  PB=NOT_MET
  PB-level_near_miss=NO
  REPLICATE=NOT_AUTHORIZED
bound, level missed by <= 3 pp, s8 >= s1 (s1=100 s8=900 slh=10):
  PB-C2 last_half_slope=10 <=512=TRUE
  PB-C3 n=8 … deviation_pct=11.111 <=10=FALSE
  PB-near_threshold=YES
  PB-level_near_miss=YES
  PB=NOT_MET
  REPLICATE=AUTHORIZED
```

#### P-S inputs

```
5 leading exempt rows of 241  : PS-rows=TRUE rows=241 evaluable=236 exempt=5 first_snapshot_elapsed=300 max(sum-lag)=-1
an empty col-15 cell after the first snapshot : PS-rows=FALSE reason=empty_or_non_integer_cell_after_first_snapshot rows=1 elapsed=2880
20 exempt rows of 241 (> 5 %) : PS-rows=FALSE reason=evaluable_below_95pct evaluable=221/241 first_snapshot_elapsed=1200
no conjunct snapshot row      : PS-rows=FALSE reason=no_conjunct_snapshot rows=241
a header with other names     : PS-rows=FALSE reason=header_mismatch
a header and no data rows     : PS-rows=FALSE reason=no_data_rows
```

#### The first predicates run, in full (scratch copy of the parent cell, `BASE=spec366-conj900`)

```
== P-M ==
PM-reconciled=TRUE
PM-split=TRUE
P5=TRUE
P5-observed: removal_rows=32 settlement_rows=32 unsettled=none
P5-zero-return: none observed
DECISION_SCRAPE=2026-09-13T09:01:33Z.txt
P6=TRUE removed_refs_observed_total=32000 considered_total=32000 gap=0
P7=TRUE restored_cancelled_total=0
windows=32 settlement_rows=32 zero_return_removal_rows=0 scrapes=16
W1 | 0-1800 | 32 | 6.132 | 31 | 17.570 | 0.349
A7-removals=32 settlements=32 unsettled=none
A7=FALSE reason=empty_window_W2 empty_window_W3 empty_window_W4 empty_window_W5 empty_window_W6 empty_window_W7 empty_window_W8
== P-S ==
PS-rows=TRUE rows=16 evaluable=16 exempt=0 first_snapshot_elapsed=0 max(sum-lag)=-1
PS-verdict=TRUE verdict=O2
== P-B ==
PB-C1 slope_W8=120120.000000 slope_W1=1678140.000000 W8<W1=TRUE recorded_decay_observation
PB-C2 last_half_slope=-141837.142857 <=512=TRUE
PB-C3 n=16 skipped_empty=0 half_start=8 quarter_start=12 last_half_mean=34149.500 last_quarter_mean=29210.500 deviation_pct=14.463 <=10=FALSE
PB-ratio W8/W1=0.07
PB-near_threshold=NO
PB-level_near_miss=NO
PB=DECAYING_NOT_BOUND
REPLICATE=AUTHORIZED
== P-F ==
PF-recon2 rows_within_2pct=8/15 NOT_MET
PF-recon3 rows_within_2pct=15/15 MET
PF-phys_footprint_mb peak=133.251 last=90.735 peak_eq_last=FALSE
PF-reclaimable_mb peak=736.125 last=736.125 peak_eq_last=TRUE
PF-phys_footprint_peak_mb last=139.079
PF-shape phys_footprint_mb last_half_slope_sign=+ slope_mb_per_hour=0.236429
PF-shape reclaimable_mb last_half_slope_sign=+ slope_mb_per_hour=4254.399286
```

(The `A7-epoch` series and the rest of the window table are in that run's own `predicates.txt`; they
are not reproduced here. The P-B reading above belongs to a **900 s** cell and is an executability
observation, not a reading about the 4 h question — a 16-row column is far below the shape this
manifest's P-B is pre-registered to read.)

## APPEND-ONLY BELOW

## §3 — the 4 h plateau cell (n=1): reading, regeneration, adjudication

Appended 2026-09-17, after the data commit `347ce2a17e977e537865868b85d048f74987ece4` (artifacts) and
before nothing else: this append is the second and last commit of the cell. Nothing above
`## APPEND-ONLY BELOW` changed. The R-Artifacts set spans the data commit **and** this append.

**Outcome in one line: STOP — P-M FALSE on PM-A7 (W3 ratio 1.047).** The pre-registered FALSE branch
governs: no re-run, no re-tuning of any bound, no widening of any carve-out, no edit to any program or to
§1/§2. The `REPLICATE=AUTHORIZED` line printed by `spec368-pb.awk` is **void** under that rule (§3.6).

### 3.1 Provenance, clean target, and the foreign-binary demo

`cargo clean -p topgun-server --release` removed 27 files / 126.4 MiB; `rm -f target/release/topgun-server`
then left `test ! -e target/release/topgun-server` true (AC-9). The stale 2026-09-13 binary inherited by
the pre-registration commit was removed by that step, not reused.

The clause-(a) demo ran exactly once, with `CARGO_TARGET_DIR` on a scratch directory holding a copy of
`/usr/bin/true` as `release/topgun-server` and `SPEC365_SOAK_BIN=/usr/bin/true`, so no build entered
`target/` and no artifact was written. `DEMO_EXIT=1`, and the output was:

```
WARNING: SPEC365_SOAK_BIN is set, so this runner did NOT build the bench
         binary from HEAD. The freeze gate is NOT discharged for this run
         and matrix.txt will say so.
FATAL: the server binary does not contain 'topgun_or_prune_restored_cancelled_total'.
       binary: /var/folders/dy/35x7phnx3pz88gmkjf9560sm0000gn/T/tmp.V81XQITFsw/release/topgun-server
       built:  2026-09-16T14:58:19Z
       sha256: 875c7eea9c66c826091ede3cc44599311dc6818caf17f9e53d97c54c288842b2
       This counter is emitted by the branch under test, so a binary
       without it was built from other sources. Attempt 1 ran exactly
       such a binary and the cell was worthless.
```

The FATAL `sha256:` equals `shasum -a 256` of the planted binary. Afterwards
`target/spec368-plateau4h-data` and `target/spec368-plateau4h-data.meta` did not exist, the evidence
directory had no untracked or modified file, and `target/release/topgun-server` did not exist (AC-10).

The measured binary was built by the runner's own invocation (`Finished release profile [optimized] in
3m 51s`), and its identity travels with the artifacts:

```
provenance: server sha256=1f506548a5126bc34216f4a3f7f9ee1ab8d57995573f3075c4bf121764c89ab3 built=2026-09-16T15:02:43Z run_start=2026-09-16T14:58:51Z topgun_or_prune_restored_cancelled_total=present
  code freeze:            d6a3d38f
  code freeze diff (.rs): EMPTY (asserted before the build)
    sha256:       1f506548a5126bc34216f4a3f7f9ee1ab8d57995573f3075c4bf121764c89ab3
```

Console line 1 and the matrix `sha256:` are the same hash, and `built` is 3 m 52 s after `run_start`, so
clause (b) held on a binary this invocation produced. The counter's presence discharges `PROV_HITS ≥ 1`
without a separate reading: clause (a) did not fire.

### 3.2 Runner attribution

```
harness exited with code 1
csv rows: 241
RESULT: instrument sound; harness exit code 1.
RUNNER_EXIT=1
false
tombstone-byte growth slope 677.7 bytes/h exceeds 512.0 bytes/h: tombstone-byte growth slope 677.7 bytes/h exceeds 512.0 bytes/h (total growth 60984 bytes over 2875 samples, last-half window 7197s)
14402
```

Class = **reading + gate attribution** (R-Chain 4, row 4): the instrument is sound and the non-zero exit
is the harness's own tombstone-slope gate firing. `passed=false`, that message as `finishedReason`,
`durationSecsActual=14402`. Recorded as attribution, never as grounds for a repeat.

### 3.3 Readout

```
READOUT: O2; retained_closed_epochs=1; reconciliation=RECONCILED
SectionA: source=matched seq=3116 conj_snapshots_total=3116
consistency: counted(claim_only=0 durability_only=1 both=0 neither=0) line(claim_only=0 durability_only=1 both=0 neither=0) retained_truncated=false OK
durable_watermark_lag: max=3 last=2
```

### 3.4 P-M — FALSE

```
PM-reconciled=TRUE
PM-split=TRUE
P5=TRUE
P5-observed: removal_rows=527 settlement_rows=527 unsettled=none
P5-zero-return: none observed
DECISION_SCRAPE=2026-09-16T19:02:44Z.txt
P6=TRUE removed_refs_observed_total=527000 considered_total=527000 gap=0
P7=TRUE restored_cancelled_total=0
windows=527 settlement_rows=527 zero_return_removal_rows=0 scrapes=241
A7=FALSE reason=ratio_ge_1_W3
A7-removals=527 settlements=527 unsettled=none
```

P-M is TRUE only if every `PM-*` row is TRUE; `A7=FALSE` is one of those rows, so **P-M is FALSE** and the
STOP branch applies. The window table (extreme-value ratio per window: max pass latency over min
inter-exit interval):

```
window | span_s | latencies | max_latency_s | intervals | min_interval_s | ratio
W1 | 0-1800 | 63 | 12.092 | 62 | 14.712 | 0.822
W2 | 1800-3600 | 64 | 15.128 | 64 | 15.884 | 0.952
W3 | 3600-5400 | 65 | 16.363 | 65 | 15.623 | 1.047
W4 | 5400-7200 | 67 | 7.117 | 67 | 16.655 | 0.427
W5 | 7200-9000 | 66 | 6.094 | 66 | 15.381 | 0.396
W6 | 9000-10800 | 66 | 7.437 | 66 | 15.402 | 0.483
W7 | 10800-12600 | 67 | 6.640 | 67 | 15.858 | 0.419
W8 | 12600-14400 | 69 | 7.377 | 69 | 16.668 | 0.443
```

The `A7-epoch` per-epoch series stays in `spec368-plateau4h.predicates.txt`: **527 lines**, maximum
per-epoch `ratio=0.439`. The single window ratio ≥ 1 is W3's, and it is an extreme-value statistic (one
worst latency against one shortest interval in the same 1800 s window), not a per-epoch average.

### 3.5 P-S — TRUE

```
PS-rows=TRUE rows=241 evaluable=241 exempt=0 first_snapshot_elapsed=0 max(sum-lag)=-1
PS-verdict=TRUE verdict=O2
```

Every one of the 241 rows was evaluable, none exempt, and the verdict is the pre-registered O2. The
recorded shape is §3.3's `consistency:` line (one `durability_only` epoch, `retained_truncated=false`,
`OK`), `retained_closed_epochs=1`, and `durable_watermark_lag: max=3 last=2`.

### 3.6 P-B — DECAYING_NOT_BOUND, and the void replicate line

```
PB-C1 slope_W8=2772.000000 slope_W1=12922.088710 W8<W1=TRUE recorded_decay_observation
PB-C2 last_half_slope=656.985130 <=512=FALSE
PB-C3 n=241 skipped_empty=0 half_start=120 quarter_start=180 last_half_mean=35172.893 last_quarter_mean=35096.492 deviation_pct=0.217 <=10=TRUE
PB-ratio W8/W1=0.21
PB-near_threshold=NO
PB-level_near_miss=NO
PB=DECAYING_NOT_BOUND
REPLICATE=AUTHORIZED
```

**`REPLICATE=AUTHORIZED` is void.** `spec368-pb.awk` evaluates P-B alone and cannot see P-M, while the
replicate policy sits under the STOP rule — and P-M is FALSE. No replicate is run, and no third run. This
is a spec/program drift: the flag should have been gated on P-M ∧ P-S. The frozen program is **not**
edited; the drift is recorded here and in the umbrella tracker as a lesson for the next carve's programs
(conductor rulings v4, ruling 1).

### 3.7 P-F — recorded, never gated

```
PF-recon2 rows_within_2pct=202/240 NOT_MET
PF-recon3 rows_within_2pct=203/240 NOT_MET
PF-phys_footprint_mb peak=8670.961 last=8670.961 peak_eq_last=TRUE
PF-reclaimable_mb peak=1231.109 last=0.000 peak_eq_last=FALSE
PF-phys_footprint_peak_mb last=8670.961
PF-shape phys_footprint_mb last_half_slope_sign=+ slope_mb_per_hour=1802.274746
PF-shape reclaimable_mb last_half_slope_sign=- slope_mb_per_hour=-292.642028
```

`decidingSeries` shape tokens, from the harness:

```
"PLATEAU_NOT_MET"
"series rss_kib rose and was still rising at the end; firing envelope BOTH"
{"name":"rss_kib","shape":"MONOTONE_RISING","firingEnvelope":"BOTH","lastHalfMean":6945402}
{"name":"redb_bytes","shape":"MONOTONE_RISING","firingEnvelope":"PEAKS","lastHalfMean":183729658}
{"name":"wal_bytes","shape":"MONOTONE_RISING","firingEnvelope":"FLOOR","lastHalfMean":1399553}
{"name":"wal_segment_files","shape":"MONOTONE_RISING","firingEnvelope":"BOTH","lastHalfMean":518}
```

### 3.8 The three eight-window slope tables

From `spec368-plateau4h.fits.txt` (27 lines). The fitter's column is named `slope_mb_per_hour` for every
series; for `tombstone_bytes` the underlying column is bytes, so those figures are **B/h**. `LH` is the
last-half fit (`n=121`), the statistic C2 uses. `skipped_empty` is 0 in all 27 fits.

`tombstone_bytes` (B/h):

| window | span_s | slope | se | r² |
|---|---|---|---|---|
| W1 | 1800 | 12,922.089 | 11,285.280 | 0.0433 |
| W2 | 1800 | 15,393.435 | 11,198.743 | 0.0612 |
| W3 | 1800 | 1,042.745 | 7,832.563 | 0.0006 |
| W4 | 1800 | 13,368.750 | 6,529.125 | 0.1263 |
| W5 | 1800 | 2,446.920 | 7,378.714 | 0.0038 |
| W6 | 1800 | 4,157.109 | 8,465.463 | 0.0082 |
| W7 | 1800 | 7,209.387 | 8,224.057 | 0.0258 |
| W8 | 1380 | 2,772.000 | 10,301.920 | 0.0033 |
| LH | 7200 | **656.985** | 999.741 | 0.0036 |

Reference tables beside it, both `PB=NOT_MET` cells:

| reference | W1 … W8 `tombstone_bytes` slopes (B/h) | W8/W1 | source |
|---|---|---|---|
| `spec362b-long4h` | 95,653.60 / 56,417.64 / 171,702.41 / 154,358.56 / 212,434.31 / 224,270.03 / 484,239.22 / 651,649.15 | **6.81×** | `spec362-manifest.md:4281-4288,4253` |
| `spec355-w1000` | 113,657.12 / 244,197.34 / 151,075.57 / 166,532.06 / 118,189.21 / 132,688.50 / 133,598.47 / 155,726.40 | **1.37×** | `spec355-manifest.md:1192-1201` |

Every window of this cell is one to two orders of magnitude below both references, and the direction is
reversed: `W8/W1=0.21` here against 6.81 and 1.37 there.

`phys_footprint_mb` (MB/h):

| window | span_s | slope | se | r² |
|---|---|---|---|---|
| W1 | 1800 | 1,929.363 | 38.555 | 0.9886 |
| W2 | 1800 | 3,117.226 | 106.869 | 0.9670 |
| W3 | 1800 | 2,576.038 | 128.895 | 0.9323 |
| W4 | 1800 | 1,780.419 | 175.508 | 0.7802 |
| W5 | 1800 | 2,846.645 | 42.542 | 0.9936 |
| W6 | 1800 | 1,438.200 | 138.163 | 0.7889 |
| W7 | 1800 | 3,966.259 | 79.374 | 0.9885 |
| W8 | 1380 | 2,144.115 | 32.416 | 0.9950 |
| LH | 7200 | **1,802.275** | 47.151 | 0.9247 |

`reclaimable_mb` (MB/h):

| window | span_s | slope | se | r² |
|---|---|---|---|---|
| W1 | 1800 | 1,515.130 | 54.089 | 0.9644 |
| W2 | 1800 | −1,808.819 | 355.256 | 0.4720 |
| W3 | 1800 | 213.754 | 389.908 | 0.0103 |
| W4 | 1800 | 1,580.826 | 147.327 | 0.7988 |
| W5 | 1800 | −1,046.328 | 72.818 | 0.8768 |
| W6 | 1800 | −108.503 | 115.136 | 0.0297 |
| W7 | 1800 | −1,808.124 | 183.848 | 0.7693 |
| W8 | 1380 | −0.010 | 0.006 | 0.1200 |
| LH | 7200 | **−292.642** | 33.569 | 0.3897 |

### 3.9 Terminal census against the references

```
{"source":"TERMINAL","elapsedSecs":14402.13937625,"keysScanned":96,"keysUndecodable":0,"orMapKeys":96,"tombstoneEntries":1676,"tombstoneBytes":38548,"tombstoneDupEntries":0,"keysWithTombstones":48,"keysAllDead":0,"maxTombstonesPerKey":42}
{"scansAttempted":1,"scansFailed":0,"samples":1,"firstBytes":38548,"minBytes":38548,"peakBytes":38548,"lastBytes":38548,"firstHalfPeakBytes":0,"lastHalfPeakBytes":38548,"riseBytes":38548,"spanSecs":0.0,"disposition":"LEVEL_SUPPRESSED","ceilingBytes":null,"passed":true,"reason":null}
```

| quantity | this cell | `spec362b-long4h` reference (`spec362-manifest.md:4073,4088-4093`) |
|---|---|---|
| tombstone entries | 1,676 | 44,452 |
| tombstone bytes | 38,548 | 1,016,398 |
| max tombstones per key | 42 | 1,147 |

Same duration, same cadence, ~26× fewer entries and ~26× fewer bytes. The census is a single end-of-run
structural read of the **durable** store, so it is not the same measurement as the resident
`tombstone_bytes` gauge (23,805 B at the last scrape); both are small.

### 3.10 Regeneration, ordering, and the AC roll-up

```
READOUT_REGEN=IDENTICAL
PREDICATES_REGEN=IDENTICAL
ORDER=OK
```

Both regenerations ran on `mktemp -d` copies, never with `cwd` = evidence. `PREDICATES_REGEN=IDENTICAL`
covers `fits.txt`, `predicates.txt` and all eight segments; the single admissible non-IDENTICAL case (a
`spec366-p67.awk` `no_persisted_scrape_in` FALSE branch) did not arise, because `P6=TRUE gap=0`.

- **AC-14** — the R-Artifacts set spans the data commit (artifacts) and this append (§3);
  `ls spec368-plateau4h.scrapes | wc -l` = **241** = CSV data rows = the pre-registered 241, so no
  call-out is owed.
- **AC-15** — `ORDER=OK` with `M=a80c823d95932ebdf4bdb805771714f266214e2c`,
  `D=347ce2a17e977e537865868b85d048f74987ece4`: `M ≠ D`, `M` an ancestor of `D`, the first commit of all
  four `spec368-*.awk`, `spec368-predicates.sh` and `spec368-plateau4h.sh` equals `M`, no artifact path in
  `M`, `## APPEND-ONLY BELOW` occurring exactly once at `M`, and the text above it identical at `M` and
  `HEAD` (`93e02eed1cd0da7969f06b5de91ed064574f2d5d595ed8631e8dc0714c0e6f42` at both).
- **AC-16** — both regeneration lines IDENTICAL, no exception invoked.
- **AC-18** — no `.rs`, no `INVARIANTS.md`, no `scripts/check-invariants.sh`, no `spec365-readout.sh`,
  `spec349c2-fit.awk` or `spec366-*` change; `git diff d6a3d38f..HEAD --name-only` lists only
  `spec368*` paths under `evidence/`. `TG-OR-005` stays open and `NAKED_BASELINE` stays 4.

### 3.11 STOP marker

**P-M FALSE on PM-A7 (W3 ratio 1.047).** Literal reason line: `A7=FALSE reason=ratio_ge_1_W3`. The
pre-registered FALSE branch was followed: the artifacts and every predicate line are recorded as written,
the `REPLICATE=AUTHORIZED` line is void (§3.6), and nothing was re-run, re-tuned, widened or edited. P-B's
`DECAYING_NOT_BOUND` is a reading, not a FALSE; P-F and the census are never STOP triggers.

### 3.12 Conductor adjudication (reference/SPEC-368-conductor-rulings-v4.md)

Quoted from ruling 2 of the conductor session's file (local conductor file, not committed); the
cross-vendor second opinion recorded there agrees on the STOP reading.

1. **Mechanism holds at 4 h.** 527 epochs removed and settled, gap 0, zero restored-cancelled,
   RECONCILED, O2 as pre-registered. The cancellation-loss class stays CLOSED.
2. **Tombstone bytes are a bounded sawtooth, not a trend.** 24–45 KB around 35.2 KB (last-half mean) vs
   35.1 KB (last-quarter mean). The last-half OLS slope, 657 ± 1000 B/h, is indistinguishable from zero
   AND from 512; the harness gate (677 B/h) fires on the same noise. C2 and the harness gate are
   non-discriminating at this noise floor. The pre-registered verdict stands: `DECAYING_NOT_BOUND`; C1's
   decay ratio 0.21 is partly a start-up artefact (W1 begins at `y_first=0`). No gate change here
   (non-goal); this IS the input to the level/ceiling re-derivation carve.
3. **A7 is a capacity-margin warning, not a mechanism failure.** Max pass latency 12–16 s in W1–W3
   against a ~15 s min inter-exit interval; 6–7 s from W4 on. Extreme-value ratio; every epoch settled,
   no backlog. But prune throughput (~1,000 refs per 6–16 s) runs at only 1–2.5× the removal cadence
   (~1,000 refs / 15 s): the steady-state tombstone level is set by removal rate × pass latency, and the
   margin is thin. Follow-up: latency and interval DISTRIBUTIONS, p99/p1, and why early passes are 2×
   slower.
4. **Memory is the headline, and it is not tombstones.** `phys_footprint` 8.67 GB still rising
   1.8 GB/h in the last half while resident tombstone bytes are ~24 KB and durable tombstones 38.5 KB
   (the census scans the durable store; the gauge counts resident bytes — different measurements, both
   small). Live tags 1.06 M / 35.9 MB. The RSS residue is the umbrella tracker's earlier item, unchanged
   by this carve.
5. **Next carve: level/ceiling re-derivation**, pre-registering (a) a ceiling test on the last-half mean
   (C3's form is the only informative P-B statistic here), (b) if a slope is kept, a noise-robust
   estimator (Theil–Sen) with a threshold ≥ 2–3 se from the measured floor (se ≈ 1,000 B/h at 2 h / 60 s
   cadence) and a power calculation, (c) the harness gate re-derived on the same basis. `TG-OR-005`
   stays open; `NAKED_BASELINE` unchanged.
