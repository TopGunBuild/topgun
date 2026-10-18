# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Versions below 0.12 are pre-release internal milestones — TopGun's public
> history begins at v2.0.0.

## [Unreleased]

- **Changed (on-disk format):** the server keeps its store in a **data directory** and moves an
  existing store there by itself, once, at the first start of this version. Nothing has to be
  run by hand; the old store file is kept. An older server binary started on the node refuses
  to start instead of serving an empty store.
  - **Where the store lives:** `TOPGUN_DATA_DIR` names the store directory, which holds
    `engine.redb`. Its default is `{directory of TOPGUN_REDB_PATH}/topgun-data` when
    `TOPGUN_REDB_PATH` is set (so a store on a volume stays on it), else `./topgun-data`. The
    container image sets `TOPGUN_DATA_DIR=/data/topgun-data` and the Helm chart
    `{mountPath}/topgun-data`. A set-but-empty value is refused at startup. Every start logs
    `store location: data directory {D}` with `data_dir`, `data_dir_source` and
    `legacy_path`.
  - **`TOPGUN_REDB_PATH`** (default `./topgun.redb`) now names only the legacy store file of
    earlier versions — the source of the migration — and the place of the marker file below.
  - **One-time, resumable migration:** a table-per-map store file at `TOPGUN_REDB_PATH` with no
    store in the data directory is copied at start-up, before the WAL is opened and before the
    server listens. Rows and the progress mark are written in one batch, so a migration killed
    at any point resumes at the next start and ends with the same content; a supervisor that
    restarts the node converges, provided each run lives longer than start-up plus 2 s plus two
    batches (a batch is at most 10 000 rows or 8 MiB). The copy is verified byte for byte
    before anything is renamed. Log lines (`INFO`): `store format: migrating {L} into {D} from
    the table-per-map format`, `store migration progress` (at most every 10 s),
    `store migration: resuming from table {T}`, `store format: migration complete`.
  - **Free space:** the migration refuses to start, changing nothing, unless
    `1.5 × the size of the old file + 64 MiB` is free on the file system of the data directory:
    `store migration: refusing to start: … bytes are free on the file system of {D}, … are needed (1.5 × the size of {L}, plus 64 MiB); nothing was changed`.
  - **The old file is kept** as `{TOPGUN_REDB_PATH}.pre-migration`, and every start logs
    `store format: the pre-migration store file is kept at {K}; delete it once this store is confirmed`
    until an operator deletes it.
  - **A marker file at the old path:** while a store is open, `TOPGUN_REDB_PATH` holds a small
    text file starting with `TOPGUN-STORE-MOVED 1` that points at this entry. Do not delete it
    while an older binary may be started with that path: an older binary refuses the marker
    file, but on an absent path it would create an empty store.
  - **Older binaries refuse:** a binary built before this version, started with the same
    environment or with `TOPGUN_REDB_PATH` pointing at the data directory, exits 1 at its
    store open, before it opens the WAL or listens, and changes no file (tested for the build
    of `b346e277`, read from the source for `07d009f0`).
  - **Two stores refuse the start:** a table-per-map file at `TOPGUN_REDB_PATH` beside a data
    directory that already holds a store is refused, naming both; move one of them aside. Every
    refusal at the store open logs one `ERROR` line, exits 1 and creates, moves or deletes
    nothing. The troubleshooting guide lists each refusal and what to do.
  - **Rollback** is supported only as a return to the pre-migration file: stop the new binary
    cleanly (exit 0), move the data directory away, delete the marker file, rename
    `{TOPGUN_REDB_PATH}.pre-migration` back to `TOPGUN_REDB_PATH`, start the old binary. **Every
    write the new binary acknowledged is then absent on the server**; no recovery path for those
    writes is claimed. There is no tool that exports the new format back, and none is planned. A
    rollback after a `kill -9` of the new binary is out of contract. Rolling forward again
    migrates the old file anew, including what the old binary wrote meanwhile.
  - **Not claimed:** an older binary pointed at the data directory's `engine.redb` itself
    (`TOPGUN_REDB_PATH={D}/engine.redb`) is not protected — it opens that file as a store with
    no maps and serves empty maps. Never point an older binary there.
  - **Backup:** stop the server cleanly and copy the whole `TOPGUN_DATA_DIR` directory
    recursively, together with the WAL directory (`TOPGUN_WAL_DIR`); a copy of a running
    server's directory is not a consistent backup. Before upgrading, copy the
    `TOPGUN_REDB_PATH` file elsewhere: the migration keeps it beside the store, but only a copy
    on another disk survives the loss of this one.
  - **Size and order:** every record key is 9 bytes longer on disk (a keyspace byte, the
    partition and the map id precede the key). A scan of a map now returns its records in
    partition order, and by key within a partition, rather than in key order.
- **Removed (server):** the Postgres backend. TopGun keeps its data in the embedded store
  inside the server, so there is no database to run. Gone are the `postgres` cargo feature,
  `STORAGE_BACKEND=postgres`, reading `DATABASE_URL`, the Postgres service and the `dbtools`
  profile of `docker-compose.yml`, the PostgreSQL choice of the `topgun` CLI setup and its
  `dev --no-db` flag (the flag did nothing), the opt-in suite `map-name-postgres`, and
  `examples/push-worker/n8n-workflow.json`.
  - **If you ran on Postgres:** the server now refuses to start when `STORAGE_BACKEND=postgres`
    is set or when `DATABASE_URL` is set (an empty value counts), and says why in its first
    log line. Unset `DATABASE_URL` to start on the embedded store.
    There is no migration tool: data kept by the Postgres backend remains available with the server built from the tag `postgres-backend-last`.
  - **Backup:** stop the server (`SIGTERM`), copy the data directory (`TOPGUN_DATA_DIR`, see
    "Changed (on-disk format)" above) and the WAL directory (`TOPGUN_WAL_DIR`), start it again;
    restore by putting both back with the server stopped. A copy of a running server's files is
    not a supported backup.
