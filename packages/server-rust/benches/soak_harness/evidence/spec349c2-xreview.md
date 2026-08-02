# Cross-vendor adversarial review — SPEC-349c2 Fix Response v1

**Model:** `z-ai/glm-5.2` via OpenRouter. **Diff reviewed:** `8d824020..HEAD` (874 lines) — the
Review v1 fix set. **Cost:** $0.0368 (13,483 prompt / 14,221 completion tokens).

Committed because SPEC-349c2's Review v1 (minor 9) found AC9's earlier `/xreview` asserted with no
artifact, and therefore unverifiable. A cross-vendor pass whose output is not committed is the same
artifact-mortality class this fix set exists to close.

**Cardinal-rule violations: none.** 4 findings, each verified against the diff and the code at HEAD
before being accepted or refuted. **2 applied, 2 refuted on evidence.**

---

## HIGH — "empty cells corrupt the fit; a real breach could false-PASS" → **REFUTED**

**Claim.** In `spec349c2-fit.awk`, the new `skipped++; next` handler skips array storage while the
END block still windows over `n` = *total* rows, so indices `[m .. n-1]` read as uninitialized `0`
and anchor the regression at the origin, depressing the slope below the 512 B/h gate.

**Why it is wrong.** The premise misreads which counter `n` is. `n` is incremented **only on the
stored path**, after `t[n]`/`y[n]` are assigned — an empty cell takes `next` *before* reaching it.
So `n` is the count of stored rows, the arrays are dense over `[0, n)`, and window and fit are
consistent. There are no uninitialized indices to read as zero.

**Measured refutation**, on a 10-row smoke CSV with one empty cell:

```
col=tombstone_bytes window=full rows_used=9 n=9 skipped_empty=1
  t_start_secs=10.0 t_end_secs=90.0 span_secs=80.0
```

`t_start_secs=10.0`, not `0.0`; the span is positive; 9 points were fitted, not 10. Under the
claimed defect the first point would have been `(0, 0)` and the span negative.

**Second, independent reason the consequence could not follow:** this column is characterization,
not the gate's input. The tombstone verdict is computed by the harness's own in-process sampler and
emitted as `soak.json`'s `tombstones` object; the shell-scraped CSV column cannot move a verdict at
all.

**Kept from it:** the output field named `rows_total` was genuinely misleading — it prints stored
rows, not total rows. Renamed to **`rows_used`** with a comment stating that empty cells are absent
rather than folded in as zeros. *(applied)*

## MED — "partial scrape degradation is invisible" → **APPLIED as documentation, not as a threshold**

The population check fails only on a wholly empty column, so a 700/720 column passes. Correct as
stated. A ratio threshold was **not** added, because the disposition rests on the same fact as
above: a degraded column leaves the committed series thinner, it does not move a verdict. Holes are
already visible in two places — `empty=` in the post-run column report, `skipped_empty=` beside
every fitted slope. The reasoning is now written at the check itself rather than left implicit.

## MED — "new required `SoakReport` fields break deserialization of prior artifacts" → **REFUTED**

The finding presupposes a `Deserialize` impl that does not exist. Every report struct in `report.rs`
derives `Debug, Clone, Serialize` — read-back into these types is not possible today with or without
the new fields, so no `#[serde(default)]` can be missing from it. Prior artifacts are read with
schema-free tools (`jq`, `python -m json`), which are unaffected by added keys.

## LOW — "float truncation only handles a trailing `.0`" → **APPLIED**

`sub(/\.0+$/, "", v)` strips only dot-followed-by-zeros, so a `12345.5` rendering survives with its
dot, fails the caller's integer guard, and is silently discarded as an unreadable cell. Replaced
with `int($2 + 0)`, which matches what the harness's own Rust parser does (parse `u64`, else parse
`f64` and truncate) — so both instruments now read the same wire text the same way.
