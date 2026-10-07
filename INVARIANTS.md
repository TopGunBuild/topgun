# TopGun Invariants Catalog

Every durability/correctness invariant the system relies on, each mapped to the code that
maintains it and the test that enforces it. **An invariant without an enforcing test is marked
`NAKED` with a tracking TODO — visibly, on purpose.** CI (`scripts/check-invariants.sh`) verifies
that every cited enforcing test still exists and that no new entry lands without either a test or
an explicit `NAKED` marker; the gate is "the NAKED count never grows silently", not "zero NAKED".

Conventions: IDs are `TG-<DOMAIN>-<NNN>` (domains: WAL, WB write-behind, OR, LWW, MRK merkle,
EVI eviction, SYNC, NAME map names, KEY per-key writer, DISP dispatch). Cite the ID verbatim in code comments and test names. Statuses:
`decided` (holds by design) · `open (SPEC/TODO-nnn)` (not yet true / not yet wired) ·
`aspirational`. Precedent: omnigraph `docs/invariants.md` (structure) — improved here with the
CI check it lacks. Origin: extraction memo 2026-07-16 + SPEC-350/351 closures.

---

### TG-WAL-001: Acked writes are durable under `kill -9` when `WalFsyncPolicy::PerOp` is active

- **Scope:** `WriteBehindDataStore` write path with a WAL bootstrap, `PerOp` policy.
- **Statement:** every write acked `Ok(())` is present after `WalRecovery::run` replays a WAL that
  survived an unclean shutdown (acked-before-crash ⊆ present-after-recovery). Under `PerOp`,
  fsync completes before the append returns.
- **Maintaining code:** `packages/server-rust/src/storage/wal/mod.rs` (PerOp arm, `sync_data`
  before return); `write_behind.rs` append-before-ack path.
- **Enforcing test:** `packages/server-rust/src/storage/crash_safety_proptest.rs` (the file's
  stated oracle is exactly this invariant; real store + WAL, SIGKILL modeled by store drop).
- **Violation consequence:** silently vanished acked writes after restart — worst CRDT-storage
  class; client re-converges only if its own op-log survived.
- **Discovered by:** SPEC-331/332/333 durability chain.
- **Status:** decided (PerOp). Batched is deliberately weaker — see TG-WAL-002.

### TG-WAL-002: `Batched` (default) fsync loss window is bounded to the group-commit window

- **Scope:** `WalFsyncPolicy::Batched` (production default).
- **Statement:** an unclean shutdown may lose acked writes appended since the last group-commit
  fsync (~10 ms timer / 100 frames) — and NO MORE than that window. The gap is a documented
  product trade-off (CLAUDE.md); its BOUND is the invariant.
- **Maintaining code:** `wal/mod.rs` batched group-commit timer task.
- **Enforcing test:** `NAKED — no test proves the loss window is bounded to
  writes-since-last-sync and no wider (TODO-602)`. `crash_safety_proptest.rs` proves the PerOp
  positive only.
- **Violation consequence:** an unbounded loss window under the default policy — the documented
  trade-off silently becomes a lie.
- **Discovered by:** extraction pilot audit 2026-07-16; fsync-tier asymmetry noted vs TiKV.
- **Status:** decided (gap intentional), **enforcement NAKED (TODO-602; the natural vehicle is
  SPEC-352b / TODO-603 — a Batched-policy truncate-to-durable-frontier fault schedule, which the
  in-process crash harness deliberately does not model)**.

### TG-WAL-003: The applied watermark is durably fsynced before any sealed segment is unlinked

- **Scope:** `mark_applied` → segment GC ordering, all policies.
- **Statement:** the watermark sidecar write+fsync completes before any sealed-segment unlink it
  licenses; a crash between them must not lose or corrupt data (GC is resumable, replay
  idempotent under the watermark filter).
- **Maintaining code:** `wal/mod.rs` `mark_applied` (fsync-before-unlink block + apply-time
  re-validation before physical delete, SPEC-350).
- **Enforcing test:** `wal_harness/cases.rs::ac7_tg_wal_003_gc_crash_point_both_directions` —
  drives the real `mark_applied` with a crash injected BETWEEN the sidecar fsync and the unlink
  loop: the production `FsyncThenUnlink` order loses nothing and recovery replays every
  acked-but-unapplied frame, while the inverted `UnlinkThenFsync` order (post-unlink/pre-fsync
  crash) loses data — the both-directions proof. `prefix_watermark_proptest.rs` (SPEC-350) also
  drives GC gating + boot seeding across incarnations.
- **Violation consequence:** under-seeded `max_observed_sequence` on restart → sequence reuse →
  recovery filter silently drops frames.
- **Discovered by:** SPEC-330 era; hardened by SPEC-350; crash-point injection proved by SPEC-352.
- **Status:** decided, **enforced** (crash-point injection closed by the harness).

### TG-WAL-005: The per-partition applied watermark is prefix-complete across incarnations

- **Scope:** `W(p)` tracker in `write_behind.rs` + `wal/mod.rs` (SPEC-350).
- **Statement:** `W(p) = min(unresolved wal_seq) − 1`; W never advances past a frame that is
  neither durably applied to the inner store nor superseded-by-carried-successor — including
  across restarts (boot-seeded from `wal.unapplied(p)`); an unseeded partition refuses to
  advance.
- **Maintaining code:** the tracker bundled with `Arc<dyn Wal>` (one struct, cannot diverge);
  boot seeding; unseeded-refuse guard.
- **Enforcing test:** `prefix_watermark_proptest.rs` — restart-crossing proptest; both loss-guards
  (unseeded-refuse, seed-retry) verified by revert during review (fail on pre-fix code).
  Generatively re-enforced by `wal_harness/cases.rs::ac4_c3_scalar_max_watermark_regression`
  (the C3 scalar-max over-advance, found from generated crash/recover sequences).
- **Violation consequence:** the SPEC-350 headline defect — acked writes of one key silently
  dropped from replay because another key's flush advanced a scalar watermark past them.
- **Discovered by:** SPEC-349 Audit v2 (three independent derivations).
- **Status:** decided, **enforced**.

### TG-WAL-006: WAL re-replay is merge-idempotent for `RecordValue::Lww` (enforced, LWW-scoped)

- **Scope:** `WalRecovery::run` replay through `replay_entry`, for `RecordValue::Lww` values.
- **Statement (positive):** re-replaying a WAL frame whose value is OLDER than the current durable
  value MUST NOT change the durable value, for `RecordValue::Lww`: `replay_entry` reads the current
  value and discards a modern Lww frame whose HLC timestamp is strictly lower (last-write-wins by
  timestamp); ties and newer timestamps write through. `WalStorePayload::Legacy` frames (synthesized
  always-merge timestamp), `RecordValue::OrMap`/`OrTombstones` frames, and a cross-kind (non-Lww
  stored) value BYPASS the gate and keep the pre-existing blind replay. `write_one` stays a
  CRDT-agnostic blind insert; the merge lives at the recovery boundary.
- **Maintaining code:** `wal/mod.rs::replay_entry` (the `RecordValue::Lww` read-compare gate) +
  `run` / call-site doc-contracts; `datastores/redb.rs` (`write_one` doc-comment records the
  guarantee lives upstream).
- **Enforcing test:** `wal/mod.rs::tests::replay_lww_gate_discards_older_frame_isolated` (older
  discarded, ties/newer through, gate-off clobbers) and `::replay_or_crosskind_and_legacy_bypass_lww_gate`
  (the bypass proof, since the harness model is LWW-only); the harness value-equality case
  `wal_harness::cases::ac4_5_replay_clobber_caught_by_value_equality_oracle`.
- **Superseded (still true, kept):** the weaker in-window proptests
  `prefix_watermark_proptest.rs::a_frame_written_after_the_partition_drains_is_still_replayed`
  and `::a_mid_loop_remove_all_failure_still_replays_the_earlier_tombstones` remain valid and are
  subsumed by the stronger property above.
- **OR residue (routed, NOT closed here):** OR-Map merge-idempotency as an independent property is
  owned by `TG-OR-003` / SPEC-349b (tracked by TODO-608), where delta-fold-delegates-to-live-apply
  answers it by construction; this invariant covers only the `RecordValue::Lww` case.
- **Violation consequence:** timestamp regression on crash-recovery — a stale re-replayed frame
  resurrects an older durable value.
- **Discovered by:** SPEC-350 execution (AC4(b) honest-unmet escalation); closed by SPEC-353.
- **Status:** decided, **enforced (LWW-scoped)**.

### TG-WAL-007: WAL write-path failures fail-stop through one abort-based mechanism

- **Scope:** `wal_fail_stop(tier, ctx) -> !`; Err taxonomy (P)/(A)/(B).
- **Statement:** (P) sealed-target = programming bug → abort; (A) pre-frame errors (encode/open,
  bytes provably not in segment) → rollback of the frameless seq only; (B) write/fsync errors
  (frame possibly in segment) → abort, never retry the fsync (fsyncgate: PostgreSQL 2018, TiKV).
  Discrimination is STRUCTURAL (pre-checks), never parsed from error content. Abort survives the
  workspace's `panic = "abort"` prohibition and tokio unwind containment.
- **Maintaining code:** `wal/mod.rs` pre-check seam + `wal_fail_stop` (`#[cfg(test)]` seam panics
  for observability).
- **Enforcing test:** `prefix_watermark_proptest.rs::a_pre_frame_append_failure_removes_only_the_frameless_sequence_from_add`
  (+ `_from_remove` twin) and `::a_post_frame_append_failure_fail_stops_at_tier_b_without_rolling_back`
  — the (c1)/(c2) inverted pair (frameless removed / frame-backed NOT removed, loss-class guard).
- **Violation consequence:** continuing on a broken WAL (silent corruption) or rolling back
  frame-backed seqs (the AC2(c2) resurrection defect).
- **Discovered by:** SPEC-350 Audits v10–v12.
- **Status:** decided, **enforced**.

### TG-WAL-008: The stalled-watermark alarm classifies two ways and never fires on correct code

- **Scope:** TrackerLeak vs AbandonedWrite classifier + two-sample confirmation.
- **Statement:** `TrackerLeak` (code bug) fires only for a Live seq absent from BOTH queue and
  in-flight registry on TWO independent samples separated by the derived re-confirm delay;
  a hung store/disk classifies `AbandonedWrite`; a transient ownerless window (resolve between
  samples) fires nothing.
- **Maintaining code:** classifier in `write_behind.rs`; `max(bound/60, floor)` derived delay.
- **Enforcing test:** `prefix_watermark_proptest.rs::a_hung_inner_store_is_an_abandoned_write_not_a_leak`
  and `::a_boot_unreplayed_sequence_is_an_abandoned_write_not_a_leak` (classifier matrix by
  scrape, commit `b3e0e89b`) incl. the transient-window negative control (AC3(a)(viii)).
- **Violation consequence:** operator misdirection — a disk-full incident diagnosed as a code
  bug, or a real leak suppressed.
- **Discovered by:** SPEC-350 Audit v7 (two-class split), v10–v12 (races).
- **Status:** decided, **enforced**.

### TG-WAL-009: WAL re-replay is idempotent for `WalOp::Remove` (enforced, Remove-scoped)

- **Scope:** `WalRecovery::replay_entry` `WalOp::Remove` arm. Modern Remove frames carry
  `timestamp: None`; replay does not consult it.
- **Statement (positive):** a `WalOp::Remove` replays UNCONDITIONALLY — a blind delete, identical to
  the live remove path (`WriteBehindDataStore::remove`, which stages a pending-delete with no
  timestamp compare). There is deliberately NO replay gate: the live path has no such condition, so a
  gate would make `replay(Remove) != live(Remove)` and is itself unsound — an HLC-sourced gate
  wrongly skipped a Remove whose sourced value-HLC was strictly older than the LWW survivor, losing
  the acked delete (surfaced as `AckedWriteLost`). Idempotency is a property of the replay ORDER, not
  a per-frame timestamp compare.
- **Warrant (why no gate is NEEDED — structural non-reachability):** under prefix-complete, strictly
  in-order replay of the unapplied window, a stale Remove can never delete a strictly-newer
  re-creation. Let the Remove be at WAL sequence N and W the prefix-complete applied watermark
  (`min(pending) - 1`, never at/above an unresolved sequence). For the Remove to be replayed, N > W.
  A re-creation strictly newer than it has, by per-partition monotonic sequencing (= arrival order),
  sequence M > N; for that re-creation to be durable-but-unframed (not re-enumerated after the
  Remove) it must sit below the watermark, M <= W. Together: `M <= W < N < M` — a contradiction. So
  either every strictly-newer re-creation's frame is also above W (enumerated and replayed AFTER the
  Remove, re-creating the value) or the Remove is the newest op for the key (deleting is
  live-correct). The stale-Remove-over-newer-frameless-value juxtaposition is reachable ONLY by
  re-applying an already-applied older frame after the in-order pass — the harness
  `re_replay_oldest_frame` seam, never a production path. The chain rests on FOUR premises, none of
  them asserted prose — three are TEST-backed (each a catalogued enforced invariant with a real test:
  (a), (c), (d)) and one is COMPILER/TYPE-backed ((b): a pure free function, which is why it cites no
  invariant ID and needs no enforcing test):
  - **(a) prefix-complete watermark** — `W = min(unresolved) - 1`, never at/above an unresolved
    sequence, so `N > W` bounds the replay window: `TG-WAL-005` / `TG-WB-001` / `TG-WAL-003`.
  - **(b) one sequence space per key** — `partition_for(map, key)` is deterministic, so a key's ops
    are all sequenced in a single per-partition space (this is a SINGLE-SPACE claim, NOT itself an
    ordering claim; the ordering is (c)/(d)): `partition_for` is a pure function.
  - **(c) strictly-monotone per-partition sequence assignment** — a later arrival gets a strictly
    HIGHER sequence, which is what makes the re-creation `M > N`: `TG-WAL-010`.
  - **(d) strictly-ascending replay of the unapplied window** — a frame at `M > N` is replayed AFTER
    the Remove at `N`, so the re-creation survives: `TG-WAL-011`.
  Premises (c) and (d) carry the load-bearing `M > N` and replayed-after steps; (a) and (b) bound the
  window and the sequence space. (b) alone is necessary-but-insufficient for the ordering — it is
  (c)+(d) that order the space.
- **Windowing residual (tracked, NOT closed by a gate):** the frameless-`flush_key` window is a
  WATERMARK/FRAMING concern rather than a Remove-idempotency one, so it is handled there and not
  papered over with an unsound Remove-replay timestamp gate. Two distinct hazards live in that
  window, and only one of them is still open:
  - **Crash-window direction (TODO-612) — CLOSED, determined a NO-OP.** A frameless `flush_key`
    durable write racing an un-resolved older Remove leaves no enumerable past-older Remove for
    recovery to mis-order. Mutation-proven by
    `flush_key_frameless_window_leaves_no_enumerable_past_older_remove`
    (`datastores/write_behind.rs`, inline `#[cfg(test)] mod`), which drives the live `flush_key`
    trait method rather than a re-implementation. This bullet previously asserted the hazard as the
    "only genuine" one and routed it to TODO-612 as open; both halves of that were superseded by
    the determination.
  - **Caller-side direction (TODO-628) — OPEN, but vacuous at HEAD.** `flush_key` makes the
    CALLER's value durable while resolving the superseded entry's sequences, so a caller passing a
    value that does not subsume a staged `WalOp::Remove` loses an acked delete **with no crash at
    all**. It is vacuous today only because `flush_key` has zero production callers (every call
    site is under `#[cfg(test)]`), which is a property of the current call graph, not a guarantee.
    Must close before any production caller is wired.