- **Changed (deployment):** the `cluster` and `auto-setup` services of `docker-compose.yml`
  keep their data on named volumes, and every server service sets
  `TOPGUN_WAL_DIR=/data/topgun-wal` (the image's existing default, so no WAL moves). The Helm
  chart gets a persistent volume, correct probe paths (`/health/ready`), `replicaCount: 1`
  and the disruption budget off by default. Kubernetes does not allow `volumeClaimTemplates`
  to change on an existing StatefulSet, so `helm upgrade` of a release installed from an
  earlier chart is refused; install such a release anew (the earlier chart kept no data on a
  volume).

- **Fixed (server):** a server on the embedded redb store no longer runs on while storing
  nothing after the database reports a fault: it now stops by itself with exit code 72, and
  a restart brings every acknowledged write back. Before, once redb reported an I/O error —
  a full disk included — it refused every later write on the same handle, also after the
  disk had room again, while the server kept acknowledging writes, retried each for as long
  as the process ran, and on a stop reported a clean exit 0. Now, when redb reports an I/O
  error, a previous I/O error, corruption or a poisoned lock, the server stops calling it,
  refuses writes arriving meanwhile as a temporary error (the client keeps them and sends
  them again), answers 503 on `/health/ready`, and shuts down with exit code 72. The
  acknowledged writes redb did not take stay in the write-ahead log and are replayed at the
  next start, which opens a fresh database handle. A remove is also no longer reported
  stored when redb could not read its table list. What changes for whoever runs the server:
  - **Run it under a supervisor that restarts on a non-zero exit code** (as for exit codes
    70 and 71). While the disk is still full, each restart exits with code 1 when it opens
    the database, until there is room; give the restart a delay of several seconds.
  - **New log lines and a metric.** One error line names the cause (`io` with its
    `io_kind`, `previous_io`, `corrupted` or `lock_poisoned`); the binary logs
    `the durable store is broken; shutting the node down` with `exit_code=72`. The new
    counter `topgun_write_behind_store_broken_total{cause}` renders all four causes at 0
    from the first scrape.
  - **Other store errors are unchanged:** they are retried until stored, as before. Postgres
    classifies none of its errors.
  - **For embedders:** `MapDataStore` gains `classify_error`, whose default answer is
    `StoreErrorClass::Unknown` (retry as before), so existing implementations compile
    unchanged; `WriteBehindDataStore::hard_flush` returns an error when the inner store
    reported itself broken, and the failure that `background_failure()` reports gains the
    methods `store_broken()` and `cause_label()`.

- **Fixed (server):** a write the server acknowledged is no longer lost when the storage
  refuses it. Before, the server tried to store a write three times and then gave up on
  it. The write stayed readable only while its record was in memory: after the record was
  evicted, a read returned the previous value, or nothing for a new key; a delete the
  storage had refused came back at once; and the next write to the same record was built
  on that old value and stored that way. Only a restart brought the write back. Now a
  write the storage refuses stays in the server's write buffer, stays readable, and is
  retried until the storage takes it — a storage outage of any length loses no
  acknowledged write on a running server. The retry limit is gone (it was never a
  setting). The server spaces its retries, so a storage that is down is called at most
  20 times in the first minute, and less often after that, however many writes are
  waiting, and after the storage returns
  every waiting write is attempted within about 6 s. What changes for whoever runs the
  server, and what remains:
  - **During an outage the write buffer fills, and then new writes are refused.** Nothing
    leaves the buffer while the storage refuses, so once `TOPGUN_WRITEBEHIND_CAPACITY`
    keys are buffered, a write of a key that is not buffered yet is refused as a
    temporary error; the client keeps it and sends it again. Writes of keys that are
    already buffered are still accepted. `TOPGUN_WRITEBEHIND_CAPACITY=0` together with a
    storage that refuses writes is not supported: nothing then limits the buffer.
  - **New log lines and metrics.** One warning, at most every 30 s, says that writes are
    being retried and names one of them; one line says when the backlog has cleared.
    After an outage has lasted `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` (60 s by default)
    the existing `WAL watermark stalled` error is also logged, every 6 s per affected
    partition, until the storage returns; that is expected. Five new metrics,
    `topgun_write_behind_flush_refusals_total`, `…_buffered_entries`,
    `…_retrying_entries`, `…_oldest_refusal_age_seconds` and `…_store_hold`, show the
    backlog; `topgun_write_behind_buffered_entries` is the one to watch against the
    capacity.
  - **A record the storage never accepts is retried for ever.** The server cannot tell a
    storage that is down from a record the storage rejects. Such a record keeps one
    place in the buffer, and enough of them fill it; there is then no time by which
    every other waiting write is stored, although a new write the storage accepts is
    stored within about 6 s of the moment it is due. Three refusals in a row also make
    the server leave the storage alone for a while, even when the storage is up and
    only those records are refused.
  - **Cleanup of deleted records stops on the whole server while any write is
    refused,** and that partition's write-ahead log is not collected. For an outage that
    ends this is intended; for a record the storage never accepts it lasts until the
    record is written again or the server restarts.
  - **A write the storage refuses while the server starts** — during recovery from the
    write-ahead log — is not retried this way: the server serves the previous value of
    that record until its next restart.
  - **Shutdown during an outage** tries each buffered write once and can take the full
    `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`; what it could not store is applied from
    the write-ahead log at the next start.
  - **Postgres was not measured,** and no performance measurement was made.
- **Changed (client):** the client now applies the server's map-name rule before anything
  is stored, so a bad name fails at the call that used it instead of coming back later as a
  refused write. Names with `-`, `/` and `.` are accepted — `user-profiles`,
  `users/profiles`, `todo/<uuid>`. `getMap` and `getORMap` throw for a name that is empty,
  contains `:`, ends in `__backup` or is longer than 512 bytes (UTF-8 bytes, not
  characters); each case has its own message. A write the server refuses for its name — one
  queued by an older client, for example — is reported as a write rejection with the new
  cause `invalid_map_name` instead of `unknown`. After a restart, pending writes the client
  holds back for their name (empty, or containing `:`) are listed by name in the warning it
  logs.
- **Breaking (client):** queries refuse a name the server cannot serve. `client.query('')`
  throws, and `queryOnce('')` / `queryOncePaged('')` reject their promise; the same holds
  for a name that ends in `__backup` or is longer than 512 bytes. `useQuery('')` does not
  throw: it returns an `error`, `loading: false` and empty data, and does not subscribe.
  Before, `useQuery('')` returned no error and empty data, with `loading` turning `false`
  once the local read finished, so some apps passed `''` to mean "do not subscribe". There
  is no option to skip a query yet: such an app now gets an `error` from the hook and has to
  stop rendering the hook or pass a real name. The rule covers exactly these methods:
  `getMap` and `getORMap` (the full client rule, `:` included) and `query`, `queryOnce`,
  `queryOncePaged` and `useQuery` (the same rule without the `:` restriction, so a map that
  already has `:` in its name can still be read). `search`, `vectorSearch`,
  `searchSubscribe` and `hybridQuery` are not covered.
- **Fixed (server):** a server whose background flush stops working no longer keeps
  accepting writes it will never store. Before, if the background task that carries
  acknowledged writes to the store crashed, the server went on acknowledging writes, reported
  itself healthy and, when stopped later, exited with code 0 as after a clean stop. Now the
  server stops itself: writes that arrive at that moment are refused as a temporary error, so
  the client keeps them and sends them again once the server is back; `/health/ready` answers
  503; the server stores what is still queued and exits with code 70, or 71 if that final
  step fails or takes too long. Run the server under a supervisor that restarts it on a
  non-zero exit code, and let it tolerate up to 70 s (with default settings) for this stop.
  After the restart, the writes the crashed task was holding are replayed from the write-ahead
  log. The same applies when the task that watches for stalled flushes crashes. One change
  reaches a healthy server too: an ordinary stop now synchronises the write-ahead log to disk
  before the process exits. The mechanism: a panic of the write-behind flush task or of the
  stall watchdog is caught on the dying task and puts the store in a failed state that it
  never leaves; the exit code is non-zero whether the panic came before or after a stop
  signal. For embedders, `hard_flush` returns an error on a failed store. Two new metric
  series, `topgun_write_behind_background_task_panics_total` with `task="flush"` and
  `task="stall_watchdog"`, count these failures and read 0 on a healthy server.
- **Fixed (server):** with the embedded store, the server's memory no longer grows with the
  number of different map names it has seen. Before, every new map name the server read or
  wrote stayed in memory until the process stopped, so an application that keeps creating
  maps under fresh names (one per document, session or user) made the server grow for as
  long as it ran. Stored data and its format on disk are unchanged.
- **Fixed (server):** writes packed into a batch envelope now count against the connection's
  rate limit like the same writes sent directly. Before, a batch of writes placed inside an
  envelope counted as one request however many writes it held. A client that packs batches
  into envelopes can now be told to slow down (error 429) where it was not before; the
  message and the handling are the ones a direct batch gets. The stock TopGun client sends no
  envelope, so applications built on it see no change. This closes the difference the entry
  below left open. What changed is how much one request counts for; the limit itself, its
  defaults and how it treats a single request larger than its burst are as they were.
- **Internal (server):** no visible change. The server's code that writes a record must now
  present proof that it holds that record's write lock, and the build fails where it does
  not; a write that presents the lock of a different record is refused and logged as a
  server error. Before, this rule was kept by review and by counting call sites.
- **Fixed (server):** writes sent inside a batch envelope are now handled exactly like
  writes sent directly, including refusals and back-off. Before, a batch of writes packed
  into an envelope was processed as one unit: if the server refused a single write in it,
  none of the batch was stored and the client was told nothing; when the server was
  overloaded the client got no signal to slow down. Now each write of a batch in the
  envelope is accepted or refused on its own and the client is told which, and an
  overloaded server answers with the same back-off message it sends for a direct write.
  What a direct write is not told, a write in an envelope is not told either: a single
  write sent outside a batch gets no message when it is refused for a reason other than
  overload, and a batch the server was too loaded to finish gets the back-off message
  and no acknowledgement. One difference remains: the request rate limit counts an
  envelope by the messages packed in it, not by the writes inside them.
- **Fixed (server):** two writes to the same record that reached the server at the same
  moment could leave it holding the older value after a restart, although it had
  acknowledged and served the newer one. A write that arrived while the server was
  attaching an embedding to the same record could be overwritten by that embedding update.
  Every write of a record is now serialised per record, so the case in which a restart
  kept the older of two values can no longer arise. One gap remains on maps with
  automatic embeddings: after an unclean stop, a write that followed an embedding update
  can still be lost on restart. This change does not fix that.
- **Fixed (server):** after a storage outage with clients still writing, the server could
  keep counting writes that had already been replaced by newer ones. Over time that shrank
  its write buffer, stopped the cleanup of deleted records and kept part of its
  write-ahead log from being collected, until the server was restarted. It now returns to
  a clean state when the storage recovers. The cause was a write the store had refused:
  if a newer write of its key was already waiting in the write buffer when the refused
  write came up for its retry, the older write was dropped without its bookkeeping. It
  is now folded into that newer write. No
  acknowledged write was lost by the defect. Measured on the embedded (redb) backend with
  six keys refused while they were still being written: before the fix, three partitions
  stayed behind their log for the whole 15 s that were measured, in 3 of 3 runs; after
  it, every partition caught up within about 2 s, in 5 of 5 runs. What remains:
  - **A write the server gives up on is folded into a newer write only if that newer
    write is already waiting in the write buffer at that moment.** If the newer write
    has been logged but has not reached the buffer yet, if it arrives later, or if
    there is none, the write the server gave up on is still applied again only by a
    restart, and until then that partition's write-ahead log is not collected, as
    described in the next entry.
  - **Shutdown during an outage, and a crash of the background flush task,** are not
    covered by this fix.
  - **Postgres was not measured.**
- **Fixed (server):** a write the store refused could stop the whole server. It answered
  no new connection, logged nothing more, ignored SIGTERM and had to be killed. The
  cause was a lock kept during the wait between two store retries; the lock is now
  released before that wait. What was measured is narrower than "any storage failure":
  a refusal injected into the embedded (redb) backend, with one or two refused keys. In
  that case the server keeps answering reads and new connections, reports each write it
  gave up on in the log (`Write-behind entry discarded after max retries`, with its map
  and key) and exits on SIGTERM. Limits that remain:
  - **A discarded write is not stored.** After the maximum number of retries it stays
    readable from memory until it is evicted or the server restarts, and it stays in
    the WAL. Only a restart applies it again; until then that partition's WAL is not
    collected.
  - **Shutdown during an outage can take the full
    `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`.** Measured with 40 refused entries and
    the default 30 s: 30.00 s. With one or two refused entries it took under 2 s. A
    supervisor with a shorter grace period (`docker stop` waits 10 s) will kill the
    process first.
  - **Postgres was not measured.**
- **Fixed (`@topgunbuild/client`):** a client closed before its socket had opened no
  longer comes back to life. Before, a socket that reported "open" after `close()`
  made the closed client authenticate, start its heartbeat and then reconnect for
  ever. In Node, the client's background timers (heartbeat, reconnect, token refresh,
  cluster health check) also no longer keep a process running by themselves; an open
  connection still does.
