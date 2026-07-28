# TopGun Invariants Catalog

Every durability/correctness invariant the system relies on, each mapped to the code that
maintains it and the test that enforces it. **An invariant without an enforcing test is marked
`NAKED` with a tracking TODO — visibly, on purpose.** CI (`scripts/check-invariants.sh`) verifies
that every cited enforcing test still exists and that no new entry lands without either a test or
an explicit `NAKED` marker; the gate is "the NAKED count never grows silently", not "zero NAKED".

Conventions: IDs are `TG-<DOMAIN>-<NNN>` (domains: WAL, WB write-behind, OR, LWW, MRK merkle,
EVI eviction, SYNC). Cite the ID verbatim in code comments and test names. Statuses:
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
- **Windowing residual (tracked, NOT closed by a gate):** the only genuine out-of-order hazard — a
  frameless `flush_key` durable write racing an un-resolved older Remove — is a WATERMARK/FRAMING
  concern, not a Remove-idempotency one, so it is routed to TODO-612 (SPEC-349/windowing), not
  papered over with an unsound Remove-replay timestamp gate.
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

### TG-EVI-001: Never-evict-dirty — an unflushed write is never evicted from the resident cache

- **Scope:** `evict_lru` in the record store.
- **Statement:** a record whose latest write has not reached the durable backend is not evictable,
  regardless of memory pressure.
- **Maintaining code:** `storage/impls/default_record_store.rs` dirty-skip.
- **Enforcing test:** `default_record_store.rs::evict_lru_skips_all_dirty_records` +
  `::evict_lru_skips_dirty_in_mixed_snapshot` + assertion in `eviction_cost_test.rs`.
- **Violation consequence:** eviction under pressure silently drops acked writes.
- **Discovered by:** eviction design (pre-catalog).
- **Status:** decided, **enforced**.

### TG-OR-001: `update_in_place`'s mutate closure runs at most once per call

- **Scope:** `RecordStore::update_in_place` seam (SPEC-347).
- **Statement:** one call invokes `mutate` at most once (doc-contract, SPEC-347); gauge side
  effects inside the closure must not double-count.
- **Maintaining code:** doc-contract + DashMap shard-lock path.
- **Enforcing test:** the literal call-counter assertion now exists —
  `or_inplace_mutate_proptest.rs::update_in_place_invokes_the_mutate_closure_exactly_once_per_call`
  counts invocations per call (`AtomicUsize`) on the insert, occupied and failed-write-through
  paths, and `::update_in_place_admits_the_take_once_shape_the_or_add_path_uses` pins the OR_ADD
  `Option::take` shape; both are mutation-proven RED against a second `mutate` call in either
  `engines/hashmap.rs` arm.
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
  no observer may silently depend on a pre-image.
- **Maintaining code:** SPEC-347 doc-contracts.
- **Enforcing test:** shape-only — the differential proptest matches notification COUNTS across
  legacy/in-place paths; content assertion on `old_value` is `NAKED (TODO-602)`.
- **Violation consequence:** a future observer reads `old_value`, silently gets wrong data.
- **Discovered by:** extraction pilot audit.
- **Status:** decided (scoped); enforcement shape-only.

### TG-OR-003: OR delta-fold recovery is semantic-set-equivalent to the snapshot path

- **Scope:** `OrDelta`/`OrDeltaFold`, wired on the RECOVERY READ side: `WalRecovery::replay_entry`'s
  `WalOp::OrDelta` arm and `impl OrDeltaFold for WalRecovery`. No emitter exists yet — every frame the
  enforcing test folds is synthesised at the codec level and injected through the harness's observed
  append seam.
- **Statement:** folding any op sequence through the delta path and the full-snapshot path yields
  equal `or_map_semantic_view` (live set + tombstones + pruned), with the durable store as fold
  base and snapshot frames as in-order absolute-set inputs.