- **Maintaining code:** `wal/mod.rs::replay_entry` (`WalOp::Remove` arm, unconditional) + `run` /
  `WalEntry` doc-contracts; `datastores/write_behind.rs` `remove`/`remove_all` append Remove frames
  with `timestamp: None`.
- **Enforcing test:** `wal/mod.rs::tests::replay_remove_replays_unconditionally_isolated`
  (strictly-older / tie / newer / sentinel / legacy-`None` Remove all delete unconditionally, under
  both `merge_gate` values — the discriminator vs the reverted gate) and the harness case
  `wal_harness::cases::ac1_ac6_stale_remove_clobber_caught_by_o1_oracle` (O1 `AckedWriteLost` catches
  the seam-injected out-of-order clobber; the `DefectMode::None` in-order run is green).
  **Coverage note (deliberate narrowing):** the GENERATIVE baseline is not a second guard for the
  gate-free property. `wal_harness`'s generator constrains arrival to a monotone-HLC domain (the O1
  oracle's sound domain, `make_hlc_monotone`), which narrows it out of the range where a reintroduced
  Remove gate would surface as `AckedWriteLost`. The isolated unit test above is therefore the SOLE
  enforcing guard against gate reintroduction; oracle genericity over the non-monotone domain is owned
  by TODO-610.
- **Sibling invariant (DISTINCT):** `TG-WAL-006` owns the same re-replay-idempotency loss-class for
  `RecordValue::Lww` VALUES — there a gate IS correct because live-Store is HLC-conditional (LWW), so
  replay must mirror that compare. Remove is the counterpart whose live semantics are UNCONDITIONAL,
  so it needs no gate; the two share the recovery-boundary layering but not a comparison basis.
- **Violation consequence:** acked-write loss on crash-recovery — either an unsound gate wrongly
  skips a legitimate delete, or (if the windowing residual were left unhandled) a stale-Remove
  clobbers a re-creation. Both surface as `AckedWriteLost`.
- **Discovered by:** SPEC-353 `/xask` (Review v1 flagged the Remove-clobber loss-class, routed to
  TODO-609); SPEC-354 Review v1 caught the unsound gate (an `AckedWriteLost` regression); closed by
  SPEC-354 via a pre-fix `/xask` + a structural non-reachability spike.
- **Status:** decided, **enforced (Remove-scoped)**.

### TG-WAL-010: Per-partition WAL sequence assignment is strictly monotone

- **Scope:** `WriteBehindDataStore::assign_wal_sequence` — the single mint point for a mutation's WAL
  sequence — per partition.
- **Statement:** every assigned WAL sequence is drawn from one process-global atomic counter
  (`next_wal_sequence`, a `fetch_add`), so a partition's assigned sequences are a strictly-increasing
  subsequence: no sequence is ever reused, and a later arrival gets a strictly HIGHER sequence than
  an earlier op on the same partition. This is what makes a re-created value carry `M > N` relative to
  an older op on the same key — premise (c) of `TG-WAL-009`.
- **Maintaining code:** `write_behind.rs` `next_wal_sequence` (atomic `fetch_add`) + `assign_wal_sequence`.
- **Enforcing test:** `write_behind.rs::tests::wal_sequence_assignment_is_strictly_monotone_per_partition`
  — 8 tasks on 8 DISTINCT partitions × 250 assigns each, asserting BOTH clauses: each partition's own
  returns are strictly increasing in ARRIVAL order (per-partition monotonicity, the `M > N` step), and
  all 2000 values are globally distinct (no reuse). Spreading across partitions is load-bearing for
  discrimination, not incidental: `assign_wal_sequence` mints inside `with_partition`, i.e. under that
  partition's OWN mutex, so a single-partition variant stays GREEN — and the whole lib suite with it —
  when the `fetch_add` is replaced by a racy load/yield/store, because the lock serializes the bump and
  hides whether the counter is atomic at all. Across 8 partitions the mutexes no longer overlap and the
  same mutation goes RED on the ARRIVAL-ORDER assertion (observed: partition 0 returned `… 83, 75 …`,
  i.e. the counter moved backwards), which aborts the test before the distinctness assertion is reached;
  a distinctness-only variant of this test reports the same break as ~705 distinct values out of 2000.
  Mutation-verified in both directions (green with `fetch_add`, red without).
- **Violation consequence:** sequence reuse or non-monotone assignment breaks both the prefix-complete
  watermark and `TG-WAL-009`'s `M > N` step, admitting a stale-Remove clobber.