- **Breaking (server, map names):** map names that are empty, longer than 512 bytes,
  end in `__backup` or contain a NUL character are now refused for client operations.
  Data already stored under such a name is not deleted: it stays in the store, but
  clients can no longer address it. No rename tool is provided. How a client sees the
  refusal depends on the request. A batched write (what the SDK sends) and an HTTP
  `/sync` operation are refused explicitly with `InvalidMapName` (code 400). A query, a
  subscription, a sync request or a single un-batched operation on such a name gets
  **no response**: the server refuses it and logs a warning, but sends nothing back, so
  an older SDK keeps waiting. In particular `query('')` used to get an empty answer and
  now gets none.
- **Fixed (server, embedded backend):** names with `-`, `/`, `.` and other characters
  outside the identifier class (`^[a-zA-Z_][a-zA-Z0-9_]*$`) are now durable on the
  embedded (redb) backend. Before, a write to such a map was acknowledged and then
  missing after a restart.
- **Behavior change (server, upgrade):** maps whose writes were stranded reappear after
  the upgrade. Writes the previous version acknowledged for such names and could not
  store are replayed from the WAL on first boot. An application that renamed its map
  since then will see the old map's data come back under the old name. The first boot
  after the upgrade replays every stranded write, so it can take longer than usual, and
  it registers every stranded map name in the store.
