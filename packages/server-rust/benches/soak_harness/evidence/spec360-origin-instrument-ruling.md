# The removal-site ORIGIN instrument — frozen ruling surface

**Subject.** The `Some(vec![])` state of `TombstoneFrontier::epoch_tags`: an epoch whose
`epoch_tags` entry EXISTS but holds an EMPTY vector. Its reachability is SETTLED AFFIRMATIVELY by the
2026-08-17 cross-vendor §C ruling (committed verbatim beside this file as
`spec360-xask-answer-3.txt`, sha256
`79a8e8b23a64d387edd92cc0bdb1d1369907738712f00051529bf1c39f5cd7d8`) and may not be re-asked. What is
open is the state's **ORIGIN**, and the instrument this artifact freezes the contract for is the one
deliverable that can resolve it.

**Two layers, and the distinction is load-bearing.**

1. The **FROZEN** layer — everything above the `<!-- FROZEN-LAYER-END -->` marker. It carries the
   Decision Table (`DT`), the Fix-Shape Mapping (`FS`), the instrument's extractor CONTRACT, the
   writer record and its reconciliation, both horns of the origin paradox, `X21`'s three limbs, the
   join key, and the pre-registered dispositions. It is digest-recorded **before** the first drive or
   emission exists in the tree, and the commit that adds it **precedes** every commit that adds a
   drive or the instrument.
2. The **APPEND-ONLY post-section** — beneath the marker at the foot of this file, written but empty
   at freeze time, carrying its OWN separate digest once it is written.

**Nothing above the freeze marker is edited after the freezing commit. Appends beneath the
post-section marker are the only writes this artifact accepts.** A defect discovered in the frozen
layer afterwards is published in the post-section as a separate fact, and the table is walked as
written. This is the two-layer shape `spec356-manifest.md` §12.0 / §12.1 established in this
lineage; predecessors in the evidence chain are `spec356-manifest.md`, `spec357b-trackergrade.ref`
and this directory's `spec359-*` records.

**A framing this artifact is bound by and does not contradict anywhere.** The 95 % → 33 % "falling
reclaim fraction" is a ratio over the counted path's share — an instrument-derived quantity — and is
**not** evidence that total reclamation degrades with width. Nothing here asserts, implies or builds
on a claim in either direction about whether reclamation plateaus; that question is **not established
either way** and its re-measurement from the durable layer (RSS + redb file size + WAL segment
retention + a store-level live-vs-dead tombstone census, ignoring every registry counter) belongs to
the measurement increment `TODO-654`.

---

## 1. The writer record — Fact A's mechanical basis, re-verified at HEAD

Re-read at HEAD (`packages/server-rust/src/tombstone_frontier_impl.rs`) while freezing this artifact.
**Every one of the five sites and every line number below was confirmed unchanged; no drift was
found.**

| Variant | Site (verified at HEAD) | Post-state | §C's four-item form |
|---|---|---|---|
| `Stamp` | `:524-531` `self.epoch_tags.entry(epoch).or_default().push(TombstoneRef { … })` | `len >= 1` | "pushed at stamp (`:524`)" |
| `Restore` | `:1036-1039` `self.epoch_tags.entry(epoch).or_default().push(tombstone_ref)` | `len >= 1` | "pushed at … restore (`:1036`)" |
| `RebuildClear` | `:855` `self.epoch_tags.clear()` | entry **absent** | "cleared by rebuild (`:855`)" |
| `RebuildInsert` | `:867-870` `if !live.is_empty() { self.epoch_max_seq.insert(e_rec, 0); self.epoch_tags.insert(e_rec, live); }` | guarded; `len >= 1` | folded into "rebuild" |
| `DrainRemove` | `:979` `self.epoch_tags.remove(&e)` | whole entry removed | "removed by drain (`:979`)" |

**Reconciliation with §C.** §C enumerates four items — pushed at stamp (`:524`) and restore
(`:1036`), cleared by rebuild (`:855`), removed by drain (`:979`) — and concludes *"all pushes are
non-empty"*. This record splits §C's single "rebuild" item into `RebuildClear` and `RebuildInsert`
because the two sites have **different post-states** (absent vs guarded-non-empty) and must therefore
be witnessed separately. **The two enumerations describe the same set of mutations**; the
five-variant form is the one this artifact keeps.

**This record is NOT a verdict.** It is Fact A of the origin paradox. It is guarded mechanically by
`W7` rather than asserted, and it is offered as an INPUT, never as an answer to the origin question —
a static enumeration cannot answer that question, which is precisely why the instrument exists.

Supporting anchors re-verified at HEAD in the same read: `drained_epochs.insert(e)` at `:986`
(unconditional on `refs.len()`); `let removed_refs = …` at `:990`; `removed_observed.insert(e, …)` at
`:995`; `drained.extend(…)` at `:996`; `let watermark = self.durable_epoch_watermark;` at `:934`;
`let ceiling = token.ceiling();` at `:953`.

---

## 2. The ORIGIN PARADOX — both horns, named

Two established facts are in direct tension, and **at least one of them must be incomplete**.

**Fact A.** No enumerated writer of `epoch_tags` at HEAD leaves an entry present-and-empty. `Stamp`
and `Restore` push after `or_default()` (`len >= 1`); `RebuildInsert` is guarded by
`if !live.is_empty()`; `RebuildClear` and `DrainRemove` leave the entry **absent**, not empty.

**Fact B.** The 8 h data entails that the state IS reached in production. The chain, each conjunct
independently citable at HEAD:

1. `considered` increments **once per ref the drain RETURNED** (`service/domain/crdt.rs:1539`, inside
   `for (epoch, r) in drained`), so `considered Δ = 0` means `drained` was empty on every pass in the
   window.
2. `DrainedByPrune` is assigned **only** where `drained_epochs.contains(&e)`
   (`tombstone_frontier_impl.rs:1009-1011`), and `drained_epochs` is written **only** inside the
   `Some(refs)` arm (`:986`). The one route that could have manufactured the attribution without a
   removal — `EpochExitKind`'s `#[default] DrainedByPrune` — is refuted at source:
   `finalize_epoch_exit` resolves `kind_hint == None` to `Unclassified` (`:738-751`), and the
   `#[default]` is reachable only from `..Default::default()` in test fixtures.
3. `drain_prunable_tombstones` has exactly **ONE** production caller (`crdt.rs:1521`), so no exit row
   can be emitted by a drain that no pass record wraps.

Given 1–3, 440 `DrainedByPrune` exits with `considered Δ = 0` entail 440 removals that returned
`Some(refs)` with `refs.len() == 0`.

**The horns.**

- **`H-origin-1`** — the writer enumeration is incomplete in a way a static read cannot see: a route
  that empties a vector without being one of the five enumerated mutation expressions, or an
  interleaving that produces the state without any single writer producing it.
- **`H-origin-2`** — the inference chain is pin-specific. The 8 h cells ran on SPEC-357-era binaries
  (`feb85268…` / `8a60f135…`); SPEC-359 has since rewritten the drain's bracket
  (`begin_sweep` / `end_sweep`, the hoisted `ceiling`, `prune_ceiling()` as the boundary authority).
  The instrument lands at **HEAD** and will be read at **HEAD**. If the origin was pin-specific, the
  instrument may observe nothing — **which is an informative result, not an instrument failure**, and
  is pre-registered as such in §8 so the measurement increment cannot misread silence.

**Neither horn is chosen here.** Choosing one would be a naming, and no data in this increment's
scope can support one.

---

## 3. `DT` — the DECISION TABLE (frozen)

Evaluated in **this order**. `R4` is first and fail-closed.

| Row | Antecedent (frozen) | Verdict | Consequence |
|---|---|---|---|
| **R4** | Step 0 fails on any leg: an unmutated suite is not green, **or** a mutation arm does not RED **against the assertion the witness set sites it on** — `W2` → **the EXIT ROW**; `W5` → **the rendered `refs_returned`**; `W6` → **its selected arm's rendered line (PRIMARY) or its arm's render (FALLBACK)**; and **NO mutation arm is graded against `X21-c`'s Δ** — **or** the planted positive control does not fire, **or** the negative control does not show the line absent | `INDETERMINATE-INSTRUMENT` | **Fail-closed.** `R1`–`R3` are `NOT EVALUATED`. No verdict, no fix, no armed instrument shipped. The failed leg is named and routed to `TODO-634` by id. |
| **R1** | Obligation **A** holds — the planted antecedent reproduces `AttributedWithoutObservation` **through the SERVICE**, with **each term read over the transport that carries it**: `considered = 0 ∧ empty_drain = true` off the **rendered pass row** (`crdt.rs:1740-1746`, `X21-b`) **and** `epochs_drained` contributing **Δ = 0** to `topgun_or_prune_epochs_drained_total` off the **Prometheus render** (`X21-c`), because that field has **no `tracing` transport at HEAD** and this increment adds none — **AND** obligation **B** holds (the instrument is delivered at the adopted siting with the adopted field set, and its emission is **proven correct over the rendered transport**, including `W5`'s observation arm, `W4`'s negative control, `W8`'s positive control and `W6`'s interleaving leg **on whichever of its two pre-registered arms `G5` selected**) | `CONFIRMED-AND-ARMED` | Publish. Land `FS` row `R1` — the two doc-contracts and nothing else. **Route the ORIGIN question, with the instrument armed, to the measurement increment — `TODO-654`, by id** — carrying both horns of the origin paradox, the `NOT-OBSERVED-AT-HEAD` pre-registration and the null-read disposition (§8). |
| **R2** | Obligation **A** holds **AND** obligation **B** does not (the line cannot be proven to emit correctly over the rendered transport) | `ARMED-UNPROVEN ⇒ NOT ARMED` | The instrument is **NOT shipped**. An emitter whose rendered form is unproven is exactly the class where a wire format defeats its own extractor, and shipping one would hand the measurement increment a transport that may silently produce nothing. Land no fix; publish what is and is not established; route both halves. |
| **R3** | Obligation **A** does **not** hold | `NOT-REPRODUCED` | The fix is **NOT WRITTEN** — the rule firing, not an omission. **AND — this row's principal content:** §C's inference chain (conjuncts 1–3 in §2) *predicts* exactly this pair, so a non-reproduction **contradicts the chain**, and that contradiction is published as this increment's headline finding, with the failing conjunct identified if identifiable. Route to the **conservative sweep** (extraction synthesis §7 F5, the RisingWave `start_full_gc` shape), pre-registered here so it is not decided at execution. |

