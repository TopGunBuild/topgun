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
| `spec349c2-plateau.sh` | the runner — **the matrix is executed by this file, not transcribed by hand** |
| `spec349c2-fit.awk` | the post-hoc OLS fit; every slope below is `awk -v col=<series> -v window=<last_half\|full> -f spec349c2-fit.awk <run>.csv` |
| `spec349c2-manifest.md` | this file |

---

## 1. Env matrix as actually run

**Binary.** Commit **`d6922f08`** (`feat(sf-349c2): execute the plateau matrix instead of
transcribing it`, 2026-08-01 17:03:52 +0300), release profile, `cargo build --release --bin
topgun-server --bench soak_harness`. The ON run started ~17:06 and the OFF run ended 19:08; the next
commit (`aae8cd44`, 19:09:59) added only the evidence files. `git diff --name-only d6922f08..HEAD --
'*.rs'` returns **nothing**, so the binary under measurement is byte-equivalent in source terms to
HEAD. Tree state at run time: **clean** (attested by the run record — see §12 finding D2 for the
observability limit on this one field).

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

**But it is a FINDING to report, and it is reported here.** Two observations belong with it:

- It fires in **BOTH** runs at a **similar magnitude** (248 k vs 283 k B/h, ~1.14 ×, against a 512 B/h
  gate — i.e. ~485 × and ~553 × over). It is therefore **not emitter-attributable**: the emitter
  changes WAL framing, not the tombstone corpus, and the census confirms `removeFrames == 0` in both
  runs.
- The independent redb corpus scan **DIVERGED** from the gauge in both runs (ON: corpus 200,860 B vs
  gauge 194,260 B; OFF: corpus 239,256 B vs gauge 245,809 B) — report-only, recorded for whoever picks
  up the tombstone gate.

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

**(b) `wal_harness/cases.rs` gained one disjunct.** `is_delta_frame_home` grew a single arm admitting
`tests/soak_wal_census.rs` (`:2326`), because the census fixture legitimately constructs delta frames
through the real encoder. This makes `cases.rs` **counted `.rs` #4 of 5**, and it spends the ledger's
one slot of headroom on a **red correctness gate** rather than on documentation.

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
  dirty-tree flag, is attested by the run record rather than artifact-observable**. Cheap future fix
  (out of scope here, since no file may change before merge): `tee` the block into `$CONSOLE_LOG`.
- **D3 — W4(a) condition 1's grep is narrower than "absent from `benches/soak_harness/`".** Both
  strings *do* occur in that directory (in `report.rs`, which **reads** the var, and in the runner
  script, which **unsets** it). They are absent from **`process.rs`**, which is what the condition
  actually requires. Recorded in full in §5 so the next reader does not re-derive this as drift.
