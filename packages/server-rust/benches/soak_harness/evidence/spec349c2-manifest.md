# SPEC-349c2 — plateau evidence manifest

The recorded, re-derivable record of what SPEC-349c1b's `WalOp::OrDelta` emitter did to the disk and
RSS growth curves of a live 60-minute soak, and of the TODO-612 `flush_key` reachability
determination that gates the 72h soak's "covers the Remove idempotency class" claim.

**Direction of the dependency, stated once so it is not read the other way round: this manifest
RECORDS slopes and promotes NOTHING. SPEC-348 DERIVES its hard-gate numbers FROM these recorded
slopes.** No threshold in this file is a gate, and no number here may be cited as one.

Every figure below is transcribed from a committed artifact in this directory, and every derived
figure (combined SE, effect size, ratios, per-op rates) was recomputed from those artifacts while
this manifest was written. Where a figure was recomputed and disagreed with a previously circulated
value, **the artifact wins** and the disagreement is recorded in §12.

## Artifacts this manifest reports on (all committed, all in this directory)

| File | Produced by |
|------|-------------|
| `spec349c2-emitter-on.csv`, `spec349c2-emitter-off.csv` | the runner's per-minute sampler (61 rows each, 0 s … 3600 s) |
| `spec349c2-emitter-{on,off}.soak.json` | harness `--json-output` |
| `spec349c2-emitter-{on,off}.mechanism.json` | harness `--mechanism-report` (the harness writes `<base>.soak.mechanism.json`; the runner renames it to the ledgered name after the run) |
| `spec349c2-emitter-{on,off}.progress.jsonl` | harness `--progress-output` (coarse independent RSS witness; no disk field) |
| `spec349c2-emitter-{on,off}.harness-console.log` | the harness's own stdout, captured by the runner — the **only** artifact of these two runs carrying the confirm-apply cursor (see §7.3) |
| `spec349c2-plateau.sh` | the runner — **the matrix is executed by this file, not transcribed by hand** |
| `spec349c2-fit.awk` | the post-hoc OLS fit; every slope below is `awk -v col=<series> -v window=<last_half\|full> -f spec349c2-fit.awk <run>.csv` |
| `spec349c2-manifest.md` | this file |

---

## 1. Env matrix as actually run

**Binary.** Commit **`d6922f08`** (`feat(sf-349c2): execute the plateau matrix instead of
transcribing it`, 2026-08-01 17:03:52 +0300), release profile, `cargo build --release --bin
topgun-server --bench soak_harness`. The ON run started ~17:06 and the OFF run ended 19:08; the next
commit (`aae8cd44`, 19:09:59) added only the evidence files. Tree state at run time: **clean**
(attested by the run record — see §12 finding D2 for the observability limit on this one field, and
for the fix that makes it artifact-observable on future runs).

**Attestation of the source under measurement — restated precisely.** Earlier drafts of this
manifest claimed `git diff --name-only d6922f08..HEAD -- '*.rs'` returns **nothing**. That claim was
true when written and is **no longer true**: Review v1's C3 required the harness to persist the
diagnostics that decide its own verdict, and that fix is `.rs`. The claim is therefore replaced with
an enumerated one, which is the form a reader can actually check:

| `.rs` changed after `d6922f08` | Extent | Can it have moved a recorded number? |
|---|---|---|
| `benches/soak_harness/report.rs` | Three additive report structs + four additive fields on `SoakReport`/`ProgressSnapshot` | **No** — serialization-only. Nothing read by a sampler, an assessment, or the write path. |
| `benches/soak_harness/main.rs` | Populates those fields from counters the summary already loaded; deletes a local struct made redundant by them | **No** — same counters, same values, one additional consumer. |
| `src/storage/datastores/write_behind.rs` | One `///` block (the TODO-628 pointer) | **No** — doc-comment; not compiled into semantics. |
| `tests/soak_wal_census.rs` | Additive `#[test]` fns asserting the fields above against a fixture | **No** — a `tests/` integration target. It is linked into neither the measured `topgun-server` binary nor the `soak_harness` bench, so no code path under measurement can reach it; it only reads. |

**The table must agree with the command.** `git diff --name-only d6922f08..HEAD -- '*.rs'` returns
**four** paths, and all four are rows above. An earlier revision listed three — omitting
`tests/soak_wal_census.rs` — which left a reader running the named command with a count the table did
not match. Since the enumeration's whole purpose is that it is checkable, an incomplete one is worse
than no enumeration at all, and the count is called out here so the check is a comparison rather
than a scan.

**Scope of this attestation, stated so it is not over-read: it covers `.rs` only.** One non-`.rs`
file in the measurement chain also changed after the runs — `spec349c2-fit.awk`, which computes every
slope in §2. Two changes landed there (`int()` truncation on the gauge scrape; `rows_total` renamed
`rows_used`), and both are **fit-neutral**: re-running the committed script over the committed CSVs
reproduces every §2.1 and §2.3 figure to the digit. That is a verified reproduction, not an
argument — anyone can re-run it. The runner (`spec349c2-plateau.sh`) also changed, but it produces no
figure in this manifest; it produces future runs.

So: **the committed artifacts in this directory were produced by `d6922f08`, and every `.rs` change
since is additive telemetry, documentation or test that cannot alter the measured path.** The weaker,
enumerated claim is the honest one; the reader is not asked to trust that the diff is empty, only to
check that each hunk is emission-only. Re-running either arm on today's HEAD would produce the same
series plus the fields these runs' JSON is missing — fields whose values, for these two runs, are
recoverable from the committed console logs (§7.3) rather than back-filled.

**Host / OS.** `Darwin MacBookPro 25.5.0` (Darwin Kernel 25.5.0, `RELEASE_ARM64_T6000`, arm64),
macOS 26.5.2 (build 25F84), `MacBookPro18,2`, 10 CPUs, 32 GiB RAM. Toolchain
`rustc 1.93.1 (01f6ddf75 2026-02-11)`.

**Data dirs (AC4(c)) — distinct, each empty at start.**

| Run | `--data-dir` | Empty at start |
|-----|--------------|----------------|
| ON  | `<repo>/target/spec349c2-on-data`  | yes — the runner **refuses to start** on a non-empty dir (`spec349c2-plateau.sh` §4: `FATAL: data dir is NOT empty`), and refuses to overwrite a pre-existing artifact |
| OFF | `<repo>/target/spec349c2-off-data` | yes — same fail-closed check, separate directory |

