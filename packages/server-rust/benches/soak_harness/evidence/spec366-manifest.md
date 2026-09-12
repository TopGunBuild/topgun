# spec366 manifest — prune-cancellation re-run (carve 8c)

§1 and §2 are committed BEFORE any `spec366-conj900` data exists. Neither is edited afterwards; §3
is appended after the run.

## §1 — Freeze and the closed difference list

### Freeze commit

- `SPEC366_CODE_FREEZE`: **`3a009e42`** (full: `3a009e42ced4bfcc771c3a7b4bec70819fbdc6a9`,
  `docs(server): state the prune task's liveness condition honestly`) — the last `.rs`-touching
  commit on `spec-8c-prune-cancel-safe`, eight commits on the SPEC-365 pin `550936dc`. At that
  commit the gate matrix was green: `cargo test -p topgun-server --lib` 1898 passed / 0 failed;
  workspace `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo fmt --check`
  clean; `pnpm test:sim` clean; `scripts/check-invariants.sh` exit 0; the full §A matrix re-run at
  this commit in a clean scratch worktree reported `MATRIX_OVERALL=PASS`, all ten gates `exit=0`.
- **The freeze MOVED from `a18e09de`, and this section is re-committed before the cell runs,
  exactly as the conditional clause below requires.** `3a009e42` is a COMMENT-ONLY change to
  `crdt.rs`: 5 insertions / 5 deletions, file length 9146 unchanged, both hunks at identical
  offsets and extents (`-751,3 +751,3`, `-1938,2 +1938,2`), so no code line moved. Two doc
  sentences claimed liveness the code does not provide — Review v1 raised them as its two majors
  and conductor rulings v7 R1 ordered the wording fixed.
- **Byte-identity against cell attempt 2 was TESTED and REFUTED**, which is why attempt 2 cannot be
  carried forward under this freeze and the cell is re-run as attempt 3 (rulings v7 R1.3). The
  measured binary was still intact on disk when the test ran (`ac34c2d7…`, control matched).
  Rebuilt at `3a009e42`: in a clean scratch worktree `a1ac5900…` (DIFFERENT), and in the main
  checkout — the same path the measurement used — `5c666185…` (DIFFERENT). The two rebuilds also
  differ from EACH OTHER, so this release profile (`lto`, `codegen-units=1`, `strip=symbols`) is
  not reproducible across build paths, and preserving comment line counts is not sufficient to
  make a comment-only edit produce a byte-identical binary.
- The literal committed in `spec366-conjuncts.sh` reads `SPEC366_CODE_FREEZE=3a009e42`, the
  abbreviated form of the SHA above; `git rev-parse 3a009e42` resolves to it, and the runner's own
  guard is `git diff --stat "$SPEC366_CODE_FREEZE"..HEAD -- '*.rs'`, which takes either form.
- **This literal is CONDITIONAL on no further `.rs` commit landing.** If any `.rs` commit lands
  after `3a009e42`, the literal in `spec366-conjuncts.sh` and this section MUST be updated and
  re-committed *before* the cell runs — otherwise the runner refuses to start (guard 2), which is
  the fail-closed direction, but the freeze recorded here would be false.

### Scripts

| file | sha256 | status |
|---|---|---|
| `spec365-conjuncts.sh` | `1087ec6f33d8dcad537ae5af43d06d7a5391e55b45d01e3efec2cff5a2074591` | frozen evidence, byte-unchanged (equals the digest recorded in `spec365-manifest.md` §1) |
| `spec365-readout.sh` | `5fbfa3e9d9df2a74830edbf89cf4036ae3e376add9171676fe70800b335d7a11` | frozen evidence, byte-unchanged (ditto); it is the readout this run uses, UNEDITED, invoked with basename `spec366-conj900` |
| `spec366-conjuncts.sh` | `e1c283a069f66cc521ece02ae567126844e147a10b130b54ea586d2f2ce92fcb` | this spec's runner. Two changes since it was first committed, each recorded here rather than narrated: **difference item 5** (the pre-clock provenance assertion), added after cell attempt 1 was invalidated; and **item 1's freeze literal repointed** `a18e09de` → `3a009e42` for attempt 3, after Review v1 forced a comment-only `.rs` fix and byte-identity against attempt 2 was refuted. Prior digests: `9b073f6c786827080e78279a60d1e23ad5cefc19b8f601f729dc6fb237734be4` before item 5, `401d61c95bcdb9ba69f91e0eb2ad6d543ef27b98e4d471d39c6cdf562e529d59` before the repoint. |
| `spec366-p5.awk` | `2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33` | P5, committed so the predicate is regenerable by path; bytes equal the §2 block |
| `spec366-p67.awk` | `ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215` | P6/P7, ditto |

### The difference, verbatim

Literal output of `diff spec365-conjuncts.sh spec366-conjuncts.sh`, so the closed list is checked
mechanically rather than narrated:

```diff
3c3,4
< # Prune-conjunct readout cell runner.
---
> # Prune-conjunct readout cell runner -- the SPEC-366 successor of the frozen
> # spec365-conjuncts.sh, which is NOT edited.
4a6,62
> # A SUCCESSOR EXISTS BECAUSE THE FROZEN RUNNER CANNOT BE RUN FOR THIS SPEC.
> # spec365-conjuncts.sh refuses to start unless `git diff <freeze>..HEAD --
> # '*.rs'` is empty against SPEC365_CODE_FREEZE, and that guard has no override
> # while SPEC-366 changes .rs files by definition; it also hard-codes
> # BASE="spec365-conj900", whose artifacts are committed evidence a re-run must
> # not overwrite. This file is therefore a COPY, and the difference list against
> # spec365-conjuncts.sh is CLOSED at exactly five items:
> #
> #   1. THE FREEZE VARIABLE IS RENAMED SPEC366_CODE_FREEZE and its literal is
> #      SPEC-366's own freeze commit. The same three refusal guards -- the
> #      placeholder check, the .rs diff against the freeze commit, the dirty
> #      .rs working tree -- are otherwise unchanged, and none has an override.
> #   2. BASE="spec366-conj900". THE ENV OVERRIDE NAMES STAY VERBATIM
> #      (SPEC365_OUT_DIR, SPEC365_FORCE, SPEC365_DATA_DIR, SPEC365_SOAK_BIN,
> #      SPEC365_SMOKE_SAMPLE_INTERVAL): the readout below is the UNCHANGED
> #      spec365-readout.sh and resolves its own OUT_DIR from SPEC365_OUT_DIR, so
> #      renaming them would point the runner and the readout at DIFFERENT
> #      directories whenever a scratch OUT_DIR is used.
> #   3. EVERY SCRAPE IS PERSISTED. scrape_prune_metrics additionally writes the
> #      raw /metrics body, VERBATIM, to ${OUT_DIR}/${BASE}.scrapes/<RFC3339
> #      UTC>.txt. No cadence change, no CSV change, no change to
> #      PRUNE_METRIC_NAMES: the same single response body is both parsed into
> #      the row and kept whole. A failed curl writes an EMPTY file, so a scrape
> #      that did not happen is a fail-closed input the manifest's P6/P7 can
> #      name, not a silently missing series. The directory JOINS the
> #      artifact-overwrite refusal, because that refusal enumerates named files
> #      and a directory beside them would otherwise let a second run MIX its
> #      scrapes with the first run's -- which corrupts the decision-scrape
> #      selection silently instead of failing loudly.
> #   4. this header and the usage text, naming the successor.
> #   5. A PRE-CLOCK PROVENANCE ASSERTION on the server binary, added after cell
> #      attempt 1 was invalidated. That attempt ran a server built from the PIN,
> #      not from this branch: an earlier experiment had shared one
> #      CARGO_TARGET_DIR between a pin worktree and the main checkout, cargo gave
> #      both source paths the same metadata hash, and `cargo build` then judged
> #      the pin-built binary fresh. The cell produced a full set of artifacts and
> #      a readout, and NOTHING in the run said the measured binary was the wrong
> #      one -- the predicates simply read as FALSE. So, after this runner's own
> #      build and BEFORE T0:
> #        (a) the binary must CONTAIN the string of a counter this branch adds,
> #            `topgun_or_prune_restored_cancelled_total`, else FATAL naming it;
> #        (b) its mtime must be >= this invocation's recorded start, else FATAL
> #            "stale artifact -- not built by this invocation";
> #        (c) its sha256, already on the matrix, is repeated as the FIRST line of
> #            the console log, so every artifact set carries the identity of the
> #            binary that produced it.
> #      A measurement that cannot say which binary it ran is not evidence.
> #
> # The matrix, the cell literals, the harness flags, the log directive and the
> # readout invocation are byte-identical, and the readout is the UNCHANGED
> # spec365-readout.sh invoked with basename spec366-conj900. The departure is
> # enumerable with:
> #   diff spec365-conjuncts.sh spec366-conjuncts.sh
> #
> # Everything from here on is spec365-conjuncts.sh's own header, kept verbatim
> # so the lineage back to spec362b-durable.sh stays readable.
> #
49c107
< #       becomes SPEC365_CODE_FREEZE here, with one added behaviour the parent
---
> #       becomes SPEC366_CODE_FREEZE here, with one added behaviour the parent
70c128
< # THE FREEZE GATE (g) PINS THE .rs TREE. SPEC365_CODE_FREEZE names the commit
---
> # THE FREEZE GATE (g) PINS THE .rs TREE. SPEC366_CODE_FREEZE names the commit
106c164
< usage: spec365-conjuncts.sh <cell>
---
> usage: spec366-conjuncts.sh <cell>
108,109c166,168
<   Derived from spec362b-durable.sh, which is not edited. The difference list
<   against that file is CLOSED and is enumerated in the header block above.
---
>   A COPY of spec365-conjuncts.sh, which is not edited. The difference list
>   against that file is CLOSED, has exactly five items, and is enumerated in
>   the header block above.
121c180
<   SPEC365_CODE_FREEZE still reads its own placeholder value, or unless the
---
>   SPEC366_CODE_FREEZE still reads its own placeholder value, or unless the
149c208
<               EXTRA_FLAGS=""; BASE="spec365-conj900" ;;
---
>               EXTRA_FLAGS=""; BASE="spec366-conj900" ;;
177a237,242
> 
> # Item 5: this invocation's start, recorded BEFORE anything is built. The
> # provenance clause below compares the server binary's mtime against it, so the
> # timestamp has to predate the build or the comparison proves nothing.
> RUN_START_EPOCH="$(date +%s)"
> RUN_START_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
179a245,249
> # Every /metrics body this run scrapes, kept verbatim, one file per sample.
> # P6/P7 are decided on a RAW body taken outside every open prune window, and no
> # CSV row can carry one: the row is 31 numbers awk-ed out of a body that today
> # is discarded.
> SCRAPES_DIR="${OUT_DIR}/${BASE}.scrapes"
378,380c448,450
< SPEC365_CODE_FREEZE=9fdaaf5a141bf24ee3b7801cb6a16c861ab228cc
< if [ "$SPEC365_CODE_FREEZE" = "PENDING_SPEC365_CODE_FREEZE" ]; then
<   echo "FATAL: SPEC365_CODE_FREEZE still reads its placeholder value." >&2
---
> SPEC366_CODE_FREEZE=3a009e42
> if [ "$SPEC366_CODE_FREEZE" = "PENDING_SPEC366_CODE_FREEZE" ]; then
>   echo "FATAL: SPEC366_CODE_FREEZE still reads its placeholder value." >&2
391,392c461,462
< if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC365_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
<   echo "FATAL: the .rs diff against the freeze commit ${SPEC365_CODE_FREEZE} could" >&2
---
> if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC366_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
>   echo "FATAL: the .rs diff against the freeze commit ${SPEC366_CODE_FREEZE} could" >&2
399c469
<   echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC365_CODE_FREEZE}; this run would not be filed under it" >&2
---
>   echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC366_CODE_FREEZE}; this run would not be filed under it" >&2
449a520,572
> 
> # ---------------------------------------------------------------------------
> # Item 5 -- PRE-CLOCK PROVENANCE ASSERTION.
> #
> # Cell attempt 1 ran a server built from the PIN and produced a complete,
> # plausible artifact set: a readout, a CSV, 16 scrapes, and predicates that
> # simply read FALSE. Nothing in the run said the measured binary was the wrong
> # one. These two clauses are what turn that silent failure into a refusal, and
> # they run BEFORE T0 so a bad binary costs nothing but a restart.
> # ---------------------------------------------------------------------------
> PROV_COUNTER="topgun_or_prune_restored_cancelled_total"
> 
> # (a) The binary must carry a symbol this branch introduces. A binary built
> #     from any earlier source simply does not contain the string.
> #     NOT `grep -q`: this runner is `set -euo pipefail`, and `grep -q` exits at
> #     the first match, so `strings` takes SIGPIPE and the PIPELINE reports 141
> #     even when the string IS present -- measured rc=141 against a binary that
> #     contains it. That would fail the assertion on a CORRECT binary and refuse
> #     every run. `grep -c` consumes all of its input, so the status reflects the
> #     match count rather than a broken pipe.
> PROV_HITS="$(strings "$SERVER_BIN" | grep -c "$PROV_COUNTER" || true)"
> if [ "${PROV_HITS:-0}" -eq 0 ]; then
>   echo "FATAL: the server binary does not contain '${PROV_COUNTER}'." >&2
>   echo "       binary: $SERVER_BIN" >&2
>   echo "       built:  $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')" >&2
>   echo "       sha256: ${SERVER_BIN_SHA256:-<unavailable>}" >&2
>   echo "       This counter is emitted by the branch under test, so a binary" >&2
>   echo "       without it was built from other sources. Attempt 1 ran exactly" >&2
>   echo "       such a binary and the cell was worthless." >&2
>   exit 1
> fi
> 
> # (b) It must have been produced by THIS invocation. A binary older than the
> #     run's own start was inherited from somewhere else, which is precisely how
> #     a stale artifact survives an intervening `cargo build` that judged it
> #     fresh.
> SERVER_MTIME_EPOCH="$(date -r "$SERVER_BIN" '+%s')"
> if [ "$SERVER_MTIME_EPOCH" -lt "$RUN_START_EPOCH" ]; then
>   echo "FATAL: stale artifact -- not built by this invocation." >&2
>   echo "       binary: $SERVER_BIN" >&2
>   echo "       built:  $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')" >&2
>   echo "       run started: ${RUN_START_UTC}" >&2
>   echo "       Remove it and let this runner rebuild it; do not reuse a binary" >&2
>   echo "       from another tree or another run." >&2
>   exit 1
> fi
> 
> # (c) The identity of the measured binary travels WITH the artifacts: the same
> #     sha256 the matrix records is repeated as the console log's first line.
> #     The predicate awks skip any line without a timestamp prefix, so this is
> #     inert to them.
> PROV_LINE="provenance: server sha256=${SERVER_BIN_SHA256} built=$(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ') run_start=${RUN_START_UTC} ${PROV_COUNTER}=present"
> echo "$PROV_LINE"
450a574,579
> # Every path that publishes the console artifact goes through this, so the
> # provenance line cannot be lost on an early-exit path.
> write_console_out() {
>   { printf '%s\n' "$PROV_LINE"; cat "$CONSOLE_LOG"; } > "$CONSOLE_OUT" 2>/dev/null || true
> }
> 
517a647,663
> # The per-sample scrape directory is an ARTIFACT and joins the refusal above.
> # The loop enumerates NAMED FILES, so a directory beside them would sit outside
> # it, and a second run would then MIX its scrapes with the surviving ones --
> # corrupting the decision-scrape selection silently rather than failing loudly.
> if [ -e "$SCRAPES_DIR" ] && [ "${SPEC365_FORCE:-0}" != "1" ]; then
>   if [ ! -d "$SCRAPES_DIR" ] || [ -n "$(ls -A "$SCRAPES_DIR" 2>/dev/null || true)" ]; then
>     echo "FATAL: artifact already exists: $SCRAPES_DIR" >&2
>     echo "       Move it aside, or re-run with SPEC365_FORCE=1 to overwrite." >&2
>     exit 1
>   fi
> fi
> rm -rf "$SCRAPES_DIR"
> if ! mkdir -p "$SCRAPES_DIR"; then
>   echo "FATAL: cannot create artifact directory: $SCRAPES_DIR" >&2
>   exit 1
> fi
> 
565c711
<   echo "  code freeze:            ${SPEC365_CODE_FREEZE}"
---
>   echo "  code freeze:            ${SPEC366_CODE_FREEZE}"
661c807
<     cp -f "$CONSOLE_LOG" "$CONSOLE_OUT" 2>/dev/null || true
---
>     write_console_out
670c816
<   cp -f "$CONSOLE_LOG" "$CONSOLE_OUT" 2>/dev/null || true
---
>   write_console_out
699c845
<   local body
---
>   local body stamp
700a847,854
>   # The body is kept WHOLE before it is reduced to 31 numbers, under the same
>   # fixed-width UTC RFC 3339 stamp the console prefix uses -- the one time
>   # domain the manifest's window rules compare in. A failed curl writes an
>   # EMPTY file on purpose: an unreadable scrape is a named fail-closed reason
>   # for P6/P7, whereas a missing file would be indistinguishable from a sample
>   # that was never due.
>   stamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
>   printf '%s' "$body" > "${SCRAPES_DIR}/${stamp}.txt" || true
937c1091
< cp -f "$CONSOLE_LOG" "$CONSOLE_OUT"
---
> write_console_out
```

### Hunk → item mapping

The AC reads "the hunks map onto the five items", not "five hunks". Item 1's rename touches every
site that reads the variable; item 3 lands as four hunks; and item 5, added after cell attempt 1 was
invalidated, lands as six because its three clauses sit at three different points in the run.

| hunk | item | what it is |
|---|---|---|
| `3c3,4` | **4** | the file's opening line names the SPEC-366 successor of the frozen runner. |
| `4a6,62` | **4** | the successor header block: why a successor exists, and the CLOSED list itself (it *describes* items 1-3 and 5; it *is* item 4). |
| `49c107` | **1 (prose)** | the parent's `(g)` freeze-gate paragraph names the renamed variable. |
| `70c128` | **1 (prose)** | the `THE FREEZE GATE (g) PINS THE .rs TREE` paragraph names the renamed variable. |
| `106c164` | **4** | `usage:` line names the successor. |
| `108,109c166,168` | **4** | usage prose: a COPY of spec365-conjuncts.sh, closed at five items. |
| `121c180` | **1 (prose)** | the usage refusal paragraph names the renamed variable. |
| `149c208` | **2** | `BASE="spec366-conj900"`. Nothing else on the cell line changes; the env override names are untouched everywhere. |
| `177a237,242` | **5** | `RUN_START_EPOCH` / `RUN_START_UTC`, recorded BEFORE any build so clause (b) compares against something that predates it. |
| `179a245,249` | **3** | `SCRAPES_DIR` -- the path the persisted bodies are written to. |
| `378,380c448,450` | **1** | the freeze variable, its literal, and guard 1 (the placeholder refusal). |
| `391,392c461,462` | **1** | guard 2 (the `.rs` diff against the freeze commit) reads the renamed variable. |
| `399c469` | **1** | guard 2's FATAL message names the renamed variable. Guard 3 (dirty `.rs` tree) reads no variable and is byte-unchanged. |
| `449a520,572` | **5** | clauses (a) and (b): the counter-string assertion and the mtime assertion, both BEFORE `T0`, each with its own FATAL. |
| `450a574,579` | **5** | clause (c)'s helper `write_console_out`, so the provenance line cannot be lost on an early-exit path. |
| `517a647,663` | **3** | `.scrapes/` joins the artifact-overwrite refusal: same FATAL wording, same `SPEC365_FORCE=1` override, refuse-when-non-empty, else clear and recreate. |
| `565c711` | **1 (mechanically forced)** | the matrix banner echoes the freeze variable. Not a choice: the runner is `set -euo pipefail`, so echoing the now-nonexistent `SPEC365_CODE_FREEZE` would abort at the banner. |
| `661c807` | **5** | console-out publish on the harness-failure path routed through the helper. |
| `670c816` | **5** | console-out publish on the second early-exit path. |
| `699c845` | **3** | `local body stamp` -- the declaration the persistence write needs. |
| `700a847,854` | **3** | the verbatim write itself, plus the reason a failed curl writes an EMPTY file. |
| `937c1091` | **5** | console-out publish on the normal completion path. |

Twenty-two hunks, five items, nothing else: item 1 → 7, item 2 → 1, item 3 → 4, item 4 → 4,
item 5 → 6. Mechanically re-checkable at any time with the `diff` above.
### Byte-identity of the parts the spec requires to be identical

Checked by extracting each region from both files and diffing it; all six reported IDENTICAL:

- `CSV_HEADER` (the 41-column D5 literal the frozen readout re-asserts);
- `PRUNE_METRIC_NAMES` (the 31-name scrape list);
- the pinned matrix block, `CHURN_CLIENTS=6` … `JITTER_SEED=20260831` (this includes
  `SERVER_PORT=47356`, so the successor still cannot collide with `spec362b-durable.sh`);
- the harness invocation, `"$SOAK_BIN" \` … `HARNESS_PID=$!` (every flag, including
  `--durable-reading`, `--live-census-interval 0`, `--sampler-jitter-seed`);
- the log directive and the whole environment-discipline block, `ORIGIN_LOG_DIRECTIVE=` …
  `unset TOPGUN_OR_DELTA_WAL`;
- the readout invocation, `READOUT_SCRIPT=` … `exit "$HARNESS_RC"`.

The cell line differs only in `BASE`; `WIDTH`, `DURATION=900`, `SAMPLE_INTERVAL=60`,
`PROVENANCE=no`, `CELL_CRASH_INTERVAL=0`, `ARM_LOG=yes`, `CELL_LIVE_CENSUS=0` and `EXTRA_FLAGS`
are unchanged.

### Consequences of keeping the list CLOSED (recorded, not fixed)

1. **The scratch data dir is still `target/spec365-conj900-data`** (`DATA_DIR` default,
   `spec365-${CELL}-data`). Renaming it is not on the closed list, so it is not renamed. The
   consequence is operational and LOUD, not silent: the runner refuses to start on a non-empty data
   dir, so a surviving SPEC-365 directory produces a FATAL before the clock, not a mixed corpus.
   Clear it, or export `SPEC365_DATA_DIR`, before the run.
2. **The matrix banner still reads `=== spec365 conjunct-readout run: cell conj900 ===`.** The
   banner is part of "the matrix … is byte-identical", so it is not retitled; the artifact paths it
   prints already carry `spec366-conj900`, and the one banner line that does change (`code freeze`)
   changes only because `set -u` forces it.
3. **Every env override keeps its `SPEC365_` name**, including in the new `.scrapes/` guard's own
   message (`re-run with SPEC365_FORCE=1`). That is deliberate: the readout resolves `OUT_DIR` from
   `SPEC365_OUT_DIR`, so one renamed variable would silently split runner and readout across two
   directories under any scratch `OUT_DIR`.

## §2 — Pre-registered predicates

All of P1, P2, P3, P5, P6, P7 must be TRUE. **P4 is WITHDRAWN** (the timeout-counter predicate is
vacuous without D6 — the counter cannot increment in this binary; it returns as a unit proof in
SPEC-367). Its number is kept as a gap so P5–P7 keep the names they carry in the pre-audit record.

Throughout, `EV` is this directory and `BASE=spec366-conj900`:

```bash
EV=packages/server-rust/benches/soak_harness/evidence
BASE=spec366-conj900
```

### The one time domain (no `mktime`)

Every scrape file is named with the same fixed-width UTC RFC 3339 stamp the console prefix uses
(`date -u +%Y-%m-%dT%H:%M:%SZ`), and every console-vs-scrape comparison below is **lexicographic
on the first 19 characters** (`YYYY-MM-DDTHH:MM:SS`) of **both** sides.

This is required, not stylistic:

- BSD `awk` (macOS) has no `mktime`, and the `settlement` row carries no `ts` field at all, so epoch
  arithmetic is unavailable on one side of the comparison;
- the console prefix carries microseconds and the file stamp does not, so comparing raw strings is
  wrong at the closing edge: `.` (0x2E) sorts BEFORE `Z` (0x5A), which would make a same-second
  scrape read as later than every microsecond console line in that second, i.e. spuriously OUTSIDE
  the window.

Truncating both sides to 19 characters removes the asymmetry, and the rule that follows from it is
pre-registered here: an **equal-second stamp is INSIDE the window at BOTH edges**. That is the
conservative direction — it can discard a usable scrape, never admit an in-flight one.

**One exception, stated so it is not mistaken for a second domain:** pairing a `settlement` row to
the `removal` row it closes is console-vs-console, where both sides carry microseconds in one
fixed-width format. That comparison uses the full stamps ("the first settlement for the same epoch
**strictly after** the removal"). Only the window *edges* are then truncated to 19 characters for
comparison against scrape file names.

### Open window, zero-return exclusion, decision scrape

- **Open window.** A window is open from a `removal` row's timestamp until the first `settlement`
  row for the **same epoch** strictly after it. A `removal` row that never gets such a settlement
  leaves its window OPEN to the end of the log.
- **A `removal` row carrying `refs_returned=0` NEVER opens a window**, and is excluded from P5's
  settlement clause. An eligible epoch can be removed from the index and return zero refs: the
  frontier emits the `removal` row and the prune loop never builds a per-epoch entry, so no
  `settlement` row can ever follow (`crdt.rs` documents this as case 2 and states that the pass
  record cannot distinguish it from "no epoch was eligible"). It is a correct-binary outcome. Such
  an epoch is listed in §3 as an observation, with its epoch and timestamp. **No widening beyond
  `refs_returned=0`.**
- **Decision scrape.** P6 and P7 are decided on the **LAST persisted scrape whose file stamp lies
  outside every open window**. `removed_refs_observed_total` is credited at drain time and
  `considered_total` at the end of the pass (the guard's `Drop`), so a sample taken INSIDE a pass
  reads MISMATCH even on a correct binary — that is what the window is for.
- **If no scrape qualifies, §3 writes `OPEN` and P6 and P7 are both FALSE.** The pinned command
  prints `DECISION_SCRAPE=OPEN` followed by two FALSE lines.

### Fail-closed

The P6/P7 awk returns FALSE **with a named reason** on: an empty or unreadable scrape file, an
absent series, or a non-integer sample. `absent == absent` is NOT true anywhere in it. The three
series are Prometheus counters rendered as unsigned integers, so the accepted sample form is
`/^[0-9]+$/` and anything else (`27000.0`, `2.7e4`, an empty token) is the named reason
`non_integer_sample`. §3 pastes the command's output verbatim.

### P1 — `reconciliation=RECONCILED`

```bash
LC_ALL=C awk '/^reconciliation=/ { seen = 1; print ($0 == "reconciliation=RECONCILED") ? "P1=TRUE" : "P1=FALSE reason=" $0 } END { if (!seen) print "P1=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
```

### P2 — `split_epochs` empty

The frozen readout appends ` split_epochs=…` to the `reconciliation=` line only when the list is
non-empty, so the absence of that token IS the predicate.

```bash
LC_ALL=C awk '/^reconciliation=/ { seen = 1; print (index($0, "split_epochs=") == 0) ? "P2=TRUE" : "P2=FALSE reason=" substr($0, index($0, "split_epochs=")) } END { if (!seen) print "P2=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
```

### P3 — verdict `O3`

First-match-wins over O4, O1, O5, O2, O3 (`spec365-readout.sh:593-616`): not O4, `X ≤ τ'`,
`X ≥ −τ`, and `ret_refs_durability_only + ret_refs_both ≤ ret_refs_claim_only`. The verdict is read
off the readout's own `READOUT:` line, not recomputed.

