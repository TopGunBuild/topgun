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

Four POST-DATA findings (PD-F15, PD-F16, PD-F17, PD-F18) were produced by the code
audit the cross-vendor round recommended (`spec358-mechanism-xask.md`). §12.0
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
(V0) stands unchanged. PD-F15-PD-F18 are recorded in `spec356-manifest.md` §12.1,
clearly labelled POST-DATA, and routed to `TODO-634` for the `ReclamationRegistry`
family's design phase to carry as a stated open input — they do not stand in for, and
must not be read as, a naming that would have prevented E-C from firing.