The console log beside each run is at `<data-dir>.meta/harness-console.log`. Its first line is the
harness's own matrix echo, identical in both runs except for nothing:

```
duration=3600s churn_clients=6 keyspace=200 crash_interval=None steady_interval=300s wal_fsync=batched or_churn=true
```

**Harness CLI flags, as literals in the committed runner (`spec349c2-plateau.sh` §1).** These are the
knobs AC1 lists as *observable only via the committed runner script + manifest* — no report struct
carries them:

| Flag | Value | Flag | Value |
|------|-------|------|-------|
| `--duration` | 3600 | `--crash-interval` | **0** (parses to `None`: no `kill -9` during the run) |
| `--churn-clients` | 6 | `--steady-interval` | 300 s |
| `--keyspace` | 200 | `--quiesce` | 3 s |
| `--or-churn` | true | `--mem-sample-interval` | 5 s |
| `--or-keyspace` | 48 (`48 % 6 == 0`) | `--mem-min-growth-mb` | 1000000 (NEUTRALIZE) |
| `--or-every` | 5 | `--mem-threshold-mb-per-hour` | 1000000 (NEUTRALIZE) |
| `--write-interval-ms` | 20 | `--mem-ceiling-mb` | 1000000 (NEUTRALIZE) |
| `--writes-per-life` | 200 | `--wal-fsync` | `batched` |
| `--offline-keys` | 3 | `--server-port` | 47349 (fixed) |
| `--confirm-interval` | 2 s | `--mechanism-report` | ON |
| CSV cadence | 60 s | `--json-output` / `--progress-output` | into this evidence dir |

