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
- **Enforcing test:** partial —
  `or_inplace_mutate_proptest.rs::new_tombstone_counted_once_across_write_failure_and_retry`
  proves no double-count across fail+retry; the literal call-counter assertion (`AtomicUsize
  == 1` per call) is `NAKED (TODO-602)`.
- **Violation consequence:** hidden internal retry double-applies CRDT mutations/gauge deltas.
- **Discovered by:** SPEC-347 review minors.
- **Status:** decided; enforcement partial.

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

- **Scope:** `OrDelta`/`OrDeltaFold` (SPEC-346 types; unwired until SPEC-349).
- **Statement:** folding any op sequence through the delta path and the full-snapshot path yields
  equal `or_map_semantic_view` (live set + tombstones + pruned), with the durable store as fold
  base and snapshot frames as in-order absolute-set inputs.
- **Maintaining code:** types + oracle landed (SPEC-346); fold delegates to the live apply path
  (single-algebra rule, SPEC-349 R-mandate).
- **Enforcing test:** `NAKED — the OR delta-fold differential recovery proof does not exist yet.
  It lands as a case on the cross-incarnation harness
  (packages/server-rust/src/storage/datastores/wal_harness/), driven by SPEC-349b — not a fork`.
- **Violation consequence:** silent post-crash divergence of OR state — the class the oracle was
  built to kill.
- **Discovered by:** SPEC-346 design.
- **Status:** open (SPEC-349a/b).

### TG-OR-004: The tombstone-bytes gauge tracks the REAL add and prune paths, test-isolatable

- **Scope:** `ProcessGauge`/scoped sink (`storage/tombstone_gauge.rs`, SPEC-351).
- **Statement:** `add_tombstone_bytes` fires on the real OR-remove path and `sub_tombstone_bytes`
  on the real epoch-prune path (mutation-proven both directions); tests bind task-local isolated
  gauges — no order-dependent global reads; negative controls never read a shared counter.
- **Maintaining code:** `record.rs` fns delegating through the scoped sink resolver.
- **Enforcing test:** SPEC-351 suite (9 tests) — real-prune-path coverage
  (`crdt.rs:1394` mutation → deterministic RED), per-binding tripwire, private-counter foreign
  traffic control.
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