**`DT` is not editable after this digest is recorded.** Every row this increment prints is computed
live through `classify_drain_attribution` (`packages/server-rust/src/tombstone_frontier.rs`) rather
than asserted in prose.

**The classifier's reading, frozen with the table.** Attribution side =
`exit.bytes_freed_attributed > 0`. Observation side = the **disjunction**
`exit.removed_refs_observed > 0 || pass.considered > 0 || !pass.empty_drain`. The four classes are
the 2×2 fold of those two booleans: `(true, true) → ObservedAndCounted`,
`(true, false) → AttributedWithoutObservation`, `(false, true) → ObservedWithoutAttribution`,
`(false, false) → Silent`. The observation side is a disjunction deliberately, so the divergent
verdict may only be named when **every** channel through which an observation could have surfaced is
silent — which makes `AttributedWithoutObservation` fail-closed rather than merely likely.

---

## 4. `FS` — the FIX-SHAPE MAPPING (frozen)

The distinction it turns on, stated normatively because the whole hazard lives here.

- **Defect (i) — ATTRIBUTION.** An empty removal is reported as a **successful drain**:
  `bytes_freed_attributed = slot.stamped_bytes` (`tombstone_frontier_impl.rs:757-758`) and
  `drained_refs_total += slot.refs_at_entry` (`:733`) are **entry-side** quantities — §C calls the
  attribution *"tautological (same field as stamped bytes) … perfectly self-consistent and perfectly
  misleading"* — while on the service side `PrunePassRecord` cannot distinguish *"no epoch was
  eligible"* from *"an eligible epoch was removed and returned zero refs"*. Fixing (i) makes the
  **instrument honest**. **It frees zero bytes.**
- **Defect (ii) — RECLAMATION.** Whether the **store content** behind those refs is ever dropped.
  Under the antecedent the refs leave the index before reaching `prune_epoch_tombstones`, so their
  durable tombstones are never dropped. Fixing (ii) frees bytes.

**Normative anti-suppression clause.** The candidate fix is **NOT** making `drained_epochs.insert(e)`
at `:986` conditional on `!refs.is_empty()`. That relabels the exit and **suppresses the signal**
rather than fixing the reclamation: the refs are still gone from the index, the durable content is
still un-dropped, and the ledger simply stops saying so — and it would delete the very rows the new
instrument exists to explain. **Any implementation whose whole content is that conditional FAILS.**