All three memory-gate neutralizers are set, so `memory.passed == true` in both runs means only *the
neutralizers were set* — it is explicitly **not** load-bearing (R2.2's non-load-bearing list).

**Environment, as the runner disciplines it (`spec349c2-plateau.sh` §3).** Actively `unset` rather
than merely not set, because the harness spawns the child **without** `env_clear()` and an operator's
stray export would silently change what the runs measure: `TOPGUN_EPOCH_WIDTH`,
`TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS`, `TOPGUN_WRITEBEHIND_BATCH_SIZE`, `SOAK_SERVER_BINARY`,
`SOAK_SERVER_LOG`, `TOPGUN_SOAK_GRACEFUL_SHUTDOWN`, `TOPGUN_MAX_RAM_MB`, `TOPGUN_EVICTION_HIGH_PCT`,
`TOPGUN_EVICTION_LOW_PCT`, `TOPGUN_EVICTION_INTERVAL_MS`.

`TOPGUN_WAL_FSYNC_POLICY` is deliberately **not** managed by the runner: the harness overwrites it on
the child unconditionally from `--wal-fsync` (`process.rs:198`), so exporting it does nothing at all.
The policy comes from the flag and is recorded in `soak.json` as `walFsync` (§5).

**The one input that differs between the runs:**

| Run | `TOPGUN_OR_DELTA_WAL` |
|-----|------------------------|
| ON  | **UNSET** (the shipped default: armed) |
| OFF | **`false`** (kill-switch: full-snapshot framing on every OR write) |

Set by the harness on the child, overwriting whatever the shell had: `STORAGE_BACKEND=redb`,
`TOPGUN_REDB_PATH=<data-dir>/topgun.redb`, `TOPGUN_WAL_DIR=<data-dir>/wal`,
`TOPGUN_WAL_FSYNC_POLICY=batched`, `TOPGUN_BIND_ADDR=127.0.0.1`, `JWT_SECRET=<harness constant>`,
`TOPGUN_JOURNAL_ENABLED=true`, `RUST_BACKTRACE=1`, `RUST_LOG=warn`, and — because they were left
unset in the shell — the harness's own write-behind cadence `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS=100`
/ `TOPGUN_WRITEBEHIND_BATCH_SIZE=5000` (`process.rs:212-221`). See §9.

**Run identity, from each `soak.json`:** ON `durationSecsActual=3600`, OFF `3602`; both
`churnClients=6`, `keyspace=200`, `crashes=0`, `writeErrors=0`, `convergenceFailures=[]`,
`recoveryFailures=[]`, `panicReport=null`, `steadyCheckpoints=11`, `recoveryCheckpoints=0`.
`totalWrites`: ON 555,743, OFF 559,824.

---

## 2. Both runs' slopes, with standard errors

### 2.1 The recorded verdicts — post-hoc last-half OLS (R2.2)

Window: rows `[floor(n/2) .. n-1]` of each 61-row CSV, i.e. **n = 31 samples spanning 1800 s →
3600 s**. `x` in hours, so the slope is MB/h directly. `se_combined = sqrt(se_ON² + se_OFF²)`
(root-sum-of-squares, the independent-samples rule; the additive rule is **not** used). A BENT or
WORSE verdict additionally requires `|slope_ON − slope_OFF| ≥ 10 %` of `|slope_OFF|`.

| Series | ON (MB/h) | OFF (MB/h) | `se_comb` | \|diff\| | effect size | verdict |
|--------|-----------|------------|-----------|----------|-------------|---------|
| `disk_total_mb` | 90.750 ± 4.066 | 58749.826 ± 1483.515 | 1483.520 | 58659.075 | 99.85 % | **BENT** |
| `wal_mb` | 82.818 ± 1.152 | 58739.633 ± 1482.156 | 1482.157 | 58656.816 | 99.86 % | **BENT** |
| `redb_mb` | 7.937 ± 3.457 | 10.705 ± 3.843 | 5.169 | 2.768 | 25.86 % | **UNMOVED** — effect size clears 10 %, but `\|diff\| = 2.768 < se_comb = 5.169`, so the two fits do **not** separate. Both conjuncts are required; this one fails the SE test |
| `rss_mb` | 8461.643 ± 122.942 | 9976.064 ± 104.575 | 161.402 | 1514.421 | 15.18 % | **BENT** (see §6 — BENT is not a plateau) |

**AC2(c), corpus split.** `wal_mb` and `redb_mb` are fit **independently**, so a
shrunk-frame-but-moved-retention regression cannot look good by relocating growth. It does not: the
BENT `disk_total_mb` verdict is carried by `wal_mb` (82.8 vs 58 739.6 MB/h), while `redb_mb` is
UNMOVED at ~8–11 MB/h in **both** runs. Growth was removed, not moved — the ON run's `redb_mb` slope
(7.937) is *below*, not above, the OFF run's (10.705).

**AC2(b) determination for `disk_total_mb`: BENT.** `slope_ON` is below `slope_OFF` by 58 659.075 MB/h
= **39.5 × `se_comb`** and by **99.85 %** of `|slope_OFF|` — both conjuncts hold.

### 2.2 The autocorrelation caveat (R2.2, verbatim, one line)

> OLS SE on an autocorrelated series is a lower bound on the true uncertainty; the ≥10 % effect-size
> floor, not the SE, is what carries the discrimination claim.

### 2.3 AC5 — cross-instrument reproduction (full-window fits over the same CSVs)

The FULL-window fit is the only statistic the harness and the CSV both compute, so it isolates *are
these two instruments watching the same process*. Tolerance `max(±10 %, ±1 MB/h)`.

| Check | CSV full-window fit | Harness key | Harness value | Δ | inside tolerance |
|-------|--------------------|-------------|---------------|---|------------------|
| ON `rss_mb` | 6063.052 | `memory.slope_mb_per_hour` (**snake_case**) | 6058.488 | 0.08 % | yes |
| OFF `rss_mb` | 7630.165 | `memory.slope_mb_per_hour` | 7631.864 | 0.02 % | yes |
| ON `disk_total_mb` | 90.406 | `q3DiskSlopeMbPerHour` (**camelCase**) | 89.592 | 0.91 % | yes |
| OFF `disk_total_mb` | 33295.320 | `q3DiskSlopeMbPerHour` | 33144.658 | 0.45 % | yes |

**All four are inside tolerance, so NO cadence/origin re-fit was needed.** Recorded anyway, because a
future near-tolerance divergence must be diagnosed rather than asserted away: the CSV is sampled every
60 s while the harness samples RSS every 5 s, and the CSV's origin is *server-ready* while the
harness's `sampler_start` is *later* (after the churn clients spawn) — so the CSV carries at most one
extra leading sample and the origin term predicts `slope_CSV ≳ slope_harness`. A divergence with the
opposite sign cannot be explained by the origin term at all.

**The two key names are in different cases and that is the emitted truth, not a typo.**
`MemoryReport` carries no `#[serde(rename_all = "camelCase")]` and an outer `rename_all` does not
descend into a nested struct, so `soak.json` emits `memory.slope_mb_per_hour`; `MechanismReport` does
carry the attribute, so `mechanism.json` emits `q3DiskSlopeMbPerHour`. The derive was deliberately
**not** added (it would be a drive-by schema break inside a measurement spec).

### 2.4 Inherited disposition — the ARMED per-op cost is UNMEASURED

SPEC-349c1b recorded **AC19 disposition (c): DEFERRED, owner TODO-621** (the load harness drives LWW
`PUT` only and cannot execute the OR write path at all). Therefore, stated in one line as that
disposition requires:

> **The armed per-op hot-path cost of the emitter is UNMEASURED at this measurement time; owner
> TODO-621.**

**The slopes above MUST NOT be read as pricing it.** A 60-minute aggregate growth measurement over a
settled binary cannot resolve a per-op hot-path cost, and citing a BENT disk verdict as evidence that
the emitter is cheap per op would be exactly the blind-instrument substitution TODO-621 exists to end.

---

## 3. Frame-kind census — counts AND per-kind bytes, both runs

Read from `<data-dir>/wal` at end of run by the relocated census (`report.rs`), emitted into each
run's `mechanism.json`.

| Field | ON | OFF |
|-------|-----|-----|
| **raw pair `(orDeltaFrames, orSnapshotFrames)`** | **(157134, 0)** | **(0, 175617)** |
| `orDeltaFrames` | 157,134 | 0 |
| `orDeltaBytesTotal` | 22,220,391 B | 0 B |
| `orDeltaBytesMean` | **141.410 B** | 0.0 B |
| `orDeltaBytesMax` | 209 B | 0 B |
| `orSnapshotFrames` | 0 | 175,617 |
| `orSnapshotBytesTotal` | 0 B | 32,565,195,476 B |
| `orSnapshotBytesMean` | 0.0 B | **185,433.047 B** |
| `orSnapshotBytesMax` | 0 B | 343,179 B |
| `orFrames` (combined) | 157,134 | 175,617 |
| `lwwFrames` | 54,549 | 64,679 |
| `removeFrames` | 0 | 0 |
| `walSegmentFiles` | 9,579 | 11,137 |
| `q1OrFrameBytesMeanEarly` / `…Late` (mixed bucket, derived) | 141.297 / 141.524 | 130,353.643 / 240,511.824 |
| `q1LwwFrameBytesMean` | 219.866 | 219.788 |

### 3.1 AC2(a) — three separated populations

- **(i) DELTA-FRAME MEAN — the direct claim, and the only normative one.** ON `WalOp::OrDelta` bucket
  **alone**: mean **141.41 B ≤ 500 B** → **HOLDS**. The bucket is non-empty (157,134 frames), so this
  is not a vacuous pass.
- **(ii) RESIDUAL SHARE `p` — CONSUMED from §4, not re-derived here.** `p` = **0 / 157134 = 0.0 %**.
  See §4 for its population definition, mix descriptor and sampling caveat. This is the *only*
  measurement of the residual anywhere in this spec's artifacts.
- **(iii) MIXED OFF/ON MEAN RATIO — DERIVED and NON-NORMATIVE; it gates nothing.**
  `q1OrFrameBytesMeanLate` OFF ÷ ON = 240 511.824 ÷ 141.524 = **1699.5 ×**. Predicted
  `1/(p + (1−p)·δ)` with `p = 0` and `δ = 141.410 / 185433.047 = 0.000763` gives **~1311 ×**. Same
  order, consistent with (i) and (ii); the gap is the early/late split (the OFF snapshot mean rises
  over the run, 130 354 B early → 240 512 B late, while the ON delta mean is flat). **No AC is
  satisfied or refuted by this number.**

### 3.2 Amortized per-op WAL byte rate

| Measure | ON | OFF | ratio |
|---------|-----|-----|-------|
| `q4OrWalBytesTotal / orFrames` | **141.4 B** per OR frame | **185,433.0 B** per OR frame | **1311 × shrink** |
| `q4OrWalBytesTotal / totalWrites` | 40.0 B per write (22,220,391 / 555,743) | 58,170.4 B per write (32,565,195,476 / 559,824) | — |

### 3.3 AC4(a) — the census's three-verdict determination

| Run | requirement | observed | verdict |
|-----|-------------|----------|---------|
| ON | `orDeltaFrames > 0` | 157,134 | **PASS** |
| OFF | `orDeltaFrames == 0` **AND** `orSnapshotFrames > 0` | 0 and 175,617 | **PASS, with its positive control satisfied** |

Neither run is INCONCLUSIVE: `orDeltaFrames + orSnapshotFrames > 0` in both. The positive control is
load-bearing precisely because the census reads the **retained** corpus after GC has unlinked applied
segments — without `orSnapshotFrames > 0` the OFF run's zero would be satisfied by a corpus holding
no OR frames at all, and would prove nothing.

### 3.4 AC4(b) — discrimination CONFIRMED, so AC2/AC3 are not void

On `disk_total_mb` the ON and OFF last-half slopes separate by **39.5 × `se_comb`** and by **99.85 %**
of `|slope_OFF|` — both R2.2 tests — **and** AC2(a)(i)'s delta-frame mean claim holds (141.41 B ≤
500 B). The instrument discriminates. **AC2 and AC3 are therefore NOT void**, and KL1 holds: the
ON/OFF difference is a measurement of the emitter, not of this box on this afternoon.