- **Breaking (`@topgunbuild/client`, `@topgunbuild/core`, server):** a node id may no
  longer contain `|` or `#` (`:` was already refused). `new TopGunClient({ nodeId })`
  and `new HLC(nodeId)` throw for such an id, and a server started with such a
  `--node-id` exits at startup with a `FATAL:` line, before it opens a port. Ids you
  did not choose are unaffected: the client's auto-generated id and the server's
  default id are always valid. New in `@topgunbuild/core`: `HLC.assertValidNodeId(nodeId)`
  checks an id without creating a clock and throws the same error as the constructor.
- **Breaking (server, OR-Map):** the server refuses an OR-Map tag that is empty or
  contains `|` or `#`. A write carrying such a tag is rejected as a schema-validation
  refusal, and a pushed OR-Map difference carrying one is not merged and is not
  answered. The SDK builds its tags from the node id, so with a valid node id it
  never sends such a tag; this concerns code that speaks the wire protocol with tags
  of its own. A tag the server already stores can still be removed.
- fix(client): creating a client with a refused node id no longer leaves `online` /
  `offline` listeners behind in the browser. The id is now checked before anything
  else is set up, so a constructor that throws leaves nothing running.
- fix(sync): a write made while offline or still waiting to be sent is no longer
  dropped after a search unsubscribe or an OR-Map sync. The server used to answer those
  requests (and an empty batch of writes) with an operation acknowledgement, which the
  client read as "everything pending was saved" and discarded writes the server had
  never received. The server no longer sends an acknowledgement for them, and the client
  now treats an acknowledgement as confirming only the writes of a batch it actually
  sent — so it is also protected against servers that are already deployed.
- **Breaking (`@topgunbuild/client`):** a custom connection provider that implements
  `sendBatch` must report each batch it sends through the new second argument,
  `onBatchSent(opIds)` — once per `OP_BATCH` it puts on the wire, with that batch's
  operation ids in order, before `sendBatch` returns. A provider that does not report
  its batches still sends them, but its writes are not confirmed: they stay pending and
  are sent again. The client logs a warning the first time this happens. The built-in
  providers, including cluster mode, already report their batches.
- feat(client): `IConnectionProvider` has a new optional member, `transport`. A custom
  provider may declare `transport: 'websocket'` or `transport: 'http'`; the built-in
  providers do. A provider that omits it keeps working, without the extra check the
  client applies to acknowledgements that arrive over a WebSocket.
- **Supported servers (`@topgunbuild/client`):** this client requires a TopGun server
  `2.0.0` or later. The TypeScript server (`@topgunbuild/server` `0.11.0` and earlier)
  is not supported: against it, writes are not confirmed.
- fix(client): in cluster mode, a message that arrives late on a node connection the
  client has already dropped is ignored instead of being read as coming from that node's
  new connection.
- fix(sync): an OR-Map client and the server now compute the same Merkle leaf hash
  for a key that holds the same tags on both sides. Previously the two used different
  formulas, so an OR-Map key could never compare equal during sync. Both sides now hash
  only the key's tags — the live ones and the removed ones — not values or timestamps.
- **Behavior change (OR-Map sync):** a client pushes to a key only the removals that
  belong to that key, instead of every removal it knows about for the whole map. A key
  whose values were all removed keeps its place in the Merkle tree for as long as its
  removals are remembered, so the removal itself is compared during sync rather than
  the key simply looking absent.
- **Breaking (`@topgunbuild/core`):** `hashORMapEntry` and `hashORMapRecord` are
  removed. Use `hashORMapLeaf(key, liveTags, tombstoneTags)`, which is the one hash
  the client and the server share.
- **Breaking (`@topgunbuild/core`):** `ORMapMerkleTree.update(key, records)` is now
  `update(key, liveTags, tombstoneTags)`. Passing two empty sets removes the key from
  the tree.
