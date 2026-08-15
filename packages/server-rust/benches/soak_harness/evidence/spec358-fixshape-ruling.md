# SPEC-358 — the Fix-Shape ruling record

## Verdict

`V0 — NOT REPRODUCED`. All three D-T rows (V1 REF-LOSS, V2 BOOKKEEPING ATTRIBUTION, V3
COUNTER-FAMILY MISNAMING) evaluated **false** over their own frozen universes; row 4
(V0) holds. See `spec358-microdiag-transcript.txt` for the full executed record and
`spec356-manifest.md` §12.1 for the appended manifest entry.

## The F-M row that fired

Per §12.0's frozen Fix-Shape Mapping (Step 3):

| Verdict | Fix shape | Why |
|---|---|---|
| **V0 — NOT REPRODUCED** | **THE RULING IS NOT WRITTEN** | No reading is named. E-C fires. |

Since the verdict is V0, the ruling row that fires is the V0 row, and its fix shape is
**THE RULING IS NOT WRITTEN**. Per R7.1, the not-written status IS the satisfying
content of this artifact — it is not an omission, and this document does not attempt to
supply a ruling in its place. There is no STANDALONE CORRECTNESS FIX to name, no
counter family to repair, no throughput-mechanism reading to state alongside one: none
of that is what V0 produced, and writing one anyway would be exactly the "reading not
on the table" §12.0 pre-forbids.

## §8.3, quoted VERBATIM (from `spec356-manifest.md:465-478`)

> ### §8.3 — And the family is NOT blocked by it, which is why this is a legitimate terminal branch
>
> **The recommended reclamation model closes safety REGARDLESS of which cause it turns out to
> be.** `ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the
> cursor-age fence with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound
> the reclaimable set by **live claims**, not by any hypothesis about *why* the current prune
> falls behind. **A selection defect, a scheduling defect and a throughput defect are all
> *contained* by a registry that never reclaims below a live claim.**
>
> What an unclassified cause costs is **fix-shape efficiency** — the family would design without
> knowing which limb to optimize first — **not safety, and not the family's ability to proceed.**
> A Step-5 outcome is therefore to be read as **an expensive answer, not a blocked one**, and any
> Step-5 outcome must be reported quoting this paragraph beside it.

(§12.0's E-C clause cites this same paragraph as the justification for starting the
`ReclamationRegistry` family with the cause unclassified; it is reproduced here again,
verbatim, beside the determination it justifies, per §8.3's own closing sentence and
per E-C's requirement.)

### Note on the quote's fidelity (added by Review v1 — the block above is NOT edited)

The block-quote above is **word-identical** to `spec356-manifest.md:465-478` but is
**re-wrapped** to this document's narrower column: every word, every emphasis marker and
every character matches, and only the line breaks differ. Review v1 read AC12's "quoted
verbatim" strictly and flagged the difference. The correction is made by ADDITION, never
by overwriting the block above — the quote as originally published stays byte-for-byte as
it was, and the byte-exact paste is supplied here beside it, at the source's own wrapping:

```
### §8.3 — And the family is NOT blocked by it, which is why this is a legitimate terminal branch

**The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be.**
`ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the cursor-age fence
with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound the reclaimable set by **live
claims**, not by any hypothesis about *why* the current prune falls behind. **A selection defect, a
scheduling defect and a throughput defect are all *contained* by a registry that never reclaims below a
live claim.**

What an unclassified cause costs is **fix-shape efficiency** — the family would design without knowing
which limb to optimize first — **not safety, and not the family's ability to proceed.** A Step-5 outcome is
therefore to be read as **an expensive answer, not a blocked one**, and any Step-5 outcome must be reported
quoting this paragraph beside it.
```

Reproduce with `sed -n '465,478p' spec356-manifest.md`; the fenced block above is that
command's output. Nothing in the determination depends on which of the two renderings a
reader uses — they carry identical text.

## The determination, stated plainly

The cause of PD-F12's production contradiction is **unclassified**. Per §8.3, this
costs **fix-shape efficiency** — the `ReclamationRegistry` family will have to design
without knowing which limb (selection, scheduling, or throughput) to optimize first —
and costs **nothing on safety**: the registry design (cursor-shaped consumers only,
`N = 30 d` retention SLA, cursor-age fence with HLC-horizon quarantine,
`ceiling = min_live_claim − fixed_margin`) bounds the reclaimable set by live claims
regardless of which of the three defect classes turns out to be the cause, or whether
the cause is ever classified at all.