| Row | (i) attribution | (ii) reclamation |
|---|---|---|
| `R1` | **PARTIALLY DELIVERED, and the verdict says exactly that.** The observation terms already exist — `removed_refs_observed` / `removed_bytes_observed` landed earlier in this lineage — so the residual (i) gap is **service-side blindness** and **naming**. Delivered as: the removal-site line (obligation B's deliverable, which closes the blindness at the frontier), plus two doc-contracts naming `bytes_freed_attributed` / `drained_refs_total` as entry-side and `considered` / `empty_drain` / `epochs_drained` as unable to discriminate. **NO existing metric series' value or meaning changes.** | **NOT DELIVERED. ROUTED.** The origin is unknown; a reclamation fix without a named origin is exactly the wrong-shaped fix this family exists to avoid. Routed to the measurement increment (`TODO-654`) and thence to a fix carve. The **conservative sweep** remains the backstop shape and is out of scope. |
| `R2` | **NOT DELIVERED.** | **NOT DELIVERED.** |
| `R3` | **NOT WRITTEN.** | **NOT WRITTEN.** Route to the conservative sweep. |
| `R4` | **NOT WRITTEN.** | **NOT WRITTEN.** |

**No existing metric series' value or meaning is changed by any row.** The measurement increment must
be able to compare its reads against this lineage's existing rounds; silently re-pointing
`bytes_freed_attributed` at the observed total mid-lineage would break that comparability. The
honesty repair is a **doc-contract plus a new line**, never a mutated series.

---

## 5. The instrument's extractor CONTRACT (frozen)

This section is the contract a later reader — and the measurement increment's extractor — is entitled
to rely on. It closes, **at source**, the class where an emitter's wire format defeats its own
extractor.

### 5.1 Target

The instrument emits under its **own** `tracing` target, distinct from every existing one, so the
line is selectable **without** a `kind` discriminant field — a siting choice that avoids widening the
field set:

```
topgun_server::tombstone_frontier::removal
```

Existing neighbouring targets it must not be confused with, both in use at HEAD:
`topgun_server::tombstone_frontier::residency` (the epoch entry row, the epoch exit row, and the
pass row at `crdt.rs:1740-1746`) and `topgun_server::tombstone_frontier::settlement` (the per-epoch
settlement row at `crdt.rs:1715-1727`).

### 5.2 The eight fields, by NAME, in order

**There is NO JSON blob. One `tracing` field per struct field.** The count is not asserted; the
**names** are.

| # | Field name | Type | Source at the site | Note |
|---|---|---|---|---|
| 1 | `ts` | see §5.4 | subscriber stamp **or** explicit `i64` Unix ms | **Display only. NEVER a join key.** |
| 2 | `op_seq` | `u64` | `self.op_seq` | secondary ordering term |
| 3 | `epoch` | `Epoch` = `u64` | `e` | **the join key**, populated on every row |
| 4 | `refs_returned` | `u64` | `removed_refs` (`= refs.len()`) | **the OBSERVATION** |
| 5 | `refs_at_entry` | `u64` | `self.epoch_slots.get(&e).map_or(0, \|s\| s.refs_at_entry)` | the **entry-side** term, carried beside the observation so the divergence is visible on one line |
| 6 | `bytes_returned` | `u64` | `removed_bytes` | the observed byte total |
| 7 | `watermark` | `Epoch` = `u64` | the `watermark` local (`:934`) | |
| 8 | `ceiling` | `Epoch` = `u64` | the `ceiling` local (`:953`) | the licence the sweep token carried |

**No field is `f64`.** Every numeric field is an unsigned integer count, byte total or epoch id; the
only signed type admitted anywhere in the set is `ts` in its explicit rendering, which follows the
tree's existing `entered_at_unix_ms: i64` convention. The field set is **§C's** and may **NOT** be
widened, narrowed or relocated. No `kind` field is added; the target does that work.

### 5.3 The separator rule, and the row's rendered shape

The sited reader is the capture already living in `tombstone_frontier_impl.rs`'s test module —
`FieldTextVisitor` (`:5152-5163`), `EventCapture` (`:5167-5182`), `captured_tracing_events`
(`:5186-5194`). Its rendering is:

- Each recorded field is appended as **`name=value` followed by exactly one ASCII space** — the
  visitor writes `"{name}={value} "` for strings and `"{name}={value:?} "` for everything else, so a
  `u64` renders as bare decimal digits.
- Fields accumulate **in the order the emitting macro declares them**, i.e. the order of §5.2.
- The row is **prefixed** with the target, and this is where the two in-tree visitors differ:

| Visitor | Prefix format | Rendered row begins |
|---|---|---|
| `tombstone_frontier_impl.rs:5178` | `format!("target={} ", …)` — **NO leading space** | `target=topgun_server::tombstone_frontier::removal ts=…` |
| `sim/tombstone_gc_proof.rs:815` | `format!(" target={} ", …)` — **WITH one leading space** | ` target=topgun_server::tombstone_frontier::removal ts=…` |

**PINNED: the sited reader for obligation A's drive and for the instrument's rendered-text proofs is
`tombstone_frontier_impl.rs`'s visitor, so the form consumed there is the NO-leading-space form.** A
reader that matches `" target="` will not match a row captured by that visitor. On `W6`'s PRIMARY
arm, where the assertion lives in `sim/tombstone_gc_proof.rs`, the **leading-space** form is the one
consumed. This is a rendered-form difference of exactly the class this contract exists to pin, and
neither form is "the" form — the reader's own visitor decides.

**Field-boundary reading rule.** A term is read from a row that has **already been selected by its
`target=` prefix**, and a match is made against `name=value` **including its trailing space**, with
the field's start anchored at the row start or at a preceding space. Matching a bare substring
without those anchors admits prefix collisions across fields and across rows and is not a compliant
read.

**No JSON blob, restated because it is the load-bearing half of this section:** every field above is
an individual `tracing` field. Nothing on this line is a serialized structure, and no consumer should
attempt to parse one.

### 5.4 `ts` — BOTH permitted renderings, pre-registered

Both of the following are compliant, and **both are pre-registered here, before the instrument
exists**:

- **(a) Subscriber stamp** — `ts` is not a recorded field at all; the configured subscriber renders
  its own timestamp. Under this rendering the line carries **seven explicit fields** and the capture
  above (which records only fields, not subscriber-formatted metadata) will show seven `name=value`
  pairs and **no `ts=` pair**. This is compliant.
- **(b) Explicit field** — `ts` is recorded as an explicit `i64` Unix-ms field, rendered
  `ts=<digits> ` like any other. Used when the configured subscriber does not render one.

The remaining **seven** fields are **always explicit** under both renderings. **Which of the two
shipped is recorded in the append-only post-section** once the instrument renders, because that is
exactly the kind of rendered-form detail this contract exists to make non-ambiguous.

**`ts` is NEVER a join key under either rendering.**

### 5.5 The slot-absent reading, pinned so it is not ambiguous at read time

A restored-then-re-drained epoch has **no slot** — its slot was retired by `finalize_epoch_exit` on
the first exit — so `refs_at_entry` renders `0` for it. Therefore:

- **`refs_returned > 0 ∧ refs_at_entry == 0`** — the signature of a **restored-then-re-drained**
  epoch. **Not** an origin observation.
- **`refs_returned == 0 ∧ refs_at_entry > 0`** — **THE ORIGIN SIGNATURE**: the removal returned an
  empty vector for an epoch that entered the index holding refs.

**No extra field is added to disambiguate.** The field set is §C's and is not widened; the
disambiguation is this reading rule.

### 5.6 The instrument's siting and arming, frozen with the contract

- **Exactly one emission site**, inside `drain_prunable`'s `Some(refs)` arm (`:979`–`:996`), adjacent
  to the existing `removed_refs` / `removed_bytes` computation (`:990-994`) and **strictly before**
  `drained.extend(…)` consumes the vector at `:996`. **No second site anywhere.**
- It **reuses** `removed_refs` and `removed_bytes`, recomputes nothing, and introduces no new
  index-proportional fold.
- It fires only for **eligible** epochs (`:963`) — never on the dark fast path (`:941`), never on a
  refused `begin_sweep` (`:948`).
- **Unconditional. No new env knob.** An arming switch would create a mode in which the measurement
  increment reads nothing and cannot tell why, which is the failure this instrument exists to
  prevent.
- **NOT registered with the metrics registry.** §C deliberately routed it to the log sink and away
  from the metrics registry. No `counter!` / `gauge!` / `histogram!` / `describe_*` is added for it,
  and `PruneRecordObserver` gains no method.

### 5.7 The FULL rendered Prometheus series names `X21-c` reads

Named in **full rendered form**, because the in-tree readers key on the exact series name and an
absent series is a **hard error** rather than a zero (`rendered_counter`,
`sim/tombstone_gc_proof.rs:1652-1659`; `rendered_value` returns `Option<&str>`,
`tombstone_frontier_impl.rs:2895-2900`):

```
topgun_or_prune_considered_total
topgun_or_prune_empty_drains_total
topgun_or_prune_nonempty_drains_total
topgun_or_prune_epochs_drained_total
```

(Constants `METRIC_PRUNE_CONSIDERED_TOTAL`, `METRIC_PRUNE_EMPTY_DRAINS_TOTAL`,
`METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL`, `METRIC_PRUNE_EPOCHS_DRAINED_TOTAL`, all in
`tombstone_frontier.rs`.)

**The `Some("…")`-never-`None` reading rule.** Every assertion over these series must be written
against `Some("…")`. `rendered_value` returns `None` for an absent series, and an assertion that
tolerates `None` reads an absent series as a zero — which is precisely how a recorder bound in the
wrong order passes vacuously. **An absent series must RED.**

### 5.8 `W7`'s pinned `include_str!` scan surface, and the gap it does not close

`include_str!` takes **literal paths resolved relative to the including file**, so the
enumeration-completeness source test can only see the files it names. The pinned list, each path
relative to `packages/server-rust/src/tombstone_frontier_impl.rs`:

```rust
include_str!("tombstone_frontier_impl.rs")   // every `self.epoch_tags` mutation lives here
include_str!("service/domain/crdt.rs")       // the one production caller lives here
```

**The claim, at exactly the strength the mechanism supports.** `W7` REDs when a **sixth**
`self.epoch_tags` writer appears **in `tombstone_frontier_impl.rs`**, or when a **second**
non-`#[cfg(test)]` caller of `drain_prunable_tombstones` appears **in either scanned file**. It is
**not** a whole-crate guarantee.

**The named gap.** A production caller added in a **third, unscanned file would NOT RED.** That gap
is named here rather than papered over. Closing it — a `std::fs` walk of `src/` from
`CARGO_MANIFEST_DIR`, or an include list asserted against a directory listing — is **out of scope**
for this increment and is routed **to `TODO-634`, BY ID** (not to `TODO-654`): a source-scan
completeness guard is a **family** concern, not part of `TODO-654`'s durable-layer read. The routing
is recorded in the append-only post-section, by id, without editing the tracker.

Two facts hold the scope honest at HEAD, both re-verified while freezing this artifact:
`self.epoch_tags` has exactly **five** mutating production sites, **all** in
`tombstone_frontier_impl.rs`; and `drain_prunable_tombstones` has exactly **one** production caller,
`crdt.rs:1521`, every other hit being inside `#[cfg(test)]`.

---

## 6. `X21`'s three limbs, and the join key

### `X21-a` — record transport. RE-SITED, and its epistemic status changes with it

The premise that obligation A's drive can hold the `PruneEpochResidencyRecord` and the
`PrunePassRecord` **in-process** is **FALSE on the service path at HEAD** and is **withdrawn**:
`prune_epoch_tombstones` returns `()` and never yields its `PrunePassRecord` local;
`drain_prunable_tombstones` (`:2057`) returns `Vec<(Epoch, TombstoneRef)>` and consumes the exit
records internally via `publish_epoch_exit` (`:2100-2102`); the only observer seam is
`prune_observer`, a `Box<dyn PruneRecordObserver>` selected from `TOPGUN_PRUNE_RECORD` inside
`TombstoneFrontier::new` (`:1329`, `:1354-1357`), with **no injection constructor**.

Therefore **the record-shaped assertions bind to the RENDERED transport.** Both records are
**reconstructed from the rendered `tracing` rows** — the exit row and the pass row
(`crdt.rs:1740-1746`) — captured in-test, exactly as the in-tree precedent at
`sim/tombstone_gc_proof.rs:978-982` does for a service-path drive. **NO injection constructor and no
capturing-observer seam is added to `TombstoneFrontier`**, and no `pub` surface is added to make the
drive possible: that would be a surface change whose only consumer is a test. **Surface ≢ transport
here**, and the classifier is fed **reconstructed** values, so `X21-a` is a **rendered-transport**
limb, not a struct-transport one.

**Its rendered-row scope is bounded by what the rows actually render, and one term falls outside
it.** The pass row carries exactly `kind`, `considered`, `empty_drain` — and **not**
`epochs_drained`, which `crdt.rs:1704` sends only to `observe_pass` and thence to the Prometheus
counter. `epochs_drained` is therefore **re-attributed to `X21-c`** and is **not** among the terms
`X21-a` reconstructs. Reading it off a row would panic in the in-tree reader (`row_field`: *"field
{name} is absent from row"*, `sim/tombstone_gc_proof.rs:844-851`), and adding it to the row is a
production edit this increment forbids: **the rendered pass row is NOT widened on any row of `DT`.**

### `X21-b` — line transport. THE LOAD-BEARING LIMB

Assertions written against the **RENDERED** `tracing` text for (a) the new removal line, (b) the pass
row (`crdt.rs:1740-1746`) and (c) the settlement row (`crdt.rs:1715-1727`). The extractor CONTRACT of
§5 is the **frozen** half of this limb; the **exact rendered SAMPLE** is the append-only
post-section's. This closes the extractor-defeat class **at source**, never by a downstream adapter,
which is the whole reason §C routed the instrument to the log sink. `W4`'s negative control and
`W8`'s positive control are part of this limb.

### `X21-c` — metrics transport. REASONED N/A for the NEW LINE; LOAD-BEARING for `epochs_drained`

Two **disjoint** scopes, and conflating them is an error this section exists to prevent.

- **The new instrument has NO metrics transport — reasoned N/A.** §C deliberately routed it to the
  log sink and away from the metrics registry, so asserting a metrics transport for it would
  fabricate a consumer that does not exist. No `counter!` / `gauge!` / `histogram!` / `describe_*` is
  added for it.
- **`epochs_drained` IS read here, and ONLY here — LOAD-BEARING.** The limb is retained for
  **obligation A**, over the **existing, unchanged** pass-counter family named in full in §5.7.
- **WHY the Prometheus transport is readable for this witness.** The in-process drive binds its local
  recorder **BEFORE** the frontier is constructed, and `MetricsPruneRecorder` resolves its handles
  **once, at construction** (`:2443-2447`), touching each series with `increment(0)` so it renders
  from the first scrape (`:2413-2421`). The recorder-binding gap that makes `tracing`'s
  `set_default` a thread-local-only transport therefore **does not apply here** — the same asymmetry
  SPEC-359's `R7` exploits when it reads `rendered_counter` off a spawned, `multi_thread` sweep
  (`sim/tombstone_gc_proof.rs:1878-1892`).
- **What makes this limb RED — at exactly the strength the mechanism supports.** The asserted
  **Δ = 0** on `topgun_or_prune_epochs_drained_total` fails when:
  1. **the plant did not take** — no eligible epoch was removed as planted; or
  2. **the drain returned refs** — either of (1) or (2) moves the series; or
  3. **the series renders ABSENT** under an inverted recorder binding, so `rendered_value` returns
     `None` and the `Some("…")` reading rule of §5.7 fails on it.

  Neither failure mode is silent.

  **`W2`'s mutation is NOT a RED trigger for this limb, and the limb does not claim it is.** `W2`
  gates `drained_epochs.insert(e)` at `:986`, and `drained_epochs` feeds **only** the exit
  attribution (`:1009-1011`); the returned `drained` vector is built independently at `:996` from the
  removed `refs` and is what `per_epoch` — hence `pass.epochs_drained` (`crdt.rs:1536-1541`, `:1704`)
  — derives from. So under the planted antecedent `drained` is empty **with or without** `W2`, and
  this limb's Δ stays 0 either way. **`W2`'s discriminating assertion is the EXIT ROW**, which is
  where it REDs.

### The join key

**`epoch`** — populated on every removal line, every exit row and every settlement row — with
**`op_seq` as secondary ordering**. **No wall-clock key anywhere. `ts` is a display field only and is
explicitly NON-joinable.**

---

## 7. `KL-1` — recorder-before-frontier, and the two consequences that rest on it

`MetricsPruneRecorder` resolves **every** metric handle **ONCE, at construction**
(`tombstone_frontier_impl.rs:2443-2447`; `touched_counter` `:2413-2421` even `increment(0)`s each
series so it renders from the first scrape), and `observe_pass` then increments those
already-resolved handles (`:2609` for `epochs_drained`). `TombstoneFrontier::new`'s
construction-order doc-contract (`:1308-1327`) binds every `/metrics` assertion in this increment.

- **`KL-1(a)`** — **if the ordering is inverted**, every handle binds to a **no-op** for its whole
  lifetime and the render assertion passes **vacuously**. This is why the recorder is bound **before**
  the frontier is constructed (`metrics::with_local_recorder`; in-tree shapes at
  `tombstone_frontier_impl.rs:3050-3055` and `sim/tombstone_gc_proof.rs:1524-1530`), and why §5.7's
  `Some("…")` rule exists as its detector.
- **`KL-1(b)`** — **because the handles resolve at construction, an increment issued from a SPAWNED
  task still lands on the recorder that was bound when the frontier was built.** The recorder-binding
  gap that makes `tracing`'s thread-local `set_default` unreadable across a spawn does **not** apply
  to metrics. This is exactly why SPEC-359's `R7` can read `rendered_counter` off a `multi_thread`,
  spawned-sweep race and get a real number rather than an empty capture. `X21-c`'s readability and
  `W6`'s fallback arm both rest on `KL-1(b)` and on nothing else.

---

## 8. Dispositions pre-registered here rather than decided at execution

### 8.1 The null-read disposition

This increment's payoff is conditional on `H-origin-2` being false, so the null branch is decided
**here**.

- **If the `Some(vec![])` state is NOT reproducible in-process, the instrument STILL SHIPS** (subject
  to `DT`), and the ORIGIN question routes to `TODO-654`'s pre-registered readings.
- **`NOT-OBSERVED-AT-HEAD` is informative, not a failure.** If the instrument observes no
  `refs_returned == 0 ∧ refs_at_entry > 0` line at HEAD, that bears on horn `H-origin-2`
  (pin-specificity). `W4`'s **negative control** is what licenses that reading — it proves the line is
  **absent** when the arm is not reached, so silence is meaningful rather than ambiguous — and `W8`'s
  **positive control** is what rules out the third reading, a window that never exercised a non-dark
  eligible drain.
- **This increment does NOT extend into diagnosis under ANY outcome:** no second drive, no soak cell,
  no new hypothesis round, no re-derivation of the 8 h entailment. **A null read ROUTES; it does not
  stall, and it does not reopen the diagnosis line the escalation clause hard-stopped.**

### 8.2 Routing, by id, without editing any tracker

- **The ORIGIN question, with the instrument armed → `TODO-654`** (the measurement increment,
  *"Plateau re-measurement from the DURABLE layer…"*, `depends_on: TODO-634`), carrying both horns,
  the `NOT-OBSERVED-AT-HEAD` pre-registration and §8.1's null-read disposition. `TODO-654` already
  carries this instrument's production read with three pre-registered readings (a) / (b) / (c).
  **Reading the instrument under production or soak load is `TODO-654`'s job, not this increment's.**
- **`W7`'s unscanned-third-file residual (§5.8) → `TODO-634`**, by id, because a source-scan
  completeness guard is a family concern.
- **Any `R4` failed leg → `TODO-634`**, by id.

**All routing is BY ID. `.specflow/todos/TODO-634.md` is byte-unedited by this increment, as are
`INVARIANTS.md` and `scripts/check-invariants.sh`.** This increment's execution creates **no new
tracker**: the hand-off is a citation of an existing id.

### 8.3 The simulation-test rule position, stated explicitly rather than left implicit

`CLAUDE.md`'s rule is that changes to domain services under `packages/server-rust/src/service/domain/`
be accompanied by a simulation test exercising the **changed behaviour** under at least one fault
scenario. **This increment's only `service/domain/` touch is a verdict-conditional DOC-CONTRACT on
`crdt.rs` (`FS` row `R1`) — zero behaviour change, provable by diff.** The rule therefore has **no
changed behaviour as its subject** here, and to the extent it names *"network partition or node
failure"* specifically, **that clause is claimed EXEMPT, explicitly and on the record.**

**A simulation witness is nevertheless supplied**, because the instrument is a concurrency
observation and deserves one: `W6`, sited in this family's existing sim home
(`sim/tombstone_gc_proof.rs`), under the **interleaving** fault dimension that file's scaffold can
actually drive — concurrent stamps and cursor ACKs released from a barrier across the drain. **There
is NO `SimNetwork` partition / delay / reorder leg**, because that file's own in-tree contract
records that its scaffold drives the interleaving dimension and not a `SimNetwork` partition, and
asserting a fault the file cannot inject grades vacuously. **A sim leg exists on BOTH of `W6`'s
pre-registered arms** — the PRIMARY grades the rendered line there, the FALLBACK grades the
`topgun_or_prune_*` render there — so the simulation obligation is discharged in
`sim/tombstone_gc_proof.rs` **either way**, and only the *granularity* of the sim assertion differs.
**There is no silent third option.**

### 8.4 The pre-shaped but UNRUN long cell

**No soak cell, no width-1000 matrix, no run of one hour or longer, no `soak_harness` runner
invocation, and no new measurement lineage anywhere on this branch.** If a long cell ever looks
unavoidable, it is **PRE-SHAPED and ROUTED BACK to `TODO-634`, never run here**.

**The pre-shaped cell, described and LEFT UNRUN:**

```
duration : 28,800 s   (8 h)
n        : 1          (one cell; no matrix, no sweep, no second arm)
purpose  : observe, over a window comparable to the one whose data
           entails Fact B, whether the removal-site instrument at HEAD
           emits any line matching the origin signature
           (refs_returned == 0 AND refs_at_entry > 0), and thereby bear
           on horn H-origin-2 (pin-specificity).
status   : NOT RUN by this increment. Routed to TODO-634 by id, and its
           production read belongs to TODO-654.
```

It is described here so that a later reader can see the shape that was declined, and so that the
decision not to run it is a recorded fact rather than an omission.

---

## 9. Rust type-mapping record

`PROJECT.md`'s Rust type-mapping rules, discharged item by item against `DrainAttributionClass`:

- **Enums over strings for known value sets** — satisfied; the classification is an `enum`, never a
  `String`.
- **No `f64` for integer-semantic fields** — no subject; the enum carries no numeric field. The
  instrument's own fields are `u64` counts, byte totals and epoch ids (plus `ts` as `i64` Unix ms in
  its explicit rendering) — **never `f64`**.
- **No `type` / `r#type` field on message structs** — satisfied; the enum has no fields.
- **`Default` on payload structs with 2+ optional fields** — no subject; it is not a payload struct
  and derives no `Default` (there is no defensible default class, and a defaulted classification
  would be exactly the silent-misread hazard this lineage keeps finding).
- **`serde` / `rename_all` / `skip_serializing_if` / `to_vec_named()`** — **no subject.**
  `DrainAttributionClass` **does not cross the MsgPack wire**; it is in-process only, so **no `serde`
  derive is added**. **If any later implementation adds a `Serialize`, the full `PROJECT.md`
  checklist applies in the same commit.**

The instrument emits `tracing` fields, not a serialized struct.

---

## 10. What this frozen layer does NOT contain

Deliberately, and each is the append-only post-section's:

- the **exact rendered SAMPLE** of the instrument's line;
- **which of `ts`'s two renderings shipped**;
- **which arm `W6` took** (PRIMARY or DECLARED FALLBACK) and the capture-helper choice that went with
  it;
- the **published verdict** and the one-sentence `FS` (i)/(ii) statement;
- the executed routings by id.

The sample is explicitly **NOT** under this layer's digest.

<!-- FROZEN-LAYER-END -->

## Frozen-layer digest

**Convention, stated so a later reader can reproduce it exactly.** The digest is the **sha256 of
every byte of this file strictly ABOVE the `<!-- FROZEN-LAYER-END -->` marker line** — that is, from
the first byte of the file up to and including the newline that terminates the line immediately
preceding the marker. The marker line itself, this digest block, and everything below are
**excluded**, which is what makes the digest computable before the digest block was written and
stable after it was.

**Reproduce it with exactly this command, run from the repository root:**

```sh
sed -n '1,/^<!-- FROZEN-LAYER-END -->$/p' \
  packages/server-rust/benches/soak_harness/evidence/spec360-origin-instrument-ruling.md \
  | sed '$d' | shasum -a 256
```

```
sha256 = 3efa6d963ee67564b629279f7ea96c727a91850343978fc4986696920ed0e55c
```

<!-- APPEND-ONLY POST-SECTION -->

## Append-only post-section

**EMPTY AT FREEZE TIME. Intentionally so.**

**Appends beneath this marker are the ONLY writes this artifact accepts.** Nothing above
`<!-- FROZEN-LAYER-END -->` may be edited after the commit that froze it; a defect found in the
frozen layer afterwards is published **here**, as a separate fact, and the frozen table is walked as
written.

This post-section carries its **OWN separate digest**, computed over its own bytes and recorded here
by the group that writes it. That digest does not exist yet, because this section does not yet have
content.


---

<!-- POST-BLOCK-BEGIN: G5-RENDERED-SAMPLE -->

## Post-block 1 — the rendered sample, the shipped choices, and the selected fault arm

**Written by `G5`, after the instrument rendered. Nothing above `<!-- FROZEN-LAYER-END -->` was
touched; the frozen-layer digest reproduces unchanged.**

### 1.1 Per-block digest convention — stated here because it must COMPOSE

This post-section will carry **more than one block**. `G6` will append a further block (the verdict,
the `FS` ruling and the routings) beneath this one, carrying its **own** digest. So the convention
cannot be *"the digest of everything below the post-section marker"* — that would be invalidated by
the next append and would make every earlier digest unverifiable.

**The convention is therefore PER BLOCK, and each block digests only its own bytes:**

> A block begins at its `<!-- POST-BLOCK-BEGIN: <id> -->` line and ends at its
> `<!-- POST-BLOCK-END: <id> -->` line. The digest is the **sha256 of every byte strictly BETWEEN
> those two marker lines** — from the first byte of the line after `BEGIN` up to and including the
> newline terminating the line before `END`. Both marker lines are excluded, and so is the digest
> block that follows `END`, which is what makes the digest computable before it is written and
> stable after it is.

This composes: appending block *n+1* cannot change block *n*'s bytes, so every earlier digest stays
reproducible forever. Each block records its own reproducing command with its own id substituted.

### 1.2 The EXACT rendered sample of the new removal line

Captured from the in-process drive through the production prune service, by the capture layer living
in `tombstone_frontier_impl.rs`'s own test module — so this is the **NO-leading-space** target-prefix
form the frozen contract §5.3 pins for that reader.

**Every row below ends with a single TRAILING space.** The visitor writes `name=value ` per pair and
never trims. A reader that trims the row and then matches `name=value ` including its trailing space
will fail on the last field.

**(i) THE ORIGIN SIGNATURE — the planted present-but-empty entry:**

```
target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787505799565 op_seq=2 epoch=1 refs_returned=0 refs_at_entry=1 bytes_returned=0 watermark=1 ceiling=2 
```

**(ii) THE POSITIVE CONTROL — the same fixture, unplanted, a genuinely non-empty removal:**

```
target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787505799565 op_seq=2 epoch=1 refs_returned=1 refs_at_entry=1 bytes_returned=4 watermark=1 ceiling=2 
```

**PER-RUN-VARYING TERMS, marked as such so a later reader does not pin a fixture artefact:**

| Term | Varies? | Note |
|---|---|---|
| `ts` | **VARIES EVERY RUN** | wall-clock Unix ms at emission. Display only; **never** a join key. |
| `op_seq` | fixture-fixed here (`2`) | a real workload's value is arbitrary. |
| `epoch`, `refs_returned`, `refs_at_entry`, `bytes_returned`, `watermark`, `ceiling` | fixture-fixed | fixed by *this* fixture, not by the instrument. |
| `message=prune removal observed` | **INVARIANT** | the message carries **no `=` character**, deliberately: the visitor renders the message as a bare `message=<words>` pair, and an `=` inside those words would break a whitespace-token field reader. |
| `target=…` | **INVARIANT** | the frozen target, exactly. |

**Field order, as rendered, matches frozen §5.2 exactly:** `ts`, `op_seq`, `epoch`, `refs_returned`,
`refs_at_entry`, `bytes_returned`, `watermark`, `ceiling`. **No JSON blob**, as contracted — one
`tracing` field per value.

### 1.3 WHICH OF `ts`'s TWO PERMITTED RENDERINGS SHIPPED

**Rendering (b) — the EXPLICIT field — shipped.**

`ts` is recorded as an explicit signed Unix-ms `tracing` field, rendering `ts=<digits> ` like any
other pair, so the line carries **eight explicit fields**. Frozen §5.4 pre-registered both renderings
as compliant; this records which one the tree got.

**Why (b) rather than (a).** Under rendering (a) the subscriber supplies the stamp and `ts` is not a
recorded field at all — which means it is **invisible to any capture that records fields rather than
subscriber-formatted metadata**, including the two in-tree captures this increment reads through. An
explicit field is observable over every transport the term will actually cross, and it follows the
tree's existing signed-Unix-ms convention for the residency rows' own timestamp fields. **`ts` is
still NEVER a join key** — the rendering choice does not touch that.

### 1.4 WHICH CAPTURE / EXTRACTION HELPER CHOICE THE FRONTIER-FILE DRIVE TOOK

**Option 2 — new `row_field`-shaped helpers inside `tombstone_frontier_impl.rs`'s OWN test module**,
not the bare `contains("name=value")` idiom the file previously used, and **not** an import from
another module's helpers (both existing sets are `#[cfg(test)]`-module-private and neither is
importable across a module boundary).