- **Breaking (`@topgunbuild/core`, types):** `ORMapSnapshot` has a new required field,
  `keyTombstones: Map<K, Set<string>>`. Code that only reads `ORMap.getSnapshot()` is
  unaffected; code that constructs an `ORMapSnapshot` itself must now supply it (an
  empty `Map` when there are none).
- feat(core): `ORMap.getKeyTombstones(key)` and `ORMap.setKeyTombstones(key, tags)`
  read and replace the removed tags recorded against a single key, and
  `ORMap.applyTombstone(tag, key?)` takes an optional `key` to record a remote removal
  against the key it belongs to. `ORMap.remove(key, value)` does this automatically.
  Omitting `key` keeps the previous behaviour: the tag is suppressed map-wide and
  belongs to no key.
- fix(query): a top-N page (a `limit` with no filter, cursor, or aggregation) over a
  map larger than RAM now streams the durable records through a bounded `limit+1` heap
  in the scan itself — it no longer materializes every non-resident row just to return a
  small page. Peak memory for such a page is O(resident) + O(limit) + one scan batch.
- fix(query): full-scan pager uses cap=limit+1 for correct pagination + mandatory _key
  tie-break shared by the scan pager and the sort stage (deterministic top-N boundary)
- **Behavior change (query result ordering):** sorted queries now break ties on the
  record key (`_key` ascending). Rows that compare equal on the sort field may surface
  in a different order than before, but the order is now deterministic across runs.
- **Behavior change (query result ordering):** a query with `limit` but no sort now
  returns rows in ascending key order. Previously the order was unspecified.
- feat(query): full-scan reads stream durable (non-resident) records and merge them
  HLC-LWW against the in-memory snapshot, so queries see records that were evicted from
  or never loaded into memory — without ever materializing the whole map in RAM.
- feat(query): new typed response code `QUERY_SNAPSHOT_OVERFLOW` (too many concurrent
  writes during a live snapshot; the client should resubscribe). A sort without a `limit`
  over a resident map is permitted (an O(result) in-memory sort, soft-capped by
  `max_query_records` — the same profile as a no-`limit` scan); the `QUERY_UNBOUNDED_SORT`
  code is reserved for a future size/residency-gated reject (sorting over a
  larger-than-RAM, non-resident match set).
- note(query): per-query Merkle is retired on the full-scan path (SYNC/QUERY separation);
  `merkleRootHash` is no longer returned. Reconnect falls back to a full snapshot
  (correct, less efficient) until the map-level Merkle tree-walk SYNC lands; clients
  degrade gracefully (`merkleRootHash` undefined → full snapshot).

## [2.0.0] - 2026-05-23

> First general-availability release of TopGun v2. Complete rewrite from the v1
> gun.js port. New Rust server, new client API, embedded backend by default,
> single-node production-ready.

### Breaking Changes

- **SDK API unified on `.subscribe` / `.onDelta`** — removed the old per-handle
  observe/onChange variants. `client.getMap(name).subscribe(fn)` returns an
  unsubscribe handle on both LWW and OR maps.
- **`TopGunClient` is now generic** — `new TopGunClient<TSchema>({...})`. Map
  and query accessors narrow types from the schema. Untyped (no generic) still
  works.
- **`TopGun` facade removed** — import and instantiate `TopGunClient` directly.
- **`@topgunbuild/client` IDBAdapter export removed** — use
  `@topgunbuild/adapters/IDBAdapter` (memory-first queueing implementation).
- **Server binary renamed `test-server` → `topgun-server`** — `cargo run --bin
  topgun-server --release` everywhere. The Docker image artifact path was
  already `/usr/local/bin/topgun-server`.
- **`JWT_SECRET` is required** — the server refuses to boot with the previous
  baked-in default and exits with a clear error if neither `JWT_SECRET` nor
  `TOPGUN_NO_AUTH=1` is set.
- **`@topgunbuild/client` `client.executeOnKey` / `executeOnKeys` and
  `ConflictResolverClient.register` / `unregister` / `list` now throw** —
  server-side custom entry processors and conflict resolvers require a WASM
  sandbox that is on the v2.x roadmap. Built-in CRDT merge logic and
  `useMergeRejections` / `ConflictResolverClient.onRejection` are unchanged.
- **`useEntryProcessor` and `useConflictResolver` React hooks removed.**
- **Default storage backend is embedded redb** (`STORAGE_BACKEND=redb`).
  Postgres still works via `STORAGE_BACKEND=postgres` + `DATABASE_URL`.
  The legacy SQLite server adapter was removed.

### Added

#### Embedded Storage (redb)
- **redb default** — server writes survive restart with no external
  dependency. File location configurable via `TOPGUN_REDB_PATH`
  (default `./topgun.redb`).
- **Production defaults for in-memory cache + write-behind buffer** —
  `TOPGUN_MAX_RAM_MB`, `TOPGUN_EVICTION_HIGH_PCT`, `TOPGUN_EVICTION_LOW_PCT`,
  `TOPGUN_EVICTION_INTERVAL_MS`, `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS`,
  `TOPGUN_WRITEBEHIND_BATCH_SIZE`, `TOPGUN_WRITEBEHIND_CAPACITY`. See
  `CLAUDE.md` for ranges.
- **`TOPGUN_CORS_ORIGINS`** — comma-separated env var to enable browser
  cross-origin access (default deny-all).
- **Scalar index startup rebuild** with durability-before-ack contract.

#### Client SDK
- **`NullConnectionProvider`** — `TopGunClient` boots without a `serverUrl`
  for pure local-only apps.
- **`AuthRequiredError`** + `TopGunClientConfig.onAuthRequired` callback —
  when the server sends `AUTH_REQUIRED` with no token configured, the
  client now logs warn + invokes the callback instead of parking silently.
- **`useSyncState`** + `useMapWithSyncState` + `useORMapWithSyncState`
  React hooks for surface-level write-concern UX.
- **`RecordSyncStateTracker`** in `SyncEngine`.

#### Tooling
- **`create-topgun-app`** — `npx create-topgun-app my-app` scaffolds a
  working Vite + React + TopGun app with a hook-first todo demo.
