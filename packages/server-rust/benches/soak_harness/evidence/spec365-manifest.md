# spec365 manifest — conjunct attribution readout

## §1 — Pre-registration

Committed before any `spec365-conj900` data exists. Nothing in this section is edited after its
commit; §2 records the sha256 of this section's byte range, and it must reproduce.

### Commits

- Base commit: `b5e2601045fe6894db684a4292933fd1aece669f`
- `SPEC365_CODE_FREEZE`: `9fdaaf5a141bf24ee3b7801cb6a16c861ab228cc`, the commit at which the
  instrument, the `/metrics` wiring and every test were complete and the full gate matrix was green
  (`cargo fmt --all -- --check`; `cargo clippy --all-targets --all-features -- -D warnings`;
  `cargo test -p topgun-server --lib` release and debug, 1893 passed / 0 failed each; `pnpm test:sim`
  39 / 0; `scripts/check-invariants.sh`; `pnpm install --frozen-lockfile`, `pnpm -r build`,
  `pnpm -r exec jest --runInBand --passWithNoTests`, `pnpm lint`, `pnpm format:check`;
  `pnpm --filter apps-docs-astro build`).
- The literal committed in `spec365-conjuncts.sh` reads
  `SPEC365_CODE_FREEZE=9fdaaf5a141bf24ee3b7801cb6a16c861ab228cc`, which equals the value above.
  Checked mechanically, together with both digests below, immediately before this section was
  committed.

### Frozen scripts

| file | sha256 |
|---|---|
| `spec365-conjuncts.sh` | `1087ec6f33d8dcad537ae5af43d06d7a5391e55b45d01e3efec2cff5a2074591` |
| `spec365-readout.sh` | `5fbfa3e9d9df2a74830edbf89cf4036ae3e376add9171676fe70800b335d7a11` |

Neither script is edited after this section is committed.

### CSV header (D5, 41 columns)

```
elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,phys_footprint_mb,phys_footprint_peak_mb,reclaimable_mb,compressed_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,clean_mb
```