The helpers now living in that file's test module: `row_field` (panics naming an absent field —
never reads it as a default), `row_u64`, `row_bool`, `rows_of_kind`, and `rendered_counter_u64`
(panics naming an absent series — never reads it as a zero).

`G5` added one more beside them, because the removal line needed a selector the existing set could
not express: **`rows_of_target`**, which selects rows **by TARGET ALONE**. `rows_of_kind` requires a
`kind=` discriminant, and the removal line deliberately carries none — its own target does that work,
which is exactly what keeps the field set at the frozen eight instead of widening it for a reader's
convenience.

**All of this is inside a file the ledger already counts.** No new counted `.rs`. The reserve slot
(5/5) was **not** used, and no exemption is claimed.

### 1.5 WHICH OF `W6`'s TWO ARMS `G5` SELECTED

> ### **SELECTED: PRIMARY.**
> The spawned drain future carries the capture subscriber with it, and the sim leg asserts the
> branch's **RENDERED LINE**.

**THE EVIDENCE — empirical, in BOTH directions, not a reading.** A throwaway probe was inserted into
the sim file, compiled, run both ways and then removed. The sim file is **byte-unedited at `G5`'s
handoff** (blob hash unchanged from the base commit, verified with `git rev-parse HEAD:<path>`).

*Preconditions, verified first:* `tracing` resolves to **0.1.44** in `Cargo.lock`, so
`tracing::instrument::WithSubscriber` is available — **CONFIRMED**; uses of
`with_subscriber` / `WithSubscriber` anywhere under `packages/server-rust/src`: **ZERO** — a
new-to-the-tree construct, **CONFIRMED** and called out as one.