- **Docker images on GHCR** — `ghcr.io/topgunbuild/topgun-server:latest`
  (multi-arch linux/amd64 + linux/arm64).
- **`@topgunbuild/mcp-server`** — MCP server for Claude Desktop and Cursor.
  Exposes eight tools (`topgun_query`, `topgun_mutate`, `topgun_search`,
  `topgun_subscribe`, `topgun_schema`, `topgun_stats`, `topgun_explain`,
  `topgun_list_maps`).

#### Cluster (single-node stable; cluster-mode partition-routing in progress)
- **Quorum-election state machine** replacing the discover-then-join protocol.
- **Cluster member lifecycle fixes** — `MemberAdded` events, disconnect on
  `MemberRemoved`, rejoin deduplication, no-auth bypass for dev clusters.

#### Docs
- New migration guides: Firebase, Y.js, Replicache, Supabase Realtime.
- New `/docs/guides/security` and `/docs/guides/mcp-server` guides.
- Roadmap, FAQ, troubleshooting, benchmarks rebuilt.
- `building-with-ai.mdx` AI Builder Guide with hook-first canon + MCP snippets.
- `llms.txt` + `llms-full.txt` for AI assistants.
- "Coming from X" comparison landing component.
- `SECURITY.md` responsible-disclosure policy.

### Changed

- Hero / landing positioning to outcome-first: "Build real-time apps that work
  offline."
- Landing fork (`landing-astro-next`) merged into the unified `docs-astro` app.
- Performance benchmarks: fire-and-forget reported as 483K ops/sec
  (corrected from a retired 560K figure).
- `TopGunClient.close()` is now async; all callers updated.

### Fixed

- 3 critical npm advisories in the prod dep tree resolved by bumping
  `better-auth >= 1.4.17` (2FA bypass), `@clerk/clerk-react >= 5.61.6`
  (middleware bypass), and forcing `fast-xml-parser >= 5.3.5` via pnpm
  override (RSS DOCTYPE bypass via `@astrojs/rss`).
- `ClusterState::update_view` made monotonic to prevent stale-view clobber.
- `topgun-server --port` default normalized to 8080; `--port 0` honored for
  ephemeral allocation in tests.
- `STORAGE_BACKEND=null` propagated through cluster spawn env (cluster test
  stability).
- Numerous cluster-routing teardown fixes (await `pool.close()`,
  `verifier.close()`, `removeNode` promises).
- `@topgunbuild/client` `pino-pretty` redirected to stderr.
- `apps/admin-dashboard` Server-Unavailable overlay no longer displays a
  non-existent `cargo run --bin topgun-server` command (the name now
  matches the actual binary).
- `apps/docs-astro/src/content/docs/guides/index.mdx` rewritten to list only
  guides that actually exist (was 22 cards, 9+ pointed at 404).
- 13 stale code samples in docs-astro corrected (`map.getAll()`, `auth: {token}`,
  `useTopic` destructuring, `create({...})`, `client.getTopic`, `cargo run --bin
  test-server`, landing "one binary" claim, `IDBAdapter('my-app')`, port 8090,
  `topgun_get/set`, quick-start slug).
- 14 SPEC-NNN references removed from `packages/server-rust/src/` to honor
  the CLAUDE.md code-comments convention.

### Removed

- **`TopGun` facade** and `TopGun.test.ts`.
- **`@topgunbuild/client` `IDBAdapter` re-export** (use `@topgunbuild/adapters`).
- **Legacy SQLite server adapter** (`BetterSqlite3Adapter`) — superseded by redb.
- **`WHITEPAPER.md`, `MIGRATION.md`, `TESTING.md`, `TODO_NEXT.md`** — outcome-
  level positioning conflicted with `CLAUDE.md`; superseded by docs-astro.
- **`specifications/` directory** (15 monolithic spec files) and `docs/`
  directory (4 legacy markdown pages) — content fully ported to
  `apps/docs-astro/`.
- **`start-clerk-server.sh`** — TS-server-era leftover; the notes-app README
  was rewritten to use `pnpm start:server` directly.
- **`scripts/analyze-bottlenecks.md`** — referenced a non-existent
  `profile-server.sh` and cited stale benchmark numbers.
- **`apps/docs-astro/src/content/blog/conflict-resolution-beyond-lww.mdx`** —
  blog promoted a feature whose backend (WASM sandbox) is on the roadmap.
- **Personal Cloudflare account_id and `easysolpro.workers.dev` URLs** stripped
  from `examples/push-worker`, `examples/storage-worker`, `examples/notes-app`.
- Dead `Hero.tsx` from landing (zero imports).
- 488 phase/spec/bug references from code comments (CLAUDE.md convention).

### Legal

- Apache ICLA at `.github/CLA.md`; cla-assistant.io bot enforcement.
- Grandfathered committers snapshot at `legal/GRANDFATHERED_COMMITTERS.md`.

### Tests

- Parallel 3-node cluster bootstrap integration tests with `serial_test`
  annotation for shared-port suites.
- Cluster failover, partition-routing, rebalance k6 scenarios.
- 4 Jest scaffold tests for `create-topgun-app`.

---

## [0.11.0] - 2026-02-07

### Added

#### HTTP Sync Protocol (`@topgunbuild/core`, `@topgunbuild/client`, `@topgunbuild/server`)
- **Stateless HTTP sync** — new `POST /sync` endpoint enables CRDT synchronization without WebSocket connections, designed for serverless environments
- **`HttpSyncProvider`** — client-side connection provider that translates sync operations into HTTP POST requests with configurable polling interval (`pollIntervalMs`)
- **`AutoConnectionProvider`** — automatically selects WebSocket or HTTP transport with transparent fallback after configurable retry attempts (`maxWsAttempts`)
- **`HttpSyncHandler`** — lightweight, stateless server-side handler for processing sync requests in serverless functions (Vercel Edge, AWS Lambda, Cloudflare Workers)
- **HTTP sync Zod schemas** — `HttpSyncRequestSchema` and `HttpSyncResponseSchema` for request/response validation with msgpackr serialization
- **Delta computation via HLC** — efficient timestamp-based filtering using `HLC.compare()` instead of Merkle tree round-trips

