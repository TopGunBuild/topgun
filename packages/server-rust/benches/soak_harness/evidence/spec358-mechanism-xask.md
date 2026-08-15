# SPEC-358 — the cross-vendor adversarial round (R7.2)

Consultant: `openrouter z-ai/glm-5.2`, run via `~/Projects/agent-future/scripts/openrouter-ask.sh`.
Two raw calls were made; both are the record (see the honesty note at the end of this
file for why there are two).

R7.2 is specific: the anchors are carried NATIVELY by this artifact — never eight
pasted strings — and EACH anchor line carries one of three disposition labels
(`APPLIED` / `RECORDED` / `REFUTED`) so the check grades engagement, not transcription.

## Anchors, with disposition and reasoning

1. **`REFUTED`** — "V0 was structurally guaranteed because Prometheus is a no-op in
   the sim."

   Refuted: the pass ROW (the `kind = "prune_pass"` tracing emission, §12.0's
   `K_p`/`E_p` transport) is a same-record mirror of exactly what `observe_pass` folds
   into Prometheus — both are built from the SAME `PrunePassRecord`, in the SAME call
   (`MetricsPruneRecorder::observe_pass` increments `passes_total`, `considered_total`,
   `dropped_total`, `bytes_freed_total`, `epochs_drained_total` and the empty/nonempty
   split from that one record). So the pass-ledger VALUES the D-T reads are read, not
   the transport that happens to also carry them to Prometheus; D5's Prometheus handles
   being permanent no-ops in-process does not manufacture the V0 verdict, because the
   D-T never touches Prometheus at all — it touches the tracing-side mirror of the same
   record. What D5 genuinely cannot reach is stated precisely under PD-F15: an epoch
   whose `epoch_tags` entry EXISTS but holds an EMPTY vector, reachable only through a
   second writer racing the first — which D5, being single-writer and sequential,
   structurally cannot construct through the public API.

2. **`REFUTED`** — "a non-drain path stamps `DrainedByPrune` (route b), possibly via
   `#[default]`."

   Refuted from source: `finalize_epoch_exit` resolves `kind_hint == None` to
   `EpochExitKind::Unclassified { .. }` — not to `DrainedByPrune`. The two non-drain
   call sites pass an explicit kind: `ClearedByRebuild` (`:835`) and
   `StillResidentAtShutdown` (`:651`). `EpochExitKind` does carry
   `#[default] DrainedByPrune`, but that default is reachable only from
   `..Default::default()` in test fixtures — no production call site relies on the
   derive. No non-drain path stamps `DrainedByPrune`. (= PD-F16.)

3. **`APPLIED`** — "audit `publish_epoch_exit` call sites before closing; it is cheaper
   than any drive."

   Applied: the audit was run. It is what produced PD-F16 (route (b) refuted from
   source, above) and confirmed PD-F15 (below): every `publish_epoch_exit` call site
   and every `EpochExitKind` construction site was enumerated and read.

4. **`APPLIED`** — "route (c): the drain publishes an exit for an epoch whose ref
   vector was empty at removal time."

   Applied and confirmed in source: inside `drain_prunable_tombstones`,
   `drained_epochs.insert(e)` sits inside `if let Some(refs) = epoch_tags.remove(&e)`
   and is **unconditional on `refs.len()`**. So an epoch whose `epoch_tags` entry
   EXISTS but holds an EMPTY vector is attributed `DrainedByPrune`, given
   `bytes_freed_attributed = slot.stamped_bytes` (> 0 whenever the slot ever stamped),
   observed as `R_obs = 0, B_obs = 0`, and contributes NOTHING to `drained` — so the
   service's `PrunePassRecord` reads `considered = 0, empty_drain = true,
   epochs_drained = 0`. That is precisely PD-F12's production signature, and it is
   exactly D-T row 1's named extreme case (`R_obs == 0 ∧ R_ent > 0`), which Step 0 leg
   (c) proves the instrument fires on when the antecedent exists. Why D5 could not
   reach it: through the public API an epoch's refs leave ONLY via the drain, which
   removes the WHOLE `epoch_tags` entry — never leaving an empty-but-present vector
   behind — and D5 is single-writer and sequential, so no second writer can empty the
   vector between entry-emission and drain. (= PD-F15.)

5. **`RECORDED`** — "multiple `TombstoneFrontier` instances (route a)."

   Recorded, not investigated: out of this increment's scope (X5 forbids a fix, X8
   forbids widening the frozen universe); routed to `TODO-634`.

6. **`RECORDED`** — "deferred/asynchronous exit publication (route e)."

   Recorded, not investigated; routed to `TODO-634`.

7. **`RECORDED`** — "partition fan-out (route d) cannot alone produce all-empty
   records."

   Recorded; consistent with PD-F17 (the Prometheus pass family is internally
   coherent — `observe_pass` builds every counter from the same `PrunePassRecord` in
   the same call, so the production symptom is not a counter-transport artefact).

8. **`RECORDED`** — "the named next drive (construct the race's END state, assert on
   the pass record)."

   Recorded as PD-F18 and routed to `TODO-634`; **NOT run here**, because X5 forbids a
   fix and X8 forbids widening the frozen universe, and it is not in any frozen
   universe this increment committed to before Step 0 ran.

## Honesty note on the two raw calls

The consultant's first response (`xask_raw.txt`) hit the OpenRouter token ceiling —
`[warn] ANSWER TRUNCATED at max_tokens (finish_reason=length)` — after producing only
the opening framing of its skeptic's-review argument (Choice A / Choice B on why D5's
construction structurally guarantees V0). A focused second call (`xask_raw2.txt`) was
made, re-posing the question with the eight routes and asking for a ranking and a
stop/continue recommendation; it completed without truncation
(`finish_reason` not truncated, `completion=9904`). **Both raw outputs are the
record** — nothing from the first (truncated) call is discarded, and the anchors above
draw from both: anchor 1 answers the first call's Choice-A argument directly; anchors
2, 5-8 draw from the second call's route ranking and code-audit recommendation; anchors
3-4 record that the recommended code audit was in fact run and what it found.

Raw files: `spec358-xask-raw1.txt` (first call, truncated) and `spec358-xask-raw2.txt`
(second call, complete), both committed beside this artifact. Both are the record this
artifact draws its anchors from. *(Review v1 correction: these were originally written
only to a session scratchpad and named `xask_raw.txt` / `xask_raw2.txt` here. A record
this file calls "the record" may not live somewhere that is deleted with the session, so
the two files were copied into the evidence directory unmodified and the names above now
point at the durable copies.)*

### Anchor 1's refutation is narrower than it reads (added by Review v1)

Anchor 1 refutes the claim *"V0 was structurally guaranteed because Prometheus is a no-op
in the sim"* — and that refutation stands exactly as written, against the
**transport-based** form of the structural argument. It does **not** dispose of every form
of it. Review v1 raised a different structural argument that the anchor does not reach:
`D5`'s SEEDING construction (every key seeded present with its own tombstone tag) forces
`D_e ≥ 1 ∧ F_e > 0` on every epoch, which makes D-T row 2 unfireable over `D5` by
construction, exactly as PD-F15 shows row 1 to be. That point is conceded, recorded as
**PD-F19** in `spec356-manifest.md` §12.1.f and routed to `TODO-634`. It does not change
the verdict — a row that cannot hold does not hold — but a reader who takes anchor 1 as
having settled "was V0 structurally guaranteed?" in general would be taking it further
than it goes.