*The probe* used the existing race scaffold unchanged in shape — a shared 2-party barrier, a spawned
cursor ACK and a spawned prune pass released together, under the existing
`#[tokio::test(flavor = "multi_thread")]` flavour. The only change was the one the PRIMARY arm calls
for: build a `tracing::Dispatch` **once**, hand `tracing::dispatcher::set_default(&dispatch)` to the
test thread and `dispatch.clone()` to the spawned future via `fut.with_subscriber(...)`.

| Direction | Change | Rows captured | Removal rows |
|---|---|---|---|
| 1 | **with** `.with_subscriber(dispatch)` | **16** | **5** |
| 2 | **without** it, everything else identical | **0** | **0** |

Direction 1's captured rows, verbatim (note the **LEADING space** before `target=` — the sim file's
visitor uses the other prefix form, per frozen §5.3):

```
 target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787506278401 op_seq=10 epoch=1 refs_returned=1 refs_at_entry=1 bytes_returned=2 watermark=1000 ceiling=6 
 target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787506278401 op_seq=10 epoch=3 refs_returned=1 refs_at_entry=1 bytes_returned=2 watermark=1000 ceiling=6 
 target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787506278401 op_seq=10 epoch=2 refs_returned=1 refs_at_entry=1 bytes_returned=2 watermark=1000 ceiling=6 
 target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787506278401 op_seq=10 epoch=4 refs_returned=1 refs_at_entry=1 bytes_returned=2 watermark=1000 ceiling=6 
 target=topgun_server::tombstone_frontier::removal message=prune removal observed ts=1787506278401 op_seq=10 epoch=5 refs_returned=1 refs_at_entry=1 bytes_returned=2 watermark=1000 ceiling=6 
```

