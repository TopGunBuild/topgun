# TopGun Invariants Catalog

Every durability/correctness invariant the system relies on, each mapped to the code that
maintains it and the test that enforces it. **An invariant without an enforcing test is marked
`NAKED` with a tracking TODO — visibly, on purpose.** CI (`scripts/check-invariants.sh`) verifies
that every cited enforcing test still exists and that no new entry lands without either a test or
an explicit `NAKED` marker; the gate is "the NAKED count never grows silently", not "zero NAKED".

Conventions: IDs are `TG-<DOMAIN>-<NNN>` (domains: WAL, WB write-behind, OR, LWW, MRK merkle,
EVI eviction, SYNC, NAME map names, KEY per-key writer, DISP dispatch, ENG byte engine). Cite the ID verbatim in code comments and test names. Statuses:
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
- **Enforcing test:** sidecar mode — `packages/server-rust/src/storage/crash_safety_proptest.rs`
  (the file's stated oracle is exactly this invariant; real store + WAL, SIGKILL modeled by store
  drop). Store mode — `store_mode_tests.rs::a12_under_batched_a_wal_cut_to_its_last_fsync_never_reuses_a_sequence`,
  `a18_a_kill_inside_or_right_after_a_commit_loses_no_acked_write` (a child process aborted
  inside and right after a commit).
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

### TG-WAL-003: The applied watermark a WAL collection reads is durable before any segment it licenses is unlinked

- **Scope:** segment collection ordering, all policies, in both WAL modes — *store mode*
  (`WalWriter::open_over_store`, what the server opens) and *sidecar mode* (`WalWriter::new`,
  tests, simulation and benches).
- **Statement:** the applied watermark a WAL collection reads is durable before any segment it
  licenses is unlinked; a crash between them must not lose or corrupt data (collection is
  resumable, replay is filtered by the watermark).
  - *Store mode:* the watermark is the engine row of the WAL partition, committed `sync: true`
    in the one engine batch of the records it covers, and never lower than the row it replaces;
    the WAL's in-memory copy advances only after that commit returned `Ok`. A collection tick
    (every 5 s) limits collection by a fresh engine read, `min(the WAL's copy, the engine
    row)`, and collects nothing when that read fails or after `finish`. `wal-collected` (the
    highest collected sequence of each partition) is durable before the first unlink it
    covers. A segment is sealed only by a tick, once it holds 8 MiB or its first frame is 30 s
    old. The `.applied` files a stop that returns `Ok` writes are rollback hints for older
    binaries, each equal to the durable engine row (also after a drain that did not finish);
    this binary reads one only through the import at open, when the WAL directory holds no
    `wal-collected` or the store is `wal_import_pending`, and deletes every one at open.
  - *Sidecar mode:* the watermark sidecar write+fsync completes before any sealed-segment
    unlink it licenses.
- **Maintaining code:** store mode — `wal/mod.rs` `open_over_store`, `mark_applied`, the
  collection tick, `write_wal_collected`, `finish`; `engine_store.rs` `commit` (records and
  `0x20` rows in one batch, a lower row refused); `write_behind.rs` the commit plan and its
  settle after `Ok`. Sidecar mode — `wal/mod.rs` `mark_applied` (fsync-before-unlink block +
  apply-time re-validation before physical delete, SPEC-350).
- **Enforcing test:** store mode — `store_mode_tests.rs::a01_a_commit_is_one_engine_write_holding_its_records_and_its_watermark_rows`,
  `a02_a_refused_commit_changes_no_watermark_and_no_pending_sequence`,
  `a06_the_wal_collects_only_up_to_the_durable_watermark_on_its_tick`,
  `a08_the_collected_mark_is_durable_before_the_first_unlink`,
  `a18_a_kill_inside_or_right_after_a_commit_loses_no_acked_write`,
  `a20_a_commit_carrying_a_watermark_below_the_held_row_is_refused_and_writes_nothing`,
  `a21_a_tick_whose_watermark_read_fails_unlinks_nothing_and_the_next_tick_collects`,
  `a22_after_finish_no_tick_unlinks_and_the_tick_task_ends`,
  `a23_a_clean_stop_writes_one_rollback_hint_per_partition_equal_to_the_engine_row`.
  Sidecar mode — `wal_harness/cases.rs::ac7_tg_wal_003_gc_crash_point_both_directions` —
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
  boot seeding; unseeded-refuse guard. Store mode: the commit plan of `write_behind.rs` puts a
  partition's row in the commit only when its new value is above the durable one — never a
  lower or equal value, so a row cannot regress after a restart resets the tracker.
- **Enforcing test:** sidecar mode — `prefix_watermark_proptest.rs` — restart-crossing proptest;
  both loss-guards (unseeded-refuse, seed-retry) verified by revert during review (fail on pre-fix
  code). Generatively re-enforced by `wal_harness/cases.rs::ac4_c3_scalar_max_watermark_regression`
  (the C3 scalar-max over-advance, found from generated crash/recover sequences). Store mode —
  `store_mode_tests.rs::a03_a_partition_whose_watermark_moved_by_coalescing_gets_its_row`,
  `a04_an_unseeded_partition_gets_no_row`,
  `a05_the_engine_watermark_never_decreases_across_commits_and_restarts`.
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
- **Windowing residual — structurally absent.** The frameless window existed only because the
  single-key flush method of the data store, removed, made a value durable in the inner store with
  no frame of its own. With the method gone, no path makes a value durable without a frame of its
  own, so the window is **structurally absent** — not "confirmed a no-op by a test". It was a
  WATERMARK/FRAMING concern rather than a Remove-idempotency one, which is why it was never papered
  over with an unsound Remove-replay timestamp gate. Its two directions:
  - **Crash-window direction (TODO-612) — CLOSED.** It was determined a no-op while the method
    existed, by a test that drove the method; the test went with the method, and the window it
    held open has no producer left.
  - **Caller-side direction (TODO-628) — CLOSED by the removal.** The method made the CALLER's
    value durable while resolving the superseded entry's sequences, so a caller passing a value
    that did not subsume a staged `WalOp::Remove` would have lost an acked delete with no crash at
    all. No caller can do that now. Normative for any successor: a single-key flush, if one is
    ever needed,
    takes no value from its caller — it flushes the buffered entry itself — and is designed
    together with its caller.
  - **A refused flush opens no similar window (established by reading the flush pass).** An entry
    the store refuses goes back to its queue with its WAL sequences un-resolved; when a newer
    write of the same key is queued by then, the sequences are carried onto that entry
    (`PartitionQueue::retire_into_queued`). The flush pass never resolves the sequences of an
    entry the store refused, and no longer abandons them either (`TG-WB-009`). Carried
    sequences are resolved when the survivor is durable, or earlier, when a subsuming write
    coalesces over the survivor (`coalesce_wal_sequences`; holder
    `a_subsuming_write_over_a_survivor_resolves_the_carried_sequences`). The early resolve is
    sound: the subsuming write's own frame is still pending at a higher sequence, so the watermark
    does not pass it. No value becomes durable without a frame of its own.
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

### TG-WAL-013: The data directory is never older than the WAL directory it is opened with