---

## 4. Residual share `p`, with its mix descriptor

**`p`'s definition of record is the wide one, and this sentence travels with the number everywhere:**

> *`p` is the share of retained OR-side frames that are full snapshots, from any cause.*

| Quantity | Value |
|---|---|
| **`p`** | **0 / 157,134 = 0.0 %** |
| numerator | OR-side full-snapshot `WalOp::Store` frames retained = **0** |
| denominator | all OR-side frames retained (`orDeltaFrames + orSnapshotFrames`) = **157,134** |
| writers | 6 churn clients |
| OR keys | 48 (`--or-keyspace 48`, `48 % 6 == 0`, single writer per key) |
| epoch width | **1000** (the production default; `soak.json` `epochWidth`) |
| op-generation rule | unique tags per client: `or_add(tag)` then `or_remove(same tag)`, one OR op every `--or-every 5` writes at `--write-interval-ms 20`. **Injected no-effect rate: `0 %`** — this driver injects no re-add-of-a-tombstoned-tag and no duplicate remove |
| run identity (in place of the seed the live harness does not have) | emitter **ON**; data dir `<repo>/target/spec349c2-on-data`; binary commit `d6922f08` |

**Why a run identity rather than a seed:** the soak harness has no `--seed` knob (`KNOWN_FLAGS`
contains none), its churn clients derive tags from their own counters against wall-clock-paced
writes, and two live 60-minute runs cannot be byte-identical. The reproducibility anchor is the OFF
run's census, which shows `orDeltaFrames == 0` and therefore a 100 % snapshot share by construction.

**⚠ RETAINED-CORPUS SAMPLING CAVEAT — record this beside `p` wherever `p` travels.** The census scans
`<data-dir>/wal` at end of run, *after* GC has unlinked applied segments, so the denominator is the
**surviving tail**, not every frame the run wrote. Under this shape the surviving residual sources are
prune-shaped, and prune-shaped events cluster in time rather than arriving uniformly, so the retained
tail is a **biased** sample of the run and `p` can land either side of the run-wide share depending on
where the tail falls relative to the last prune. **`p` is an UPPER BOUND on TODO-619's residual, not a
measurement of it, and it MUST NOT be treated as a run-wide rate.** Narrowing it is TODO-619's
business.

**SPEC-349c1a's 12.37 % is SHAPE-INVALID here and was not consumed** — it was measured over a driver
injecting two no-effect sources at 20 % of rounds, which this spec's churn client never drives.
`p = 0.0 %` above is a re-measurement over this spec's own pinned shape, on the very run whose slopes
are reported.

---

## 5. `walFsync` and epoch width, copied from each run's `soak.json`

| Field | ON | OFF |
|-------|-----|-----|
| `walFsync` | `"batched"` | `"batched"` |
| `epochWidth` | `1000` | `1000` |

Both runs agree with each other and with the pinned matrix. `walFsync` is a **direct readout** of the
value set on the child (`Config.wal_fsync` → `process.rs:198`), not a proxy.

**W4(a)'s three validity conditions for the epoch-width field** (it is derived in the *harness*
process from the *harness's* environment by a re-implementation of the bin's rule — it is not read
back from the child, so it is a valid proxy only while all three hold):

1. **`process.rs` never sets `TOPGUN_EPOCH_WIDTH` and never calls `env_clear()`** — so the child
   inherits the harness's value verbatim and harness-env == child-env for this variable.
   **Re-verified now, by grep, at this manifest's HEAD:**
   - `grep -rn "TOPGUN_EPOCH_WIDTH\|env_clear" benches/soak_harness/process.rs` → **no matches**
     (exit 1). Condition 1 **HOLDS**.
   - Recorded honestly, because a naive grep over the whole directory does **not** come back empty:
     `TOPGUN_EPOCH_WIDTH` appears in `benches/soak_harness/report.rs:57` (a doc comment), `:132` and
     `:143` (`effective_epoch_width()` **reading** the var — the derivation itself), and in
     `evidence/spec349c2-plateau.sh:149` (`unset`) and `:330` (an echo); `env_clear` appears once, in
     a comment at `evidence/spec349c2-plateau.sh:141`. **None of these sets the variable on the child
     or clears the child's environment**, which is what the condition is about. The whole-directory
     grep is the wrong instrument for this condition; `process.rs` is the right one.
2. **`SOAK_SERVER_BINARY` is UNSET**, so the child is the `CARGO_BIN_EXE_topgun-server` of the same
   build rather than an arbitrary prebuilt binary with a possibly different default. The runner
   actively `unset`s it (`spec349c2-plateau.sh:158`), and additionally surfaces both binaries' link
   times and warns if they are more than 600 s apart.
3. **The harness names `topgun_server::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH`** rather than a
   literal `1000` (`report.rs:146`, `:148`), so the harness's rule and the server's rule cannot drift
   apart at a future default change.

---

## 6. AC3 disposition — RSS

**Recorded disposition: BENT.** `slope_ON = 8461.643 ± 122.942` MB/h vs `slope_OFF = 9976.064 ±
104.575` MB/h; `|diff| = 1514.421` MB/h = **9.4 × `se_comb`** (161.402) and **15.18 %** of
`|slope_OFF|` — both R2.2 conjuncts hold, ON below OFF.