Column 41 (`clean_mb`, footprint's TOTAL-row Clean column) was added before this section was
committed, after the first smoke run showed that the footprint reconstruction could not hold without
it. It feeds the footprint check (§E) only: `|rss_mb − (phys_footprint_mb + clean_mb +
reclaimable_mb)| / rss_mb ≤ 0.02`, over the rows with `elapsed_secs > 0`. The runner reads
`footprint --swapped --format bytes`, because the default output rounds to whole MB.

### Frozen decision rules

`tau = max(0.10 × B_store, 2048)` bytes.

| outcome | condition (first match wins) | meaning | routed by the conductor to |
|---|---|---|---|
| **O4 INDETERMINATE_INSTRUMENT** | no `conjunct` line; `retained_truncated=true`; §A count mismatch; any empty cell in columns 6, 11–40 of the last row (32–40 feed §C's last-row metric check); `B_store == 0`; `ret_epochs_unslotted > 0` **and** §C's total `restored_sum == 0` (unexplained); `conj_current_epoch < 2` | the readout cannot be read | re-run (execution cap 3 total) |
| **O1 OUTSIDE_INDEX** | `X > tau'`, where `tau' = tau` normally, or `tau + n × max(bytes_returned over all removal rows)` when `ret_epochs_unslotted > 0` and §C's total `restored_sum > 0` (a restore window; `n = ret_epochs_unslotted`; see §B) | retained tags live in OR slots with no epoch-index entry; neither conjunct can reclaim them | TODO-634 (origin of unindexed tombstones); 8b cannot close the plateau |
| **O5 INDEX_EXCEEDS_STORE** | `X < −tau` | the index holds refs whose bytes are no longer resident | TODO-634 (accounting divergence) |
| **O2 INDEX_DURABILITY** | `ret_refs_durability_only + ret_refs_both > ret_refs_claim_only` | the durability conjunct binds | TODO-634 (write-behind / watermark carve) |
| **O3 INDEX_CLAIM** | otherwise | claim conjunct or structural one-epoch retention | TODO-669 (8b) |

- `O2`/`O3` are evaluated over the `ret_refs_*` columns, which the fold populates from **closed**
  epochs only (`e != current_epoch`); the open epoch never contributes to either numerator and never
  routes to O2 or O3.
- `reconciliation=RECONCILED` iff every §C row is `RECONCILED` or `IN_FLIGHT`. Otherwise it is `SPLIT`,
  and the `SPLIT` epochs are listed.
- `retained_closed_epochs` is the number of §A epochs with `open` false.

### Pre-registered prediction

- **`READOUT: O1`**, with `X/B_store ≥ 0.5` on the last row; `retained_closed_epochs ≤ 1`, and that
  epoch, if present, is `claim_only`; `reconciliation=RECONCILED`; maximum `claim_lag_max ≤ 3`.
- **Rationale:**
  - The 4 h cell exited 435 of about 437 epochs with the tracker at the frontier.
  - SPEC-357b bounded the index at about one open epoch, while 44,452 tags were resident.
- **`durable_watermark_lag`:** no prediction. It is report-only.

The harness exit code and the shipped byte-slope gate (which has fired on short cells before) are
recorded, and are **not** readout inputs. `ABORTED` means an absent `RESULT:` console line, the lineage
rule.

### Execution cap

3 executions of `conj900` in total, counting every re-run an O4 verdict routes.

### Smoke validation (precondition for this section)

Held. The runner and readout were run end to end against the release binary built at
`SPEC365_CODE_FREEZE`, on a 120 s local cell (`SPEC362B_SMOKE_DURATION=120`, 10 s cadence), with the
final committed scripts. The readout produced a non-O4 verdict from real `conjunct`, `removal` and
`settlement` lines, and §A's consistency check passed. The smoke output is scratch and is not part of
the evidence. Earlier smoke runs on prior script revisions exposed the footprint-reconstruction defect
that column 41 and the exact-byte footprint read correct.

### Known limits (pre-registered caveats on the readout's mechanical rules)

1. `op_seq` is constant within a drain pass and advances only on a stamp (`:521`); two drains with
   no OR remove between them share an `op_seq`, so §C's `IN_FLIGHT` rule ("max `op_seq`" = the
   last pass) can read an earlier pass's genuine `NO_SETTLEMENT` as `IN_FLIGHT`. The error is
   one-directional: it can hide a SPLIT but never invent one.
2. §B/O4's `ret_epochs_unslotted > 0` guard is a **last-row** value, compared against §C's
   `restored_sum` **total over the whole cell**; one restore anywhere in the cell can explain any
   number of unslotted epochs at the end. The looseness is confined to O4's guard; O1's
   restore-window bound (§B) stays sound.
3. Even with R3 step 5's `seq`/`conj_snapshots_total` pin and R7 §A's matching-line selection, the
   fallback path (no console-log line has a `seq` matching the last CSV row's `conj_snapshots_total`)
   leaves §A reading the last `conjunct` line while §B/O2/O3 read the last CSV row — two instants
   that need not coincide.

### Additional caveat recorded at implementation (not part of the specified list)

- The `seq` = `conj_snapshots_total` pin holds for serial scrapes only: the two increments are
  independent atomics, so two overlapping `/metrics` requests can interleave them. The harness
  scrapes serially, so the pin holds for this cell; known limit 3's fallback covers any mismatch.

## APPEND-ONLY BELOW

## §2 — Executed record

Execution 1 of 3 (cap). One `conj900` run: 900 s, 16 CSV rows at a 60 s cadence, started after §1
was committed (`5d91128d`); artifacts committed in `857d6cdc`.

### Verdict (§F, verbatim)

```
READOUT: O2; retained_closed_epochs=1; reconciliation=SPLIT
```

Routing per the frozen table: **O2 INDEX_DURABILITY → TODO-634** (write-behind / watermark carve).
`reconciliation=SPLIT` localises PD-F12 at the drain-return / prune-loop boundary (split epochs 10, 27).

### §A — retained epochs (`conjunct` line `seq=195`, matched to the last row's `conj_snapshots_total=195`)

| epoch | class | open |
|---|---|---|
| 29 | durability_only | false |
| 30 | | true |

Consistency: counted `claim_only=0 durability_only=1 both=0 neither=0` equals the line's counts;
`retained_truncated=false`. OK.

### §B — where the retained tag tombstones live (last row)

| quantity | value |
|---|---|
| `B_store` | 46641 |
| `ret_stamped_bytes` (closed epochs) | 22000 |
| `ret_stamped_bytes_open_epoch` | 21472 |
| `B_index` | 43472 |
| `X = B_store − B_index` | 3169 |
| `X / B_store` | 0.067945 |
| `tau` | 4664.1 |

`X ≤ tau`, so O1 does not fire and `X ≥ −tau`, so O5 does not fire; `ret_refs_durability_only >
ret_refs_claim_only` routes to O2. Durable census (post-kill redb): tombstone_entries=1454,
or_map_keys=96, keys_with_tombstones=48, max_tombstones_per_key=38.

### §C — per-exited-epoch reconciliation

27 exited epochs (2–28), one drain pass each.

| status | count | epochs |
|---|---|---|
| RECONCILED | 25 | all others |
| NO_SETTLEMENT | 2 | 10, 27 |
| IN_FLIGHT | 0 | |
| MISMATCH | 0 | |

Totals: refs_returned=27000, considered=25000, dropped=25000, matched_nothing=0, absent=0,
restored_sum=0, bytes_returned=577718, bytes_freed=534718. The last-row metric check reads
`removed_refs_observed_total=27000` against `considered_total=25000` (MISMATCH): the counters
independently confirm the two unsettled drains, 2000 refs that left the index and never reached
the prune loop. Neither split epoch is the last pass, so IN_FLIGHT does not apply.

### §D — claim lag (195 `conjunct` lines)

max `claim_lag_max` = 1; `claim_lag_p50` min/median/max = 0/0/1; `claim_lag_p99` = 0/0/1;
max `claims` = 1; `durable_watermark_lag` max = 4, last = 2.

### §E — footprint reconstruction

`rows_within_2pct=15/15` (rows with `elapsed_secs > 0`).

### Harness and provenance

- Harness exit code: **1**. Attribution: the shipped tombstone-byte slope hard gate failed
  (`27591.9 B/h` over 180 samples; peak 48049, last 44045 bytes). `finishedReason`: `tombstone-byte growth slope 27591.9 bytes/h exceeds 512.0 bytes/h: tombstone-byte growth slope 27591.9 bytes/h exceeds 512.0 bytes/h (total growth 48049 bytes over 180 samples, last-half window 446s)`.
  Recorded, and not a readout input (§1).
- `matrix.txt`: repo HEAD `5d91128d943e28e798e50130d7cc64b3498dc87e`; server binary sha256
  `27e71e98841c2b1128423186028e43ba3defc688ddbcd61c0f5547a5400b6527`, built by the runner from
  that HEAD, whose `.rs` tree equals `SPEC365_CODE_FREEZE` (the runner's D6 guard held).
- Not a readout input, recorded for the successor: `rss_mb` rose from 0.031 to 546.578 MB over the
  cell (OLS 2260.7 MB/h) while `B_store` stayed under 47 KB, so the RSS growth is not
  tombstone bytes.

### Prediction scored

| clause | predicted | observed | score |
|---|---|---|---|
| verdict | `READOUT: O1` | `READOUT: O2` | miss |
| `X/B_store` on the last row | `≥ 0.5` | 0.068 | miss |
| `retained_closed_epochs` | `≤ 1` | 1 | hit |
| class of that epoch | `claim_only` | `durability_only` | miss |
| reconciliation | `RECONCILED` | `SPLIT` (10, 27) | miss |
| max `claim_lag_max` | `≤ 3` | 1 | hit |

### §1 integrity

sha256 over §1's byte range (from the first byte of this file through the `## APPEND-ONLY BELOW`
line, inclusive), computed with
`awk '/^## APPEND-ONLY BELOW$/{print; exit} {print}' spec365-manifest.md | shasum -a 256`:
`d1da2c4aad7468d87d87d2809759b27f2b936555026a587918728bb6f2188d20`. It equals the value computed
when §1 was committed.