- **Discovered by:** SPEC-354 Review v2 (cataloguing the gate-free warrant's premises).
- **Status:** decided, **enforced**.

### TG-WAL-011: WAL recovery replays the unapplied window in strictly ascending sequence order

- **Scope:** `Wal::unapplied` ordering contract + `WalRecovery::run` replay loop.
- **Statement:** `Wal::unapplied` returns the unapplied window sorted strictly ascending by WAL
  sequence — a defensive `sort_by_key(|e| e.sequence)` that holds REGARDLESS of the order frames were
  appended or enumerated across segments — and `run` replays that Vec in order. A frame at sequence
  `M > N` is therefore always replayed AFTER the frame at `N` (premise (d) of `TG-WAL-009`), and the
  contiguous-success frontier the applied watermark advances to is well-defined by sequence.
- **Maintaining code:** `wal/mod.rs::WalWriter::unapplied` (the `all.sort_by_key(|e| e.sequence)`
  before the `applied_seq` filter) + `WalRecovery::run`'s in-order replay loop.
- **Enforcing test:** `wal/mod.rs::tests::wal_recovery_replays_in_strictly_ascending_sequence_order`
  — frames appended out of order (seq 3, 1, 2) are returned by `unapplied` and replayed 1, 2, 3.
  Mutation-verified: removing the `sort_by_key` fails the FIRST assertion (`unapplied`'s returned order,
  `left: [3, 1, 2]` vs `right: [1, 2, 3]`), which aborts the test before the replay-order assertion is
  reached — so the enforcement is real, but it is the enumeration-order assertion that discriminates.
- **Violation consequence:** out-of-order replay would let a stale Remove delete a strictly-newer
  re-creation replayed before it, or miscompute the contiguous frontier — an acked-write loss.
- **Discovered by:** SPEC-354 Review v2 (cataloguing the gate-free warrant's premises).
- **Status:** decided, **enforced**.

### TG-WAL-012: A `WalOp::Store` OR frame is a COMPLETE post-state snapshot of one key

- **Scope:** `WalOp::Store` frames carrying `WalStorePayload::Record(RecordValue::OrMap { .. })`
  — the frame kind legacy WALs hold and bulk/SYNC ingestion still produces — as read by
  `WalRecovery::replay_entry`'s absolute-set `add`.
- **Statement:** such a frame carries the key's WHOLE post-state as of its own sequence: the live
  record set AND the tombstone set. Every effect at or below that sequence, removes included, is
  therefore already inside it. No partial, live-set-only or field-projected OR snapshot is framed.
- **Maintaining code:** the payload TYPE, not a runtime check. `WalEntry` is per-key and its
  `value` is a `WalStorePayload::Record(RecordValue)` — one whole value — and `RecordValue::OrMap`
  carries `records` and `tombstones` as non-optional fields. A partial OR snapshot is
  unrepresentable, not merely unwritten.
- **Enforcing test:** `wal_harness::cases::tg_or_003_ac3c_snapshot_frame_is_an_absolute_set_not_a_union`
  enforces the CONSEQUENCE — a snapshot tombstoning a live durable tag must REPLACE the slot, not
  union with it. The property ITSELF is COMPILER/TYPE-backed, the two-kinds-of-backing precedent of
  `TG-WAL-009`'s premise (b): it holds by construction of the payload type, so there is no mutation
  that could redden a behavioural test without failing to compile first, and it needs no test of
  its own for the same reason premise (b) cites none.
- **Violation consequence:** the absolute-set `add` in `replay_entry` becomes UNSAFE. Completeness,
  not recency, is what licenses that replace: tombstones a partial snapshot omitted would come back
  from the store's older value, resurrecting deleted tags after a crash. It is the load-bearing
  precondition of the OR fold's warrant (`TG-OR-003`).
- **Discovered by:** SPEC-349b Review v1 — the one precondition of that warrant left uncatalogued.
- **Status:** decided, **enforced (type-backed)**.

### TG-WB-001: The flushed watermark is prefix-complete — no mid-range hole

- **Scope:** entry-ordering-space `pending_seqs` / `flushed_watermark()` (tombstone fence
  consumer; INDEPENDENT of TG-WAL-005's wal_seq-space tracker).
- **Statement:** `flushed_watermark()` never returns a value above a still-buffered sequence;
  assign+track is atomic under one lock (the mid-range-hole guard).
- **Maintaining code:** `write_behind.rs` `assign_tracked_sequence` + `resolve_pending`.
- **Enforcing test:** `write_behind.rs::ac3c_flushed_watermark_prefix_complete_never_exposes_hole`
  + surrounding block (coalesce-resolves-a-hole, prune-frontier-stall regression). Additionally
  exercised by the `wal_harness` frame oracle (O2) via `ac3_ac14_baseline_coverage_and_timing`.
- **Violation consequence:** a tombstone pruned while its bytes are still RAM-only → resurrection
  after crash.
- **Discovered by:** SPEC-330.
- **Status:** decided, **enforced**.

### TG-WB-002: A crash rebuilds write-behind pending state solely from the durable WAL

- **Scope:** `WriteBehindDataStore` boot / `ensure_wal_seeded` across an incarnation boundary.
- **Statement:** an unclean crash discards ALL in-memory write-behind state (staging buffer,
  pending tracker, in-flight registry, seeded-partition set); the next incarnation reconstructs its
  pending/seeded state EXCLUSIVELY from `wal.unapplied(p)`, so no acked write depends on any
  in-memory structure surviving the crash. A partition boots empty and seeds lazily on first access.
- **Maintaining code:** `write_behind.rs` boot seeding (`ensure_wal_seeded` from `wal.unapplied`).
- **Enforcing test:** `wal_harness/cases.rs::ac2_crash_destroys_in_memory_state` asserts every
  non-first incarnation boots with an empty pending tracker before any op runs;
  `ac5_c12_empty_boot_seed_regression` proves the harness detects the blind-boot violation from
  generated cross-incarnation sequences, with a single-incarnation negative control.
- **Violation consequence:** a restart that trusts stale/absent in-memory state → the pending
  tracker boots blind, the watermark advances past un-applied frames, acked writes are lost (C12).
- **Discovered by:** SPEC-352 harness (built alongside the TG-WAL-003 crash-injection work).
- **Status:** decided, **enforced**.

### TG-WB-003: The inner store leads, never lags, the resolved frames; a pending OR key shares one slot cell

- **Scope:** `WriteBehindDataStore` over any inner `MapDataStore` — what a flush persists for a key
  relative to the WAL frames the write-behind has resolved — and the slot cell (`SlotCell`) a key
  whose latest pending entry is a `DelayedOp::StoreCell` shares between the engine, that entry, its
  staging slot and an in-flight flush.
- **Statement:**
  (a) The inner store's value for a key contains every mutation of that key whose frame the
  write-behind has resolved. It may also contain later mutations whose frames are appended but
  unresolved — recovery's in-order re-fold is idempotent over them (`TG-WAL-011`, `TG-OR-003`).
  (b) It may also contain mutations that were never framed: any in-place mutation whose op ended
  between the engine mutation and a successful append, whatever ended it (an append error, the
  shutdown gate, the capacity rejection, cancellation of the write, a crash), and in-place writes
  whose provenance owes no write-through. Every (b) mutation is un-acked and was already visible in
  memory, and OR re-application is idempotent, so a retry converges. A cancelled op never promotes
  its WAL sequence; while that sequence is `Appending`, the flushed watermark does not pass it
  (true before and after TODO-672 items 1–2 make the append cancel-safe).
  (c) For a key whose latest pending entry is a `StoreCell`, ONE cell is shared by the engine
  (while resident), that entry, its staging slot and any in-flight flush. Eviction or removal drops
  only the engine's reference; re-adoption (the materialize step of `update_in_place`, `get`'s load)
  re-inserts the SAME cell, and only through the generation-checked insert (`TG-OR-007`); a flush
  encodes the cell's state at flush time under the cell lock; a non-cell writer (`add()`, or engine
  `put` on a shared cell) never mutates the cell; the cell is freed when its last reference drops.
- **Maintaining code:** `write_behind.rs` — `add_with_witness` enqueuing and staging the caller's
  cell (`DelayedOp::StoreCell`, `StagedValue`), `load_slot` answering `Loaded::Cell`,
  `persist_entry` encoding the cell under its lock (`add_encoded`, else one clone into `add`),
  `hard_flush`; `hashmap.rs` — `Arc<SlotCell>` slots, `put_if_absent_at` / `update_in_place`
  adopting a `SlotInit::Cell`, `remove` / `remove_if` dropping only the engine's `Arc`;
  `default_record_store.rs` — `WriteSource::Cell` on the write-through, `load_slot` in the
  materialize loop and in `get`.
- **Enforcing test:**
  R6 (simulation, node failure, both `or_delta_wal` settings):
  `sim::or_delta_recovery::tests::a_flush_that_leads_an_unresolved_frame_recovers_to_the_partition_converged_state`,
  `sim::or_delta_recovery::tests::an_evicted_pending_key_re_adopted_before_its_flush_recovers_to_the_partition_converged_state`.
  Re-adoption shares the cell — AC-2b and AC-2d together:
  `storage::impls::default_record_store::tests::materialize::an_or_add_adopts_the_staged_cell_of_an_evicted_pending_key`
  (AC-2b: the cell taken BEFORE the eviction is `Arc::ptr_eq` to the slot after the OR_ADD, so it
  catches a re-adoption that copies the staged value into a new cell) and
  `storage::impls::default_record_store::tests::materialize::an_evicted_cell_is_re_adopted_while_its_flush_is_pending`
  (AC-2d: the engine's cell after `op2` is the queued entry's cell, and the flush persists
  old ∪ op ∪ op2; it catches a re-adoption that materializes a fresh cell beside the queued one,
  NOT a copying adoption — `op2` enqueues the copy it made, so the pointer check passes; AC-2b is
  the guard for that); `storage::impls::default_record_store::tests::materialize::a_get_adopts_the_staged_cell_until_the_flush_releases_it`
  (AC-2a: `get` adopts, the flush releases the queue and staging references, the cell is freed
  after a further eviction). (b), un-framed mutations (AC-2e):
  `storage::datastores::write_behind::prefix_watermark_proptest::an_unframed_mutation_after_a_pre_frame_append_error_is_unacked_and_idempotent`,
  `::an_unframed_mutation_rejected_by_the_shutdown_gate_is_unacked_and_idempotent`,
  `::an_unframed_or_remove_is_unacked_and_idempotent`,
  `::a_cancelled_append_never_lets_the_watermark_pass_its_sequence`,
  `::a_capacity_rejected_mutation_is_persisted_by_the_next_write_through`.
  (a), lead-not-lag (AC-3):
  `storage::impls::default_record_store::tests::materialize::two_in_place_writes_with_a_flush_between_persist_both`,
  `storage::datastores::write_behind::prefix_watermark_proptest::a_flush_racing_an_in_flight_op_persists_every_resolved_frame`
  (asserted against the WAL's resolved set). Flush of a cell to a non-encoding inner store (AC-6):
  `storage::datastores::write_behind::tests::a_non_encoding_inner_receives_each_flushed_cell_once_with_its_current_value`
  (one `inner.add` per flushed entry, carrying the cell's value at flush time).
- **Violation consequence:** (a)/(b) — a flush that encodes a stale cell, or skips a pinned cell on
  eviction, leaves the inner store behind a resolved frame whose WAL segment may then be collected:
  acked OR ops lost after a crash. (c) — a copy where the design shares a cell reintroduces the
  per-op whole-record copies the cell removes (engine, queue, staging), and a cell mutated by a
  non-cell writer, or re-inserted without the generation check, can resurrect a removed value.
- **Discovered by:** SPEC-373b (carve 9c part b — one slot cell per key).
- **Status:** decided, **enforced**.

### TG-WB-004: No write-behind queue shard guard is held across an await

- **Scope:** every guard on `queues` (the per-partition write-behind queue map, a `DashMap`) in
  `write_behind.rs`.
- **Statement:** a `queues` shard guard never lives across an `.await`. A store refusal therefore
  never blocks a synchronous `queues` access — a client write, a remove, the stall watchdog — for
  longer than a synchronous critical section.
- **Maintaining code:** `write_behind.rs` `flush_loop`, retry branch: the refused entry is put back
  on its queue in one statement, so the guard is a temporary that is gone before the retry backoff
  is awaited.
- **Enforcing test:** `write_behind.rs::the_queue_shard_is_free_during_a_retry_backoff` — a write to
  the partition under retry backoff must complete within 1000 ms. Two more enforcers cover other
  parts; the table below says which covers what.
- **Violation consequence:** the node stops. The flush task sleeps on a timer while it holds the
  shard lock; the stall watchdog (or any writer of that partition) blocks a runtime worker on the
  lock; if that worker was the one driving the runtime's timer and I/O driver, the timer never
  fires and the lock is never released. No new connection is answered, nothing is logged and
  SIGTERM is ignored — signals are delivered through the same driver — so the process has to be
  killed. Measured with the store refusing from the start and the default configuration: 5 of 5
  runs hung before the fix, 5 of 5 healthy after it.
- **Measured scope:** a refusal injected into the embedded (redb) backend, one or two refused
  keys. In that case the node keeps answering reads and new connections, logs exactly one discard
  line per refused key and exits on SIGTERM (2 ms to 1.46 s). Postgres was not measured. This row
  states that the node does not hang on a store refusal; it does not state that the node is
  otherwise unaffected by one — see "After a store refusal" below.
- **Discovered by:** SPEC-381a wave A0 (rollback testing); fixed and catalogued by SPEC-382.
- **Status:** decided, **enforced** — at the one site the test covers and for the guard shapes the
  lint reports; see the blind spots.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `the_queue_shard_is_free_during_a_retry_backoff` | ONE site: the retry branch of `flush_loop`. No other `queues` guard, no other map | CI (`cargo test`) |
| lint gate: `clippy::await_holding_invalid_type` over the type list in the root `clippy.toml`, and `clippy::await_holding_lock` | WIDER than the statement: every listed DashMap/DashSet guard type in every target of both workspace crates — within the shapes the table below shows it reports | CI (`cargo clippy`) |
| `tests/integration-rust/store-refusal-liveness.test.ts` | the CONSEQUENCE at node level for a store refusal — reads and new connections answered, exactly one discard line per refused key, clean exit on SIGTERM within 15 s — with one or two refused keys in each of its first four arms. No site in particular. Its `STORE-DOWN-FROM-START` arm is the one that goes red under the default configuration. The file now has five arms; the fifth, `REFUSAL-LIFTED-UNDER-LOAD`, refuses six load keys while they are still being written and belongs to `TG-WB-005` | local only (`pnpm test:integration-rust:liveness`) — left out of the default `pnpm test:integration-rust` run, locally and in CI, because it needs a second server build with the `fault-injection` feature (TODO-770) |

**After a store refusal — what this invariant does not cover.** Before the fix the node hung
within seconds of a refusal, so none of the states below was reached in practice. They are
reachable now. None of them is a violation of the statement above.

- **Write-behind accounting no longer leaks after a refusal.** This gap is closed: a refused
  entry that meets a newer write of its key is retired into that write, with its count, its
  entry sequence and its WAL sequences all disposed of. The contract, its one-writer-per-key
  precondition and what it still leaves out are in `TG-WB-005`.
- **A discarded write is re-applied only by a restart.** After max retries the entry sequence is
  resolved, so the tombstone fence moves, and the WAL sequences are abandoned on purpose, so the
  frame is replayed at the next boot. Until that restart the partition's WAL watermark stays
  below the frame, its WAL is not collected and, under continued writes, the watchdog reports
  `AbandonedWrite`. This holds when no newer write of the key is queued at the discard; when one
  is, the discarded entry's WAL sequences go to that write instead (`TG-WB-005`).
- **Shutdown can take the full `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`.** The flush loop looks
  at the shutdown signal once per batch and sleeps a backoff per refused entry. Measured with 40
  refused entries and the default ladder: 30.00 s, the whole timeout.

**What the lint reports — measured examples, not an exhaustive list** (clippy 0.1.93; one fixture
per shape, each holding a guard across an await). A shape that is not in this table was not
measured; do not read it as reported.

| Shape | Reported |
|-------|----------|
| `let g = map.get(&k).unwrap();` — a `Ref` binding | yes |
| `let e = map.entry(k);` — an `Entry` binding | yes |
| `let it = map.iter();` — an `Iter` binding | yes |
| `let g = set.get(&k).unwrap();` — a `DashSet` `Ref` binding | yes |
| a guard that is a temporary of the awaiting statement | yes |
| `let g = map.get(&k);` — an `Option<Ref>` binding | **no** |
| `match map.get(&k) { Some(ref r) => { ….await } None => {} }` | **no** |
| `if let Some(r) = map.get(&k) { ….await }` | yes |
| `if let Some(v) = map.get(&k).as_deref() { ….await }` | **no** |
| `let g = map.try_get(&k);` — a `TryResult` binding | yes |
| `let Some(g) = map.get(&k) else { return };` | yes |
| `match map.get(&k) { Some(r) => { ….await } … }` — a by-value binding, unlike the `ref r` row above | yes |
| `if let Some(mut q) = map.get_mut(&k) { ….await }` | yes |
| a `std::sync::Mutex` or `parking_lot::Mutex` guard (`await_holding_lock`) | yes |
| `for k in map.iter().map(\|e\| *e.key()) { ….await }` — an iterator adaptor as the `for` iterable | **no** |
| `let it = map.iter().filter(..).map(..);` — an iterator adaptor binding | **no** |
| `let found = map.iter().find(..);` — an `Option<RefMulti>` binding | **no** |
| `let v: Vec<_> = map.iter().collect();` — a `Vec<RefMulti>` binding | **no** |
| `let g = map.get_mut(&k);` — an `Option<RefMut>` binding | **no** |
| `(map.get(&k).unwrap(), 1u8)` — a guard inside a tuple | **no** |
| a struct field holding a `Ref` | **no** |

**Blind spots.**

- The lint reports a guard only when a value of a listed type is itself alive across the await.
  Wrap the guard in anything else — an `Option`, a `Vec`, a tuple, a struct, an iterator adaptor
  over `iter()`, including an adaptor used as a `for` iterable — and it passes unreported. The
  rows marked **no** above are examples of that, not the whole set.
- The unreported shapes were looked for by reading, twice, and by no gate. First, every `get` /
  `get_mut` / `try_get` / `try_get_mut` / `iter` / `iter_mut` / `try_entry` call on a DashMap or
  DashSet name in both crates was classified (326 calls, none live across an await). Second, a
  text scan during review looked for the wider set — an accessor call in a `let` / `if let` /
  `while let` / `match` / `for` head with an await in scope, an iterator adaptor binding, a
  function returning a guard type — and found no instance. Nothing re-runs either reading, so a
  new guard of an unreported shape is not caught by any gate.
- Both readings key on declared names. They do not see a receiver reached through a method call
  or a field path whose last segment is not a declared DashMap/DashSet name, or macro-generated
  code.
- `for` loops. A `for` over an iterator adaptor is measured as not reported (table above). A
  plain `for … in &map` or `for … in map.iter()` was not measured, so nothing is claimed for it;
  the first reading did not look at `for … in &map` or at `entry(..)`.
- A guard held across a blocking call that is not an `.await`.
- A guard moved into a closure capture: the lint matches the type's path. Not measured.
- A hand-written `Future::poll`.
- Code that is not compiled on the CI platform or under any enabled feature.
- Lock types that are not listed: redb transactions, `arc_swap` and `quick_cache` guards
  (TODO-767).
- The blocked side. A synchronous `queues.get` on a runtime worker is legal and stays legal; the
  invariant only forbids the holder from suspending. A shutdown path that does not depend on the
  runtime driver is TODO-767.

### TG-WB-005: Every write-behind entry that leaves a queue or a flush batch is accounted

- **Scope:** `WriteBehindDataStore` — the pending counter (`pending_count`), the entry-sequence
  set (`pending_seqs`) and the per-partition WAL-sequence tracker, on a node that is not shutting
  down and whose flush task is alive.
- **Precondition — upheld by `TG-KEY-001`:** the predicate below is proven for one writer per
  `(map, key)` at a time. For record keys that is what `TG-KEY-001` states: every write of a
  record, the LWW PUT path included, holds the record's per-key writer, so two paths can no
  longer write one key concurrently. A caller that writes this store directly, not through the
  record store, is outside `TG-KEY-001` and has to exclude a second writer of its key itself;
  the one known exception, the cursor-forget fallback, has no production caller (TODO-777). One
  deterministic test still runs an interleaving of two writers BELOW the per-key writer (listed
  below). That is evidence for accounting under that one interleaving, not a proof for
  concurrent writers in general.
- **Not claimed:** which VALUE the store keeps. That is the subject of `TG-KEY-001`, with its
  own exclusions, and this row adds nothing to it — in particular nothing for maps with
  automatic embeddings. What the carry below needs is stated here because it is why the
  precondition matters. Carrying a retired entry's WAL sequences onto the queued entry is safe
  only if the queued entry covers the retired one, which means it is the newer write of the key
  — the one-writer precondition. If two writers broke it and the queued entry held the OLDER
  value, storing that entry would resolve the newer write's frame although the newer value was
  never stored, and a restart would recover the older value. Nothing in the write-behind store
  can tell the two cases apart: an entry does not carry the value's own timestamp, and a later
  arrival with an older value has the higher entry sequence. The precondition is upheld by
  `TG-KEY-001`, so a record writer can no longer produce the case in which the queued entry
  holds the older value.
- **Statement:** an entry leaves a partition queue or a flush batch only through a site that
  disposes of all three: its count, its entry sequence, and its WAL sequences — resolved,
  carried onto a queued entry of the same key, or, only when no such entry exists, abandoned
  with the `Abandoned` tag. In particular a refused entry that finds a newer write of its key on
  the queue is retired into that write: the count goes down by one, the entry sequence is
  resolved, and the WAL sequences move to the newer entry under the same shard lock, so they
  resolve when it becomes durable and are replayed at boot if it never does. This happens before
  the retry backoff is awaited, so a cancelled flush task cannot leave it half done.
- **Mechanical predicate:** once the store accepts writes again and the write-behind store is
  idle — no entry is queued AND the flush loop is between passes, holding no drained batch
  ("every queue empty" alone is not idle: the loop may hold a batch it has just drained) —
  `pending_operation_count() == 0`, `flushed_watermark() == assigned_write_sequence()`, and no
  WAL sequence is `Live`.
- **Maintaining code:** `write_behind.rs` — `PartitionQueue::retire_into_queued`, called from
  `reinsert_front` (retry branch of `flush_loop`) and from the discard branch.
- **Enforcing test:** `prefix_watermark_proptest.rs::interleaved_refusals_and_writes_to_one_key_leave_nothing_unaccounted`
  (property: refusals, accepts, writes and removes of one key in generated order, 64 cases) and
  `write_behind.rs::a_refused_entry_superseded_by_a_newer_write_is_retired_into_it`. Both run in CI
  (`cargo test`). The table below lists the others and what each one covers.
- **Violation consequence:** all three last until the node is restarted. The write buffer loses
  one slot per dropped entry, because the capacity check reads the inflated counter. The cleanup
  of deleted records stops for the whole store, because the flushed watermark it waits for no
  longer advances. That partition's write-ahead log is not collected, and the stall watchdog
  reports `TrackerLeak`. No acknowledged write is lost: the newer entry carries the value and
  the pinned frame is replayed at boot.
- **Discovered by:** the SPEC-382 implementation review (TODO-773); fixed and catalogued by
  SPEC-383.
- **Status:** decided, **enforced** — under the one-writer precondition above, which
  `TG-KEY-001` upholds for record keys.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `interleaved_refusals_and_writes_to_one_key_leave_nothing_unaccounted` | the predicate, for one key and one writer, over generated orders of refusals and writes; both the retry path and the discard path | CI (`cargo test`) |
| `a_refused_entry_superseded_by_a_newer_write_is_retired_into_it` | the retry path, one fixed order: counter, watermark, WAL sequences, watchdog class | CI (`cargo test`) |
| `a_write_in_the_flush_window_then_a_refusal_leaves_nothing_unaccounted` | the retry path at its smallest: one write in the flush window, one refusal | CI (`cargo test`) |
| `a_superseded_retry_entry_stays_replayable_until_its_survivor_is_durable` | WHY the WAL sequences are carried and not resolved at once: with a second key in the same partition, a crash before the newer entry is durable must still replay the retired frame | CI (`cargo test`) |
| `a_subsuming_write_over_a_survivor_resolves_the_carried_sequences` | a later write that subsumes the newer entry resolves the sequences that entry carried. Also the ORDER on the retry path: inside the retry backoff the count and the flushed watermark already have their settled values, so accounting moved behind the backoff fails it | CI (`cargo test`) |
| `a_discarded_entry_with_a_queued_newer_write_is_retired_into_it` | the discard path: an entry on its last attempt, with a newer write queued, is not tagged `Abandoned` | CI (`cargo test`) |
| `a_discarded_delta_entry_stays_replayable_until_its_survivor_is_durable` | WHY the discard path carries too: with a second key in the same partition, a crash before the newer entry is durable must still replay the discarded frame | CI (`cargo test`) |
| `a_retired_entry_hands_its_earlier_due_time_to_the_queued_write` | the queued entry takes the earlier due time of the two, so a key rewritten during an outage keeps its flush schedule | CI (`cargo test`) |
| `two_writers_of_one_key_interleaved_between_sequence_and_insert_leave_nothing_unaccounted` | NOT an enforcer of the precondition. Evidence for accounting only, under one interleaving of two writers that the precondition excludes. It asserts nothing about the stored value | CI (`cargo test`) |
| `tests/integration-rust/store-refusal-liveness.test.ts`, arm `REFUSAL-LIFTED-UNDER-LOAD` | the predicate at node level: six keys refused while they are still written; after the refusal is lifted every partition's WAL lag returns to 0 | local only (`pnpm test:integration-rust:liveness`), for the reason given under `TG-WB-004` (TODO-770) |

**Not covered.** Each of these is an exit the statement names but no test above reaches, or a
case outside the precondition.

- **A discard with no newer write queued.** The frame stays pinned by design until a restart
  replays it (TODO-757).
- **Shutdown.** The flush task aborted at the shutdown timeout, the error, timeout and
  final-sweep exits of `hard_flush`, and a `hard_flush` future dropped mid-drain (TODO-768).
- **The flush task ending by a panic** on a node that keeps running (TODO-775).
- **`flush_key`** cancelled at either of its two awaits (the store call, the watermark advance),
  and `flush_key` against a key whose entry is in a flush batch. Latent: every caller is test
  code (TODO-775, with TODO-628).
- **`reset`** on a WAL-backed store with queued entries (TODO-775).
- **A write cancelled inside the WAL append** (`TG-WB-003` (b), TODO-672).
- **Two concurrent writers of one key.** The writers of a record key are serialised above this
  store (`TG-KEY-001`), so it no longer meets them. One interleaving below the writer is still
  tested here, for accounting only. The case in which the carry would make the outcome after a
  restart worse (see "Not claimed") is now covered by two tests driven through the service:
  `two_routes_and_a_refused_flush_recover_the_later_stamp_after_a_crash` and
  `an_older_value_arriving_behind_a_refused_newer_one_does_not_win_after_a_crash`.

### TG-EVI-001: Never-evict-dirty — an unflushed write is never evicted from the resident cache

- **Scope:** `evict_lru` in the record store.
- **Statement:** a record whose latest write has not reached the durable backend is not evictable,
  regardless of memory pressure.
- **Maintaining code:** `storage/impls/default_record_store.rs` dirty-skip; `RecordMetadata::is_dirty`
  is exact by write token (`stored_token != write_token`, set by `on_store`), not by millisecond, so a
  write stamped in the same millisecond as the previous persist stays dirty; `evict_lru` removes each
  snapshot candidate through `StorageEngine::remove_if`, re-checking under the key's lock that the
  resident is still clean and still the write the snapshot saw (SPEC-374).
- **Enforcing test:** `default_record_store.rs::evict_lru_skips_all_dirty_records` +
  `::evict_lru_skips_dirty_in_mixed_snapshot` + assertion in `eviction_cost_test.rs`;
  `::evict_lru_keeps_a_record_written_after_its_snapshot` (a write between the snapshot and the
  removal), `::a_write_in_the_mark_stored_millisecond_is_not_evicted` and
  `record.rs::a_write_in_the_stored_millisecond_is_dirty` (same-millisecond write).
- **Violation consequence:** eviction under pressure silently drops acked writes.
- **Discovered by:** eviction design (pre-catalog).
- **Status:** decided, **enforced**.

### TG-OR-001: `update_in_place`'s mutate closure runs at most once per call

- **Scope:** `RecordStore::update_in_place` seam (SPEC-347).
- **Statement:** one call invokes `mutate` at most once per attempt, and at most once per call
  (doc-contract, SPEC-347; attempts since SPEC-374): on a key that is durable but not resident the
  seam may make several attempts (resident probe, load, generation-checked insert), but only the
  outcomes returned BEFORE the closure (`Absent`, `Stale`) continue, and an attempt that runs the
  closure ends the call. Gauge side effects inside the closure must not double-count.
- **Maintaining code:** doc-contract + DashMap shard-lock path.
- **Enforcing test:** the literal call-counter assertion now exists —
  `or_inplace_mutate_proptest.rs::update_in_place_invokes_the_mutate_closure_exactly_once_per_call`
  counts invocations per call (`AtomicUsize`) on the insert, occupied and failed-write-through
  paths, and `::update_in_place_admits_the_take_once_shape_the_or_add_path_uses` pins the OR_ADD
  `Option::take` shape; both are mutation-proven RED against a second `mutate` call in either
  `engines/hashmap.rs` arm. The materialize path is counted by
  `::a_stale_materialize_retries_without_re_invoking_the_closure` (stale load, retry, one call),
  `::a_materialized_no_op_runs_the_closure_once_and_caches_nothing` (the prune shape) and
  `::a_key_absent_everywhere_without_init_loads_once_and_never_mutates`.
  `::new_tombstone_counted_once_across_write_failure_and_retry` covers the gauge half (no
  double-count across fail+retry).
  Scope of that evidence, stated honestly: it counts the production pair the record-store factory
  builds (`DefaultRecordStore` + `HashMapStorage`). Other `RecordStore` impls, including the trait's
  own default fallback, are not counted.
- **Violation consequence:** hidden internal retry double-applies CRDT mutations/gauge deltas.
- **Discovered by:** SPEC-347 review minors.
- **Status:** decided, **enforced** for the production store/engine pair.

### TG-OR-002: OR observers receive the documented `old_value` contract (post-image)

- **Scope:** observer fan-out on the in-place OR write path.
- **Statement:** `update_in_place` passes the post-image as "old value" (documented, intentional);
  no observer may silently depend on a pre-image. On the materialize path (a durable key that was
  not resident) the sequence is `on_load(pre-image)` then `on_update(post, post)`, never `on_put`.
- **Maintaining code:** SPEC-347 doc-contracts.
- **Enforcing test:** shape-only — the differential proptest matches notification COUNTS across
  legacy/in-place paths; content assertion on `old_value` is `NAKED (TODO-602)`. The materialize
  order is pinned by `default_record_store.rs::materialize_fires_on_load_then_on_update` and its
  Merkle leaf by `::materialize_merkle_leaf_is_the_leaf_of_old_union_op`.
- **Violation consequence:** a future observer reads `old_value`, silently gets wrong data.
- **Discovered by:** extraction pilot audit.
- **Status:** decided (scoped); enforcement shape-only.

### TG-OR-003: OR delta-fold recovery is semantic-set-equivalent to the snapshot path

- **Scope:** `OrDelta`/`OrDeltaFold`, wired on the RECOVERY READ side: `WalRecovery::replay_entry`'s
  `WalOp::OrDelta` arm and `impl OrDeltaFold for WalRecovery`. The matching WRITE side is live:
  `WriteBehindDataStore::add_with_witness` frames an OR mutation as `WalOp::OrDelta` when its
  `TOPGUN_OR_DELTA_WAL` arming flag is on **and** the mutation point handed it a witness; a `None`
  witness or a disarmed flag still frames a full snapshot, so both frame kinds remain reachable in
  production and both are in scope for this invariant.
- **Statement:** folding any op sequence through the delta path and the full-snapshot path yields
  equal `or_map_semantic_view` (live set + tombstones + pruned), with the durable store as fold
  base and snapshot frames as in-order absolute-set inputs.
- **Maintaining code:** types + oracle landed (SPEC-346); fold delegates to the live apply path
  (single-algebra rule, SPEC-349 R-mandate). That path now has a named anchor: `crdt.rs::apply_or_delta`
  — the ONE extracted pure apply of the add-wins / remove-wins / prune algebra, which the live OR_ADD,
  OR_REMOVE and epoch-prune call sites all route through (SPEC-349a). The delta fold must delegate to
  that symbol rather than re-implement the algebra; a second hand-written copy is what this invariant
  forbids, and the symbol is what SPEC-349b's delegation is checked against.
  The WRITE side is maintained code too: `storage/datastores/write_behind.rs`'s `add_with_witness`
  and `wants_or_witness` overrides — the ONE production emitter of `WalOp::OrDelta`, gated by
  `WriteBehindConfig::or_delta_wal` — and the witness-capture point on `service/domain/crdt.rs`'s
  OR write path, which produces the mutation the frame carries. A second store-side override of
  either method is a second framing decision, and the package-wide symbol count hosted in `crdt.rs`
  is what forbids one appearing unnoticed.
- **Enforcing test:** the `tg_or_003_*` case family in
  `packages/server-rust/src/storage/datastores/wal_harness/cases.rs` — a case family on the
  cross-incarnation harness, driven through its existing `Driver` and reference model, NOT a fork.
  14 cases: `tg_or_003_ac1_recovery_equivalence_over_every_fold_base_shape` (all three R1.3 fold-base
  shapes — durable `OrMap`, absent, legacy `OrTombstones` — plus the tombstone-bytes gauge via the real
  `storage::record::reconcile_tombstone_bytes` boot walk), `…ac3a…` (snapshot above the watermark with
  an empty store), `…ac3b…` (snapshot-only legacy window), `…ac3c…` (absolute-set: a snapshot
  tombstoning a live durable tag), `…ac3d…` (cross-kind/legacy base under a delta, post-state pinned
  literally), `…ac4…` (`Prune` as pure tombstone-set subtraction), `…ac5…` (single-algebra
  behavioural: remove-wins suppression of a re-added tombstoned tag), `…ac16…` (the injection rides the
  observed append seam only), `…ac2…` (stranded base + re-fold idempotency across `mark_applied` +
  segment GC), `…ac7b…` (the non-subsuming survivor's carry-forward route), `…ac9…` (delta-frame
  construction stays inside its sanctioned home — ADVISORY ONLY, an early warning that names the
  offending file at review time, with **nothing stronger standing behind it**: the tier-P append
  refusal that used to hold the property at frame level is retired, because the sanctioned emitter
  would have fail-stopped on its first OR write),
  `…ac9_injected_frames_carry_the_golden_on_disk_shape…` (the harness's synthetic frames bound to the
  checked-in golden bytes), `…ac10ii_b…` (legacy `Store`/`Remove`-only replay), `…ac11d…` (re-replay
  of an applied OR frame is a no-op on set AND gauge — the OR merge-idempotency residual).
  Mutation-proven: deleting the fold's `normalize_to_or_map` call reddens exactly the non-`OrMap`
  base-shape arms while the `OrMap`-only cases stay green; and dropping `apply_or_delta`'s tombstone
  dedup reddens exactly the re-fold arms.
  Limit of the `…ac9…` belt, recorded rather than left implicit: a construction hidden behind an
  `include!` of a non-`.rs` fragment, a macro, a build-script body or an aliased import the
  classifier does not model passes the scan **and nothing catches it afterwards**. That is why the
  belt is labelled advisory above, and why what actually binds at the byte level is named separately:
  the fold equivalence measured by the cases here, the store-boundary emission proof in
  `write_behind.rs`, and the checked-in golden bytes the emitter's own output is pinned to.
- **Violation consequence:** silent post-crash divergence of OR state — the class the oracle was
  built to kill.
- **Discovered by:** SPEC-346 design.
- **Status:** decided, **enforced** on both sides. The synthetic-frame caveat is RETIRED: the reader
  has now folded frames a REAL producer wrote — the store-boundary emission proof drives the live
  write-behind emitter into a real `WalWriter` and asserts the emitted variant on disk. The
  simulation recovery case runs the same churn through a crash under BOTH settings of the framing
  switch and requires the two recovered states to be semantic-set equal; it asserts framing only
  through a **byte-total divergence** between the two runs, because the belts forbid that module
  naming the variant. *(That divergence is load-bearing, not decoration: without it a write path
  that stopped delivering a witness would frame snapshots on both sides and the differential would
  pass while comparing a store against itself.)* The harness still synthesises frames,
  but by choice rather than necessity: synthesis is what lets a case place a delta on a fold base a
  real write reaches only when interrupted at the right instant. Landing the reader first remains the
  right order, for the reason that has not changed — an unfoldable delta frame on disk is a
  permanently lost mutation, not a self-healing one.

### TG-OR-004: The tombstone-bytes gauge tracks the REAL add and prune paths, test-isolatable

- **Scope:** `ProcessGauge`/scoped sink (`storage/tombstone_gauge.rs`, SPEC-351).
- **Statement:** `add_tombstone_bytes` fires on the real OR-remove path and `sub_tombstone_bytes`
  on the real epoch-prune path (mutation-proven both directions); tests bind task-local isolated
  gauges — no order-dependent global reads; negative controls never read a shared counter.
- **Maintaining code:** `record.rs` fns delegating through the scoped sink resolver, plus the two
  counter call sites in `crdt.rs`: `add_tombstone_bytes` inside the OR_REMOVE mutate closure's
  new-tombstone guard, and `sub_tombstone_bytes` in `crdt.rs::prune_epoch_tombstones`'s post-write
  `Ok(_)` arm behind `dropped`. Both deliberately sit OUTSIDE the extracted pure apply
  (`crdt.rs::apply_or_delta`, whose counter-freeness is itself asserted, behaviourally by
  `crdt.rs::or_apply_moves_no_tombstone_bytes_on_any_arm` and structurally by
  `::or_apply_body_names_no_tombstone_byte_counter`). That purity rule is NOT
  permission to move them into it: the decrement in particular must fire only after the durable write
  succeeds, because the gauge tracks bytes actually resident, not bytes removed from an in-memory copy.
  Citations are kept line-number-free on purpose — the SPEC-349 extraction relocated the surrounding
  code, and a line citation would have drifted silently.
- **Enforcing test:** SPEC-351 suite (9 tests) — real-prune-path coverage at
  `crdt.rs::prune_epoch_tombstones`, which the long-lived prune task runs as its pass (an OR write
  only requests a wake), post-write `Ok(_)` arm (mutating that `sub_tombstone_bytes` call →
  deterministic RED), per-binding tripwire, private-counter foreign traffic control.
- **Violation consequence:** the SPEC-345 tombstone hard gate reads a fiction; the 72h soak's
  primary instrument lies.
- **Discovered by:** SPEC-351 audit C1 (the gauge was previously asserted only against a test
  mirror — the discovered hole this entry closes).
- **Status:** decided, **enforced**.

### TG-OR-005: Resident OR tombstone bytes stay under a derived ceiling (bounded) and level-stable (steady) under sustained churn at the production epoch width

- **Scope:** the epoch-scoped tombstone prune under sustained OR churn at the **production** epoch
  width (`TOPGUN_EPOCH_WIDTH` unset → 1000), with a tracked client confirming, in **non-crash runs**.
  The crash-run bound (a recovery re-stamps every live tombstone into one epoch) is not derived and is
  deferred under TODO-634.
- **Statement:** two clauses, each carrying one claim.
  **Bounded (ceiling):** resident OR tombstone bytes (`topgun_ormap_tombstone_bytes`) never exceed
  `C = K × W × b_max` with `K = 2 + ⌈S_A / W⌉` — one open epoch, the epochs that exited since the
  previous prune pass (one under the pass-latency premise), and at most `⌈S_A / W⌉` epochs held by the
  write-behind durability fence. `S_A` is the soak harness's
  `max_count_in_window(remove_attempts, A, Δ)`: the most remove attempts in any counted window, which
  spans `A` plus one sample gap plus the latency allowance `Δ`. The bound rests on two premises, both
  RECORDED and neither enforced: **A** — no write-behind sequence stays pending longer than
  `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS`, which the server alarms on but does not enforce (enforcement
  tracked in TODO-689) — and **Δ (P-Δ)** — attempt-to-stamp latency ≤ the 5 s allowance
  (constant), recorded against the measured ack max (acked removes: the recorded remove-latency p99
  and max), with the un-acked count bounding what that evidence cannot see. The K in this statement is the K the cited cell measured and
  gated on; it is never re-derived from an exact-`A` count.
  **Steady (level):** the last-quarter mean lies within `0.10 × max(last-half mean, W × b_max)` of the
  last-half mean.
- **Status:** `evidenced by one pre-registered 4 h cell (spec370-plateau4h; not CI-enforced; bounded
  clause under premise A)`.
- **Evidence:** the 4 h cell `spec370-plateau4h`, data commit `3ac48cd8`
  (`packages/server-rust/benches/soak_harness/evidence/spec370-plateau4h.predicates.txt`; reading in
  `spec370-manifest.md` §3). Decision row T: `PLATEAU=TRUE`, `READING=BOUNDED_STEADY`,
  `REPLICATE=NOT_NEEDED`. The cell's lines, verbatim:
  ```
  PK-derived stamps_window_max=2774 epoch_width=1000 held_max_derived=3 ceiling_epochs=5
  PK-crosscheck csv_stamps_per_row_max=3342 stamps_window_max=2774 <=FALSE recorded_not_gated
  PK-premise=TRUE held_max_observed=2 durable_watermark_lag_max=3 held_max_derived=3
  PC-max n=241 skipped_empty=0 run_max=50002 run_max_elapsed=11820 last_half_max=50002 ceiling=115000 run_max_over_ceiling=0.435 <=ceiling=TRUE
  PL half_start=120 quarter_start=180 last_half_mean=34691.413 last_quarter_mean=34729.623 deviation_bytes=38.210 tolerance_bytes=3469.141 deviation_pct=0.110 direction=up <=tolerance=TRUE
  PH=TRUE
  PA-wal_watermark_alarm_lines=0 recorded_not_gated
  ```
- **What is measured, and what is NOT claimed.** In a committed 4 h run at width 1000
  (`spec355-w1000.*`, 2878 samples, 458 epochs, a tracked client ACKing throughout so the
  low-water mark advanced), resident bytes grew **0 → 646,306 B** and the series **ended at its
  maximum**. Eight equal windows fitted independently gave slopes of **113,657 – 244,197 B/h** with
  r² 0.70–0.97 and **no decay** (W8 is 37 % above W1), against a 512 B/h gate bound. **No plateau
  was found within the measured horizon.** This is deliberately *not* stated as a proof of
  unboundedness: a 4 h observation cannot exclude a bound at some higher level or longer horizon,
  and wording it as a refutation would invite a future reader to stop looking for the real bound.
- **The prune is NOT dead — it falls behind.** Across the same committed series the gauge
  decrements on 33.5 % of steps, freeing 299,349 B over the run, so the prune fires. Its **reclaim
  fraction** (freed ÷ added) degrades with both width and elapsed time. **HEAD runs only:**
  **≈95–98 %** at widths 100/300, **≈80–92 %** at width 1000 over 1800 s, and **33.1 %** at width
  1000 over 4 h. The pre-family arm is *worse* at every width and is not inside those ranges
  (86.7 % at width 100; 66.4 % / 80.9 % at width 1000), so the ranges are a HEAD-only summary, not
  a binary-agnostic one. Two caveats travel with the numbers: the CSV samples at 60 s, so the gross
  added/freed columns are **lower bounds** (the *net* column is exact and the cadence is identical
  across every run), and the temporal limb compares `n = 2` at 1800 s against `n = 1` at 4 h. That
  degradation, not a dead prune, is what this entry is open on.
- **Not a regression.** A pinned pre-SPEC-349-family server (`181723d0`) is *worse* at this width,
  n = 2 vs 2 with disjoint ranges (level 55,787 vs 37,670, t = 11.88; slope 191,961 vs 65,049,
  t = 4.56). **That claim stops at the pin's own date (2026-07-27) and is not a statement about the
  whole history:** the 2026-07-13 → 2026-07-27 interval — which contains two OR-path merges — is
  **un-probed**, and is owned by `TODO-634` as a diagnostic-on-demand task with a runnable protocol.
  Do not read "not a regression" as "characterised all the way back".
- **Maintaining code:** `crdt.rs::prune_epoch_tombstones` and the epoch frontier it consumes
  (`tombstone_frontier_impl.rs`). Citations are kept line-number-free on purpose, per `TG-OR-004`.
- **Enforcing test:** `NAKED — evidenced by a pre-registered 4 h soak cell
  (packages/server-rust/benches/soak_harness/evidence/spec370-manifest.md §3); no CI-run test proves the
  bound; the harness gate that asserts it runs only in soak runs; premise A is not enforced (TODO-634, TODO-689)`.
- **Violation consequence:** the soak harness reds on the SPEC-370 tombstone clauses — the level
  ceiling (run max > `C = K × W × b_max`) or level stability (last-quarter mean outside the band
  around the last-half mean) — and on a long-lived node resident tombstone bytes would exceed the
  derived ceiling `C`.
- **Distinct from `TG-OR-004`, which is gauge FIDELITY** (does the counter track the real add/prune
  paths), **not BOUNDEDNESS** (do the bytes stay bounded). A red tombstone gate is **not** evidence
  against `TG-OR-004`; **do not flip it.** `TG-OR-004` is `decided, enforced` and this measurement
  in fact *depends* on it holding — the numbers above are only meaningful because that gauge is
  known to track the real paths. The distinction is sited here, not only in a spec, because
  `TG-OR-004`'s own Violation-consequence text reads "the SPEC-345 tombstone hard gate reads a
  fiction", which is exactly the row a future reader confronting a red tombstone gate lands on and
  is tempted to flip.
- **Caveat on the gate that surfaced this.** The 512 B/h clause is a **last-half OLS slope**, i.e. a
  rate detector, and SPEC-355 measured it to be an unreliable one at this workload: two *identical*
  width-100 runs gave slopes 4.6× apart with a second instrument flipping sign, and the gate's
  verdict is non-monotonic in width (FAIL at 100, PASS at 300, FAIL at 1000). When a prune fix
  lands, the bound must be re-derived with a **level/ceiling** estimator rather than re-armed on the
  same slope statistic. The soak harness gate is now a derived level ceiling plus level stability;
  the 512 B/h slope is report-only. The harness's S_A measurement under-counted the server stamp
  proxy by up to 20 % in the evidencing cell; the error tightens the ceiling (TODO-690).
- **Discovered by:** SPEC-355 (R3.2's pre-registered 8-window plateau test), resolving TODO-630.
  Evidence: `packages/server-rust/benches/soak_harness/evidence/spec355-manifest.md`.

### TG-OR-006: The per-epoch prune record is exit-path exhaustive and gauge-neutral

- **Scope:** the per-pass / per-epoch prune record emitted around
  `crdt.rs::prune_epoch_tombstones` and the observer that renders it
  (`tombstone_frontier.rs` contract, `tombstone_frontier_impl.rs` implementation), armed by
  `TOPGUN_PRUNE_RECORD`.
- **Statement:** two claims about the *instrument*, not about the prune it measures.
  **(a) Exit-path exhaustive:** every tombstone ref the sweep considers leaves through exactly one
  counted exit —
  `considered == dropped + matched_nothing + absent + restored_read_error + restored_evicted +
  restored_write_error + restored_cancelled` — and that identity holds on **every** exit path,
  **including a pass dropped (cancelled) before it settles a ref**: the pass guard re-indexes each
  still-unsettled ref as its `Drop` runs and counts it as `restored_cancelled`, so a ref that
  quietly stops being accounted for can hide neither behind a falling reclaim fraction nor behind a
  cancellation. **(b) Gauge-neutral:** arming the record moves no tombstone bytes and
  reclaims no differently — the same workload run armed and disarmed yields identical isolated
  gauge deltas and an identical reclaim outcome, and the single `sub_tombstone_bytes` call stays in
  `prune_epoch_tombstones`'s post-write `Ok(_)` arm behind `dropped` while the recorder body names
  no tombstone-byte counter at all.
- **Maintaining code:** the exit enum and the record structs (`tombstone_frontier.rs`), the
  metrics-emitting and null observers plus the single arming read at `TombstoneFrontier::new`
  (`tombstone_frontier_impl.rs`), and the ledger inside `crdt.rs::prune_epoch_tombstones`.
  Citations are kept line-number-free on purpose, per `TG-OR-004`.
- **Enforcing test:** `prune_exit_ledger_sums_to_considered` (the seven-exit identity, each exit
  driven exactly once), `cancelled_prune_pass_restores_every_unsettled_ref` (the cancelled limb),
  `prune_guard_drop_is_panic_free_and_gauge_neutral` (guard `Drop` panic-free, naming no
  tombstone-byte counter) and `prune_record_armed_disarmed_gauge_neutral` (limb (b)), in `crdt.rs`.
  With the default store a key evicted between the prune's residency check and its write completes as a durable
  removal (`Dropped`); `restored_evicted` is reachable only for a store whose `update_in_place` cannot
  materialize the key, and is driven by
  `prune_restores_the_ref_when_the_store_cannot_materialize_the_evicted_key` (SPEC-374).
- **The pass-siting premise is enforced by the identity TOGETHER WITH its two count pins, not by the
  sum alone.** `passes == empty_drains + nonempty_drains` is pinned alongside `nonempty_drains == 1`
  and `empty_drains >= 1`, because a pass observation made *conditional on work* survives the bare
  sum as `1 == 0 + 1`; only moving the observation **into the loop body** breaks the sum (six passes
  for one drain). Do not read the sum identity as going RED on any mis-sited pass — it does not, and
  the two count pins are what close that hole.
- **Violation consequence:** the prune record becomes an instrument that either loses refs between
  its exits (so a degrading reclaim fraction has no attributable cause) or perturbs the very
  tombstone bytes `TG-OR-004` tracks — in which case every measurement taken with the record armed,
  including SPEC-356b's classification, is reading its own footprint.
- **Distinct from both neighbouring OR rows, and the three-way split is sited here on purpose:**
  `TG-OR-004` is gauge **FIDELITY** (does the counter track the real add/prune paths), `TG-OR-005`
  is **BOUNDEDNESS** (do the bytes stay bounded under sustained churn), and this row is
  **INSTRUMENT NEUTRALITY OF THE NEW RECORD** (does adding the record change what those two
  measure). A red `TG-OR-005` is evidence against neither of the other two.
- **Discovered by:** SPEC-356a (the instrument half of the TODO-634 prune-record family).
- **Status:** decided, **enforced**.

### TG-OR-007: A write on a durable key merges into the durable value, whatever the key's residency

- **Scope:** every production insert path of the record store — the in-place write seam
  (`DefaultRecordStore::update_in_place`) and `get`'s load path — and the removals that race them
  (`remove`, `evict_lru`, graceful-shutdown `hard_flush`); SYNC OR ingest
  (`SyncService::handle_ormap_push_diff`).
- **Statement:** on every production insert path, a mutation on a durable key merges into the key's
  durable value, and a value read from the data store is inserted into the engine only if no removal
  of that key intervened since the read began; residency (resident, evicted, post-restart) and
  concurrent reads never change the result. `evict`, `evict_all`, `evict_expired` and `clear` can
  drop dirty records and are outside this invariant; none has a production caller. A slot cell
  loaded from a pending write (`TG-WB-003` (c)) is re-adopted only through the same
  generation-checked insert, so a removal that intervened between the read and the insert refuses
  it exactly as it refuses a loaded value; a staged delete stops `load_slot` answering the cell.
- **Per path, for the pending key's cell C** (g = the vacancy generation read before `load_slot`;
  every insert of C is `SlotInit::Cell(C)`, generation-checked under the entry lock):
  (i) `update_in_place` materialize — an eviction after g ⇒ `Stale` before the mutation, the retry
  reloads the still-staged C; a reader's insert of C first ⇒ the writer mutates that same C in
  place; REMOVE is excluded by the per-key writer; a flush of C's older entry encodes C before or
  after the mutation (lead, never lag). (ii) `get` load (also the prune's non-resident `get`) — a
  REMOVE staged after the load either removes the inserted C in `engine.remove` or moves g ⇒
  `Stale`, answered but never cached; an eviction after g ⇒ `Stale`; a writer's insert first ⇒
  `Resident`, the writer's C. (iii) eviction (`evict_lru` → `remove_if`, `evict`, `evict_all`,
  `evict_expired`) — drops only the engine's `Arc` and bumps g; entry, staging and an in-flight
  flush keep C; a write stamped on C after `evict_lru`'s snapshot is read under entry → cell and
  keeps the key (`TG-EVI-001`). (iv) REMOVE — the staged delete replaces the queue entry and the
  staging slot, both dropping C, so C is never re-adopted; `engine.remove` drops the engine's `Arc`
  and bumps g; a reader that loaded C before the delete is removed or refused `Stale` as in (ii).
- **Maintaining code:** `DefaultRecordStore::update_in_place` (resident probe, then load and a
  generation-checked insert, retried on `Stale` a bounded number of times, then a retryable error);
  `get` inserting through `StorageEngine::put_if_absent_at`; the engine's striped vacancy generation,
  advanced by every removal under the key's lock, also for an absent key; `remove` staging the
  durable delete before it empties the engine; REMOVE holding the same per-key writer as the
  in-place OR writes (`crdt.rs`); `hard_flush` joining the flush loop before it drains (SPEC-374);
  for slot cells, `MapDataStore::load_slot` (the write-behind answers the staged cell,
  `Loaded::Cell`) in the materialize loop and in `get`, and the cell-aware inserts
  `StorageEngine::put_if_absent_at` / `update_in_place` taking `SlotInit::Cell` (SPEC-373b);
  SYNC OR ingest holding the shared per-key writer for every entry — across the gate check, the seam
  call (load, mutate, staging) and the OR_ADD broadcast — and merging through `update_in_place`
  (init an empty `OrMap`, the closure returns `changed: true`, no witness); that writer is the same
  `Arc` as `CrdtService`'s in the server binary and in the simulation (`sync.rs`). The in-memory lost
  update on a resident key (the thread-parallel window) is excluded by the in-place merge running
  under the engine's per-key entry lock.
- **Enforcing test:** `crdt.rs::or_add_on_non_resident_key_keeps_the_durable_entries`,
  `::or_remove_on_non_resident_key_keeps_the_other_durable_entries`,
  `::or_add_after_eviction_keeps_the_durable_entries`,
  `tests/non_resident_or_restart.rs::first_post_restart_or_add_keeps_every_earlier_value` (real
  process, kill -9); `default_record_store.rs::reader_does_not_cache_a_load_that_an_eviction_superseded`,
  `::reader_does_not_cache_a_load_that_a_remove_superseded`,
  `::reader_between_the_steps_of_a_remove_does_not_resurrect_it`;
  `crdt.rs::remove_during_a_materializing_or_add_is_not_undone`,
  `::or_add_between_the_steps_of_a_remove_does_not_resurrect_it`;
  `write_behind.rs::hard_flush_applies_a_later_remove_after_the_loops_in_flight_store`.
  On a staged cell (the same races, the reader or writer taking `Loaded::Cell`; each asserts the
  `load_slot` park was hit exactly once): `default_record_store.rs::reader_does_not_cache_a_load_that_an_eviction_superseded_on_a_staged_cell`,
  `::reader_does_not_cache_a_load_that_a_remove_superseded_on_a_staged_cell`,
  `::reader_between_the_steps_of_a_remove_does_not_resurrect_it_on_a_staged_cell`,
  `::evict_lru_keeps_a_record_written_after_its_snapshot_on_a_staged_cell`;
  `crdt.rs::remove_during_a_materializing_or_add_is_not_undone_on_a_staged_cell`,
  `::or_add_between_the_steps_of_a_remove_does_not_resurrect_it_on_a_staged_cell`. Re-adoption of
  the cell, AC-2b together with AC-2d:
  `default_record_store.rs::an_or_add_adopts_the_staged_cell_of_an_evicted_pending_key` (the cell
  taken before the eviction is the slot's cell after the OR_ADD — catches a copying adoption) and
  `::an_evicted_cell_is_re_adopted_while_its_flush_is_pending` (the engine holds the queued
  entry's cell after `op2` and the flush persists every op — catches a fresh cell beside the
  queued one, not a copying adoption, which AC-2b covers).
  SYNC OR ingest (`crdt.rs::tests::non_resident_writes::`): `push_during_a_remove_does_not_resurrect_it`,
  `::push_across_an_evicted_or_add_keeps_the_acked_add`,
  `::push_with_a_failed_load_keeps_the_durable_value`,
  `::push_between_the_steps_of_a_remove_does_not_resurrect_it`,
  `::push_and_or_add_stage_in_mutation_order`.
- **Violation consequence:** the first OR write on a key after a restart or an eviction replaces the
  key's durable value with a one-op slot (every earlier entry and tombstone lost, unrecoverable from
  the WAL once its segments are collected), or a removed value is resurrected by a stale read; a
  SYNC push resurrects a removed key or loses an acked OR op.
- **Discovered by:** TODO-700 (red tests), SPEC-374; extended to slot cells by SPEC-373b; SYNC OR
  ingest by SPEC-375.
- **Status:** decided, **enforced**.

### TG-MRK-001: The OR-Map Merkle leaf hash is set-canonical (order-independent)

- **Scope:** `merkle_leaf_hash` (`map_data_store.rs`), mirrored bit-identically by the TS client's
  `hashORMapLeaf` (`packages/core/src/ORMapMerkle.ts`) — the granularity is a cross-language
  protocol contract. The core-rust `ORMap::hash_entry` (`packages/core-rust/src/or_map.rs`) is a
  **partial** mirror: it computes the same leaf restricted to an empty tombstone set, because that
  type carries no per-key tombstone attribution. It is on no sync path; completing or removing it
  is tracked in TODO-726.
- **Statement:** two OR-Map states with the same tag/tombstone SETS hash identically regardless
  of insertion order (tags and tombstones sorted by Unicode code point before hashing; values,
  timestamps and TTL never contribute). A slot with no live tags and no tombstones has no leaf.
  For a given key, the leaf encoding is injective only because a stored tag is non-empty and
  contains neither `|` nor `#`; node-id construction and server ingest enforce it. A slot or a
  device replica that held such a tag before the rule is outside this note (tracker TODO-737).
- **Maintaining code:** the sort and the empty-slot early return in `merkle_leaf_hash`; the
  code-point sort in `hashORMapLeaf`, and the presence rule in `ORMapMerkleTree.update`.
- **Enforcing test:** Rust arm `or_leaf_hash_matches_oracle_and_ignores_input_order` and `empty_or_slot_yields_no_leaf` (`map_data_store.rs`); tag-admissibility arms `or_op_with_an_inadmissible_tag_is_refused_before_the_batch_applies` and `a_stored_inadmissible_tag_stays_removable` (`service/domain/crdt.rs`), `push_diff_with_an_inadmissible_tag_merges_nothing` and `both_or_ingest_paths_refuse_the_same_tags` (`service/domain/sync.rs`); TS arm `hashORMapLeaf is independent of tag and tombstone input order` (`packages/core/src/__tests__/merkle-vectors.test.ts`).
  The proptest shuffles the input order of `records` and of `tombstones` and asserts an equal hash,
  and pins the streamed hash to the joined-string formula it replaced. Both languages are pinned to
  one golden vector file (`packages/core-rust/tests/fixtures/merkle_vectors.json`) by
  `merkle_vectors_or_leaf_cases_match_canonical_leaf` and the TS vector suite, including a case
  whose code-point order differs from its UTF-16 code-unit order.
  Supporting arms for tag admissibility: `hlc_rejects_a_node_id_with_a_leaf_separator`
  (`packages/core-rust/src/hlc.rs`) for the Rust node-id rule; the simulation test
  `inadmissible_or_tag_is_refused_on_both_sides_of_a_partition` (`sim/cluster.rs`) for refusal
  under a network partition; in TS, `should reject node ID containing a pipe` and
  `should reject node ID containing a hash sign` (`packages/core/src/__tests__/HLC.test.ts`) and
  `should throw when nodeId contains a Merkle-leaf separator`
  (`packages/client/src/__tests__/TopGunClient.test.ts`).
- **Violation consequence:** false Merkle mismatches → sync storms, or false matches → silent
  divergence; breaks the SPEC-349 semantic-set recovery warrant.
- **Discovered by:** extraction pilot audit; load-bearing for SPEC-346/349 (the /xask
  Merkle-ordering caveat was refuted BY this sort — the sort itself deserves a test).
- **Status:** decided, **enforced** (Rust and TS arms).

### TG-MRK-002: Client and server Merkle roots are comparable, and equal OR roots imply the client holds the covered tombstones

- **Scope:** `RecordValue::OrMap` slots whose stored tags are all admissible (non-empty, no `|`,
  no `#`; `TG-MRK-001`), and LWW records, on the client-facing sync responses `SYNC_RESP_ROOT`,
  `SYNC_RESP_BUCKETS`, `ORMAP_SYNC_RESP_ROOT`, `ORMAP_SYNC_RESP_BUCKETS` and `ORMAP_SYNC_RESP_LEAF`,
  on the durable path and on the in-memory fallback. **Excluded:** legacy
  `RecordValue::OrTombstones` slots; `OrMap` slots that hold a tag stored before the admissibility
  rule that contains `|` or `#` (known gap, tracker TODO-737); routed-mode responses (paths with a
  3-digit partition prefix), which keep their per-partition shape; `ORMAP_DIFF_RESPONSE`, which is
  not session-served (tracker TODO-729). The LWW half claims comparable SHAPE only: a client holding
  a locally removed LWW key, or a timestamp newer than the server's re-stamp, does not reach equal
  roots (tracker TODO-736; no soundness effect, LWW responses carry no epoch).
- **Statement:** every root and bucket reported to a client is the root or bucket of ONE trie per
  (map, kind) over all keys of that kind — depth 3, path = lowercase 8-hex `fnv1a(key)`, node hash =
  `combine_hashes` of its children — the same construction the TS client builds; a server bucket map
  never carries a child whose hash is 0. Per-partition trees remain the write-path structure. The OR
  leaf is canonical (`TG-MRK-001`): no leaf iff the key has no live tag and no per-key tombstone,
  otherwise a hash over the key, its sorted live tags and its sorted per-key tombstones. A key is in
  the client OR tree iff it holds a live record or an attributed tombstone, and the tombstones the
  client attributes to a key are the server's per-key set plus its own pending removes, replaced on
  every leaf or diff entry and cleared down to the pending removes when the server reports the key
  absent. Equal OR roots therefore imply, for every key whose server slot is an `OrMap` with
  admissible tags, that the client holds every tombstone of the server snapshot the round descends
  by (up to FNV-32 collision), and the client confirms the conveyed covering epoch on equal roots.
  For a given key, the leaf encoding is injective because no stored tag is empty or contains `|` or
  `#`; ingest and node-id construction enforce it (see TG-MRK-001's note). The "for a given key" is
  required: a key containing `|` is ambiguous ACROSS keys — key `a` with tags {b, c} encodes like
  key `a|b` with tag {c} — which hides no tombstone, since the text after the single `#` is
  identical. **Known gap:** legacy `OrTombstones` slots have no leaf, so equal roots do not imply the
  client holds their tags; harmless because they are never epoch-stamped; tracker TODO-559.
  `ac2_untouched_legacy_ortombstones_never_prune_eligible` is the test that keeps the gap harmless.
- **Statement (epoch vs snapshot):** every covering epoch conveyed or marked delivered in an OR sync
  round — root, bucket and leaf, durable path and fallback — is ≤ the round epoch fixed at that
  round's `ORMapSyncInit`, and a root served from a session is ≤ that session's build epoch. A cached
  session older than the live epoch is rebuilt on `ORMapSyncInit`, so the delivered cursor advances
  at least once per epoch advance; a bucket request never rebuilds, so a walk keeps its snapshot. A
  continuation request on a connection with no open round conveys no epoch and delivers nothing.
  `ORMAP_PUSH_DIFF` conveys no covering epoch and marks nothing delivered. **Precondition:** OR sync
  rounds on one (map, connection) do not overlap: no second ORMapSyncInit for a map is sent while
  that map's walk on the same connection is in flight (first-party TS client: one init per map per
  connection, SyncEngine.startMerkleSync on !wasAuthenticated; soak tracker: serial). **Known
  limit:** a client that overlaps rounds can have an old walk's leaf replies confirm the newer
  round's epoch; tracker TODO-730.
- **Statement (drained walk):** a client confirms a map's covering epoch only when the walk that
  conveyed it has drained, and then the minimum its root and its leaves conveyed; a leaf response
  never omits a key whose read failed. The confirm sites pass only an epoch taken from a sync
  response; this depends on server events carrying no epoch. **Premises of the walk generation:**
  (1) the generation is raised on every sync init AND at the moment the connection is lost
  (`SyncEngine.handleConnectionLost` → `ORMapSyncHandler.onConnectionLost`), and every handler
  reads it at entry and again after each await that precedes a request, a count change, a push or
  a confirm, and after each await inside the loop that merges a response's entries: a response
  resumed under a changed generation applies nothing more (no record, tombstone or attribution),
  so a snapshot served before the loss cannot be written over a map the next connection has
  replaced by a full resync; it likewise sends, counts and confirms nothing on
  the next connection, and the sync start (`SyncEngine.startMerkleSync`) checks the engine's
  connection generation after each await, so it sends no sync init on a connection it did not
  start on; (2) a handler is entered synchronously when its frame arrives. A
  server-sent `BATCH` would break (2): `SyncEngine.handleBatch` awaits each inner message, so a
  later frame of the batch could enter after the connection changed. The Rust server sends no
  outbound `BATCH` today. A walk whose handling failed, whose
  root or one of whose leaves conveyed no epoch, or one of whose requests is never answered,
  confirms nothing (fail closed). **Reload limit:** a reloaded device claims epoch 0 (its confirmed
  epoch is not persisted), so under active protection it is answered with a full resync, not the
  in-sync branch; tracker TODO-739.
- **Maintaining code:** the flat session build in `durable_merkle.rs` (`build_session`, one trie per
  map and kind inside `MerkleSession`, `map_data_store.rs`); the on-demand flat fold of the
  in-memory fallback (`MerkleSyncManager::aggregate_lww_root_hash` / `aggregate_ormap_root_hash` /
  `aggregate_lww_buckets` / `aggregate_ormap_buckets`, `merkle_sync.rs`); in `sync.rs`, the session
  registry's per-(map, connection) round epoch (`round_epoch` / `set_round_epoch`) and per-session
  `epoch_at_build`, the stale check in `handle_ormap_sync_init`, the cap applied by
  `covering_epoch` to every continuation response of `handle_ormap_merkle_req_bucket`, and the leaf
  loops returning an error when a `store.get` fails; tag admissibility (`or_tag_refusal` /
  `admit_or_tags`, `crdt.rs`, and HLC node-id validation). Client: the per-map walk state and
  generation in `ORMapSyncHandler` (`packages/client/src/sync/ORMapSyncHandler.ts`: `enterWalk`,
  `countRequest`, `foldLeafEpoch`, `leaveWalk`), the per-key tombstone attribution it mirrors and
  persists, and `ORMapMerkleTree` / `hashORMapLeaf` (`packages/core/src`).
- **Enforcing test:** Rust, flat shape — `session_reports_flat_trie_root_and_buckets_for_vectors`
  (`storage/durable_merkle.rs`), `aggregate_roots_and_buckets_equal_flat_trie_for_vectors`
  (`storage/merkle_sync.rs`), both pinned to the golden vector file
  `packages/core-rust/tests/fixtures/merkle_vectors.json`, and the simulation test
  `sync_root_is_the_flat_trie_root_across_partition_and_heal` (`sim/cluster.rs`).
  Rust, epoch vs snapshot (`service/domain/sync.rs`) —
  `ormap_sync_init_epoch_never_postdates_cached_session`,
  `repeated_ormap_sync_init_advances_delivered_per_epoch_advance`,
  `ormap_push_diff_neither_conveys_nor_delivers_an_epoch`,
  `ormap_leaf_request_fails_when_a_key_cannot_be_read`, and the simulation test
  `partial_walk_never_confirms_a_tombstone_in_a_skipped_subtree` (`sim/tombstone_gc_proof.rs`).
  Rust, known gap — `ac2_untouched_legacy_ortombstones_never_prune_eligible`
  (`tombstone_frontier_impl.rs`). Rust, tag admissibility —
  `hlc_rejects_a_node_id_with_a_leaf_separator` (`packages/core-rust/src/hlc.rs`),
  `or_op_with_an_inadmissible_tag_is_refused_before_the_batch_applies` and
  `a_stored_inadmissible_tag_stays_removable` (`service/domain/crdt.rs`),
  `push_diff_with_an_inadmissible_tag_merges_nothing` (`service/domain/sync.rs`).
  Integration, real server (`tests/integration-rust/merkle-comparability.test.ts`) —
  `R3a: converged client reconnect takes the in-sync branch for LWW and OR`,
  `R3b: fresh device after a full walk reconnects in sync`,
  `R3c: attribution restored from its buckets yields the server's root`,
  `R11: the covering epoch is acknowledged only after the walk has drained`; and, under
  `Integration: OR-Map push scoping during a forced-divergence walk`,
  `the pushed entry for key B carries no tag that was removed from key A` and
  `the pushed entry for key B carries the tag that was removed from key B`.
  TS, drained walk (`packages/client/src/sync/__tests__/ORMapWalkDrain.test.ts`, describe
  `ORMapSyncHandler confirms a covering epoch only for a drained walk`) —
  `does not confirm after the first of two leaves, and confirms once after the leaf that carries the missing tombstone`,
  `confirms the lowest epoch its leaves conveyed, once`,
  `confirms nothing when one of its leaves conveyed no epoch`,
  `confirms nothing for a walk cut short by a new sync init, and a late leaf of it is applied without confirming`,
  `confirms nothing when the handling of one of its leaves failed`,
  `confirms nothing while one of its requests is never answered`,
  `still confirms at once on equal roots, once on a one-leaf walk, and once when a full-resync walk drains`,
  `a leaf of an earlier walk that finishes during the next walk neither confirms nor shortens that walk`,
  `confirms at the end of a bucket response when that is the last response of the walk to finish`,
  `confirms nothing when the handling of a bucket response failed`,
  `counts the epoch of the root that opened the walk: a lower one is what is confirmed, a missing one confirms nothing`,
  `a full-resync root still discarding local state when the connection is lost opens no walk on the next connection`,
  `a zero root still clearing stale attribution when the connection is lost opens no walk on the next connection`,
  `a response still being applied when the connection is lost sends and confirms nothing afterwards`,
  `a leaf still being applied when the connection is lost applies nothing more once the next connection has replaced the map`,
  `a diff response still being applied when the connection is lost applies nothing more`;
  and in `packages/client/src/__tests__/SyncEngine.test.ts`,
  `a sync start still enumerating held maps when the connection is lost sends no sync init afterwards`.
  TS, per-key tombstones (`packages/client/src/sync/__tests__/ORMapKeyTombstones.test.ts`) — under
  `pushORMapDiff`: `the entry for key B carries no tag that was removed from key A` and
  `the entry for key B carries the tag that was removed from key B`; under
  `equal roots require the same per-key tombstones`:
  `a client whose key lacks a tag the server leaf includes does not take the equal-roots confirm path`;
  every test of the describe `server absence clears the attribution down to the pending removes`;
  every test of the describe `a remove the server lost is handed back to it for the key it belongs to`,
  with its real-server counterpart `tests/integration-rust/lost-remove-heal.test.ts` (describe
  `Integration: a server that lost an acknowledged OR-Map remove`).
- **Violation consequence:** a mismatch between roots that should be equal ⇒ a full walk on every
  sync, O(keys × tombstones) rewrites. A false match ⇒ a covering epoch confirmed without its
  tombstones ⇒ a prune drops a tombstone that client never applied, and the removed value can be
  resurrected.
- **Discovered by:** SPEC-378 (client and server roots were never comparable once the non-linear
  `combine_hashes` replaced the summed aggregate); landed as SPEC-378a (canonical leaf and client
  attribution), SPEC-379 (tag admissibility) and SPEC-378b (flat shape, epoch vs snapshot, drained
  walk).
- **Status:** decided, **enforced** (Rust, TS and integration arms).

### TG-SYNC-001: At most one terminal verdict per op per exchange, exactly one when the exchange acks

- **Scope:** one client→server operation exchange on either transport — a WebSocket `OP_BATCH`
  and the `OP_REJECTED` / `OP_ACK` / `ERROR` frames it produces
  (`network/handlers/websocket.rs`), and the HTTP `/sync` request and its
  `{ ack, errors[] }` response body (`network/handlers/http_sync.rs`).
- **Statement:**
  (a) No op is ever both covered by an `OP_ACK` and named in an `OP_REJECTED`. (b) If the exchange
  ends in an `OP_ACK`, every id-bearing op of the batch has exactly one verdict: covered by the ack
  (`results` when present, else every id-bearing op of the answered batch) or named in an
  `OP_REJECTED`. (b') If the exchange ends with `OP_REJECTED` frames and no ack, every id-bearing
  op is named in exactly one of them. (c) If the exchange ends in an `ERROR` frame, only the
  already-emitted `OP_REJECTED` verdicts exist; no op may be marked synced client-side; every
  un-named op remains pending and is safe to re-send because a Permanent-failed sub-batch applied
  nothing (TG-SYNC-003) and an accepted op's re-apply is LWW-idempotent (OR-Map re-apply is the
  pre-existing hazard TODO-665 item 4 owns).
  (d) `OP_REJECTED{permanent: true}` is terminal across exchanges: the client never re-sends that op.
- **Stated residue — id-less ops are outside (b)/(b') by construction,** because the clauses say
  "id-bearing": an id-less op cannot be named; a non-SDK client whose id-less op is permanently
  refused keeps today's behaviour (anonymous `ERROR`, op re-sent). Tracked as **TODO-665 item 5**.
  The SDK always assigns ids (`SyncEngine.ts`), so every op this codebase emits is covered and the
  status below is honest rather than aspirational.
- **Maintaining code:** the per-operation verdict fold and its frame shaping
  (`network/handlers/websocket.rs`), the same fold mapped onto the HTTP response body
  (`network/handlers/http_sync.rs`), the exhaustive permanence classification the fold branches on
  (`service/operation.rs` — `disposition`, `wire_code`, `error_kind`), and the client-side
  retirement chain (`packages/client/src/SyncEngine.ts`). Citations are kept line-number-free on
  purpose, per `TG-OR-004`.
- **Clause (d) is enforced client-side, not by the Rust fns cited below,** and the gate does not
  check it: terminality *across* exchanges cannot be observed inside the single server-side exchange
  those fns drive. It is covered by the `packages/client` unit test
  `a later OP_ACK covering a refused op neither resurrects it nor reports it synced`
  (`packages/client/src/__tests__/SyncEngine.test.ts`), which delivers an `OP_ACK` covering an
  already-retired op id and asserts the op is still reported `'rejected'`. `check-invariants.sh`
  greps only `packages/server-rust/src` and `packages/server-rust/benches`, so a client citation
  cannot live in the field below.
- **Enforcing test:** `op_batch_every_op_gets_exactly_one_terminal_verdict` (clauses a and b),
  `op_batch_all_refused_emits_rejections_and_no_ack` (clause b') and
  `op_batch_transient_mid_fallback_emits_no_ack` (clause c), all in `websocket.rs`'s test module;
  the HTTP siblings are `http_sync_mixed_batch_names_the_refused_op_and_acks_the_rest`,
  `http_sync_all_refused_names_every_op_and_sends_no_ack` and
  `http_sync_transient_reports_context_less_error_and_no_ack` in `http_sync.rs`.
- **Violation consequence:** an op with **no** verdict wedges the client — it is neither retired nor
  acked, so it is re-sent on every flush and blocks the queue behind it, which is exactly the defect
  this row was written for. An op with **two** verdicts is worse: the client is told the same write
  was both taken and refused, and which one it believes is a race.
- **Discovered by:** the per-op rejection work resolving TODO-662; witnessed end-to-end by
  `tests/integration-rust/rejected-op-refusal.test.ts`.
- **Status:** decided, **enforced** (clause (d) client-side, as stated above).

### TG-SYNC-002: A refusal is never re-described as an acceptance

- **Scope:** the outbound frame sequence of one WebSocket operation exchange
  (`network/handlers/websocket.rs`), and the `{ ack, errors[] }` pair of one HTTP `/sync` response
  (`network/handlers/http_sync.rs`).
- **Statement:** for a given `opId`, the `OP_REJECTED` set and the `OP_ACK` acceptance coverage are
  disjoint, and the `OP_REJECTED` frame precedes the `OP_ACK` on the connection's outbound channel.
- **Maintaining code:** the fold that emits every refusal before shaping the ack, and the ack
  builder that carries `results` naming only accepted ops (`network/handlers/websocket.rs`);
  the HTTP handler emits the errors alongside an ack built from the same accepted set
  (`network/handlers/http_sync.rs`).
- **Enforcing test:** `op_rejected_precedes_ack_and_ack_excludes_refused_op`, with
  `transient_mid_fallback_yields_429_after_earlier_rejections` and
  `fallback_redispatches_in_original_order` covering the ordering under the fallback path — all in
  `websocket.rs`'s test module.
- **Violation consequence:** the client sees the ack first, marks the op synced, and the refusal
  that follows contradicts a state the application has already rendered. Ordering is what lets the
  client treat a refusal as terminal without re-checking every ack against it.
- **Discovered by:** the per-op rejection work resolving TODO-662.
- **Status:** decided, **enforced**.

### TG-SYNC-003: A sub-batch that fails with a Permanent disposition has applied nothing

- **Scope:** one dispatch of an operation sub-batch through the service pipeline, from the
  middleware stack down to `crdt.rs`'s `handle_op_batch`.
- **Statement:** when a sub-batch dispatch returns an error whose disposition is Permanent, no
  operation of that sub-batch was applied. This is what the per-op singleton re-dispatch relies on:
  if a Permanent failure could occur *after* some ops were applied, the fallback would apply them a
  second time.
- **Maintaining code:** the basis is layer-by-layer, in stack order. `MetricsLayer` only observes: it
  records the outcome and returns the inner result unchanged, so it raises no error of its own and
  applies nothing. `LoadShedLayer` fails with `Overloaded` before the inner future is built at all.
  `TimeoutLayer` may fail *after* the inner future has started — it drops that future mid-flight, so a
  sub-batch under it can already have applied operations — which is safe for this row because
  `Timeout` is `Transient`, and a Transient failure enters no singleton fallback at all.
  `AuthorizationLayer` builds the inner future before the RBAC decision — the reserved-map refusal
  returns `Forbidden` before any future exists — but fails before that future is ever *polled*: the
  Deny arm returns `Forbidden` and drops it un-awaited, and nothing has been
  applied because `Arc<CrdtService>::call` does all of its work inside the boxed future it returns —
  since `Forbidden` is Permanent, this row's guarantee on the Authorization path rests on exactly
  that property. The Router dispatches to the domain service, where the rest of this bullet takes
  over; `handle_op_batch` raises every Permanent variant in
  its validate loops *before* its apply loops; `apply_single_op` and `broadcast_event` return only
  `Internal`, which is Transient. A future Permanent error raised mid-apply breaks this row, and
  this row is what a reviewer trips over.
- **Enforcing test:** `permanent_subbatch_failure_applied_nothing_before_fallback` and its
  `permanent_subbatch_failure_applied_nothing_before_fallback_schema_case` companion — journal-count
  assertions over the two Permanent classes that reach the apply layer — plus
  `transient_subbatch_yields_429_error_and_no_fallback`, which pins that a Transient failure enters
  no fallback at all; all in `websocket.rs`'s test module.
- **Violation consequence:** double-apply on the fallback path. LWW re-apply is idempotent so the
  damage is invisible there, but an OR-Map add re-applied under a fresh tag is a duplicate element
  that no later remove will collect — silent divergence, discovered long after the write.
- **Discovered by:** the per-op rejection work resolving TODO-662; it is the precondition R5's
  singleton re-dispatch is only safe under.
- **Status:** decided, **enforced**.

### TG-SYNC-004: An OP_ACK is a verdict only on operations of a batch the receiving client sent

- **Scope:** every outbound frame of a client connection (`network/handlers/websocket.rs`), the
  `OperationResponse` type a domain service answers a dispatched message with
  (`service/operation.rs`), and the client's acknowledgement handler
  (`packages/client/src/SyncEngine.ts`).
- **Statement:**
  (a) *Server.* An `OP_ACK` is emitted only in answer to a message that carries operations — a
  `CLIENT_OP`, or an `OP_BATCH` with at least one op, top-level or inside a `BATCH` envelope — and
  its `lastId` and `results` derive only from op ids of that message. Nothing else is answered with
  one: not a push diff, not an unsubscribe, not an empty batch, which get no frame at all.
  (b) *Client.* An ack that carries `results` names the accepted ops itself and needs nothing more
  (TG-SYNC-001, TG-SYNC-002); an empty `results` names none, so it accepts nothing and is never
  read as an ack without `results`. An ack without `results` is applied only if its `lastId` is, by
  exact string equality, the last id of a batch the client sent — and, on a connection provider
  that declares the WebSocket transport, only if it also carries `achievedLevel`. A provider that
  declares HTTP, or declares no transport, is decided by the match alone, because a results-less
  HTTP acknowledgement never carries a level. An applied ack retires only ops of that batch — when
  several sent batches share a last id, only the ops present in every one of them — never a
  numeric prefix and never "everything pending". An ack that is not applied is not a verdict on
  the op log at all: no op is marked synced, no durable row is deleted, and the matched record is
  kept. (c) *Client.* A sent-batch record is given up only when its last op can no longer be sent
  again. A record evicted under the size bound keeps its key with an empty set, which acknowledges
  nothing. Either way a later, wider batch ending in the same op is intersected with the earlier
  record instead of being recorded afresh, so a late ack of the earlier batch cannot retire an op
  that batch never carried.
- **Stated residue — the id-less `"unknown"`.** When the answered message offers no op id to name,
  the ack still goes out, with the literal `"unknown"` in `lastId`: a client that sends id-less
  ops has no other acknowledgement to wait for, and the string names no operation. The SDK always
  assigns ids, so it never sends such a message, and under clause (b) it never applies that frame.
- **Stated residue — a provider that declares no transport.** It gets the match alone, so over a
  WebSocket to an already-released server that still sends the stray frame, a frame counter that
  happens to equal the last id of a batch in flight is applied. Every built-in provider that
  reaches a server declares its transport; only a third-party provider can be in this position.
- **Maintaining code:** the response-to-frame mapping, which builds no acknowledgement of its own,
  and the batch fold, which builds one from the ids of the batch it answers and sends no frame for
  an empty batch (`network/handlers/websocket.rs`); the `Ack` response variant, compiled out of
  every non-test build so that no handler can return one (`service/operation.rs`); the operation
  handlers, the only domain code that answers with an `OP_ACK` message, and only for a message
  that carried ops (`service/domain/crdt.rs`); the sent-batch registry — recorded when a batch is
  handed to the transport, or reported per frame by a provider that splits a flush across nodes —
  together with the ack handler's decision and the registry's prune and eviction
  (`packages/client/src/SyncEngine.ts`); and the pool's rule that only a node's current socket
  speaks for it, so a dropped socket cannot deliver a frame under the node's new connection
  (`packages/client/src/cluster/ConnectionPool.ts`). Citations are kept line-number-free on
  purpose, per `TG-OR-004`.
- **Clauses (b) and (c) are enforced client-side, not by the Rust fns cited below,** and the gate
  does not check them. Clause (b) is covered by four `packages/client` unit tests
  (`packages/client/src/__tests__/SyncEngine.test.ts`), each of which injects, byte for byte, a
  frame an already-released server really sends:
  `old-server frame: a results-less OP_ACK whose lastId answers no sent batch is not a verdict`,
  `old-server frame: a non-numeric lastId with no acceptance set is not a verdict`,
  `old-server frame: on HTTP, an ack without achievedLevel retires the answered batch` and
  `old-server frame: on WebSocket, a results-less OP_ACK without achievedLevel is not a verdict even when its lastId is the last id of a sent batch`.
  They are permanent: once the current server stops producing these frames, nothing else in the
  repository exercises them. Clause (c) is covered, in the same file, by
  `a refused op that is still in the op log keeps its sent-batch entry, so a late ack cannot retire an op its batch never carried`
  and
  `an ack that matches a refused op's entry does not free the key while the op can still be sent`,
  and the pool rule by
  `a frame from a socket the pool has dropped is not delivered under the node's new connection`
  (`packages/client/src/__tests__/ConnectionPool.test.ts`). `check-invariants.sh` greps only
  `packages/server-rust/src` and `packages/server-rust/benches`, so a client citation cannot live
  in the field below.
- **Enforcing test:** clause (a) — `empty_op_batch_emits_no_frame`,
  `nested_empty_op_batch_emits_no_frame` and `ack_response_variant_emits_no_frame` in
  `websocket.rs`'s test module; `op_batch_empty_answers_with_no_frame` in `crdt.rs`;
  `refused_push_diff_answers_with_no_frame` and
  `ormap_push_diff_answers_with_no_frame_and_stores_data` in `sync.rs`.
- **Violation consequence:** permanent client-side acked-loss. A client that applies an
  acknowledgement no batch of its own earned marks pending writes synced and deletes their durable
  op-log rows; the server never received them, nothing re-sends them, and they are missing after
  the next reload — with no error on either side.
- **Discovered by:** TODO-738 (a search unsubscribe or an OR-Map push diff, answered with an
  `OP_ACK` carrying a dispatcher counter, retired writes made offline); closed by SPEC-380;
  witnessed end-to-end by `tests/integration-rust/stray-op-ack.test.ts`.
- **Status:** decided, **enforced** (clauses (b) and (c) client-side, as stated above).

### TG-NAME-001: The redb table-name mapping is injective, class-disjoint and renames nothing

- **Scope:** `RedbDataStore`: the mapping from a `(map name, is_backup)` pair to a redb table name
  (`table_name_for`) and its inverse on primaries (`list_maps`), for every name the store accepts.
  Postgres is outside it — there the map name is a column value, not a table name.
- **Definitions:** `S` (storable) = names that are non-empty, do not end in `__backup` and contain
  no U+0000. `V` (identifier class) = members of `S` matching `^[a-zA-Z_][a-zA-Z0-9_]*$` — exactly
  the set the store accepted before this invariant. `W` = `S \ V`. `table(n, backup)`: for
  `n ∈ V` → `"map__"+n` / `"map__"+n+"__backup"`; for `n ∈ W` → `"mapr__"+n` / `"maprb__"+n`.
- **Statement:** for all `n ∈ S` and both values of `backup`:
  1. *Four ranges.* `P_old = "map__"·V`, `B_old = "map__"·V·"__backup"`, `P_new = "mapr__"·W`,
     `B_new = "maprb__"·W`.
  2. *Old vs new, primary vs backup (new).* Every string of `P_old ∪ B_old` has `_` at byte index
     3; every string of `P_new ∪ B_new` has `r` there. Every string of `P_new` has `_` at byte
     index 4; every string of `B_new` has `b` there. So `(P_old ∪ B_old)`, `P_new` and `B_new`
     are pairwise disjoint.
  3. *Primary vs backup (old).* `"map__"+a == "map__"+b+"__backup"` ⇒ `a == b+"__backup"`, which
     is not in `S`. So `P_old ∩ B_old = ∅`, and a table of `P_old` never ends in `__backup`.
  4. *Within a range* the mapping is a constant prefix (plus, for `B_old`, a constant suffix)
     around `n`, hence injective. With 2 and 3: `table` is injective on `S × {primary, backup}`.
  5. *`list_maps` is the total, unambiguous inverse on primaries.* The three prefixes are mutually
     exclusive (step 2: none is a prefix of another), so each catalog name takes at most one
     branch, in any order: `mapr__` → strip 6 bytes → the raw name; `maprb__` → skipped; `map__` →
     strip 5 bytes, skipped if the rest ends in `__backup`, else the name; anything else →
     ignored. Sharp cases:
     - `V`-name `r__x` → `map__r__x` (index 3 is `_`) — decodes to `r__x`, not to a `W`-name `x`;
     - `V`-name `rb__x` → `map__rb__x` — decodes to `rb__x`, is not a `W` backup;
     - `W`-names `map__a-b`, `mapr__a-b`, `maprb__a-b` → `mapr__map__a-b`, `mapr__mapr__a-b`,
       `mapr__maprb__a-b` — each decodes to itself (only the first 6 bytes are stripped; index 4
       is `_`, so none is read as a backup);
     - `W`-name `a-b__backup_data` → `mapr__a-b__backup_data` — the `W` branch has no suffix logic;
     - `V`-name `x__backup_data` → `map__x__backup_data` — does not end in `__backup`, listed;
     - a name ending in `__backup` (either class) is not in `S` and reaches no table.
  6. *Nothing is renamed.* For `n ∈ V` both table names are byte-identical to the ones the binary
     built from `07d009f0` uses. The three fixed internal maps (`_topgun_tombstone_cursors_v2`,
     `_topgun_device_credentials`, `__topgun_policies`) are in `V`. No table with byte `r` at
     index 3 can exist in a store written only by earlier binaries, whose only table constructor
     was `format!("map__…")`.
- **Maintaining code:** `packages/server-rust/src/storage/datastores/redb.rs` — `table_name_for`
  (refusal plus mapping in one call: it applies the shared rule `check_map_name` of
  `storage/map_data_store.rs`, refuses a name outside `S` and returns the table name, so no path
  to a table can skip the refusal) and `list_maps`. Citations are kept line-number-free on
  purpose, per `TG-OR-004`.
- **Enforcing test:** `table_names_keep_the_old_class_and_prefix_the_new_class` (steps 1-6 and the
  sharp cases of step 5), `store_refused_names_reach_no_table_and_backups_stay_unlisted` (step 3)
  and `map_names_outside_the_identifier_class_round_trip` (the round trip over the probe set) —
  all in `redb.rs`'s test module. The old-binary consequence of step 2 is proven outside the Rust
  tree, by the two-binary run `tests/integration-rust/map-name-rollback.test.ts`: the `07d009f0`
  binary starts and serves every `V` map on a store that holds `mapr__` / `maprb__` tables, after
  a clean stop and after `kill -9`. That run needs the older binary, so CI excludes it and it is
  run locally; the rollback note in `CLAUDE.md` states what was measured.
- **Violation consequence:** two maps, or a map and another map's backup partition, share one
  table: one silently overwrites the other, or a primary is read as a backup and dropped from
  `list_maps`, so the boot serves Merkle root 0 for durable data. Or an existing table is renamed
  and its data is unreachable after an upgrade. Or an earlier binary lists a table it cannot scan
  and refuses to start, which breaks rollback.
- **Discovered by:** the map-name durability investigation of TODO-751 (acked writes to
  `user-profiles` / `users/profiles` gone after a restart on redb); SPEC-381a.
- **Status:** decided, **enforced**.

### TG-NAME-002: A name outside the admissible set is refused before the WAL append and before the ack

- **Scope:** the server binary's operation pipeline, which always installs the authorization
  layer (`bin/topgun_server.rs`: `build_operation_pipeline(router, &config,
  evaluator_for_factory.clone())`, the evaluator always passed as `Some`), for operations of
  origin `Client`, `HttpClient` or `Anonymous` in the enumerated families. **Excluded:** a pipeline
  built without the layer (the bench load harness and the test callers that pass `None`); trusted
  origins; HTTP `/sync` queries.
- **Definitions:** `S` as in TG-NAME-001. `N` (admissible at ingress) = members of `S` of at most
  512 bytes of UTF-8; `N ⊆ S`.
- **Statement:** for every operation of origin `CallerOrigin::Client | HttpClient | Anonymous`
  dispatched through that pipeline and belonging to one of the **enumerated families** — `ClientOp`
  (LWW `PUT`/`REMOVE`, OR `OR_ADD`/`OR_REMOVE`), `OpBatch` (every op), `EntryProcess`,
  `EntryProcessBatch`, `ORMapPushDiff`, `QuerySubscribe`, `Search`, `SearchSubscribe`,
  `HybridSearch`, `HybridSearchSubscribe`, `SyncInit`, `MerkleReqBucket`, `ORMapSyncInit`,
  `ORMapMerkleReqBucket`, `ORMapDiffRequest` — if any carried map name `n ∉ N`, then: the operation
  returns `OperationError::InvalidMapName` (disposition `Permanent`), the inner service is never
  called, no WAL frame is appended and no `OP_ACK` is sent for the refused operation. For an
  `OpBatch` the unit is the **sub-batch** (the ops of the batch that share a partition), and the
  outcome depends on whether its ops carry ids (`ClientOp.id` is an `Option`):
  - *every op of the refused sub-batch carries an id* — the sub-batch is re-dispatched one op at a
    time; the refusal is attributed to the offending op(s), and each valid op of that sub-batch is
    applied exactly once (the first dispatch applied nothing — TG-SYNC-003);
  - *any op of the refused sub-batch has no id* — the sub-batch is NOT re-dispatched: it is refused
    whole, **none of its ops is applied**, valid ones included, and the batch is answered with one
    batch-level `ERROR` carrying code 400, which names no operation.

  Sub-batches of the same batch that hold no name outside `N` are dispatched as before.
- **Not covered, and why none is a durability hole:** three families carry a `map_name` field and
  are not checked — `VectorSearch`, `RegisterResolver` and `JournalSubscribe` (an `Option`). None
  of them makes a durable store write under that name (read / registration / subscription paths),
  so a refused-shape name there cannot become an acked-then-lost write. Their siblings in the
  authorization bypass group (`JournalRead`, `UnregisterResolver`, `ListResolvers`) are in the
  same position. `TopicPublish` and `CounterSync` carry a topic / counter name, not a map name.
- **Stated boundaries:** (a) Trusted origins (`Forwarded`, `Backup`, `Wan`, `System`) early-return
  in `AuthorizationService::call`; for them the store's own check (TG-NAME-001, `table_name_for`)
  is the last line. (b) HTTP `/sync` **queries** do not pass through the pipeline; they read
  resident records only and write nothing. HTTP `/sync` **operations** do pass through it, by the
  same per-op fold as an `OpBatch`, and are covered. (c) The invariant states what the server
  does, not what the client is told: on the single-message WebSocket path a refusal produces no
  frame. (d) A pipeline built **without** the authorization layer is outside the invariant:
  `build_operation_pipeline` installs `AuthorizationLayer` only when it is given an evaluator. The
  callers that pass `None` are the bench load harness and tests; no shipped path builds such a
  pipeline.
- **Maintaining code:** `service/middleware/authorization.rs` ingress check;
  `network/handlers/websocket.rs` `is_redispatchable`; the layer is installed by
  `service/middleware/pipeline.rs` `build_operation_pipeline` when it is given an evaluator, which
  the server binary always does. Citations are kept line-number-free on purpose, per `TG-OR-004`.
- **Enforcing test:** `a_name_outside_the_admissible_set_never_reaches_the_inner_service` (every
  checked family × every refused name, under both policy-store states) and
  `a_refused_op_in_any_batch_position_refuses_the_whole_dispatch` (an offending op in a non-first
  position), both in `authorization.rs`'s test module. The two `OpBatch` outcomes (with ids / with
  an id-less op) and the per-path wire behaviour are proven at the wire by
  `tests/integration-rust/map-name-refusal.test.ts`.
- **Violation consequence:** a write the store cannot hold is acknowledged and then lost at flush
  — acked-then-not-durable — and its WAL frame fails replay on every boot and pins that
  partition's WAL GC.
- **Discovered by:** the same investigation as TG-NAME-001: nothing checked a map name before the
  ack; SPEC-381a.
- **Status:** decided, **enforced**. Stated limit, not a gap in the invariant: on the
  single-message paths the refusal sends no frame (tracker TODO-744).

### TG-KEY-001: Every write of a record holds that record's per-key writer

- **Scope:** every production call of `RecordStore::put`, `update_in_place` and `remove`: the
  client write paths (LWW PUT and REMOVE, `OR_ADD`, `OR_REMOVE`), the tombstone prune pass,
  `ORMapPushDiff` and the embedding write-back.
- **Statement:** every such call runs while its task holds the per-key writer of exactly that
  `(map, key)`, taken from the one `KeyWriterRegistry` of the process. The writer of a key is the
  mutex of the key's stripe in a table of fixed size (`KEY_WRITER_STRIPES`); one key always maps
  to one stripe, two keys may share one, and a task holds at most one writer at a time. For a
  client operation the writer is taken before the server stamp is minted and released after the
  write-through has returned and the operation's journal entry and broadcasts are issued. For
  stamps minted under the writer, stamp order = engine order = durable-queue order for one key.
  That equality does not hold across an embedding write-back, and nothing here says what a
  restart recovers on a map with automatic embeddings — see "Not claimed".
- **How it is enforced:** by tests and by sweep predicates, not by a type. Until `TG-KEY-002`
  exists, "every production call" rests on a sweep of the call sites: the number of acquire
  sites and their position relative to the stamp, the store call and the fan-out, re-run when a
  writer is added. "One registry" is a wiring predicate on the server binary — the registry is
  constructed once and handed to every writer — not a property of a type: a service constructed
  without the shared registry gets a private one.
- **Not claimed:**
  - **A timestamp comparison.** An operation whose stamp is not minted under the writer (a
    stamp the caller supplies and the server applies as given) is ordered by arrival: a lower
    stamp that arrives later still replaces a higher one.
  - **That stamp equality identifies a version for stamps the server did not mint.** The
    embedding write-back attaches a vector only if the record still carries the stamp its text
    was read under. Equal stamps mean "the same version" only for server-minted stamps; two
    writes that carry the same caller-supplied stamp are not told apart.
  - **The embedding write-back's stamp.** It is wall-clock time under a synthetic node id, not
    from the server clock. For a key of an embedding-enabled map "stamp order = engine order"
    therefore does not hold across a write-back.
  - **The restart property on embedding-enabled maps.** After an unclean stop, replay can drop
    an acknowledged client write that followed an embedding write-back, because its stamp can be
    lower than the write-back's. Known by reading, not shown by a test. Tracker TODO-778.
  - **The writers that call `MapDataStore` directly** (frontier cursors, device credentials,
    policies). They do not go through the record store and this row says nothing about them.
  - **How long a writer waits.** `acquire` has no bound of its own; what a holder that never
    returns costs is stated in the module doc of `key_writer.rs`.
- **Maintaining code:** `service/domain/key_writer.rs` (`KeyWriterRegistry`, `acquire`);
  `service/domain/crdt.rs` — `apply_op_under_writer`, the one caller of `apply_single_op`, and the
  prune pass; `service/domain/sync.rs` — `handle_ormap_push_diff`;
  `service/domain/embedding/hook.rs` — `write_back_one_embedding`; `bin/topgun_server.rs`
  constructs the one registry and passes it to each of them.
- **Enforcing test:** `crdt.rs::two_routes_writing_one_key_leave_engine_and_store_on_the_later_stamp`
  — two writes of one key arriving by two routes, the first parked inside the store: the second
  waits for the writer, and memory and the durable store both end on the later stamp. Runs in CI
  (`cargo test`). The table below lists the others and what each one covers.
- **Violation consequence:** two writers of one record that are not serialised can each pass
  the store's own checks and interleave. Memory then serves one value while the durable store
  holds the other, and after a restart the server returns the OLDER of the two although it
  acknowledged and served the newer one; with a refused flush in between, write-behind accounting
  resolves the newer write's log frame although its value was never stored (`TG-WB-005`). An
  embedding write-back that is not serialised with client writes overwrites a client write that
  landed between its read and its put, or attaches a vector computed from the old text to the
  new value. Holding the writer removes these interleavings; it does not make a restart recover
  the last acknowledged write on a map with automatic embeddings (see "Not claimed").
- **Discovered by:** the SPEC-383 implementation review (two concurrent writers of one key under
  a refused flush) and the SPEC-384 call-site sweep (the LWW PUT path and the embedding
  write-back took no writer); fixed and catalogued by SPEC-384a1.
- **Status:** decided, **enforced** — by the tests below and by sweep predicates; not by a type.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `two_routes_writing_one_key_leave_engine_and_store_on_the_later_stamp` | two routes, the first parked inside the record store: memory and the durable store agree on the later stamp | CI (`cargo test`) |
| `two_routes_writing_one_key_leave_store_staging_and_restart_on_the_later_stamp` | the same with the first writer parked inside the write-behind store: the store, staging and a restart all give the later stamp | CI (`cargo test`) |
| `a_lower_stamped_put_never_lands_after_a_higher_stamped_one` | the stamp is minted under the writer: the journal order of two puts of one key is their stamp order | CI (`cargo test`) |
| `two_routes_and_a_refused_flush_recover_the_later_stamp_after_a_crash` | two routes plus a refused flush: after a crash and recovery the recovered value is the later-stamped one and equals what the live server served | CI (`cargo test`) |
| `an_older_value_arriving_behind_a_refused_newer_one_does_not_win_after_a_crash` | a write arriving while the newer one sits in a refused flush: the same assertion after a crash and recovery | CI (`cargo test`) |
| `an_lww_put_waits_for_a_parked_or_push_of_the_same_key` | one registry for LWW and OR writers: an LWW put waits for an `ORMapPushDiff` of the same key | CI (`cargo test`) |
| `a_client_write_during_an_embedding_write_back_is_not_overwritten` | the write-back's read-modify-write holds the writer: a client write during it is not overwritten | CI (`cargo test`) |
| `a_client_write_during_a_write_back_is_still_embedded_and_reaches_the_store` | a client write that waited behind a write-back still enqueues its own embedding event and reaches the store | CI (`cargo test`) |
| `a_stale_embedding_is_never_attached_to_a_newer_value` | the stamp check under the writer: a vector computed from an older text is not attached to a newer value | CI (`cargo test`) |
| `concurrent_same_key_puts_wait_on_the_shared_writer_and_converge_across_a_partition`, `same_key_puts_on_one_node_converge_to_a_higher_stamp_from_across_the_partition` | on a simulated node: two puts of one key wait on the shared writer; the nodes differ while partitioned and converge after it heals, whichever node holds the highest stamp | `pnpm test:sim` (feature `simulation`) |
| `two_different_keys_on_one_stripe_both_complete` | two keys on one stripe exclude each other, the waiter is served on release, and alternating writers all complete | CI (`cargo test`) |
| `registry_holds_no_per_key_state_after_100_000_distinct_keys` | nothing is kept per key written: no stripe is still referenced or locked after 100 000 distinct keys | CI (`cargo test`) |
| `stripe_table_footprint_is_within_the_stated_constant` | the table's size, from the sizes of its parts, is within `KEY_WRITER_FOOTPRINT_BOUND_BYTES` | CI (`cargo test`) |
| `a_second_acquire_by_the_holding_task_on_the_same_stripe_stays_pending` | WHY a task holds at most one writer: a second acquire on the holder's own stripe never returns | CI (`cargo test`) |
| `count_alloc_acquire_is_bounded_on_first_use_and_zero_on_repeat` | a stripe's first use allocates at most once and within the bound; a second pass over the same keys allocates nothing. Measured on macOS arm64 only | local only (`--features count-alloc`, `--ignored`); CI never enables the feature |