**⚠ BENT IS NOT A PLATEAU, and this manifest says so in those terms.** RSS still climbs at
**8461 MB/h with the emitter ON**. In absolute terms the ON run went **3 MB → 6090 MB in 60 minutes**
(CSV first/last `rss_mb`), against the OFF run's **3 MB → 7459 MB**. The emitter removed roughly 15 %
of the RSS growth rate and left the rest standing. Nothing here is a plateau, nothing here is a
plateau "in sight", and **claiming a plateau on this evidence is an explicit AC3 FAILURE**, not a
pass.

The three open contributors named beside this disposition, as AC3 requires:

- **TODO-590** — jemalloc.
- **TODO-591** — record-clone elimination.
- **TODO-593** — prune verification.

SPEC-347 already REFUTED the premise that shrinking the per-op allocation plateaus RSS: its finding
was that growth is **retention-dominant**. This measurement is consistent with that — the per-op WAL
allocation shrank 1311 × (§3.2) while RSS growth fell only 15 %.

---

## 7. The `passed`-verdict attribution (R2.2's reconciliation clause)

Both runs report **`passed: false`**, and both report it for the same reason. This is recorded here,
next to the slopes, rather than left in a JSON field nobody reads.

| Run | `passed` | cause, read from `finishedReason` | `pendingGates` |
|-----|----------|-----------------------------------|----------------|
| ON | `false` | **the tombstone-byte gate**: `tombstone-byte growth slope 248148.9 bytes/h exceeds 512.0 bytes/h` (total growth 209 550 B over 720 samples, last-half window 1797 s) | `disk growth slope 89.6 MB/h exceeds 50.0 MB/h — EXPECTED until TODO-566 bounds OR-Map tombstones (report-only, did NOT fail the run)` |
| OFF | `false` | **the tombstone-byte gate**: `tombstone-byte growth slope 283066.2 bytes/h exceeds 512.0 bytes/h` (total growth 248 713 B over 720 samples, last-half window 1796 s) | `disk growth slope 33144.7 MB/h exceeds 50.0 MB/h — EXPECTED until TODO-566 bounds OR-Map tombstones (report-only, did NOT fail the run)` |

The gate is SPEC-345's promoted byte-slope gate, **512 B/h**, deliberately left hard-gated for these
runs.

**Does it bear on any AC?** No: it gates the tombstone-byte corpus, a property none of AC1–AC5
asserts, so per R2.2's reconciliation clause it does **not** refute AC1–AC5 — the AC list and the
harness verdict answer different questions.

### 7.1 CLASSIFICATION: this is **(b) a genuine bound breach**, not a protocol artifact

"A finding" is not a disposition, so the finding is disposed of here. There are exactly two
candidate explanations for a hard gate that fires, and they are mutually exclusive:

- **(a) protocol artifact** — the gate presupposes conditions this protocol does not create, so it
  could not have passed regardless of the code's health. A gate that cannot pass under its own test
  protocol says nothing about the system.
- **(b) genuine bound breach** — every condition the gate presupposes is satisfied, the gate is
  live and meaningful, and the measured slope exceeds the bound.

**The verdict is (b).** (a) is refuted on all three of the conditions SPEC-345's gate presupposes.
**Every figure in the table below is re-readable from a committed artifact in this directory —
§7.3 gives the per-figure provenance, file by file and line by line, and it is the section to check
before relying on any number here.** (Earlier revisions of this manifest had one of these legs
resting on transcription; that is no longer the case, and §7.3 records the correction.)

| Precondition the gate presupposes | Observed | Refutes (a)? |
|---|---|---|
| A **tracked confirm-apply client** is in the protocol — without it the low-water-mark never advances and the epoch-scoped prune is never licensed, which is precisely the vacuous-gate trap SPEC-345 called out | The tracked client is driven **unconditionally**: `no_ack` defaults `false` (`main.rs`), the runner passes no `--no-ack`, and the call site is `if !cfg.no_ack`. It also demonstrably **worked**: `confirms=1727`, `confirmErrors=51` — errors are ~3 % of rounds and cannot account for a 485 × overshoot | ✅ |
| **Enough epochs cycle** at the pinned `epochWidth=1000` for prune eligibility to be reached inside the window | `lastConfirmedEpoch` advanced monotonically across all 11 checkpoints in **each** arm, reaching **113** at run end in both — **ON:** 11 → 21 → 30 → 39 → 48 → 57 → 66 → 75 → 84 → 94 → 103, final 113; **OFF:** 11 → 20 → 29 → 39 → 48 → 57 → 66 → 76 → 85 → 94 → 104, final 113. The last-half window (1797 s) therefore spans ~56 epochs — far past the ~2-epoch prune ramp, and the 120 s min-window guard is cleared with `samples=720` (not a blind monitor) | ✅ |
| The **LWM actually advances** — not merely that a client is present | The monotone cursor above IS the LWM advance, and it is corroborated **gauge-independently** by the post-run redb corpus scan, which sums the real on-disk tombstone corpus rather than reading the counter: ON 200,860 B, OFF 239,256 B. A stuck-gauge artifact would not reproduce in an independent instrument | ✅ |

**Why this matters beyond this spec.** `.specflow/archive/SPEC-345.md` recorded that the PASS is
demonstrable "with either `TOPGUN_EPOCH_WIDTH=100` + ≥15 min (done: −1707 B/h), **or ≥30–60 min at
the default width**". These are the first live 60-minute runs **at the default width** — the first
test of that second disjunct — and it fails by 485 ×. So exactly one of these is true, and which one
is not yet known:

1. The prune **regressed** since 2026-07-13, or
2. SPEC-345's PASS was only ever demonstrable at the non-production width 100, and the second
   disjunct was an unverified extrapolation.

**Deferral, in CLAUDE.md's required form** (this measurement spec does not fix it):

- **WHY deferral is acceptable here:** this spec is a *measurement* spec whose entire value is that
  the binary that produced the slopes is the binary on disk. Changing prune code to chase this
  finding would invalidate every number in §2–§4 and require re-running both 60-minute arms. The
  finding is also **not emitter-attributable** — it fires in BOTH arms at the same order of
  magnitude (248 k vs 283 k B/h; stated precisely rather than as "similar", the **OFF arm is 14.1 %
  HIGHER** than the ON arm — both are 485–553 × over the bound, so the gap does not bear on the
  classification, but the direction, emitter-OFF worse, is carried into TODO-630), and the
  census confirms `removeFrames == 0` in both runs — so it is not this spec's change under test.
