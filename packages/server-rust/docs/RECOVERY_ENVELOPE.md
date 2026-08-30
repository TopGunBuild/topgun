# Recovery Envelope

Three knobs decide how much history the server keeps for a client that has fallen behind. They are
easy to confuse, because all three sound like "how far back do we go", and two of them are not yet
implemented. This page is the single place where all three are stated together, with the unit each
one is denominated in and the reason it does not collapse into the other two.

## The envelope

| Knob | Unit | Value today | Hazard hedged | Owner |
|---|---|---|---|---|
| Retention SLA `N` | wall-clock **days** | *not yet implemented* (proposed default: 30 d) | how long a device may be offline before its cursor is fenced | deferred — `TODO-634` |
| Clock-skew tolerance | wall-clock **minutes** | *not yet implemented* | a stamp from the future (`phys(stamp) > server_now + skew_max`) | deferred — `TODO-634` |
| Reclamation margin | **epochs** | `0` (`DEFAULT_RECLAMATION_MARGIN_EPOCHS`, overridable via `TOPGUN_RECLAMATION_MARGIN_EPOCHS`) | an over-reported claim | implemented — `packages/server-rust/src/reclamation_registry.rs` |

### Why they do not collapse into each other

- **Retention SLA vs reclamation margin.** The SLA's hazard is *elapsed time*: how long a device may
  be away before the server stops keeping its place. The margin's hazard is *a wrong number*: a
  client claiming progress it has not made. One is denominated in days because clocks measure
  absence; the other in epochs because an over-reported claim is wrong by some count of epochs.
  Converting between them requires a churn rate — epochs per day — which varies per deployment and
  per workload. A knob that pretends the conversion is fixed is false precision.
- **Clock-skew tolerance vs the other two.** Skew tolerance bounds how far a *timestamp* may lead
  the server's own clock before the stamp is rejected. It gates admission of a stamp; it does not
  decide how long anything is kept. A deployment can be perfectly clock-synchronised and still need
  a long SLA, and vice versa.
- **The reclamation margin hedges over-reported claims only.** It is **not** a durability mechanism
  (durability is the WAL fsync policy and the write-behind drain) and it is **not** the fix for
  min-pinning (an abandoned laggard that never releases pins the boundary regardless of the margin;
  the fix is the cursor-age retention fence, listed as deferred above). Its hedge is also **narrow**
  by construction: `packages/server-rust/src/tombstone_frontier_impl.rs:428` clamps every ACK to
  `claimed.min(delivered).min(current_max_epoch)`, so a client cannot claim above the highest epoch
  its connection was actually delivered. The margin covers only a claim over-reported *within* that
  delivered range.

## Normative clauses

**(a) These three values may not be changed independently without re-reading this table.** That is
the reason they are written down in one place. Raising the retention SLA does not license lowering
the reclamation margin, and tightening the skew tolerance does not shorten the SLA; each hedges a
hazard the other two do not touch. A change to any one of them is a change to the recovery envelope
as a whole and must be argued as such.

**(b) A derived margin sits on top of a boundary that already retains one epoch, and must not
re-add it.** A client cursor at epoch `e` means *applied through `e` inclusive*, while the
eligibility conjunct the sweep filters on is strictly-below (`ceiling > e`,
`packages/server-rust/src/tombstone_frontier_impl.rs:971`). The newest epoch the fleet has confirmed
is therefore already retained: a reclamation margin of `0` is **not** zero conservatism. Any future
rule that derives the margin from observed client lag — the `ceil(p99.9) + 1` shape sketched for the
lag-telemetry work — is stated **relative to that already-retained epoch**. Adding `+ 1` on top of
it double-counts by one epoch, permanently, on every sweep.

## Deferred, and where each item is tracked

None of the following is implemented in this tree. They are recorded here by id so an operator
reading the table above can see what the envelope is *missing*, not only what it has. All four are
tracked on **`TODO-634`**:

- the **retention SLA** (`N` days) and the cursor-age fence that enforces it;
- the **clock-skew tolerance** and its rejection of future-dated stamps;
- **lag telemetry** — per-sweep client lag with rolling p50 / p99 / p99.9 and a per-client outlier
  flag;
- the **derived margin** computed from that telemetry, with a hard cap and operator override in the
  downward direction only, subject to clause (b) above.

Until they land, the effective envelope is exactly one knob: the reclamation margin, at `0`.
