# Ephemeral Storage Class

An app that shows who is on a page, who is in a live session, or any other state that only means something while
its owner is connected (presence, cursors' last position, a "typing" flag, a seller's live-session status) needs
that state shared in real time, queryable like any other map, and gone when its owner leaves — without paying for
a write-ahead log, an fsync and a store write on every update. The **ephemeral storage class** is that: a map the
operator declares ephemeral keeps every live-query, ACL and tenancy property of an ordinary LWW map, but its records
live only in memory on the node, belong to the connection identity that wrote them, and are removed after their
owner leaves or their TTL passes. Nothing about them is ever durable.

This page is the design of the class: how a map becomes ephemeral, how writes are asserted and answered, who owns a
record and when it goes, what is bounded, what is reserved on the wire before the 1.0 freeze, which invariants the
implementation will land, and how the work is split into steps `959-1` .. `959-11`.

| | |
|---|---|
| **Base commit** | `e3e4b5d7` (every `path:line` below was re-read there; the code under `packages/server-rust`, `packages/core-rust`, `packages/client` and `packages/core` is identical at `2948434a`) |
| **Status** | design, not implemented |
| **Normative layer** | the decisions D0–D10, the invariant table, the wire table and the split; everything else is explanatory |
| **Tracker** | TODO-959 (this page is step 959-0) |

**Citation paths.** Server paths are relative to `packages/server-rust/src/` (`network/handlers/websocket.rs:930`).
Paths starting `core-rust/src/`, `client/src/` or `core/src/` are relative to `packages/`. `INVARIANTS.md` is at the
repository root. A range `a-b` is inclusive.

**Two constants used throughout.** `G` is the grace period after an owner stops being live
(`TOPGUN_EPHEMERAL_GRACE_MS`, default 30000). `S` is the removal tick, 1000 ms (D4).

## Contents

