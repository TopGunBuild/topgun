# `/xreview` — cross-vendor adversarial review of SPEC-356a's `.rs` diff

**Round:** R9(b) of SPEC-356a — the **implementation** round. Scoped to **this half's own diff**
(`spec356a-` basename), unlike the family-scoped `/xask` round in `spec356-xask-preregistration.md`.
**Run:** 2026-08-03, before merge. **Diff under review:** `git diff 21f19393..HEAD -- '*.rs'` —
2,201 insertions / 17 deletions across `tombstone_frontier.rs`, `tombstone_frontier_impl.rs`,
`service/domain/crdt.rs`.
**Vendor / model:** OpenRouter `z-ai/glm-5.2` via `openrouter-review.sh`.
**Cardinal rule put to it:** the prune-record instrument must be **observationally neutral** — it must
not perturb tombstone-byte accounting, must not change which refs are dropped or restored, and must add
no fold, allocation or syscall to the per-`OR_REMOVE` hot path.

**Vendor verdict on the cardinal rule: NOT VIOLATED.**

**Status of this document.** Advisory. Every finding was verified against the actual code before
disposition — line by line, not by reading the summary. Findings that survived were **fixed**; findings
that did not were **refuted-with-reason**. No finding is left undisposed.

---

## Disposition ledger

| # | Finding | Verified? | Disposition |
|---|---|---|---|
| **MED-1** | **Torn epoch-triple read, and the torn copy overwrites the correctly-locked gauges.** `prune_epoch_tombstones` read `(current_epoch, low_water_mark, durable_epoch_watermark)` through three separate accessors **after** `drain_prunable_tombstones` released the frontier lock — three independent acquisitions, tearable by a concurrent ACK. `observe_drained_epoch` then republished that triple over `current_epoch` / `low_water_mark` / `durable_epoch_watermark` / `last_drained_epoch`, the same four gauges the frontier already publishes from `observation_snapshot()` taken **under the drain's own lock**. Last writer wins, and the last writer was the torn one. | **CONFIRMED — and worse than reported.** Two doc comments claimed the triple was *"snapshotted atomically with the drain"*. It was not: `current_epoch()` → `self.lock()`, `low_water_mark()` → a cursor fold under its own lock, `durable_epoch_watermark()` → `refreshed_watermark()`, which **recomputes from the store**. A doc-contract asserting a property the code does not have is the false-invariant hazard CLAUDE.md names, and here it would have made a 4 h characterization run's epoch-state correlation silently wrong. | **FIXED at source** (`01230749`). The triple is dropped from `PruneEpochRecord` and from the caller; `observe_drained_epoch` records only its three distributions; the locked publish via `observe_epoch_state` is now the **only** writer of those gauges. The struct doc now states *why* the triple is not carried. Bonus: three lock acquisitions and a watermark recomputation per non-empty drain are gone, which lowers perturbation. |
| **MED-3** | **Double cursor fold under one lock.** On an advancing ACK, `refresh_low_water_mark` computes the LWM, then `split_observation` immediately recomputes it via `self.low_water_mark()` — a second min-fold over the same cursor map, under the same acquisition. | **CONFIRMED.** `refresh_low_water_mark` stores the result in `observed_lwm`, then `split_observation` folded again for an identical answer. | **FIXED** (`01230749`). `split_observation(&self, lwm: Epoch)` now takes the value; the three LWM-movement callers pass the just-computed `state.observed_lwm`, and the drain caller passes `state.low_water_mark()`. Lock hold time on advancing ACKs drops back toward its pre-instrument shape in the cursor-count dimension. |
| **LOW-1** | **Phantom LWM advance after a recovery rebuild.** `rebuild_into_epoch` re-seeds the index but leaves `observed_lwm` at its stale value (0 on a fresh process), so the first post-recovery read computes `lwm − 0` and reports the entire recovered low-water mark as one advance. | **CONFIRMED.** `rebuild_into_epoch` resets `epoch_tags`, `epoch_max_seq`, `indexed_refs` and `durable_epoch_watermark`, but not `observed_lwm`. | **FIXED** (`01230749`). The advance baseline is re-seeded from the recovered fleet position at the end of the rebuild. Impact was bounded — SPEC-356b's cells run `crash-interval 0` from a fresh store, so the LWM starts at 0 — but the counters feeding the retention-ceiling evidence must not carry a burst no client earned. |
| **MED-2** | **`Vec` allocation + O(indexed epochs + tracked claims) fold held under the lock on every LWM advance.** `split_observation` collects `claim_lags` into a heap `Vec` inside the frontier `Mutex`, blocking concurrent `stamp_tombstone`. | **CONFIRMED as described, but WITHIN the declared budget.** R3's perturbation budget licenses index-proportional work **on the LWM-movement path and on non-empty drains** — explicitly *never* per `OR_REMOVE` — and this is that path. G1's design note ("per-claim lags are a borrowed slice, not a struct field, so the capture allocates nothing") is about **not retaining** them in the record, which still holds. | **REFUTED-WITH-REASON (accepted, not fixed).** The lags must exist as a slice to be passed at all; the only ways to avoid the allocation are to emit **inside** the lock (strictly worse) or to snapshot the cursor map first (the same allocation, one indirection earlier). MED-3's fix already removes the larger share of the added hold time. Recorded so a later reader does not re-raise it. |
| **LOW-2** | **`now_millis()` syscall on every `confirm_apply_ack`**, including replays and duplicates, taken before the lock and before knowing the ACK's disposition. | **CONFIRMED as described; deliberate.** The stall gauge is refreshed on **every** ACK by design — the regime worth seeing is the one where the mark is **not** moving, and a gauge that only ticked on an advance would freeze exactly when it starts to matter. Not on the `OR_REMOVE` path, so the cardinal rule is untouched; the drain path already reads the clock only on its non-empty branch. | **REFUTED-WITH-REASON (accepted, not fixed).** Removing it would blind the one series whose whole purpose is to measure the absence of movement. |

