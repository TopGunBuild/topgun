# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Versions below 0.12 are pre-release internal milestones — TopGun's public
> history begins at v2.0.0.

## [Unreleased]

- **Changed (on-disk format): the server stores its data in RocksDB.** The data directory now
  holds `engine.rocksdb/`, and the server moves an existing `engine.redb` there by itself, once,
  at the first start of this version. Nothing has to be run by hand, and the redb store is kept.
  - **Upgrade through this release.** The redb store, and both one-time migrations (from
    `engine.redb` and from the table-per-map file of `TOPGUN_REDB_PATH`), are removed in the next
    release. **Upgrading from a version before this release: start this release first, then the
    next one.** A node still on the table-per-map file goes through both migrations in one start.
  - **`STORAGE_BACKEND`** defaults to `rocksdb`. `redb` is deprecated: this release accepts it as
    another name for the embedded store — it starts RocksDB, migrating an existing redb store —
    and logs one `WARN`, `STORAGE_BACKEND=redb is deprecated: …`; the next release refuses it
    with exit 1. Any other value is refused:
    `Unknown STORAGE_BACKEND='…' (expected: rocksdb, null)`.
  - **The one-time migration** runs inside the store open, before the write-ahead log is opened
    and before the server listens. It copies every row byte for byte — records, map registry,
    metadata, keyring and applied watermarks; sealed values stay sealed — verifies the copy,
    renames `engine.redb` to `engine.redb.pre-rocksdb`, and then serves RocksDB; WAL recovery
    then replays every frame above the watermark into it. Log lines (`INFO`):
    `store engine: migrating {D}/engine.redb into {D}/engine.rocksdb`,
    `store engine: migration progress` (at most every 10 s), `store engine: resuming after …` or
    `store engine: resuming at the verification` after an interruption,
    `store engine: migration complete`, and at every later start, while the kept file exists,
    `store engine: the redb store is kept at {D}/engine.redb.pre-rocksdb; delete it once this store is confirmed`.
    The boot summary line reports `store_engine=rocksdb`.
  - **Free space and time.** The migration refuses to start, changing nothing, unless
    `1.5 × the size of engine.redb + 64 MiB` is free on the file system of the data directory.
    Measured in-process on a Mac (APFS, release build, through the store open with the default
    256 MiB engine budget, not through the server binary) on an `engine.redb` of 1 000 000 rows of
    1 KiB values in 100 maps (2.16 GB): 6.56 s, 152 362 rows/s; `engine.rocksdb/` came out at
    0.49 × the redb file (1.05 GB), and the migration used at most 1.45 GB of free space on the
    way. Time grows with the number of rows; Linux is not measured. Measured at `949d8872`, before
    RocksDB was opened with `avoid_flush_during_recovery=false` and a `max_open_files` limit. While the kept file exists
    the node holds both stores.
  - **Resumable.** Every copy batch (at most 10 000 rows or 8 MiB of values) is synced together
    with the mark that says how far the copy got, so a start killed after the first batch
    resumes at the next start and ends with the same content as an uninterrupted one. A start
    killed while the first batch is still being written leaves an `engine.rocksdb/` with no row
    and no mark; the next start removes it with a `WARN` (`store engine: removing …`) and
    migrates `engine.redb` anew.
  - **Refusals.** Two stores in one data directory, a kept file with no store, an
    `engine.rocksdb/` with engine data but no `CURRENT` file, an engine that does not open
    (another process's lock included), too little free space and a failed verification each exit
    1, and none moves, deletes or overwrites `engine.redb` or `engine.redb.pre-rocksdb`, but
    not every refusal changes nothing: an `engine.rocksdb/` holding no row is removed before the
    start looks further; the migration's own refusals come after `engine.redb` was opened for
    writing — its records are unchanged, or sealed when encryption was just turned on, but its
    bytes may change; and a failed verification leaves the filled `engine.rocksdb/`, refused until
    it is moved away. The reason is always on stderr as
    `Error: store open: refusing to start: …`; most refusals also log one `ERROR` line, but some
    — a raw I/O error while checking the location, `store engine: cannot open` or
    `store engine: cannot create` of the migration's target, a failed copy-batch or seal-pass
    write — reach only stderr. Run one server process per data directory: a start that finds an
    `engine.rocksdb/` holding no row removes it without RocksDB's lock. The troubleshooting
    guide lists each refusal and what to do.
  - **Older binaries refuse.** Every data-directory binary built before this version finds no
    `engine.redb` in a migrated data directory and refuses it at its store open
    (`store location: {D} is not a store directory, an empty directory or absent`), exit 1,
    before it opens the WAL and before it listens, changing no file (tested for the build of
    `7692c380`). This also replaces the refusals named in the entries below: a data-directory
    binary from before encryption at rest or before the applied-watermark move now stops at this
    line first. A binary from before the data-directory layout (such as `07d009f0` or `b346e277`)
    reads only `TOPGUN_REDB_PATH` and stops on the marker file there with
    `Error: I/O error: invalid data`, as before.
  - **Rollback** is a return to the kept file: stop this version until its stop exits 0 and logs
    `drained=true`, move `{D}/engine.rocksdb` away, rename `{D}/engine.redb.pre-rocksdb` to
    `{D}/engine.redb`, move the WAL directory away and create an empty one (its new segments are
    unreadable to the older binary), and start the older binary. **Every write acknowledged since
    the migration is then absent on the server**, and so is every write the older binary left in
    its WAL above its watermark before the upgrade — after a `kill -9`, a crash, or a stop that
    logged `drained=false` — because this version replays those into RocksDB, not into the kept
    file. To lose only the writes since the migration, stop the older binary until its stop logs
    `drained=true` before upgrading. There is no tool that exports RocksDB back to redb, and none
    is planned.
  - **Memory: `TOPGUN_ENGINE_CACHE_MB`** (default `256`) is the memory budget of RocksDB — its
    block cache, index and filter blocks and memtables share it. It comes on top of
    `TOPGUN_MAX_RAM_MB`, which bounds only the record cache, and it is approximate. A set-but-empty
    value or one below `64` is fatal at startup; one that does not parse warns and takes the
    default. The boot summary line gains `engine_cache_mb`.
  - **File descriptors.** Before the store opens, the server raises its soft `RLIMIT_NOFILE` to
    the hard limit and lets RocksDB keep half of it open (at least 20, at most 40 960); the boot
    summary line gains `nofile_soft` and `rocksdb_max_open_files`. A raise the system refuses, and
    a soft limit below 1024, each log a `WARN`; the node still starts. Give the service a high
    `LimitNOFILE=`.
  - **New metrics:** `topgun_store_l0_files`, `topgun_store_pending_compaction_bytes`,
    `topgun_store_write_stopped`, `topgun_store_delayed_write_rate`,
    `topgun_store_block_cache_usage_bytes` and `topgun_store_table_readers_mem_bytes`, read every
    5 s and present from the first scrape.
  - **When the store is synced.** Under the write-ahead log a flush pass is committed to RocksDB
    without a sync: the WAL holds every frame of the pass, and the store is synced by each WAL
    collection tick (every 5 s) and by a clean stop, before any WAL frame they cover is
    collected. RocksDB is opened with its recovery flush on (`avoid_flush_during_recovery=false`),
    so rows a killed process left unsynced are durable once the next start has opened the store.
    Under `TOPGUN_WAL_FSYNC_POLICY=none` (tests and benchmarks) a power loss now also loses
    records the store held since the last tick. A tick whose store sync fails collects nothing,
    logs `WAL collection skipped: the store could not be synced` and counts
    `topgun_wal_gc_skipped_total{reason="store_sync_failed"}`; a stop whose store read or sync
    fails writes no rollback hint and logs
    `write-behind shutdown: the WAL's rollback hints could not be written; …` instead of
    `rollback hints written` with `drained=true`.
  - **Exit code 72 on RocksDB.** RocksDB is taken for broken after an I/O error (`cause="io"`) or
    corruption (`cause="corrupted"`); `io_kind` is always `"Other"`, so a full disk is not told
    apart from another I/O error, and the `error` field carries RocksDB's own text. A store that
    fails already at its open exits 1 (`store engine: cannot open …`) at every start until the
    cause is gone. Every other RocksDB error is retried as before.
  - **Throughput above the write buffer's limit.** Measured on a Mac with the in-process load
    harness (open loop, 60 s, three runs per build, default settings; release build of `5de49bfe`,
    which differs from this version only by the `max_open_files` floor and the refusal logging): at 10 000 offered writes/s
    this version acknowledges 5 231–5 234 writes/s against 6 466–6 495 on the redb build before it,
    about 19 % fewer, at a lower durable p99 (16.8–18.9 ms against 61.3–77.1 ms); the rest are
    refused as a temporary error, which clients retry. The cause is the write-behind flush loop,
    which falls into step with the faster store passes (TODO-930), not the store itself. At
    5 000 offered writes/s both builds acknowledge 4 937–4 973 writes/s.
  - **Store size, one workload.** After 60 s at 3 000 writes/s of new keys with empty values
    (180 000 writes), sampled with `du` every 0.5 s around the end of the stop's drain on a Mac:
    24 672–24 680 KiB for `engine.rocksdb/` against 44 156–44 300 KiB allocated for `engine.redb`
    (a 67.9 MB file), three runs each. This is not a general ratio.
  - **Build and binary.** RocksDB is compiled into the server binary (Apache-2.0, see `NOTICE`),
    without any compression library. The macOS arm64 release binary grew from 17 409 504 bytes
    (`7692c380`) to 24 036 000 bytes, about 6.3 MiB (file sizes of local release builds, read
    with `stat`); Linux is not measured here. Building it needs
    `clang` and `libclang`; the container image builds on `rust:1.93.1-bookworm`, the toolchain of
    `rust-toolchain.toml`.

- **Changed (on-disk format): WAL format v3, one log stream per node.** The write-ahead log
  stores the frames of every partition in one stream, `stream-{n:016}.log`: one writer thread
  appends them, and a separate thread fsyncs what was written, so one fsync covers a whole group
  of writes however many partitions it spans, and a write never waits behind an fsync. Every
  segment starts with a 57-byte header (`TGWS`, header version 3), and every frame carries its
  partition and sequence; a sealed frame authenticates both. Segments of the per-partition
  formats (v2 and v1) are still read and leave by ordinary collection; both readers are removed
  in the next release. No earlier binary reads a v3 segment.
  - **The policies keep their meaning.** `batched` (the default) acknowledges a write once its
    frame is written and fsyncs once the oldest unfsynced frame of the node is 10 ms old or 100
    frames are unfsynced — one window per node instead of one per partition. `per_op`
    acknowledges a write only after an fsync that covers it. Measured with the WAL append
    benchmark on a Mac M1 Max (10 concurrent writers over 271 partitions, 100-byte values, three
    10 s runs per policy, release build of `df0f0317`, whose server and `wal_append` sources are
    those of `4820cc02`): `per_op` 1 091–1 210 writes/s against 20 492–22 211 for `batched`. In
    an earlier A/B on the same benchmark group commit raised `per_op` 3.3-fold over the
    per-partition log it replaces (medians 1 231 and 1 356 writes/s against 269 and 286). Linux
    is not measured.
  - **Behaviour change: a failed WAL fsync stops the node, also under `batched`.** It used to log
    `WAL batch fsync failed; acked frames may not be durable` and go on. After a failed fsync the
    operating system may already have dropped the unwritten data and cleared the error, so a
    later fsync that succeeds proves nothing, and going on would keep acknowledging writes whose
    durability is unknown. The node now logs `WAL fail-stop: unrecoverable condition, terminating process`
    and aborts, exit code 134; so does a WAL writer that makes no progress for 30 s while writes
    wait. Check the disk; a restart replays the WAL.
  - **Collection and sealing.** The active segment is sealed once it holds 8 MiB or 30 to 35 s
    after its first frame; a sealed segment is deleted once every partition with a frame in it is
    covered by the watermark the store holds. One write the store refuses for ever therefore
    keeps every segment that holds a later frame of its partition, which under ongoing writes is
    the WAL of the whole node, not only of its partition, until it is stored (TODO-824); watch
    `topgun_wal_retained_bytes_total`.
  - **Metrics.** `topgun_wal_retained_bytes{partition}` is removed; `topgun_wal_retained_bytes_total`
    stays. New: `topgun_wal_fsyncs_total`, `topgun_wal_fsync_seconds`, `topgun_wal_group_frames`,
    `topgun_wal_queue_depth`, `topgun_wal_queue_head_age_seconds` and
    `topgun_wal_unsynced_frames`.

- **Fixed (server): vector search works on the server binary.** `VECTOR_SEARCH` used to get no
  answer from the shipped binary, so `client.vectorSearch` waited for its timeout; the server now
  answers it. Vector indexes, including the `_embedding` indexes of `TOPGUN_VECTOR_MAPS`, are
  rebuilt from the stored records at every start, where they used to be empty after a restart
  until their records were written again; start-up time grows with the stored records of vector
  maps. Vector search no longer returns a record that was deleted or has expired while it was not
  in memory, returns records that are not in memory with their value, and applies its `filter` to
  them. When several records hold the same vector, each of them is found, also after a restart.
- **Fixed (client): vector and hybrid search with a `queryVector` reach the server.** The TS SDK
  sent the query vector as a map instead of bytes, and the server refused the request.
- **Changed (`@topgunbuild/core`, `@topgunbuild/client`): a `Uint8Array` or `Buffer` value is
  stored and synced as binary.** It used to be sent as an object with numeric keys. An
  `ArrayBuffer` value, previously sent as an empty object, is now stored as binary too and reads
  back as a `Uint8Array` (a `Buffer` in Node), not as an `ArrayBuffer`. Other typed arrays (for
  example a `Float32Array`) are unchanged and still sent as an object with numeric keys; a
  `DataView` is unchanged too and still sent as an empty object, so its bytes are lost. Records
  written earlier keep their old shape.
- **Deprecated (server): `dedupEnabled` of vector indexes.** The field of
  `POST /api/admin/indexes/vector` and of stored vector index descriptors is still accepted and has
  no effect: every record is indexed under its own entry, including records that share a vector.
- **Added (server): records written with a TTL expire on the server.** The `ttlMs` of
  `map.set(key, value, ttlMs)` and `orMap.add(key, value, ttlMs)` used to be kept by clients only:
  the server dropped it, kept the record for good and returned it to every query, HTTP `/sync`,
  search and client that loaded the map later. The server now stores the TTL with the record and
  counts it from the moment it stores the write (its own timestamp, never the client's). Once that
  moment plus `ttlMs` has passed, query snapshots, HTTP `/sync`, full-text and hybrid search, and
  vector search leave the record out, and a background sweep then deletes it the ordinary way: live-query
  subscribers get a LEAVE, clients get the usual remove event, and the event journal records a
  DELETE with `metadata.reason = "expired"`. The delete can lag the expiry by up to the sweep
  interval plus one sweep cycle. The TTL is part of the stored value, so it survives the
  write-ahead log, eviction and restarts. A `ttlMs` of `0` is kept as no TTL. Readers whose clocks
  differ can disagree about an expiring record for as long as their clocks differ.
- **Added (server): `TOPGUN_EXPIRY_SWEEP_INTERVAL_MS` (default `10000`) and
  `TOPGUN_EXPIRY_SWEEP_ROWS_PER_SEC` (default `4096`).** The least time between the starts of two
  sweep cycles, and how many stored rows the sweep reads per second. An interval below `1000` or a
  rate of `0` is fatal at startup; a value that does not parse warns and takes the default. The
  boot summary line gains `expiry_sweep_interval_ms` and `expiry_sweep_rows_per_sec`.
- **Added (server): expiry metrics**, present from the first scrape:
  `topgun_expiry_removed_total{kind="lww_record"|"or_tag"}`,
  `topgun_expiry_skipped_total{reason="absent"|"changed"|"not_expired"}`,
  `topgun_expiry_sweep_rows_scanned_total`, `topgun_expiry_sweep_candidates_total{kind="lww"|"or"}`,
  `topgun_expiry_dispatch_overloaded_total`, `topgun_expiry_dispatch_failed_total`,
  `topgun_expiry_sweep_cycles_total`, `topgun_expiry_sweep_last_cycle_seconds`,
  `topgun_expiry_sweep_task_alive`, `topgun_expiry_sweep_pass_panics_total` and
  `topgun_expiry_sweep_maps_skipped_total`.
- **Added (`@topgunbuild/core`, `@topgunbuild/client`): `LWWMap.pruneExpired()`.** It drops every
  expired LWW record from the map and its Merkle tree, writes no tombstone and sends nothing.
  `TopGunClient` runs it once a second while at least one LWW map is open, also offline, and
  before it compares a map's Merkle root with the server, and removes the dropped records from
  local storage; a key with an unsynced local write is kept until the server has it. Without it a
  client would keep the Merkle leaf of a record the server deleted, and its sync would never find
  the map in step again.
- **Changed (server): an OR-Map entry that arrives by `ORMAP_PUSH_DIFF` is stamped at receipt.** A
  tag the server does not hold yet is stored with a timestamp of the server's clock instead of the
  client's, as an `OR_ADD` already was, because the timestamp is now the entry's expiry anchor. A
  tag the server already holds keeps its stored timestamp and TTL. The tag itself is unchanged.
- **Changed (wire):** a record without a TTL is sent without a `ttlMs` key, where the server used
  to send `ttlMs: nil`, and the Merkle sync responses (leaf, bucket and diff, LWW and OR-Map) now
  carry each record's `ttlMs`. `ttlMs` stays optional everywhere; the Merkle hashes do not include
  it.
- **Note (server): records written before this release do not expire.** The server never kept
  their TTL, and nothing migrates them.
- **Removed (server): the store-level expiry configuration.** `StorageConfig::default_ttl_millis`
  and `default_max_idle_millis`, `ExpiryPolicy` and `ExpiryReason`, the `expiry` parameter of
  `RecordStore::put` and `RecordStore::update_in_place`, and `RecordStore::has_expired`,
  `evict_expired` and `is_expirable` are gone. Nothing in the server set them: every value was
  `0` or `ExpiryPolicy::NONE`, so no behaviour changes. A record expires by its own `ttlMs`
  (`TG-TTL-001`). The internal `expiration_time` parameter of the store's write methods and the
  `expiration_time` field of the write-ahead log's store frame are gone too; nothing set them
  either. Frames written by earlier versions that carry the field are still read, and the field is
  ignored.
- **Note (rollback):** the store format is unchanged. A server built before this release starts
  on a store and write-ahead log written by this one: it serves records with a TTL as records
  without one, and drops the TTL of any record, or of every entry of an OR-Map key, that it
  rewrites. A record it did not rewrite still expires once this release runs again.
- **Fixed (client): a key deleted on one client, or removed by the server's expiry sweep, now
  disappears from other online clients.** Until now `map.get` on another connected client kept
  returning the old value and `useMap` kept rendering it until a reload. Now the key is dropped
  from that client's map and from its local storage as soon as the delete arrives, unless that
  client has its own unsynced write of the same key, which is kept and sent. The server is
  unchanged, so older client versions keep the old behaviour until they are upgraded.
- **Fixed (server): a scalar index created on a map that already holds records receives
  every later write and delete of that map.** Writes to the map's existing partitions did not
  reach an index created at runtime: its `entryCount` stayed at the backfill count, and deleted
  keys kept their entries. A restart repaired it.
- **Fixed (server): after a restart, a scalar index restored at start and one created with
  `create_index` cover the stored records, not only the ones in memory.** Start-up time now grows
  with the stored rows of indexed maps.
- **Fixed (server): a scalar index no longer loses the keys of evicted records, and a rewrite
  replaces the key's entry instead of adding a second one.**
- **Fixed (server): HTTP `/sync`, full-text search and hybrid search see stored records after a
  restart or eviction.** They answered only over the records loaded in memory, so after a restart
  `/sync` returned nothing, `SEARCH` and hybrid `fullText` found nothing, hybrid `exact` dropped
  matches (also where a scalar index existed on the predicate attribute) and `includeValue` left
  values out; after eviction they left out the evicted records. Each now reads the stored map,
  merged with the records in memory, without loading the map into memory. A read the store
  refuses is answered with an error (a 500 entry for that `/sync` query, `error` on the search
  response, and no subscription registered), never with a partial result. A key deleted while it
  was not in memory no longer stays in the full-text index. The cost: the first full-text search
  of a map in a process, and the first after writes made while the map had no search
  subscription, reads the whole stored map to build its index, and every `/sync` query and hybrid
  `exact` request reads the whole stored map.
- **Added (server): `TOPGUN_TENANT_CLAIM` (opt-in).** Names the top-level JWT claim that carries
  a caller's tenant, taken verbatim (so a namespaced claim such as `https://example.com/tenant`
  works). When it is set, every client token on the WebSocket handshake and on HTTP `/sync` must
  carry in that claim a tenant of 1 to 128 characters from `A-Z`, `a-z`, `0-9`, `_` and `-` that
  does not start with `_topgun_` or `__topgun_` (in any letter case); any other token is refused as
  a token without `sub` is, before a custom auth validator runs. A set but empty value, a value
  that is not valid Unicode, and the variable together with `TOPGUN_NO_AUTH` are fatal at startup.
  The boot summary line gains `tenant_claim` (the claim name, or `off`). Unset, nothing changes:
  the claim is never read.
- **Added (server): `tenantClaim` in a token-exchange provider's `claims` mapping.** With
  `TOPGUN_TENANT_CLAIM` set, the exchange copies that external claim's string value into the minted
  token under the `TOPGUN_TENANT_CLAIM` name (a missing or non-string value mints no tenant), and
  refuses to mint when `TOPGUN_TENANT_CLAIM` is `sub`, `roles`, `exp` or `iat`. With it set, the
  admin API also refuses (403, naming the claim) any token that carries the tenant claim.
- **Added (server): the tenant boundary.** With `TOPGUN_TENANT_CLAIM` set, a client of tenant `t`
  reaches only maps, topics, counters and locks named `t.…`, before any policy and whatever the
  policies, the admin-subject list or an empty policy store allow; `SqlQuery` is refused, and a
  journal read or subscription without a map returns only the tenant's events. HTTP `/sync`
  queries now also refuse a reserved map (`_topgun_*`, `__topgun_*`) with a 403 entry, with or
  without tenancy, where they returned an empty result.
- **Added (server): tenant deletion, `DELETE /api/admin/tenants/{tenant}`.** With
  `TOPGUN_TENANT_CLAIM` set and RBAC configured, the admin API deletes every map a tenant owns:
  it writes the deny policy `tenant-deleted:{tenant}` (`{tenant}.**`, all actions), drops the maps
  once every write of them is durable, including maps whose first write is still buffered, and
  removes their scalar and vector index descriptors. It refuses, changing nothing, with tenancy off
  (409), without a configured policy store (409) and for an invalid tenant (400); every other
  answer leaves the tenant denied, and re-running finishes what an earlier run left. An accepted
  delete runs to the end even if the client disconnects, and logs `tenant delete finished`.
  Deletion is logical: values stay in sealed WAL segments until collected, in backups and in a
  kept `.pre-migration` file. Re-onboarding a deleted tenant takes deleting its policy and a
  restart. See the "Tenants and tenant deletion" section of the RBAC guide.
- **Changed (server): `GET /api/admin/maps` lists the store's maps.** The listing now comes from
  the store, so a map not touched since the last start is listed too (its `entryCount` is the
  resident count, 0 for a map not loaded), and reserved maps are left out in any letter case. On
  `STORAGE_BACKEND=null` the listing is empty.
- **Changed (server): policy patterns are checked.** `POST /api/admin/policies` refuses (400,
  naming the segment) a `mapPattern` segment that contains `*` and is neither `*` nor `**`, such as
  `users/*` or `a*`, which would match only a name spelled with that star. Stored policies are not
  rewritten.
- **Removed (core-rust): `RequestContext.tenant_id`.** The field was never filled; a caller's
  tenant is carried by `Principal`.
- **Added (server): encryption at rest.** With `TOPGUN_ENCRYPTION_AT_REST=on`,
  `TOPGUN_MASTER_KEY_PROVIDER=file` and `TOPGUN_MASTER_KEY_FILE` (64 hex characters, mode `0600`
  or `0400`), a stolen disk, a copied data directory or WAL directory, or a backup is unreadable
  without the master key: record values and WAL frames are sealed with XChaCha20-Poly1305 under
  data keys, which the data directory keeps only wrapped by the master key. Turning it on, or off,
  is a restart; the start rewrites every stored value in a resumable pass before it accepts a
  connection, and each value grows by 47 bytes. Record keys and map names stay plaintext, and so
  does whatever was written before encryption was turned on, so disk encryption is still
  required. Once a store has had encryption on, every start needs its master key — also after it
  was turned off; without it the node logs `encryption: refusing to start: …` and exits 1 before
  it opens the WAL, writing nothing. The master key is rotated by a start with
  `TOPGUN_MASTER_KEY_PREVIOUS_FILE` set to the old key, which rewrites only the wrapped keys.
  The boot summary line gains `encryption_at_rest` (`off` or `on:file`). The only key provider is
  a local file; AWS KMS and Vault Transit are not available yet. See the "Encryption at rest"
  guide.
- **Changed (on-disk format): WAL format v2.** This release writes v3 stream segments instead
  (see "WAL format v3" above) and still reads v2 ones. A v2 segment starts with a 61-byte header (`TGWS`, header version 2) that names, besides the partition, the
  first sequence, a random salt and the data key id, three format fields: `layout` = 0 (the
  partition function), `payload_schema` = 1 (the WAL entry schema) and `cipher` = 0 (plaintext)
  or 1 (XChaCha20-Poly1305, sealed under the data key the header names). A reader that does not
  know a value refuses the segment by name. The header is written and fsynced before the first
  frame, and a process never appends to a segment it did not create. Segments of the earlier
  format are still read, and leave by ordinary collection; this reader stays for one version.
  - **Older binaries refuse:** every binary built before this version — including the
    `feacb7d4`…`727db547` binaries that read OR delta frames — reads a v2 segment as a frame with
    the wrong magic at offset 0 and refuses to start before it accepts a connection. A store that
    has had encryption on is refused by every older binary at its store open
    (`store format: refusing to open: unknown metadata …`, naming `keyring`), also after
    encryption was turned off. There is no tool that rewrites v2 segments as v1 or takes the
    keyring out of a store, and none is planned.
  - **Rollback:** a stop that logs `drained=true` now unlinks every WAL segment whose frames the
    store holds, so with nothing left above the watermark the WAL directory holds no segment, and
    the documented return to the pre-migration file keeps working. After a crash, a non-zero exit
    or `drained=false`, v2 segments remain and an older binary refuses them: start the new binary
    again and stop it until the line reads `drained=true`. A store that has had encryption on goes
    back to an older binary only by that return to the pre-migration file.
- **Fixed (server):** a store file left by a crash while the embedded store grew it (power loss
  or `kill -9` during a commit) made the server exit with code 101 and only a Rust panic trace at
  every start: the embedded redb store asserts inside its open on a header whose layout is longer
  than the file. The server now refuses such a file like any store that does not open: it exits 1
  with one `ERROR` line, `store open: {file} does not open with this version: redb panicked while
  opening it (…)`, which names the file, says to keep it (its records are probably intact) and to
  restore the data directory and the WAL directory from the same backup to bring the node back.
  The same holds for the old store file a one-time migration reads. A redb panic after the open
  is still not caught.
- **Fixed (server):** a crash while the embedded store grows its file no longer leaves a store file
  that refuses to open. The server now builds against redb 2.6.3 with the upstream fix that makes
  the grown file length durable before the larger layout can reach the store's header (one extra
  file sync per growth; the file doubles while it is small). The file format is unchanged. A file
  left by an earlier version after such a crash is still refused with exit 1 and the
  `store open: … redb panicked while opening it` line above.