**What the two directions establish TOGETHER, which neither establishes alone.** The rendered line is
observable across the spawn **if and only if** the subscriber travels with the future. Direction 2
reproduces exactly the empty capture the sim file's own doc-contract predicts for a thread-local
`set_default` under a multi-threaded runtime — so a sim leg asserting a rendered line over a spawned
drain **without** carrying the subscriber would be reading an empty capture and passing **vacuously**.
Direction 1 shows the mechanism closes that gap.

**THE REASON.** PRIMARY is selectable, so PRIMARY is selected — there is no silent third option. The
adaptation is confined to the already-counted sim file; it needs no production edit, no new counted
`.rs`, no change of runtime flavour and no change to the scaffold's thread model. And it grades the
sim leg at **LINE** granularity, which is strictly stronger than the fallback's **metrics**
granularity for an instrument whose entire deliverable *is* a rendered line.

**Consequences of the selection, for `G6`:**

- The **paired single-thread in-process rendered-line witness** in `tombstone_frontier_impl.rs` — the
  fallback's second, mandatory half — is **NOT owed**, because the fallback was not taken.
- The sim leg must still: read which of the two barrier orderings the round took and assert **that**
  branch's consequences (never a disjunction both branches satisfy); gate on a **non-empty** capture
  before reading any term; and carry its own mutation arm, REDing when the observation mutation is
  applied.
- The sim-side reader must consume the **LEADING-space** target-prefix form.

### 1.6 THE `C13` MODULE-DOC RE-WORDING THE SELECTED ARM ENTAILS — the wording `G6` must apply

The sim file's module doc currently asserts **flatly** that on that scaffold **both** in-process
transports — the `tracing` capture and the metrics recorder — are thread-local, and that the aggregate
leg is therefore recorded at aggregate granularity **only**. Two corrections are owed, and they are
owed on **different terms**:

**(A) THE METRICS CLAUSE — owed on BOTH arms, NOT arm-conditional.** The flat "both transports are
thread-local" assertion is refuted by the tree independently of which arm ships, and a doc-contract
the code contradicts is a false-invariant hazard. Correct it to say:

> The **recorder BINDING** is thread-local, but `MetricsPruneRecorder` resolves every metric handle
> **once, at construction**, so an increment issued from a **spawned** task still lands on the
> recorder that was bound when the frontier was built. Metrics are therefore **readable across a
> spawn**, which is why the raced-sweep leg can read a rendered counter under a `multi_thread`
> runtime and get a real number rather than an empty render. The aggregate leg stays at **AGGREGATE**
> granularity **on its own terms** — its predicate *is* the aggregate conservation snapshot — and
> **not** because metrics are unreadable.

**(B) THE *TRACING* CLAUSE — NARROWED, NOT DELETED. Owed on the PRIMARY arm, which is the arm
selected.** Correct it to say:

> A capture bound by `tracing::subscriber::set_default` is **thread-local**, and is therefore
> **invisible to a spawned task UNLESS that task's future carries the subscriber with it**
> (`with_subscriber`). The aggregate leg does **not** carry one, and stays at AGGREGATE granularity
> for exactly that reason. A leg that *does* carry one observes the spawned drain's rows at **line**
> granularity.

**The aggregate leg's own recorded granularity does NOT change** under either correction. Both edits
are confined to that file's doc comment, change **no behaviour**, add **no** provenance marker, and
consume **no** additional counted `.rs` and **no** exemption shape — the file is already counted and
the cap is not exceeded.

### 1.7 What this block does NOT contain, and who owes it

**`G6` will append a further block** beneath this one — the **verdict**, the **`FS` ruling** with its
one-sentence statement of which of defect (i) and defect (ii) was delivered, and the **routings by
id** — and that block will carry **its own digest** under the per-block convention of §1.1. This
block makes **no** verdict claim, walks **no** row of the Decision Table, and routes nothing.

Nothing in this block asserts, implies or builds on a plateau claim in either direction.

<!-- POST-BLOCK-END: G5-RENDERED-SAMPLE -->

### Post-block 1 digest

**Reproduce it with exactly this command, run from the repository root:**

```sh
sed -n '/^<!-- POST-BLOCK-BEGIN: G5-RENDERED-SAMPLE -->$/,/^<!-- POST-BLOCK-END: G5-RENDERED-SAMPLE -->$/p' \
  packages/server-rust/benches/soak_harness/evidence/spec360-origin-instrument-ruling.md \
  | sed '1d;$d' | shasum -a 256
```

```
sha256 = 4bfd62e3740f00a0c30de65938f407720a1d51575113cc86c2d164257de017d0
```


---

<!-- POST-BLOCK-BEGIN: G6-VERDICT -->

## Post-block 2 — the Decision-Table walk, the published verdict, the `FS` ruling and the routings

**Written by `G6`, after the interleaving fault leg executed. Nothing above
`<!-- FROZEN-LAYER-END -->` was touched; the frozen-layer digest reproduces unchanged
(`3efa6d963ee67564b629279f7ea96c727a91850343978fc4986696920ed0e55c`), and post-block 1's bytes are
untouched, so its own digest reproduces unchanged too. This block carries its own digest under the
per-block convention post-block 1 §1.1 states.**

### 2.1 THE DECISION TABLE, WALKED IN ITS FROZEN ORDER

The table is walked **as written**. It was not edited, and no row's antecedent was re-worded to fit
what execution found.

---

#### ROW `R4` — evaluated FIRST, fail-closed. **ANTECEDENT DOES NOT HOLD. R4 DOES NOT FIRE.**

`R4`'s antecedent is a **disjunction of four failure legs**. Each is stated and answered:

**(a) "an unmutated suite is not green."** FALSE — every suite is green on the branch tip:

| Suite | Result |
|---|---|
| `cargo test --release -p topgun-server` (whole crate, 18 targets) | **0 failed**; lib alone 1862 passed / 0 failed / 2 ignored |
| `pnpm test:sim` | **28 passed**, 0 failed (27 at HEAD + the one test this group added) |
| doc tests | 0 passed / 0 failed / 5 ignored |

**(b) "a mutation arm does not RED against the assertion `N10` sites it on."** FALSE — all three
arms RED, each against its own sited assertion, and each names it:

| Arm | `N10` sites it on | Executed outcome |
|---|---|---|
| `W2` (gate `:986` on `!refs.is_empty()`) | the **EXIT ROW** | RED, naming the three-term exit-row assertion. The same run shows `topgun_or_prune_epochs_drained_total` **passing at Δ = 0** under the mutation — the limb `N6` `X21-c` says this arm does not move, and it did not. |
| `W5` (emit `slot.refs_at_entry` for `refs_returned`) | the **rendered `refs_returned`** | RED, naming `refs_returned != refs.len()`. |
| `W6` (same mutation, interleaving leg) | **its arm's rendered LINE** | RED, naming `refs_returned != refs.len()`, on the sim file's own rendered row. Shown twice: on the leg's quiescent reference (which runs first) and — with the quiescent gate suppressed by a throwaway probe — on **round 0 of the raced leg itself**, at `ceiling=3`, i.e. while that round was asserting the **admitted** branch's own consequences. |

**No mutation arm is graded against `X21-c`'s Δ.** This was checked, not assumed: `W2`'s transcript
records the Δ = 0 limb *passing* under the mutation, and the arm's RED is the exit row.

**(c) "the planted positive control did not fire."** FALSE, on both readings of "positive control":
`W3`'s planted classifier control returns `AttributedWithoutObservation` on the planted record and
does **not** return it on the unplanted twin over the same fixture; `W8`'s instrument positive
control emits a removal line with `refs_returned = 1`, `refs_at_entry = 1`, `bytes_returned = 4`,
asserted on rendered text by field name.

**(d) "the negative control did not show the line absent."** FALSE — `W4` shows **zero** removal
rows on **both** paths that skip the arm (no-eligible-epoch with the watermark non-zero, and the
dark fast path at `watermark == 0`), each arm gating on a **non-empty** capture first, so the
absence is read off a transport that demonstrably worked.