## Invariants the vendor independently verified and found HOLDING

Recorded because a review that only ever reports defects loses the information that the load-bearing
properties were checked by a second pair of eyes:

- **Exactly one `sub_tombstone_bytes` call**, in the post-write `Ok(_)` arm, behind `dropped`, unmodified by the ledger.
- **`apply_or_delta` untouched** — no tombstone-byte counter, no metrics counter.
- **Exit-ledger exhaustiveness** — every path through the loop body increments exactly one of the six exit counters; the `if !ran` / `else if !dropped` restructure preserves mutual exclusivity.
- **`indexed_refs` accounting** — stamp (+1), drain (`saturating_sub` by `refs.len()`), restore (+1), rebuild (re-seed). **No underflow path.**
- **Disarmed path** — `NullPruneRecorder`'s methods are empty bodies: no allocation, no atomic, no metrics call. `TOPGUN_PRUNE_RECORD` read once at construction.
- **Handle caching** — all 33 series resolved once in `MetricsPruneRecorder::new` and touched eagerly; no per-call-site `counter!`/`gauge!`/`histogram!` macro in any observation method.
- **No guard held across `await`**; **no lock-ordering hazard** between the frontier `Mutex` and the key-writer guard.

## Gate re-run after the fixes

```
cargo check   -p topgun-server                                   -> Finished, 0 errors
cargo fmt --check                                                -> FMT_OK
cargo clippy  -p topgun-server --all-targets --all-features -D warnings -> Finished, 0 warnings
cargo test --release -p topgun-server --lib                      -> 1790 passed; 0 failed; 2 ignored
```

**One intermittent recorded rather than hidden:** on the first post-fix full-suite run,
`storage::crash_safety_proptest::crash_on_partial_active_segment_recovers_intact_prefix` failed. It
passed **3/3 in isolation** and the full suite passed clean on re-run (1790/0). It is a WAL
crash-safety proptest, outside this diff's blast radius (which touches only the tombstone frontier and
`prune_epoch_tombstones`), and the same suite was green on the same test before this diff. Classified as
a pre-existing intermittent under full-suite load, **not** as a consequence of these changes — and
written down so the next contributor who sees it has a prior.