- **Tracker:** **TODO-630** (tombstone-byte bound breach at the production epoch width) — the
  investigation, its first experiment and its fork are specified there.
- **Owner:** the TODO-566 / SPEC-345 tombstone-GC line, sequenced **before** the 72 h soak
  (TODO-484 re-run) and before TODO-586. See §7.2.

**Also recorded, report-only:** the independent redb corpus scan **DIVERGED** from the gauge in both
runs (ON: corpus 200,860 B vs gauge 194,260 B; OFF: corpus 239,256 B vs gauge 245,809 B). The
divergence is small and in opposite directions, so it does not change the classification, but it is
recorded for whoever picks up the tombstone gate.

### 7.2 What this red does to the 72 h soak — a NAMED pre-soak blocker

The 72 h soak (TODO-484 re-run) runs **this harness**, at the **same production epoch width**, and
`tombstones.passed` is hard-ANDed into the run verdict alongside convergence, recovery, memory and
panic. Its last-half window is ~36 h, i.e. vastly *more* epoch cycling than the window that already
failed — so on the evidence here the 72 h soak would **fail on this clause by construction**.

Recording it as anything less than a named blocker would mean spending 72 hours to rediscover a
number already measured twice. **TODO-630 is therefore a pre-soak blocker, sequenced before
TODO-586 and before the 72 h soak run.**

**The split is deliberate and narrow — this does NOT block SPEC-348:**

| Consumer | Status | Why |
|---|---|---|
| **SPEC-348 disk (WAL + redb) gate** | **UNBLOCKED** — proceeds from this spec's numbers | The disk evidence chain is independent of the tombstone gauge: §2's slopes come from `du` on the real paths, AC5 cross-reproduces them from the committed CSVs, and AC2's verdicts do not read the gauge |
| **SPEC-348 RSS gate** | **NOT derivable here** (unchanged) | §6 — RSS is BENT, not plateaued; no bound is derivable. Unrelated to this red |
| **72 h soak / TODO-586** | **BLOCKED on TODO-630** | Same harness, same width, same hard-ANDed clause |

### 7.3 Provenance of the §7.1 figures — what is committed and what is NOT

The classification in §7.1 must be readable against its own evidence, so the provenance of each
figure is stated rather than assumed:

| Figure used in §7.1 | Committed artifact it can be re-read from | Status |
|---|---|---|
| Slopes 248,148.9 / 283,066.2 B/h, `samples=720`, window spans | `*.soak.json` → `finishedReason` (verbatim), and from this run forward the `tombstones` object | **SURVIVES** |
| redb corpus scan 200,860 / 239,256 B, gauge 194,260 / 245,809 B | `*.mechanism.json` | **SURVIVES** |
| `epochWidth=1000`, `walFsync="batched"`, `crashes=0` | `*.soak.json` | **SURVIVES** |
| `no_ack=false`, the unconditional tracked-client drive, the `if !cfg.no_ack` call site | `main.rs` at the measured commit + the runner's own flag list | **SURVIVES** (source, not run output) |
| `confirms=1727`, `lastConfirmedEpoch=113`, `confirmErrors=51`, and the per-checkpoint cursor sequence | `spec349c2-emitter-on.harness-console.log` and `spec349c2-emitter-off.harness-console.log` — final summary at **line 75** of each; the 11 checkpoint lines at **8, 13, 18, 24, 29, 34, 40, 45, 50, 55, 61** (ON) and **8, 13, 19, 24, 29, 34, 40, 45, 50, 55, 61** (OFF) | **SURVIVES** |

**Correction on the record — the earlier "gone" claim was false.** An earlier revision of this
section stated that the harness stdout the runner wrote to `<data-dir>.meta/harness-console.log`
under `target/` was **gone**, and recorded the confirm-apply figures **as transcribed** on that
ground. That was wrong as a statement of fact: both files had sat under `target/` since the runs,
untouched, with their original run mtimes (ON 10,594 B, 18:06; OFF 10,857 B, 19:08). Nothing was
deleted — the files were simply never looked for after the runner finished. The premise being false,
the remedy Review v1's C3 originally named is the one that applies: **both logs are now committed
into this directory**, byte-identical to the originals, and the figures above are **artifact-backed
with path and line cites**, not transcribed. §7.1's three legs are therefore checkable end-to-end
from this directory alone. The corroborating evidence named below (the `no_ack` source path, the
gauge-independent redb corpus scan) remains true and is now redundant rather than load-bearing.

*Why the correction is recorded rather than silently overwritten:* a provenance table whose stated
reasons are not themselves checkable is the same class of defect it exists to prevent. The reader
should be able to see that this row changed and why.

**The instrument defect is real independently of the correction above, and it is fixed rather than
merely noted.** That the scratch file happened to survive is luck, not design: it lives under
`target/`, which any `cargo clean` removes. The numbers that failed BOTH runs must not depend on it.
So the remedy is both halves — the two logs are committed (above), *and* from this commit forward the
harness persists these figures itself:

- `*.soak.json` carries a `tombstones` object (samples, first/peak/last bytes, slope, passed,
  reason), a `disk` object, and a `confirmApply` object (`confirms`, `lastConfirmedEpoch`,
  `confirmErrors`) — i.e. **every verdict that `passed` is hard-ANDed with is persisted beside it**.
- `*.progress.jsonl` carries the per-checkpoint confirm-apply cursor, so the **series** — not just
  the final count — survives, which is what distinguishes a steadily-advancing LWM from one that
  advanced once and stalled.
- The CSV carries a `tombstone_bytes` column scraped from the server's own `/metrics`, so the gauge
  series that decides the gate is a committed column beside `rss_mb` and `disk_total_mb`. An empty
  cell means a scrape that did not answer and is dropped from the fit; a column with **no** readings
  at all is declared an instrument defect by the runner's post-run check.

Everything else that is ANDed into `passed` stayed green in both runs: `convergenceFailures=[]`,
`recoveryFailures=[]`, `panicReport=null`, `crashes=0`, `writeErrors=0`. `memory.passed == true` is
**not** load-bearing (the neutralizers were set by design), and `pendingGates` is report-only by
construction.

---

## 8. OT5 — the TODO-612 `flush_key` reachability determination