```bash
LC_ALL=C awk '/^READOUT: / { seen = 1; v = $2; sub(/;$/, "", v); print (v == "O3") ? "P3=TRUE" : "P3=FALSE reason=verdict_" v } END { if (!seen) print "P3=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"
```

### P4 — WITHDRAWN

Number kept as a gap. No command.

### P5 — settlement discipline

Every `settlement` row has `restored_cancelled=0`, and every epoch with a `removal` row has at
least one `settlement` row — **except at most one terminal-pass epoch** under the carve-out below,
and excluding any epoch whose `removal` row carried `refs_returned=0`.

**Carve-out (terminal pass only).** At most ONE epoch may lack a settlement, and only when BOTH
hold: (i) it is the frozen readout's `IN_FLIGHT` epoch — its `op_seq` equals the global max
(`spec365-readout.sh:388-397`); and (ii) its `removal` row timestamp is within **15 s** before the
console log's last server timestamp. The 15 s is computed **without `mktime`**: both stamps are
split at `T`, each time converted to seconds-of-day (`h*3600 + m*60 + s`), and `86400` added when
the dates differ by one day. Date adjacency is decided by an integer Gregorian day index
(`daynum()` below — pure arithmetic, no library call); a difference above one day, or a negative
one, fails the clause outright. Any other unsettled epoch ⇒ **P5 FALSE**. The carve-out is about a
pass cut off by teardown; it is independent of, and does not stack with, the `refs_returned=0`
exclusion.