- [D0 — The class model](#d0--the-class-model)
- [D1 — What the class excludes and keeps](#d1--what-the-class-excludes-and-keeps)
- [D2 — Declaration, resolution and assertion](#d2--declaration-resolution-and-assertion)
- [D3 — Ownership](#d3--ownership)
- [D4 — Lifetime, TTL and renewal](#d4--lifetime-ttl-and-renewal)
- [D5 — Restart, reconnect storm and ordering](#d5--restart-reconnect-storm-and-ordering)
- [D6 — Resurrection without tombstones](#d6--resurrection-without-tombstones)
- [D7 — Offline](#d7--offline)
- [D8 — Memory](#d8--memory)
- [D9 — Cluster](#d9--cluster)
- [D10 — Wire and format items](#d10--wire-and-format-items)
- [Proposed invariants](#proposed-invariants)
- [Acceptor](#acceptor)
- [Implementation split](#implementation-split)
- [Second-opinion disposition](#second-opinion-disposition)
- [Deferred items](#deferred-items)

---

## D0 — The class model

### Decision

There is one storage class, `ephemeral`, on ordinary **LWW** maps; `durable` is today's behaviour. Topics stay the
mechanism for stateless high-rate signals (cursor moves, reactions, typing pulses). Rooms (TODO-297) are client sugar
over an ephemeral map plus a topic. Live aggregates (D10.4) apply to every map, not only ephemeral ones. OR maps
cannot be ephemeral in v1.

### Options considered

- **(a) The class on maps** — chosen.
- **(b) A `RoomRegistry` in `MessagingService`.** Edge case that rejects it: a watcher over several hundred present
  visitors needs filters, group-by and live updates; under (b) the query engine would have to be rebuilt over a
  second store.
- **(c) Both.** Two mechanisms for one need; rejected by (b)'s edge case plus the deletion-over-addition rule.

### Reason

- The TODO-862 watcher needs exactly what live queries already give a map: predicates, `group_by` and aggregations
  (`core-rust/src/messages/base.rs:198-207`), snapshot plus deltas.
- Topics give a newcomer no snapshot: a topic carries signals, not state.
- OR records are outside live-query scope: evaluation skips every non-LWW record
  (`service/domain/query.rs:299-305`). OR tombstones would also bring back the reclamation the class exists to
  avoid.

### Stated limits

- No OR map can be ephemeral in v1; an OR op on an ephemeral map is refused (D2).
- High-rate signals that need no snapshot (30 Hz cursors) belong on topics, not in an ephemeral map: every ephemeral
  write is a stamped LWW apply with live-query fan-out.

---

## D1 — What the class excludes and keeps

### Decision

**Exclusions for an ephemeral write (normative):**

- no WAL frame and no WAL sequence;
- no write-behind slot or capacity check;
- no store row and no map-registry entry;
- no Merkle observer or leaf, no full-text (search) index and no embedding (see "Observers");
- no `SYNC_INIT` / `ORMAP_SYNC_INIT` / HTTP `/sync`;
- no tombstone;
- no outbox row (TODO-632);
- no S3 copy (TODO-844);
- no snapshot or migration payload (TODO-850);
- no eviction (records over a null store stay dirty and are never evicted, see Reason);
- no TTL sweep: `record.ttlMs` is **stripped before apply** (its deadline goes to the ephemeral registry, D4), so the
  stored record carries no `ttl_ms`, and neither the `TG-TTL-001` read filter nor the `TG-TTL-002` sweep ever sees an
  ephemeral record. The sweep also skips ephemeral maps by class (959-2), because on `STORAGE_BACKEND=null` it reads
  resident rows and presence rows would consume its rows-per-second budget.

**Kept:** the per-key writer (`TG-KEY-001`); server HLC stamping and LWW merge; schema validation; ACL and tenancy
(`TG-AUTHZ-001`, `TG-TEN-001`); live queries (snapshot plus deltas); the journal, only as the D10.5 opt-in.

**Observers, decided per observer.** The record-store factory keeps a **second, separate list** of observer
factories for ephemeral stores, set by a `with_ephemeral(EphemeralConfig, ephemeral_observer_factories)` builder;
`get_or_create` consults that list instead of the durable one when the map resolves ephemeral. No observer factory
is edited, so `storage/merkle_sync.rs`, `service/domain/search/mod.rs` (for the observer) and
`service/domain/embedding/hook.rs` are not touched by 959-1.

| Observer | Ephemeral maps | Edge case that decides it |
|---|---|---|
| Merkle (`storage/merkle_sync.rs:373-384`) | **not attached** | Merkle leaves serve `SYNC_INIT` catch-up, which the class refuses (D2); a leaf per presence key would be O(keys) memory outside the budget and a hash update per write. |
| Search, Tantivy (`bootstrap.rs:250`) | **not attached**; a search, hybrid search or their subscription on an ephemeral map is refused with `storage_class_mismatch` (959-5b) | A map's search index is built lazily from a scan on the first search after `needs_population` is set (`service/domain/search/mod.rs:650-659`, `service/domain/search/mod.rs:803-814`). Without the observer that index would be built once and never updated, so a search would silently return a stale set; with the observer every presence write would pay the Tantivy update, whose ACCEPTOR verdict is П. Refusing is the only option that is neither stale nor П. |
| Embedding (`service/domain/embedding/hook.rs:992`) | **not attached**; a map named in both `TOPGUN_VECTOR_MAPS` and the ephemeral list is a boot refusal (D2, refusal 4) | The hook calls an external provider per write and writes a vector back into the record: per presence update, a network call and a second write. Skipping it silently would make semantic search over that map return nothing, so the overlap is refused at startup. |
| Index observer, scalar and vector (`service/domain/index/mutation_observer.rs:274`) | **attached**; a vector index on an ephemeral map cannot exist: `VECTOR_SEARCH` on one is refused with `storage_class_mismatch` (959-5b, `service/domain/query.rs:701`), registering a vector index on one through the admin API is refused with 409 (959-9, before `network/handlers/admin.rs:1666`), and a vector map declared ephemeral is D2 refusal 4 (`bootstrap.rs:594`) | One factory carries both index kinds. *Scalar:* an index exists only for a map an operator registered one for (restored at boot by `bootstrap.rs:1144`, built per spec at `service/domain/index/scalar_rebuild.rs:104`, created at `network/handlers/admin.rs:1131`). It is in memory and fed by the ordinary put and remove, so it stays exact for an ephemeral map and is empty after a restart like the map; not attaching it would leave a registered index empty and a query that uses it wrong. *Vector:* without the embedding hook a vector index on an ephemeral map would hold only vectors clients send, and `VECTOR_SEARCH` would answer from a partial set; refusing it matches boot refusal 4 and leaves no ephemeral map with a vector index to maintain. |
| Arrow cache (`service/domain/arrow_cache.rs:251`, `datafusion` feature) | **attached** | It only invalidates a cached table per write (O(1), no per-key structure). Not attaching it would serve a stale SQL table. |
| Live-query updates | unaffected | Not an observer: `CrdtService` broadcasts them directly (`bootstrap.rs:562-565`). |

An ephemeral live-query snapshot therefore scans the resident stores of the map (on a null store `scan_merged` emits
only the resident snapshot, `storage/factory.rs:285-286`, `storage/factory.rs:323`), or uses an operator-registered
scalar index. The read surfaces refused on an ephemeral map are listed once, in D2.

**Convergence.** A client gets ephemeral state through a live-query snapshot plus deltas. There is no catch-up from a
sequence. `QUERY_RESP.merkleRootHash` is absent for an ephemeral map, as it is for every map today (the only
production literal sets it `None`, `service/domain/query.rs:987`); no code changes for it.

**Ack.** An ephemeral write is answered `applied` (D2, `EPHEMERAL_WRITE_RESP`) after the in-memory apply.
`writeConcern` is not enforced today: every accepted op answers `achieved_level: APPLIED`
(`network/handlers/websocket.rs:1249`), and its effective carrier is the `OP_BATCH` batch
(`core-rust/src/messages/sync.rs:31-49`; reapplied per sub-batch, `network/handlers/websocket.rs:1113-1139`).
`EPHEMERAL_WRITE` has no batch-level `writeConcern` or `timeout`; the server dispatches its ops with neither, and a
`writeConcern` inside an op is ignored, as on every path today. `applied` is the honest level for the class. There is
no `writeConcern` refusal (deletion over addition), and so no mixed-batch `writeConcern` case to decide.

### Options considered

- **Observers: one list, with a per-factory `None` for ephemeral maps** (each factory asks the class). Rejected: it
  edits three observer factories (`merkle_sync.rs`, the search factory, the embedding hook) and makes every future
  factory remember the class; a second list in the factory is one edit and fails closed for a new factory.
- **Observers: attach none.** Rejected on the scalar-index and Arrow-cache edge cases above (a registered index left
  empty, a stale SQL table).
- **Ack: refuse `writeConcern` above `APPLIED` for ephemeral ops.** Rejected: nothing enforces it today, so a refusal
  would be a new contract for a field no path honours.

### Reason

- Records over a null store stay dirty: `mark_stored` is skipped when the data store is null
  (`storage/impls/default_record_store.rs:357`), and dirty records are never evicted
  (`storage/impls/default_record_store.rs:752-756`).
- The factory shares one `data_store` across maps and builds every store over it (`storage/factory.rs:51-53`,
  `storage/factory.rs:155-196`), so per-map routing to a null-backed store is a factory decision.
- The production factory gets five observer factories — search, Merkle, embedding, scalar/vector index, and the
  Arrow cache under `datafusion` (`bootstrap.rs:617-633`, `bootstrap.rs:640-643`).
- `MerkleObserverFactory` returns an observer for every `(map, partition)` (`storage/merkle_sync.rs:373-384`).

### Stated limits

- A scalar index's memory is outside the ephemeral budget (D8); registering one on an ephemeral map is the
  operator's opt-in.
- If `writeConcern` enforcement ever lands, it must cap an ephemeral op's `achieved_level` at `APPLIED` rather than
  refuse it (recorded on TODO-959).

---

## D2 — Declaration, resolution and assertion

### Decision

#### Declaration

- The operator declares the class with `TOPGUN_EPHEMERAL_MAPS`: a comma-separated list of globs in the policy glob
  syntax (`service/policy/mod.rs:165`).
- Matching uses the **tenant-local** name when `TOPGUN_TENANT_CLAIM` is set (the name after the `t.` segment), and the
  full name otherwise.
- Names with the `_topgun_` / `__topgun_` prefix are **always durable**, whatever the patterns match
  (`service/middleware/authorization.rs:264`).
- The list is read once at boot and is fixed for the process. Unset means no ephemeral maps. Set but empty is fatal
  at startup, like the other variables.
- Narrowing a list (ephemeral → durable) is always accepted, because ephemeral maps hold nothing after a restart.

#### Boot refusals

Each exits 1 **before the server accepts a connection**, and logs one `ERROR` line plus the stderr line
`Error: storage class: refusing to start: {reason}`. "Before it listens" would be false: the client listener is bound
at `bin/topgun_server.rs:926`, before anything below. Refusals 1 and 2 run before the WAL is opened and write
nothing. Refusal 3 runs inside WAL recovery and refusal 4 after it; what has been written by then is listed with
each, and every item is safe on a later start once the configuration is fixed.

1. **Cluster mode.** `TOPGUN_EPHEMERAL_MAPS` set while cluster mode is on (a non-empty seed list,
   `bin/topgun_server.rs:938`) is refused: `storage class: refusing to start: TOPGUN_EPHEMERAL_MAPS is not supported
   in cluster mode until the cluster liveness lease exists (TODO-850)`.
   - Edge case that decides it: on a record node, an owner connected to another node has no registered connection and
     no lease, so liveness at apply (D6) would either drop every remote-owned write or, relaxed, keep them for ever.
     Only the lease separates the two.
   - Lift condition: D9.
2. **Durable registry map.** The server lists the durable map registry (`TG-ENG-005`, `INVARIANTS.md:3370`). If any
   listed name resolves ephemeral, it refuses: `storage class: refusing to start: the durable map {name} matches the
   ephemeral pattern {pattern}`. The check runs per tenant, by applying the same resolver to every full registry name.
3. **Durable map only in the WAL.** WAL recovery refuses to start on a frame whose map resolves ephemeral:
   `storage class: refusing to start: the WAL holds a frame of map {name}, which matches the ephemeral pattern
   {pattern}`. It **applies no frame of a map that resolves ephemeral.**
   - Edge case that decides it: the registry row is issued in the batch of a map's first stored record
     (`INVARIANTS.md:3370`). A durable map whose frames were acked but not yet flushed (after `kill -9` or a
     `drained=false` stop) is in the WAL but not in the registry. Without this check, adding its name to the list
     before a restart would replay acked durable writes into a map that now resolves ephemeral: lost, or registering
     the map behind refusal 2's back.
   - **Where exactly.** `WalRecovery::run` replays one partition at a time (`storage/wal/mod.rs:2945-2983`). The check
     runs on each partition's unapplied entries after `unapplied(partition)` returns and before `replay_partition` is
     called for it; on a match it returns the refusal and replays nothing of that partition. Durable frames of
     partitions replayed earlier in the same run **may already be applied**: that replay is the ordinary, idempotent
     one (frames above the store watermark, applied in order, the watermark committed in the same batch,
     `storage/wal/mod.rs:2896-2908`), and the next start replays nothing below that watermark again. No full pre-scan
     of the WAL is added: it would read every unapplied frame twice on every start to protect a case the per-partition
     check already covers.
   - **What has been written before the refusal**, all by `WalWriter::open_over_store`, which every start runs before
     recovery (`storage/wal/mod.rs:873-960`), and why each is safe on a later start with the configuration fixed:
     - the applied watermarks are imported into the store if they were not yet (`storage/wal/mod.rs:930-938`): it is
       the import the next start would make, and once made it is not repeated;
     - `wal-collected` is written if absent (`storage/wal/mod.rs:940-942`): all zeros, which no watermark is below,
       so the next start's collected-above-watermark check passes;
     - the `.applied` sidecars are deleted (`storage/wal/mod.rs:944`): they were imported first; they are rollback
       hints for an older binary, which every start of this binary deletes anyway;
     - a torn stream tail is cut (`storage/wal/mod.rs:945`): it holds no acknowledged frame (a frame is acked only once
       written whole), and every start cuts it;
     - the log stream starts (`storage/wal/mod.rs:1227-1238`): its first segment follows every segment on disk and
       holds no frame of this run, because recovery writes no frame;
     - the collection task starts (`storage/wal/mod.rs:958`): its first tick runs one interval after the open
       (`storage/wal/mod.rs:964-967`), and a tick unlinks only sealed segments whose frames are all at or below their
       partition's watermark — frames the store already holds;
     - the durable frames of partitions replayed before the refusing one, with their watermarks (above).
   - The resolver reaches `WalRecovery` through a builder, `WalRecovery::with_class_resolver(...)`, set by
     `WalBootstrap::open_over_store` (`storage/datastores/write_behind.rs:2169`). `WalRecovery::new(`
     (`storage/wal/mod.rs:2752`) is called in 13 files; a new parameter there would force them all, a builder forces
     none.
   - **Stderr format.** The binary's error arm for `WalBootstrap::open_over_store` prints `FATAL: {err}` today
     (`bin/topgun_server.rs:1066-1071`). 959-2, which already holds `bin/topgun_server.rs`, prints a storage-class
     refusal from that arm as the `Error: storage class: refusing to start: {reason}` line instead, so refusal 3
     reads like the others.
4. **Vector map declared ephemeral.** A map named in `TOPGUN_VECTOR_MAPS` that resolves ephemeral is refused:
   `storage class: refusing to start: the vector map {name} matches the ephemeral pattern {pattern}`.
   - Edge case that decides it: the embedding observer is not attached to ephemeral stores (D1), so semantic search
     over that map would silently find nothing.
   - It runs in `build_services` (`bootstrap.rs:515-521`), where the vector configuration is read
     (`bootstrap.rs:579`), after WAL recovery and before the server accepts a connection. What has been written by
     then is what recovery writes on every start: no frame of an ephemeral map (refusal 3).
   - **Same stderr format.** `build_services` returns `Result<_, String>`, and the binary prints that string verbatim
     with `exit_fatal` (`bin/topgun_server.rs:641-644`, exit 1) at the single-node call site
     (`bin/topgun_server.rs:1273`; the cluster call site `bin/topgun_server.rs:1237` is never reached with a list set,
     because refusal 1 exits first). So `build_services` logs the `ERROR` line itself and returns
     `Error: storage class: refusing to start: the vector map {name} matches the ephemeral pattern {pattern}`, which
     becomes the stderr line unchanged. It does not use the `FATAL:` prefix of the existing `build_services` strings
     (`bootstrap.rs:439-442`, `bootstrap.rs:467-469`).

Where they run: refusal 1 in the binary's boot configuration (959-9); refusal 2 a library check (959-2) the binary
calls before `WalBootstrap::open_over_store` (`bin/topgun_server.rs:1058`); refusal 3 in `WalRecovery::run`, reached
through `open_over_store` (959-2); refusal 4 in `build_services` (959-2).

#### Assertion per call: the message type

- The class is asserted by the **message type**, not by a field. Ephemeral writes travel in their own client→server
  message, `EPHEMERAL_WRITE {requestId, ops}`, answered by its own server→client message,
  `EPHEMERAL_WRITE_RESP {requestId, results}` (D10.6). `OP_BATCH`, `CLIENT_OP` and an HTTP `/sync` push assert
  `"durable"`; `EPHEMERAL_WRITE` asserts `"ephemeral"`. Neither `OP_BATCH` nor `ClientOp` gains a field.
- **Server-side carrier.** `OperationContext` (`service/operation.rs:73-99`) gains `storage_class: StorageClass`, set
  to `Durable` by `OperationContext::new` (`service/operation.rs:111-122`). The WebSocket handler sets it to
  `Ephemeral` (`Operation::set_storage_class`, beside `set_connection_id` / `set_principal`) on every operation it
  builds from an `EPHEMERAL_WRITE`, and on no other. Every other path — `OP_BATCH` through the verdict fold and its
  per-partition rebuild, `CLIENT_OP`, HTTP `/sync` — keeps `Durable` from `new`, with nothing to carry and nothing to
  forget: a path that dropped the class would assert `Durable`, and a durable assertion on an ephemeral map is
  refused (loud, never a silent durable write).
- The server compares `ctx.storage_class` with the resolved class of every op's `mapName` in the validation pass of
  `handle_op_batch` (`service/domain/crdt.rs:395-471`), after `admit_write` (so an unauthorised op is refused with 403,
  not told the map's class) and before schema validation, in all three branches (connection, `HttpClient`,
  `Anonymous`), and at the same point of `handle_client_op` (`service/domain/crdt.rs:294`). A mismatch in either
  direction is refused with `storage_class_mismatch` (D10.1). The check runs before anything of the sub-batch is
  applied, so the refusal is Permanent with nothing applied (`TG-SYNC-003` holds) and `is_redispatchable` is `true`
  for it; on the `OP_BATCH` path that lets the existing verdict fold re-dispatch the sub-batch op by op and give the
  other ops their own verdicts (`network/handlers/websocket.rs:1344-1389`).
- **One type.** `StorageClass` is defined once, in `core-rust/src/messages/sync.rs` (serialised `"durable"` /
  `"ephemeral"`), by 959-1; the server's resolver and `OperationContext` use that type. No second server enum, no
  conversion.
- The ephemeral client handle sends only `EPHEMERAL_WRITE`; the durable `SyncEngine` never sends it and never receives
  `EPHEMERAL_WRITE_RESP`. Effect: a legacy or durable client writing to an ephemeral map is refused instead of
  silently filling IndexedDB and the op log, and the handle writing to a map that is now durable is refused too.

#### Also refused with `storage_class_mismatch` on an ephemeral map

- every OR op (in the `handle_op_batch` validation pass, 959-5a);
- **every `SyncService` operation** whose map resolves ephemeral: `SYNC_INIT`, `MERKLE_REQ_BUCKET`, `ORMAP_SYNC_INIT`,
  `ORMAP_MERKLE_REQ_BUCKET`, `ORMAP_DIFF_REQUEST` and `ORMAP_PUSH_DIFF`, by one check in `SyncService`'s dispatch
  (`service/domain/sync.rs:1938-1955`), by the payload's map name, before the handler runs (959-5b).
  - Edge case that decides it: `handle_ormap_push_diff` (`service/domain/sync.rs:1589`) merges OR records **and
    tombstones** into the map's store through `get_or_create` (`service/domain/sync.rs:1627-1629`), outside
    `handle_op_batch`. Without the check a pushed diff would store OR records with no owner, outside both budgets,
    never removed by `owner_gone` or `expired`, with tombstones — breaking D0, D1, `TG-EPH-004` and `TG-EPH-007`.
    Refusing only the two sync-init messages would leave the push, bucket and diff requests open; refusing at the
    dispatch covers all six in one place, and a new `SyncService` operation is covered by the same check.
- HTTP `/sync` push or pull; HTTP has no connection that could own the record (D3).
  - **Push** needs no check of its own: its operations are built through `OperationContext::new` and so assert
    `Durable`, and the class check in `handle_op_batch` refuses an ephemeral-map op of a push with 409.
  - **Pull** does not go through `handle_op_batch`, so `network/handlers/http_sync.rs` refuses a pull of an ephemeral
    map itself (959-5b), with an error entry carrying code 409.
- `SEARCH`, `SEARCH_SUB`, `HYBRID_SEARCH` and `HYBRID_SEARCH_SUB` (the search service handles them,
  `service/domain/search/mod.rs:1493-1499`; D1 "Observers"; 959-5b);
- `VECTOR_SEARCH`, which `QueryService` handles (`service/domain/query.rs:701`), consistent with boot refusal 4
  (959-5b);
- registering a vector index on an ephemeral map through the admin API (959-9).

#### Per-op verdicts of an ephemeral write

The owner rule (D3) and the budget check (D8) are exact only under the key writer, at apply. In the `OP_BATCH`
contract an apply-time refusal would come after earlier ops of the sub-batch are applied, and a transient error names
no op: the exchange ends in one `ERROR` and the client re-sends the whole batch (the `transient` sink,
`network/handlers/websocket.rs:1320-1323`, `network/handlers/websocket.rs:1556-1575`). One key held by another owner
(the guest → login case lasts up to `G + S`) would hold every other key of its sub-batch hostage, re-apply the earlier
ones on every retry, and leave the handle unable to say which key is blocked. The ephemeral write therefore has its
own dispatch and its own response.

- **Dispatch: one singleton per op.** The WebSocket handler intercepts `EPHEMERAL_WRITE` in `dispatch_one_message`,
  beside the `OP_BATCH` intercept (`network/handlers/websocket.rs:930-933`), so a top-level frame and one inside a
  transport `BATCH` envelope take the same path. It groups the ops by partition and dispatches **each op as its own
  singleton `Operation::OpBatch`** (built by `classify_op_batch_for_partition`, `service/classify.rs:83-110`, with no
  write concern or timeout, then `set_connection_id`, `set_principal` and `set_storage_class(Ephemeral)`): in request
  order within a partition (two writes of one key keep their order), concurrently across partitions. It does **not**
  go through the `OP_BATCH` verdict fold.
- **Verdicts: positional.** `results[i]` is the verdict of `ops[i]`: exactly one per op, in request order, so the
  response needs no op ids and an op without an id is no special case (`ClientOp.id` is ignored on this path). Each
  singleton's outcome maps to one of three statuses, using the 959-3 methods on `OperationError`:
  - `applied` — the singleton succeeded;
  - `refused {code}` — a Permanent error for which `is_redispatchable()` is `true` (403, 400, 409, 413, 422): an
    admission refusal decided before apply, which a retry cannot change;
  - `retry {code}` — a Transient error (423, 507, 401, 429, 500, 504), **or** a Permanent error for which
    `is_redispatchable()` is `false` (`WrongService`, `UnknownService`): those describe server misrouting, and the
    `OP_BATCH` fold keeps them out of per-op refusals for the same reason (`network/handlers/websocket.rs:1211-1242`,
    `network/handlers/websocket.rs:1344-1348`). A server fault never makes the handle drop a key.
- **Response.** One `EPHEMERAL_WRITE_RESP {requestId, results}` per request, after every singleton has finished. No
  exchange-level `ERROR` frame and no exchange-level `transient` sink on this path: with one op per singleton every
  outcome names its op. An empty `ops` is answered with an empty `results`. A request is not dispatched, and so not
  answered, when its frame is lost, over the envelope cap, or behind a closed lane (below); the handle's request
  timeout covers all three.
- **Before dispatch.** `message_cost` (`network/handlers/websocket.rs:719-725`) charges an `EPHEMERAL_WRITE` its op
  count, like `OP_BATCH` (`TG-RATE-001`: one frame cannot carry many ops for the price of one). A frame the rate
  limiter refuses is answered, best-effort like today's 429 (`network/handlers/websocket.rs:401-430`), by an
  `EPHEMERAL_WRITE_RESP` whose every result is `retry {429}`; nothing of it was dispatched.
- **A refused `BATCH` envelope.** The rate limiter charges an envelope the sum of its inner costs and refuses it whole
  (`network/handlers/websocket.rs:401-430`). The read loop still sends the generic `ERROR` 429 first, unchanged
  (durable clients rely on it), then, best-effort in the same way, one `EPHEMERAL_WRITE_RESP` per inner
  `EPHEMERAL_WRITE`, in envelope order, each with every result `retry {429}`. An **over-cap** envelope (more than
  `MAX_BATCH_INNER_ITEMS` = 8192 items, `network/handlers/websocket.rs:671`) is answered, as today, by one `ERROR` 413
  from its task and nothing else (`network/handlers/websocket.rs:1646-1662`): `UnpackedEnvelope::OverCap` keeps only
  the item count, so its inner requests are not known and stay unanswered (also when the rate limiter refuses an
  over-cap envelope: only the `ERROR` 429). The handle's request timeout covers them like any lost answer.
- **What a retry re-applies.** Each singleton applies at most one op, and a `refused` or `retry` one applied nothing.
  Every op is named in the response, so the handle re-sends only the keys whose verdict was `retry` (with its own
  backoff) and never a key that was `applied`: **within one answered request, no op is re-applied by a retry of
  another op**. The one exception is a request with **no** answer (lost response, disconnect, request timeout): the
  handle cannot know which ops were applied, re-sends the current desired state of every key whose latest request it
  is, and re-applies those that were applied — LWW-idempotent (the value is the desired state), at the cost of a
  duplicate fan-out per re-applied key.
- **No atomicity loss.** Ephemeral ops carry no cross-op atomicity requirement: the handle sends the current desired
  value per key (D7), each op is independent, and a request is only a transport grouping.

#### Order across requests of one connection: the ephemeral lane

Without a rule two `EPHEMERAL_WRITE` frames of one connection can apply out of order: the read loop spawns one task
per frame (`network/handlers/websocket.rs:459-465`), up to `MAX_IN_FLIGHT` = 32 at once
(`network/handlers/websocket.rs:51`), and the server stamps a write at apply (`service/domain/crdt.rs:975-978`), so
the op applied last wins LWW whatever order the client sent. A re-send after a timeout, or a second request sent while
the first is unanswered (a cursor move), would let an older value land after a newer one, and the handle, having
marked the key `published` from the newer request's answer, would never correct it.

**The server processes one connection's `EPHEMERAL_WRITE` requests in arrival order.** The handle may have any number
of requests outstanding.

- **Where it lives.** In `network/handlers/websocket.rs` only (959-5a). The read loop, which is sequential
  (`network/handlers/websocket.rs:322-466`), gives every frame that holds at least one `EPHEMERAL_WRITE` — a top-level
  one, or an envelope whose unpacked messages include one — a **lane ticket** when it hands the frame to its task
  (`network/handlers/websocket.rs:461`). A ticket is a link in a per-connection chain: it holds the receiver of its
  predecessor's completion signal and the sender of its own (a `oneshot` pair: no lock, no task). It is made after the
  permit and after the rate-limit check, so a refused frame holds no link. Frames with no `EPHEMERAL_WRITE` get no
  ticket and are unaffected.
- **Taking the turn.** `dispatch_one_message`, at its `EPHEMERAL_WRITE` intercept, awaits its ticket's turn (the
  predecessor's signal, or the predecessor's sender being dropped) before it dispatches any singleton. The ticket is
  released when the frame's task ends, by dropping it, so a task that panics releases it too. One request's singletons
  all finish before any singleton of the next request of that connection starts. Because one key maps to one
  partition worker whose queue is FIFO (`service/dispatch.rs:93`, `service/dispatch.rs:162-184`), even a predecessor
  singleton answered `retry {504}` but still queued applies before the successor's op of that key.
- **Envelopes.** One ticket per frame. An envelope's task already walks its inner messages in order and awaits each
  (`dispatch_envelope`, `network/handlers/websocket.rs:1631-1680`; `TG-DISP-001` (a)), so several `EPHEMERAL_WRITE`s
  in one envelope apply in envelope order, and the envelope as a whole takes its place in the lane where the read loop
  read it. `TG-DISP-001` is unchanged: `dispatch_one_message` stays the one entry on both paths; the ticket is an
  argument threaded through `dispatch_message` and `dispatch_envelope` (`network/handlers/websocket.rs:879`,
  `network/handlers/websocket.rs:919`, `network/handlers/websocket.rs:1631`, all private to that file and called only
  there). Inner messages before the envelope's first `EPHEMERAL_WRITE` do not wait for the turn; everything after it
  in the envelope waits behind it, as it does today.
- **Bound and back-pressure.** The lane has no queue of its own: a ticket lives only in a frame task, and every frame
  task holds one of the connection's 32 permits (`network/handlers/websocket.rs:452-454`). So at most 32 requests of
  a connection are in the lane, and when 32 frames are in flight the read loop waits for a permit — today's
  back-pressure for every frame of the connection: no `retry {429}`, no close.
- **Connection end.** The read loop marks the lane closed when it leaves the loop for any reason (close frame, error,
  or the cancel token fired by takeover or the reaper, `network/handlers/websocket.rs:326-333`). The closed flag is
  checked at **every** `EPHEMERAL_WRITE` intercept, after the turn is taken and before any singleton of that request
  is dispatched — not once per ticket: one ticket per frame means an envelope's later requests already hold the turn,
  so a once-per-ticket check would let an envelope in progress apply all its remaining requests after a takeover. A
  request whose intercept finds the lane closed is neither dispatched nor answered; the handle re-publishes on its
  next transition to connected (D5). The envelope's non-ephemeral inner messages after a skipped request are
  dispatched as today. The request in progress at the close finishes, because the handler waits for every in-flight
  task before `registry.remove` (`network/handlers/websocket.rs:488-506`).
- **The handle's rule (959-9).** For each key the handle records the `requestId` of the latest request that carried
  it. A verdict for a key counts only when it comes from that request; a verdict from an earlier request is ignored,
  because the later request applies after it on the lane. A request unanswered after the handle's **request timeout**
  marks the keys whose latest request it is `retrying` and re-sends their current desired state in a new request,
  which becomes their latest. If the original is still on the server, the re-send is behind it on the same lane and
  applies after it, so an answer that is only slow is never overtaken; if the connection changed, the original either
  finished or was dropped at the lane close, apart from the one-request window below. The request timeout therefore
  needs no relation to the server's operation timeout: it bounds only how long a lost answer is waited for, and 959-9
  pins it by test as a handle constant. Nothing here claims that it exceeds the operation timeout.
- **Durable `OP_BATCH`.** The same mechanism (one task per frame, stamp at apply) can reorder two `OP_BATCH` frames of
  one connection touching one key; that question is TODO-478's and is not changed here.

#### Handshake

- `AUTH_ACK` and `DEVICE_ACK` (`AuthAckData`, `DeviceAckData`, `core-rust/src/messages/client_events.rs:120-156`) gain
  an optional `storageClassHash`.
- It is lowercase hex of the first 16 bytes of SHA-256 over `"tg-storage-class-v1\n"` followed by the sorted,
  de-duplicated patterns joined by `"\n"`.
- It is **diagnostic**; correctness rests on the per-call assertion. The client exposes it and logs a warning when it
  changes between connections; the cluster join compares the same value (D9); agents (TODO-960) key a cached pattern
  list by it.
- If TODO-957's capabilities object lands first, the field moves inside it under the same name.

#### Admin API

- `GET /api/admin/storage-classes` returns `{patterns, hash, ephemeral: {bytes, entries, owners, ownersInGrace}}`.
  Counts only: no owner identity is returned (D3).
- It follows the existing admin auth (tokens with a tenant claim are refused under tenancy).
- It reaches the resolver and the registry through `AppState.store_factory` (`network/handlers/mod.rs:63-103`); no
  `AppState` field is added.

#### Schema mirror

A schema-DSL mirror (`defineMap(..., {storageClass})`) is **not** part of v1. The server list is authoritative.
Deferred (TODO-959).

### Options considered

**Declaration.**
- (a) A boot-time server glob — chosen (conductor ruling).
- (b) Per-map metadata set through the admin API at runtime. Rejected: a map's class could change under live
  records and an open WAL; refusals 2 and 3 exist because the class must be fixed before recovery.

**The class carrier and the verdict route.**
- (a) The `EPHEMERAL_WRITE` / `EPHEMERAL_WRITE_RESP` pair — chosen.
- (b) A class field on `OP_BATCH` plus a namespaced op id (for example `e:{n}`) and server acks for non-numeric ids.
  Rejected on its edge cases. The client routes every `OP_ACK` / `OP_REJECTED` to `SyncEngine`
  (`client/src/sync/ClientMessageHandlers.ts:199-200`), which matches them by op id against IndexedDB row ids
  (`client/src/SyncEngine.ts:994`); a `permanent: true` refusal retires and deletes the op with that id
  (`client/src/SyncEngine.ts:3554-3575`) and a `results` entry marks it synced (`client/src/SyncEngine.ts:2026-2068`).
  *Numeric* handle ids collide with durable op ids: an ephemeral 409 would retire and delete the durable op with that
  id, a `results` entry would mark it synced — loss of a durable write. *Non-numeric* ids get no partial ack: the
  server sends no `OP_ACK` when no accepted id is numeric (`network/handlers/websocket.rs:1603-1613`), because a
  non-numeric `lastId` marks every pending op synced (`network/handlers/websocket.rs:1409-1427`), so accepted keys
  are never confirmed and are re-sent with every retry of a blocked key. Making it work needs both client handlers to
  learn to skip foreign ids (one forgotten check retires a durable op), a new ack shape, and a carrier field on
  `OpBatchPayload` (5 production files force on its literals).
- (c) A class field on `ClientOp` (`core-rust/src/messages/base.rs:210-251`): about 14 forced production files, and
  (b)'s routing problem unsolved.

**Ephemeral verdicts.**
- (a) Singleton dispatch with positional verdicts in a dedicated response — chosen.
- (b) An admission pre-check in the validation pass plus a re-check at apply. Rejected: *race* — between the pass and
  the apply of a later op another connection can take the key or fill the budget, so the re-check can still refuse
  after earlier ops applied, and (b) still needs (a)'s per-op verdict; *hostage* — the validation pass is atomic for
  the sub-batch (`service/domain/crdt.rs:395-471`), so one key's pre-check refusal refuses every key.
- (c) Per-op transient verdicts collected inside one multi-op sub-batch. Rejected: a new response shape from the CRDT
  service through the middleware stack, where (a) needs only the existing singleton `Operation::OpBatch`.
- (d) Per-op verdicts inside the `OP_BATCH` exchange (`OP_REJECTED{permanent: false}` plus `OP_ACK.results`).
  Rejected: those frames reach `SyncEngine` and are matched against durable op ids — the routing defect of carrier
  option (b).

**Order across requests.**
- (a) Arrival order per connection on the server (the lane) — chosen: overlapping requests and re-sends are safe
  with no wire field and no handle-side queue, in one file already in 959-5a.
- (b) At most one outstanding request per handle, newer values coalesced into the next request, plus a server
  deadline after which unstarted singletons answer `retry {504}`. Rejected: two mechanisms where (a) is one, and a
  cursor move would wait a full round trip behind any slow request.
- (c) A per-key handle sequence compared at apply. Rejected: a wire field on every op and per-key server state for
  the one-request cross-connection window (a) leaves; recorded as the way to close that window if it ever matters.

**Lane bound.**
- A separate bounded lane queue answering `retry {429}` when full. Rejected: a second bound beside the permit bound,
  and a client that pipelines a burst would be told to retry what the server could simply have read later.
- Closing the connection when the lane is long. Rejected: a burst is not abuse; the rate limiter already throttles
  abuse (`network/handlers/websocket.rs:401-430`).

**Handshake.**
- Sending the pattern list to clients. Rejected: an offline-first client writes before its first connection, so a
  list learned at the handshake cannot prevent a local write; the per-call assertion is the only check that holds.

### Reason

- A dedicated response is routed by its type to the handle and never reaches `SyncEngine`, because the client's
  `MessageRouter` dispatches by message `type` (`client/src/sync/ClientMessageHandlers.ts:188-200`): the two id spaces
  are separate by construction, not by a check two handlers must both remember.
- The class needs no wire field: the carrier is one server-internal `OperationContext` field with two struct literals
  (`service/operation.rs:111-122` and the inline test at `service/middleware/timeout.rs:106`).
- Per-op verdicts are the response's own shape, not a clause added to the `OP_BATCH` exchange contract, so
  `TG-SYNC-001` (`INVARIANTS.md:2372-2388`) and `TG-SYNC-003` (`INVARIANTS.md:2443-2450`) hold **unchanged and are
  not amended**. Every Permanent refusal of an op is decided in the validation pass — the 409 there, and the 413 of a
  record that alone exceeds the per-owner budget (a size against a constant, D8) — and a singleton refused at apply
  (423, 507) applied nothing. The positional contract of `EPHEMERAL_WRITE_RESP` is part of `TG-EPH-003`.
- HTTP has no ephemeral message, so `client/src/connection/HttpSyncProvider.ts:360` ("an operation-attributed entry
  is always a permanent verdict") stays true.
- Policies match maps by glob, so the class list must be reviewable next to policies: hence the admin endpoint.
- **Cost (T1, T2).** Dispatch concurrency is at most the number of distinct partitions a request touches — the same
  as an `OP_BATCH` of those ops — so the exposure to `load_shed`'s `max_concurrent_operations` is unchanged. The cost
  is one dispatch per op; `k` ops of one partition are `k` serial dispatch round trips, as in the existing singleton
  re-dispatch (`network/handlers/websocket.rs:1360-1364`). A presence client writes a few keys, which hash to distinct
  partitions in most cases. The durable `OP_BATCH` path gains only the class comparison in the validation pass and,
  in the read loop, one check per frame for an `EPHEMERAL_WRITE` (a type match on a top-level frame; on an envelope a
  scan of its already unpacked messages, at most 8192). The lane adds no order across connections, so it does not cap
  node throughput (T1 is per node); it serialises one connection's requests, so one connection's ephemeral rate is at
  most about (ops per request) / (request latency), far above a presence client's human rate. 959-5a runs the load
  harness, fire-and-wait (CLAUDE.md requires it for `service/domain/crdt` and `network/handlers/` changes): the durable
  scenario against the base commit (a regression over 20 % needs justification), a multi-key ephemeral scenario
  reported against T1 and T2, and one connection's pipelined ephemeral throughput (reported, not gated).

### Stated limits

- **Cross-connection order is not guaranteed.** Each connection has its own lane. Under takeover
  (`network/handlers/websocket.rs:543-558`), or on a reconnect of the same handle while the server still finishes the
  old connection, the old connection's request in progress when its read loop exited — at most one per connection,
  because the closed flag is checked at every intercept — can apply after the new connection's first requests,
  including its re-publish of the same key; the older value then stands until the handle writes that key again. The
  window is that request's remaining singletons, each bounded by the operation timeout (5000 ms,
  `bootstrap.rs:82`). Two tabs of one profile writing one key are two concurrent writers under one owner (D3),
  ordered by apply as any two LWW writers are.
- **The lane's cost.** Ephemeral frames waiting for their turn hold permits, so a connection with many pipelined
  ephemeral requests delays its own durable frames until the lane advances; each waiting request waits at most for its
  predecessors' singletons, each bounded by the operation timeout. Other connections are not affected. While all 32
  permits are held by frames waiting on the lane the read loop reads nothing, so the connection's `PING` (which sets
  `last_heartbeat`, `service/domain/coordination.rs:157`) is not processed either; a lane stall longer than
  `idle_timeout` (60 s, `network/config.rs:187`) gets the connection reaped (`network/reaper.rs:119-135`). The
  serialisation makes this reachable where 32 concurrent frames were not (32 waiting requests × their singletons × up
  to 5000 ms each). The reap closes the lane, and the handle re-publishes on its next transition to connected (D5):
  the cost is a reconnect, not a lost or reordered write.
- The handshake hash is diagnostic only; a client that ignores it is still refused per call.
- A non-batch refusal (`CLIENT_OP`, the six `SyncService` operations) is answered with no frame, as every non-batch
  error other than 429 is today (`network/handlers/websocket.rs:947-967`); see D10.1 "Carriers".

---

## D3 — Ownership

### Decision

- Every ephemeral record has **exactly one owner**, stamped by the server from the writing connection and never read
  from the payload.
- The owner is the connection's `frontier_client_id(principal, device_id)` (`network/handlers/websocket.rs:546`).
- A connection with no device identity gets a connection-scoped owner `conn:{node_id}:{boot_nonce}:{connection_id}`.
- An owner is **live** on a node while a registered connection fixed to that owner exists there.

**Owner rule.**
- A PUT or REMOVE of a **live** record stamped by another owner is refused with `ephemeral_not_owner` (D10.1). The rule
  is decided at apply, under the key writer, and the refusal is that op's own `retry {423}` verdict in
  `EPHEMERAL_WRITE_RESP` (D2): it blocks that key only, and the handle reports that key as blocked.
- A PUT to a key with no live record takes ownership.
- The only exceptions: server-internal removals (`expired`, `owner_gone`) and tenant drop. Grace removal removes only
  records stamped by that owner.
- No policy override in v1. Sellers and agents act through addressed delivery (TODO-958), not by writing into other
  owners' records.

**Presence keying (the recommended pattern).** Presence keys are **per tab (per connection)**: `{userId}:{tabId}`,
with `tabId` minted per page load and kept in memory. Rooms (TODO-297) key membership the same way. Apps group by
user to count people, and by device for multi-device presence. A key shared by the tabs of one profile is allowed and
has the D7 limit.

**Liveness bookkeeping (a constraint on 959-6a).** `remove()` drops `conn_identity` before it calls the disconnect
observers, and an observer receives only a `ConnectionId` (`network/connection.rs:371-390`), so a disconnect observer
cannot look up the identity. The ephemeral registry keeps its own state, and the rules below make it exact against a
concurrent `remove()`.

- **State.** `conn_owner: ConnectionId → OwnerId`, and per owner an entry `{live: set of ConnectionId, grace deadline,
  keys}` behind the owner-entry lock (D6). **An owner is live while its `live` set is non-empty**: liveness counts the
  identity's registered connections fixed to it, not one current connection. During a takeover the old and the new
  connection are both registered until the old one's `remove()`, so the set never passes through empty and no grace
  starts.
- **Fixing a connection's owner (one operation, two triggers).** A connection's owner is fixed **once, for its
  lifetime**, by whichever comes first:
  - the identity bind: an identity-bound observer that `ConnectionRegistry` calls from `claim_device_ownership`
    (`network/connection.rs:399-428`), next to the existing disconnect observers (`network/connection.rs:381-388`),
    fixes `frontier_client_id(principal, device_id)`;
  - the connection's first ephemeral write when it has no identity yet: the write path fixes the connection-scoped
    owner.
  A later trigger for an already fixed connection is ignored by the ephemeral registry (the connection's device
  ownership in `ConnectionRegistry` is unaffected).
- **Fix, ordered against `remove()`.** Under the owner-entry lock the registry inserts `conn → owner` and adds the
  connection to the owner's `live` set (cancelling a pending grace removal, D5); then, still under the lock, it
  re-checks that the connection is registered in `ConnectionRegistry`. If it is not, the fix is **rejected**: the
  insert is undone, and if the set is now empty the owner's grace starts. The disconnect observer (`unfix`) takes the
  same lock, removes the connection from `conn_owner` and from the set, and starts grace when the set becomes empty;
  an `unfix` for an unknown connection is a no-op. Because `remove()` deletes the connection from `connections`
  (`network/connection.rs:372`) **before** it calls the observers (`network/connection.rs:381-388`), every
  interleaving ends with no dead connection in a `live` set:
  - `remove()`'s delete comes before the fix's re-check → the re-check rejects the fix;
  - it comes after the re-check → the observer fires after the fix was inserted and removes it.
  A fix for a connection already removed is rejected, and an owner can never stay live through a connection that is
  gone (`TG-EPH-005`).
- The WebSocket handler and `AppState` are not touched for this: the bind hook is in `network/connection.rs`, the
  `unfix` is an ordinary `on_disconnect` observer (as `service/domain/sync.rs:289` registers one), and the first-write
  fix is in the write path (959-6b). The `EPHEMERAL_WRITE` handler added in 959-5a only sets the operation's class; it
  reads no identity.

### Options considered

- **(a) The device-scoped identity, `frontier_client_id(principal, device_id)`** — chosen.
- **(b) The connection id as owner.** Rejected on E2 and E3 below.
- **(c) `(device, connection)`: device for grace, connection for attribution.** Rejected: under takeover no removal
  acts at connection granularity, so the connection-level attribution stamp would have no reader.

Edge cases that decide it (tabs of one profile are **one** identity, because the device credential lives under
`deviceId` in the IndexedDB database `topgun_offline_db` that every tab of one origin and profile shares —
`client/src/TopGunClient.ts:341`, `client/src/SyncEngine.ts:930-933`, `client/src/SyncEngine.ts:953-957` — and a
second connection with that identity **takes over**: the older connection is cancelled,
`network/connection.rs:281-286`, `network/handlers/websocket.rs:543-558`):

- **E2: two tabs of one profile write one key, then one tab closes.** Connection scope removes a key a still-open tab
  holds, at every close and every takeover. Device scope keeps it while either tab reconnects within grace.
- **E3: two tabs write different keys (the recommended keying), then one closes.** Connection scope would remove the
  other tab's keys at every takeover too, since only one connection per identity exists on a node — it fails for this
  keying as well. Under device scope the closed tab's keys stay until the profile's last connection is gone plus
  grace, until the handle's `dispose()` removes them (called on `pagehide`), or until their `ttlMs`.
- **E4: an identity-less connection.** Device binding fails open: a connection has no device identity when binding
  errs (`network/handlers/websocket.rs:570-576`) or when there is no store factory
  (`network/handlers/websocket.rs:529-531`). It is covered by the connection-scoped fallback.

**Why the owner is fixed for the connection's lifetime.** A fail-open bind leaves `device_id` `None`, and a later
`DEVICE_HELLO` on the same connection can still bind one, because `DEVICE_HELLO` binds only while `device_id` is
`None` (`network/handlers/websocket.rs:354-389`).
- *If the owner followed the bind:* the connection's next write to a key it wrote before would meet its own
  connection-scoped record as another owner's and be refused with 423 until that record's grace ended — a connection
  blocking itself.
- *If identity-less ephemeral writes were refused until the identity phase ends:* the server cannot know when it ends
  (a NO_AUTH client may get no `DEVICE_ACK` and connects through its grace timer, `client/src/SyncEngine.ts:767-789`;
  a server with no store factory never binds), so such clients could never publish presence.
- *Fixed for the lifetime:* the connection keeps writing as its connection-scoped owner; its records go `G + S` after
  it closes; its next connection, which binds the identity, writes as the identity and meets those records with 423
  for up to `G + S` — the same window as guest → login. In the normal order (`DEVICE_HELLO` is the first frame, and
  frames after it are read only once it is handled) the bind comes first and the owner is the identity.

### Reason

- The identity is bound in `claim_device_ownership`, which inserts the reverse index, then the forward ownership, then
  re-checks that the connection is still registered and reconciles a dead claim (`network/connection.rs:399-428`);
  the ephemeral fix copies that insert-then-re-check shape under its own lock.
- Deciding at apply under the key writer is the only place the owner rule is exact (D2).

### Stated limits

- The owner rule does not reserve a key for an identity before it writes, or after its record is removed: a client can
  take a key someone else has not written yet. Per-visitor key reservation needs resource templating
  (TODO-858 / TODO-958).
- **E3, crashed tab.** A crashed tab misses `pagehide`, so its keys stay until the profile's last connection is gone
  plus grace, or their `ttlMs`.
- **E4 then `DEVICE_HELLO`.** A connection whose first ephemeral write came before it had an identity keeps its
  connection-scoped owner for its lifetime.
- **Guest → login.** The owner identity includes the principal, so a guest who logs in (TODO-862 item 6, TODO-858)
  becomes a new owner. The guest record keeps the key, and the new identity's write is refused with 423 for up to
  `G + S` after the guest identity stopped being live. 423 is transient, so the handle retries and succeeds once the
  guest record is removed.
- **The owner identity is never sent to another principal.** It carries the device id, so it lives only in the
  ephemeral registry: not in the record value, a snapshot, a delta, a journal event or admin output (counts only).
  Cluster-internal messages (D9) may carry it.
- **Takeover loop (TODO-747, TODO-972).** Whether a displaced tab reconnects and takes the identity back in a loop is
  **unverified**. Under such a loop the owner stays live, because each reconnect lands within grace, and each reconnect
  re-publishes (D5): the loop costs writes, not correctness. Because two open tabs are the common case the cost is not
  accepted blind: **TODO-747 and TODO-972 traced is a precondition of 959-9**, the step that ships the client handle.

---

## D4 — Lifetime, TTL and renewal

### Decision

- An owned record lives while its owner is live, plus a grace period `G` (`TOPGUN_EPHEMERAL_GRACE_MS`, default 30000,
  range 0–600000; outside the range is fatal).
- **Renewal is the connection itself.** An owner is live while its connection is registered; the connection is kept
  by the client's app-level `PING` and reaped at `idle_timeout`. There is **no renewal message**.
- **TTL is optional per record.** A PUT may carry `record.ttlMs` (`core-rust/src/hlc.rs:523`). It yields a deadline
  = the owner node's **monotonic receive instant** + `ttlMs`; it is renewed by re-writing the record; it is stripped
  before apply (D1) and never stored. `TG-TTL-001/002` do not apply to ephemeral maps; `TG-EPH-006` replaces them. Use
  case: a typing indicator that ends 5 s after the last keystroke while the tab stays connected.
- **Expiry base: the receive instant, not the HLC stamp.**
- **No jitter on the server.**
- **Removal tick `S` = 1000 ms**, a constant, not a variable. The tick reads a deadline queue and the owner index and
  never scans rows. Removal is chunked, with a backlog gauge. Each removal is the ordinary REMOVE apply and fan-out
  with `System` origin, re-checked under the key writer, like `ExpireRecord` (`service/domain/crdt.rs:999-1014`). The
  tick calls the removal directly (a remover trait the CRDT service implements), concurrently across keys with a
  bounded number in flight; it does not go through the operation pipeline one awaited op at a time.

**Bounds, stated as `TG-EPH-005/006`, each while the removal backlog is 0:**
- **TTL'd record:** readable and in live-query windows until at most deadline + `S` — the `TTL + sweep interval`
  bound. Reads are not filtered at the deadline.
- **Clean close:** removed at most `G + S` after the close.
- **Half-open peer, single node:** removed at most `idle_timeout + reaper_interval + G + S` = **101 s** with defaults
  (`idle_timeout` 60 s and `reaper_interval` 10 s, `network/config.rs:187-189`). For states that must vanish faster,
  use `ttlMs`, and lower `G` where flicker is acceptable.
- **Cross-node:** not reachable in v1 (D2 refusal 1). After the lift (TODO-850) the bound is the smaller of the TTL
  deadline + `S` and close detection + `G` + the lease lapse.

### Options considered

- **Renewal: (a) the connection (chosen); (b) mandatory TTL with a renew heartbeat** (RESEARCH C3, second opinion
  "missing" item 1). Rejected: at T4 (10⁵ connections) renew traffic is O(owned keys) per interval and carries no
  information; connection liveness is O(connections) and already exists. Timer throttling is *not* a reason: the
  `PING` that keeps a connection live is itself a JS timer, so a throttled background tab would miss renewals and
  pings alike — throttling does not separate the options.
- **Expiry base: (a) monotonic receive instant (chosen); (b) the HLC stamp's millis.** Rejected: the stamp is the
  server's HLC (`service/domain/crdt.rs:975-978`), which follows the wall clock, so a forward wall-clock step would
  expire a whole cohort at once, and an HLC pushed ahead by a skewed peer would extend lifetimes past `ttlMs`. A
  monotonic instant is immune to both and is never persisted, which is correct for a class that never leaves the
  process.
- **Jitter: (a) none (chosen); (b) ±10 % on the deadline.** Rejected: shortening a deadline flickers a record whose
  owner re-writes it on time; lengthening it breaks the bound; owned records do not expire by time, so no cohort forms
  after a restart; the client's reconnect jitter already spreads re-publishing.
- **Removal driver: (a) a direct remover (chosen); (b) the TTL sweep's dispatch path.** Rejected: the sweep dispatches
  one op at a time and awaits each (`service/expiry.rs:601`) behind a queue of 1024 (`service/expiry.rs:107`), which is
  its П ceiling.
- **`G` default: (a) 30000 (chosen); (b) 45000+, to cover a capped reconnect attempt.** Rejected: it lengthens every
  departed visitor's presence by as much.

### Reason

**Liveness is the client's app-level `PING`.** Server liveness counts only the client's `PING`
(`service/domain/coordination.rs:153-158`); WebSocket ping/pong frames do not count
(`network/handlers/websocket.rs:276-278`). The client sends `PING` from a JS timer every 5 s
(`client/src/SyncEngine.ts:478-482`).

**Sizing `G` against the client backoff.** The client's reconnect backoff starts at 1000 ms, multiplier 2, cap
30000 ms (`client/src/connection/SingleServerProvider.ts:26-28`), with jitter 0.5–1.5× applied **after** the cap
(`client/src/connection/SingleServerProvider.ts:428-436`). Attempt `n` (from 0; `n` is `reconnectAttempts` before its
increment) waits `d_n = min(1000·2ⁿ, 30000) × U(0.5, 1.5)` ms after attempt `n − 1` **failed**. A failed attempt ends
when the connect is refused (about 0 s) or at its per-attempt timeout of `reconnectDelayMs × 5` = 5 s
(`client/src/connection/SingleServerProvider.ts:179-189`) when the outage black-holes connects; only then is the next
delay scheduled (`client/src/connection/SingleServerProvider.ts:406-420`).
- **Refused connects.** Attempts `n = 0..3` fire within 1.5, 4.5, 10.5 and 22.5 s of the close at the latest; attempt
  `n = 3` fires no earlier than 7.5 s (`n = 4` no earlier than 15.5 s and up to 46.5 s).
- **Black-holed connects.** Each failed attempt adds up to 5 s: attempt `n = 3` fires between 22.5 and 37.5 s, attempt
  `n = 2` between 13.5 and 20.5 s.
- **Guarantee, for any mix of refused and timed-out attempts:** a client whose server becomes reachable again within
  **7.5 s** of the close starts a connect attempt within **24.5 s** of the close, whatever the jitter.
  *Proof.* Let attempt `k` be the last one fired before 7.5 s (attempt 0 always is, at ≤ 1.5 s). Attempt 3 never fires
  before 7.5 s (its delays alone sum to at least 0.5 + 1 + 2 + 4 s), so `k ≤ 2`. Attempt `k + 1` fires after attempt
  `k` ended (at most 5 s after it fired) plus `d_{k+1} ≤ 1.5 × 8000` ms, so before 7.5 + 5 + 12 = 24.5 s, and it fires
  after 7.5 s, when the server is reachable. The handshake then has 5.5 s before `G` = 30 s; the reconnect binds the
  identity and cancels the grace removal (D5). With refused connects only, the attempt starts within **22.5 s**.
- Times are measured from the close as both sides see it (a clean close). On a half-open peer the server detects the
  close later, which only adds slack.

**Why the removal tick does not inherit the sweep's ceiling:** see [Acceptor](#acceptor), T11.

### Stated limits

- Between 7.5 s and `G` the reconnect depends on the jitter and on how attempts fail: the pending attempt may be
  `n = 4`, which can fire up to 46.5 s after the close with refused connects, and later with timeouts. A client that
  has reached the cap waits 15–45 s between attempts, so it has usually been away longer than `G`; its reconnect
  re-publishes (D5): flicker, not loss. `G` covers the first backoff steps, not the capped backoff.
- **A hidden, throttled tab.** If a browser throttles a hidden tab's `PING` timer below one ping per `idle_timeout`
  (60 s), the server reaps the connection, the owner stops being live (if no other tab of the profile is connected),
  and its records are removed `G + S` later. When the tab becomes visible it reconnects and re-publishes: flicker, not
  loss. Whether and when this happens per browser is unverified (TODO-973).
- The half-open bound is 101 s with defaults; nothing here promises 30 s on a half-open peer.
- A TTL deadline is not a read filter: a TTL'd record stays readable up to `S` past its deadline.

---

## D5 — Restart, reconnect storm and ordering

### Decision

**Restart.** An ephemeral map is empty after any process start. Clients re-publish their **current desired state**
(the last value per key, not an op log) on **every transition of the client to the connected state** — the wiring
that resubscribes queries (`handleAuthAck`). That covers `AUTH_ACK`, `DEVICE_ACK` and the NO_AUTH client that gets no
`DEVICE_ACK` and connects through its grace timer (`completeAuthOptionalConnection` → `handleAuthAck`,
`client/src/SyncEngine.ts:767-789`), which a trigger on the two ack frames would miss.

**Spreading the storm.** The existing reconnect jitter (0.5–1.5× of a 1000 ms first delay,
`client/src/connection/SingleServerProvider.ts:428-436`) spreads reconnects. Re-publish volume per client is bounded
by the per-owner cap (D8). Server overload answers with the existing `429 Overloaded` (transient), as a `retry {429}`
verdict per op (D2); the handle retries those keys with its own backoff and does not re-send keys that were
`applied`.

**Coalesced fan-out (decided here, built by TODO-931).**
- **What is coalesced:** row deltas of a live-query subscription, per `(subscription, key)`.
- **This is new work, not a setting.** Today's outbound queue is a bounded `mpsc` of already-encoded bytes
  (`network/connection.rs:45-50`, `network/connection.rs:82`), capacity 256 (`network/config.rs:186`): a queued item
  cannot be found or replaced and carries no `(subscription, key)`. TODO-931 builds a **keyed per-connection delta
  stage** in front of that channel, which drains into the channel as it has room. Frames that are not row deltas keep
  the channel and its existing policy.
- **(a) Structured until dequeue.** An entry in the stage is `{queryId, key, changeType, value}`, not bytes. The fold
  rewrites `changeType`, so the frame is built and encoded **per connection at dequeue**. TODO-931's encode-once
  applies to the value, which an entry may hold as shared encoded bytes; the frame around it is per connection.
- **Window:** the time a delta waits in the stage. No timer: a delta is never held back when the channel has room, so
  steady-state latency is unchanged.
- **Rule:** when a delta for `(subscription, key)` arrives while an undelivered one for the same pair is in the stage,
  the held one is replaced in place (keeping its position) by the fold: `ENTER+UPDATE → ENTER(new)`,
  `ENTER+LEAVE → nothing`, `UPDATE+UPDATE → UPDATE(new)`, `UPDATE+LEAVE → LEAVE`, `LEAVE+ENTER → UPDATE(new)`.
- **(c) Sizing in keys, not messages.** With one entry per `(subscription, key)`, the stage holds at most the sum of
  the connection's subscription result sizes, whatever the update rate. Its bound is set in entries and bytes, by
  TODO-931, so that the whole result of the largest live query the server admits fits.
- **(b) Overflow never drops a single delta.** When the stage reaches its bound, it discards every pending entry of the
  subscription that holds the most entries and marks that subscription for a **re-snapshot**: at dequeue the
  subscription gets a fresh `QUERY_RESP` for its `queryId`, computed then. Requirement: the client treats a
  `QUERY_RESP` for an active `queryId` as a full replacement of that query's rows (rows absent from it leave); TODO-931
  verifies the client does, or makes it so. Until it does, the fallback is to disconnect the watcher — never a silent
  drop.
- **Owner:** TODO-931's fan-out stage, off the key writer, because per-connection queues and encode-once live there. It
  applies to every map, durable or ephemeral. 959-9 (enablement) depends on TODO-931, so no shipped binary meets the
  storm without it.
- Aggregate subscriptions coalesce by their own window (D10.4).

**Ordering, normative.**
- A reconnect of an identity cancels that owner's pending grace removal **at bind time**, before any op of the new
  connection is applied.
- Every removal (`owner_gone`, `expired`) is re-checked under the key writer:
  - `owner_gone` removes only if the record is still stamped by that owner **and** the owner is not live at the check;
  - `expired` removes only if the record's deadline is still the one the tick saw.
- A write between the tick and the check therefore always wins.
- No in-grace state survives a restart.

**Failover (cluster).** Not reachable in v1 (D2 refusal 1). After the lift a new partition owner starts empty; the
client cannot know that its records' partition moved, so the reserved server→client `EPHEMERAL_REPUBLISH` (D10.6) is
sent by the node that holds the client's connection. Emitting and handling it belong to the lift (TODO-850).

### Options considered

- **Coalescing: (a) a keyed stage with queue replacement and no timer (chosen); (b) a fixed coalescing timer.**
  Rejected: it adds its full length to every delta in steady state, to save work the replacement rule already saves
  under backlog.
- **(c) A server-advised re-publish jitter in the handshake.** Rejected: it duplicates the reconnect jitter.
- **(d) A boot-time "settling window" that defers watcher snapshots.** Rejected: the server cannot know when
  re-publishing is complete, and for the length of the window the watcher's view is empty or late.
- **Overflow: (a) re-snapshot one subscription (chosen); (b) today's drop-then-disconnect policy.** Rejected: today's
  `try_send_broadcast` drops one event when the channel is full and disconnects only after 256 consecutive drops
  (`network/connection.rs:135-171`, `network/config.rs:194`), and a dropped ephemeral delta has no Merkle repair, so a
  watcher would stay silently wrong until then. A disconnect would also make the watcher re-subscribe everything and
  re-publish its own ephemeral state into the storm; a re-snapshot costs one result for one subscription.
- **Re-publish trigger: (a) the transition to connected (chosen); (b) the `AUTH_ACK` / `DEVICE_ACK` frames.**
  Rejected: it misses the NO_AUTH client that connects through its grace timer.

### Reason

A TODO-862 watcher over several hundred present visitors gets one delta per re-published key in every restart storm,
which overflows a 256-message queue every time but fits a keyed stage that holds one entry per visitor. The re-check
rules make the tick-vs-write race decided by the key writer, which every write already holds (`TG-KEY-001`).

### Stated limits

- Coalescing does not exist until TODO-931 lands; 959-1..959-8 do not change fan-out code, and 959-9 waits for it.
- Until TODO-931 verifies the client's full-replacement handling of `QUERY_RESP`, overflow disconnects the watcher.
- A reconnect storm costs one re-publish per key per client; it is bounded by the per-owner cap, not avoided.

---

## D6 — Resurrection without tombstones

### Decision

- An owned ephemeral write is applied only while its owner is **live** at apply time: the originating connection is
  in its owner's `live` set (D3) on a single node (and, after the cluster lift, the owner lease holds). Otherwise it is
  dropped; no ack is deliverable, because the connection is gone. Each drop is counted
  (`topgun_ephemeral_dropped_dead_owner_total`).
- Index insertion and the liveness check are ordered against that owner's removal by one owner-entry lock:
  - the disconnect marks the owner not live under its entry lock before removal enumerates the owner's keys;
  - a write checks liveness and inserts the key into the owner index under the same entry lock;
  - so a record cannot be inserted for an owner after its removal enumerated the owner's keys. This includes `G = 0`.
- **Lock order: key writer → owner entry**, never the reverse. Removal snapshots the owner's key list under the entry
  lock, releases it, then removes each key under its key writer with the D5 re-check. No deadlock is possible.
- Invariant (`TG-EPH-005`): **no record stamped by owner X exists more than `G + S` after X stopped being live**. This
  holds on a single node; the cross-node form follows D4 after the lift.
- A client re-publishing after a removal is the owner asserting current state, not resurrection.

### Options considered

- **(a) Liveness at apply (chosen).**
- **(b) A `last_removed_at` guard per removed key** (second opinion item 5). Evaluated and rejected: the server mints
  every client write's stamp at apply (`StampPolicy::Mint` → `sanitize_hlc()` under the key writer,
  `service/domain/crdt.rs:975-978`), so a delayed op carries a fresh stamp greater than any removal stamp, and the
  guard would refuse nothing. Its memory would also need its own expiry.
- **(c) Tombstones.** Rejected: they bring back the reclamation the class exists to avoid (D0, D1).

### Reason

Liveness at apply closes the one remaining resurrection source: an op queued in the dispatcher before the disconnect
and applied after the grace removal. The second opinion's "≤ TTL + grace" bound was written for a mandatory-TTL
design that D4 rejects; the bound here is `G + S`, and it does not depend on stamps.

### Stated limits

- The bound holds while the removal backlog is 0; with a backlog it is later by the backlog's drain time, which the
  `topgun_ephemeral_removal_backlog` gauge shows.
- Single node only in v1; the cross-node form needs the lease (D9).

---

## D7 — Offline

### Decision

- The ephemeral client handle has no op-log entry, no IndexedDB write and no queued op. While offline, writes update
  only an in-memory desired state per key; a page reload empties it. On reconnect the handle sends its desired state.
- **Staleness bound.** A desired entry with a `ttlMs` whose deadline has passed locally is dropped, not sent — the
  server would have removed it anyway. An entry without a TTL describes the current state of a live handle and is sent,
  whatever its age; its bound is the handle's lifetime (`dispose()` removes the handle's keys; a reload drops them).
- **Conflicts.** Across identities the owner rule (D3) refuses the second writer while the first owner's record is
  live, so there is no silent overwrite. Within one identity (two tabs) the last applied write wins with **no
  notice**.
- The room and presence guide must warn against putting anything a user would miss into an ephemeral map.
- **A durable handle on an ephemeral map.** A durable `getMap` on an ephemeral map writes locally first; the server
  then refuses with `storage_class_mismatch`, which surfaces as a `WriteRejection` with the new cause (D10.1).

### Options considered

- **Staleness: (a) the TTL where one exists, else the handle's lifetime (chosen); (b) drop desired entries older than
  a fixed age** (the conductor's "older than a bound" wording, second opinion item 7a). Decided against (b), with its
  edge case: a tab offline for 10 minutes that is still on the product page **is** on that page, so dropping its entry
  would be wrong. With per-tab keys (D3) every key has a single writer, so (a) holds without exception.
- **Offline writes: (a) desired state in memory (chosen); (b) queue ephemeral ops in the op log.** Rejected by ruling 7
  and by its edge case: a queued op replays a sequence of states nobody needs, and its IndexedDB row would collide with
  the durable op-id space (D2).

### Reason

A presence value is the current state, not a history; sending the last desired value per key is both minimal and
correct after any outage. The durable op log is the wrong store for it (D2, the op-id collision).

### Stated limits

- **Keys shared by the tabs of one profile (E2).** Tab A goes offline; tab B changes or removes the shared key; A comes
  back. A re-publishes its TTL-less value with **no age bound and no notice**, overwriting B's change or bringing back
  what B removed. Per-tab keys avoid it.
- Within one identity the last applied write wins with no notice.
- A durable handle's local IndexedDB copy of a refused write stays; the fix is to use the ephemeral handle.

---

## D8 — Memory

### Decision

**Budgets.**
- A node budget `TOPGUN_EPHEMERAL_MAX_MB` (default 256, minimum 1).
- A per-owner budget `TOPGUN_EPHEMERAL_MAX_KB_PER_OWNER` (default 64, minimum 1). It is **per node**: in v1 (single
  node) it is the owner's whole budget; after the cluster lift an owner whose keys span N partition-owner nodes can hold
  up to N times it.
- Cost = key bytes + encoded value bytes + a fixed per-entry overhead, which 959-8 pins by measurement. Many small keys
  hit the byte cap too, so no separate key-count cap is needed.
- Ephemeral cost is **excluded** from the eviction orchestrator's `current_ram` and reported beside it.
- Total node RAM = `TOPGUN_MAX_RAM_MB` + `TOPGUN_ENGINE_CACHE_MB` + `TOPGUN_EPHEMERAL_MAX_MB`.

**Admission.**
- A new key, or a write that grows a record, is refused with `ephemeral_capacity` (507, transient) if it would exceed
  either budget. The check is made at apply, under the key writer, where it is exact, and the refusal is that op's own
  `retry {507}` verdict (D2), so a full budget blocks only the keys that would grow it.
- Updates that keep or shrink the size, and every remove, are accepted.
- A record that alone exceeds the per-owner budget is refused with the existing `value_too_large` (413, permanent),
  decided in the validation pass (a size against a constant), so nothing of the sub-batch is applied (`TG-SYNC-003`).
- Nothing is ever dropped to make room.

**Metrics.** Reserved names, registered eagerly by the step that first moves them, present at zero from the first
scrape:

| Metric | Labels | Registered by |
|---|---|---|
| `topgun_ephemeral_bytes` | — | 959-8 |
| `topgun_ephemeral_entries` | — | 959-8 |
| `topgun_ephemeral_owners` | — | 959-7 |
| `topgun_ephemeral_owners_in_grace` | — | 959-7 |
| `topgun_ephemeral_refused_total` | `scope="node"\|"owner"` | 959-8 |
| `topgun_ephemeral_removed_total` | `reason="expired"\|"owner_gone"\|"client"` | 959-7 |
| `topgun_ephemeral_removal_backlog` | — | 959-7 |
| `topgun_ephemeral_dropped_dead_owner_total` | — | 959-7 |

### Options considered

- **At the cap: (a) refuse (chosen); (b) drop the oldest record.** Rejected: it silently removes a present visitor.
- **Fairness: (a) a per-owner byte cap in v1 (chosen, second opinion item 8); (b) the node cap alone.** Rejected: one
  owner at the cap could churn max-size records while every new visitor is refused — "first arrivals win".
- **Accounting: (a) excluded from the orchestrator (chosen); (b) counted in `current_ram`.** Rejected: see Reason.
- **Refusal form: (a) a typed per-op transient verdict (chosen); (b) dropping the connection as back-pressure.**
  Rejected: it would take the owner's other keys with it and re-publish them into the same full budget.

### Reason

The eviction orchestrator sums the cost of every store (`storage/eviction_orchestrator.rs:86-153`) and warns when
dirty records cause an eviction shortfall (`storage/eviction_orchestrator.rs:167`); ephemeral records are dirty and
never evicted (D1). Counting them would make the orchestrator evict durable maps for ephemeral memory and warn on
every tick.

### Stated limits

- The per-owner budget multiplies by the number of partition-owner nodes after the cluster lift.
- The per-entry overhead is a measured constant (959-8), not a number this page fixes.
- A scalar index registered on an ephemeral map is outside the budget (D1).

---

## D9 — Cluster

### Decision

- Owner node only: no replica and no backup. Clients rebuild.
- **v1 is single-node:** ephemeral maps with cluster mode are refused at startup (D2 refusal 1) until the lift.

**Reserved now, so TODO-844 and TODO-850 can freeze around it:**
1. **Manifest.** The class list and its `storageClassHash` are part of the cluster manifest and identical on every
   node. A joining node with a different hash is refused with a reserved `JoinRejectReason::StorageClassMismatch`.
2. **Exclusions.** Ephemeral partitions have no snapshot, no WAL package, no S3 object, no outbox row and no migration
   transfer. The new owner after a failover **and** after a planned rebalance starts empty, and clients re-publish. An
   in-memory transfer for planned migration is a possible later optimisation, deferred (TODO-850); if it ever exists
   it is a separate stream outside every durable format.
3. **Session directory (one shape, two users).** A reserved cluster message
   `SESSION_LIVENESS {node_id, epoch, live: [SessionEntry], gone: [owner_id]}` with
   `SessionEntry {owner_id, principal_id, device_id?, connections}`. It is sent periodically, as a lease; record nodes
   treat an owner whose lease lapsed as not live. The same directory is what TODO-958 needs to route addressed delivery
   to a principal or a device across nodes: one directory, not two. Its period and lapse belong to TODO-850. Message
   volume is O(owners per node) per period, not O(keys).
4. **`EPHEMERAL_REPUBLISH`** (server→client, D10.6), sent to every connection that wrote to an ephemeral map once a
   partition of one is lost.
5. **Remote watchers** use the same cluster query path as durable maps and re-snapshot when a partition changes owner
   (TODO-850 / TODO-931).
6. **Forwarded client ops carry the owner.** A client op forwarded from the node holding the connection to the
   partition-owner node carries `owner_id` and the sender's lease `epoch` in the inter-node envelope. The owner is
   stamped on the connection's node and a record node cannot derive it; liveness at apply on the record node (D6)
   checks the owner against the lease at that epoch.

**Lift condition** (recorded on TODO-850): items 3, 4 and 6 implemented and tested — the lease exists, forwarded client
ops carry the owner identity and lease epoch, and `EPHEMERAL_REPUBLISH` is emitted and handled — then D2 refusal 1 is
removed and the cross-node bound of D4 applies.

### Options considered

- **(a) Owner node only (chosen); (b) a synchronous backup per update.** Rejected: it costs the cross-node round trip
  the class exists to avoid.
- **v1: (a) refuse cluster mode until the lease (chosen, ruling 11); (b) run clustered with the liveness rule relaxed.**
  Rejected on refusal 1's edge case: remote-owned writes would either all drop or never go.
- **Directory: (a) one `SESSION_LIVENESS` shape shared with TODO-958 (chosen); (b) a liveness message per class plus a
  separate routing directory.** Rejected: two directories of the same sessions, kept consistent by hand.

### Reason

Cluster mode is a non-empty seed list (`bin/topgun_server.rs:938`); the cluster listener is bound after WAL recovery
(`bin/topgun_server.rs:1117-1121`), so refusal 1 can run before the WAL opens. Half-open detection alone is
`idle_timeout` 60 s + `reaper_interval` 10 s (`network/config.rs:187-189`), longer than `G`, so cross-node grace
without a lease would be best-effort at most — hence the lease, not a promise.

### Stated limits

- T10 ("no loss of acknowledged writes") does not cover ephemeral data. Losing it on node failure is the contract.
- After the lift, grace is local to the record node's view of the lease; the cross-node bound is the smaller of the TTL
  deadline + `S` and close detection + `G` + the lease lapse, not `G + S`.

---

## D10 — Wire and format items

### Decision

The wire table below is normative: every row has a name, carrier, values, disposition, when it is emitted, and the
owning step.

**D10.1 — Error kinds.** They extend `ALL_ERROR_KINDS` (`service/operation.rs:635-646`) and `wire_code`
(`service/operation.rs:705-717`); the codes 409, 423 and 507 are unused today (codes in use: 400, 401, 403, 413, 422,
429, 500, 501, 504).

**Classification moves onto `OperationError` (959-3, behaviour-neutral).** Today a new variant must be placed by hand in
four exhaustive matches outside `service/operation.rs`: `transient_precedence`
(`network/handlers/websocket.rs:1153-1168`), `batch_error_precedence` (`network/handlers/websocket.rs:1177-1191`) and
`is_redispatchable` (`network/handlers/websocket.rs:1227-1242`), and the label match in
`service/middleware/metrics.rs:97-110`, which duplicates `error_kind()`. 959-3 moves the three `websocket.rs` functions
onto `OperationError` as `transient_rank()`, `batch_error_rank()` and `is_redispatchable()` — exhaustive, no `_` arm,
today's values — deletes them from `websocket.rs`, and replaces the `metrics.rs` match with `e.error_kind()`. After it a
new variant is classified in `service/operation.rs` alone, and the compiler still forces every method to place it. No
fifth method is needed: the `EPHEMERAL_WRITE` dispatcher maps a singleton's error with `disposition()` and
`is_redispatchable()` alone.

| Kind | Code | Disposition | `is_redispatchable` | `transient_rank` | `batch_error_rank` | Metric label (`error_kind`) | Emitted when | Carrier | Step |
|---|---|---|---|---|---|---|---|---|---|
| `storage_class_mismatch` | **409** | Permanent | **yes**: decided in the validation pass, before anything of the sub-batch is applied, so a singleton re-dispatch cannot apply a write twice (`TG-SYNC-003`) | — (Permanent: `u8::MAX`) | **2**: after `Forbidden` (0) and `InvalidMapName` (1), before `SchemaInvalid`; it is a property of the call and the map name, like `InvalidMapName`, checked after `admit_write` and before schema, so the ranks follow the order of the checks; the later ranks shift by one | `storage_class_mismatch` | the asserted class ≠ the op map's resolved class (an `OP_BATCH`, `CLIENT_OP` or HTTP `/sync` push op on an ephemeral map; an `EPHEMERAL_WRITE` op on a durable map); an OR op on an ephemeral map; every `SyncService` operation on an ephemeral map (`SYNC_INIT`, `MERKLE_REQ_BUCKET`, `ORMAP_SYNC_INIT`, `ORMAP_MERKLE_REQ_BUCKET`, `ORMAP_DIFF_REQUEST`, `ORMAP_PUSH_DIFF`); an HTTP `/sync` pull; `SEARCH`, `SEARCH_SUB`, `HYBRID_SEARCH`, `HYBRID_SEARCH_SUB` or `VECTOR_SEARCH` on an ephemeral map; registering a vector index on an ephemeral map (admin); a journal filter spanning both classes (D10.5) | see "Carriers" | 959-5a (write paths), 959-5b (sync, pull, search, vector search), 959-9 (admin), 959-10 (journal) |
| `ephemeral_not_owner` | **423** | **Transient** | no (Transient never reaches the gate) | **4**: after `Internal` (3) | — (Transient: `u8::MAX`) | `ephemeral_not_owner` | a live record is held by another owner (it clears when that record goes) | `EPHEMERAL_WRITE_RESP.results[i]` = `retry {423}` | 959-6b |
| `ephemeral_capacity` | **507** | Transient | no | **5**: after 423 | — (Transient: `u8::MAX`) | `ephemeral_capacity` | the node or owner budget is exceeded | `EPHEMERAL_WRITE_RESP.results[i]` = `retry {507}` | 959-8 |

- **Precedence.** 423 and 507 arise only at apply on an ephemeral map, so only for `EPHEMERAL_WRITE` singletons (an
  `OP_BATCH` op on an ephemeral map is refused with 409 in the validation pass first), whose verdicts are per op. They
  never reach the `OP_BATCH` fold's `transient` sink, so they never compete in `transient_rank` in a correct build;
  they rank last so that, if one ever reached the sink, a connection- or server-scoped cause (401, 429, 504, 500) would
  be the one reported.
- **Redispatch.** Only 409 is re-dispatched by the `OP_BATCH` fold, through the existing gate. The `EPHEMERAL_WRITE`
  dispatcher re-dispatches nothing: every op is already a singleton.
- **Metrics.** The three labels join `ALL_ERROR_KINDS` (10 → 13 entries). The metrics middleware counts only an
  operation whose result is `Err` (`service/middleware/metrics.rs:84-111`), so what reaches
  `topgun_operation_errors_total` (label `error`, through `e.error_kind()` after 959-3) depends on the surface:
  - **counted there:** the 409 of the `OP_BATCH`, `CLIENT_OP` and HTTP `/sync` push class check (an `Err` of
    `handle_op_batch` / `handle_client_op`), the 409 of the `SyncService` dispatch check (an `Err` before the handler),
    and each `EPHEMERAL_WRITE` singleton's 409, 423 or 507 (each singleton is its own operation);
  - **not counted there:** the search, hybrid-search and vector-search refusals (an `Ok` response with an `error`
    string), and the HTTP `/sync` pull and admin vector-index refusals (neither passes the operation pipeline). Each of
    these sites increments the same counter itself, `topgun_operation_errors_total{service,
    error="storage_class_mismatch"}`, with the `service` label the middleware already uses for the search and query
    services and the fixed labels `http_sync` and `admin` where no service is involved — one line in a file its step
    already touches (959-5b: `service/domain/search/mod.rs`, `service/domain/query.rs`,
    `network/handlers/http_sync.rs`; 959-9: `network/handlers/admin.rs`). Every class refusal is counted once on one
    series, and no surface twice.
  - The client-op refusal counter (`network/handlers/websocket.rs:1370-1375`) counts the `OP_BATCH` fold's per-op
    refusals as today, and the `EPHEMERAL_WRITE` dispatcher counts its `refused` verdicts on the same counter
    (`transport="ws"`).
- **Carriers.**
  - `EPHEMERAL_WRITE_RESP.results[i]` (`refused {code}` or `retry {code}`) for every op of an ephemeral write: 409 on a
    durable map, 423, 507, and the existing codes;
  - `OP_REJECTED{code: 409, permanent: true}` for an `OP_BATCH` op on an ephemeral map (`OpRejectedPayload`,
    `core-rust/src/messages/sync.rs:556-577`; every frame is sent `permanent: true`,
    `network/handlers/websocket.rs:1451-1469`);
  - the HTTP `/sync` error entry `code` 409 for a push op or a pull of an ephemeral map;
  - the `error` field of `SEARCH_RESP`, `HYBRID_SEARCH_RESP` and `VECTOR_SEARCH_RESP`, whose text starts with
    `storage_class_mismatch`; a subscription request (`SEARCH_SUB`, `HYBRID_SEARCH_SUB`) is answered the way its handler
    answers any refused subscription today, which 959-5b re-verifies and cites;
  - the admin API's HTTP 409 `ErrorResponse` for a vector-index registration;
  - **no frame** for a `CLIENT_OP` and the six `SyncService` operations: today a non-batch operation's error other than
    429 is logged and answered with nothing (`network/handlers/websocket.rs:947-967`), a 403 on `SYNC_INIT` included.
    The 409 still protects the map (nothing is merged or read) and is counted on `topgun_operation_errors_total`; the
    client-visible signal for a durable handle on an ephemeral map is the 409 its `OP_BATCH` writes get (D7).
    Answering every non-batch refusal with a frame is a change to all non-batch operations and is outside this design;
    it is tracked by **TODO-680**.
- **No `writeConcern` refusal** (D1).
- **Why `ephemeral_not_owner` is transient.** A user who cleared site data has a new device id, and a guest who logs
  in has a new principal (D3). Their old record still holds the key until grace ends, and a permanent refusal would
  make them absent for good.
- **Client.** `WriteRejectionCause` (`core/src/WriteRejection.ts:27-34`) gains only `'storage_class_mismatch'`, for
  the durable `SyncEngine`'s `OP_BATCH` path. The ephemeral handle produces no `WriteRejection`s: it routes
  `EPHEMERAL_WRITE_RESP` by message type and `requestId` (a handle-local counter that never meets an op-log id) and
  reports each key's state through its own status events, from that key's verdict: `published` (`applied`), `blocked`
  (`retry {423}`), `capacity` (`retry {507}`), `retrying` (any other `retry`, or a request unanswered within the
  request timeout), `refused` (`refused {code}`). It retries `blocked`, `capacity` and `retrying` keys with its own
  backoff and never re-sends a key that was `applied`. It sends at most one op per key per request, because it holds
  one desired value per key (D7). It may have several requests outstanding, and a key's state follows only the verdict
  of the latest request that carried it (D2).

**D10.2 — Live aggregate refusal.**

| Item | Carrier | Values | Disposition | Emitted when | Step |
|---|---|---|---|---|---|
| `QUERY_LIVE_AGGREGATE_UNSUPPORTED` | `QUERY_RESP.code` (a string code, like `QUERY_UNBOUNDED_SORT`; `core-rust/src/messages/query.rs:107-134`) | the string | refusal of the subscription | for every live subscription carrying `group_by` or `aggregations` (TODO-971); after 959-11 only for functions outside COUNT/SUM/AVG (MIN, MAX) | TODO-971; narrowed by 959-11 |

MIN/MAX live aggregates are deferred (TODO-971).

**D10.3 — Removal reason vocabulary.**

| Item | Carrier | Values | Disposition | Emitted when | Step |
|---|---|---|---|---|---|
| removal `reason` | the journal event `reason` and the `reason` label of `topgun_ephemeral_removed_total` | `expired` (the existing value, built at `service/domain/crdt.rs:1087-1088`, documented at `service/domain/crdt.rs:1009`), `owner_gone`; an explicit client remove carries no reason (metric label `client`) | informational | each server-internal removal | 959-7 |

`QUERY_UPDATE` (`core-rust/src/messages/client_events.rs:85-97`) gains **no** reason field; watchers that need the
cause opt into the journal.

**D10.4 — Aggregate subscription shape.**
- **Subscribe.** The existing `QUERY_SUB` with `group_by` and `aggregations` (`core-rust/src/messages/base.rs:198-207`).
  COUNT, SUM and AVG only.
- **Snapshot.** `QUERY_RESP` rows keyed by the group key, the same shape and field names as today's DAG snapshot
  (`service/domain/query.rs:515-545`; group rows carry `__key`). The snapshot is **exact** and never coalesced.
- **Deltas.** `QUERY_UPDATE` with `key` = group key, `value` = aggregate row, and `changeType`
  (`core-rust/src/messages/base.rs:59-63`): `ENTER` when the group's row count goes 0→n; `UPDATE`; `LEAVE` when it goes
  n→0, including when the whole map empties. No new change type.
- **Coalescing.** At most one update per group per window `W` (`TOPGUN_LIVE_AGGREGATE_WINDOW_MS`, default 250, minimum
  50). The window belongs to a **shared accumulator** per `(map, canonical predicate, group_by, aggregations, effective
  authorization filter)`; one computation per change serves N subscribers. Per-subscriber `queryId` framing and
  encode-once belong to TODO-931.
- **Restart.** Subscriptions are in memory. After a restart a client re-subscribes and gets a fresh exact snapshot. No
  DELETE replay exists or is needed.
- **Precision.** SUM over integers uses an i128 accumulator, emitted as an integer when it fits i64 and as f64
  otherwise. Float SUM uses a compensated (Neumaier) running sum, reset to exactly 0 when the group's count reaches 0.
  AVG is emitted as f64 = sum / count at emission. Values that are not numeric follow the DAG snapshot's coercion.
- **Proposed `TG-AGG-001`.** At quiescence, the delta stream folded onto the snapshot equals a fresh snapshot: exactly
  for integer data, within the accumulated rounding for floats.

**D10.5 — Journal.**
- Ephemeral events never enter the durable-history ring.
- `TOPGUN_EPHEMERAL_JOURNAL_CAPACITY` (default 0 = off) sizes one separate ring for all ephemeral maps, with its own
  sequence space.
- A `JournalSubscribe` / `JournalRead` whose map filter resolves to ephemeral maps reads that ring; a filter with no
  map reads only the durable ring; a filter spanning both classes is refused with `storage_class_mismatch`.
- The ring is built in 959-10, **after TODO-933** (the Event Journal's П verdict).

**D10.6 — Reserved messages and fields.**

| Item | Carrier | Values | Disposition | Emitted when | Step |
|---|---|---|---|---|---|
| `EPHEMERAL_WRITE` | client→server message | payload `{requestId: string, ops: ClientOp[]}`; asserts the ephemeral class; `requestId` is the handle's own counter; op ids are ignored; charged its op count by the rate limiter; one connection's requests apply in arrival order (the ephemeral lane, D2) | — | the handle publishes desired state | 959-4 (type), 959-5a (handled), 959-9 (sent by the TS handle) |
| `EPHEMERAL_WRITE_RESP` | server→client message | payload `{requestId: string, results: EphemeralOpResult[]}`; `results[i]` is the verdict of `ops[i]`, one per op; `EphemeralOpResult` = `{status: "applied"}` \| `{status: "refused", code: number}` \| `{status: "retry", code: number}`; routed by the client to the ephemeral handle, never to `SyncEngine` | `applied` / `refused` (permanent) / `retry` (transient) | once per dispatched request, after every singleton; best-effort all-`retry {429}` for a rate-limited request | 959-4 (type), 959-5a (emitted), 959-9 (read by the TS handle) |
| `AUTH_ACK.storageClassHash`, `DEVICE_ACK.storageClassHash` | handshake frames | lowercase hex of the first 16 bytes of SHA-256 over `"tg-storage-class-v1\n"` + the sorted, de-duplicated patterns joined by `"\n"` | diagnostic | every `AUTH_ACK` / `DEVICE_ACK` while a list is set | 959-9 |
| `EPHEMERAL_REPUBLISH` | server→client message | payload `{mapNames?: string[]}`; absent means all | instruction to re-publish | a partition of an ephemeral map is lost (D9) | cluster lift (TODO-850) |
| `SESSION_LIVENESS` with `SessionEntry` | cluster message | `{node_id, epoch, live: [SessionEntry], gone: [owner_id]}`; `SessionEntry {owner_id, principal_id, device_id?, connections}` | lease | periodically, per node (D9 item 3) | cluster lift (TODO-850) |
| Forwarded-op envelope `owner_id`, `epoch` | inter-node envelope | the stamped owner and the sender's lease epoch | — | every forwarded client op (D9 item 6) | cluster lift (TODO-850) |
| `JoinRejectReason::StorageClassMismatch` | cluster join reply | the variant | refusal of the join | a joining node's `storageClassHash` differs (D9 item 1) | cluster lift (TODO-850) |
| `GET /api/admin/storage-classes` | admin API | `{patterns, hash, ephemeral: {bytes, entries, owners, ownersInGrace}}` | — | on request | 959-9 |
| Ephemeral journal ring | `JournalSubscribe` / `JournalRead` | a separate ring sized by `TOPGUN_EPHEMERAL_JOURNAL_CAPACITY` (default 0) | 409 for a filter spanning both classes | each ephemeral apply while the ring is on (D10.5) | 959-10 |
| Outbox exclusion | outbox / CDC (TODO-632) | ephemeral maps excluded, no opt-in | — | never | TODO-632 |

`OP_BATCH`, `OP_ACK` and `OP_REJECTED` are unchanged: no `OP_BATCH` field is added.

**D10.7 — Outbox and CDC (TODO-632).** Ephemeral maps are excluded, with **no** opt-in in v1. An ephemeral change in a
durable outbox would make each presence update a durable append, the cost the class exists to avoid. This deviates
from TODO-959, which names "journal/outbox as opt-in": the journal opt-in is kept (D10.5), the outbox opt-in is not. A
later opt-in, if any, carries aggregates or transitions, not rows; deferred (TODO-632, TODO-959).

**D10.8 — Reserved as code now? No.** The items are reserved in this page's tables only. Each lands as code in the step
that first emits or reads it.

### Options considered

- **Error codes: (a) new HTTP-style codes 409/423/507 on the existing `u32` vocabulary (chosen); (b) one generic code
  with a string sub-kind.** Rejected: the vocabulary is a closed set with a disposition per kind
  (`service/operation.rs:680-697`), and the client mirrors it as a closed union (`core/src/WriteRejection.ts:27-34`).
- **Removal reason on `QUERY_UPDATE`: (a) no field, journal only (chosen); (b) a reason field.** Rejected: deletion
  over addition — every watcher would pay for a field few read.
- **Aggregate deltas: (a) the existing ENTER/UPDATE/LEAVE (chosen); (b) a new change type.** Rejected: the existing
  three map exactly.
- **Aggregate window: (a) a shared accumulator (chosen); (b) per subscription.** Rejected: N computations and N
  windows for N identical subscriptions.
- **Journal: (a) a separate ring (chosen); (b) the shared ring.** Rejected: the journal is one global ring with a
  global switch (`service/domain/journal.rs:55-67`), so presence updates would flood it and push durable history out.
- **Reservation: (a) in this page only (chosen); (b) enum variants now.** Rejected: an enum variant with no emitter is a
  contract no test can exercise; `disposition` and `wire_code` are exhaustive matches, so a dead variant would have to
  be classified by guesswork.

### Reason

Wire refusals use HTTP-style `u32` codes and a closed `error_kind` vocabulary with a disposition
(`service/operation.rs:635-743`); `OP_REJECTED` carries `code: Option<u32>` and `permanent: bool`
(`core-rust/src/messages/sync.rs:556-577`); `QUERY_RESP` carries a string `code` and `QUERY_UPDATE` is
`{queryId, key, value, changeType}` (`core-rust/src/messages/client_events.rs:85-97`). Each row above reuses the
carrier that already exists for its kind of signal.

### Stated limits

- A refusal of a non-batch operation is silent on the wire today (TODO-680).
- MIN/MAX live aggregates are refused until a later step.
- Float aggregates are exact only up to the accumulated rounding.

---

## Proposed invariants

These rows are written in `INVARIANTS.md` format and are **landed by the implementation steps, not here**: this page
does not edit `INVARIANTS.md`, because rows without tests would grow the NAKED count. The first landing step also adds
`EPH` (ephemeral class) and `AGG` (live aggregates) to the domain list (`INVARIANTS.md:9-12`). No existing row is
amended: `TG-SYNC-001` and `TG-SYNC-003` hold unchanged (D2).

| ID | Scope | Statement | Stated limits | Maintaining code (planned) | Enforcing test (planned) | Violation consequence | Lands in |
|---|---|---|---|---|---|---|---|
| TG-EPH-001 | one write to a map that resolves ephemeral | An ephemeral write makes no WAL frame or sequence, no write-behind slot, no store row or registry entry, no Merkle leaf, no search index entry and no embedding, and its cost is outside the eviction orchestrator; ephemeral records are never evicted. | Scalar indexes and the Arrow cache stay attached (D1). | `storage/ephemeral.rs`, `storage/factory.rs`, `storage/eviction_orchestrator.rs` | `tg_eph_001_ephemeral_write_touches_no_durable_surface`, `tg_eph_001_orchestrator_excludes_ephemeral_cost` | presence updates pay durable cost, or ephemeral data leaks into a durable tier | 959-1 |
| TG-EPH-002 | the class of a map and the process's boot | A map's class is a pure function of its tenant-local name and the boot-time list; reserved names are durable; the server exits 1 before it accepts a connection when the list is set in cluster mode or a durable registry map matches an ephemeral pattern (both before the WAL opens, writing nothing), when a WAL frame's map resolves ephemeral (inside recovery; no frame of that map is applied; the open-time WAL housekeeping and the idempotent replay of earlier partitions may have run), or when a vector map resolves ephemeral (after recovery). | The client listener is bound before all four (`bin/topgun_server.rs:926`). | `storage/ephemeral.rs` (resolver), `storage/wal/mod.rs`, `bin/topgun_server.rs`, `bootstrap.rs` | `tg_eph_002_resolver_tenant_local_and_reserved`, `tg_eph_002_refuses_registry_map`, `tg_eph_002_refuses_wal_frame_before_replay`, `tg_eph_002_refuses_vector_map`, `tg_eph_002_refuses_cluster_mode` | acked durable writes replayed into an ephemeral map, or a map changing class under live data | 959-1 (resolver), 959-2 (registry, WAL and vector checks), 959-9 (environment, cluster refusal) |
| TG-EPH-003 | one client call on either class | The class is asserted by the message type (`EPHEMERAL_WRITE` = ephemeral; `OP_BATCH`, `CLIENT_OP`, HTTP `/sync` = durable) and reaches the write path only as `OperationContext.storage_class`; an op whose map class differs (either direction) and an OR op on an ephemeral map are refused with 409 before anything of the sub-batch is applied; every `SyncService` operation, an HTTP `/sync` pull, `SEARCH`/`SEARCH_SUB`/`HYBRID_SEARCH`/`HYBRID_SEARCH_SUB`/`VECTOR_SEARCH` on an ephemeral map, and an admin vector-index registration on one, are refused with 409 and touch nothing; every `EPHEMERAL_WRITE` is answered by at most one `EPHEMERAL_WRITE_RESP` whose `results[i]` is the only verdict of `ops[i]`, each op is applied at most once per request, an op the response names `refused` or `retry` applied nothing, and `WrongService`/`UnknownService` are never `refused`; no singleton of an `EPHEMERAL_WRITE` starts before every singleton of every earlier `EPHEMERAL_WRITE` of the same connection has finished. | Arrival order per connection only; nothing is claimed across connections (at most one in-progress request of a displaced connection may land late, D2). A rate-limited, over-cap or lane-closed request may go unanswered. | `service/domain/crdt.rs`, `network/handlers/websocket.rs`, `service/domain/sync.rs`, `network/handlers/http_sync.rs`, `service/domain/search/mod.rs`, `service/domain/query.rs`, `network/handlers/admin.rs` | `tg_eph_003_mismatch_refused_both_directions`, `tg_eph_003_positional_verdicts`, `tg_eph_003_two_request_reorder`, `tg_eph_003_closed_lane_in_envelope`, `tg_eph_003_ormap_push_diff_stores_nothing` | a durable write silently stored in memory only, an ephemeral write in the WAL, or an older value overwriting a newer one | 959-5a (write paths, response), 959-5b (sync, pull, search, vector search), 959-9 (admin) |
| TG-EPH-004 | one ephemeral record | Each ephemeral record has exactly one owner, stamped by the server from the connection's owner, which is fixed once for the connection's lifetime, and never sent to another principal; a write or remove of another owner's live record is refused with a `retry {423}` verdict that applies nothing and blocks no other op of the request; only `expired`, `owner_gone` and tenant drop bypass this. | No key reservation before a first write or after removal (D3). | `service/domain/crdt.rs` | `tg_eph_004_owner_rule_refuses_other_owner`, `tg_eph_004_blocked_key_does_not_block_others`, `tg_eph_004_owner_never_in_snapshot_or_delta` | one visitor overwrites or removes another's presence; a device id leaks | 959-6b |
| TG-EPH-005 | one owner on a single node | No record stamped by owner X exists more than `G + S` after X stopped being live (single node; backlog 0); an owner is live while a registered connection fixed to it exists, and a fix for a connection already removed is rejected; owner-gone removal is re-checked under the key writer and cancelled by a reconnect at bind time; a write of a non-live owner is dropped at apply. | Single node; with a backlog the bound is later by its drain time. | `storage/ephemeral.rs`, `network/connection.rs`, `service/domain/crdt.rs` | `tg_eph_005_fix_rejected_after_remove` (both interleavings), `tg_eph_005_dead_owner_write_dropped`, `tg_eph_005_owner_gone_within_g_plus_s` | a departed visitor stays present, or a record resurrects after removal | 959-6a (bookkeeping), 959-6b (liveness at apply), 959-7 (removal) |
| TG-EPH-006 | one ephemeral record with `ttlMs` | `ttlMs` is stripped before apply, so `TG-TTL-001/002` never see an ephemeral record; a record with `ttlMs` is removed, as an ordinary REMOVE with reason `expired`, at most `S` after its monotonic deadline (backlog 0); a write after the tick wins. | Reads are not filtered at the deadline. | `storage/ephemeral.rs`, `service/domain/crdt.rs` | `tg_eph_006_ttl_stripped`, `tg_eph_006_expired_within_s`, `tg_eph_006_write_after_tick_wins` | a typing flag never clears, or a fresh write is removed | 959-7 |
| TG-EPH-007 | admission of one ephemeral write | A new key or a growth over the node or owner budget is refused with 507 (or 413 when the record alone exceeds the owner budget); no other record is dropped to make room. | The per-owner budget is per node (×N after the lift). | `storage/ephemeral.rs`, `service/domain/crdt.rs` | `tg_eph_007_node_budget_507`, `tg_eph_007_owner_budget_507`, `tg_eph_007_oversize_record_413_in_validation` | unbounded memory, or one owner starving the rest | 959-8 |
| TG-EPH-008 | the process lifetime | Ephemeral maps are empty after every process start, and nothing ephemeral is in the WAL, the store, a snapshot, a migration, S3 or the outbox. | Cluster form lands with TODO-850. | `storage/factory.rs`, `storage/wal/mod.rs` | `tg_eph_008_empty_after_restart`, `tg_eph_008_nothing_in_wal_or_store` | presence of a departed visitor reappears after a restart | 959-1 and 959-2 (single node); cluster form with TODO-850 |
| TG-AGG-001 | one live aggregate subscription | At quiescence, an aggregate delta stream folded onto its snapshot equals a fresh snapshot (exact for integers). | Floats within the accumulated rounding. | `service/domain/aggregate.rs` | `tg_agg_001_fold_equals_snapshot` (proptest), `tg_agg_001_empty_map_leaves` | a dashboard drifts from the data | 959-11 |

---

## Acceptor

Against the project's result acceptor — its end-goal targets T1–T12, its mandatory architecture practices (§2) and
its per-component verdicts (§4: **Б** blocks a target; **П** has a ceiling below the target that must be raised before the target is claimed;
**Д** reaches it):

- **T1.** Ephemeral writes skip the WAL and write-behind, so they never consume the durable path's T1 budget. Their
  cost is the in-memory apply plus fan-out.
- **T1 and T2, dispatch cost.** An `EPHEMERAL_WRITE` costs one dispatch per op, with concurrency at most the number of
  partitions it touches (the same as an `OP_BATCH` of those ops), and `k` ops of one partition are `k` serial round
  trips (D2, "Cost"). The durable path gains one class comparison per op in the validation pass and one per-frame
  check in the read loop. 959-5a runs the load harness, fire-and-wait, on the durable scenario against its base
  (> 20 % regression needs justification) and on a multi-key ephemeral scenario reported against T1 (≥ 2·10⁴
  confirmed writes/s per node) and T2 (p50 ≤ 2 ms, p99 ≤ 10 ms at 10⁴/s).
- **T4.** At 10⁵ connections there is no per-key renewal traffic (D4); liveness is the existing per-connection `PING`;
  per-owner state is one index entry; re-publish is spread by the existing jitter.
- **T5 and T6.** Ephemeral writes enter the live-query fan-out, whose verdict is **Б** (TODO-931). By the rule "new
  functions that touch a Б component wait for its closure", **959-9 (enablement) depends on TODO-931**, and the storm
  coalescing of D5 is a requirement placed on TODO-931. 959-1..959-8 do not change fan-out code and may land before it.
  Aggregates (959-11) also follow TODO-931.
- **Event Journal (П, TODO-933).** The ephemeral ring (D10.5) lives in the Event Journal, whose verdict is П: a global
  lock and a scan of journal subscriptions on every write. With the ring on, every ephemeral write would pay that
  ceiling, so **959-10 depends on TODO-933**. Until then the ring does not exist; its default is off in any case.
- **T7.** A separate, bounded budget; never evicted; no O(keys) structure outside the budget, with one stated
  exception: a scalar index an operator registers on an ephemeral map (D1) is opt-in and outside the budget. The
  Merkle and search observers, which would be O(keys), are not attached.
- **T9.** Ephemeral maps never reach `SYNC_INIT`.
- **T10.** Not covered by design (D9).
- **T11.** Removal is chunked, carries a backlog gauge, and leaves no tombstones to prune. It does **not** inherit the
  TTL sweep's П ceiling (≤ 1024 removals/s), although it uses the same ordinary REMOVE apply. That ceiling comes from
  how the sweep finds and dispatches removals: it scans rows at a paced rate and awaits one dispatched op at a time
  (`service/expiry.rs:601`) behind a 1024-entry queue (`service/expiry.rs:107`). The removal tick reads due entries
  from a deadline queue and the owner index (no row scan) and calls the remover directly with a bounded number of
  removals in flight across keys; the per-key writer still serialises each key. 959-7 must show that the removal rate
  keeps up with arrival: target at least 2·10⁴ removals/s on the bench host, measured, not asserted.
- **Practices.**
  - §2.1: bounded budgets with named refusal policies and metrics (D8).
  - §2.2: the per-owner entry lock is not a shared data-path lock. There is one entry per identity, held for O(1) work
    (a liveness check and an index insert, or a key-list snapshot), contended only by that owner's own writes and its
    own removal, never across owners. Lock order is key writer → owner entry (D6). The removal tick is service work and
    runs in its own task, off the partition workers.
  - §2.5: per-key coalescing in a keyed per-connection delta stage (D5, new work in TODO-931) and aggregate coalescing
    (D10.4).
  - §2.6: the keyed stage is bounded in entries and bytes; a watcher whose pending keys exceed it gets a re-snapshot of
    its largest subscription, never a silently dropped delta (D5).
  - §2.7: a chunked removal tick (D4).
- **Final design.** Every step is part of the final design. The single-node restriction before the lease (D2, D9) is
  not temporary code: the lease and the forwarded-op fields are additions (TODO-850), and the refusal is removed by
  the lift.

---

## Implementation split

**Cap rules.** Each step stays within **5 production `.rs` + 10 test-only `.rs`** files, trait-first (the step's G1
is types and traits only). Every forced file counts, and the table names each with the reason it is touched: a
behaviour edit, a struct literal broken by a new field, an arm of an exhaustive match forced by a new enum variant, a
`mod` declaration for a new file, a caller of a changed signature, or wiring. Counts are measured at `e3e4b5d7` with
the greps in the footnote; **each implementation step re-measures them against its own base commit before it is
planned, and a step that then exceeds the cap is split again at that point, never squeezed.** Wiring avoids two known
cascades: a new `CrdtService::new` parameter (`service/domain/crdt.rs:154-204`, called in 11 files) and a new
`AppState` field (`network/handlers/mod.rs:63-103`, literals in 14 files); optional dependencies attach through
builders and through the record-store factory, which both `CrdtService` and `AppState.store_factory` already reach.

| Step | Scope | Production `.rs` with reason (count) | Test-only `.rs` forced | G1 types/traits | Depends | Invariants |
|---|---|---|---|---|---|---|
| **959-1** | Class core, not reachable from the environment: the one `StorageClass` type; resolver (globs, tenant-local names, reserved prefixes, hash); per-map routing to a null-backed store; the separate ephemeral observer-factory list (D1); orchestrator exclusion. | `storage/ephemeral.rs` (new), `storage/mod.rs` (`mod` decl), `storage/factory.rs` (routing; `with_ephemeral(EphemeralConfig, ephemeral_observer_factories)` builder; `get_or_create` consults the ephemeral list for the class), `storage/eviction_orchestrator.rs` (exclusion), `core-rust/src/messages/sync.rs` (`StorageClass` enum, referenced by module path so `messages/mod.rs` is untouched) (5) | — | `StorageClass`, `StorageClassResolver`, `EphemeralConfig` | 959-0 | EPH-001, EPH-002 (resolver), EPH-008 |
| **959-2** | Boot refusals 2, 3 and 4 as library checks, called by the binary with an empty list; the TTL sweep skips ephemeral maps; production wiring of the ephemeral observer list (scalar index, Arrow cache). | `storage/wal/mod.rs` (`WalRecovery::with_class_resolver` builder, per-partition check before `replay_partition`; no `WalRecovery::new` caller changes), `storage/datastores/write_behind.rs` (`open_over_store` takes the resolver and sets the builder), `bin/topgun_server.rs` (registry check before `bin/topgun_server.rs:1058`; passes `EphemeralConfig::default()`; prints a storage-class refusal from the `open_over_store` error arm in the `Error: storage class:` form), `bootstrap.rs` (`build_services` takes `EphemeralConfig`; factory `with_ephemeral` with the index and Arrow-cache factories; vector-map refusal), `service/expiry.rs` (skip) (5) | `benches/load_harness/store_arm.rs`, `benches/load_harness/main.rs` (callers) | `StorageClassRefusal` | 959-1 | EPH-002 (registry, WAL, vector), EPH-008 |
| **959-3** | Behaviour-neutral preparation: `OperationError` classification moves onto the type (D10.1). `transient_rank()`, `batch_error_rank()`, `is_redispatchable()` become exhaustive methods with today's values, the three `websocket.rs` functions are deleted, and the `metrics.rs` label match becomes `e.error_kind()`. No wire, metric or verdict changes. | `service/operation.rs` (three methods), `network/handlers/websocket.rs` (`network/handlers/websocket.rs:1153-1242` deleted; call sites use the methods), `service/middleware/metrics.rs` (`service/middleware/metrics.rs:97-110` → `e.error_kind()`) (3) | — | — | — | — |
| **959-4** | The ephemeral write message pair and the server-side class carrier, with no handler yet: `EPHEMERAL_WRITE` / `EPHEMERAL_WRITE_RESP` types and their `classify` arms; `OperationContext.storage_class` (`Durable` in `new`) and `Operation::set_storage_class`. An `EPHEMERAL_WRITE` reaching a server of this step is classified `RequiresPartitionSplit`, logged at `warn` and answered with nothing; nothing is applied (`network/handlers/websocket.rs:992-1000`). TS: Zod schemas for both messages in `packages/core`; cross-language fixtures for both (`core-rust/tests/cross_lang_compat.rs` and the TS fixture generator, not forced). | `core-rust/src/messages/sync.rs` (payload types `EphemeralWritePayload`, `EphemeralWriteRespPayload`, `EphemeralOpResult`; inline round-trip tests), `core-rust/src/messages/mod.rs` (the two `Message` variants and their `pub use`), `service/classify.rs` (two arms of the exhaustive `classify` match: `EphemeralWrite` → `RequiresPartitionSplit`, `EphemeralWriteResp` → `ServerToClient`), `service/operation.rs` (`OperationContext` field set in `new`; `set_storage_class`), `service/middleware/timeout.rs` (its inline-test `OperationContext` literal, `service/middleware/timeout.rs:106`) (5) | — | `EphemeralWritePayload`, `EphemeralWriteRespPayload`, `EphemeralOpResult`, `OperationContext::storage_class` | 959-3 | — |
| **959-5a** | The class check on the write paths and the `EPHEMERAL_WRITE` handler: mismatch in either direction and OR ops on an ephemeral map refused with 409 in all three validation branches (HTTP push included, through `OperationContext::new`); the handler (singleton dispatch per op, positional verdicts, one `EPHEMERAL_WRITE_RESP`, `message_cost` = op count, the rate-limited answer for a top-level frame and per inner request of a refused envelope, the ephemeral lane with the closed flag checked at every intercept, refusal metric). Tests include the **two-request reorder**: two `EPHEMERAL_WRITE`s of one connection write one key, the first held long enough to finish after the second would without the lane (for example, behind several ops of one partition ahead of the key's op); the key ends with the second request's value and the first answer leaves before the second — top-level, inside one envelope, and an envelope followed by a top-level frame; and a request whose turn comes after the read loop exited is neither dispatched nor answered, both as a top-level frame and as a later `EPHEMERAL_WRITE` inside an envelope whose earlier one was in progress when the lane closed (the later request is skipped; the envelope's non-ephemeral messages after it are dispatched). TS: client cause `'storage_class_mismatch'`. Load harness fire-and-wait: durable scenario against the base, multi-key ephemeral scenario against T1/T2, one-connection pipelined throughput reported. | `service/operation.rs` (`StorageClassMismatch` and its arms in `disposition`, `wire_code`, `error_kind`, `ALL_ERROR_KINDS`, the three 959-3 methods and the test guard `one_of_every_error_variant`, `service/operation.rs:907-944` — one file), `service/domain/crdt.rs` (check in the validation pass of `handle_op_batch` and in `handle_client_op`, `service/domain/crdt.rs:294`), `network/handlers/websocket.rs` (intercept beside `network/handlers/websocket.rs:930`; `dispatch_ephemeral_write`; `message_cost` arm; the rate-limited `EPHEMERAL_WRITE_RESP`s in the read loop, `network/handlers/websocket.rs:401-430`; the lane ticket made at `network/handlers/websocket.rs:461` and threaded through the private `dispatch_message` / `dispatch_envelope` / `dispatch_one_message` — same file, nothing else forced) (3) | the load-harness ephemeral scenario (new, under `benches/load_harness/`) | `OperationError::StorageClassMismatch`, `EphemeralVerdict` (server-side mapping) | 959-4 | EPH-003 (write paths, response) |
| **959-5b** | The 409 surface beyond the write path: every `SyncService` operation on an ephemeral map (`ORMAP_PUSH_DIFF` included), HTTP `/sync` pull, `SEARCH`/`SEARCH_SUB`/`HYBRID_SEARCH`/`HYBRID_SEARCH_SUB`, `VECTOR_SEARCH`. | `service/domain/sync.rs` (one check in the dispatch, `service/domain/sync.rs:1938-1955`, by the payload's map name, before the handler; the factory is reached at `service/domain/sync.rs:1628`), `network/handlers/http_sync.rs` (pull refusal and its counter increment), `service/domain/search/mod.rs` (search, hybrid search and their subscriptions, through the factory it holds, `service/domain/search/mod.rs:1577-1584`, and the counter increment), `service/domain/query.rs` (`VECTOR_SEARCH`, `service/domain/query.rs:701`, through the factory `QueryService` already holds, `service/domain/query.rs:476`, `service/domain/query.rs:569-579`, and the counter increment) (4) | — | — | 959-5a | EPH-003 (sync, pull, search, vector search) |
| **959-6a** | Ownership bookkeeping, no write-path change: `EphemeralRegistry` with `conn_owner`, owner entries and live sets; fix-once with the registered-connection re-check under the owner-entry lock; `unfix` on disconnect; production and simulation wiring. | `storage/ephemeral.rs` (`EphemeralRegistry`, fix/unfix), `storage/factory.rs` (holds the registry, `ephemeral()` accessor), `network/connection.rs` (identity-bound observer slot, called from `claim_device_ownership`), `bootstrap.rs` (register the bind observer and the `on_disconnect` unfix), `sim/cluster.rs` (same wiring for simulation) (5) | — | `EphemeralRegistry`, `OwnerId`, `IdentityBoundObserver` | 959-5b | EPH-005 (bookkeeping) |
| **959-6b** | Owner rule and liveness at apply: owner stamp, first-write fix of an identity-less connection, owner rule (423, a `retry` verdict through the 959-5a handler with no handler change), liveness at apply. | `service/domain/crdt.rs` (stamp, rule, liveness at apply, first-write fix), `service/operation.rs` (`EphemeralNotOwner` with its arms) (2) | simulation tests | `Admission` | 959-6a | EPH-004, EPH-005 (apply) |
| **959-7** | Grace and TTL removal: `ttlMs` strip and deadline, the removal tick, owner-gone, the tick's production and simulation wiring, removal metrics. | `storage/ephemeral.rs` (deadline queue, tick, chunking, backlog), `service/domain/crdt.rs` (remover implementation, re-check, strip), `bootstrap.rs` (spawn the tick, register metrics), `sim/cluster.rs` (spawn the tick in simulation) (4) | simulation tests | `EphemeralRemover` | 959-6b | EPH-005 (removal), EPH-006 |
| **959-8** | Budgets: cost accounting, admission at apply (507, a `retry` verdict per op), 413 in validation, budget metrics. | `storage/ephemeral.rs` (accounting), `service/domain/crdt.rs` (507 at apply; 413 in the validation pass), `service/operation.rs` (`EphemeralCapacity` with its arms), `bootstrap.rs` (register metrics) (4) | — | `EphemeralBudget` | 959-7 | EPH-007 |
| **959-9** | Enablement and public surface: `TOPGUN_EPHEMERAL_MAPS` / `TOPGUN_EPHEMERAL_GRACE_MS` / `TOPGUN_EPHEMERAL_MAX_MB` / `TOPGUN_EPHEMERAL_MAX_KB_PER_OWNER`, cluster refusal (D2 refusal 1), startup line; handshake hash; `GET /api/admin/storage-classes`; tenant drop clears the tenant's ephemeral maps and registry entries; a vector-index registration on an ephemeral map is refused with 409. TS: `client.ephemeral(name)` handle (desired state, `EPHEMERAL_WRITE` with a handle-local `requestId`, `EPHEMERAL_WRITE_RESP` routed to the handle in `client/src/sync/ClientMessageHandlers.ts` and never to `SyncEngine`, overlapping requests with the latest-request rule per key, the request timeout, re-publish, `dispose()` on `pagehide`, per-key status events), `useEphemeral`, the hash warning. Docs: CLAUDE.md variables and a docs-astro guide with the D3 keying and D7 warnings. Integration tests, including a durable op and an ephemeral request in flight on one connection whose verdicts each reach only their own sender, and the **two-request reorder through the handle**: the handle sends request 1 for a key, then request 2 with a newer value while 1 is unanswered (and, in a second case, re-sends after its request timeout while the original is still on the server); the key ends with the newest value, its status is `published` from the latest request's verdict, and a late answer of the earlier request does not change it. | `bin/topgun_server.rs` (environment, cluster refusal before `bin/topgun_server.rs:1058`, startup line), `core-rust/src/messages/client_events.rs` (hash field), `network/handlers/websocket.rs` (emit the hash, `network/handlers/websocket.rs:219`, `network/handlers/websocket.rs:380`), `network/handlers/admin.rs` (endpoint; tenant drop, `network/handlers/admin.rs:888-898`; vector-index refusal before `network/handlers/admin.rs:1666`, with its counter increment), `network/module.rs` (route, beside `network/module.rs:723`) (5) | integration tests | `EphemeralHandle` (TS) | 959-8, **TODO-931**, **TODO-747 and TODO-972 traced** | EPH-002 (environment, cluster) |
| **959-10** | Ephemeral journal ring and its filter rule. | `service/domain/journal.rs` (ring), `service/domain/persistence.rs` (filter rule, 409), `service/domain/crdt.rs` (route ephemeral events), `bootstrap.rs` (construct, read `TOPGUN_EPHEMERAL_JOURNAL_CAPACITY`), `bin/topgun_server.rs` (startup line) (5) | — | the journal ring selector | 959-9, **TODO-933** | journal statements |
| **959-11** | Live aggregates: shared accumulators, coalesced per-group deltas, the narrowed D10.2 refusal. TS: QueryHandle live aggregates. | `service/domain/aggregate.rs` (new), `service/domain/mod.rs` (`mod` decl), `service/domain/query.rs`, the fan-out module as TODO-931 leaves it (`service/domain/crdt.rs` today), `bootstrap.rs` (read `TOPGUN_LIVE_AGGREGATE_WINDOW_MS`, construct, log the effective window) (5) | — | `AggregateAccumulator` | **TODO-931**, **TODO-971** | TG-AGG-001 |
| TODO-297 | Rooms: client sugar over an ephemeral map plus a topic. | TS only | — | — | 959-9 | — |

- **Footnote — the greps, under `packages/`, at `e3e4b5d7`.** `ClientOp {`, `OperationContext {`,
  `AuthAckData {` / `DeviceAckData {`, `CrdtService::new(`, `AppState {`, `build_services(`,
  `WalBootstrap::open_over_store`, `WalRecovery::new(`, `ObserverFactory for`. A literal ending in
  `..Default::default()` does not break, and a hit that is a return type (`-> OperationContext {`) is not a literal:
  `OperationContext` has two literals, `service/operation.rs:111-122` and `service/middleware/timeout.rs:106`.
  **Exhaustive matches on `OperationError`:** `OperationError::(Unauthorized|Overloaded|WrongService)\b` and
  `Self::WrongService\b`, then read each hit: a `match` with no `_` (or binding catch-all) arm is forced by every new
  variant. At `e3e4b5d7` that is `service/operation.rs` (four sites: `disposition`, `wire_code`, `error_kind` and the
  test guard), `network/handlers/websocket.rs` (three) and `service/middleware/metrics.rs` (one); after 959-3, only
  `service/operation.rs`. `network/handlers/websocket.rs:1562-1565` matches with a `ref e` catch-all and does not
  break. **Exhaustive matches on `Message`:** `Message::ORMapSyncRespLeaf\b`, then read each hit: the only `match`
  with no `_` arm is `service/classify.rs` (`service/classify.rs:127`, server→client variants at
  `service/classify.rs:398`). `OpBatchPayload {` and `OpBatchDispatchContext {` no longer force anything: no step adds
  a field to either.
- **Exemptions:** none. Every step is ≤ 5 counted production files without a shape exemption. Counts: 959-1 5, 959-2
  5, 959-3 3, 959-4 5, 959-5a 3, 959-5b 4, 959-6a 5, 959-6b 2, 959-7 4, 959-8 4, 959-9 5, 959-10 5, 959-11 5.
- **Order:** 959-1 → 959-2 → 959-3 → 959-4 → 959-5a → 959-5b → 959-6a → 959-6b → 959-7 → 959-8 → 959-9 (after TODO-931
  and the TODO-747/972 traces) → 959-10 (after TODO-933) and TODO-297. 959-11 comes after TODO-931 and TODO-971, in
  parallel with the rest.
- **Before the TODO-844/850 freeze.** The cluster reservations (D9) are recorded on those TODOs.
- **Simulation tests.** Every step that changes behaviour in `service/domain/` carries simulation tests (CLAUDE.md
  "Simulation Testing"). 959-5a and 959-5b carry the 409 surfaces (959-5b: an `ORMAP_PUSH_DIFF` to an ephemeral map
  stores nothing); 959-6a wires the simulation node; 959-6b carries liveness at apply under a delayed dispatch (a write
  queued before its connection's close is dropped) and the per-op 423 verdict in a mixed ephemeral request (the other
  ops `applied`, none re-applied on the blocked key's retry); 959-7 carries owner-gone removal under a node restart.
  959-6a proves the fix/`remove()` race by unit tests that force both interleavings. 959-3 and 959-4 are
  behaviour-neutral (959-4's message is decoded and not handled) and carry none; existing tests must pass unchanged.
- **Load harness.** 959-5a changes `service/domain/crdt.rs` and `network/handlers/`, so it runs the fire-and-wait load
  harness as D2 "Cost" states.

---

## Second-opinion disposition

A cross-vendor second opinion (glm-5.3) reviewed the options before this design. Every item is addressed below.

| # | Item | Verdict | Where | Reason |
|---|---|---|---|---|
| 1 | Owner by device is wrong for multi-tab; use `(device, connection)`; say grace removes only records stamped by that owner | adopted, changed | D3 | The device credential is per browser profile and shared by its tabs, and a second tab takes the identity over; connection-scoped removal fails E2 and E3, and a connection-level stamp would have no reader under takeover. Device scope is kept; "removes only records stamped by that owner" is stated; per-tab keys are the recommended keying. |
| 2 | Reconnect storm: client jitter, server advisory backoff, coalesced fan-out after restart, ordering against in-grace removal and the sweep | adopted, changed | D5 | Existing reconnect jitter plus per-op `retry {429}`; coalescing per `(subscription, key)` in a keyed stage built by TODO-931 (no timer, re-snapshot on overflow); advisory jitter and a settling window rejected; reconnect cancels grace at bind time; ephemeral records never meet the TTL sweep (D1). |
| 3 | Class migration: handshake hash; assert the class per map, per call | adopted, changed | D2 | The hash is in `AUTH_ACK` / `DEVICE_ACK` as a diagnostic; the assertion is per call by message type (`EPHEMERAL_WRITE` vs `OP_BATCH`), refused with 409 in both directions; narrowing the list is always accepted. |
| 4 | TTL clock: receive time vs HLC; ±10 % jitter; state the `TTL + sweep` bound | adopted, changed | D4 | Monotonic receive instant adopted; server jitter rejected (it flickers or breaks the bound, and no cohort forms); the bound deadline + `S` is `TG-EPH-006`. |
| 5 | Resurrection: state the bound; consider a `last_removed_at` guard | adopted, changed | D6 | Liveness at apply under the owner-entry lock; the guard rejected because the server mints every stamp at apply, so it would refuse nothing; the bound is `G + S` (`TG-EPH-005`), not `TTL + grace`. |
| 6 | Boot check per tenant; expose the class list in the admin API; tenant-local matching | adopted | D2 | The registry check applies the resolver to every full name (per tenant); matching is tenant-local; `GET /api/admin/storage-classes` shows patterns, hash and counts. |
| 7 | Offline: drop desired state older than N seconds; document LWW without notice; warn in the presence guide | adopted, changed | D7 | Age-based drop rejected (a tab offline 10 minutes still on the page is on the page); TTL-expired entries are dropped; LWW without notice and the guide warning are stated. |
| 8 | Per-owner cap in v1; typed per-op refusal or connection back-pressure | adopted | D8 | `TOPGUN_EPHEMERAL_MAX_KB_PER_OWNER` in v1; refusal is a typed per-op transient verdict (507), not a dropped connection. |
| 9 | Cluster liveness source is ambiguous; state the cross-node bound so nobody promises 30 s | adopted, changed | D9, D4 | v1 refuses cluster mode until the lease; after the lift the bound is the smaller of TTL + `S` and close detection + `G` + lease lapse; the half-open single-node bound (101 s) is stated. |
| 10 | Aggregate coalescing: shared vs per subscription; empty map; exact snapshot; aggregates after restart | adopted, changed | D10.4 | Shared accumulator keyed with the authorization filter; LEAVE on n→0 including an empty map; exact snapshot, coalesced deltas; after a restart subscriptions are gone and re-subscribe to a fresh snapshot, so no DELETE replay. |
| M1 | TTL renewal protocol: re-write vs heartbeat | rejected | D4 | No renewal message: renewal is the connection's own `PING`; O(owned keys) renew traffic carries no information. 30 Hz cursors belong on topics (D0). |
| M2 | Enumerate the wire error codes | adopted | D10.1 | 409 `storage_class_mismatch`, 423 `ephemeral_not_owner`, 507 `ephemeral_capacity`, with dispositions, ranks and carriers; `QUERY_LIVE_AGGREGATE_UNSUPPORTED` (D10.2). |
| M3 | Fix the removal `reason` vocabulary | adopted, changed | D10.3 | `expired` and `owner_gone`; an explicit client remove carries no reason; journal and metric only, no `QUERY_UPDATE` field. |
| M4 | AVG precision and format | adopted | D10.4 | i128 integer SUM, Neumaier float SUM reset at count 0, AVG as f64 at emission. |
| M5 | Reserve the session-liveness message | adopted | D9, D10.6 | `SESSION_LIVENESS` with `SessionEntry`, shared with TODO-958. |
| M6 | Journal: shared ring or a separate small ring | adopted | D10.5 | A separate ring, default off, behind TODO-933. |

---

## Deferred items

Each is recorded on the TODO named.

- A schema-DSL mirror of the class (`defineMap(..., {storageClass})`) — TODO-959.
- An outbox opt-in for aggregates or transitions (a deviation from TODO-959's "journal/outbox as opt-in") — TODO-632,
  TODO-959.
- An in-memory transfer of ephemeral partitions for planned migration — TODO-850.
- MIN/MAX live aggregates — TODO-971.
- The `writeConcern` cap at `APPLIED` for ephemeral ops if enforcement ever lands — TODO-959.
- An answer frame for every refused non-batch operation — TODO-680.
- Cross-request order of durable `OP_BATCH` frames of one connection — TODO-478.