- **Security (server):** OR-Map push-diff events were delivered to connections not subscribed to
  the map. They now reach only the connections with a query subscription on that map, other than
  the one that pushed, as events of ordinary writes already did. A client that subscribes to the
  map receives them as before.
- **Security (server):** a lock release, a counter read, and the unsubscribe operations of
  queries, search, hybrid search and the event journal did not check the caller's ownership or
  policy. A lock is now released only by the connection that holds it; a counter read is checked
  as a read of the counter name, as a counter update is checked as a write; an unsubscribe removes
  only the calling connection's subscription, and two connections that choose the same
  subscription id now each keep their own. **Behaviour change:** under policies, reading a counter
  needs a read permission on its name (`client.getPNCounter()` on a counter the policy does not
  let the caller read is refused).
- **Security (server):** vector search, journal reads and subscriptions that name a map, conflict
  resolver registration, removal and listing, and topic subscriptions were not authorized against
  their map or topic. Each is now checked by the same policies as its neighbours: vector search,
  journal operations, resolver listing and topic subscriptions as reads, resolver registration and
  removal as writes; a reserved map name is refused for all of them. **Behaviour change:** a
  journal read or subscription that names no map (`getEventJournal()` / `useEventJournal` without
  a map) still succeeds, but now returns and pushes only the events of maps the caller may read,
  and never events of the server's reserved maps — also on a server without policies. A caller
  allowed to read every map sees the same events as before.