## E-C's HARD STOP

E-C's own text: *"If this increment does not name the mechanism, the diagnosis line
HARD STOPS. There is no diagnosis round after this one."* "Does not name the
mechanism" is defined purely mechanically: *"the frozen table resolves to V0, or Step 0
fires fail-closed with any `INDETERMINATE-*`."* The frozen table resolved V0. **E-C
fires.** There is no further diagnosis round for PD-F12 under this spec. The
`ReclamationRegistry` family (`TODO-634`) starts its design with the cause
unclassified, exactly as E-C specifies, justified by §8.3 above.

## The POST-DATA findings — routed, not a naming

Five POST-DATA findings (PD-F15, PD-F16, PD-F17, PD-F18, and PD-F19 added by Review v1)
were produced by the code audit the cross-vendor round recommended
(`spec358-mechanism-xask.md`) and by the review that followed it. §12.0
pre-authorizes exactly this outcome: *"A reading not on this table cannot be published
as this round's verdict; if the data suggests one, it is recorded as a POST-DATA
finding and routed, and the table's own verdict still stands."*

PD-F15 in particular names a code-level mechanism —
`drain_prunable`'s `drained_epochs.insert(e)` being unconditional on `refs.len()`,
which produces exactly PD-F12's production signature when an epoch's `epoch_tags`
entry exists but holds an empty vector. **This does not amount to "the increment named
the mechanism after all."** The D-T's row 1 (V1 REF-LOSS) ranges over `D5`'s own
universe — the 3000-epoch sequential, single-writer drive — and evaluated **false**
there, by construction: D5 cannot construct an empty-but-present `epoch_tags` entry
through the public API, because refs leave only via the drain, which removes the whole
entry. PD-F15 is a STATIC, code-level observation reached by audit, not a row-1
violation reached by execution inside D5's frozen universe. The table's own verdict
(V0) stands unchanged. PD-F15-PD-F19 are recorded in `spec356-manifest.md` §12.1,
clearly labelled POST-DATA, and routed to `TODO-634` for the `ReclamationRegistry`
family's design phase to carry as a stated open input — they do not stand in for, and
must not be read as, a naming that would have prevented E-C from firing.

**PD-F19 qualifies how much this `V0` weighs, and the family must inherit it at that
weight:** D-T row 2 was, like row 1, unfireable over `D5` by construction — `D5` seeds
every key present with its own tombstone tag, so every epoch settles with `D_e ≥ 1 ∧
F_e > 0` while row 2 requires `D_e == 0 ∧ F_e == 0` — so over `D5` only row 3 was ever
live, and `V0` means "the one live row did not fire", not "three independent readings
were tested and none fired". The verdict itself is untouched: a row that cannot hold by
construction does not hold, which is exactly what `V0`'s definition reads.

## Two notes on how this round's own record should be read (added by Review v1)

- **D-T row 3's published value is asserted, not computed.**
  `sim/tombstone_gc_proof.rs` sets `row3_condition` to the literal `false`, and the
  `--nocapture` line consequently prints `row3(V3 COUNTER-FAMILY-MISNAMING)=false` in the
  same shape as the two genuinely computed row values. This is **frozen-by-design**, not an
  oversight: row 3's condition is "I1 or I2 is VIOLATED on an in-scope pass", and I1/I2 are
  asserted true on every in-scope pass ABOVE that line, so a violation surfaces as the
  test's own panic and never reaches the evaluation at all. `false` is therefore the only
  value the print site can be reached with. A reader comparing the three printed row values
  should read row 3's as "asserted, and a violation would have panicked above", where rows 1
  and 2 are computed off the capture.
- **Spec-clause markers in the four counted `.rs` files are inherited style.** The files
  carry `R4.6`, `AC4a`, `R1.5`, `C12`, `K1`, `X21-d` and similar in doc-comments. These
  resolve to a `.specflow/` document that is local-only and archived when the spec closes,
  so they go dangling for a future reader. No action taken here, and none is owed by this
  round: the style predates it (the base commit already carries 47 such markers in
  `tombstone_frontier_impl.rs` and 41 in `crdt.rs`), and the rule that IS machine-checked —
  no `SPEC-NNN` provenance in code comments — passes clean over all four files.