**Every leg of `R4`'s disjunction is false ⇒ `R4` does not fire. The walk proceeds.**

---

#### ROW `R1` — **ANTECEDENT HOLDS. `R1` FIRES.**

`R1` is a conjunction of obligation **A** and obligation **B**.

**Obligation A — HOLDS.** The planted present-but-empty `epoch_tags` entry, driven **through the
SERVICE** (`prune_epoch_tombstones`), reproduces `AttributedWithoutObservation` with the class
**computed live** by `classify_drain_attribution`, every term read over the transport that carries
it:

- **rendered exit row** — `exit_kind = DrainedByPrune`, `bytes_freed_attributed = 4` (> 0),
  `removed_refs_observed = 0`;
- **rendered pass row** (`kind = "prune_pass"`) — `considered = 0`, `empty_drain = true`;
- **Prometheus render** — `topgun_or_prune_epochs_drained_total` **Δ = 0** across the drive, read as
  `Some("…")` and never `None`;
- **non-vacuity gate first** — the capture is asserted non-empty and of the expected per-kind row
  count before any term is read.

**Obligation B — HOLDS.** The instrument is delivered at the adopted siting (exactly one
`tracing::info!` inside the `Some(refs)` arm, adjacent to the existing `removed_refs` /
`removed_bytes` locals and strictly before `drained.extend(...)`), with §C's field set by name, on
its own target, unconditional, not registered with the metrics registry — and its emission is
**proven correct over the rendered transport**:

| Reading | Witness | Outcome |
|---|---|---|
| zero-returned (the origin signature) | the planted drive | rendered line captured verbatim (post-block 1 §1.2(i)) |
| positive | `W8` | rendered line captured verbatim (post-block 1 §1.2(ii)) |
| silent | `W4`, both paths | zero removal rows off a non-empty capture |
| observation, not a copy | `W5` | REDs by name under the mutation |
| under the **interleaving** fault | `W6`, **PRIMARY arm** | rendered LINE asserted across a spawn; branch read, not disjoined; REDs by name under the mutation |

**⇒ `DT` row `R1` FIRES.**

---

#### ROWS `R2` and `R3` — **NOT REACHED.**

The table is evaluated in its frozen order and `R1` fired, so `R2` (A holds ∧ B does not) and `R3`
(A does not hold) are not reached. Recorded for completeness: `R2`'s antecedent is **false** because
B holds, and `R3`'s is **false** because A holds. §C's inference chain therefore is **not**
contradicted by this spec's control — the miniature reproduces exactly the pair the chain predicts.

### 2.2 THE PUBLISHED VERDICT

> # **CONFIRMED-AND-ARMED**
>
> `DT` row `R1`. The `PD-F18` consequence holds when driven through the service, and §C's adopted
> removal-site origin instrument is landed and proven to emit correctly over the transport its
> consumer will actually read — including under the interleaving fault, at rendered-line
> granularity, across a spawn.

### 2.3 THE `FS` RULING — the one-sentence (i)/(ii) statement, claiming no more

> **This spec PARTIALLY DELIVERED defect (i), attribution — the removal-site observation line plus
> two doc-contracts naming `bytes_freed_attributed` / `drained_refs_total` as entry-side and
> `considered` / `empty_drain` / `epochs_drained` as unable to discriminate — and did NOT deliver
> defect (ii), reclamation, which is ROUTED because a reclamation fix without a named origin is
> exactly the wrong-shaped fix this umbrella exists to avoid.**

**What "partially delivered" means here, in `FS` row `R1`'s own terms and no further.** The
observation terms already existed; the residual (i) gap was **service-side blindness** and
**naming**, and that is what was closed — the blindness at the frontier by the line, the naming by
the contracts. **It frees zero bytes.** **No existing metric series' value or meaning changed**, and
that is deliberate: re-pointing one mid-lineage would break comparability with every round already
measured against it.

**`FS` row `R1`'s fix, as landed — two doc-contracts and nothing else:**

1. `packages/server-rust/src/tombstone_frontier.rs` — on `PruneEpochResidencyRecord::bytes_freed_attributed`
   and on `METRIC_PRUNE_DRAINED_REFS_TOTAL`, naming both **entry-side**, with the `TODO-634` tracker
   pointer `CLAUDE.md` sanctions for a deferred property.
2. `packages/server-rust/src/service/domain/crdt.rs` — on `prune_epoch_tombstones`, recording that
   `considered` / `empty_drain` / `epochs_drained` **cannot distinguish** *"no epoch was eligible"*
   from *"an eligible epoch was removed and returned zero refs"*, with the same pointer.

**What was NOT done, explicitly.** The anti-suppression clause was honoured: `drained_epochs.insert(e)`
was **not** gated on `!refs.is_empty()`. That change would relabel the exit and suppress the very
rows the new instrument exists to explain, while the refs would still be gone from the index and the
durable content still un-dropped. **The rendered pass row was not widened** — it carries exactly
`kind`, `considered`, `empty_drain` on the branch, as at `main`.

### 2.4 ROUTING — `C9` / `C9a` handed to `TODO-654`, BY ID

**Routed by id in this artifact only. No tracker file was edited by this spec.**

**To `TODO-654`** (*"Plateau re-measurement from the DURABLE layer…"*, `depends_on: TODO-634,
SPEC-360`) — the **ORIGIN question, with the instrument armed**, carrying:

- **BOTH horns of the origin paradox, neither chosen:**
  - **`H-origin-1`** — the writer enumeration is incomplete in a way a static read cannot see: a
    route that empties a vector without being one of the five enumerated mutation expressions, or an
    interleaving that produces the state without any single writer producing it.
  - **`H-origin-2`** — the inference chain is **pin-specific**. The 8 h cells ran on `SPEC-357`-era
    binaries; the drain's bracket has since been rewritten. The instrument lands at HEAD and will be
    read at HEAD.
- **The `NOT-OBSERVED-AT-HEAD` pre-registration.** If the instrument observes no
  `refs_returned == 0 ∧ refs_at_entry > 0` line at HEAD, that is an **informative result** bearing on
  `H-origin-2`, **NOT** an instrument failure. `W4`'s negative control is what licenses that reading
  (silence means the arm was not reached, because the line is unconditional and provably absent only
  when the arm is skipped), and `W8`'s positive control is what rules out the third reading (a window
  that never exercised a non-dark eligible drain). All three readings the instrument can produce —
  **silent / zero-returned / positive** — were exercised before it shipped.
- **The null-read disposition (`C9a`), normative.** The instrument **SHIPS** regardless; a null read
  **routes**, it does not stall and it does not reopen the diagnosis line `SPEC-358`'s `E-C`
  hard-stopped. `TODO-654`'s pre-registered readings (b) *"every line
  `refs_returned == refs_at_entry`"* and (c) *"no lines at all while epochs exit"* are exactly the two
  null shapes and each already carries its routing there.
- **Reading the instrument under load is `TODO-654`'s job, not this spec's.** `SPEC-360` landed the
  instrument and proved it emits correctly; it did **not** read it under production or soak load.

**The extractor contract this hand-off depends on is the FROZEN layer above**, and the exact rendered
samples are post-block 1 §1.2. A consumer must note two rendered-form facts: rows end with a **single
trailing space**, and the **target prefix differs by a leading space** between the two in-tree
capture visitors (§5.3).

### 2.5 ROUTING — `W7`'s unscanned-third-file residual handed to `TODO-634`, BY ID

**To `TODO-634`** — explicitly **NOT** to `TODO-654` — the **source-scan completeness gap** in `W7`:

`W7` reads its scan surface through `include_str!`, which resolves literal paths relative to the
including file, so its claim is scoped to exactly the two files it names
(`tombstone_frontier_impl.rs`, `service/domain/crdt.rs`). It REDs when a **sixth** `self.epoch_tags`
writer appears in `tombstone_frontier_impl.rs`, or when a **second** non-`#[cfg(test)]` caller of
`drain_prunable_tombstones` appears in **either** scanned file. **A production caller added in a
third, unscanned file would NOT RED.** Closing that (a `std::fs` walk of `src/` from
`CARGO_MANIFEST_DIR`, or an include list asserted against a directory listing) is out of scope here.

**The reason it goes to the umbrella and not to the measurement increment:** a source-scan
completeness guard is a **FAMILY** concern — it protects Fact A of the origin paradox for every
carve that comes after this one — and is **no part of `TODO-654`'s durable-layer plateau read**,
whose object is RSS, redb file size, WAL segment retention and a store-level live-vs-dead tombstone
census. Filing it against `TODO-654` would attach a static-analysis guard to a measurement cell that
has no use for it.

**Both routings are by ID, in this post-section, and no tracker file was edited.** `TODO-634.md`,
`INVARIANTS.md` and `scripts/check-invariants.sh` are byte-unedited on this branch, provable by
`git diff --stat main...HEAD` over those three paths returning empty output.

### 2.6 `AC20` — THE LOAD HARNESS, RUN ONCE, GRADED TWICE

**The cell:** `cargo bench --bench load_harness -- --connections 200 --duration 30` — fire-and-wait
at the harness defaults, **the same cell `SPEC-359` ran**. A 15 s cell would not be comparable and
was not run.