#### Deterministic Simulation Testing (`@topgunbuild/core`)
- **`VirtualClock`** — injectable time source for deterministic test execution
- **`clockSource` injection** — HLC, LWWMap, and ORMap now accept pluggable clock sources
- **DST infrastructure** — `SeededRNG`, `VirtualNetwork`, `InvariantChecker`, `ScenarioRunner` exported from core package

#### Query Optimizer Enhancements (`@topgunbuild/core`)
- **Point lookup optimization** — `QueryOptimizer` detects equality filters on indexed fields and generates `PointLookupStep`/`MultiPointLookupStep` execution plans
- **Network-aware cost model** — distributed query cost estimation accounting for network latency, partition count, and data transfer
- **Index hints** — `QueryOptions.indexHints` allows explicit index selection (`force`, `prefer`, `ignore`) for query planning

### Fixed
- **server**: Correct `HttpSyncError` field types and add optionality markers
- **server**: Preserve TLS status message in HTTP sync handler
- **client**: Align `requestTimeoutMs` default with spec (30s)
- **core**: Consolidate `ClockSource` interface to single definition

### Documentation
- Serverless deployment guide with Vercel Edge, AWS Lambda, and Cloudflare Workers examples
- `HttpSyncProvider` and `AutoConnectionProvider` API reference
- `POST /sync` endpoint reference with request/response schemas
- HTTP Sync section in transport decision guide
- "TopGun Goes Serverless" blog post
- "Merkle Trees & Delta Sync" blog post

---

## [0.10.1] - 2026-01-20

### Fixed
- Minor fixes and stability improvements

---

## [0.10.0] - 2026-02-04

### Breaking Changes

- **client**: Remove deprecated `ClusterClient.sendMessage()` - use `send(data, key)` instead
- **core**: Remove legacy constructor from `QueryOptimizer` - use options object
- **core**: Remove legacy array format from `CRDTDebugger.importHistory()` - use v1.0 format

### Added

#### Observability & Debugging
- **CRDTDebugger** - Full CRDT operation recording, replay, conflict detection, and export (JSON/CSV/NDJSON)
- **SearchDebugger** - BM25 TF-IDF breakdown, RRF rank contributions, timing analysis
- **PrometheusExporter** - Comprehensive metrics: connections, operations, CRDT merges, sync, cluster, storage, queries
- **Grafana Dashboard** - Pre-built dashboard JSON for TopGun monitoring

#### Admin Dashboard
- **SetupWizard** - 6-step guided setup for new deployments
- **DataExplorer** - Browse and edit map data with JSON editor
- **QueryPlayground** - Monaco editor with Cmd+Enter execution
- **ClusterTopology** - Visual cluster ring and partition distribution
- **CommandPalette** - Cmd+K quick navigation
- **Settings** - Hot-reloadable runtime configuration
- **Login/Auth** - JWT-based admin authentication

#### Distributed Search & Subscriptions
- **ClusterSearchCoordinator** - Scatter-gather distributed FTS with RRF merge
- **DistributedSubscriptionCoordinator** - Live query subscriptions across cluster nodes
- **QueryCursor** - Cursor-based pagination for distributed queries
- **SearchCursor** - Cursor-based pagination for distributed search
- **CLUSTER_SEARCH_REQ/RESP** - New cluster protocol messages
- **CLUSTER_SUB_REGISTER/UPDATE** - Distributed subscription protocol

#### CLI & Developer Experience
- **CLI Commands** - `topgun doctor`, `setup`, `dev`, `test`, `config`, `cluster`, `debug:crdt`, `debug:search`
- **BetterSqlite3Adapter** - SQLite storage adapter for server-side persistence
- **DevContainer** - VS Code/Codespaces development environment
- **Docker Compose Profiles** - `admin`, `monitoring`, `dbtools` profiles

#### Infrastructure
- **Environment Validation** - Zod schema for server configuration
- **TOPGUN_DEBUG_ENDPOINTS** - Security control for debug endpoints (disabled by default)
- **foreignKeyMap** - Custom database schema support for better-auth adapter

### Changed

- **server**: Refactor DistributedSubscriptionCoordinator into focused coordinators (Base, Search, Query)
- **server**: Wire SearchCoordinator.dispose() into LifecycleManager for timer cleanup
- **server**: Extract handlers from ServerCoordinator into domain modules
- **core**: Split schemas.ts (1160 lines) into domain-focused modules
- **core**: Structured logging with pino throughout codebase

### Removed

- Dead code from ServerCoordinator (~1,263 lines)
- Phase/Spec/Bug references from code comments (488 occurrences)
- `jest.retryTimes` from flaky tests (proper fixes applied)

### Fixed

- Timer cleanup in SearchCoordinator preventing memory leaks
- Type safety improvements across client message handlers
- WebCrypto polyfill for consistent test environment

### Tests

- CLI command tests (28 tests covering all commands)
- 1814 tests in core package
- Cluster E2E tests for partition routing, node failure, replication

## [0.9.0] - 2026-01-06

### Added

#### MCP Server Package
- **@topgunbuild/mcp-server** - New package enabling AI assistants (Claude Desktop, Cursor) to interact with TopGun databases via Model Context Protocol (MCP)
- Query, mutate, and search data using natural language
- Full-text hybrid search (BM25 + exact matching)
- Real-time subscriptions for watching changes
- Schema inspection and query explanation
- HTTP and stdio transports
- Security controls: restricted maps, read-only mode, auth tokens

#### Documentation
- Comprehensive MCP Server guide

## [0.8.1] - 2026-01-05

### Added

#### Hybrid Search with RRF Fusion (Phase 12)
- **Reciprocal Rank Fusion (RRF)** for merging search results from multiple sources
- **QueryExecutor** with parallel FTS + filter execution
- **LiveFTSIndex** for live hybrid query updates
- **HybridQueryHandle** for client-side hybrid queries
- **useHybridQuery** React hook for faceted search UIs
- **_score sorting** for relevance-based results