**VERDICT: NO-OP-CONFIRMED.**

**Executable proof (this is the second half of OT5, and it is required):**
`flush_key_frameless_window_leaves_no_enumerable_past_older_remove`, in the inline `#[cfg(test)] mod
tests` of `packages/server-rust/src/storage/datastores/write_behind.rs` (fn at `:4049`), beside
`flush_key_resolves_watermark_after_inner_add_not_before`. It drives the **live** `flush_key` path —
the real `MapDataStore` trait method on a real `WriteBehindDataStore` — rather than the fabricated
`re_replay_oldest_frame` seam.

### The argument

**The window IS open in the literal sense R1 asks.** At the crash point the superseded `Remove` is
both un-resolved and enumerable-past: `flush_key` registers the superseded entry's WAL sequences as
in-flight *before* `inner.add`, and resolves them only *after* `inner.add` returns. A crash inside
that window leaves a frameless newer durable value with an older `Remove` still in the replay window.
So the honest statement is **not** "the watermark excludes the window" — that is backwards.

**Enumerability is GUARANTEED BY the prefix-complete discipline, not excluded by it.** The invariant
is `applied_seq < min(pending)`, so the superseded `Remove` at `N` is necessarily still enumerable.
That is the premise, not the refutation.

**The loss does not follow, because of the enumerability closure.** `applied_seq < N` implies every
frame **above** `N` is *also* enumerable, and TG-WAL-011's strictly-ascending replay of the unapplied
window replays each of them **after** `N`. And a queued `Remove` is by construction the newest op for
its key — `PartitionQueue` holds exactly one entry per `(map, key)` — so any strictly-newer value
carries its **own** frame at some `M > N` in the same window. This holds under **both** coalesce
dispositions:

- a subsuming `Store` coalesces and **early-resolves** the superseded sequences, so there is nothing
  stale left to replay; and
- a non-subsuming `OrDelta` **carries forward** the superseded sequences, so the newer frame at
  `M > N` is enumerable and replays after the `Remove` at `N`.

Either way the older `Remove` cannot land over the newer value.

**The only residual premise is caller-side, and it is currently vacuous.** `flush_key` takes an
arbitrary caller-supplied value and has **zero production callers at HEAD** — every originating call
is `#[cfg(test)]`, and the eviction path does not call it. That residual is documented on the function
itself (see §10 F5) rather than left implicit.

### ⛔ The PRE-SOAK GATE is DISCHARGED, and here is the precise reason

The gate's own premise — *"the 72h soak drives the LIVE `flush_key` path"* — is **FALSE at HEAD**. No
production path reaches `flush_key`, so the soak neither exercises nor **can** exercise this residual.
The gate is therefore discharged on the determination above **plus** that reachability fact, and the
reason is stated this precisely so the gate cannot silently re-arm: **the moment a production caller
is wired to `flush_key`, the caller-side residual stops being vacuous and this discharge must be
re-examined** (see the TODO spin-off required in §10 F5).

---

## 9. Measurement regime — what SPEC-348 is deriving from

**SPEC-348 MUST record this regime beside any number it derives from the slopes above.**

| Parameter | Value in these runs | Production default |
|-----------|--------------------|--------------------|
| write-behind flush interval | **100 ms** (harness default, `process.rs:212-216`) | 1000 ms |
| write-behind batch size | **5000** (harness default, `process.rs:217-221`) | 100 |
| epoch width | **1000** (production default) | 1000 |
| WAL fsync policy | **`batched`** (`--wal-fsync batched`) | `batched` |
| crash interval | **0** — no `kill -9` during either run | n/a |

**These slopes were measured at the harness's flush cadence, not production's.** The harness default
is kept for instrument-identity with every other soak run (including the historical characterization
runs this work descends from), and the consequence is accepted explicitly: flush cadence moves *when*
bytes land in redb versus the WAL, so a production gate derived from these numbers must be argued
against this regime, not against a 1000 ms / 100 one.

### 9.1 The handoff direction — SPEC-348 **derives**, this spec **promotes nothing**

This spec sets **no** threshold and promotes **no** gate. Every number in §2–§4 is a recorded
measurement. SPEC-348 is the spec that turns a number into a gate, and it derives from these
artifacts rather than inheriting a verdict from them.

### 9.2 The loosening argument SPEC-348 is REQUIRED to make (AC11)

This is the obligation SPEC-348 must discharge, stated here in the artifact SPEC-348 actually opens
rather than only in the spec file:

- **RSS is ALREADY hard-gated at 2.0 MB/h** in this harness (`monitor.rs`, the RSS assertion
  default). SPEC-348's charter phrase "promote RSS from report-only to HARD" is therefore **stale**:
  there is nothing to promote. Any RSS number SPEC-348 sets that is **above 2.0 MB/h is a LOOSENING
  of a live gate**, and it must be **argued as one** — naming the current value (2.0 MB/h), the
  derived value, and why raising it is correct — not described as a promotion.
- **SPEC-348 MUST NOT carry forward the 25 MB/h figures** from this spec's earlier drafts, or any
  figure not read from the committed artifacts in this directory. Those figures are gone from this
  spec deliberately: promoting 25 MB/h against a live 2.0 MB/h gate would have loosened it **12×**.
- **For RSS specifically, no gate is derivable from these runs at all.** §6 records RSS as **BENT,
  not plateaued** — it still climbs at 8461 MB/h in the ON arm. A bound cannot be derived from a
  series that has not flattened, so SPEC-348's RSS clause has no input here regardless of the
  loosening argument. Its disk (WAL + redb) clause is the part these runs actually feed.
- **The tombstone-byte gate is out of SPEC-348's scope and is separately blocked** — see §7.1/§7.2
  and TODO-630. SPEC-348's disk gate proceeds; the 72 h soak does not.

---

## 10. Deferred findings (recorded, NOT fixed here — the measurement runs last and alone)