**Pre-registered known limit (SPEC-365's, verbatim):** `op_seq` is constant within a pass, so
condition (i) ALONE could hide a genuine `NO_SETTLEMENT` of the last pass — condition (ii) is
exactly why (i) is not used alone.

**Readout interaction, pre-registered.** The frozen readout parses only the pre-existing settlement
fields and sums only three `restored_*` fields into `restored_sum`, so any `restored_cancelled > 0`
on a settlement row makes that epoch's six-exit check fail and it reads `MISMATCH` in the readout's
§C. That outcome is **P5 FALSE anyway**, and the readout is NOT edited.

Committed as `evidence/spec366-p5.awk`, whose bytes equal the block below.
`sha256 = 2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33`.
The predicate is therefore regenerable by path, which AC 17 requires:

```awk
# P5 -- settlement discipline, over the ANSI-stripped console log.
function add(a, b) { return (a == "") ? b : a "; " b }
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function sod(t,   h, m, s2) {                       # seconds of day of YYYY-MM-DDTHH:MM:SS
    h = substr(t, 12, 2) + 0; m = substr(t, 15, 2) + 0; s2 = substr(t, 18, 2) + 0
    return h * 3600 + m * 60 + s2
}
function daynum(t,   y, mo, d, a, yy, mm) {         # Gregorian day index; integer only, no mktime
    y = substr(t, 1, 4) + 0; mo = substr(t, 6, 2) + 0; d = substr(t, 9, 2) + 0
    a = int((14 - mo) / 12); yy = y + 4800 - a; mm = mo + 12 * a - 3
    return d + int((153 * mm + 2) / 5) + 365 * yy + int(yy / 4) - int(yy / 100) + int(yy / 400) - 32045
}
BEGIN { ESC = sprintf("%c", 27); gmax = -1 }
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]\./) last_ts = substr(line, 1, 19)
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned"); os = fld(line, "op_seq")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/ || os !~ /^[0-9]+$/) { unparsable = add(unparsable, "removal row at " substr(line, 1, 19)); next }
        nrem++
        if (rr + 0 == 0) { zn++; zero[zn] = "epoch=" ep " at=" substr(line, 1, 19); next }
        if (!(ep in rem)) { rem[ep] = 1; ord[++nord] = ep }
        rts[ep] = substr(line, 1, 19)
        if (!(ep in maxop) || os + 0 > maxop[ep] + 0) maxop[ep] = os + 0
        if (os + 0 > gmax) gmax = os + 0
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch"); rc = fld(line, "restored_cancelled")
        nset++
        if (ep !~ /^[0-9]+$/) { unparsable = add(unparsable, "settlement row at " substr(line, 1, 19)); next }
        settled[ep]++
        if (rc == "") rcbad = add(rcbad, "epoch " ep ": restored_cancelled ABSENT")
        else if (rc !~ /^[0-9]+$/) rcbad = add(rcbad, "epoch " ep ": restored_cancelled non-integer '" rc "'")
        else if (rc + 0 != 0) rcbad = add(rcbad, "epoch " ep ": restored_cancelled=" rc)
        next
    }
}
END {
    msg = ""
    if (unparsable != "") msg = add(msg, "unparsable rows: " unparsable)
    if (nrem == 0) msg = add(msg, "no removal row in the log")
    un = 0; unlist = ""
    for (i = 1; i <= nord; i++) {
        ep = ord[i]
        if (!(ep in settled)) { un++; unlist = unlist (un > 1 ? "," : "") ep; only = ep }
    }
    if (rcbad != "") msg = add(msg, "restored_cancelled clause: " rcbad)
    carve = ""
    if (un == 1) {
        ci = (maxop[only] + 0 == gmax + 0)
        cii = 0; why = ""
        if (last_ts == "") why = "no server timestamp in the log"
        else {
            dd = daynum(last_ts) - daynum(rts[only])
            if (dd < 0 || dd > 1) why = "date difference above one day (dd=" dd ")"
            else {
                delta = sod(last_ts) - sod(rts[only]) + 86400 * dd
                if (delta >= 0 && delta <= 15) cii = 1
                else why = "removal at " rts[only] " is " delta "s before the last server line " last_ts ", not within 15s"
            }
        }
        carve = sprintf("carve-out: epoch %s in_flight=%s (epoch max op_seq %s vs global %s) terminal=%s%s",
                        only, (ci ? "yes" : "no"), maxop[only], gmax, (cii ? "yes" : "no"), (cii ? "" : " [" why "]"))
        if (!(ci && cii)) msg = add(msg, "unsettled epoch " only " fails the terminal-pass carve-out")
    } else if (un > 1) {
        msg = add(msg, "unsettled epochs " unlist " (the carve-out admits at most one)")
    }
    print (msg == "" ? "P5=TRUE" : "P5=FALSE reason=" msg)
    printf("P5-observed: removal_rows=%d settlement_rows=%d unsettled=%s\n", nrem, nset, (un ? unlist : "none"))
    if (carve != "") print "P5-" carve
    for (i = 1; i <= zn; i++) print "P5-zero-return: " zero[i]
    if (zn + 0 == 0) print "P5-zero-return: none observed"
}
```

```bash
LC_ALL=C awk -f spec366-p5.awk "$EV/$BASE.harness-console.log"
```

### P6 and P7 — one shared, fail-closed command on the decision scrape

- **P6:** `removed_refs_observed_total == considered_total` on the decision scrape.
- **P7:** `topgun_or_prune_restored_cancelled_total == 0` on the same scrape.

They share one command because they share the decision scrape: selecting it twice would be two
selection rules, and a divergence between them is exactly the defect this evidence exists to rule
out. Committed as `evidence/spec366-p67.awk`, whose bytes equal the block below.
`sha256 = ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215`.
The predicate is therefore regenerable by path, which AC 17 requires:

```awk
# P6/P7 -- decided on the LAST persisted scrape lying outside every open prune
# window. Fail-closed: every branch that cannot READ a number prints FALSE with
# a named reason. `absent == absent` is never true.
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function fail(reason) { print "P6=FALSE reason=" reason; print "P7=FALSE reason=" reason; exit 0 }
BEGIN { ESC = sprintf("%c", 27) }
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]\./) next
    stamp_full = line; sub(/ .*/, "", stamp_full)          # full-precision console stamp
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/) { bad_rows++; next }
        if (rr + 0 == 0) { zero_rows++; next }            # a zero-return removal NEVER opens a window
        nw++; w_ep[nw] = ep; w_open_full[nw] = stamp_full; w_open19[nw] = substr(line, 1, 19)
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch")
        if (ep !~ /^[0-9]+$/) { bad_rows++; next }
        ns++; s_ep[ns] = ep; s_full[ns] = stamp_full; s_19[ns] = substr(line, 1, 19)
        next
    }
}
END {
    if (bad_rows > 0) fail("unparsable_console_row(" bad_rows ")")
    # Close each window with the FIRST settlement of the same epoch strictly after
    # it. Console-vs-console only, so the full-precision stamps compare directly:
    # both sides carry microseconds in one fixed-width format.
    for (i = 1; i <= nw; i++) {
        w_close19[i] = ""                                  # "" == still OPEN
        best = ""
        for (j = 1; j <= ns; j++) {
            if (s_ep[j] != w_ep[i]) continue
            if (s_full[j] <= w_open_full[i]) continue
            if (best == "" || s_full[j] < best) { best = s_full[j]; w_close19[i] = s_19[j] }
        }
    }
    # The scrape stamps, lexicographic on the first 19 characters of the file name.
    cmd = "ls -1 " scrapes_dir " 2>/dev/null"
    nf = 0
    while ((cmd | getline fn) > 0) {
        if (fn !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z\.txt$/) continue
        nf++; files[nf] = fn; stamps[nf] = substr(fn, 1, 19)
    }
    close(cmd)
    if (nf == 0) fail("no_persisted_scrape_in " scrapes_dir)
    # LAST scrape outside EVERY window. Equal-second is INSIDE at both edges.
    ds = ""; ds_file = ""
    for (k = 1; k <= nf; k++) {
        inside = 0
        for (i = 1; i <= nw; i++) {
            if (stamps[k] < w_open19[i]) continue
            if (w_close19[i] == "" || stamps[k] <= w_close19[i]) { inside = 1; break }
        }
        if (inside) continue
        if (ds == "" || stamps[k] > ds) { ds = stamps[k]; ds_file = files[k] }
    }
    if (ds == "") { print "DECISION_SCRAPE=OPEN"; fail("decision_scrape_OPEN(no scrape outside every open window)") }
    print "DECISION_SCRAPE=" ds_file
    path = scrapes_dir "/" ds_file
    if ((getline probe < path) < 0) fail("unreadable_scrape " ds_file)
    close(path)
    nlines = 0
    while ((getline mline < path) > 0) {
        nlines++
        if (mline ~ /^[[:space:]]*#/) continue
        nm = mline; sub(/[[:space:]].*$/, "", nm)
        b = index(nm, "{"); if (b > 0) nm = substr(nm, 1, b - 1)
        rest = mline; sub(/^[^[:space:]]+[[:space:]]+/, "", rest); sub(/[[:space:]].*$/, "", rest)
        if (!(nm in seen)) { seen[nm] = 1; val[nm] = rest }
    }
    close(path)
    if (nlines == 0) fail("empty_scrape " ds_file)
    OBS = "topgun_or_prune_removed_refs_observed_total"
    CON = "topgun_or_prune_considered_total"
    CAN = "topgun_or_prune_restored_cancelled_total"
    # P6
    if (!(OBS in seen)) print "P6=FALSE reason=absent_series " OBS " in " ds_file
    else if (!(CON in seen)) print "P6=FALSE reason=absent_series " CON " in " ds_file
    else if (val[OBS] !~ /^[0-9]+$/) print "P6=FALSE reason=non_integer_sample " OBS "='" val[OBS] "'"
    else if (val[CON] !~ /^[0-9]+$/) print "P6=FALSE reason=non_integer_sample " CON "='" val[CON] "'"
    else if (val[OBS] + 0 != val[CON] + 0) printf("P6=FALSE reason=gap removed_refs_observed_total=%s considered_total=%s gap=%d\n", val[OBS], val[CON], val[OBS] - val[CON])
    else printf("P6=TRUE removed_refs_observed_total=%s considered_total=%s gap=0\n", val[OBS], val[CON])
    # P7
    if (!(CAN in seen)) print "P7=FALSE reason=absent_series " CAN " in " ds_file
    else if (val[CAN] !~ /^[0-9]+$/) print "P7=FALSE reason=non_integer_sample " CAN "='" val[CAN] "'"
    else if (val[CAN] + 0 != 0) print "P7=FALSE reason=restored_cancelled_total=" val[CAN]
    else print "P7=TRUE restored_cancelled_total=0"
    printf("windows=%d settlement_rows=%d zero_return_removal_rows=%d scrapes=%d\n", nw, ns, zero_rows + 0, nf)
}
```

```bash
LC_ALL=C awk -f spec366-p67.awk -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"
```

### P6's arithmetic, pre-registered (not merely recorded in §3)

A prediction written after the data is not one. Three literals, before the run:

1. **Predicted gap on the decision scrape: `0`** — `removed_refs_observed_total == considered_total`.
2. **The reference gap this spec fixes: exactly `2,000`.** In the committed SPEC-365 cell the last
   CSV row reads `removed_refs_observed_total=27000` against `considered_total=25000`, and that gap
   is precisely the two `NO_SETTLEMENT` epochs (10 and 27) at `refs_returned=1000` each — the
   cancellation this spec removes. P6 is therefore a measurement with a known failing value, not a
   tautology.
3. **Predicted `refs_returned=0` rows: `0`.** The committed SPEC-365 log has none (all 27 `removal`
   rows read 1,000), which makes the zero-return exception **defensive**. §3 must therefore state
   "no zero-return epoch observed" **explicitly**, so an empty observation list is never mistaken
   for an unparsed field.

### Executability check (against SPEC-365's committed artifacts — not this spec's data)

Every command above was run, at authoring time, against `spec365-conj900.*` and against synthetic
scrape directories, to prove §2 is executable as written rather than plausible as written. No
SPEC-366 artifact exists or was produced. Observed:

| command | on the SPEC-365 artifacts | reads as |
|---|---|---|
| P1 | `P1=FALSE reason=reconciliation=SPLIT split_epochs=10,27` | correct: 365 was SPLIT |
| P2 | `P2=FALSE reason=split_epochs=10,27` | correct |
| P3 | `P3=FALSE reason=verdict_O2` | correct: 365 was O2 |
| P5 | `P5=FALSE reason=restored_cancelled clause: epoch 2: restored_cancelled ABSENT; …; unsettled epochs 10,27 (the carve-out admits at most one)`; `P5-observed: removal_rows=27 settlement_rows=25 unsettled=10,27`; `P5-zero-return: none observed` | correct on both clauses, and it demonstrates fail-closed on an ABSENT field: the SPEC-365 binary had no `restored_cancelled` on the settlement row, which this spec's binary adds (`crdt.rs:1651`) |
| P6/P7 window logic | `windows=27 settlement_rows=25 zero_return_removal_rows=0`; a scrape stamped inside epoch 2's window and one after epoch 10's never-closed window are both rejected; the last qualifying stamp is selected | correct: epochs 10 and 27 leave OPEN windows, so every later scrape is excluded |
| P6/P7 fail-closed | empty file ⇒ `empty_scrape`; missing series ⇒ `absent_series <name>`; `2.7e4` ⇒ `non_integer_sample`; `restored_cancelled_total=3` ⇒ `P7=FALSE reason=restored_cancelled_total=3`; empty directory ⇒ `no_persisted_scrape_in <dir>`; no qualifying scrape ⇒ `DECISION_SCRAPE=OPEN` + two FALSE lines | every branch names its reason |

### FALSE branch, pre-registered

If any of P1–P3, P5–P7 is FALSE, the executor STOPS and reports the predicate, its literal value
and its transport. It does **not** re-run the cell, re-tune a bound, widen a carve-out or edit a
predicate. The conductor decides what the FALSE means and what runs next.

## APPEND-ONLY BELOW

## §3 — Executed record

### Attempt 1 — INVALID (binary provenance). Consumes no predicate verdict.

The pre-registered predicates were never evaluated against this spec's binary, so the FALSE values
below say nothing about the fix. Recorded here because a discarded measurement that leaves no trace
is indistinguishable from one that was never taken.

**The binary the cell actually ran** — `target/release/topgun-server`:

- `sha256 = 1078e168f5a7b0894924d025d567339f626d55019ca805332f93b562507e76d9`
- mtime `2026-09-12T13:16:26Z`
- does **not** contain `topgun_or_prune_restored_cancelled_total`

**Four-binary control table.** The counter string is the discriminator: this branch emits it, no
earlier source does.

| binary | source | counter | sha256 |
|---|---|---|---|
| `target/release/topgun-server` (what the cell ran) | — | **absent** | `1078e168…76d9` |
| matrix worktree build | branch `c6dd1efa` | present | `96244aa2…1ef8` |
| AC 14 branch-side build | branch | present | `6b604cbe…b324` |
| AC 14 pin-side build | pin `550936dc` | absent | `53bf4adc…2851` |

Branch source demonstrably produces the counter, so the fault was the binary, not the code.

**Observed, as observed** (not predicate verdicts):

```
READOUT: O2; retained_closed_epochs=1; reconciliation=SPLIT   (split_epochs=23,25)
last-row metric check: removed_refs_observed_total=28000 considered_total=26000 MISMATCH
harness exit 1, finishedReason = tombstone-byte growth slope 8497.0 bytes/h exceeds 512.0 bytes/h
P1=FALSE reason=reconciliation=SPLIT split_epochs=23,25
P2=FALSE reason=split_epochs=23,25
P3=FALSE reason=verdict_O2
P5=FALSE reason=restored_cancelled clause: epoch 2..29: restored_cancelled ABSENT; unsettled epochs 23,25
P5-observed: removal_rows=28 settlement_rows=26 unsettled=23,25
P5-zero-return: none observed
DECISION_SCRAPE=2026-09-12T14:04:00Z.txt
P6=TRUE removed_refs_observed_total=19000 considered_total=19000 gap=0
P7=FALSE reason=absent_series topgun_or_prune_restored_cancelled_total in 2026-09-12T14:04:00Z.txt
windows=28 settlement_rows=26 zero_return_removal_rows=0 scrapes=16
```

`restored_cancelled` absent from **every** settlement row, and the counter absent from the scrape
while the other 76 `topgun_or_prune_*` series were present, is impossible for a branch binary — the
const is in `PRUNE_COUNTER_NAMES`, eagerly touched so it renders at 0 from the first scrape,
incremented in `observe_pass`, and emitted at `crdt.rs:1651`.

**Cause.** An earlier load-harness experiment shared one `CARGO_TARGET_DIR` between a pin worktree
and the main checkout. Cargo gave both source paths the same metadata hash, so the pin build landed
in `target/release/` and a later `cargo build` judged it fresh. The cell's own build produced a fresh
soak bench (13:53:58Z) and left the server at 13:16:26Z. The harness runs the server **out of
process**, so a stale binary silently decided the whole cell.

**Disposition.** Artifacts moved to `.specflow/artifacts/spec366-conj900-attempt1-INVALID/` (local,
never committed); this block is their only committed record. Difference item 5 exists so this class
of failure refuses before `T0` instead of producing a plausible readout.

### Attempt 2 — VALID measurement. P1, P2, P5, P6, P7 TRUE; **P3 FALSE**.

**Provenance (difference item 5, the reason this attempt can be believed at all).** The assertion ran
after the runner's own build and before `T0`, and passed:

```
provenance: server sha256=ac34c2d7e39bca3e0ce4aabce7122833658c660a2e30f455ba9d90808bc0b8f7 built=2026-09-12T15:26:48Z run_start=2026-09-12T15:23:11Z topgun_or_prune_restored_cancelled_total=present
```

That line is the console log's first line (clause c) and the same sha256 is on the matrix. The binary
was built at `15:26:48Z` against a recorded run start of `15:23:11Z`, so clause (b) held on real data;
it contains the counter, so clause (a) held. Its sha differs from attempt 1's stale `1078e168…`.
Matrix also records `code freeze: a18e09de` and `code freeze diff (.rs): EMPTY (asserted before the
build)`.

**Readout**

```
READOUT: O2; retained_closed_epochs=1; reconciliation=RECONCILED
```

Re-running the UNCHANGED `spec365-readout.sh spec366-conj900` reproduces it **byte-for-byte**
(AC 17's regenerability clause, checked).

**Predicates, run verbatim from the committed programs**

```
P1=TRUE
P2=TRUE
P3=FALSE reason=verdict_O2
P5=TRUE
P5-observed: removal_rows=32 settlement_rows=32 unsettled=none
P5-zero-return: none observed
DECISION_SCRAPE=2026-09-12T15:38:51Z.txt
P6=TRUE removed_refs_observed_total=25000 considered_total=25000 gap=0
P7=TRUE restored_cancelled_total=0
windows=32 settlement_rows=32 zero_return_removal_rows=0 scrapes=15
```

Transports: P1/P2/P3 over `spec366-conj900.readout.txt`; P5 over the ANSI-stripped
`spec366-conj900.harness-console.log` via `spec366-p5.awk`; P6/P7 over the decision scrape via
`spec366-p67.awk`. No fail-closed reason was emitted — every series was present and integral.

**Zero-return observation, stated explicitly** so an empty list is never mistaken for an unparsed
field: **no zero-return epoch was observed.** `P5-zero-return: none observed`, and the P6/P7 line
reads `zero_return_removal_rows=0`. §2 predicted exactly this, which makes item 2a's exclusion
defensive rather than load-bearing here.

**What the fix did, against the SPEC-365 baseline.** §2 pre-registered the reference gap as exactly
2,000 (`removed_refs_observed_total=27000` vs `considered_total=25000`, the two `NO_SETTLEMENT`
epochs 10 and 27 at 1,000 refs each). On the decision scrape the gap is now **0**, `split_epochs` is
empty, and SectionC reports **every one of the 32 exited epochs as RECONCILED**:

```
totals: passes=32 refs_at_entry=32000 refs_returned=32000 bytes_returned=687709
        considered=32000 dropped=32000 matched_nothing=0 absent=0 restored_sum=0 bytes_freed=687709
```

No epoch was left unsettled, and `restored_sum=0` — no ref took a restore exit at all, cancelled or
otherwise, which is the expected shape when no pass is cut short.

**CSV last-row metric check (recorded; NOT P6's transport).**

```
last-row metric check: removed_refs_observed_total=30000 considered_total=29000 MISMATCH
```

This is the pre-registered in-flight artifact: `removed_refs_observed` is credited at drain and
`considered` at the end of the pass, so a CSV row sampled mid-pass reads MISMATCH on a correct
binary. It is why P6 is decided on the decision scrape instead, where the gap is 0.

**Harness exit attribution.** `exit 1`, `passed: false`,
`finishedReason = tombstone-byte growth slope 795.9 bytes/h exceeds 512.0 bytes/h (total growth
49214 bytes over 180 samples, last-half window 446s)`. Recorded as attribution, not as a predicate:
the runner states plainly that a non-zero harness exit is not automatically a failed
characterization. For contrast, attempt 1's slope was 8497.0 bytes/h — the same gate, ~10.7× lower.

**A7 — pass latency and inter-exit interval (RECORDED, NOT GATED).** §2 pins no command for this, so
the extraction is not a pinned predicate; the numbers below come from the console log's RFC 3339
prefixes in the same time domain the pinned predicates use.

| metric | value |
|---|---|
| settled epochs | 32 (every exited epoch) |
| pass latency, mean | 4.3 s |
| pass latency, max | 5 s |
| inter-exit interval, mean | 27.0 s |
| inter-exit interval, min | 18 s |
| **max(latency) / min(inter-exit)** | **0.278** |

Per-epoch latency ranged 2–5 s across epochs 2–33, with no epoch left open. A7 asks whether one
serial task keeps up: at 0.278 the ratio is well inside parity, so the single prune task is not the
bottleneck at this cadence. No ratio > 1, so there is nothing to report as a TODO-634 finding and
nothing to tune.

**P3 is FALSE, and this is the pre-registered FALSE branch.** The predicate required verdict `O3`;
the readout gives `O2` with `retained_closed_epochs=1`. Per §2's FALSE branch the executor reports
the predicate, its literal value and its transport, and does **not** re-run the cell, re-tune a
bound, widen a carve-out or edit a predicate. What `O2` means here — and whether one retained closed
epoch is expected at this cell size — is the conductor's adjudication, not the executor's.

**Adjudication (conductor rulings v6 R1).** P3 FALSE is a **pre-registration error, not a fix
failure**; it is recorded as FALSE and closed by adjudication. `O3` requires
`durability_only + both ≤ claim_only`. In this cell `claim_only = 0` (claim lag 0–1) while the durable
watermark lags 1–3 epochs (`durable_watermark_lag max=3 last=2`), so at ANY snapshot the newest closed
epoch (here 32) is durability-held: `durability_only = 1 > 0` ⇒ `O2` by the readout's first-match
rule. SPEC-365's cell read the same shape (epoch 29 `durability_only`, O2). Nothing in this spec's
code moves the durable watermark, so `O3` was never reachable by the fix — the prediction was about
the instrument's steady state, and it was wrong. No re-tuning, no re-run, no edit to §2. The fix's
claim rests on P1/P2/P5/P6/P7 and on the pre-registered gap 2,000 → 0.

**Waiting discipline:** 0 no-op polls; liveness confirmations only, as recorded in the STOP 3b report.