#### New Predicates
- FTS: `match()`, `matchPhrase()`, `matchPrefix()`
- Filters: `eq()`, `gt()`, `lt()`, `gte()`, `lte()`, `contains()`
- Logical: `and()`, `or()`

## [0.8.0] - 2026-01-04

### Added

#### Full-Text Search (Phase 11)
- **BM25 Ranked Search** - Industry-standard relevance algorithm with Porter stemming and 174 English stopwords
- **Server-side FTS Indexes** - Centralized indexes with automatic updates on data changes
- **Live Search Subscriptions** - Real-time search results with delta updates (ENTER/UPDATE/LEAVE events)
- **useSearch React Hook** - Easy integration with debounce support for search-as-you-type UIs
- **O(1) Live Updates** - scoreSingleDocument optimization for 1000x faster live search updates
- **Notification Batching** - 16ms batching window to prevent update storms

### Performance
- scoreDocument (1K docs): 1000× faster
- scoreDocument (10K docs): 10000× faster

## [0.7.0] - 2026-01-01

### Added

#### Cluster Replication (Phase 10)
- **ReplicationPipeline** - Async and sync replication with configurable consistency levels (EVENTUAL, QUORUM, STRONG)
- **Gossip Protocol** - Automatic cluster member discovery
- **Partition-aware routing** - Direct operations to partition owners

#### Anti-Entropy Repair
- **MerkleTreeManager** - Efficient hash-based data verification using Merkle trees
- **RepairScheduler** - Automated periodic repair with configurable scan intervals
- **Bucket-level comparison** - Minimizes network traffic

#### Failover Handling
- **PartitionReassigner** - Automatic partition rebalancing on node failure
- **Phi Accrual Failure Detector** - Adaptive failure detection
- **ReadReplicaHandler** - Support for reading from replica nodes

#### Documentation
- New "Cluster Replication" guide

### Tests
- 38 E2E cluster tests covering replication, failover, and partition routing

## [0.6.0] - 2025-12-31

### Added

#### Lazy Index Building (Phase 9.01)
- `LazyHashIndex`, `LazyNavigableIndex`, `LazyInvertedIndex` classes
- Deferred index construction until first query for fast startup
- Progress callbacks for index materialization
- `lazyIndexBuilding` option in `IndexedLWWMap`

#### Attribute Factory (Phase 9.02)
- `generateAttributes<V>()` with curried type inference
- `attr()` and `multiAttr()` helper functions
- `createSchema<V>()` fluent builder
- Nested path support (e.g., `'address.city'`)

#### Compound Index Auto-Detection (Phase 9.03)
- `CompoundIndex` class for multi-attribute O(1) lookups
- Compound query tracking in `QueryPatternTracker`
- Compound index suggestions in `IndexAdvisor`
- Automatic `CompoundIndex` usage in `QueryOptimizer`

### Tests
- 140+ new tests for Phase 9 features
- 1264 total tests in core package

## [0.5.0] - 2025-12-30

### Added

#### Query Engine (Phase 7)
- **HashIndex** - O(1) equality lookups
- **NavigableIndex** - O(log N) range queries with SkipList-based SortedMap
- **QueryOptimizer** - Cost-based query planning with automatic index selection
- **StandingQueryIndex** - Pre-computed results for live queries
- **ResultSet Pipeline** - Lazy, composable query results

#### Full-Text Search Foundation (Phase 8.01)
- **InvertedIndex** - Token-based indexing for text search
- New query predicates: `contains`, `containsAll`, `containsAny`
- Configurable tokenization with filters (lowercase, stop words, min length)
- Multi-value attribute support

#### Adaptive Indexing (Phase 8.02)
- **QueryPatternTracker** - Runtime statistics collection
- **IndexAdvisor** - Intelligent index suggestions based on query patterns
- **AutoIndexManager** - Optional automatic index creation
- **DefaultIndexingStrategy** - Auto-index scalar fields

#### New Classes
- `IndexedLWWMap` - LWWMap with index support
- `IndexedORMap` - ORMap with index support

### Performance
- Equality queries: **100-1000× faster** than full scan
- Range queries: **O(log N)** instead of O(N)
- Text search: Sub-millisecond on 100K+ documents

## [0.4.0] - 2025-12-28

### Added

#### New CRDTs and Data Structures
- **PN Counter** (Phase 5.2) - Positive-Negative counter CRDT supporting increment/decrement operations with eventual consistency
- **Event Journal** (Phase 5.04) - Append-only event log for event sourcing patterns with full CRDT synchronization
- **Entry Processor** (Phase 5.03) - Atomic server-side map operations with sandboxed execution via isolated-vm

#### API Enhancements
- **Custom Conflict Resolvers** (Phase 5.05) - Register custom conflict resolution strategies for domain-specific merge logic
- **Delta Updates for Subscriptions** (Phase 5.1) - Incremental updates instead of full data snapshots for improved performance
- **Offline Persistence for PNCounter** (Phase 5.02) - Full offline support for counter operations

#### React Hooks
- `usePNCounter` - Hook for working with PN Counter in React applications
- `useEventJournal` - Hook for subscribing to Event Journal streams
- Enhanced `useQuery` with delta update support

#### Documentation
- PNCounter guide and API reference
- Entry Processor guide and API reference
- Event Journal guide and API reference
- Conflict resolution blog post with ML/AI integration examples

### Fixed
- **react**: Added `maxChanges` option to prevent memory leaks in change tracking
- **server**: Mark isolated-vm as external for bundling
- **server**: Add production warning when isolated-vm is unavailable

### Changed
- **client**: Replace console.log with logger in QueryHandle for better debugging

## [0.3.0] - 2025-12-25

### Added
- Phase 4 completion: Cluster support with split-brain protection
- FailureDetector and FencingManager for cluster resilience
- Cluster Client documentation

### Fixed
- Test alignment with current implementation

## [0.2.1] - 2025-12-24

### Fixed
- Minor bug fixes and stability improvements

## [0.2.0] - 2025-12-23

### Added
- Phase 0 complete: Core CRDT implementation
- LWWMap and ORMap with HLC timestamps
- MerkleTree for efficient delta sync
- Basic client-server synchronization
