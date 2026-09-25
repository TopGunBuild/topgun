# SPEC-373a (carve 9c, part a) — leaf-hash stream + prune residency check: pre-registration manifest

## §1 Pre-registration (frozen at commit M′; nothing above the APPEND-ONLY marker changes after M′)

**M′ supersedes M (`677f39f8`).** M was reviewed before any count-alloc cell ran (conductor rulings v7; cross-vendor
review over `639a7b19..677f39f8`) and found to carry real defects in the verdict program and the cell chain: the
manifest text stated the last-half start as `floor(n/2)` while the code (byte-identical to `spec372-k.awk`) starts
at the 1-based point `int((n-1)/2)+1`; the build freeze covered only `*.rs`; a zero after-rate crashed the verdict
awk into a truncated file with exit 0; malformed pre-M lines were silently read as a STOP; the chain did not
verify its own freeze; malformed probe rows were dropped without a trace; the PA/PM1 "verbatim" claim was not
checked mechanically. No cell had run against M, so the fix and this re-freeze are pre-registration, not a
post-hoc change. The diagnostic dhat pair, the E program and its outputs, `E_FROZEN` and `SHARES_STOP` are
unchanged from M (same bytes, same numbers); the runner `spec373a-cells.sh` and `spec373a-diag.sh` are
unchanged. **The cells run against M′ only.** M's history stays in git.

### Question and scope
Does removing two per-op whole-slot copies on the OR write path — the joined/formatted strings of the OR
Merkle leaf hash (R1) and the prune probe's engine `get` clone of a resident key (R2) — lower the
count-alloc allocation rate of the SPEC-371/372 OR-churn cell by the share dhat attributes to them?
The mechanism is proven by tests (AC-1..AC-3, AC-3g; STOP 3/3a/3b); these cells read the aggregate.

### Code under test
- Pin (before): `61f84658` (SPEC-374 merged). `git diff --quiet 61f84658 origin/main` → **empty** at M
  (origin/main = `61f8465868a550a81061e52b553d9d1b77fed6f1`).
- Branch head H (after): the freeze literal of `spec373a-cells.sh`, `SPEC373A_CODE_FREEZE=4998d884` — the
  last `.rs` commit (`1177d3df` R1, `3b05b085` R2, `4998d884` test-only). Every later commit on the branch
  is `.sh`/`.md`/data only; the runner refuses a checkout whose `.rs` tree differs from the freeze.
- After-cell rule (conductor rulings v6, pathspec widened by v7 item 2): a1/a2 run the server built from H. If any
  BUILD INPUT changes on the branch after the cells — `packages/server-rust/{src,Cargo.toml,build.rs}`,
  `packages/core-rust`, the root `Cargo.toml`, `Cargo.lock` — a1/a2 are re-run; test/doc-only changes outside that
  pathspec do not invalidate them.

### Cells
`spec373a-cells.sh <cell>` (runner), launched by `spec373a-diag.sh` (d3e/d3l) and `spec373a-chain.sh`
(b1/a1/b2/a2, in that interleaved order). Every cell: cadence 60 s, crash-interval 0, live-copy census at
300 s, log directive armed, journal default; every other matrix literal is `spec372-allocdiag.sh`'s own
(= SPEC-370/371's: churn-clients 6, keyspace 200, or-churn true, or-keyspace 48, or-every 5,
write-interval-ms 20, writes-per-life 200, offline-keys 3, confirm-interval 2, steady-interval 300,
quiesce 3, mem-sample-interval 5, wal-fsync batched, mem gate neutralised, jitter seed 20260831).