- **Removed (server):** `WalWriter::new`, the WAL constructor that kept the applied watermark in
  files beside the log, is gone. Open the WAL with `WalWriter::open_over_store` over a store that
  keeps applied watermarks, as the server does.
- **Changed (server):** a start after an unclean stop replays the write-ahead log in batches.
  The frames a partition has not yet stored are applied up to 1000 frames or 8 MiB of encoded
  values at a time, each batch one store commit that also records how far the log is stored,
  instead of one commit per frame — so a restart after a full write buffer takes seconds rather
  than the better part of a minute. The result is the same as replaying frame by frame: a batch
  reads the values its own earlier frames wrote. A frame the storage refuses is retried alone and
  does not hold up the other frames, as before; a broken storage still stops the replay of that
  partition and the node exits at its first flush. The start now logs one `WAL recovery complete`
  line with the number of frames and batches and the time it took.
- **Fixed (server):** a WAL segment that the collection could not delete no longer stops the
  collection of the other partitions, and is no longer forgotten until the next restart: it stays
  counted in `topgun_wal_retained_bytes_total` and is deleted by a later collection.
- **Changed (server):** the write buffer is stored in batches. A flush pass now takes up to
  1000 buffered writes or 8 MiB of encoded values, whichever comes first, and stores them,
  together with how far the write-ahead log has been stored, in one store commit instead of
  one store call per write (on RocksDB not synced at once; see "When the store is synced"
  above). While writes are still due, the next pass starts at once instead
  of waiting for `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS`, so a full buffer is emptied in a few
  passes. What changes for whoever runs the server:
  - **One refused record no longer holds up the others.** When the storage refuses a commit of
    several writes, the server splits it in two halves and commits each, again and again, until
    the refused record is alone; every other write of the pass is stored in that pass, at a
    cost of at most 20 extra commits for a pass of 1000. The refused record is retried on its
    own schedule, as before. One place in every pass is kept for the oldest refused write
    that is due, so a refused write is not starved by a stream of new ones.
  - **A new rule for leaving the storage alone.** The server now takes the storage for down
    when both halves of a refused commit are refused — after three calls — instead of after
    three refusals in a row. The hold lengths and the probe are unchanged. The first refused
    commit after the storage came back is split down to single writes and starts no hold, so
    two refused records do not keep the storage on hold. Two records the storage refuses in
    one pass still start one hold even though the storage is up; no such record is known for
    the embedded store.
  - **Shutdown stores the buffer in the same batches.** The drain at a stop runs passes of the
    same limits, each a store commit that the clean stop then syncs, so a clean stop leaves
    nothing to replay; after a failed stop the drained writes' frames stay in the WAL and replay
    at the next start. A refused
    drain commit is split down to single writes for as long as
    `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS` allows; a write it could not store is applied from
    the write-ahead log at the next start, and one refused write no longer makes the drain give
    up on the rest. A commit that has started is not cut, so a stop can overrun the timeout by
    one commit.
  - **`TOPGUN_WRITEBEHIND_BATCH_SIZE` stays without effect.** The pass limits are fixed; the
    setting is still accepted so that a configuration that sets it still starts.
  - **New metrics:** `topgun_write_behind_flush_passes_total` (passes that committed at least
    one write) and `topgun_write_behind_flush_pass_entries_total` (writes those passes
    stored), both from the first scrape, and the histogram
    `topgun_write_behind_flush_pass_seconds` (how long one committed pass took).
    `topgun_write_behind_flush_refusals_total` now counts refused commits, each half of a
    split commit on its own, and `topgun_write_behind_retrying_entries` counts writes refused
    on their own, so it reads 0 when a hold begins.