**Raw output, verbatim:**

```
Running scenario: throughput
Connections: 200, Duration: 30s

operation                           count     p50 µs     p95 µs     p99 µs   p99.9 µs     max µs
--------------------------------------------------------------------------------------------
write_latency                      112487       1657       6343      12423      26447      37375

ops/sec: 37495
PASS [throughput_assertion]
```

**GRADING 1 — against `SPEC-359`'s number on the same host**
(`.specflow/archive/SPEC-359.md:3676-3683` — *"fire-and-wait, defaults (200 connections, 30 s)"*:
113 263 ops, 37 754 ops/s, p50 1 479 µs):

| Term | `SPEC-359` | `SPEC-360` | Δ | Verdict vs the 20 % tolerance |
|---|---|---|---|---|
| ops | 113 263 | 112 487 | **−0.69 %** | PASS |
| ops/s | 37 754 | 37 495 | **−0.69 %** | PASS |
| p50 | 1 479 µs | 1 657 µs | **+12.0 %** | PASS (inside 20 %, investigated below) |

**The p50 movement, stated rather than absorbed.** +12 % on p50 with throughput flat to within 0.7 %
is not the shape a per-operation cost regression takes — a real added per-write cost would depress
ops/s in step. The instrument this spec added fires **once per eligible epoch removed**, never on the
write path and never on the dark path, and the harness scenario stamps no tombstones and drives no
prune pass, so **zero** removal lines are emitted during the measured window. Both runs are
single-shot cells on a shared developer host with no repetition and no confidence interval; a 178 µs
move on a p50 of that magnitude is within the run-to-run spread such a cell carries. It is recorded
here so it is visible rather than filed as noise, and both absolute floors pass with large margin.

**GRADING 2 — against `benches/load_harness/baseline.json`'s absolute `fire_and_wait` floors:**

| Floor | Threshold | Measured | Verdict |
|---|---|---|---|
| `min_ops_per_sec` | 30 000 | **37 495** | **PASS** (+25.0 % over the floor) |
| `max_p50_us` | 5 000 | **1 657** | **PASS** (66.9 % under the ceiling) |

The harness's own gate printed `PASS [throughput_assertion]`.

**This is the standing hot-path gate `CLAUDE.md` already requires, not a measurement cell.** It ran
once, in well under an hour, and it carries Observable Truth 9: the instrument's stated cost is now a
**measured fact** rather than the assumption it was.

### 2.7 `N11` — THE `CLAUDE.md` SIMULATION-RULE POSITION, RESTATED WITH `W6`'s ARM NOW KNOWN

Recorded explicitly rather than left to pass silently.

**The exemption claim, unchanged.** `CLAUDE.md`'s rule is that changes to domain services under
`packages/server-rust/src/service/domain/` be accompanied by a simulation test exercising the
**changed behaviour** under at least one fault scenario. This spec's **only** `service/domain/` touch
is the verdict-conditional **doc-contract** on `crdt.rs` — **zero behaviour change, provable by
diff**. The rule therefore has **no changed behaviour as its subject** here, and to the extent it
names *"network partition or node failure"* specifically, that clause is claimed **EXEMPT, explicitly
and on the record**.

**What was supplied anyway, and it is stronger than the exemption.** `W6` is a real simulation
witness in this family's existing sim home, under the **interleaving** fault dimension that file's
scaffold can actually drive — and **on the PRIMARY arm** it grades at **rendered-LINE** granularity,
the strongest transport available for an instrument whose entire deliverable *is* a rendered line:

- a prune sweep and a second device's cursor ACK released together from a shared barrier, both
  spawned, both contending for the real frontier mutex, 32 rounds with the spawn order alternating;
- the round **reads** which of the two barrier orderings it took (off the refusal counter) and
  asserts **that** branch's removal-line set — refused ⇒ epochs {1,2,3,4,5} at ceiling 6, admitted ⇒
  epochs {1,2} at ceiling 3 — never a disjunction both branches satisfy;
- observed split on the recorded run: **16 refused / 16 admitted**, printed in the transcript so a
  collapsed distribution would be visible;
- the capture is gated **non-empty** before any field is read, and the rows are read **across a
  spawn** only because the sweep's future carries the subscriber with it;
- every raced line is compared as text against the **quiescent** drain's line for the same epoch,
  `ts` and the round's own `ceiling` apart.

**There is NO `SimNetwork` partition / delay / reorder leg, and that is deliberate.** `SimNetwork`'s
fault injection affects routing between nodes registered with the harness; a directly-driven drain
crosses no such edge, so such a criterion would be unsatisfiable in the sited file or satisfied
**vacuously**. The file's own in-tree contract says so, and asserting a fault a scaffold cannot inject
is precisely the class this spec guards against everywhere else.

**The `C13` doc-contract correction post-block 1 §1.6 specified was applied**, in the sim file's doc
comments only, changing no behaviour and adding no provenance marker:

- **(A) the metrics clause — corrected, as owed on BOTH arms.** The flat "both in-process transports
  are thread-local" assertion is refuted by the tree independently of the arm: the recorder BINDING is
  thread-local, but `MetricsPruneRecorder` resolves every handle **once, at construction**, so an
  increment from a spawned task still lands on the recorder bound when the frontier was built.
  Metrics are therefore **readable across a spawn** — which is why the raced-sweep arms in that file
  read rendered counters under a `multi_thread` runtime and get real numbers. **The aggregate leg's
  own recorded granularity does NOT change**: it stays AGGREGATE on its own terms, because its
  predicate *is* the aggregate conservation snapshot, and not because metrics are unreadable.
- **(B) the tracing clause — NARROWED, not deleted, as owed on the PRIMARY arm.** A capture bound by
  `set_default` is thread-local and therefore invisible to a spawned task **unless that task's future
  carries the subscriber with it**; the aggregate leg does not carry one and stays at AGGREGATE
  granularity for exactly that reason, while a leg that does carry one reads at line granularity.

### 2.8 THE CAPTURE-RACE RESIDUAL, DISPOSED OF RATHER THAN LEFT SILENT

The predecessor group recorded that the sim file's capture helper carries the same
callsite-interest-rebuild race it fixed in the frontier file, and warned it would surface here as
*the instrument appearing not to emit*. **It did not bite**: 8 consecutive filtered sim runs and
repeated full-lib release runs are green with no missing rows.

The leg is nevertheless built to keep the window narrow: **one** capture is installed for the whole
test and the sink is drained between rounds, rather than installing and dropping a subscriber 33
times — reducing this leg's contribution from 33 rebuild windows to one. The predecessor's
serialisation remedy was **not** applied to this file: it was not needed, and it would have required
holding a `std::sync::Mutex` guard across `.await` points in a `multi_thread` test, which the lint
gate rejects. The residual therefore remains **open for that file's other captures** and is recorded
here rather than quietly closed.

### 2.9 WHAT THIS BLOCK DOES NOT CLAIM

- **No plateau claim, in either direction, anywhere.** Nothing in this block asserts, implies or
  builds on one, and nothing treats a falling reclaim fraction as evidence that total reclamation
  degrades with width. The plateau is **not established either way**, and re-measuring it belongs to
  the measurement increment.
- **No origin naming.** Neither horn is chosen. This spec makes no prediction about which one the
  origin falls on, and nothing here should be read as evidence for either.
- **No measurement leg.** No soak cell, no width-1000 matrix, no run of one hour or longer, no
  `soak_harness` invocation, no new measurement lineage. The pre-shaped long cell described in the
  frozen layer remains **UNRUN**.
- **No reclamation fix.** Defect (ii) is routed, not delivered, and the conservative sweep remains the
  backstop shape and remains out of scope.

### 2.10 THE COUNTED `.rs` LEDGER, FINAL

| Slot | File | Status |
|---|---|---|
| 1/5 | `packages/server-rust/src/tombstone_frontier.rs` | taken |
| 2/5 | `packages/server-rust/src/tombstone_frontier_impl.rs` | taken |
| 3/5 | `packages/server-rust/src/service/domain/crdt.rs` | taken — **only** by `FS` row `R1`'s doc-contract, as the Delta held it |
| 4/5 | `packages/server-rust/src/sim/tombstone_gc_proof.rs` | taken |
| 5/5 | — | **HELD IN RESERVE, UNUSED** |

**4 counted `.rs`, 1 in reserve, 0 exemptions.** The exemption ledger is **EMPTY**; no `PROJECT.md`
sanctioned shape was invoked. `git diff --name-only main...HEAD -- '*.rs'` lists exactly those four
paths, each named in this spec's Delta.

<!-- POST-BLOCK-END: G6-VERDICT -->

### Post-block 2 digest

**Reproduce it with exactly this command, run from the repository root:**

```sh
sed -n '/^<!-- POST-BLOCK-BEGIN: G6-VERDICT -->$/,/^<!-- POST-BLOCK-END: G6-VERDICT -->$/p' \
  packages/server-rust/benches/soak_harness/evidence/spec360-origin-instrument-ruling.md \
  | sed '1d;$d' | shasum -a 256
```

```
sha256 = d3d1442282d6a48559534017e7a858b5a6a1c22df3e9350cfa5289f83737d716
```