- **Scope:** `WalWriter::open_over_store` (store mode, the server's WAL) over the
  `EngineMapDataStore` of the data directory; `wal-collected` in the WAL directory; recovery and
  the sequence seed in store mode.
- **Statement:** every partition's applied watermark — the engine row, or the `.applied` sidecar
  it is about to be imported from — is at least the `wal-collected` mark of the WAL directory, or
  the start is refused with nothing written (`WAL directory check: refusing to start: the data
  directory is older than the WAL directory …`, the first partition below its mark named).
  `wal-collected` is durable before any segment it covers is unlinked, and never exceeds the
  durable watermark. A data directory with no applied watermarks (no `wal_watermark` marker)
  beside a WAL directory that holds `wal-collected` is refused unless the store is
  `wal_import_pending` (just migrated from the old file) — so neither a pre-marker data
  directory beside a WAL directory after `kill -9` (no hints) nor one beside a WAL directory
  after a stop with hints is imported silently.
  - *The reverse case — a WAL directory older than the data directory.* It is not refused, and
    it replays nothing wrong and reuses no sequence; no shared id or epoch ties the two
    directories. (i) The replay window is above the engine watermark: `unapplied(p)` hands only
    frames with `sequence > W(p)`, so every frame of the older copy at or below `W(p)` is
    filtered, and a frame at or below `W(p)` that reached recovery would refuse the start
    (`WAL recovery: refusing to start: partition {p} holds frame {sequence} at or below its
    applied watermark {W}; …`), never be re-applied. (ii) The sequence counter is seeded from
    the engine, `max(W(p), the highest sequence in any segment)`, so a new write never takes a
    number at or below a stored row. Hence nothing acknowledged **and already in the data
    directory** is lost or overwritten. What the data directory does not hold is only what the
    WAL held above `W(p)` at the moment the data directory was copied; after a stop that exited
    0 **and logged `write-behind shutdown: rollback hints written` with `drained=true`** that set
    is empty, so a backup of both directories taken after such a stop loses nothing acknowledged
    whichever of the two copies is older. An exit 0 alone is not enough: a drain that timed out
    exits 0 with frames above `W(p)` (`drained=false`). A copy of a running server is not a
    backup.
- **Maintaining code:** `wal/mod.rs` — `open_over_store` (the import decision, the
  `wal-collected` read, `refuse_collected_above_watermark`), `write_wal_collected`, the
  collection tick (the mark written before the unlink, `G(p) ≤` the durable watermark),
  `unapplied`, `max_observed_sequence`; `WalRecovery::run` (the refusal of a frame at or below
  the engine watermark); `engine_store.rs` `applied_watermarks`.
- **Enforcing test:** `store_mode_tests.rs::a09_a_data_directory_older_than_the_wal_directory_is_refused_and_nothing_is_written`
  (both variants: a data directory below the collected mark, and one with no applied watermarks
  beside the WAL directory of a killed run and of a clean stop),
  `a08_the_collected_mark_is_durable_before_the_first_unlink`,
  `a24_a_store_just_migrated_beside_a_used_wal_directory_imports_the_sidecars_once`; the reverse
  case `a19_a_wal_directory_older_than_the_data_directory_replays_nothing_at_or_below_the_watermark_and_reuses_no_sequence`;
  the migrated case end to end, `tests/integration-rust/map-name-rollback.test.ts` R-D.
- **Violation consequence:** a data directory restored from an older backup beside a newer WAL
  directory would serve without the frames the WAL has already collected — acknowledged writes
  silently gone — or a sidecar no longer describing the data directory would be imported as its
  watermark.
- **Not claimed:** that a data directory and a WAL directory from two different nodes, or from
  two unrelated runs, are recognised as such; only the order of the two copies is checked.
- **Discovered by:** SPEC-395a.
- **Status:** decided, **enforced**.

### TG-WB-001: The flushed watermark is prefix-complete — no mid-range hole

- **Scope:** entry-ordering-space `pending_seqs` / `flushed_watermark()` (tombstone fence
  consumer; INDEPENDENT of TG-WAL-005's wal_seq-space tracker).
- **Statement:** `flushed_watermark()` never returns a value above a still-buffered sequence;
  assign+track is atomic under one lock (the mid-range-hole guard).
- **Maintaining code:** `write_behind.rs` `assign_tracked_sequence` + `resolve_pending`.
- **Enforcing test:** `write_behind.rs::ac3c_flushed_watermark_prefix_complete_never_exposes_hole`
  + surrounding block (coalesce-resolves-a-hole, prune-frontier-stall regression), and
  `write_behind_batch_tests.rs::b10_a_refused_commit_leaves_staging_counts_and_both_watermarks_unchanged`
  (a refused pass commit leaves the flushed watermark where it was). Additionally
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
- **Maintaining code:** `write_behind.rs` `put_back`, which a flush pass calls for an entry the
  store refused and for every entry it did not attempt when a store-wide hold began: the entry is
  put back on its queue in one statement, so the guard is a temporary. The refusal branch no
  longer awaits anything — the time of the next attempt is written on the entry and a later pass
  honours it (`TG-WB-009`) — so at this site the statement holds by construction.
- **Enforcing test:** `write_behind.rs::the_queue_shard_is_free_during_a_retry_backoff` — a write to
  the partition of a refused entry, while that entry waits for its next attempt, must complete
  within 1000 ms. With no await left in the refusal branch the test no longer discriminates at
  that site; it is kept as a guard. Two more enforcers cover other parts; the table below says
  which covers what.
- **Violation consequence:** the node stops. The flush task sleeps on a timer while it holds the
  shard lock; the stall watchdog (or any writer of that partition) blocks a runtime worker on the
  lock; if that worker was the one driving the runtime's timer and I/O driver, the timer never
  fires and the lock is never released. No new connection is answered, nothing is logged and
  SIGTERM is ignored — signals are delivered through the same driver — so the process has to be
  killed. Measured with the store refusing from the start and the default configuration: 5 of 5
  runs hung before the fix, 5 of 5 healthy after it.
- **Measured scope:** a refusal injected into the embedded (redb) backend, one or two refused
  keys. In that case the node keeps answering reads and new connections and exits on SIGTERM
  (2 ms to 1.46 s, measured before refused entries were retried until stored; the times were not
  measured again). This row
  states that the node does not hang on a store refusal; it does not state that the node is
  otherwise unaffected by one — see "After a store refusal" below.
- **Discovered by:** SPEC-381a wave A0 (rollback testing); fixed and catalogued by SPEC-382.
- **Status:** decided, **enforced** — at the one site the test covers and for the guard shapes the
  lint reports; see the blind spots.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `the_queue_shard_is_free_during_a_retry_backoff` | ONE site: where a flush pass puts a refused entry back on its queue. It no longer discriminates at that site: the pass does not await in the refusal branch any more, so there is no await left there for a guard to be held across. Kept as a guard. No other `queues` guard, no other map | CI (`cargo test`) |
| lint gate: `clippy::await_holding_invalid_type` over the type list in the root `clippy.toml`, and `clippy::await_holding_lock` | WIDER than the statement: every listed DashMap/DashSet guard type in every target of both workspace crates — within the shapes the table below shows it reports | CI (`cargo clippy`) |
| `tests/integration-rust/store-refusal-liveness.test.ts` | the CONSEQUENCE at node level for a store refusal — reads and new connections answered, clean exit on SIGTERM within 15 s. No site in particular. Its `STORE-DOWN-FROM-START` arm is the one that goes red under the default configuration on a binary without the fix. The file has six arms. What the arms assert about the refused write itself — it stays readable, a retry warning is logged, it reaches the store once the store accepts again, and it is read back from a second process after a SIGTERM during the refusal — belongs to `TG-WB-009`; the last arm, `REFUSAL-LIFTED-UNDER-LOAD`, refuses six load keys while they are still being written and belongs to `TG-WB-005` | local only (`pnpm test:integration-rust:liveness`) — left out of the default `pnpm test:integration-rust` run, locally and in CI, because it needs a second server build with the `fault-injection` feature (TODO-770) |

**After a store refusal — what this invariant does not cover.** Before the fix the node hung
within seconds of a refusal, so none of the states below was reached in practice. They are
reachable now. None of them is a violation of the statement above.

- **Write-behind accounting no longer leaks after a refusal.** This gap is closed: a refused
  entry that meets a newer write of its key is retired into that write, with its count, its
  entry sequence and its WAL sequences all disposed of. The contract, its one-writer-per-key
  precondition and what it still leaves out are in `TG-WB-005`.
- **A refused write is retried until the store takes it.** It is not given up on and does not
  wait for a restart: the entry stays buffered, with its entry sequence pending and its WAL
  sequences `Live`. For as long as the store refuses it, the tombstone fence and the partition's
  WAL watermark stay below it, the partition's WAL is not collected and, under continued writes,
  the watchdog reports `AbandonedWrite`. The schedule, its bounds and what it leaves out are in
  `TG-WB-009`.
- **Shutdown can take the full `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`.** Measured before
  refused entries were retried until stored, with 40 refused entries and the retry ladder of
  that time: 30.00 s, the whole timeout. Not measured again since. The cause named then — the
  flush loop slept a backoff per refused entry — is gone: a pass no longer waits on behalf of an
  entry. What a shutdown costs against a store that still refuses is one store call per buffered
  entry (`TG-WB-009`, "Not claimed").

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
  disposes of all three: its count, its entry sequence, and its WAL sequences. The exits are
  the success terminal (the store accepted the entry: all three resolved), the retirement into a
  queued newer write of the same key, and the shutdown drain. "Abandoned with the `Abandoned`
  tag" is true only of the shutdown drain, which is outside the scope above; on a running node
  no WAL sequence is abandoned. An entry the store refuses goes back on its queue and has left
  nothing: its count, its entry sequence and its WAL sequences stay as they were
  (`TG-WB-009`). The retirement happens when an entry that is put back — after a refusal, or
  un-attempted when a store-wide hold began — finds a newer write of its key on the queue: the
  count goes down by one, the entry sequence is resolved, and the WAL sequences move to the
  newer entry under the same shard lock, so they resolve when it becomes durable and are
  replayed at boot if it never does. Nothing suspends between the store's answer and the end of
  the put-back, so a cancelled flush task cannot leave it half done. A flush batch is one pass,
  and a pass holds at most `L = 1000` entries or 8 MiB of encoded values (the first entry taken
  whole); the entries it does not take stay queued, untouched. With `C = capacity > 0` this
  bounds the count: `pending_count <= C - 1 + W + H`, `H <= min(L, C - 1 + W)`, and `H <= 1`
  during a store-wide hold (`TG-WB-006`, "Not claimed", `capacity` as a hard limit). Every
  commit of a pass settles its entries only after the store accepted it; a refused commit
  changes no entry's state before the refusal is handled (`TG-WB-009`).
- **Mechanical predicate:** once the store accepts writes again and the write-behind store is
  idle — no entry is queued AND the flush loop is between passes, holding no drained batch
  ("every queue empty" alone is not idle: the loop may hold a batch it has just drained) —
  `pending_operation_count() == 0`, `flushed_watermark() == assigned_write_sequence()`, no
  WAL sequence is `Live`, no WAL sequence is `Abandoned`, and the inner store holds the last
  acknowledged operation of every key.
- **Maintaining code:** `write_behind.rs` — `PartitionQueue::retire_into_queued`, called from
  `reinsert_front`, which `put_back` calls for an entry the store refused and for every entry a
  pass did not attempt when a hold began; `settle_stored`, the one success terminal.
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
| `interleaved_refusals_and_writes_to_one_key_leave_nothing_unaccounted` | the predicate, for one key and one writer, over generated orders of refusals and writes, after one fixed order with more refusals than the retired retry limit allowed; no sequence may be `Abandoned` and the inner store must hold the last operation | CI (`cargo test`) |
| `a_refused_entry_superseded_by_a_newer_write_is_retired_into_it` | the retry path, one fixed order: counter, watermark, WAL sequences, watchdog class | CI (`cargo test`) |
| `a_write_in_the_flush_window_then_a_refusal_leaves_nothing_unaccounted` | the retry path at its smallest: one write in the flush window, one refusal | CI (`cargo test`) |
| `a_superseded_retry_entry_stays_replayable_until_its_survivor_is_durable` | WHY the WAL sequences are carried and not resolved at once: with a second key in the same partition, a crash before the newer entry is durable must still replay the retired frame | CI (`cargo test`) |
| `a_subsuming_write_over_a_survivor_resolves_the_carried_sequences` | a later write that subsumes the newer entry resolves the sequences that entry carried. Also WHEN the retirement is accounted: while the refused entry's next attempt is still ahead, the count and the flushed watermark already have their settled values | CI (`cargo test`) |
| `a_refused_entry_with_a_queued_newer_write_is_retired_into_it` | the retirement with the newer write already queued when the store refuses: the refused entry is not tagged `Abandoned`, and the key becomes durable through the newer write | CI (`cargo test`) |
| `a_retired_delta_entry_stays_replayable_until_its_survivor_is_durable` | WHY the sequences are carried for a delta frame too: the newer entry is refused ten times and stays queued with both sequences `Live`; with a second key in the same partition, a crash before the newer entry is durable must still replay the retired frame | CI (`cargo test`) |
| `a_retired_entry_hands_its_earlier_due_time_to_the_queued_write` | the queued entry takes the earlier due time of the two, so a key rewritten during an outage keeps its flush schedule | CI (`cargo test`) |
| `two_writers_of_one_key_interleaved_between_sequence_and_insert_leave_nothing_unaccounted` | NOT an enforcer of the precondition. Evidence for accounting only, under one interleaving of two writers that the precondition excludes. It asserts nothing about the stored value | CI (`cargo test`) |
| `tests/integration-rust/store-refusal-liveness.test.ts`, arm `REFUSAL-LIFTED-UNDER-LOAD` | the predicate at node level: six keys refused while they are still written; after the refusal is lifted every partition's WAL lag returns to 0 | local only (`pnpm test:integration-rust:liveness`), for the reason given under `TG-WB-004` (TODO-770) |

**Not covered.** Each of these is an exit the statement names but no test above reaches, or a
case outside the precondition.

- **Shutdown.** The flush task aborted at the shutdown timeout, the error, timeout and
  final-sweep exits of `hard_flush`, and a `hard_flush` future dropped mid-drain (TODO-768).
- **The flush task ending by a panic.** The node does not keep running: the store fails and the
  server binary exits non-zero (`TG-WB-006`).
- **A write cancelled inside the WAL append** (`TG-WB-003` (b), TODO-672).
- **Two concurrent writers of one key.** The writers of a record key are serialised above this
  store (`TG-KEY-001`), so it no longer meets them. One interleaving below the writer is still
  tested here, for accounting only. The case in which the carry would make the outcome after a
  restart worse (see "Not claimed") is covered by one test driven through the service that needs
  the writer: `two_routes_and_a_refused_flush_recover_the_later_stamp_after_a_crash`. A second
  one, `an_older_value_arriving_behind_a_refused_newer_one_does_not_win_after_a_crash`, shows
  that a write which arrives later behind a refused flush also carries the higher stamp, because
  its stamp is minted after its read of the previous value; that test passes without the writer.

### TG-WB-006: A write-behind flush task or stall watchdog that ends by a panic puts the store in a failed state: new writes are refused, on a WAL-backed store the dead batch stays replayable, and the server binary exits non-zero

- **Scope:** `WriteBehindDataStore` — its two background tasks (the flush task and the stall
  watchdog), its lifecycle state, `hard_flush` — and the shutdown path of the server binary
  (`bin/topgun_server.rs`). The store half holds for any embedder; the exit code is a property
  of the server binary only.
- **Statement:** when the flush task or the stall watchdog ends by a panic, the dying task
  itself, before its poll returns to the runtime, puts the store in a failed state and records
  which task failed and with what message. That happens in every lifecycle state, a shutdown
  already under way included, and the failed state is never left. From then on:
  - `add`, `remove` and `remove_all` return an error before a WAL sequence is assigned and
    before anything is appended to the WAL, so no write is acknowledged by a store that can no
    longer flush;
  - nothing resolves, abandons or re-queues the WAL sequences of the batch the dead flush task
    held. On a WAL-backed store those frames, and every later frame of the same partitions,
    stay in the unapplied window and are replayed at the next start, in ascending order
    (`TG-WAL-005`, `TG-WAL-010`, `TG-WAL-011`), so the replay ends on the newest value of a key;
  - `hard_flush` still drains and persists what is queued, syncs the WAL before each of its
    returns, reports how many entries it did not persist, and returns an error whose source is
    the recorded failure — whether the failure was recorded before it was called or while it
    ran;
  - the server binary reports not ready, runs its ordinary shutdown and ends with exit code 70
    when that shutdown completed, or 71 when the shutdown itself panicked or passed its
    deadline (30 s + the write-behind shutdown timeout + 10 s). The code is non-zero whichever
    came first, the panic or a stop signal: the binary reads the failure again after
    `hard_flush` has returned. A stop with no recorded failure still exits 0.
  The failed state has a second cause, an inner store that reports itself broken
  (`TG-WB-010`); for it exit code 72 replaces 70, and everything else above holds as stated.
- **Mechanical predicate:** once the poll of a panicked background task has returned, (a)
  `background_failure()` holds `Some` with that task and the panic message, and a second failure
  does not replace it; (b) the next `add`, `remove` or `remove_all` returns the failed-store
  error and the WAL saw no append for it; (c) after `hard_flush`, which returns `Err` with a
  `BackgroundFailure` as its source, `wal.unapplied(p)` still returns the dead batch's frames
  and every later frame of each partition `p` the batch touched; (d)
  `exit_code_for(Panicked, Completed) == 70` and `exit_code_for(_, PanickedOrOverran) == 71`, and
  the decision that feeds it is true whenever the stop was caused by a failure, or the record
  holds one, or the drain's result carries one.
- **Maintaining code:** `write_behind.rs` — `ObservedTask` (the wrapper both tasks are spawned
  through by `spawn_background_task`; it catches the unwind inside `poll`),
  `record_background_failure` (state, error line, counter and signal in one stretch with no
  await), the `Lifecycle` state read by the three write entries, `hard_flush`.
  `wal/mod.rs` — `Wal::sync`, `WalWriter::sync`. `bin/topgun_server.rs` —
  `store_failure_or_signal`, `apply_shutdown_cause`, `store_failed`, `exit_code_for`,
  `start_failure_deadline`, and the two `std::process::exit` calls after the drain.
- **Enforcing test:** `write_behind.rs::a_flush_task_panic_fails_the_store` and
  `write_behind.rs::a_stall_watchdog_panic_fails_the_store` — after the panicked task's poll has
  returned, the very next write is refused and `hard_flush` returns an error. Both run in CI
  (`cargo test`). The table below lists the others and what each one covers.
- **Violation consequence:** a node whose flush task has died keeps acknowledging writes that
  nothing will ever flush while the process lives, reports itself healthy, and on a later stop
  tells its supervisor "clean stop" with exit code 0. Operators see no line and no metric. The
  write buffer fills until the capacity check rejects writes for an unrelated-looking reason.
- **Visible to an operator:** one error line per failure
  (`write-behind background task panicked; the store refuses writes from now on`, fields `task`
  and `panic`), the counter `topgun_write_behind_background_task_panics_total{task}` (both
  `task` values render at 0 from the first scrape), `/health/ready` and `/healthz` at 503 while
  the stop runs (`/health/live` stays 200), and the exit code.
- **Discovered by:** a review of the exits `TG-WB-005` leaves uncovered; fixed and catalogued by
  SPEC-386.
- **Status:** decided, **enforced** — within the limits listed under "Not claimed" below.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `a_flush_task_panic_fails_the_store` | a flush-task panic: the first write after the panicked poll returned is refused; `hard_flush` returns an error | CI (`cargo test`) |
| `a_stall_watchdog_panic_fails_the_store` | the same for a panic of the stall watchdog, on a WAL-backed store | CI (`cargo test`) |
| `a_flush_task_panic_is_observed_without_any_call` | the failure record holds the task and the panic text with no write and no `hard_flush` in between | CI (`cargo test`) |
| `a_flush_task_panic_is_logged_once_at_error` | exactly one error line, with both fields | CI (`cargo test`) |
| `a_background_task_panic_is_counted_by_task` | both counter series at 0 after construction; the `flush` series at 1 after the panic, the other still 0 | CI (`cargo test`) |
| `a_failed_store_refuses_add`, `a_failed_store_refuses_remove`, `a_failed_store_refuses_remove_all` | each write entry returns the failed-store error, the WAL saw no append, the pending count is unchanged | CI (`cargo test`) |
| `a_write_past_the_check_when_the_task_dies_is_drained_at_shutdown` | a write that passed the state check before the failure returns `Ok` and its value is persisted by the shutdown drain | CI (`cargo test`) |
| `the_dead_tasks_batch_stays_unresolved_through_shutdown` | on a real WAL: `hard_flush` completes with a non-zero pending count, the report carries the batch size, and the later frame of a key that is in both the batch and the drain is still unapplied | CI (`cargo test`) |
| `after_a_flush_task_panic_a_restart_holds_every_acked_write` | recovery over the same directory: every key holds its last acknowledged value — one key only in the dead batch, one only drained, one in both; an OR-Map key written with delta framing holds each tag once | CI (`cargo test`) |
| `hard_flush_syncs_the_wal_before_every_return` | `sync` is the last WAL call on the empty-queue return, the timeout-sweep return and the failed-store return | CI (`cargo test`) |
| `wal_writer_sync_leaves_no_unfsynced_frame` (`wal/mod.rs`) | under `batched`, after appends to two partitions and `sync`, neither active segment has an un-fsynced frame | CI (`cargo test`) |
| `a_stall_watchdog_panic_is_recorded_and_shutdown_still_drains` | a watchdog failure: the record names the watchdog, the flush task still stops on the shutdown signal, `hard_flush` drains everything, returns an error and reports nothing un-persisted | CI (`cargo test`) |
| `a_flush_task_panic_after_shutdown_began_is_still_a_failure` | the order "stop first, panic second" in the store: the failure is recorded, `hard_flush` returns the failed-store error, the parked batch stays unapplied | CI (`cargo test`) |
| `a_clean_stop_is_not_a_failure` | what is NOT a failure: `hard_flush` on a healthy store, a store with no WAL, the flush task aborted at the shutdown timeout | CI (`cargo test`) |
| `hard_flush_completes_with_a_poisoned_pending_seqs_lock`, `hard_flush_completes_with_a_poisoned_flush_loop_handle_lock`, `hard_flush_completes_with_a_poisoned_wal_tracking_cell_lock` | a lock poisoned by a panicking thread does not stop the shutdown path | CI (`cargo test`) |
| `the_capacity_pre_check_bound_is_the_stated_one` | the bound the capacity pre-check keeps while a flush pass holds a batch: re-writes of the held keys are accepted, a new key is rejected | CI (`cargo test`) |
| `distinct_new_key_writers_past_the_check_exceed_capacity_by_at_most_their_number` (`prefix_watermark_proptest.rs`) | the other term of the bound: writers of distinct new keys parked between the check and their insert are all accepted and the count ends at `capacity - 1` plus their number; and the boundary itself — a new key is accepted when the check reads `capacity - 1` and refused when it reads exactly `capacity` | CI (`cargo test`) |
| `a_flush_pass_holds_every_due_entry_whatever_the_batch_size` | below the pass limits (at most 1000 entries and 8 MiB of encoded values per pass) a pass takes every due entry out of every queue, with `batch_size` 1 and due keys in two partitions, two of them in one; the limits themselves are held by `write_behind_batch_tests.rs::b02_a_pass_takes_at_most_1000_entries_or_8_mib_and_the_first_entry_whole` | CI (`cargo test`) |
| `a_remove_of_a_key_that_is_not_buffered_is_refused_at_capacity`, `a_remove_all_of_a_key_that_is_not_buffered_is_refused_at_capacity` | the same check in `remove` and in `remove_all` (one key per call): accepted at `capacity - 1`, refused at exactly `capacity`, a buffered key accepted at `capacity` without being counted again | CI (`cargo test`) |
| the seven `bin_cause_*` tests (`bin/topgun_server.rs`) | the binary's decisions without a process: a failure puts the health controller in `Draining`; signal and a healthy store → 0; signal, then a failure in the record or in the drain's result → 70; failure and a completed shutdown → 70; a shutdown that did not complete → 71, with or without a recorded failure | CI (`cargo test`) |
| `the_failure_deadline_calls_its_exit_action_unless_cancelled` (`bin/topgun_server.rs`) | the deadline function: left alone it calls its exit action once with 71; cancelled, it never calls it | CI (`cargo test`) |
| `tests/integration-rust/flush-task-panic.test.ts` | the CONSEQUENCE at process level on a `fault-injection` build, the client connection open throughout: with one write after the panic sentinel the process exits 70 by itself; with a stream of writes the shutdown drain panics too and it exits 71; after each, a restart on the same data directory reads every acknowledged key | local only (`pnpm test:integration-rust:liveness`) — opt-in, not run in CI, for the reason given under `TG-WB-004` (TODO-770) |

**Not claimed.** Each of these is outside the statement, or is in it and rests on something
weaker than a test.

- **A deterministic panic.** Established by reading: boot replay calls the inner store directly
  and does not run the flush loop. A panic that originates inside the inner store's `add` or
  `remove` for a given record is met again at replay, on `main`, before the listener is bound:
  the node fails at every start. A panic that originates in write-behind's own code is not on
  the replay path. No quarantine policy exists (TODO-805).
- **A deterministic panic of the stall watchdog.** The watchdog is under the same policy as the
  flush task, so a watchdog panic that repeats at every start gives a crash loop caused by a
  diagnostic task, on a node whose flushing may work. It is not restarted in the process and not
  exempted (TODO-805).
- **Other background tasks.** The batched WAL fsync timer, the partition workers, the tombstone
  cursor worker and the rest are not supervised (TODO-804, which lists them).
- **A flush task or watchdog that returns without a panic** while the store is not shutting
  down (TODO-804).
- **Embedders other than the server binary.** The library serve path (`network/module.rs`) does
  not read the failure signal; a store embedded there refuses writes and returns `Err` from
  `hard_flush`, and nothing shuts the process down (TODO-804).
- **The inner store after a panic inside it.** Whether redb stays usable for the
  shutdown drain after one of its calls panicked was not established. If the drain panics, the
  process exits with 71.
- **Durability of the un-fsynced tail** when the shutdown panics or overruns under `batched`:
  the frames were written to the file; they are not fsynced.
- **The window between the state check and the failure is not empty.** A write that loaded the
  lifecycle state before the dying task stored it is acknowledged: its frame is in the WAL, and
  its entry is persisted by the shutdown drain or replayed at the next start.
- **The 503 at process level.** The health reaction is enforced by the `bin_cause_*` unit tests;
  an answer of 503 from a running process between the failure and its exit was not observed.
- **The deadline at process level.** The deadline is tested through its function, not on a
  process; the behaviour with a runtime that has stopped completely was not observed.
- **The order "state before the log line and the signal"** inside `record_background_failure`
  is pinned by reading the code, not by a test or a mutant.
- **`hard_flush` exits already tracked elsewhere** — the abort at the shutdown timeout, the
  error and timeout exits, a dropped `hard_flush` future (TODO-768).
- **`capacity` as a hard limit.** Decided: there is no hard limit and none is planned. The
  pre-check keeps a soft bound. With `C = capacity > 0` and `L = 1000`, the most entries one
  flush pass takes: `pending_count <= C - 1 + W + H` and `H <= min(L, C - 1 + W)`, so the count
  stays below `2 * (C + W)` and below `C + W + L`. `W` is the number of writes of
  distinct keys that are not buffered and that are, at one instant, between the capacity check
  and their queue insert; `W >= 1` while any write is in progress, because the inserting writer
  is itself past its check. `H` is the number of entries the running flush pass holds whose key
  was re-written while the pass holds them; a pass holds the entries that were due when it
  began, at most `L` of them and 8 MiB of encoded values, and `batch_size` does not limit it. A third line holds while a store-wide hold is in force
  (`TG-WB-009`): `H <= 1`, because a hold puts back every entry the pass did not attempt and a
  pass during a hold holds at most the one entry it probes the store with; an entry waiting for
  its next attempt is in its queue, not in a pass, and adds nothing to `H`. What changes with a
  store that refuses is not the bound but how long it is met: nothing leaves the buffer, so the
  count stays at the bound until the store accepts again, and for that long every write of a key
  that is not buffered is refused. **Precondition:** one writer per key (`TG-KEY-001`). The
  formula also rests on `TG-WB-007`: a key's staging slot exists only while an entry of the key
  is buffered, or while the key's writer is between staging the key and queueing its entry. The
  enforcers are the five capacity rows of the table above. Not covered: a panic between a
  write's staging and its queue insert leaves a key staged with nothing buffered, whose next
  write passes the check at any count (`TG-WB-007`, "Not claimed"; TODO-817).
- **Which statements of the flush loop or of an inner store can panic.** Not enumerated; the
  tests inject one.
- **A stop started by a signal has no deadline of its own**, before this entry or after it. When
  a task panics during such a stop, the binary learns of it after `hard_flush` returned and
  exits 70 at once; nothing bounds that stop beyond what bounded it before (the shutdown timeout
  inside `hard_flush`, the 30 s wait for in-flight requests).
- **A panic of `hard_flush` itself on the signal path.** There `hard_flush` is awaited on
  `main`; its panic is a panic on `main` and the process exits 101, whether or not a failure was
  recorded. The code is non-zero; it is not 71. `exit_code_for` returns 71 for "no failure,
  shutdown panicked or overran", and no production call reaches that cell.
- **The order "signal, then panic" at process level.** It is enforced in the store by
  `a_flush_task_panic_after_shutdown_began_is_still_a_failure` and in the binary by the two
  `bin_cause_row_2*` tests; no process was run through it.
- **A failure recorded after the binary's last read.** A watchdog that panics after the binary
  read the record following `hard_flush` is not seen; the process exits 0. The same holds for a
  flush task whose panic coincides with its abort at the shutdown timeout: the shutdown does not
  wait for an aborted task. In every other case the flush task has ended before `hard_flush`
  drains.
- **The watchdog's path to the exit code at process level.** The watchdog reaches the exit only
  through the shared failure record; the store half is tested, and the process-level test
  injects its panic where an entry is persisted, which the watchdog never does. No process was
  run with a watchdog panic.
- **The two call sites in `main`** — the call of the deadline function and the read of the
  failure after `hard_flush` — are pinned by reading, not by a test or a mutant. The unit tests
  drive the functions, not their call sites; the only check of the wiring is the process-level
  test, which is local.
- **The dead batch on a store without a WAL.** `WriteBehindDataStore::new` builds a store with
  no WAL; there the dead batch's writes are lost at the exit. "Stays replayable" is claimed for
  a WAL-backed store only, which is what the two tests of the batch use.
- **`unpersisted_entries` is not the size of the dead batch.** It is the pending count, which
  also holds the entries the shutdown drain timed out or failed on.
- **The inner store after an exit that skipped its destructors.** Exit codes 70 and 71 are
  produced by `std::process::exit`, which does not close redb. What an
  unclosed redb file costs at the next start was not verified; the restarts of the process-level
  test observe it for two scenarios and no more.
- **What a client receives for a refused write.** The store's error maps to `internal`, 500,
  transient by reading; no test asserts it. The process-level test asserts only that the answer
  is not an acknowledgement and records what it was.

### TG-WB-007: A write-behind staging slot exists only while an entry of its key is buffered, or while the key's writer is between staging the key and queueing its entry

- **Scope:** `staging`, `queues` and the flush pass of `WriteBehindDataStore`; its three write
  entries (`add`, `remove`, `remove_all`); the two sites that clear a slot. The relation is
  stated for a node that is not shutting down and whose flush task is alive. For the shutdown
  drain only the clearing rule is stated.
- **Precondition:** one writer per `(map, key)` at a time (`TG-KEY-001`).
- **Statement:**
  1. A write stages its key directly before its queue insert. In a non-test build no statement
     lies between the `stage` call and the queue insert, the stretch from that call to the end
     of the insert has no await, and no lock taken earlier is held across it.
  2. Every site that removes a slot removes it after its entry was persisted, and only when the
     slot's sequence equals that entry's sequence: the success terminal of `flush_loop`, and the
     shutdown drain. An entry the store refuses keeps its slot (`TG-WB-009`). Without the
     comparison the drain would strip the slot of a newer write of the key, whether that write's
     entry is already queued or not.
  3. Therefore a slot for a key exists only while an entry of the key is queued or held by the
     running flush pass, or while the key's writer is between `stage` and its queue insert.
- **Mechanical predicate:** on an idle store — no entry queued and the flush loop between
  passes, in the sense of `TG-WB-005` — `staging` holds no slot.
- **Maintaining code:** `write_behind.rs` — `add_with_witness`, `remove`, `remove_all` (the
  order of the two steps), `stage`, `clear_staging_if_current`, `flush_loop` (its success terminal,
  `settle_stored`), `drain_for_shutdown`.
- **Enforcing test:** `write_behind.rs::a_pass_between_the_two_steps_of_a_new_key_write_leaves_no_slot_without_an_entry`,
  `write_behind.rs::a_pass_between_the_two_steps_of_a_coalescing_write_leaves_no_slot_without_an_entry`,
  `write_behind.rs::a_pass_between_the_two_steps_of_a_remove_leaves_no_slot_without_an_entry`,
  `write_behind.rs::a_pass_between_the_two_steps_of_a_remove_all_leaves_no_slot_without_an_entry`,
  `write_behind.rs::a_shutdown_drain_between_the_two_steps_of_a_write_keeps_slot_and_entry_together`,
  `write_behind.rs::a_write_completed_while_the_drain_persists_an_older_entry_of_its_key_keeps_its_slot`
  and `write_behind.rs::ac8_flush_does_not_clear_newer_coalesced_staging`. All run in CI
  (`cargo test`). The table below says which form each one covers.
- **Violation consequence:** a key staged with nothing buffered passes the capacity check at any
  count, and each occurrence leaves one such key until it is written again, so the excess over
  `capacity` has no bound. The slot's memory is not returned by eviction. The slot stays in
  every scan and Merkle overlay, and a read of the key is answered from it instead of from the
  store.
- **Discovered by:** the audit of the previous write-behind work (the capacity bound of
  `TG-WB-006`), which reached `pending_count` 4 against a capacity of 1 through this
  interleaving; fixed and catalogued here.
- **Status:** decided, **enforced** — within the limits listed under "Not claimed" below.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `a_pass_between_the_two_steps_of_a_new_key_write_leaves_no_slot_without_an_entry` | a new key: a flush pass between the two steps of an `add`; at rest `staging` is empty, and the key's next write is refused at capacity | CI (`cargo test`) |
| `a_pass_between_the_two_steps_of_a_coalescing_write_leaves_no_slot_without_an_entry` | a coalescing write: the pass persists the older entry of the key while the newer write is between its two steps; the newer write keeps slot and entry, and after shutdown `staging` is empty | CI (`cargo test`) |
| `a_pass_between_the_two_steps_of_a_remove_leaves_no_slot_without_an_entry` | the same for `remove` | CI (`cargo test`) |
| `a_pass_between_the_two_steps_of_a_remove_all_leaves_no_slot_without_an_entry` | the same for one key of a `remove_all` | CI (`cargo test`) |
| `a_shutdown_drain_between_the_two_steps_of_a_write_keeps_slot_and_entry_together` | the shutdown drain between the two steps of a write of a queued key: slot and entry carry the same sequence afterwards and a read answers the newer value | CI (`cargo test`) |
| `a_write_completed_while_the_drain_persists_an_older_entry_of_its_key_keeps_its_slot` | the shutdown drain around its own store call: a write that completes while the drain persists the older entry of its key keeps its slot and stays readable | CI (`cargo test`) |
| `ac8_flush_does_not_clear_newer_coalesced_staging` | the sequence comparison itself: `clear_staging_if_current` leaves a slot of another sequence | CI (`cargo test`) |

**Not claimed.**

- **A panic of the writer between `stage` and the queue insert.** It leaves a slot with nothing
  buffered, until the key is written again or the node restarts. And on every base, before this
  entry and after it, a panic of the writer after its entry sequence is assigned leaves that
  sequence pending, which holds the flushed watermark until a restart. Which statements there
  can panic was not enumerated. Both are tracked in TODO-817.
- **Two concurrent writers of one key.** Outside the precondition. The queue may hold the lower
  sequence while staging holds the higher, and no terminal then clears the slot.
- **Entries a shutdown gives up on.** The entries a shutdown drain gives up on at its timeout,
  and the batch a dead flush task held (`TG-WB-006`), keep their slots on a store that is
  exiting.
- **A write that reaches its queue insert after the shutdown drain's last look at the queues.**
  It stays queued and is not persisted by that process. On a WAL-backed store its frame replays
  at the next start; on a store without a WAL the write is gone.
- **That a test or a CI lint holds "no await in the stretch".** It is held by a predicate over
  the source run when the change was accepted, and by the comment-contract in the code. A later
  edit that adds an `.await` or a held lock between `stage` and the queue insert is caught by
  nothing automatic.

### TG-WB-008: A resumable value scan through the write-behind store over an inner store that declares its scan order emits a key in at most one batch

- **Scope:** `scan_values` / `scan_values_batched` of `WriteBehindDataStore` for a primary scan
  over an inner store that declares a scan order (`MapDataStore::scan_order`) — redb today,
  declaring `PARTITION_THEN_KEY`.
- **Preconditions:** the inner store upholds the four clauses of the declaration's contract,
  stated over the positions of its `ScanOrder`; the
  caller passes each returned cursor back unchanged, once, in order, with the same map; one
  writer per `(map, key)` (`TG-KEY-001`).
- **Statement:**
  1. The position ranges of the batches of one scan tile the position space of the declared
     order: each batch covers the keys whose position is above the position of the cursor it was
     resumed from, up to and including the position of the cursor it returns.
  2. A key is emitted in at most one batch, at most once.
  3. A key that, from the start to the return of the call covering its range, always has a
     staging slot holding a value or a durable row, and no staged delete, is emitted once.
  4. A staged delete is emitted nowhere.
  5. Over a store that declares a scan order, a batch with rows whose cursor's position is not
     the position of its last row fails the scan call.
- **Maintaining code:** `map_data_store.rs` — `ScanOrder`; `write_behind.rs` —
  `overlay_scan_batch`, the two scan bodies (snapshot before the inner read), `flush_loop` and
  `drain_for_shutdown` (row before slot); `engine_store.rs` — the partition scan (`scan_from`)
  and `PARTITION_THEN_KEY`, which `scan_order` declares.
- **Enforcing test:** `write_behind.rs::a_staged_key_whose_durable_row_is_in_a_later_batch_is_emitted_once`,
  `write_behind.rs::a_staging_only_key_is_emitted_in_the_batch_whose_key_range_holds_it`,
  `write_behind.rs::a_key_flushed_between_two_batches_is_emitted_once`,
  `write_behind.rs::a_key_written_behind_the_scan_position_is_not_emitted_and_one_written_ahead_is_emitted_once`,
  `write_behind.rs::a_rowless_batch_with_a_cursor_still_covers_its_key_range`,
  `write_behind.rs::a_cursor_that_is_not_a_key_fails_the_scan_of_a_key_ordered_store`,
  `write_behind.rs::a_staged_key_whose_durable_row_is_in_the_first_batch_is_emitted_once`,
  `write_behind.rs::staging_only_keys_over_an_empty_durable_map_are_emitted_once`,
  `write_behind.rs::a_staged_delete_is_emitted_in_no_batch`,
  `write_behind.rs::a_key_flushed_while_its_batch_is_being_read_is_still_emitted`,
  `write_behind.rs::a_key_being_flushed_is_emitted_once_before_and_after_its_row_is_durable`,
  `write_behind.rs::a_key_flushed_while_a_resumed_batch_is_being_read_is_still_emitted`,
  `write_behind.rs::a_resumed_scan_given_a_cursor_that_is_not_a_key_fails`,
  `write_behind.rs::a_cursor_returned_by_a_resumed_batch_that_is_not_a_key_fails_the_scan`,
  `write_behind.rs::a_cursor_below_the_one_a_batch_was_resumed_from_fails_the_scan`,
  `write_behind.rs::a_declared_order_cursor_past_the_last_row_of_the_first_batch_fails_the_scan`,
  `write_behind.rs::a_declared_order_cursor_past_the_last_row_of_a_resumed_batch_fails_the_scan`,
  `write_behind.rs::a_cursor_that_is_not_a_key_passes_through_while_nothing_is_staged_for_the_map`,
  `write_behind.rs::over_a_store_that_does_not_declare_key_order_a_staged_key_with_a_later_row_is_still_emitted_twice`
  (pins the order-free rule over a store that does not declare a scan order, double emission
  included — the open defect, not wanted behaviour),
  `redb.rs::scan_values_returns_rows_in_the_declared_order_and_each_cursor_stands_for_the_last_key_of_its_batch`,
  `redb.rs::the_redb_store_declares_its_scan_order` (re-asserted for the data-directory store:
  it declares `PARTITION_THEN_KEY`),
  `engine_store_tests.rs::s28_a_write_behind_scan_over_the_engine_store_emits_a_key_at_a_partition_boundary_once`,
  `write_behind.rs::a_staged_key_at_a_batch_boundary_between_partitions_is_emitted_once`,
  `map_data_store.rs::key_bytes_order_positions_are_the_key_bytes_and_reject_a_cursor_that_is_not_utf8`,
  `processors.rs::a_bounded_scan_returns_a_staged_non_resident_key_once`,
  `processors.rs::a_bounded_scan_returns_a_key_written_after_the_resident_snapshot_once`,
  `topgun_server.rs::the_store_the_binary_selects_for_redb_emits_a_staged_key_once_under_write_behind`.
- **Not claimed:** the scan is not a snapshot; "once" is about the key, not about the value at
  the start of the scan; a key written after its range was passed is not emitted by that scan; a
  key deleted during the scan; an inner store that does not declare a scan order —
  none of the shipped stores; the test double that does not is the one this row was measured
  against; a wrapper between the write-behind
  store and a store that declares a scan order and does not forward the declaration; order of
  rows inside a
  batch; backup scans; two interleaved scans; `enumerate_leaves`; the order of the shutdown drain
  being held by a test; a batch without rows that returns a cursor is not checked against
  clause 5 (TODO-856).

### TG-WB-009: A write-behind entry the store refuses is retried on a schedule and is never dropped

- **Scope:** the flush task of `WriteBehindDataStore` on a node that is not shutting down,
  whose flush task is alive and whose inner store has not reported itself broken
  (`TG-WB-010`): the flush pass — its selection, its one commit and the halving of a refused
  commit — what it does with an entry the store refused, the store-wide hold and its probe.
  Below, `B = backoff_base_ms`, `K = backoff_cap_ms`, `I = flush_interval_ms`, and
  `L = 1000`, the most entries one pass takes; the numbers are for the defaults (500, 5000 and
  1000 ms) and for store calls that return promptly, unless a clause says otherwise.
- **Precondition:** one writer per `(map, key)` at a time (`TG-KEY-001`).
- **Statement:**
  1. **Nothing is dropped.** An entry the store refuses goes back to its queue with its staging
     slot, its count, its entry sequence pending and its WAL sequences `Live`, and is attempted
     again; the number of attempts is not limited. No branch of the flush task clears a staging
     slot, lowers the count, resolves an entry sequence or abandons a WAL sequence for an entry
     the store refused. The only other outcome of a put-back is the retirement into a queued
     newer write of the same key (`TG-WB-005`), which takes over the retry state.
  2. **The schedule.** After its n-th refusal on its own — alone in a commit, by a record-level
     refusal, or as a refused probe — an entry is not attempted again for `min(B << n, K)`,
     counted from the return of the refused call: 1000, 2000, 4000, 5000, 5000 … ms. The
     arithmetic saturates: the wait stays at `K` for any number of refusals. The wait belongs to
     the key, not to one write of it: a write that replaces the queued entry inherits it. A pass
     takes up to `L` entries or 8 MiB of encoded values, whichever comes first (the first entry
     taken is admitted whole whatever its size), with one slot reserved for the oldest due
     refused entry: when any refused entry is due, the one whose time came longest ago is taken
     first. The rest are taken in pass order — those never refused, in write order, then those
     refused, the one whose time came longest ago first — and the commit is written in that
     order, the reserved entry at its own place among the refused ones. A pass never waits on
     behalf of an entry. While entries are due and the store is neither held nor broken, passes
     run back to back without the interval sleep. Below the limits a pass takes every due entry.
  3. **The store-wide hold.** A pass commits its entries in one store call. A refused commit of
     one entry refuses that entry alone (statement 2). A refused commit of two or more entries
     is split into two halves, committed one after the other in the same pass: when one half is
     refused, the accepted half is stored and the refused half is split again; when **both
     halves** are refused, the store refuses, not a record, and a hold starts: every entry of
     the commit not yet stored goes back untouched (no backoff, no first-refusal time), the
     entries the pass did not take stay queued, and until the hold's time has passed no pass
     takes anything from a queue or calls the store, whoever woke it. Calls before a hold:
     exactly 3 (the commit and its two halves). Then a pass takes ONE due entry, the probe: the
     newest entry never refused, or, when none is due, the refused entry whose time came
     longest ago. Accepted: the hold ends and passes go on over everything that is due.
     Refused: the hold is renewed, longer. Hold lengths are `min(B << k, K)` after the k-th
     failed probe in a row, counted from the return of the refused call: 500, 1000, 2000, 4000,
     5000, 5000 … ms. A new hold always starts at the first length. **One exception:** the
     first refused commit of two or more entries after an accepted probe is split to single
     entries — both halves refused there start no hold; every refused part is halved until it
     is one entry, which is refused alone. Only once that commit is resolved may a refused
     commit start a hold again. The state is not armed at process start, and a hold that starts
     by any other path disarms it.
  4. **Bounds that follow.** Notation for the pass bounds: `m` = the least number of
     never-refused entries a pass takes when that many are due —
     `m = min(L − 1, ⌊(8 MiB − v_max) / v_max⌋)` for encoded values of at most `v_max` bytes, so
     `m = 999` for values up to ~8 KiB and `m ≥ 1` while `v_max ≤ 4 MiB`; `C_max` = the longest
     commit; `C_pass` = the longest pass, `≤ (1 + 2⌈log₂ L⌉) · C_max` when it meets one refused
     record. Hold time is excluded from every pass count below.
     - *One refused record in a pass of n entries:* at most `2⌈log₂ n⌉` extra commits; the
       other `n − 1` are stored in that pass, and the refused one goes on the schedule of
       statement 2 — n = 1000 → at most 20 extra.
     - *k refused records:* at most `2k⌈log₂ n⌉` extra commits **as long as no two refused
       halves meet**; when they do, a hold starts (statement 6). Proven by test for k = 1 and
       for the meeting case; the general formula is **not proven** by a test.
     - *A store that refuses everything is left alone.* The flush task makes at most
       `3 + ⌈log₂(K / B)⌉ + 1 + D / K` commits in any stretch `D` that begins with the first
       refusal and contains no accepted probe — `8 + D / 5 s`, 20 in 60 s — whatever the number
       of buffered entries. When a refused call takes `t_r` to return, calls do not overlap and
       the last term is `D / (K + t_r)`: a store that is slow to refuse is still left alone for
       a whole hold after each refusal.
     - *A store that goes down again right after an accepted probe:* the first refused commit,
       of `n ≤ L` entries, costs at most `2n − 1` commits (a full halving tree) and starts no
       hold; its entries take the per-entry schedule; the next refused commit of two or more
       entries starts a hold after 3 calls. So a stretch `D` that begins with that refusal costs
       at most `2L − 1 + 8 + D / 5 s` commits — 2 019 in 60 s with `L = 1000`. The state stays
       armed until the next refused commit, however long that is. For a store that flaps —
       accepted probes recurring — each accepted probe adds at most `2L − 1`; a bound over
       several flaps is **not proven** by a test.
     - *When the store returns at `T`, with no write after `T`:* every entry buffered at `T` is
       in a pass that begins no later than `T + K + I + (⌈N_T / m⌉ − 1) · C_pass`, `N_T` =
       entries buffered at `T` — `T + 6000 ms` while they fit in one pass. With writes after
       `T` the never-refused ones among them precede refused entries; the refused-entry bound
       below applies to those. `C_max` itself is **not bounded** by this store.
     - *Order.* Entries of one partition may be stored in any order; the WAL watermark and the
       flushed watermark stay at or below the lowest unresolved sequence. When the store
       accepts a key again, the last value it receives for the key is the key's newest buffered
       value, and it receives no older value of the key after a newer one.
     - *A new healthy key `H` due at `t0`* is attempted in one of the first `⌈(Q_H + 1) / m⌉`
       passes that begin after `t0`, `Q_H` = never-refused due entries ahead of it in
       `(store_time, sequence)` at `t0` (later writes sort after it, except a rewrite of a key
       already ahead, which keeps that key's place) — **provided `store_time` does not decrease
       across writes**. Production passes the wall clock, which can step back, and `store_time`
       is caller-supplied; after a backward step a later write can sort ahead of `H`, and the
       bound is **not proven** then. During a hold, within `K + I` after it is due — 6000 ms.
       Refused records do not enter `Q_H`: they take the reserved slot and leftover capacity
       only. For `v_max > 4 MiB` (`m` may be 0 in a pass whose reserved entry is large) the
       bound is **not proven**.
     - *A refused entry `T` due at `t0`* is attempted in one of the first `R_T + 1` passes that
       begin after `t0`, outside holds, `R_T` = due refused entries ahead of `T` in
       `(next_attempt, sequence)` at `t0` — whatever the load of new writes, because the
       reserved slot takes the oldest due refused entry in every pass and an entry refused
       again gets a later `next_attempt` (behind `T`), so the set ahead of `T` only shrinks. In
       time: within `(R_T + 1) · (C_pass + I)` of `t0` plus hold time — `I` because a run of
       passes ends after a pass that committed nothing (for example every entry refused alone),
       and the loop then sleeps the interval. During holds one probe per hold applies: with `P`
       records refused for ever, a refused healthy key is attempted after at most `P` refused
       probes, at most `(P + 1) · (K + I)` — 186 s for `P = 30`; linear in `P`, not a constant.
  5. **What is not bounded — part of the statement, not a footnote.**
     Without classifying store errors there is no time by which every buffered key reaches the store while some records are refused for ever.
     A healthy key written before `N` refused keys that have not been tried yet, with no newer
     healthy write, waits up to one hold for each of them, when the hold rule takes them for a
     store that is down (statement 6); so does a refused healthy key for every never-refused
     entry that is due. Two or more records the engine itself refuses (not caught as a
     record-level refusal before the commit) in different halves of a pass make the store look
     down once: one hold starts; after its accepted probe the next refused commit is split to
     single entries, which isolates every such record of that commit to the per-entry schedule
     and stores the rest in the same pass. Bound for that case: one hold cycle plus at most
     `2n − 1` commits for the split commit of `n` entries; then the isolated records are retried
     alone (statement 2). Not proven: a stream of new engine-refused records arriving while
     isolated ones are still retried — each new pair may cost one more hold cycle. No such
     record-level engine refusal is known for redb (TODO-824); the known record-level errors are
     refused per entry before the commit. Not bounded either: the time to store a healthy key
     when encoded values exceed 4 MiB (statement 4). A refused entry is **not** starved by
     sustained load: the reserved slot. **The cost of the reserved slot:** a pass whose
     reserved entry is a record the store keeps refusing pays one halving, at most
     `2⌈log₂ n⌉` extra commits; with `P` such records under sustained load each is due once per
     `K`, so at most `P` halved passes per `K` — `P · 2⌈log₂ L⌉ = 20 P` extra commits per 5 s.
     Not a defect, but a few tens of them can exceed the flush budget; not bounded further.
  6. **The hold has a false positive.** Both halves of a refused commit refused start a
     store-wide hold also on a healthy store when two of its particular records are refused, one
     in each half: the hold is a heuristic for "the store is down", and such records look the
     same to it. Two refused records anywhere in a pass start a hold even when everything else
     in it is stored; only a pass with exactly one refused entry gets through without one.
  7. **Records the store never accepts fill the buffer.** Records the store never accepts stay
     in the buffer, and once they fill it the node refuses writes of new keys until an operator
     intervenes.
- **The known cost.** A refused entry keeps its entry sequence pending, so the node-wide flushed
  watermark stops behind it and no tombstone is pruned on any map of the node. For a store that
  returns, this is the correct behaviour: the fence must not pass a tombstone the store did not
  get. For a record the store never accepts it lasts until the key is rewritten or the node
  restarts (TODO-828, TODO-824).
- **Maintaining code:** `write_behind.rs` — `run_flush_pass`, `select_pass` (the selection with
  the reserved slot and the pass limits), `encode_put`, `commit_prepared` (rows planned per
  commit), `settle_batch`, `commit_resolving_records`, `commit_bisect` (the halving, the
  both-halves rule), `start_hold`, the `split_to_singles` state of `RetrySchedule`,
  `requeue_refused`, `put_back`, `settle_stored`, `choose_probe`, `retry_backoff`,
  `flush_loop` (passes back to back), `report_retries`; `PartitionQueue::best_due`, `take`;
  `DelayedEntry::pass_order`, `inherit_from`; `PartitionQueue::retire_into_queued` (the retry
  state of the retired entry).
- **Enforcing test:** `write_behind.rs::a_refused_entry_stays_buffered_and_holds_the_flushed_watermark`,
  `write_behind.rs::an_entry_refused_for_longer_than_the_old_ladder_is_stored_when_the_store_returns`,
  `write_behind.rs::a_pass_over_refused_entries_ends_without_a_timer_wait`,
  `write_behind.rs::one_refused_entry_costs_one_store_call_per_backoff_and_delays_no_other_key`,
  `write_behind.rs::a_rewritten_refused_key_keeps_its_backoff`,
  `write_behind.rs::a_superseded_entry_hands_its_backoff_to_the_queued_write`,
  `write_behind.rs::a_store_that_refuses_everything_gets_a_bounded_number_of_calls`,
  `write_behind.rs::a_hold_makes_no_store_call_even_when_a_flush_is_requested`,
  `write_behind.rs::after_a_refusal_longer_than_the_old_ladder_the_buffer_drains_within_one_backoff_cap_and_one_interval`,
  `write_behind.rs::keys_of_one_partition_are_stored_out_of_write_order_and_the_watermarks_follow`,
  `write_behind.rs::a_refused_cell_entry_persists_what_the_cell_holds_when_it_is_accepted`,
  `write_behind.rs::healthy_keys_written_after_refused_ones_are_never_starved`,
  `write_behind.rs::a_refused_healthy_key_is_reached_after_at_most_one_probe_per_rejected_key`,
  `write_behind.rs::a_store_that_refuses_slowly_is_called_once_at_a_time_and_left_alone_for_a_hold`,
  `write_behind.rs::never_refused_entries_are_stored_before_refused_ones_in_a_pass`,
  `write_behind.rs::refused_entries_are_attempted_longest_waiting_first`,
  `write_behind.rs::entries_put_back_by_a_hold_are_accounted_like_a_retry`,
  `write_behind.rs::an_accepted_probe_ends_the_hold_and_the_next_trip_starts_at_the_first_length`,
  `write_behind.rs::one_refused_record_among_healthy_ones_starts_no_hold`,
  `write_behind.rs::a_probe_does_not_take_an_entry_whose_time_has_not_come`,
  `write_behind.rs::a_probe_whose_key_was_rewritten_before_it_was_taken_stores_the_new_value_in_that_pass`,
  `write_behind.rs::an_accepted_probe_is_followed_by_a_normal_pass_over_what_is_due`,
  `write_behind.rs::the_probe_is_the_never_refused_entry_with_the_latest_store_time_whatever_its_sequence`,
  `write_behind.rs::an_entry_refused_a_hundred_times_backs_off_at_the_cap_and_the_flush_task_lives`,
  `write_behind.rs::a_refusal_count_at_its_maximum_stays_there`, and the nineteen tests of
  `write_behind_batch_tests.rs` over a counting engine on redb:
  `b01_a_pass_is_one_durable_engine_write_holding_its_entries_and_rows`,
  `b02_a_pass_takes_at_most_1000_entries_or_8_mib_and_the_first_entry_whole`,
  `b03_passes_run_back_to_back_while_entries_are_due`,
  `b04_one_refused_record_is_isolated_with_at_most_2_log2_n_extra_commits`,
  `b05_a_store_that_refuses_every_commit_starts_a_hold_after_three_calls`,
  `b06_refused_records_in_both_halves_start_a_hold_the_documented_false_positive`,
  `b07_record_level_refusals_are_handled_per_entry_without_halving`,
  `b08_a_broken_store_in_a_pass_resolves_nothing_and_calls_no_more`,
  `b09_the_shutdown_drain_commits_in_passes_with_rows_and_stops_at_its_deadline`,
  `b10_a_refused_commit_leaves_staging_counts_and_both_watermarks_unchanged`,
  `b11_after_the_store_returns_every_buffered_entry_is_attempted_within_the_formula`,
  `b12_no_pass_commit_is_non_durable`,
  `b13_after_an_accepted_probe_two_records_refused_in_different_halves_are_isolated_without_a_new_hold`,
  `b14_a_store_that_goes_down_right_after_an_accepted_probe_splits_once_then_holds`,
  `b15_a_half_committed_after_its_sibling_plans_its_rows_after_the_sibling_settled`,
  `b16_a_recommit_never_marks_a_refused_entry_applied`,
  `b17_a_refused_entry_is_attempted_under_sustained_load`,
  `b18_a_new_healthy_key_waits_at_most_its_queue_position_in_passes` and
  `b19_a_run_of_passes_yields_and_stops_at_the_shutdown_signal`. All run in CI (`cargo test`).
  Which test holds which bound: one refused record, b04 (n = 1000, at most 21 writes); the
  meeting case, b06; the store that refuses everything, `a_store_that_refuses_everything_gets_a_bounded_number_of_calls`
  and b05; the store that goes down right after an accepted probe, b14 (n = 1000: at most
  1 999 commits and no hold, then a hold after exactly 3 more); the store that returns, b11; a
  new healthy key, `healthy_keys_written_after_refused_ones_are_never_starved`, b04 and b18; a
  refused entry under sustained load, b17; the split to single entries after an accepted probe,
  b13. At process level, over redb with an injected refusal:
  `tests/integration-rust/store-refusal-liveness.test.ts` — the refused write stays readable, a
  retry warning is logged, the write reaches the store once the store accepts again, and it is
  read back from a second process after a SIGTERM during the refusal. That file is local only
  (`pnpm test:integration-rust:liveness`), for the reason given under `TG-WB-004`; a CI job that
  runs it is TODO-832.
- **Violation consequence:** an acknowledged write is lost on a running node. When the flush
  gave up on a refused entry, the entry's staging slot went with it, and a read of the key was
  then answered from the inner store, which never got the write: after an eviction an LWW write
  read as the previous value or as no record, a removed record came back at once, an OR add
  lost its tag and an OR remove its tombstone — and the next write of the key was built on that
  stale value and stored that way. Only a restart, replaying the WAL, brought the write back.
- **Discovered by:** a review of what a read returns after the flush gave up on an entry; fixed
  and catalogued by SPEC-390; restated for batched passes by SPEC-395b.
- **Status:** decided, **enforced** — within the limits listed under "Not claimed" below.

**Not claimed.**

- **A time by which every key reaches the store.** Statement 5. The only sign that a record is
  one the store rejects is a refused call, and a hold allows a fixed number of calls per
  period, so `P` such records cost at least `P` periods under any order of attempts. Measured,
  before passes were batched, for a never-refused healthy key written before `N` refused keys
  not tried yet, with no newer healthy write: 7.5 s for `N = 3`, 142.5 s for `N = 30`, 442.5 s
  for `N = 90` — in the order of 14 hours for a full default buffer of 10 000. A known cost; no
  test pins it. Classifying store errors and parking such records is TODO-824.
- **A store that never accepts a record.** The entry is retried for the life of the process and
  holds one buffer place and its partition's WAL watermark. Nothing parks or classifies it
  (TODO-824). A redb store whose error is an I/O error, a previous I/O error, corruption or a
  poisoned lock is not retried this way: it reports itself broken and the node stops
  (`TG-WB-010`).
- **Tombstone pruning while an entry is refused.** The known cost above. How long it has lasted
  is read from `topgun_write_behind_oldest_refusal_age_seconds` as a lower bound: the fence has
  stood for at least that long, and can have stopped earlier, at the write of the entry. How
  many tombstones wait has no series (TODO-828).
- **Two or more refused records due in one pass.** They start a hold (statement 6); a healthy
  key then waits for the probe, up to `K + I`. "Stored in that pass" is stated for one refused
  record in a pass.
- **`TOPGUN_WRITEBEHIND_CAPACITY=0` with a store that refuses** is not supported. With no
  capacity nothing stops the arrival of records the store always refuses, each is newer than a
  waiting healthy key, and there is then no bound at all on when that key is stored; memory
  grows without limit.
- **A store call that does not return promptly.** Calls of the flush task do not overlap and
  each hold is counted from the return of the refused call. No timeout is added around the
  store call: a call that never returns stops the flush task for as long as it hangs. The
  commit runs synchronously on the runtime worker (TODO-865).
- **Atomicity of the halving on other stores.** The halving reasons about whole commits; it is
  exact for the engine store, whose commit is one engine write. The default commit of other
  stores (test doubles) writes entry by entry and stops at its first failing write: a refused
  half may already have written its earlier entries, which a later commit writes again —
  idempotent for the doubles used here.
- **The shutdown drain against a store that is down.** The drain runs passes of the same limits
  over every queued entry, each a commit with its watermark rows; the hold does not apply to
  it. A refused drain commit of two or more entries is halved down to single entries, commit by
  commit, for as long as the drain deadline allows (checked before every commit); a single
  entry refused is abandoned, the rest of its pass goes on, and every entry not stored when the
  deadline passes is abandoned too — its frame replays at the next start on a WAL-backed store.
  A drain pass of `n` entries makes at most `2n − 1` commits. A commit that has started is not
  cut on the engine store, so a stop may overrun its deadline by one commit. On a redb store
  that reports itself broken the drain stops at its first such refusal and abandons the rest
  without calling the store (`TG-WB-010`).
- **A frame the store refuses at boot.** WAL recovery logs the refusal and continues; the frame
  is not handed to this schedule and is not retried until the next restart (TODO-823).
- **How many times the store is called for one key.** A key written again while its entry is
  held by a pass is stored twice when the held entry is accepted; an entry in a halved commit
  is in up to `1 + 2⌈log₂ n⌉` of its commits. The statement gives the order, not the count.
- **WAL growth during an outage.** No frame is collected in a partition whose watermark is
  held; nothing measures or limits the growth (TODO-757).
- **The stall alarm during an outage.** Once a partition's stall has outlasted
  `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS`, the watchdog reports it at `ERROR` on every tick for
  every partition with a refused entry and ongoing writes, for the rest of the outage. It is
  not rate-limited (TODO-757).
- **Two writers of one key.** Outside the precondition. A caller that writes the write-behind
  store directly — `DeviceIdentityStore` — is outside `TG-KEY-001` and has to exclude a second
  writer of its key itself.
- **redb.** The tests in `write_behind.rs` use a test double as the inner store; the batch tests
  use redb through a counting engine that injects the refusals; the process-level test uses
  redb with an injected refusal. No record-level refusal of redb itself is known.
- **The crash harness.** Its model still describes a flush that gives up; its proofs of
  recovery after a crash are proofs about that model and are not evidence for this schedule
  (TODO-829).
- **Simulation.** A simulation cluster node has no write-behind store and no store fault, so no
  simulation test exercises this row (TODO-830).
- **Performance.** Not measured here; the measurement of the batched pass is SPEC-395c R5.
- **The client.** Nothing is sent to a client about a refused entry.

### TG-WB-010: A write-behind store whose inner store reports itself broken fails like a store whose background task panicked: new writes are refused, nothing it holds is resolved, and the server binary exits 72

- **Scope:** `WriteBehindDataStore` — the refusal branch of its flush task (a pass commit, each
  half of a bisect, a re-commit after a record-level refusal, and a probe) and of its shutdown
  drain, its failure record and lifecycle state, `hard_flush`;
  `MapDataStore::classify_error` and its one override, `EngineMapDataStore::classify_error`
  (the redb store, `RedbDataStore`, is `EngineMapDataStore` over `RedbKvEngine`); and the
  shutdown path of the server binary (`bin/topgun_server.rs`). The store half holds for any
  embedder; the exit code is a property of the server binary only.
- **Statement:**
  1. **Classification is by type.** An inner-store error is classified by
     `MapDataStore::classify_error`, whose default answer is `Unknown`. Only the redb store
     overrides it (its engine maps the redb error to `KvError::Broken { cause }`, and the
     store's `classify_error` reads that variant): it returns `StoreBroken(cause)` for a
     `redb::StorageError` — bare, or inside
     `CommitError::Storage`, `TransactionError::Storage` or `TableError::Storage` — that is one
     of four causes: `Io` (with the `io::ErrorKind`, so a full disk shows as `StorageFull`),
     `PreviousIo` (the handle has latched after an earlier I/O error), `Corrupted` and
     `LockPoisoned`. The answer is decided from the error's type, never from its message. Every
     other error — `ValueTooLarge`, `TableTypeMismatch`, the map-name refusal, any non-redb error
     — is `Unknown`. `StoreBroken` means an error the store treats as fatal for this process; it
     is not a proof that no later call could succeed (see "Not claimed").
  2. **The reaction is the failed state of `TG-WB-006`.** When an inner-store call of the flush
     task or of the shutdown drain returns an error classified `StoreBroken`, the store, in one
     synchronous stretch with no await, records the failure (cause `StoreBroken`, task `Flush`),
     enters the failed state, logs one error line and counts it. From then on `add`, `remove`
     and `remove_all` return an error before a WAL sequence is assigned and before anything is
     appended to the WAL, and `hard_flush` returns an error whose source is the recorded
     failure, whatever started the stop.
  3. **Nothing it holds is resolved.** On a `StoreBroken` commit every entry of the pass not
     stored goes back to its queue **untouched** — no retry count, no backoff, no first-refusal
     time: a broken store says nothing about a record, so no entry is singled out for a retry
     state. The error line names the first entry of the failing commit. The drain abandons
     every entry it has not stored; their WAL sequences are abandoned, never resolved. On a
     WAL-backed store every acknowledged write the store did not take stays in the unapplied
     window and is replayed at the next start.
  4. **The flush task stops calling the store.** After the failure no later pass makes an inner
     call; the flush task still exits on the shutdown signal. The shutdown drain calls the store
     at most once more: it stops at its first `StoreBroken` refusal.
  5. **Once per process.** A second `StoreBroken` classification — the drain after the flush
     task — adds no line and no count. **The first recorded failure wins:** the record is set
     only if empty, so a broken store followed by a watchdog panic exits 72 (with the panic line
     also in the log), and a panic followed by a broken store exits 70 (with the store-broken
     line and count also there).
  6. **The server binary exits 72.** It reports not ready, runs the shutdown of `TG-WB-006`, and
     ends with exit code 72 when that shutdown completed, 71 when it panicked or passed its
     deadline. The kind is read from the recorded failure after `hard_flush` returned, so a stop
     begun by a signal on a broken store also exits 72.
  7. **`Unknown` follows `TG-WB-009`.** An error classified `Unknown` is handled by
     `TG-WB-009`: in the flush task by the halving of the refused commit, the store-wide hold
     and the per-entry schedule; in the shutdown drain by the split to single entries until the
     drain deadline.
- **Mechanical predicate:** on a write-behind store over a `RedbDataStore` whose backend has
  returned an I/O error: (a) once the flush task's call that returned the error has returned,
  the next `add`, `remove`, `remove_all` returns the store-broken sentence and the WAL saw no
  append for it; (b) `background_failure()` holds `Some` with `store_broken() == true`; (c) the
  flush task makes no further inner call; (d) `hard_flush` calls the store at most once and
  returns `Err`; (e) every WAL frame of every entry it did not store is still in
  `wal.unapplied(p)`; (f) `exit_code_for(StoreBroken, Completed) == 72`; (g) a store reopened
  over the same bytes with the backend healed, after `WalRecovery`, holds every acknowledged
  value.
- **Maintaining code:** `map_data_store.rs` — `StoreErrorClass`, `StoreBrokenCause`,
  `MapDataStore::classify_error`. `redb.rs` — `broken_cause`, the `fault_io` backend
  (`fault-injection` builds and tests only). `redb_engine.rs` — the error mapping
  (`kv_from_storage`, `kv_from_commit`, `kv_from_transaction`, `kv_from_table`) that turns the
  four causes into `KvError::Broken`. `engine_store.rs` — `classify_error`, and the
  registry-read rule: a registry read that fails is an error, never "no such map", so a remove
  is not reported done on a broken store. `write_behind.rs` — `run_flush_pass`, `commit_resolving_records` and `commit_bisect` (a
  `StoreBroken` commit puts every entry of the pass it did not store back with `put_back`,
  untouched, and is never halved), `flush_loop` (a broken store ends the run of passes for good)
  and the `store_broken` flag of `RetrySchedule`, `record_store_broken` (record, state, line, counter in that order),
  `failed_reason`, the refusal arm of `drain_for_shutdown`, `finish_shutdown`,
  `BackgroundFailure::{store_broken, cause_label}`. `bin/topgun_server.rs` — `exit_code_for`,
  `FailureKind`, `failure_kind_after_drain`, `log_store_failure`.
- **Enforcing test:** `redb.rs::a_latched_redb_store_fails_the_write_behind_store` — over a
  redb store whose backend returned ENOSPC once, the next write is refused, the failure is
  recorded and `hard_flush` returns an error. It runs in CI (`cargo test`). The table below
  lists the others and what each one covers.
- **Violation consequence:** a node whose redb handle has latched acknowledges writes (their
  frames are in the WAL) and stores none of them, retries each for the life of the process,
  holds the node-wide tombstone fence and fills its buffer until writes of new keys are refused
  — until something restarts the process, which nothing does. On a stop it reports a clean exit
  0. An acknowledged remove could be reported stored when the catalog could not be read, so its
  frame was resolved without the delete being stored.
- **Visible to an operator:** one error line per process,
  `write-behind store: the durable store is broken and stores nothing more in this process; writes are refused from now on and the node stops`,
  with fields `cause`, `io_kind` (for `io` only), `error`, `map`, `key` and `site`
  (`flush` | `shutdown_drain`); when the drain stops, one line
  `write-behind shutdown drain stopped at a broken store: the entries it did not store stay un-resolved and replay from the WAL at the next start`
  with `abandoned_entries` and `cause`; the binary's line
  `the durable store is broken; shutting the node down` with `cause` and `exit_code`; the
  counter `topgun_write_behind_store_broken_total{cause}` (`io`, `previous_io`, `corrupted`,
  `lock_poisoned`, each rendered at 0 from the first scrape); a refused write's sentence
  `write-behind store has failed (the durable store is broken); write rejected for …`;
  `/health/ready` and `/healthz` at 503 while the stop runs; exit code 72.
- **Discovered by:** a review of what the retry-until-stored schedule of `TG-WB-009` does on a
  redb handle that refuses every call after its first I/O error; fixed and catalogued by
  SPEC-391.
- **Status:** decided, **enforced** — within the limits listed under "Not claimed" below.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `a_latched_redb_store_fails_the_write_behind_store` (`redb.rs`) | predicate (a), (b) and (d)'s `Err`, over redb with ENOSPC injected below it: after the refused flush the next write is refused, the record holds a failure, `hard_flush` returns `Err` | CI (`cargo test`) |
| `a_shutdown_drain_on_a_latched_redb_store_reports_a_failure` (`redb.rs`) | the drain alone meets the broken store: `hard_flush` returns `Err` and the failure is recorded, so a stop begun by a signal is a failure | CI (`cargo test`) |
| `a_remove_on_a_store_that_cannot_read_its_catalog_is_not_reported_done`, `a_remove_all_on_a_store_that_cannot_read_its_catalog_is_not_reported_done` (`redb.rs`) | a registry read that fails is an error on both remove paths, not "no such map, nothing to delete" (the test names keep the word "catalog" of the table-per-map store) | CI (`cargo test`) |
| `t_classify_by_type` (`redb.rs`) | each of the four causes in each wrapper, also under `.context(..)`, is `StoreBroken` with its cause; `ValueTooLarge`, the map-name refusal, a real `TableTypeMismatch`, the fault-injection refusal string and a bare `std::io::Error` are `Unknown` | CI (`cargo test`) |
| `t_previous_io_through_the_write_behind_flush` (`redb.rs`) | a handle that latched before the write-behind store saw it: the first flush meets `PreviousIo` through `TransactionError` and the store fails with cause `previous_io` | CI (`cargo test`) |
| `t_restart_heals` (`redb.rs`) | predicate (e) and (g): on a real WAL the un-stored frames stay unapplied; reopened over the same bytes with the backend healed and recovered from the WAL, every acknowledged value is there and a removed key is absent | CI (`cargo test`) |
| `the_sentinel_backend_refuses_writes_while_the_file_exists` (`redb.rs`) | the fault backend the process-level test uses: a commit passes without the sentinel file and fails with ENOSPC (`os error 28`) while it exists | CI (`cargo test`) |
| `t_no_more_calls_after_a_broken_store` (`write_behind.rs`) | predicate (c): one pass makes one inner call; every entry is back untouched, none lost; two more passes make no call | CI (`cargo test`) |
| `b08_a_broken_store_in_a_pass_resolves_nothing_and_calls_no_more` (`write_behind_batch_tests.rs`) | after `Broken` the flush task starts no pass and makes no inner call, with entries still due, and the test ends | CI (`cargo test`) |
| `t_drain_one_call_on_a_broken_store` (`write_behind.rs`) | predicate (d) and (e) for the drain: one inner call, `Err` with a store-broken source, every entry's frames abandoned and none resolved, the drain line with `abandoned_entries` | CI (`cargo test`) |
| `t_unknown_unchanged` (`write_behind.rs`) | an `Unknown` refusal: a hold after the commit and its two halves with every entry back untouched, then a drain that splits to single entries and abandons the four, no record, `hard_flush` `Ok` | CI (`cargo test`) |
| `t_once_per_process` (`write_behind.rs`) | broken in a pass, then again in the drain: one line, count 1 | CI (`cargo test`) |
| `t_metric_broken` (`write_behind.rs`) | the four series at 0 after construction; the failure's cause at 1; the panic counter unchanged | CI (`cargo test`) |
| `t_refuse_broken` (`write_behind.rs`) | each write entry returns the store-broken sentence, the WAL saw no append, the pending count is unchanged | CI (`cargo test`) |
| `t_read_after_break` (`write_behind.rs`) | after the failure a read of an acknowledged key answers its value from the slot, with no inner call | CI (`cargo test`) |
| `a_panicked_store_shutdown_keeps_its_context_text` (`write_behind.rs`) | the panic texts of `TG-WB-006` are not changed by the new cause: `hard_flush`'s context after a panic stays byte for byte | CI (`cargo test`) |
| `bin_broken_exit_code_for_is_total`, `bin_broken_signal_then_a_broken_store_in_the_record_exits_72` (`bin/topgun_server.rs`) | predicate (f): `exit_code_for` over every kind and shutdown outcome; on the signal path a store-broken record read after `hard_flush` gives 72, a panicked one 70 | CI (`cargo test`) |
| `tests/integration-rust/store-broken.test.ts` | the CONSEQUENCE at process level on a `fault-injection` build, ENOSPC injected below redb by a sentinel file, a client connection open throughout: the process exits 72 by itself with the three lines; after a restart with the disk healed every acknowledged key reads back; a restart while the disk is still full exits 1 at the store open, and a third start with room reads every key back | local only (`pnpm test:integration-rust:liveness`) — opt-in, not run in CI, for the reason given under `TG-WB-004` (TODO-770) |

**Not claimed.** Each of these is outside the statement, or is in it and rests on something
weaker than a test.

- **The read path.** `load`, `load_slot`, `load_all`, scans and `list_maps` of an uncached page
  on a latched handle return the error to their caller, per request; they do not fail the node.
  A latched node that receives no write is not stopped until a write reaches the flush
  (TODO-838).
- **The backup pass-throughs** `add_backup` / `remove_backup` return the error to their caller
  and are not classified.
- **Boot recovery.** A store-broken refusal during WAL replay is logged and the boot continues;
  the node starts on a latched handle and stops at its first flush (TODO-823).
- **A crash loop** on a corrupted file or a still-full disk is not detected or delayed by the
  process; the supervisor paces it. On a still-full disk it is a loop of exit 1 at the store
  open.
- **One corrupted page stops the node**, although redb raises `Corrupted` per page and the other
  pages still read. Likewise a `LockPoisoned` that came from a per-transaction lock after a
  caught panic stops it. The class is "fatal for this process" by decision, not a proof that no
  later call could succeed.
- **A full disk is injected, not real.** ENOSPC comes from a wrapper below redb; no run used a
  real full filesystem. In particular exit 134 — the WAL append aborting the process on a really
  full disk shared by the WAL (`TG-WAL-007`) — is not shown: the injection never reaches the
  WAL.
- **`io_kind`** is visible on the first error only; a `PreviousIo` record has none.
- **The metric** may not be scraped before the process exits.
- **The per-entry drain `warn!` lines** for abandoned entries are still one per entry, so a
  large buffer logs one line per entry; on a broken store each names the broken store and its
  `cause` instead of a timeout.
- **`ValueTooLarge`, `TableTypeMismatch`, the map-name refusal** are `Unknown` here (TODO-824).
- **A panic inside redb** (its own `.lock().unwrap()`) is the `TG-WB-006` path, not this one.
- **The window.** A write that loaded the lifecycle state before the record was made is
  acknowledged: its frame is in the WAL; the drain abandons it; it replays.
- **A store without a WAL** (`WriteBehindDataStore::new`): abandoned entries are lost at the
  exit, as under `TG-WB-006`.
- **A latched handle with nothing queued at SIGTERM exits 0.** The drain makes no store call, so
  it never meets the latch; the node stops clean and the next start opens a fresh handle.
  Consistent with the read path being out (TODO-838).
- **The order "record before state"** inside `record_store_broken` is pinned by reading the
  code, not by a test or a mutant: no test can place a write between two synchronous
  statements of one thread.
- **The order of two failures in one process** (the first record wins) is pinned by reading;
  no test drives a broken store and a panic in the same process.

### TG-EVI-001: An acknowledged write the store has not taken is readable on a running node

- **Scope:** `RecordStore::get` and `MapDataStore::load` / `load_slot` over a write-behind store
  whose flush task is alive; and, for the second statement, `evict_lru` in the record store.
- **Statement:**
  1. From the ack until the inner store has accepted the write, a read of the key returns that
     write — from the resident record, or, when the record is not resident, from the key's
     staging slot. Eviction does not change the answer.
  2. The narrower rule this row used to be, kept: `evict_lru` never removes a record with
     `is_dirty() == true`, where dirty means "`data_store.add` has not returned". Over a
     write-behind store `add` returns once the write is buffered, so a record is clean, and
     evictable, while its write is still on its way to the inner store; that is why statement
     1 rests on the staging slot and not on this rule.
- **Maintaining code:** for statement 1, `write_behind.rs` — `stage` before the queue insert
  (`TG-WB-007`); `requeue_refused`, which keeps the slot of an entry the store refused
  (`TG-WB-009`); `load` / `load_slot`, which answer from the slot before they ask the inner
  store. For statement 2, `storage/impls/default_record_store.rs` dirty-skip;
  `RecordMetadata::is_dirty` is exact by write token (`stored_token != write_token`, set by
  `on_store`), not by millisecond, so a write stamped in the same millisecond as the previous
  persist stays dirty; `evict_lru` removes each snapshot candidate through
  `StorageEngine::remove_if`, re-checking under the key's lock that the resident is still clean
  and still the write the snapshot saw (SPEC-374).
- **Enforcing test:** statement 1 — `crdt.rs::an_lww_write_refused_ten_times_and_evicted_reads_the_acked_value`,
  `::a_first_lww_write_refused_ten_times_and_evicted_reads_the_acked_value`,
  `::an_lww_tombstone_refused_ten_times_and_evicted_stays_a_tombstone`,
  `::a_remove_refused_ten_times_stays_removed`,
  `::an_or_add_refused_ten_times_and_evicted_keeps_its_tag`,
  `::an_or_remove_refused_ten_times_and_evicted_keeps_its_tombstone`,
  `::a_write_of_a_key_under_retry_is_stored_with_the_refused_one`,
  `::an_or_remove_under_retry_survives_the_next_write_of_its_key`,
  `write_behind.rs::a_value_added_straight_to_the_store_is_loadable_while_refused` and
  `write_behind.rs::a_device_identity_is_presented_back_and_stays_revoked_while_the_store_refuses`.
  Statement 2 — `default_record_store.rs::evict_lru_skips_all_dirty_records` +
  `::evict_lru_skips_dirty_in_mixed_snapshot` + assertion in `eviction_cost_test.rs`;
  `::evict_lru_keeps_a_record_written_after_its_snapshot` (a write between the snapshot and the
  removal), `::a_write_in_the_mark_stored_millisecond_is_not_evicted` and
  `record.rs::a_write_in_the_stored_millisecond_is_dirty` (same-millisecond write).
- **Violation consequence:** a read returns a value older than an acknowledged write, or no
  record, on a node that is running; and for statement 2, eviction under pressure drops a
  write the store was never given.
- **Discovered by:** eviction design (pre-catalog) for statement 2; statement 1 by the review
  named under `TG-WB-009`, restated by SPEC-390.
- **Status:** decided, **enforced** — within the limits listed under "Not claimed" below.

**Not claimed.**

- **A frame the store refuses at boot.** The node serves the previous value until the next
  restart (TODO-823).
- **A value left in memory when `data_store.add` fails.** The record store writes memory before
  the store call, so a write that was not acknowledged can be read (TODO-825).
- **`evict_expired`.** It removes expired records without looking at the dirty flag; what a
  read then returns for a buffered write was not examined (TODO-827).
- **Other nodes.** Whether other nodes of a cluster see a different value for the length of
  one node's store outage was not examined (TODO-827).
- **A node whose flush task has died, or that is shutting down.** Outside the scope
  (`TG-WB-006`).
- **Memory.** A slot's memory is not returned by eviction, and the capacity counts keys, not
  bytes (`TG-WB-007`; TODO-826).

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

### TG-NAME-001: Map names are stored as their bytes in the map registry; the table-per-map names exist only in a legacy file, which the one-time migration decodes totally and injectively and copies without renaming

- **Scope:** `RedbDataStore` (`EngineMapDataStore` over `RedbKvEngine`) for every name the store
  accepts; the legacy table-name mapping `table_name_for` and its inverse `decode_table_name`,
  used only by the one-time migration of a table-per-map store file (`TG-ENG-007`).
- **Definitions:** `S` (storable) = names that are non-empty, do not end in `__backup` and contain
  no U+0000. `V` (identifier class) = members of `S` matching `^[a-zA-Z_][a-zA-Z0-9_]*$` — exactly
  the set the store accepted before map names left the identifier class. `W` = `S \ V`.
  `table(n, backup)`, the legacy mapping: for `n ∈ V` → `"map__"+n` / `"map__"+n+"__backup"`; for
  `n ∈ W` → `"mapr__"+n` / `"maprb__"+n`.
- **Statement:**
  1. *New format.* A map name is stored as its UTF-8 bytes in the map registry of the data
     directory's store (`TG-ENG-005`); no redb table carries it. The store still refuses every name
     outside `S` (`check_map_name`, applied by `EngineMapDataStore` before anything touches the
     engine). The reason this set is kept in the new format is **compatibility of the
     input contract** (`TG-NAME-002`): every client and every older store refuse the same names, so a
     name one of them accepts is accepted by all. It is **not** a table-name collision, which the
     registry no longer has.
  2. *Legacy decode.* A legacy table name decodes by exactly one of four rules: `mapr__X` →
     `(X, primary)`; `maprb__X` → `(X, backup)`; `map__X__backup` with `X ∈ V` → `(X, backup)`;
     `map__X` with `X ∈ V` not ending in `__backup` → `(X, primary)` — the four ranges
     `P_new = "mapr__"·W`, `B_new = "maprb__"·W`, `B_old = "map__"·V·"__backup"`,
     `P_old = "map__"·V` — and is accepted only if it round-trips: `table(X, backup) == name`. The
     ranges are pairwise disjoint (byte 3 is `_` in `P_old ∪ B_old` and `r` in `P_new ∪ B_new`;
     byte 4 separates `P_new` from `B_new`; `"map__"+a == "map__"+b+"__backup"` would need
     `a ∉ S`), so the decode is total on the image of `table` and injective. Any other table name,
     any multimap table and any table whose key/value types are not `&str`/`&[u8]` refuses the
     migration, and the legacy file is left as it was.
  3. *Nothing renamed.* After a migration `list_maps` equals the `list_maps` of the legacy file
     byte for byte, and every row's key and value bytes are preserved (values are copied verbatim,
     not decoded).
- **Maintaining code:** `packages/server-rust/src/storage/datastores/redb.rs` — the legacy
  decoder `decode_table_name`, `read_legacy_catalog`, `table_name_for` (refusal plus mapping in
  one call, so no legacy table name is obtainable for a name outside `S`) and the copy and
  verification of the migration; `engine_store.rs` — the map registry and `check_name`. Citations
  are kept line-number-free on purpose, per `TG-OR-004`.
- **Enforcing test:** `table_names_keep_the_old_class_and_prefix_the_new_class` (the four ranges,
  their disjointness and the sharp cases of the legacy mapping),
  `store_refused_names_reach_no_table_and_backups_stay_unlisted` and
  `map_names_outside_the_identifier_class_round_trip` — in `redb.rs`'s test module;
  `m02_a_legacy_file_is_migrated_byte_for_byte_and_kept_under_the_pre_migration_name`,
  `m03_list_maps_after_a_migration_equals_the_legacy_list_maps` and
  `m07_a_legacy_table_that_is_not_a_store_table_refuses_the_migration` — in
  `redb_migration_tests.rs`. What an older binary does on a migrated store is stated by
  `TG-ENG-007` and proven by the two-binary run `tests/integration-rust/map-name-rollback.test.ts`
  (the rollback rule in `CLAUDE.md`); it needs the older binary, so CI excludes it and it is run
  locally.
- **Violation consequence:** two legacy tables would be copied into one map, or a primary read as
  a backup and dropped from `list_maps`, so the migrated store serves Merkle root 0 for durable
  data; or a map is renamed by the migration and its data is unreachable after the upgrade; or a
  name one client accepts is refused by another store.
- **Discovered by:** the map-name durability investigation of TODO-751 (acked writes to
  `user-profiles` / `users/profiles` gone after a restart on redb); SPEC-381a; rewritten for the
  data-directory store by SPEC-394d.
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
  in `AuthorizationService::call`; for them the store's own check (TG-NAME-001, `check_map_name`
  in `EngineMapDataStore`) is the last line. (b) HTTP `/sync` **queries** do not pass through the pipeline; they read
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
- **How it is enforced:** in two parts. That every production call PASSES a writer token of
  its key is enforced by the type and by the store (`TG-KEY-002`): a call of a `RecordStore`
  mutator without a token does not compile, and a token acquired for another key is refused.
  That the token is taken EARLY ENOUGH — before the server stamp is minted and before the first
  read the write depends on — and held until the fan-out is done still rests on the tests below
  and on the position predicates: the number of acquire sites and their position relative to
  the stamp, the store call and the fan-out, re-run when a writer is added. "One registry" is a
  wiring predicate on the server binary — the registry is constructed once and handed to every
  writer — not a property of a type: a service constructed without the shared registry gets a
  private one.
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
- **Maintaining code:** `storage/key_writer.rs` (`KeyWriterRegistry`, `acquire`), re-exported at the old path;
  `service/domain/crdt.rs` — `apply_op_under_writer`, the one caller of `apply_single_op`, and the
  prune pass; `service/domain/sync.rs` — `handle_ormap_push_diff`;
  `service/domain/embedding/hook.rs` — `write_back_one_embedding`; `bin/topgun_server.rs`
  constructs the one registry and passes it to each of them.
- **Enforcing test:** `crdt.rs::two_routes_writing_one_key_leave_engine_and_store_on_the_later_stamp`
  — two writes of one key arriving by two routes, the first parked inside the store: memory and
  the durable store both end on the later stamp. Runs in CI (`cargo test`). The table below lists the others and what each one covers.
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
- **Status:** decided, **enforced** — that a call passes its key's token, by the type and the
  store (`TG-KEY-002`); that the token is taken early enough and held long enough, by the tests
  below and by position predicates.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `two_routes_writing_one_key_leave_engine_and_store_on_the_later_stamp` | two routes, the first parked inside the record store: memory and the durable store agree on the later stamp | CI (`cargo test`) |
| `two_routes_writing_one_key_leave_store_staging_and_restart_on_the_later_stamp` | the same with the first writer parked inside the write-behind store: the store, staging and a restart all give the later stamp | CI (`cargo test`) |
| `a_put_that_takes_the_writer_last_carries_the_higher_stamp` | the stamp is minted under the writer: a put parked in front of its acquire while another put of the key completes takes the higher stamp, is recorded second, and its value is the one memory and the durable store keep | CI (`cargo test`) |
| `a_lower_stamped_put_never_lands_after_a_higher_stamped_one` | the mint is not in front of the read of the previous value; the journal order of two puts of one key is their stamp order. It does not need the writer: it passes without one | CI (`cargo test`) |
| `two_routes_and_a_refused_flush_recover_the_later_stamp_after_a_crash` | two routes plus a refused flush: after a crash and recovery the recovered value is the later-stamped one and equals what the live server served | CI (`cargo test`) |
| `an_older_value_arriving_behind_a_refused_newer_one_does_not_win_after_a_crash` | a write parked in the read of the previous value while another sits in a refused flush: it mints after that read, so the later arrival is also the higher stamp, and the value recovered after a crash is the one the live server served. It does not need the writer: it passes without one | CI (`cargo test`) |
| `an_lww_put_waits_for_a_parked_or_push_of_the_same_key` | one registry for LWW and OR writers: an LWW put waits for an `ORMapPushDiff` of the same key | CI (`cargo test`) |
| `a_client_write_during_an_embedding_write_back_is_not_overwritten` | the write-back's read-modify-write holds the writer: a client write during it is not overwritten | CI (`cargo test`) |
| `a_client_write_during_a_write_back_is_still_embedded_and_reaches_the_store` | a client write that waited behind a write-back still enqueues its own embedding event and reaches the store | CI (`cargo test`) |
| `a_stale_embedding_is_never_attached_to_a_newer_value` | the stamp check under the writer: a vector computed from an older text is not attached to a newer value | CI (`cargo test`) |
| `concurrent_same_key_puts_wait_on_the_shared_writer_and_converge_across_a_partition`, `same_key_puts_on_one_node_converge_to_a_higher_stamp_from_across_the_partition` | on a simulated node: two puts of one key wait on the shared writer; the nodes differ while partitioned and converge after it heals, whichever node holds the highest stamp | `pnpm test:sim` (feature `simulation`) |
| `two_different_keys_on_one_stripe_both_complete` | two keys on one stripe exclude each other, the waiter is served on release, and alternating writers all complete | CI (`cargo test`) |
| `registry_holds_no_per_key_state_after_100_000_distinct_keys` | nothing is kept per key written: no stripe is still referenced or locked after 100 000 distinct keys | CI (`cargo test`) |
| `stripe_table_footprint_is_within_the_stated_constant` | the table's size, from the sizes of its parts, is within `KEY_WRITER_FOOTPRINT_BOUND_BYTES` | CI (`cargo test`) |
| `a_second_acquire_by_the_holding_task_on_the_same_stripe_stays_pending` | WHY a task holds at most one writer: a second acquire on the holder's own stripe never returns | CI (`cargo test`) |
| `acquires_of_free_keys_complete_in_one_poll_of_their_task` | taking a free key does not make the task yield: a task that awaits nothing but 1 024 acquires of keys nobody holds is polled once. It guards the write path against paying a trip through the run queue for a lock nobody contends, which is what filled the partition workers' inboxes under CPU pressure. Red when the acquire is charged to the task's cooperative budget: 8 polls instead of 1. Not covered: an acquire of a HELD key, which still waits for the release | CI (`cargo test`) |
| `count_alloc_acquire_is_bounded_on_first_use_and_zero_on_repeat` | a stripe's first use allocates at most once and within the bound; a second pass over the same keys allocates nothing. Measured on macOS arm64 only | local only (`--features count-alloc`, `--ignored`); CI never enables the feature |

### TG-KEY-002: A `RecordStore` mutator cannot be called without a writer token, and refuses a token of another key

- **Scope:** the five methods of the `RecordStore` trait that write a record value — `put`,
  `update_in_place`, `remove`, `put_backup`, `remove_backup` — on every implementor, and every
  call of them in the crate, production and test.
- **Statement:** each of the five takes `&KeyWriteToken`, so a call without a token does not
  compile. The token is the lock guard of its key's writer: it is the only handle on that lock,
  it cannot be copied, and it exists exactly as long as the writer is held (`TG-KEY-001`), so
  presenting one means holding the writer — not having held it once. `DefaultRecordStore`
  checks the token against the `(map, key)` of the call as the first statement of each of the
  five, and the trait's default `update_in_place` does the same before it reads or runs the
  caller's closure. A token acquired for another key, for another map, or for another key that
  shares the requested key's stripe — whose lock it does hold — is refused before anything is
  written, notified or mutated. The check is always on; it is not a debug assertion and is
  behind no feature.
- **What a refused call looks like:** the store logs it once, at error level, with the pair the
  token was acquired for, the pair it was presented for and the mutator's name: a mismatch is a
  defect of the server's own caller, never load and never client input. The error keeps the
  typed mismatch as its source at the store boundary and prints one fixed sentence that names no
  map and no key; that sentence is all a client can be sent. The client sees an internal,
  retryable error: the refused write is valid and succeeds once the server is fixed. Until then
  that client re-sends and is refused every time, and the store's error line is the signal.
- **Not claimed:**
  - **"No write without a token."** The claim is "no `RecordStore` mutator without a token".
    `MapDataStore` takes no token, and the writers that call it directly are listed under
    `TG-KEY-001`.
  - **The methods that change resident or engine state, or flush, without writing a new record
    value** (`evict`, `evict_all`, `evict_expired`, `evict_lru`, `clear`, `reset`, `destroy`,
    `init`, `get`, `get_all`, `soft_flush`) take no token.
  - **The methods that hand out the raw engine or data store** (`storage`, `map_data_store`)
    stay on the trait and take none. They have no production caller; nothing but a grep keeps it
    so.
  - **That a method is on the right list.** The classification test below fails on a method
    nobody classified; a method put on the wrong list passes it.
  - **That the token was taken early enough or is held long enough.** The borrow covers the
    store call, not the caller's critical region; that part is `TG-KEY-001`'s.
  - **A way for a client to retire a write the server refuses on every attempt.** Its queue
    head stays blocked until a fixed server is deployed.
- **Maintaining code:** `storage/record_store.rs` — the trait's five signatures, the default
  `update_in_place`, `KEY_WRITER_MISMATCH_CONTEXT`; `storage/impls/default_record_store.rs` —
  `refuse_foreign_token` and its five calls; `storage/key_writer.rs` — `KeyWriteToken`, `check`,
  `KeyWriterMismatch`.
- **Enforcing test:** `default_record_store.rs::a_token_for_another_key_is_refused_and_nothing_is_written`
  — for each of the five mutators, over a token of another key, of another map and of another
  key on the same stripe: a typed mismatch, both keys' values unchanged, no data-store call, no
  observer notification, no closure run, one error line, and a text that names no key. Runs in
  CI (`cargo test`). The table below lists the others and where each one runs.
- **Violation consequence:** a caller that locked the wrong key, or no key, writes a record
  outside its writer. That is the interleaving `TG-KEY-001` exists to remove: memory serves one
  value while the durable store holds the other, and a restart returns the older one. Without
  the signatures a new caller that forgets the writer compiles and passes every test that does
  not race it.
- **Discovered by:** the SPEC-384 call-site sweep, which found the rule held only by counting
  acquire sites; SPEC-384b1 introduced the token type and SPEC-384b2 the signatures and the
  refusal.
- **Status:** decided, **enforced** — for the five existing mutators by their signatures, which
  CI compiles at every call site, and by the store's check; a method added to the trait is
  caught by the classification test, which checks that it was classified and not that it was
  classified correctly.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| the five signatures | every call site of the five mutators is compiled with a token | CI: every build and `cargo test --all-targets` |
| `a_token_for_another_key_is_refused_and_nothing_is_written` | each of the five mutators of `DefaultRecordStore` refuses a token of another key, of another map and of another key on the same stripe; nothing is written; one error line; the text a client would get names no key | CI (`cargo test`) |
| `the_default_update_in_place_refuses_a_foreign_token_before_the_closure_runs` | the trait's default `update_in_place` refuses before the caller's closure runs, and does not log | CI (`cargo test`) |
| six one-statement mutants, one per mutator and one for the default body | removing one refusal turns exactly that mutator's observation red | one shot, at landing; not a guard |
| `every_method_of_the_record_store_trait_is_classified` | every method of the trait, `fn` and `async fn` (32), is on exactly one of four lists — takes the token; reads only; changes resident or engine state, or flushes, without writing a new record value; hands out the raw engine or data store — so a new method fails the test until it is classified. **It checks classification, not behaviour — a wrong label passes; it reads source text** | CI (`cargo test`) |
| a production call with its token argument deleted | does not compile | one shot, at landing; not a guard |
| the doctest pair on `RecordStore::put` | the same, as a `compile_fail` block with a `no_run` twin that differs only in the token | LOCAL only (`cargo test --doc`); no CI job runs doc tests; not a guard |
| `a_token_covers_exactly_the_key_it_was_acquired_for` | a token answers for its own pair only, not for another key of its own stripe; while it lives the key's writer is taken | CI (`cargo test`) |
| `check_names_both_identities_on_a_mismatch_and_is_ok_on_a_match` | `check` names both pairs and the error survives `anyhow` | CI (`cargo test`) |
| `assert_not_impl_any!(KeyWriteToken<'static>: Clone, Copy)` | the token cannot be copied | CI: the test build fails to compile |

### TG-DISP-001: A message is handled by the same code top-level and inside a `BATCH` envelope

- **Scope:** every client message the WebSocket handler dispatches to a domain service
  (`network/handlers/websocket.rs`), whether it arrives as its own frame or as an inner item of a
  `BATCH` envelope, and the classification of a message into an operation (`service/classify.rs`).
- **Statement:**
  (a) *One entry.* A message that is not itself an envelope is routed and answered by one
  function, `dispatch_one_message`, on both paths. The envelope only unpacks: it walks its inner
  items in order and hands each to that function, awaiting one before the next. So whatever a
  message is answered with top-level — an acknowledgement, a per-op refusal, a 429 back-off
  frame — it is answered with inside an envelope, and the answers of one envelope leave in
  envelope order.
  (b) *The split.* An `OP_BATCH` is always split by its keys' partitions and each sub-batch runs
  on that partition's worker, top-level or enveloped. Classification refuses a whole `OP_BATCH`
  (`ClassifyError::RequiresPartitionSplit`), so no transport can hand one to the global worker by
  classifying it.
  **Exceptions — four, and each is a difference between the two paths:**
  1. `DeviceHello` is handled in the connection's read loop. Inside an envelope it is not accepted.
  2. `ClientApplyAck` is handled in the read loop as well. Inside an envelope it is not accepted.
  3. A nested `BATCH` is not unpacked: an envelope is one level deep.
  4. The rate-limit charge is taken in the read loop, per frame, after the envelope is unpacked
     there: what an envelope costs is `TG-RATE-001`.
- **Not claimed:**
  - **An answer to what the envelope does not accept.** A `DeviceHello`, a `ClientApplyAck` or a
    nested `BATCH` inside an envelope is dropped with a log line and the client gets no frame.
    The same holds for an inner item that does not decode and for a truncated tail: the rest of
    a truncated envelope is not dispatched and nothing says so. Tracker TODO-785.
  - **That nothing can build a batch without a partition.** An operation can still be constructed
    by hand with no partition and passed to a service directly; simulation and unit tests do. The
    dispatcher does not refuse it. What keeps such a batch correct is the per-key writer
    (`TG-KEY-001`), not its route.
  - **That a full worker inbox is what the back-off test produces.** The first enforcing test
    stages the overload error at the service; that a full inbox raises that error is the
    dispatcher's own test.
  - **Any transport other than the WebSocket envelope.** HTTP sync builds its sub-batches per
    partition itself and has no envelope.
- **Maintaining code:** `network/handlers/websocket.rs` — `dispatch_one_message`, its two callers
  `dispatch_message` and `dispatch_envelope`, `UnpackedEnvelope::unpack`, which decodes an
  envelope's inner items in the read loop, and `dispatch_op_batch`;
  `service/classify.rs` — the `OpBatch` arm of `classify`; `service/operation.rs` —
  `ClassifyError::RequiresPartitionSplit`. Citations are line-number-free, per `TG-OR-004`.
- **Enforcing test:** one per property, each shown red on a tree with that property removed.
  Clause (a) is not one property but four, and no single test holds "the same code on both paths":
  - *the back-off answer* — `an_overloaded_inner_message_is_answered_with_a_back_off_frame`;
  - *the principal an inner message is authorized as* —
    `an_enveloped_message_is_authorized_as_the_connections_principal`;
  - *the connection an inner message is handled for* —
    `an_enveloped_single_write_is_handled_as_its_connections_write`;
  - *envelope order* — `tests/integration-rust/enveloped-op-batch.test.ts`, and only it: no Rust
    test sees the order. A tree that dispatches the inner items last-to-first, or all at once, is
    green on every in-process test and red on this one.
  Clause (b), *the split* — `an_enveloped_op_batch_is_split_onto_its_keys_partition_workers`.
  The four Rust tests are in `websocket.rs`'s test module and run in CI (`cargo test`); the
  integration test runs in CI (`pnpm test:integration-rust`). The table below says what each was
  shown red on, and lists the others. What the tests do not hold: any other argument or branch the
  two paths could come to differ in. That there is one entry at all is held by structure — one
  `.classify(` call site in `websocket.rs` — and not by a test.
- **Violation consequence:** an enveloped `OP_BATCH` that is classified whole runs on the global
  worker with no partition: every op of it is serialised behind one worker, one refused op fails
  the whole batch at validation, and the client is sent nothing — no per-op refusal, no
  acknowledgement of the accepted ops, no back-off frame when the server is overloaded. A client
  waiting for a verdict on those ops receives none.
- **Discovered by:** the SPEC-384 call-site sweep (the envelope kept its own classify-and-dispatch
  block, the one production caller that classified an `OP_BATCH` whole); fixed and catalogued by
  SPEC-384a2.
- **Status:** decided, **enforced** — by the tests below; not by a type.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `an_overloaded_inner_message_is_answered_with_a_back_off_frame` | clause (a), the back-off answer: an overloaded `CLIENT_OP` is answered with one 429 frame top-level and inside an envelope, and so is an overloaded enveloped `OP_BATCH`. Shown red on the tree where the envelope splits an `OP_BATCH` but keeps its own block, without the 429 arm, for every other message: the enveloped `CLIENT_OP` got no frame. It does not see the principal or the connection id: the overload is staged before either is read | CI (`cargo test`) |
| `an_enveloped_message_is_authorized_as_the_connections_principal` | clause (a), the principal: under a policy that admits a write only for the principal its record names, one `OP_BATCH` is answered differently for two principals, and for each of them the enveloped frames are byte-equal to the top-level ones and the same records are stored. Shown red, 20 of 20, on the tree where the envelope hands its inner messages on with no principal: every op was refused | CI (`cargo test`) |
| `an_enveloped_single_write_is_handled_as_its_connections_write` | clause (a), the connection id: a `CLIENT_OP` that names no connection is taken by the CRDT service for a trusted internal call — the caller's stamp is kept and the schema check is skipped. Top-level and enveloped alike, a valid write is stored under a stamp the server minted, and a write the map's schema rejects gets no frame and is not stored. Shown red, 20 of 20, on the tree where the envelope keeps its own block — with the 429 arm — that does not set the connection id: the client's stamp was stored and the schema-invalid write was acknowledged and stored | CI (`cargo test`) |
| `an_enveloped_op_batch_is_split_onto_its_keys_partition_workers` | clause (b): the ops of one enveloped `OP_BATCH` whose keys fall in two partitions reach the service labelled with those two partitions. Shown red on the tree before the fix: one batch with no partition. What it reads is the label the dispatcher routes by; that a label reaches its worker is the dispatcher's own test | CI (`cargo test`) |
| `an_enveloped_op_batch_refusal_is_answered_like_a_top_level_one` | the frames answering a batch with one refused op are byte-equal on both paths; the accepted op is stored and the refused one is not | CI (`cargo test`) |
| `nested_empty_op_batch_emits_no_frame` | an empty enveloped `OP_BATCH` is not dispatched and gets no frame, while a non-empty one after it in the same envelope is dispatched and acknowledged. Shown red on the tree where the envelope skips an inner `OP_BATCH` | CI (`cargo test`) |
| `classify_refuses_an_op_batch` | classification returns `RequiresPartitionSplit` for an `OP_BATCH` | CI (`cargo test`) |
| `tests/integration-rust/enveloped-op-batch.test.ts` | clause (a), envelope order — the only enforcer of it — against a real server binary: an enveloped `OP_BATCH` with one refused op is answered with the refusal and the acknowledgement of the accepted ops, ahead of the acknowledgement of a write placed after it in the same envelope. Shown red on the tree that dispatches the inner items last-to-first and on the tree that dispatches them all at once: nothing arrived ahead of the later write's acknowledgement | CI (`pnpm test:integration-rust`) |

### TG-RATE-001: The cost a frame presents to its connection's limiter is not less than the number of client operations it hands to dispatch

- **Scope:** every inbound WebSocket frame that reaches the per-connection rate limiter in the
  read loop (`network/handlers/websocket.rs`): a message sent on its own, and a `BATCH` envelope
  with whatever it packs.
- **Statement:** for every inbound frame, the cost presented to the connection's limiter is not
  less than the number of client operations the frame hands to dispatch. A **client operation**
  is one `ClientOp` of an `OP_BATCH`, or one message that is not an `OP_BATCH`; it is counted
  once however many times dispatch is called with it. A frame the limiter refuses hands no
  operation to dispatch.
  How it is kept:
  (a) *Cost of one message.* An `OP_BATCH` costs the number of operations it carries, at least
  one; every other message costs one — an envelope found inside an envelope included.
  (b) *One decode, one value.* The read loop unpacks an envelope once, before the charge, and
  moves the unpacked value into the dispatch task. The cost is computed from the messages that
  value holds and is stored nowhere, and dispatch consumes those messages and reads no envelope
  bytes of its own. The unpacker is handed the envelope's `data` bytes and nothing else, so the
  count the envelope declares has no way into the cost.
  (c) *What an envelope costs.* The sum of what its decodable inner messages cost, plus one per
  framed item that does not decode, at least one. An envelope framing more than 8 192 items is
  not decoded at all, costs its framed item count, and is answered whole with one 413.
  (d) *A nested envelope* costs one token and is neither unpacked nor dispatched: an envelope is
  one level deep (`TG-DISP-001`, exception 3).
  (e) *Refusal.* A frame the bucket does not cover is answered with one 429 and dropped whole in
  the read loop; nothing of it reaches the dispatch task.
- **Not claimed:**
  - **What the bucket takes for the cost presented, and any bound on operations per frame.** The
    bucket charges `min(cost, capacity)` (`network/rate_limit.rs`, `try_consume_at`): a frame
    whose cost is above the burst passes on a full bucket and is charged the burst, top-level and
    enveloped alike. This entry is a statement per frame about the cost presented; what the
    bucket then does with it, and the rate that results over time, are outside it. Tracker
    TODO-797, which also holds the considered hard cap on operations per frame.
  - **A limiter configured with refill 0.** It answers yes and charges nothing
    (`network/rate_limit.rs`, `try_consume_at`). By configuration.
  - **`ORMAP_PUSH_DIFF`.** A message with N entries is one client operation by the definition
    above and costs one.
  - **The number of dispatch calls.** The per-op fallback dispatches an operation of a
    permanently refused sub-batch a second time, as a singleton, for the one token it was charged.
  - **HTTP `/sync`,** which this bucket does not charge. Tracker TODO-799.
  - **An undecodable top-level frame,** which is dropped before the charge, **and decode before
    refusal:** an envelope the bucket then refuses has already been decoded. Tracker TODO-798.
  - **Any limit across connections,** per principal or per address; cost by bytes or CPU.
  - **An answer to a nested envelope or to an inner item that does not decode.** Still
    `TG-DISP-001`'s; tracker TODO-785.
- **Maintaining code:** `network/handlers/websocket.rs` — `message_cost`,
  `UnpackedEnvelope::unpack` and `UnpackedEnvelope::cost`, `InboundFrame::cost`, the charge and
  its `continue` in the read loop of `handle_socket`, `dispatch_envelope`. Citations are
  line-number-free, per `TG-OR-004`.
- **Enforcing test:** `an_enveloped_op_batch_costs_its_operation_count` and the four other unit
  tests of the table below, in `websocket.rs`'s test module, for clauses (a)–(d); and
  `tests/rate_limit_envelope_integration.rs`, **the only enforcer of the read-loop wiring** — of
  the cost the read loop presents and of clause (e). The unit tests never enter the read loop:
  a read loop that presented some other figure than the unpacked value's cost, or that sent the
  429 and dispatched the frame anyway, is green on all of them. All run in CI (`cargo test`).
  Held by the signature and not by a test: that the declared count is not read, and that the
  cost is of the list dispatched.
- **Violation consequence:** a peer packs operations so that they cost less than they are. Before
  this entry a frame `BATCH[ OP_BATCH(N) ]` presented a cost of one and dispatched N operations
  across the partition workers, N limited only by the 2 MB frame.
- **Discovered by:** the SPEC-384 planning read of the read loop (the charge counted an
  envelope's framed items without decoding them); left open by SPEC-384a2, fixed and catalogued
  by SPEC-385.
- **Status:** decided, **enforced** — by the tests below and, for the two properties named above,
  by the unpacker's signature.

| Enforcer | What it covers | Where it runs |
|----------|----------------|---------------|
| `an_enveloped_op_batch_costs_its_operation_count` | an envelope holding one `OP_BATCH` of 7 costs 7; one holding `OP_BATCH(3)`, `CLIENT_OP`, `OP_BATCH(0)`, `PING` costs 6. Shown red on the mutant where an inner `OP_BATCH` costs one inside the envelope's sum, and on the mutant where an empty `OP_BATCH` costs nothing | CI (`cargo test`) |
| `an_envelope_costs_what_its_messages_cost_on_their_own` | property, 256 generated envelopes under a fixed generator: 1 to 12 items — batches of 0 to 39 operations, single writes, pings, nested envelopes, items that do not decode — an arbitrary declared count, an optional truncated tail. The cost is the sum of what the items cost on their own, the held messages are the decodable items in order, and the operations held never exceed the cost. Shown red on the two mutants above and on those where an undecodable item is free, where the walk ends at the first undecodable item, and where a nested envelope's items are unpacked | CI (`cargo test`) |
| `an_envelope_never_presents_a_cost_below_what_it_dispatches` | through the real dispatch path: one envelope with two real batches, an empty batch, an undecodable item and a nested envelope presents a cost of 12 and hands 9 operations to the domain service. Shown red on the same five mutants | CI (`cargo test`) |
| `a_nested_envelope_costs_one_token_and_is_not_unpacked` | clause (d): cost 1, nothing dispatched, no frame, with both headers declaring a count that is not true. Shown red on the mutant that unpacks a nested envelope's items | CI (`cargo test`) |
| `an_oversized_envelope_is_not_decoded_and_is_refused_whole` | clause (c), the cap: 8 193 framed items, the first a real batch — nothing held, cost 8 193, one 413, nothing dispatched. Shown red on the mutant with the cap check removed | CI (`cargo test`) |
| `inbound_op_cost_counts_batch_ops` | the costs of single messages, and of envelopes whose items do not decode (50 items cost 50, none costs 1, a header declaring 99 over 3 items costs 3). Shown red on the mutants where an empty `OP_BATCH` costs nothing, where an undecodable item is free, where the walk ends at the first undecodable item, and — alone — on the mutant with no floor of one on an envelope's cost | CI (`cargo test`) |
| `tests/rate_limit_envelope_integration.rs` :: `an_enveloped_op_batch_drains_the_bucket_like_a_top_level_one` | **the cost the read loop presents**, through a real socket: an envelope holding one `OP_BATCH` of 100 drains a 100-token bucket, so both batches of 60 that follow are answered 429. Shown red, 20 of 20, on the tree before the fix (one 429), on the mutant where an inner `OP_BATCH` costs one, and — alone among all the tests — on the mutant where the read loop presents the framed item count instead of the unpacked value's cost. Control: `a_top_level_op_batch_drains_the_bucket` | CI (`cargo test`) |
| `tests/rate_limit_envelope_integration.rs` :: `a_refused_envelope_hands_nothing_to_dispatch` | **clause (e)**, through a real socket with a recording service behind a one-worker dispatcher: an envelope of 70 writes the bucket does not cover gets one 429 and not one of its writes is recorded — neither by the time a write sent after it is, nor by the time the recorder has then been silent for one second. Shown red, 20 of 20, on the mutant where the read loop sends the 429 and moves the frame to dispatch anyway, and on the one where it dispatches the refused frame 200 ms later from a task of its own — the only test that is, on either. Control: `an_admitted_envelope_hands_every_operation_to_dispatch` records all 70 with no 429. The zero is read twice. The first read, when the later write arrives, orders a dispatched write ahead of it only on one worker and on the single-threaded test runtime polling tasks in spawn order; the second, after the quiet period, does not depend on task order. A late write can only turn the test red, never green; a write dispatched later than the quiet period is still not seen | CI (`cargo test`) |
| nine one-change mutants, ten tests each | each mutant turned red exactly the tests named for it above and no other | one shot, at landing; not a guard |

### TG-ENG-001: A `KvEngine` write batch is atomic and ordered, also for a concurrent reader; `shutdown` keeps every returned batch

- **Scope:** every implementation of the byte-level engine boundary `KvEngine`
  (`storage/datastores/kv_engine.rs`): the reference engine `MemKvEngine` and `RedbKvEngine`. No
  server code calls the boundary yet.
- **Statement:** the ops of one `write` apply in batch order (a later op on a key wins; a range
  delete removes keys put earlier in the batch). A batch holding a range delete with
  `start > end` is refused with `InvalidArgument` and nothing of it is applied. When `write`
  returns `Ok`, every later read of the process sees all of the batch, also when it was written
  with `sync = false`, and no reader ever sees a strict subset of a batch: a snapshot contains each
  batch whose `write` overlapped it wholly or not at all. After `shutdown` returned and the engine
  was dropped, an engine opened on the same files holds every batch whose `write` returned before
  `shutdown`. After a crash the durable state is the result of a prefix of the called batches, each
  whole, containing every batch made durable by a returned `sync = true` write or `sync()`.
- **Not claimed:** `Eventual` durability; engines without a crash target (none today); device
  reordering across a completed `fsync` (redb's own assumption). The crash model is "everything
  written after the last sync may be lost, whole, per write or per 512-byte sector"; redb runs with
  its default one-phase commit.
- **Maintaining code:** `kv_engine.rs` — `MemKvEngine::write` (one lock acquisition per batch) and
  `MemKvEngine::shutdown`; `redb_engine.rs` — `RedbKvEngine::write` (validation, then one write
  transaction), `sync`, `shutdown` (sync, then drop the database).
- **Enforcing test:** `k03_redb_every_crash_point_recovers_a_prefix_that_holds_every_durable_batch`,
  `k04_redb_random_crashes_recover_a_prefix_that_holds_every_durable_batch`,
  `k05_mem_crashes_recover_a_prefix_that_holds_every_durable_batch` — in
  `storage/datastores/kv_crash_tests.rs`; `c04_batch_ops_apply_in_order`,
  `c05_a_batch_with_an_invalid_range_changes_nothing`,
  `c25_a_concurrent_reader_never_sees_part_of_a_batch`,
  `m03_a_reader_never_sees_a_batch_parked_between_its_ops`,
  `r01_a_reader_never_sees_part_of_a_batch_while_its_commit_syncs`,
  `c24_the_engine_matches_the_model`, `c27_shutdown_then_reopen_keeps_every_returned_batch` — in
  `storage/datastores/kv_conformance.rs`. The `c` cases run for every engine; `m03` parks a mem
  write between its first and second op and reads from another thread; `r01` parks redb's durable
  commit inside `sync_data` and reads at every park. `k03` kills redb at every backend op of a
  fixed 12-batch workload over an in-process backend that loses unsynced writes (dropped, kept per
  write, or torn per sector) and also checks a never-armed run recovers exactly the durable
  prefix; `k04` does the same at random ops of 200 seeded workloads (`k04b`, ignored, 10 000);
  `k05` crashes the reference engine keeping every number of whole unsynced batches.
- **Violation consequence:** a record layer built on the engine would expose half of a multi-key
  update to a reader, or lose acknowledged batches across a clean restart.
- **Discovered by:** SPEC-394a (the boundary's contract).
- **Status:** decided, **enforced** for both engines. Shown red on one-change mutants: a batch
  applied op by op (mem), one commit per op (redb, `r01`, 10 of 10 runs, each ending `FAILED`), the
  database forgotten instead of dropped (redb, `c27`). A14 (Mem lock per op) is killed
  deterministically by `m03_a_reader_never_sees_a_batch_parked_between_its_ops`, 10 of 10;
  `c25_a_concurrent_reader_never_sees_part_of_a_batch` is a probabilistic smoke test (A14 killed in
  8 of 10 runs at its frozen iteration count) and is not counted as A14's killer.

### TG-ENG-002: A snapshot is a consistent point-in-time view, iterated in byte order; a prefix walk follows the page and walk rules

- **Scope:** `KvEngine::snapshot`, `KvSnapshot::get` / `iter`, `KvEngine::scan_prefix` of every
  engine (both engines here use the trait's default `scan_prefix`).
- **Statement:** every ordered view is strictly ascending by unsigned byte-lexicographic order. A
  snapshot never sees a put, delete or range delete made after it, and none of its iterators is
  affected by writes made while it runs; of two snapshots taken one after the other by one thread
  the later contains every batch of the earlier. `iter` honours an inclusive start, an exclusive
  end and open bounds. `scan_prefix` returns the keys under the prefix greater than its cursor, at
  most `limit`, from one snapshot; its `resume_after` is `Some(last key)` iff the page is full. A
  walk returns a key at most once and returns it iff it is present in the snapshot of the call
  whose page interval contains it: a key present for the whole walk exactly once, a key put behind
  the cursor never, and deleting the cursor key changes nothing for later pages.
- **Maintaining code:** `kv_engine.rs` — the default `KvEngine::scan_prefix` and
  `prefix_successor`, `MemKvEngine::snapshot` (an immutable copy) and its snapshot's `iter`;
  `redb_engine.rs` — `RedbKvEngine::snapshot` (a held read transaction) and its snapshot's `iter`.
- **Enforcing test:** `c09_iteration_is_in_ascending_byte_order`,
  `c11_a_snapshot_does_not_see_writes_made_after_it`,
  `c12_an_iterator_is_unaffected_by_writes_made_while_it_runs`,
  `c14_scan_prefix_pages_cover_every_key_once`,
  `c26_a_cursor_walk_with_writes_between_pages_follows_the_page_rule`,
  `c24_the_engine_matches_the_model` — in `storage/datastores/kv_conformance.rs`, for every engine.
- **Violation consequence:** a scan resumed by key would skip or repeat records after concurrent
  writes, or a reader would mix two points in time.
- **Discovered by:** SPEC-394a.
- **Status:** decided, **enforced**. Shown red on one-change mutants of the cursor (inclusive
  resume bound, restart on a deleted cursor key, one-key look-ahead, open upper bound, a successor
  without the `0xFF` strip) and of the snapshot (live reads, a fresh read transaction per `iter`,
  an inclusive end bound).

### TG-ENG-003: An engine refuses what it does not offer, typed, without side effects; a broken engine says so by type

- **Scope:** `KvEngine::capabilities` and every call of an operation an engine may not offer
  (today only `ingest`); the error mapping of `RedbKvEngine`.
- **Statement:** an operation an engine does not offer returns `KvError::Unsupported(capability)`
  whatever its arguments, changes nothing and never panics, and `capabilities()` says so in
  advance; an operation declared present is not refused. `KvError::Broken { cause }` is returned
  only for what `TG-WB-010` treats as fatal for the process (an I/O error, a previous I/O error,
  corruption, a poisoned lock), decided from a typed classification — the engine's error type, or
  the error-kind enum its binding library exposes — and never from error text; everything else is
  `KvError::Other`. For redb the cause is the one the production store's `broken_cause` gives,
  because the engine calls that function.
- **Maintaining code:** `redb_engine.rs` — `RedbKvEngine::capabilities`, `RedbKvEngine::ingest`,
  `kv_from_storage`, `kv_from_commit`, `kv_from_transaction`, `kv_from_table`; `redb.rs` —
  `broken_cause`.
- **Enforcing test:** `c17_ingest_replaces_a_range_or_is_refused_as_unsupported`,
  `c18_capabilities_are_truthful`, `r02_a_latched_engine_reports_broken_previous_io`,
  `r03_storage_errors_map_to_their_broken_cause`,
  `r04_redb_ingest_is_refused_as_unsupported_and_never_panics` — in
  `storage/datastores/kv_conformance.rs`.
- **Violation consequence:** a caller would depend on an operation that is not there, or a broken
  store would be retried for ever instead of stopping the node (exit 72).
- **Discovered by:** SPEC-394a.
- **Status:** decided, **enforced**. Shown red on one-change mutants: a capability declared
  present while refused, a panicking `ingest`, an argument check before the capability check,
  `PreviousIo` mapped to `Other`.

### TG-ENG-004: A record's engine key is `keyspace ‖ hash_to_partition(key) ‖ map_id ‖ key`, and a map's scan walks it in `(partition, key)` order

- **Scope:** `EngineMapDataStore` and the record-key codec of `engine_store.rs`.
- **Statement:** a record of map `m`, key `k` lives at exactly one engine key,
  `0x10 ‖ p(k) ‖ id(m) ‖ k` for the primary keyspace and `0x11 ‖ …` for the backup one, where
  `p = topgun_core::partition::hash_to_partition` (u32 BE) and `id(m)` is the map's registry id
  (u32 BE). The encoding is injective and decodes back; the keys under a `keyspace ‖ p ‖ id`
  prefix are exactly that triple's keys. A map's scan walks the 271 partitions in sequence from
  one snapshot, so its rows come in `(partition, key)` order — the byte order of their engine
  keys; a returned cursor is the engine key of the last row of its batch, and a resume starts
  strictly after it. `PARTITION_THEN_KEY` positions (`p ‖ key bytes`) agree with that order, so the
  store declares it and the four clauses of the scan-order contract hold (`TG-WB-008`).
  The applied watermark of WAL partition `p` lives at `0x20 ‖ p ‖ "applied"` (`p` u32 BE), its
  value the sequence as u64 BE; an absent row means 0. That partition is the WAL partition
  `fnv1a("{map}:{key}") % 271`, which differs from the record partition above until TODO-843.
- **Maintaining code:** `engine_store.rs` — `RecordKey::{encode, decode}`, `PARTITION_THEN_KEY`,
  `partition_range`, `resume_partition`, `scan_from`, `enumerate_leaves`, `visit_records`.
- **Enforcing test:** `s01_record_key_golden_vectors`,
  `s02_record_key_encoding_is_injective_and_decodes_back`,
  `s03_a_record_key_prefix_holds_exactly_its_triple`,
  `s04_the_partition_is_hash_to_partition_of_the_user_key`,
  `s11_a_scan_returns_keys_of_every_partition_in_partition_then_key_order`,
  `s12_each_cursor_is_the_engine_key_of_the_last_row_and_a_resume_is_strictly_after_it`,
  `s28_a_write_behind_scan_over_the_engine_store_emits_a_key_at_a_partition_boundary_once`,
  `s31_a_cursor_of_another_map_or_keyspace_fails_the_scan`,
  `s32_partition_then_key_positions_agree_with_engine_key_order` — in
  `storage/datastores/engine_store_tests.rs`, each over the in-memory and the redb engine.
- **Violation consequence:** a record would be read under the wrong partition or map, a scan
  would skip or repeat rows, or the write-behind overlay would place a buffered key in the wrong
  batch.
- **Not claimed:** that a record's engine partition and the WAL partition of its `0x20` row are
  the same number — the WAL partitions by `"{map}:{key}"`, so in general they differ (TODO-843).
- **Discovered by:** SPEC-394b2.
- **Status:** decided, **enforced**.

### TG-ENG-005: The map registry is a bijection name ↔ id, an id is issued in the batch of the map's first record, and `list_maps` lists the maps with a primary record ever stored

- **Scope:** the registry of `EngineMapDataStore` (`0x01` map by name, `0x02` map by id,
  `next_map_id`), its cache and `registry_lock`.
- **Statement:** a map's id is issued, from 1 upward and never reused, in the same engine batch
  as the map's first record or `Touch`, together with both registry rows and the new
  `next_map_id`; a failed first write leaves no registry row and no record. Two first writes of
  two new maps get distinct ids, because a write that issues an id or sets a flag re-reads the
  registry under `registry_lock` and holds it until its batch has returned. `u32::MAX` is never
  issued: when no id is left, a write that needs one fails with `map id space exhausted` and
  changes nothing. No record key carries an id the registry does not hold. A `Delete` never
  issues an id or sets a flag. The cache is filled only from rows that exist. `list_maps` returns
  every registry name whose primary flag is set, ascending, once. After a crash: no orphan record,
  inverse registry, `list_maps` = names with a primary put in the recovered prefix; every registered
  name has a put in that prefix.
- **Maintaining code:** `engine_store.rs` — `plan_registry`, `cached_row`, `read_map_id`,
  `write_records`, `list_maps`, the `0x01` / `0x02` codecs and `map_id_in`.
- **Enforcing test:** `k06_redb_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps`,
  `k07_mem_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps` — in
  `storage/datastores/kv_crash_tests.rs`, every crash point of two fixed store workloads and 200
  seeded ones; `s05_map_ids_are_issued_from_one_and_survive_a_reopen`,
  `s06_a_failed_first_write_leaves_no_registry_entry_and_no_record`,
  `s07_two_first_writes_of_two_new_maps_get_distinct_map_ids`,
  `s08_a_map_with_only_backup_rows_is_not_listed_and_one_emptied_by_removes_stays_listed`,
  `s09_reads_of_absent_maps_grow_no_registry_cache`,
  `s26_touch_registers_a_map_without_a_record`,
  `s33_an_exhausted_map_id_space_refuses_a_new_map_and_changes_nothing`,
  `s34_the_public_codecs_read_what_the_store_wrote` — in
  `storage/datastores/engine_store_tests.rs`, each over the in-memory and the redb engine.
- **Violation consequence:** two maps would share an id and read each other's records, a record
  would be stored under an id no name resolves to, or `list_maps` would differ from the redb
  store's.
- **Not claimed:** a crash model beyond `TG-ENG-001`'s. No operation deletes a map.
- **Discovered by:** SPEC-394b2.
- **Status:** decided, **enforced**.

### TG-ENG-006: A store opens only at a format, ack mode and keyspace set this binary knows and with no unfinished migration; a migration mark is written only in the batch of its records

- **Scope:** `EngineMapDataStore::{open, open_for_migration, read_migration_mark}` and the
  `SetMigrationMark` op of `write_records`.
- **Statement:** `open` formats an empty engine with `format = 1`, `next_map_id = 1` and
  `ack_mode = 0` (local) in one durable batch. A non-empty engine is opened only when `format` is
  1, `ack_mode` is 0, `next_map_id` is at least 1, the metadata keyspace holds no other name, no
  key lies in a reserved keyspace (`0x03..=0x0F`, `0x12..=0x1F`, `0x21..=0xFF`) and no
  `migration` row is present; anything else is refused with an error starting
  `store format: refusing to open:` and nothing is written. Only `open_for_migration` admits a
  store with a migration mark; on an empty engine it writes the format rows and the mark in one
  batch. A mark is written only through `RecordOp::SetMigrationMark`, in the one batch of its
  call's records. After a crash the migration mark is the one carried by the last recovered batch
  that carried one. The import marker `wal_watermark` (`[0x01]`) says the WAL directory's
  `.applied` sidecars were imported: `0x20` rows exist only with it (a row without the marker is
  refused at open), a malformed `0x20` key or a value that is not 8 bytes is refused, and the
  marker is written only by `import_applied_watermarks`, in the batch of the imported rows. The
  import-pending mark `wal_import_pending` (`[0x01]`) is written only in the batch that clears
  the migration mark and deleted only in the import batch; the two marks are never present
  together (both present is refused at open), and a mark holding anything but `[0x01]` is
  refused.
- **Maintaining code:** `engine_store.rs` — `read_store_metadata`, `format_rows`, `open`,
  `open_for_migration`, `read_migration_mark`, `write_records` (the `SetMigrationMark(None)` and
  `MarkWatermarksImported` arms), `import_applied_watermarks`.
- **Enforcing test:** `k06_redb_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps`,
  `k07_mem_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps` — in
  `storage/datastores/kv_crash_tests.rs`; `s18_a_fresh_engine_gets_format_one_and_local_ack_mode`,
  `s19_a_store_of_an_unknown_format_or_with_unknown_metadata_is_refused`,
  `s20_a_non_empty_engine_without_format_metadata_is_refused`,
  `s21_a_key_in_a_reserved_keyspace_is_refused_at_open`,
  `s29_a_store_with_a_migration_mark_is_refused_by_open_and_admitted_by_open_for_migration`,
  `s30_a_migration_mark_is_written_in_the_batch_of_its_records` — in
  `storage/datastores/engine_store_tests.rs`, each over the in-memory and the redb engine;
  `a15_the_engine_refuses_malformed_partition_metadata` and
  `a24_a_store_just_migrated_beside_a_used_wal_directory_imports_the_sidecars_once` — in
  `storage/wal/store_mode_tests.rs`.
- **Violation consequence:** a binary would serve a store of a newer format or another ack mode
  as if it were its own, or a half-finished migration as complete; a WAL would be filtered by
  rows that no import vouched for, or its sidecars imported twice.
- **Not claimed:** that only the migration issues `SetMigrationMark` — a mark set on a store from
  `open` makes the next `open` refuse (fail-closed, nothing is lost); keeping the op to the
  migration path is the migration's obligation (`TG-ENG-007`, clause 6).
- **Discovered by:** SPEC-394b2.
- **Status:** decided, **enforced**.

### TG-ENG-007: The store is a data directory in a format this binary knows; a table-per-map file is migrated once, resumably, and kept; every location that is not unambiguous is refused without side effects; while a store is open the old path holds the stub, which an older binary refuses; only the migration writes the mark

- **Scope:** `RedbDataStore::open_location` (the location rules L1–L13, the migration and the
  stub) and `RedbDataStore::new` (the same rules with no legacy path), as called by the server
  binary's `select_datastore` before `WalWriter::new` and before the listener; `run_migration`.
  `D` = `TOPGUN_DATA_DIR` (or the directory derived from `TOPGUN_REDB_PATH`, or
  `./topgun-data`); `L` = `TOPGUN_REDB_PATH` (default `./topgun.redb`), the legacy path;
  `K` = `{L}.pre-migration`; the stub = a regular file at `L` of at most 16 384 bytes whose first
  21 bytes are `TOPGUN-STORE-MOVED 1\n`.
- **Statement:**
  1. *One store, a known format.* The store is the data directory `D`, holding `engine.redb`.
     Every open of an existing store goes through `EngineMapDataStore::open` or
     `open_for_migration`, so a store of an unknown format or with an unfinished migration is
     never served (`TG-ENG-006`).
  2. *Migrated once, resumably, and kept.* A table-per-map file at `L` with no store in `D` is
     migrated once: its rows are copied with their value bytes verbatim, and the migration mark
     that says how far the copy got is written in the one engine batch of the copied rows, so an
     interrupted migration — at any point, any number of times — converges to the same content as
     an uninterrupted one. The copy is verified (row counts, every value byte for byte, the
     primary map set) before the legacy file is renamed to `K`, which is kept until an operator
     deletes it.
  3. *Refusal without side effects.* Every location that is not unambiguous is refused — an
     empty variable, `D` and `L` naming the same place or one inside the other, a `D` that is
     neither a store, an empty directory nor absent, **two stores** (a data-directory store and a
     table-per-map file at the old path), an empty file at `L` beside a store, a mark that cannot
     be resumed at this location, a kept `K` with no store, a stub with no store, a file at `L`
     that is not a table-per-map store, a catalog with a table that does not decode, too little
     free space. A refusal logs one error line, returns the error (the binary exits 1), and
     creates, moves and deletes nothing: `L`, `K` and the stub stay byte-identical and the sorted
     path list is unchanged. The engine directory's **key/value content** is unchanged; its
     **bytes are not claimed**, because a refusal that must read the mark opens the engine, and
     that open commits. The logical content is the `KvEngine` key space of `D/engine.redb` —
     every key and value of `KvSnapshot::iter(KeyRange::default())`; access to `engine.redb`
     other than through `RedbKvEngine` (for example a raw `redb::Database` handle adding a table)
     is out of contract.
  4. *The stub.* Whenever the server holds an open store in `D` and nothing is at `L`, it writes
     the stub there, whole or not at all (a temporary file, `sync_all`, `rename`, `fsync` of the
     parent), so `L` is never observed empty or partial. This binary never migrates the stub. A
     binary built before the layout change (read for `07d009f0`, tested for `b346e277`) started
     with the same environment meets the stub, and started with `TOPGUN_REDB_PATH={D}` meets a
     directory: either way its redb refuses the open, before `WalWriter::new` and before it
     listens, so it exits non-zero, serves nothing and changes no file — neither the store nor
     the WAL.
  5. *Exclusive verification.* The migration and its verification run inside the store open,
     before the server opens its WAL or accepts any write, on the one engine handle this process
     opened (redb's exclusive lock refuses a second open of `engine.redb`) and holds from the
     location rules to the end of the migration, so the verification's snapshots see one state.
  6. *Only the migration writes the mark.* In production code the only function that constructs
     `RecordOp::SetMigrationMark` is `run_migration` in `redb.rs` (the first mark goes through
     `open_for_migration`).
- **Maintaining code:** `storage/datastores/redb.rs` — `StoreLocation::from_env`,
  `RedbDataStore::{open_location, new}`, the row evaluation, `MovedMarker`, the migration mark
  codec, `run_migration`, `verify`, `read_legacy_catalog`; `bin/topgun_server.rs` —
  `select_datastore`. Citations are kept line-number-free on purpose, per `TG-OR-004`.
- **Enforcing test:** `m01_a_fresh_start_creates_the_data_directory_and_the_stub`,
  `m02_a_legacy_file_is_migrated_byte_for_byte_and_kept_under_the_pre_migration_name`,
  `m04_k_interrupted_migrations_end_byte_for_byte_equal_to_an_uninterrupted_one`,
  `m05_a_killed_process_resumes_from_its_mark`,
  `m08_every_refusing_row_of_the_location_table_touches_nothing`, `m09_the_same_place_rules`,
  `m11_a_store_with_a_legacy_file_beside_it_is_refused_and_touches_nothing`,
  `m12_insufficient_free_space_refuses_and_touches_nothing`,
  `m14_an_older_binary_cannot_open_the_stub_or_the_data_directory`,
  `m19_the_stub_is_never_a_migration_source`,
  `m21_a_store_with_an_empty_file_at_the_old_path_is_refused_and_touches_nothing`,
  `m22_the_migration_holds_the_engine_exclusively_until_the_store_is_open` — in
  `storage/datastores/redb_migration_tests.rs`. Clause 6 is checked mechanically: a program over
  `redb.rs` lists the enclosing production function of every `SetMigrationMark(` and must print
  exactly `run_migration`, and no other production file but `engine_store.rs` names the op.
  Clause 4 for a real older binary is proven by the two-binary run
  `tests/integration-rust/map-name-rollback.test.ts` (local, the `b346e277` binary,
  `FORMAT_OLD_SERVER_BINARY`): the older binary on the stub and on the data directory, after a
  clean stop and after `kill -9` of the newer one; the documented rollback and rolling forward
  again; a table-per-map file beside a store refused. It needs the older binary, so CI excludes
  it and it is run locally.
- **Violation consequence:** an older binary started on the old path initialises an empty store
  and serves empty maps; a server started with a store in `D` and a table-per-map file at `L`
  silently serves one of them and leaves the other's writes out; a refused start moves or
  deletes the operator's only copy of the data; an interrupted migration serves a partial copy;
  a mark written outside the migration makes the next open refuse a healthy store.
- **Not claimed:** an older binary pointed at the data directory's `engine.redb` itself
  (`TOPGUN_REDB_PATH={D}/engine.redb`) is not protected: it opens that file as a table-per-map
  store with no maps and serves empty maps. The bytes of `engine.redb` across a refusal. A
  rollback after a `kill -9` of the new binary. Any binary older than `07d009f0`. A tool that
  exports the new format back to tables — there is none.
- **Discovered by:** SPEC-394d (the table-per-map store moved behind `KvEngine`).
- **Status:** decided, **enforced**.