| cell | duration | flavour | server built at | teardown | role |
|---|---|---|---|---|---|
| d3e | 300 | DH | `61f84658` | SIGTERM (dhat flush) | pre-registration input (dhat, c3e shape) |
| d3l | 900 | DH | `61f84658` | SIGTERM | pre-registration input (dhat, c3l shape) |
| b1, b2 | 900 | CA | `61f84658` | SIGKILL | before pair |
| a1, a2 | 900 | CA | H (`4998d884`) | SIGKILL | after pair (= SPEC-373b's before pair) |

### Pre-registration input: the diagnostic dhat pair (run, committed with M)
`spec373a-diag.sh`, chain start 2026-09-24T17:00:08Z, end 17:30:44Z, both `RUNNER_EXIT=0` and
`RESULT: instrument sound`. Builds (`spec373a-diag-builds.txt`):
- DH `sha256=2ec77c39f2914ac0154f22ed3271d896201c4594b54068b8ad6d7d3d6928764f`, built from the clean detached
  checkout `target/spec373a-src-61f84658` (HEAD `61f8465868a5…`), `--profile release-with-debug --features dhat-heap`
- H (harness) `sha256=991a100a28218d70798a8104ced642b3192c37b30da1b87dddfcbcdbb2f1b895`, built at `639a7b19`
  (`.rs` tree = the freeze)

dhat end times `te` (µs): d3e 309 980 094, d3l 911 983 784 (spec371: c3e 306 423 049, c3l 910 862 613) — the
same 300 s / 900 s shape; window ≈ 310 → 912 s. Recorded, not gated: the pin pair served fewer writes than
the 371 pair (d3l `totalWrites` 44 952 vs c3l 52 457; host load average 8.8 at d3l start), so its window is
smaller in MB (4 299 vs 5 251); the shares are ratios of the same window and are read as such.

### The E program (conductor rulings v2 Critical 1, v3 items 5–6)
`packages/server-rust/benches/soak_harness/evidence/spec373a-shares_61f.py` (committed; byte-identical to the spec's
`.specflow/research/spec373-dhat-attribution/shares_61f.py`, which must keep hashing equal — the chain checks it),
run under `LC_ALL=C`. The outputs quoted below were produced by the `.specflow` copy at M, with the same bytes. Method, verbatim from
SPEC-373a Measurement: window = c3l − c3e (dhat `tb`), shares ÷ window total; `LEAF_87/90/93` = bytes of
program points whose FIRST (innermost) `topgun_*` frame (a frame containing `topgun_server::` or
`topgun_core::`) is `storage/map_data_store.rs:87` / `:90` / `:93`; `PRUNE` = bytes of every program point
whose chain contains the engine-get-clone frame AND the probe site, summed in-program, no top-N cut;
`OVERLAP` = bytes satisfying both. STOP-D: self-check outside 0.104772 / 0.028618 ± 0.000005, or a
`LEAF_*_MB` / `PRUNE_MB` ≤ 0 on either run. STOP-O: `OVERLAP_MB > 0`. STOP-S: `|new − old| / old > 1/3` for
`LEAF_SHARE` or `PRUNE_SHARE`, old = the self-check OUTPUT. Precedence D > O > S.

Pin literal sets, read from the code by these commands (output at each pin quoted):
```
git show <pin>:packages/server-rust/src/storage/map_data_store.rs | grep -n 'tags.join\|tomb_tags.join\|fnv1a_hash(&format!("key:'
git show <pin>:packages/server-rust/src/storage/engines/hashmap.rs | grep -n 'fn get(' -A1
git show <pin>:packages/server-rust/src/service/domain/crdt.rs | grep -n 'match store.get(&r.key, false).await'
```
| pin | leaf lines | engine get clone | probe site |
|---|---|---|---|
| `46dcc12a` | `map_data_store.rs:87`, `:90`, `:93` | `hashmap.rs:64` (regex `6[3-4]`) | `crdt.rs:1780` |
| `61f84658` | `map_data_store.rs:87`, `:90`, `:93` | `hashmap.rs:92` | `crdt.rs:1786` |

**Self-check** — `LC_ALL=C python3 shares_61f.py --pin 46dcc12a spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz`
(`spec373a-selfcheck.txt`):
```
  WINDOW_MB=5251.3476
  LEAF_87_MB=135.3647
  LEAF_90_MB=7.3617
  LEAF_93_MB=407.4686
  LEAF_SHARE=0.104772
  PRUNE_MB=150.2835
  PRUNE_SHARE=0.028618
  OVERLAP_MB=0.0000
  E=0.133390
  P_BYTES=0.866610
  SELF_CHECK=PASS (LEAF 0.104772 vs 0.104772, PRUNE 0.028618 vs 0.028618, tol 0.000005)
  STOP=none
  E_FROZEN=0.1334
  P_BYTES_FROZEN=0.8666
```
**Pin reading** — `LC_ALL=C python3 shares_61f.py --pin 61f84658 spec373a-d3e.dhat.json.gz spec373a-d3l.dhat.json.gz --old spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz`
(`spec373a-shares.txt`):
```
  OLD_LEAF_SHARE=0.104772
  OLD_PRUNE_SHARE=0.028618
  OLD_SELF_CHECK=PASS
  WINDOW_MB=4299.2113
  LEAF_87_MB=98.3819
  LEAF_90_MB=5.8018
  LEAF_93_MB=296.3361
  LEAF_SHARE=0.093161
  PRUNE_MB=97.6507
  PRUNE_SHARE=0.022714
  OVERLAP_MB=0.0000
  E=0.115875
  P_BYTES=0.884125
  S_LEAF_REL=0.110821
  S_PRUNE_REL=0.206319
  STOP=none
  E_FROZEN=0.1159
  P_BYTES_FROZEN=0.8841
```

### Frozen values (read by the verdict program; never recomputed; unchanged from M)
SHARES_STOP=none
E_FROZEN=0.1159
P_BYTES_FROZEN=0.8841

The spec's placeholders (`E_old = 0.1334`, `P_BYTES_old = 0.867`) are superseded by the pin reading above,
as the spec directs. `ALLOC_LIVE` is expected unchanged (the removed copies are transient).

### Units (conductor rulings v3 item 1; normative for the readout)
`E` is a dhat-`tb` share: dhat adds the FULL new size on every `realloc`, stats_alloc (the CA probe's
`bytes_allocated`, `bin/topgun_server.rs:467-479`) adds only the growth. In stats_alloc units the `:93`
`format!` term is ≈ 2/3 of its dhat bytes (the string grows ~3× per call), so the removable leaf term at the
pin reads ≈ (98.4 + 5.8 + ⅔ × 296.3) / 4 299 ≈ 7.0 % instead of 9.3 %, and the window denominator is also
inflated by an unknown factor. The class thresholds keep `E` from dhat: even in stats_alloc units the
expected cut (≈ 7.0 % + 2.3 % ≈ 9.3 %) stays above the CONFIRMED margin `E/2` = 5.8 %. **If the measured class
is NOT_MET, the readout must quote this unit gap before NOT_MET is read as a miss.**

### Per-bucket context table (does not feed E)
`spec373a-tb2_61f.py <profile>` (committed; byte-identical to `.specflow/research/spec373-dhat-attribution/tb2_61f.py`), first-match in the frozen order of the
SPEC-373a bucket table (rows 1–10); outputs `spec373a-d3e.tb2.txt`, `spec373a-d3l.tb2.txt`. Every row's
regex was checked against its source line at `61f84658` (`hashmap.rs:92/136/158/255/268`,
`write_behind.rs:1538/2428/2463/2518/2649`, `map_data_store.rs:72-97`).

### Verdict program (SPEC-373a Measurement; `spec373a-verdict.sh <EV> spec373a-manifest.md`)
- **Definitions** (from `spec372-manifest.md:126-130`, `CHURN_RATIO` (k1), with the window start stated as the
  code computes it). One point per DISTINCT `alloc_probe_elapsed_s` (adjacent repeats dropped), `n` points,
  1-based. **The last half starts at point `h = int((n−1)/2) + 1`** (0-based `ceil(n/2) − 1`: `floor(n/2)` for odd
  n, one point EARLIER for even n — with ~15–30 probe points per 900 s cell, even n is ordinary) and ends at
  point `n`. This is byte-identical to `spec372-k.awk:104` (`h = int((n - 1) / 2) + 1`), kept for parity with
  the SPEC-371/372 numbers; `spec372-manifest.md`'s prose "floor(n/2)" described odd n only.
  `BYTES_ALLOC_RATE` = (`bytes_alloc`[n] − `bytes_alloc`[h]) / (`e`[n] − `e`[h]); `ALLOC_LIVE` =
  `alloc_live_bytes`[n]; both printed `%.6f` / integer with `points=`, `h=` and `window_s=`.
0. **Pre-M′ lines.** Exactly one `SHARES_STOP=` and one `E_FROZEN=` line (anchored at column 0),
   `SHARES_STOP ∈ {none, D, O, S}`, `E_FROZEN` numeric — else the program exits **3 with no flags**. A missing
   line is never read as a STOP. STOP-D/O/S = `SHARES_STOP`.
1. **STOP-V** for each of b1, b2, a1, a2 on any of: `PA` false; `PM1` false; `SKIPPED_<cell> > 0` (rows whose
   three probe fields `bytes_alloc`, `alloc_live_bytes`, `alloc_probe_elapsed_s` are neither all empty nor all
   numeric; all-empty pre-first-probe rows are not counted); a `BYTES_ALLOC_RATE` or `ALLOC_LIVE` that is n/a or
   not > 0. The STOP line names every failing clause, e.g. `STOP=V (a1:rate=0.000000 a2:rate=0.000000)`.
   `RUNNER_EXIT` is recorded per cell, not gated.
   **PA and PM1 are executed from the frozen file, not transcribed.** The verdict asserts
   `sha256(spec371-predicates.sh) = 7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b`, takes
   the PE/PA awk program from its lines 101–131 and the PM1 program from lines 154–169, strips only the closing
   `' "$CSV"`, asserts each ends at its closing brace, and runs them with `fl=CA` / `pm=` exactly as the frozen
   wrapper does. The mechanical proof — `diff` of the frozen invocation block against what the verdict runs,
   where only the shell plumbing differs:
   ```
   $ diff <(sed -n '100,131p' spec371-predicates.sh) <(sed -n '101,131p' spec371-predicates.sh | sed '$ s/'\'' "\$CSV"$//')
   1d0
   <     awk -F, -v fl="$FLAVOUR" -v dur="$DURATION" -v cad="$CADENCE" '
   32c31
   <       }' "$CSV"
   ---
   >       }
   $ diff <(sed -n '153,169p' spec371-predicates.sh) <(sed -n '154,169p' spec371-predicates.sh | sed '$ s/'\'' "\$CSV"$//')
   1d0
   <     awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" '
   17c16
   <       }' "$CSV"
   ---
   >       }
   ```
   The missing-input branches around them (`PA=FALSE reason=no_matrix_or_csv`,
   `PM1=FALSE reason=no_counter_or_csv`) are the frozen wrapper's own.
2. `R_ij = BYTES_ALLOC_RATE(a_i) / BYTES_ALLOC_RATE(b_j)`, `I = [min, max]`; same for `ALLOC_LIVE` ⇒ `LIVE_I`.
3. `s_b = |b1 − b2| / mean(b1, b2)`, `s_a` likewise, per metric.
4. STOP-R: `min I > 1 + max(s_b, s_a)` or `min LIVE_I > 1 + max(s_b_live, s_a_live)`.
5. Class: INDETERMINATE iff `s_b ≥ E/2`; else CONFIRMED iff `max I ≤ 1 − E/2`; else NOT_MET.
   With `E = 0.1159`: `E/2 = 0.05795`, CONFIRMED needs `max I ≤ 0.94205`.
6. Flags, after every STOP predicate: `STOP=<none|D|O|S|V|R>` (first in D > O > S > V > R), then
   `VERDICT_BYTES=<class>` (`WITHHELD` under any STOP), `I=[…]`, `S_B=…`, `E=…`, `LIVE_I=[…]`.
7. **Exit status:** 0 = the flags block was printed; 3 = invalid pre-M′ line or frozen predicate source; 4 = a
   program step failed (every awk's exit status is checked; the intermediate file is kept). The chain logs the
   status and says "NO flags" when it is non-zero.
8. Merge: with AC-1..AC-3 green, merge on CONFIRMED, NOT_MET or INDETERMINATE; any STOP → conductor.

**Synthetic cases** (`spec373a-synth.sh`; pinned inputs, expectations derived by hand in its header), run
against this verdict program before M′ — all twelve as expected:

| case | what it exercises | output |
|---|---|---|
| S1 | baseline | rc=0 STOP=none CONFIRMED I=[0.8515,0.8700] |
| S2 | cut too small | rc=0 STOP=none NOT_MET I=[0.9505,0.9650] |
| S3 | before-pair spread ≥ E/2 | rc=0 STOP=none INDETERMINATE S_B=0.0952 |
| S4 | rate regression | rc=0 STOP=R WITHHELD I=[1.2871,1.3100] |
| S5 | PM1 false on b2 | rc=0 STOP=V (b2:PM1) WITHHELD |
| S6 | SHARES_STOP=S | rc=0 STOP=S WITHHELD |
| S7 | live regression | rc=0 STOP=R WITHHELD LIVE_I=[1.3000,1.3000] |
| S8 | **even n** (n=14, kink at 480 s) | rc=0 `h=7 window_s=420-840`, b 1142.857143, a 971.428571, CONFIRMED I=[0.8500,0.8500] (start `int(n/2)+1` would give 0.8000) |
| S9 | **zero after-rates** | rc=0 STOP=V (a1:rate=0.000000 a2:rate=0.000000) WITHHELD, I=n/a |
| S10 | one non-numeric `bytes_alloc` | rc=0 SKIPPED_b1=1, STOP=V (b1:skipped=1) WITHHELD |
| S11 | manifest without `SHARES_STOP=` | rc=3, no flags |
| S12 | `E_FROZEN=abc` | rc=3, no flags |

### Runner diff: every hunk maps to one of the seven items (`diff spec372-allocdiag.sh spec373a-cells.sh`)
| hunk | item |
|---|---|
| `1a2,46` | 1–7 — this runner's header (the parent's header follows verbatim) |
| `284c329`, `286,287c331,332`, `291,300c336,341` | 1 — usage text: name, lineage, cell list |
| `305c346` | 2 — freeze variable name in the usage text |
| `313,314c354,356` | 2 + 7 — required env names in the usage text, incl. `SPEC373A_SERVER_COMMIT` |
| `332c374` | 1 — flavour column doc (`CA|DH`) |
| `341a384,385`, `343,357c387,390` | 1 + 7 — the cell table and its `CELL_SERVER` column |
| `360c393` | 4 — basename `spec373a-<cell>` |
| `442c475` | 4 — data dir `target/spec373a-<cell>-data` |
| `595,597c628,630` | 4 — port 47359 |
| `622,624c655,657` | 2 + 3 — freeze variable and literal `4998d884` |
| `635,636c668,669`, `643c676`, `660,661c693,694`, `664c697`, `666c699`, `740c783`, `763c806`, `793c836`, `899c942`, `911c954` | 2 — env names |
| `658c691` | 2 — the chain names in the "builds nothing" comment |
| `669a703,714` | 7 — the server code-state assertion |
| `705,707d749`, `708a751` | 5 — flavour markers CA/DH |
| `894c937` | 1 — matrix banner |
| `944a988` | 7 — `server code:` line in the matrix |

Item 6 (console line 1 `flavour=`) has no hunk: the line is built from `FLAVOUR`.

### Programs frozen at M′ (sha256; `ORDER=OK` re-checks these bytes; the chain checks them at start)
- `8f9e99abc62160b73235bba3689b8f843013420fcdd255e99e0e4201d041bafd` `packages/server-rust/benches/soak_harness/evidence/spec373a-cells.sh`
- `6117793832aac860d3f4bb0280f25fb918863cb6d1bb1fd0a333f4236053be01` `packages/server-rust/benches/soak_harness/evidence/spec373a-diag.sh`
- `f5746bd63768fa0a9b48a4c6678df5511a2bebe270272a9c80279e5eca00970e` `packages/server-rust/benches/soak_harness/evidence/spec373a-chain.sh`
- `6f1a87bbdfe8c7335c0f4c806f74ec5536b935fcd8cdf3ef06250b024dd06179` `packages/server-rust/benches/soak_harness/evidence/spec373a-verdict.sh`
- `e255fb6159db914692e0c2a373af4359de4a9d1f95de0867d2d632378e8251cd` `packages/server-rust/benches/soak_harness/evidence/spec373a-synth.sh`
- `6ab430824b6c72852f1fd49c74a220e355bfecb374c0eb54c05bc995d0e0084b` `packages/server-rust/benches/soak_harness/evidence/spec373a-shares_61f.py`
- `33c3fa874aedfcf4002c55c37d08f61d06c1849e449c9b86ea59240a842707f3` `packages/server-rust/benches/soak_harness/evidence/spec373a-tb2_61f.py`
- `7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b` `packages/server-rust/benches/soak_harness/evidence/spec371-predicates.sh`
- `814c7a3b60dffaaf232df737d7d04ae84ead9eb4bdbe54a1552c749cb674cf6c` `packages/server-rust/benches/soak_harness/evidence/spec372-allocdiag.sh`

The `.specflow/research/spec373-dhat-attribution/{shares_61f,tb2_61f}.py` copies must hash equal to the two
committed `spec373a-*` copies (the chain refuses otherwise). Changes from M: chain, verdict and synth are new
bytes (v7 items 1–8); the two E programs are now committed; `spec371-predicates.sh` (executed by the verdict)
and the parent runner are listed. Unchanged: cells, diag, and both E programs' bytes.

Commands, verbatim:
```
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec373a-{cells,diag,chain,verdict,synth}.sh
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec373a-{shares_61f,tb2_61f}.py .specflow/research/spec373-dhat-attribution/{shares_61f,tb2_61f}.py
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec371-predicates.sh packages/server-rust/benches/soak_harness/evidence/spec372-allocdiag.sh
```
The parent programs `spec372-allocdiag.sh`, `spec371-predicates.sh` and `spec372-k.awk` are not edited.

### The §1 prefix sha256 — the command
Computed at M′ and at every later commit by exactly this command (the marker line is included in the hash). M's own
prefix sha256 was `c457e344116e7a9db35ea1e234337cd40a93b7d37b7f9c96e6fb8eddc2305b5f`; M′'s is recorded in the
executor report and re-computed by the chain, never written into §1 (it would change the hash):
```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec373a-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### ORDER=OK (checked by the chain at start, at the cells' data commit and at HEAD)
1. M′ is an ancestor of the commit.
2. The §1 prefix sha256 at the commit (and of the working-tree manifest the verdict reads) equals the one at M′.
3. Every program listed above hashes to its listed sha256, and the two `.specflow` E-program copies hash equal to
   the committed ones.
4. No build input differs from the freeze and the working tree is clean over it:
   `git diff --quiet 4998d884..<commit> -- packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock`
   and an empty `git status --porcelain` over the same pathspec (else a1/a2 are re-run, rulings v6/v7).
`spec373a-chain.sh` runs 1–4 before any build (M′ passed as `SPEC373A_MANIFEST_COMMIT`) and prints
`ORDER=OK manifest_commit=… prefix_sha256=… programs=…` into `spec373a-chain.log`; any mismatch refuses the run.
It also refuses an empty freeze literal and a harness glob that does not resolve to exactly one binary.

### Carried traps
`LC_ALL=C` everywhere a number is parsed (host is `ru_RU`); literals from the code (commands above);
program sha binding; flags printed after every STOP predicate; post-mortem row rule PM1; synthetic cases pin
their inputs; smoke over every path before the cells; never bisect on `rss_mb`; release builds are not
byte-reproducible — only the sha256 of the LAUNCHED binary counts (console line 1 vs `spec373a-builds.txt`).

## APPEND-ONLY BELOW

## §3 — carve 9c part a readout: the OR write path allocates 14–18 % fewer bytes per second

Appended 2026-09-25, after the data commit D = `0e899efe` (`docs(soak): record the carve 9c part a count-alloc
cells`, 45 artifacts, no program or build-input change). The cells ran once, detached, against M′ = `639adc1c`
(`spec373a-chain.log`: chain start 2026-09-24T19:20:08Z, end 20:32:45Z), after an admission smoke over every path
whose launch gate (`ORDER=OK`, a verdict line, `SMOKE COMPLETE`) passed. No cell was re-run.

**ORDER=OK at D:** (1) M′ `639adc1c` is an ancestor of D; (2) the §1 prefix sha256 at D is
`6049ebe12e00f4763713e6e4133c9366b92a4ac3b705fd41ad9c1f5dcfe99a81`, equal to M′'s (the §1 command); (3) the chain's
own start-of-run check printed `ORDER=OK manifest_commit=639adc1c prefix_sha256=6049ebe1…9a81 programs=9`, and
`git diff --quiet 639adc1c..D` holds over the nine frozen programs; (4) the build inputs at D equal `4998d884` and
the working tree was clean over them.

### 3.1 The flags, verbatim (`spec373a.verdict.txt`, exit status 0)
```
STOP=none
VERDICT_BYTES=CONFIRMED
I=[0.8234,0.8596]
S_B=0.0034
E=0.1159
LIVE_I=[0.9824,1.0237]
```
CONFIRMED because `S_B = 0.0034 < E/2 = 0.05795` and `max I = 0.8596 ≤ 1 − E/2 = 0.94205`. STOP-R is false (rate
`min I 0.8234` vs `1 + max(s_b, s_a) = 1.0395`; live `min LIVE_I 0.9824` vs `1.0238`). Cross ratios:
`R_a1_b1 0.8567`, `R_a1_b2 0.8596`, `R_a2_b1 0.8234`, `R_a2_b2 0.8263`; `S_A 0.0395`.

### 3.2 The cells
| cell | server (sha256 on console line 1 = `spec373a-builds.txt`) | PA | PM1 | SKIPPED | RUNNER_EXIT | `BYTES_ALLOC_RATE` B/s (points, window) | `ALLOC_LIVE` B | totalWrites |
|---|---|---|---|---|---|---|---|---|
| b1 | pin `f17f8c29…158f` | TRUE | TRUE | 0 | 0 | 55 099 819.13 (15, 420–840 s) | 68 130 048 | 158 580 |
| a1 | head `bf40ad79…579c` | TRUE | TRUE | 0 | 0 | 47 201 569.63 (15, 450–870 s) | 69 743 296 | 158 975 |
| b2 | pin `f17f8c29…158f` | TRUE | TRUE | 0 | 0 | 54 911 918.79 (16, 420–900 s) | 69 319 747 | 154 647 |
| a2 | head `bf40ad79…579c` | TRUE | TRUE | 0 | 0 | 45 371 635.20 (15, 420–840 s) | 68 099 992 | 158 196 |

Every cell printed `RESULT: instrument sound` and `post_mortem_rows=0`. Ops parity (recorded, not gated): total
writes within 2.8 % across the four cells. `ALLOC_LIVE` is flat (`LIVE_I` within ±2.4 %), as pre-registered: the
removed copies are transient.

**The after pair is SPEC-373b's before pair.** a1 and a2 — `spec373a-a1.*` and `spec373a-a2.*` in this directory
(CSV, matrix, harness and runner consoles, scrapes, soak/durable/mechanism JSON), server `bf40ad79…579c` built at the
freeze `4998d884` — are the reference SPEC-373b measures against.

### 3.3 Observation: the measured cut exceeds E (recorded; decides nothing; not explained)
The measured cut, `1 − I` = 14.0 %–17.7 %, is larger than the pre-registered `E = 11.6 %` (dhat units, §1) and than
its stats_alloc-unit estimate of ≈ 9.3 % (§1 Units). The class does not depend on it. **No mechanism is claimed for
the extra ≈ 3–6 pp.** Candidate mechanisms, named as HYPOTHESES only, for a later carve to test:
- H-a: the leaf-hash `format!` string grows by several reallocations per call; stats_alloc counts each growth
  step, and the per-call growth may be larger at the CA cells' ops rate and slot sizes than the dhat-window
  estimate (⅔ of the dhat `:93` bytes) assumed.
- H-b: the prune probe's clone share may be larger at the CA cells' ops rate (≈ 175 writes/s) than in the dhat
  window, whose pin pair served fewer writes (d3l 44 952 in 900 s, §1), so a share measured there under-states it.
Neither is tested here; the readout, the PR and INVARIANTS.md make no claim about the excess.

### 3.4 Scope
One host (macOS, M1 Max), count-alloc regime, 900 s cells, the SPEC-370/371/372 matrix. The class reads the
allocation RATE of the whole server; the per-change mechanism is proven by the local count-alloc tests
(`count_alloc_leaf_hash`: 106 009 → 16 000 B at N = 1 000; `count_alloc_prune_probe_resident`: p/c 2.000 → 1.000) and
by AC-1/AC-1b and AC-3g in CI.