- **Changed (on-disk format):** each write-ahead-log partition's applied watermark — how far the
  WAL has been stored — lives in the data directory, committed in the same batch as the records
  it covers. The data directory and the WAL directory (`TOPGUN_WAL_DIR`) are one unit from now
  on: back them up and restore them together.
  - **One-time import:** the first start of this version reads the WAL directory's
    `partition-*.applied` files into the data directory once, together with an import marker,
    deletes the files, and writes `wal-collected` (how far the WAL has been collected) into the
    WAL directory. Nothing has to be run by hand.
  - **Older binaries refuse:** once this version has opened a store, the binary built from
    `6a030bb6`, and every data-directory binary before it, exits non-zero at its store open on
    the import marker (`store format: refusing to open: unknown metadata …`), before it opens
    the WAL or listens, and changes no file of the WAL directory. There is no tool to go back,
    and none is planned.
  - **Rollback to the pre-migration file** stays possible, now with a stricter first step: the
    new binary must exit 0 **and** log `write-behind shutdown: rollback hints written` with
    `drained=true`. That stop writes a `partition-{p:03}.applied` rollback hint into the WAL
    directory for each partition that has a segment or a watermark — the watermark the data
    directory holds — for the older binary to read; keep the WAL directory in place. After a crash, a non-zero exit or `drained=false`,
    start the new binary again and stop it until the line reads `drained=true`.
  - **Consistency check at start:** a data directory older than the WAL directory beside it is
    refused before the server listens, with nothing written:
    `WAL directory check: refusing to start: the data directory is older than the WAL directory …`
    ending `Restore the data directory and the WAL directory from the same backup, taken with
    the server stopped`. A WAL directory older than the data directory is accepted: nothing at
    or below the data directory's watermark is replayed and no sequence number is reused.
  - **Collection and sealing:** the WAL is collected on a tick every 5 s, up to the watermark
    the data directory holds, synced first; the active segment is sealed once it holds 8 MiB or
    30 to 35 s after its first frame, not on every write.
  - **Metrics:** `topgun_wal_retained_bytes_total` reports the bytes the WAL keeps, from the
    first scrape; it grows while the store refuses writes.
    `topgun_wal_gc_skipped_total{reason="watermark_read_failed"}` and
    `{reason="store_sync_failed"}` count ticks that could not read the watermark or sync the
    store and collected nothing.

- **Changed (on-disk format):** the server keeps its store in a **data directory** and moves an
  existing store there by itself, once, at the first start of this version. Nothing has to be
  run by hand; the old store file is kept. An older server binary started on the node refuses
  to start instead of serving an empty store.
  - **Where the store lives:** `TOPGUN_DATA_DIR` names the store directory, which holds
    `engine.rocksdb/` (see the RocksDB entry above). Its default is `{directory of TOPGUN_REDB_PATH}/topgun-data` when
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
    refusal at the store open exits 1 with its reason on stderr and creates, moves or deletes
    nothing; most also log one `ERROR` line. The troubleshooting guide lists each refusal and
    what to do.
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

- **Fixed (server):** a server on the embedded store no longer runs on while storing
  nothing after the database reports a fault (described here for redb; for RocksDB, the store
  of this release, see "Exit code 72 on RocksDB" above): it now stops by itself with exit code 72, and
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
