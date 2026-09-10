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