- **Maintaining code:** types + oracle landed (SPEC-346); fold delegates to the live apply path
  (single-algebra rule, SPEC-349 R-mandate). That path now has a named anchor: `crdt.rs::apply_or_delta`
  — the ONE extracted pure apply of the add-wins / remove-wins / prune algebra, which the live OR_ADD,
  OR_REMOVE and epoch-prune call sites all route through (SPEC-349a). The delta fold must delegate to
  that symbol rather than re-implement the algebra; a second hand-written copy is what this invariant
  forbids, and the symbol is what SPEC-349b's delegation is checked against.
- **Enforcing test:** the `tg_or_003_*` case family in
  `packages/server-rust/src/storage/datastores/wal_harness/cases.rs` — a case family on the
  cross-incarnation harness, driven through its existing `Driver` and reference model, NOT a fork.
  13 cases: `tg_or_003_ac1_recovery_equivalence_over_every_fold_base_shape` (all three R1.3 fold-base
  shapes — durable `OrMap`, absent, legacy `OrTombstones` — plus the tombstone-bytes gauge via the real
  `storage::record::reconcile_tombstone_bytes` boot walk), `…ac3a…` (snapshot above the watermark with
  an empty store), `…ac3b…` (snapshot-only legacy window), `…ac3c…` (absolute-set: a snapshot
  tombstoning a live durable tag), `…ac3d…` (cross-kind/legacy base under a delta, post-state pinned
  literally), `…ac4…` (`Prune` as pure tombstone-set subtraction), `…ac5…` (single-algebra
  behavioural: remove-wins suppression of a re-added tombstoned tag), `…ac16…` (the injection rides the
  observed append seam only), `…ac2…` (stranded base + re-fold idempotency across `mark_applied` +
  segment GC), `…ac7b…` (the non-subsuming survivor's carry-forward route), `…ac9…` (no production
  construction site), `…ac10ii_b…` (legacy `Store`/`Remove`-only replay), `…ac11d…` (re-replay of an
  applied OR frame is a no-op on set AND gauge — the OR merge-idempotency residual). Mutation-proven:
  deleting the fold's `normalize_to_or_map` call reddens exactly the non-`OrMap` base-shape arms while
  the `OrMap`-only cases stay green; dropping `apply_or_delta`'s tombstone dedup reddens exactly the
  re-fold arms.
- **Violation consequence:** silent post-crash divergence of OR state — the class the oracle was
  built to kill.
- **Discovered by:** SPEC-346 design.
- **Status:** decided, **enforced** (reader side). The emitter lands separately; until it does, the
  invariant is proven against synthetic frames, which is the point of landing the reader first — an
  unfoldable delta frame on disk is a permanently lost mutation, not a self-healing one.

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
  `crdt.rs::prune_epoch_tombstones`, post-write `Ok(_)` arm (mutating that `sub_tombstone_bytes` call
  → deterministic RED), per-binding tripwire, private-counter foreign traffic control.
- **Violation consequence:** the SPEC-345 tombstone hard gate reads a fiction; the 72h soak's
  primary instrument lies.
- **Discovered by:** SPEC-351 audit C1 (the gauge was previously asserted only against a test
  mirror — the discovered hole this entry closes).
- **Status:** decided, **enforced**.

### TG-MRK-001: The OR-Map Merkle leaf hash is set-canonical (order-independent)

- **Scope:** `merkle_leaf_hash` (`map_data_store.rs`), mirrored by the TS client
  (`packages/core/src/ORMapMerkleTree.ts`) — the granularity is a cross-language protocol
  contract.
- **Statement:** two OR-Map states with the same tag/tombstone SETS hash identically regardless
  of insertion order (tags and tombstones sorted before hashing).
- **Maintaining code:** the sort in `merkle_leaf_hash`.
- **Enforcing test:** `NAKED for the order-independence claim (TODO-602)` — adjacent coverage
  only (buffered-vs-flushed fixed-sequence equality; LWW-arm hash format).
- **Violation consequence:** false Merkle mismatches → sync storms, or false matches → silent
  divergence; breaks the SPEC-349 semantic-set recovery warrant.
- **Discovered by:** extraction pilot audit; load-bearing for SPEC-346/349 (the /xask
  Merkle-ordering caveat was refuted BY this sort — the sort itself deserves a test).
- **Status:** decided (code sorts); enforcement NAKED.