| # | Site | Finding |
|---|------|---------|
| **F1** | `src/storage/wal/mod.rs:1612-1616` | The `re_replay_oldest_frame` doc-comment says the seam manufactures a condition "the live `flush_key` path **produces**". Per §8 the live path does not produce it — the enumerability closure prevents the stale replay. The doc **overstates the hazard**. |
| **F2** | `src/storage/wal/mod.rs:2074-2081` | Same overstatement at the seam's use site ("reproduces the stale re-replay the live `flush_key` path **can cause**"). |
| **F3** | `src/storage/datastores/wal_harness/cases.rs:697-698` | Same overstatement in the AC4.5 case doc ("the older-frame-in-window condition the live `flush_key` path **can produce**"). |
| **F4** | `INVARIANTS.md:187-190` | TG-WAL-009's *Windowing residual* bullet still routes the frameless-`flush_key` hazard to TODO-612 as open. It **should carry this determination** (NO-OP-CONFIRMED for the crash window; the live residual is caller-side and vacuous at HEAD). |
| **F5** | `src/storage/datastores/write_behind.rs` (`flush_key` doc-contract) | **The mirror-direction finding, and the one with teeth.** `flush_key` resolves-and-advances the superseded entry's WAL sequences *after* making the **caller's** value durable, having written no frame of its own. A caller whose value does **not** subsume a superseded `Remove` therefore **loses that acked delete with no crash at all**. Unreachable at HEAD (zero production callers; every caller is `#[cfg(test)]`), documented on the fn itself, **and it needs a TODO spin-off that must be closed BEFORE any production caller is wired.** |
| **F6** | `src/storage/datastores/wal_harness/cases.rs` (`tg_or_003_ac9_delta_construction_stays_inside_its_sanctioned_home`, `is_delta_frame_home` at `:2321`) | The AC9 delta-frame-home gate's belt (3) walks the **whole package** (`walked_sources(package, package)`), not just `src/`. So **any new `.rs` anywhere under `packages/server-rust/` that constructs a `WalOp::OrDelta` breaks the gate** until it is added to `is_delta_frame_home`. Record the coupling: it is deliberate (it is what covers the bench binaries) and it is a maintenance cost on every future file that touches delta framing. |

---

## 11. Two exemptions beyond the spec's ledger — SURFACED for the reviewer to rule on

Neither was absorbed silently; both are stated here so the reviewer rules rather than discovers.

**(a) `write_behind.rs`'s new `flush_key` doc-contract.** Doc-only, cascade-free, on exactly the same
ground as **E1** (which is admitted on cascade-freeness plus an explicit auditor ruling). It is what
makes §8's no-op determination **durable**: without it the caller-side residual (F5) lives only in a
manifest, where the next contributor wiring an eviction caller will not read it. E2 already admits
this file under shape 4 for the test; the doc-contract is a second, smaller extent in the same file.

**Ruling (Review v1, minor 7): (a) is ADMITTED**, on the ground stated — doc-only, cascade-free,
load-bearing for §8's determination. It is also the **third** recurrence of the pattern that
codified shapes 3 and 4 in PROJECT.md: a spec-side obligation that can only be written where the
subject lives, on a file already exempt for a different reason. PROJECT.md now carries it as
**shape 5** so the next spec cites a rule instead of re-arguing the exemption.

**(b) `wal_harness/cases.rs` gained one disjunct.** `is_delta_frame_home` grew a single arm admitting
`tests/soak_wal_census.rs` (`:2326`), because the census fixture legitimately constructs delta frames
through the real encoder. This makes `cases.rs` **counted `.rs` #4 of 5**, and it spends the ledger's
one slot of headroom on a **red correctness gate** rather than on documentation.

*Ruling (Review v1, minor 8): the edit is a one-line arm on an existing item with no signature,
lifetime or borrow change, so it is plausibly **shape 1** and need not have been counted at all. It
is left COUNTED deliberately — the conservative reading costs one slot the spec did not need, and a
ledger that over-counts is the safe direction to err. Headroom is 1 either way.*

**Final budget, stated honestly:**

- **Counted (4 of 5):** `benches/soak_harness/main.rs`, `benches/soak_harness/report.rs`,
  `tests/soak_wal_census.rs`, `src/storage/datastores/wal_harness/cases.rs`.
- **Exempt:** `benches/soak_harness/monitor.rs` (**E1**, doc-comment-only, admitted by ruling);
  `src/storage/datastores/write_behind.rs` (**E2**, shape 4 — the additive `#[tokio::test]` — plus
  the `flush_key` doc-contract in (a) above).
- **Headroom remaining: 1 slot** — unspent. The conditional counted slot for a TODO-612 **fix** was
  not consumed, because the determination is NO-OP-CONFIRMED and no production edit was made.

---

## 12. Discrepancies found while writing this manifest

Recorded rather than smoothed over, per the rule that the artifact wins.

- **D1 — the AC4(b) separation multiple.** The figure circulated to this step was "36 × the combined
  SE". Recomputed from the committed CSVs it is **39.5 ×** for `disk_total_mb`
  (58 659.075 / 1 483.520 = 39.54) and 39.6 × for `wal_mb`. **The artifact value, 39.5 ×, is what this
  manifest records.** The direction of the error is conservative (the real separation is *wider* than
  claimed), and no verdict changes.
- **D2 — the runner's effective-matrix echo is not in `harness-console.log`.** The runner prints its
  full matrix block (repo HEAD, dirty-tree flag, `uname -a`, data dir, `TOPGUN_OR_DELTA_WAL`,
  `TOPGUN_EPOCH_WIDTH`) to **its own stdout**, while `$CONSOLE_LOG` captures only the redirected
  harness child (`"$SOAK_BIN" … > "$CONSOLE_LOG"`). The console logs therefore carry the harness's
  one-line matrix echo but **not** the runner's block. §1 is consequently reconstructed from three
  artifact sources — the committed runner's literals (AC1's own "observable only via the committed
  runner script" class), the harness's console first line, and `soak.json` — and one field, **the
  dirty-tree flag, is attested by the run record rather than artifact-observable**.

  **FIXED for all future runs (Review v1, minor 11).** The deferral ground ("no file may change
  before merge") did not survive: the runner is a shell script, and this fix changes no `.rs`. The
  block is now `tee`d into a **committed** artifact, `<base>.matrix.txt`, in the evidence directory
  — chosen over `tee`ing into `$CONSOLE_LOG`, which would have left it in the same scratch location
  under `target/` that §7.3 exists to stop relying on. **For THESE two runs the field remains
  attested**, since the runs are not being repeated; only future runs carry it as an artifact.
- **D3 — W4(a) condition 1's grep is narrower than "absent from `benches/soak_harness/`".** Both
  strings *do* occur in that directory (in `report.rs`, which **reads** the var, and in the runner
  script, which **unsets** it). They are absent from **`process.rs`**, which is what the condition
  actually requires. Recorded in full in §5 so the next reader does not re-derive this as drift.
