# SPEC-373a (carve 9c, part a) — leaf-hash stream + prune residency check: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

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
- After-cell rule (conductor rulings v6): a1/a2 run the server built from H. If any production `.rs` file
  changes on the branch after the cells, a1/a2 are re-run; test/doc-only changes do not invalidate them.

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
`.specflow/research/spec373-dhat-attribution/shares_61f.py`, run under `LC_ALL=C`. Method, verbatim from
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

### Frozen values (read by the verdict program; never recomputed after M)
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
`.specflow/research/spec373-dhat-attribution/tb2_61f.py <profile>`, first-match in the frozen order of the
SPEC-373a bucket table (rows 1–10); outputs `spec373a-d3e.tb2.txt`, `spec373a-d3l.tb2.txt`. Every row's
regex was checked against its source line at `61f84658` (`hashmap.rs:92/136/158/255/268`,
`write_behind.rs:1538/2428/2463/2518/2649`, `map_data_store.rs:72-97`).

### Verdict program (SPEC-373a Measurement, verbatim steps; `spec373a-verdict.sh <EV> spec373a-manifest.md`)
- **Definitions** (verbatim from `spec372-manifest.md:126-130`, `CHURN_RATIO` (k1)): "one point per
  DISTINCT `alloc_probe` line; last half = points `floor(n/2) … n-1` … Ratio = (Δ`bytes_alloc` / Δ probe
  seconds) / `alloc_live_bytes` at the window's LAST point". `BYTES_ALLOC_RATE` = the numerator;
  `ALLOC_LIVE` = the denominator; distinct points are keyed on `alloc_probe_elapsed_s` exactly as
  `spec372-k.awk` does.
0. STOP-D/O/S: the `SHARES_STOP=` line above, read, never recomputed.
1. STOP-V: each of b1, b2, a1, a2 passes `PA` (`spec371-manifest.md:129-130`) and `PM1`
   (`spec371-manifest.md:142`), both evaluated with the verbatim `spec371-predicates.sh` predicates.
   `RUNNER_EXIT` is recorded per cell, not gated.
2. `R_ij = BYTES_ALLOC_RATE(a_i) / BYTES_ALLOC_RATE(b_j)`, `I = [min, max]`; same for `ALLOC_LIVE` ⇒ `LIVE_I`.
3. `s_b = |b1 − b2| / mean(b1, b2)`, `s_a` likewise, per metric.
4. STOP-R: `min I > 1 + max(s_b, s_a)` or `min LIVE_I > 1 + max(s_b_live, s_a_live)`.
5. Class: INDETERMINATE iff `s_b ≥ E/2`; else CONFIRMED iff `max I ≤ 1 − E/2`; else NOT_MET.
   With `E = 0.1159`: `E/2 = 0.05795`, CONFIRMED needs `max I ≤ 0.94205`.
6. Flags, after every STOP predicate: `STOP=<none|D|O|S|V|R>` (first in D > O > S > V > R), then
   `VERDICT_BYTES=<class>` (`WITHHELD` under any STOP), `I=[…]`, `S_B=…`, `E=…`, `LIVE_I=[…]`.
7. Merge: with AC-1..AC-3 green, merge on CONFIRMED, NOT_MET or INDETERMINATE; any STOP → conductor.

Synthetic cases (smoke; pinned inputs and hand-derived expectations in `spec373a-synth.sh`), run before M
against this verdict program, all seven as expected: S1 CONFIRMED, S2 NOT_MET, S3 INDETERMINATE, S4 STOP=R,
S5 STOP=V (b2:PM1), S6 STOP=S, S7 STOP=R (live).

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

### Programs frozen at M (sha256; `ORDER=OK` re-checks these bytes at every later commit)
- `8f9e99abc62160b73235bba3689b8f843013420fcdd255e99e0e4201d041bafd` `packages/server-rust/benches/soak_harness/evidence/spec373a-cells.sh`
- `6117793832aac860d3f4bb0280f25fb918863cb6d1bb1fd0a333f4236053be01` `packages/server-rust/benches/soak_harness/evidence/spec373a-diag.sh`
- `41e41752bab034b8f0cf8bf0f3bd25fb922aa1794a56dbbc936a9ff2c4fac45e` `packages/server-rust/benches/soak_harness/evidence/spec373a-chain.sh`
- `d3ad601fae4b42695c5ee9c9a201085c247b8b83dceef8109af3d98e33152728` `packages/server-rust/benches/soak_harness/evidence/spec373a-verdict.sh`
- `f004698e2ef8476f37a44317aa16b5e923768902f9533480e4715d4bb2d8a51f` `packages/server-rust/benches/soak_harness/evidence/spec373a-synth.sh`
- `6ab430824b6c72852f1fd49c74a220e355bfecb374c0eb54c05bc995d0e0084b` `.specflow/research/spec373-dhat-attribution/shares_61f.py` (local, not in git: `.specflow/` is never committed)
- `33c3fa874aedfcf4002c55c37d08f61d06c1849e449c9b86ea59240a842707f3` `.specflow/research/spec373-dhat-attribution/tb2_61f.py` (local, not in git)

Commands, verbatim:
```
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec373a-{cells,diag,chain,verdict,synth}.sh
shasum -a 256 .specflow/research/spec373-dhat-attribution/shares_61f.py .specflow/research/spec373-dhat-attribution/tb2_61f.py
```
The parent programs `spec372-allocdiag.sh` (`814c7a3b…cf6c`), `spec371-predicates.sh` and
`spec372-k.awk` are not edited.

### The §1 prefix sha256 — the command
Computed at M and at every later commit by exactly this command (the marker line is included in the hash):
```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec373a-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### ORDER=OK (checked at the cells' data commit and at HEAD)
1. M is an ancestor of the commit.
2. The §1 prefix sha256 at the commit equals the one at M.
3. `git diff --quiet M..<commit> -- packages/server-rust/benches/soak_harness/evidence/spec373a-{cells,diag,chain,verdict,synth}.sh`
   holds, and the two `.specflow` programs still hash to the values above.
4. `git diff --quiet 4998d884..<commit> -- '*.rs'` holds (else a1/a2 are re-run, conductor rulings v6).

### Carried traps
`LC_ALL=C` everywhere a number is parsed (host is `ru_RU`); literals from the code (commands above);
program sha binding; flags printed after every STOP predicate; post-mortem row rule PM1; synthetic cases pin
their inputs; smoke over every path before the cells; never bisect on `rss_mb`; release builds are not
byte-reproducible — only the sha256 of the LAUNCHED binary counts (console line 1 vs `spec373a-builds.txt`).

## APPEND-ONLY BELOW
