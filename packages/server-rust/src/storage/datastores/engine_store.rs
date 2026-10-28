//! [`EngineMapDataStore`]: a [`MapDataStore`] over any [`KvEngine`].
//!
//! # Key format (`TG-ENG-004`)
//!
//! One engine holds every map. The first byte of a key names its keyspace:
//!
//! | First byte | Keyspace | Key after the byte | Value |
//! |---|---|---|---|
//! | `0x00` | store metadata | `format`, `next_map_id`, `ack_mode`, `migration`, `wal_watermark`, `wal_import_pending`, `keyring`, `seal_pass` | u32 BE `1`; u32 BE; u8 `0` (local); opaque bytes; `[0x01]`; `[0x01]`; [`KeyringRow`]; [`SealPass`] |
//! | `0x01` | map by name | the map name, UTF-8 | map id u32 BE ‖ flags u8 |
//! | `0x02` | map by id | map id u32 BE | the map name, UTF-8 |
//! | `0x10` | primary records | partition u32 BE ‖ map id u32 BE ‖ user key | the record's msgpack, plain or sealed |
//! | `0x11` | backup records | as `0x10` | as `0x10` |
//! | `0x20` | applied WAL watermarks | WAL partition u32 BE ‖ `applied` | sequence u64 BE |
//!
//! Every other first byte is reserved. The partition of a record key is
//! [`hash_to_partition`] of the user key, the function the cluster routes a
//! key by, so a partition's records are one contiguous range per map. The
//! user key is the last field, so no separator or escaping is needed.
//!
//! A map's scan walks the 271 partitions in sequence, so its rows come in
//! `(partition, user key)` order — the byte order of their engine keys — and
//! a cursor is the engine key of the last row ([`PARTITION_THEN_KEY`]).
//!
//! A `0x20` row holds the applied watermark of a **WAL** partition,
//! `fnv1a("{map}:{key}") % 271` — not the record partition above, from which
//! it differs. An absent row means 0 (`TG-ENG-004`).
//!
//! # Registry (`TG-ENG-005`) and metadata (`TG-ENG-006`)
//!
//! A map id is issued in the same engine batch as the map's first record, so
//! a failed first write leaves neither a registry row nor a record. A store
//! opens only at a format, ack mode and keyspace set this binary knows, and
//! with no unfinished migration; anything else is refused at open with
//! nothing written.
//!
//! The import marker `wal_watermark` says the WAL directory's `.applied`
//! sidecars were imported; it is written only by
//! [`MapDataStore::import_applied_watermarks`], in the batch of the imported
//! rows, and a `0x20` row without it is refused at open. The import-pending
//! mark `wal_import_pending` is put in the batch that ends a migration and
//! deleted only in the import batch; the two are never present together.
//! Both hold exactly `[0x01]`; a malformed `0x20` key or value is refused.
//!
//! # Sealed values (`TG-ENC-002`, `TG-ENC-004`)
//!
//! A store opened by [`EngineMapDataStore::open_sealed`] holds the unwrapped
//! keyring of its `keyring` row. While a key is active, every record value
//! is sealed where it enters a batch, in `write_records`; every read of a
//! record value opens it, and a value that does not open is an error, never
//! skipped. Turning encryption on or off writes a `seal_pass` row that
//! [`EngineMapDataStore::run_seal_pass`] carries to its end before the store
//! is served. [`EngineMapDataStore::open`] refuses a store holding either row.
//!
//! # Durability
//!
//! Every [`MapDataStore`] method writes with `sync = true`: the store is
//! write-through, durable on return, as the redb store is. A
//! [`MapDataStore::commit`] is one durable engine batch holding its records
//! and its `0x20` rows, so no row is ever durable without the records it
//! covers; a row never moves below the value it holds.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use async_trait::async_trait;
use parking_lot::{Mutex, MutexGuard, RwLock};
use topgun_core::partition::{hash_to_partition, PARTITION_COUNT};

use super::kv_engine::{KeyRange, KvEngine, KvError, KvSnapshot, WriteBatch, WriteOptions};
use crate::storage::map_data_store::{
    check_map_name, merkle_leaf_hash, AppliedWatermarks, FlushCommit, LeafSink, MapDataStore,
    MerkleLeaf, PutValue, RecordRefused, ScanBatch, ScanCursor, ScanOrder, StoreErrorClass,
    StoreWrite,
};
use crate::storage::record::RecordValue;
use crate::storage::sealing::{
    self, fingerprint_hex, open_value, sealed_key_id, Encryption, EncryptionMode, Keyring,
    KeyringRow, PassDirection, SealPass,
};

/// Store metadata.
pub const KS_META: u8 = 0x00;
/// Map name → `map_id ‖ flags`.
pub const KS_MAP_BY_NAME: u8 = 0x01;
/// Map id → map name.
pub const KS_MAP_BY_ID: u8 = 0x02;
/// Primary records.
pub const KS_PRIMARY: u8 = 0x10;
/// Backup records.
pub const KS_BACKUP: u8 = 0x11;
/// Partition metadata: the applied watermark row of every WAL partition.
pub const KS_PARTITION_META: u8 = 0x20;

/// Metadata name of the format version (u32 BE).
pub const META_FORMAT: &[u8] = b"format";
/// Metadata name of the next map id to issue (u32 BE).
pub const META_NEXT_MAP_ID: &[u8] = b"next_map_id";
/// Metadata name of the ack mode (u8, `0` = local).
pub const META_ACK_MODE: &[u8] = b"ack_mode";
/// Metadata name of an unfinished migration's mark (opaque bytes).
pub const META_MIGRATION: &[u8] = b"migration";
/// Metadata name of the import marker (`[0x01]`): the WAL directory's
/// `.applied` sidecars have been imported into the `0x20` rows.
pub const META_WAL_WATERMARK: &[u8] = b"wal_watermark";
/// Metadata name of the import-pending mark (`[0x01]`): the store was produced
/// by a migration and its WAL sidecars have not been imported yet.
pub const META_WAL_IMPORT_PENDING: &[u8] = b"wal_import_pending";
/// Metadata name of the keyring ([`KeyringRow`]); never deleted once written.
pub const META_KEYRING: &[u8] = b"keyring";
/// Metadata name of an unfinished seal or unseal pass ([`SealPass`]).
pub const META_SEAL_PASS: &[u8] = b"seal_pass";
/// The one format this binary reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Flag bit of a `0x01` row: a primary record of the map was ever stored.
pub const FLAG_PRIMARY: u8 = 0b01;
/// Flag bit of a `0x01` row: a backup record of the map was ever stored.
pub const FLAG_BACKUP: u8 = 0b10;

/// The only ack mode this binary knows: an ack waits for the local engine.
const ACK_MODE_LOCAL: u8 = 0;

/// Prefix of every refusal of `open` / `open_for_migration`.
const REFUSAL: &str = "store format: refusing to open:";

/// Prefix of every refusal of `open_sealed` for want of a key.
const KEY_REFUSAL: &str = "encryption: refusing to start:";

/// Rows of one seal pass batch, as a migration batch.
const SEAL_PASS_BATCH_ROWS: usize = 10_000;
/// Value bytes of one seal pass batch, as a migration batch.
const SEAL_PASS_BATCH_BYTES: usize = 8 * 1024 * 1024;
/// How often a seal pass logs its progress.
const SEAL_PASS_PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// Number of leaves accumulated before invoking the sink once, as the redb store does.
const LEAF_BATCH_SIZE: usize = 1024;

/// Per-batch byte budget of a value scan called with `max_batch_cost == 0`,
/// as the redb store's.
const DEFAULT_SCAN_BATCH_COST: u64 = 8 * 1024 * 1024;

/// The tail of every applied watermark row key.
const APPLIED_TAIL: &[u8] = b"applied";

/// The number of WAL partitions, as an index bound.
const WAL_PARTITIONS: usize = PARTITION_COUNT as usize;

/// The engine key of the applied watermark row of WAL partition `partition`:
/// `KS_PARTITION_META ‖ partition u32 BE ‖ b"applied"` (12 bytes).
#[must_use]
pub fn applied_row_key(partition: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + 4 + APPLIED_TAIL.len());
    key.push(KS_PARTITION_META);
    key.extend_from_slice(&partition.to_be_bytes());
    key.extend_from_slice(APPLIED_TAIL);
    key
}

/// The WAL partition of a well-formed applied watermark row key, else `None`.
fn decode_applied_row_key(key: &[u8]) -> Option<u32> {
    if key.len() != 1 + 4 + APPLIED_TAIL.len()
        || key[0] != KS_PARTITION_META
        || &key[5..] != APPLIED_TAIL
    {
        return None;
    }
    let partition = be_u32(&key[1..5]);
    (partition < PARTITION_COUNT).then_some(partition)
}

/// The engine key of the metadata row `name`: `KS_META ‖ name`.
#[must_use]
pub fn meta_key(name: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + name.len());
    key.push(KS_META);
    key.extend_from_slice(name);
    key
}

/// One of the two record keyspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordSpace {
    /// `KS_PRIMARY`.
    Primary,
    /// `KS_BACKUP`.
    Backup,
}

impl RecordSpace {
    /// The keyspace byte.
    #[must_use]
    pub fn byte(self) -> u8 {
        match self {
            Self::Primary => KS_PRIMARY,
            Self::Backup => KS_BACKUP,
        }
    }

    fn of(backup: bool) -> Self {
        if backup {
            Self::Backup
        } else {
            Self::Primary
        }
    }

    /// The registry flag bit a record of this keyspace sets.
    fn flag(self) -> u8 {
        match self {
            Self::Primary => FLAG_PRIMARY,
            Self::Backup => FLAG_BACKUP,
        }
    }
}

/// `space ‖ partition ‖ map_id`, the 9-byte head of every record key.
fn record_prefix(space: u8, partition: u32, map_id: u32) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(space);
    prefix.extend_from_slice(&partition.to_be_bytes());
    prefix.extend_from_slice(&map_id.to_be_bytes());
    prefix
}

fn be_u32(bytes: &[u8]) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[..4]);
    u32::from_be_bytes(word)
}

/// A decoded record key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKey {
    /// The keyspace.
    pub space: RecordSpace,
    /// `hash_to_partition(key)`.
    pub partition: u32,
    /// The map's registry id.
    pub map_id: u32,
    /// The user key.
    pub key: String,
}

impl RecordKey {
    /// `space ‖ hash_to_partition(key) ‖ map_id ‖ key`.
    #[must_use]
    pub fn encode(space: RecordSpace, map_id: u32, key: &str) -> Vec<u8> {
        let mut bytes = record_prefix(space.byte(), hash_to_partition(key), map_id);
        bytes.extend_from_slice(key.as_bytes());
        bytes
    }

    /// The inverse of [`RecordKey::encode`].
    ///
    /// # Errors
    ///
    /// When the bytes are shorter than 9, the first byte is not a record
    /// keyspace, the user key is not UTF-8, or the partition is not
    /// `hash_to_partition` of the user key — no key this store writes.
    pub fn decode(bytes: &[u8]) -> anyhow::Result<RecordKey> {
        if bytes.len() < 9 {
            bail!("record key is {} bytes, shorter than 9", bytes.len());
        }
        let space = match bytes[0] {
            KS_PRIMARY => RecordSpace::Primary,
            KS_BACKUP => RecordSpace::Backup,
            other => bail!("record key starts with {other:#04x}, not a record keyspace"),
        };
        let partition = be_u32(&bytes[1..5]);
        let map_id = be_u32(&bytes[5..9]);
        let key = std::str::from_utf8(&bytes[9..])
            .map_err(|e| anyhow!("record key is not valid UTF-8: {e}"))?;
        // Every partition is derived from the key; a mismatch cannot come
        // from this store's writes.
        if partition != hash_to_partition(key) {
            bail!("record key carries partition {partition}, not the partition of {key:?}");
        }
        Ok(RecordKey {
            space,
            partition,
            map_id,
            key: key.to_string(),
        })
    }
}

/// A decoded `0x01` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    /// The map name.
    pub name: String,
    /// The map's id.
    pub map_id: u32,
    /// `FLAG_PRIMARY` / `FLAG_BACKUP` bits.
    pub flags: u8,
}

/// The engine key of the `0x01` row of `name`.
#[must_use]
pub fn map_by_name_key(name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + name.len());
    key.push(KS_MAP_BY_NAME);
    key.extend_from_slice(name.as_bytes());
    key
}

/// The value of a `0x01` row.
fn map_by_name_value(map_id: u32, flags: u8) -> Vec<u8> {
    let mut value = Vec::with_capacity(5);
    value.extend_from_slice(&map_id.to_be_bytes());
    value.push(flags);
    value
}

/// Decodes a `0x01` row.
///
/// # Errors
///
/// When the key is not `KS_MAP_BY_NAME ‖ UTF-8`, the value is not 5 bytes,
/// or the id is 0.
pub fn decode_map_by_name_row(key: &[u8], value: &[u8]) -> anyhow::Result<RegistryEntry> {
    let Some((&KS_MAP_BY_NAME, name)) = key.split_first() else {
        bail!("registry key is not in the map-by-name keyspace");
    };
    let name = std::str::from_utf8(name)
        .map_err(|e| anyhow!("registry map name is not valid UTF-8: {e}"))?;
    let (map_id, flags) = decode_map_by_name_value(value)?;
    Ok(RegistryEntry {
        name: name.to_string(),
        map_id,
        flags,
    })
}

fn decode_map_by_name_value(value: &[u8]) -> anyhow::Result<(u32, u8)> {
    if value.len() != 5 {
        bail!("registry row is {} bytes, not 5", value.len());
    }
    let map_id = be_u32(&value[..4]);
    if map_id == 0 {
        bail!("registry row holds map id 0, which is never issued");
    }
    Ok((map_id, value[4]))
}

/// The engine key of the `0x02` row of `map_id`.
#[must_use]
pub fn map_by_id_key(map_id: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(5);
    key.push(KS_MAP_BY_ID);
    key.extend_from_slice(&map_id.to_be_bytes());
    key
}

/// Decodes a `0x02` row into `(map_id, name)`.
///
/// # Errors
///
/// When the key is not 5 bytes starting `KS_MAP_BY_ID`, or the value is not
/// UTF-8.
pub fn decode_map_by_id_row(key: &[u8], value: &[u8]) -> anyhow::Result<(u32, String)> {
    if key.len() != 5 || key[0] != KS_MAP_BY_ID {
        bail!("registry key is not a 5-byte map-by-id key");
    }
    let name = std::str::from_utf8(value)
        .map_err(|e| anyhow!("registry map name is not valid UTF-8: {e}"))?;
    Ok((be_u32(&key[1..5]), name.to_string()))
}

/// The id the `0x01` row of `name` holds in `snapshot`, `None` only when the
/// row is absent. No name check and no cache.
///
/// # Errors
///
/// An engine error is returned, never read as "absent"; a malformed row is
/// an error.
pub fn map_id_in(snapshot: &dyn KvSnapshot, name: &str) -> anyhow::Result<Option<u32>> {
    Ok(registry_row_in(snapshot, name)?.map(|(map_id, _)| map_id))
}

fn registry_row_in(snapshot: &dyn KvSnapshot, name: &str) -> anyhow::Result<Option<(u32, u8)>> {
    snapshot
        .get(&map_by_name_key(name))?
        .map(|value| decode_map_by_name_value(&value))
        .transpose()
}

/// The scan order of [`EngineMapDataStore`]: a key's position is its
/// partition (u32 BE) followed by its UTF-8 bytes, which is the order of the
/// record keys within one keyspace and map.
pub const PARTITION_THEN_KEY: ScanOrder = ScanOrder {
    name: "partition-then-key",
    key_position: partition_then_key_position,
    cursor_position: partition_then_key_cursor_position,
};

fn partition_then_key_position(key: &str) -> Vec<u8> {
    let mut position = Vec::with_capacity(4 + key.len());
    position.extend_from_slice(&hash_to_partition(key).to_be_bytes());
    position.extend_from_slice(key.as_bytes());
    position
}

/// A cursor is a record key; its position drops the keyspace and map id,
/// which the scan it came from fixes.
fn partition_then_key_cursor_position(cursor: &ScanCursor) -> anyhow::Result<Vec<u8>> {
    let bytes = &cursor.0;
    if bytes.len() < 9 || (bytes[0] != KS_PRIMARY && bytes[0] != KS_BACKUP) {
        bail!("scan cursor is not a record key: wrong length or keyspace");
    }
    let key = std::str::from_utf8(&bytes[9..])
        .map_err(|e| anyhow!("scan cursor is not valid UTF-8: {e}"))?;
    if be_u32(&bytes[1..5]) != hash_to_partition(key) {
        bail!("scan cursor is not a record key: its partition is not its key's");
    }
    let mut position = bytes[1..5].to_vec();
    position.extend_from_slice(key.as_bytes());
    Ok(position)
}

/// Refuses exactly the names the shared rule makes a store refuse, with the
/// redb store's text, before anything touches the engine.
fn check_name(map: &str) -> anyhow::Result<()> {
    if let Err(violation) = check_map_name(map) {
        if violation.refused_by_store() {
            bail!("Invalid map name {map:?}: {violation}");
        }
    }
    Ok(())
}

/// The metadata rows of a store this binary formats.
fn format_rows() -> [(Vec<u8>, Vec<u8>); 3] {
    [
        (meta_key(META_FORMAT), FORMAT_VERSION.to_be_bytes().to_vec()),
        (meta_key(META_NEXT_MAP_ID), 1u32.to_be_bytes().to_vec()),
        (meta_key(META_ACK_MODE), vec![ACK_MODE_LOCAL]),
    ]
}

/// The first key of `range` in `snapshot`, if any.
fn first_key_in(snapshot: &dyn KvSnapshot, range: KeyRange) -> anyhow::Result<Option<Vec<u8>>> {
    match snapshot.iter(range)?.next() {
        Some(entry) => Ok(Some(entry?.0)),
        None => Ok(None),
    }
}

/// What the metadata of a non-empty engine says, once every check passed.
struct StoreMetadata {
    migration: Option<Vec<u8>>,
    /// The import marker is present.
    imported: bool,
    /// The import-pending mark is present.
    import_pending: bool,
    /// The applied watermark of every WAL partition; an absent row is 0.
    applied: [u64; WAL_PARTITIONS],
    /// The keyring row, checked; a seal pass row is checked against it.
    keyring: Option<KeyringRow>,
}

/// Refuses a one-byte mark whose value is not `[0x01]`.
fn check_flag_mark(name: &str, value: Option<&[u8]>) -> anyhow::Result<bool> {
    match value {
        None => Ok(false),
        Some([1]) => Ok(true),
        Some(v) => bail!("{REFUSAL} the {name} mark holds {v:?}, not [1]"),
    }
}

/// The checked keyring row, after checking the seal pass row against it
/// (`TG-ENC-004`).
fn check_sealing_rows(
    keyring: Option<&[u8]>,
    seal_pass: Option<&[u8]>,
) -> anyhow::Result<Option<KeyringRow>> {
    let keyring = keyring
        .map(KeyringRow::decode)
        .transpose()
        .map_err(|e| anyhow!("{REFUSAL} {e}"))?;
    let seal_pass = seal_pass
        .map(SealPass::decode)
        .transpose()
        .map_err(|e| anyhow!("{REFUSAL} {e}"))?;
    if let Some(pass) = &seal_pass {
        // Enable and disable write the key states and the pass in one batch,
        // so none of these is a state this store produces.
        let active = match &keyring {
            None => bail!("{REFUSAL} a seal pass without a keyring"),
            Some(row) => row.active_id().is_some(),
        };
        match pass.direction {
            PassDirection::Seal if !active => bail!("{REFUSAL} a seal pass with no active key"),
            PassDirection::Unseal if active => bail!("{REFUSAL} an unseal pass with an active key"),
            _ => {}
        }
        if pass
            .cursor
            .first()
            .is_some_and(|&b| b != KS_PRIMARY && b != KS_BACKUP)
        {
            bail!("{REFUSAL} a seal pass cursor outside the record keyspaces");
        }
    }
    Ok(keyring)
}

/// The applied watermark rows, checked (`TG-ENG-006`); an absent row is 0.
fn read_applied_rows(
    snapshot: &dyn KvSnapshot,
    imported: bool,
) -> anyhow::Result<[u64; WAL_PARTITIONS]> {
    let rows = KeyRange {
        start: Some(vec![KS_PARTITION_META]),
        end: Some(vec![KS_PARTITION_META + 1]),
    };
    let mut applied = [0u64; WAL_PARTITIONS];
    let mut any_row = false;
    for entry in snapshot.iter(rows)? {
        let (key, value) = entry?;
        let Some(partition) = decode_applied_row_key(&key) else {
            bail!("{REFUSAL} a key in the reserved keyspace {KS_PARTITION_META:#04x}");
        };
        let Ok(word) = <[u8; 8]>::try_from(value.as_slice()) else {
            bail!(
                "{REFUSAL} the applied watermark row of WAL partition {partition} is {} bytes, \
                 not 8",
                value.len()
            );
        };
        applied[partition as usize] = u64::from_be_bytes(word);
        any_row = true;
    }
    // A row is only ever written with the marker, so a row without it is
    // not a state this store produces.
    if any_row && !imported {
        bail!("{REFUSAL} applied watermark rows without the import marker");
    }
    Ok(applied)
}

/// `None` for an engine that holds no key; otherwise the checked metadata,
/// or a refusal (`TG-ENG-006`). Reads only.
fn read_store_metadata(snapshot: &dyn KvSnapshot) -> anyhow::Result<Option<StoreMetadata>> {
    if first_key_in(snapshot, KeyRange::default())?.is_none() {
        return Ok(None);
    }
    let mut format = None;
    let mut next_map_id = None;
    let mut ack_mode = None;
    let mut migration = None;
    let mut marker = None;
    let mut pending = None;
    let mut keyring = None;
    let mut seal_pass = None;
    let meta = KeyRange {
        start: Some(vec![KS_META]),
        end: Some(vec![KS_MAP_BY_NAME]),
    };
    for entry in snapshot.iter(meta)? {
        let (key, value) = entry?;
        match &key[1..] {
            META_FORMAT => format = Some(value),
            META_NEXT_MAP_ID => next_map_id = Some(value),
            META_ACK_MODE => ack_mode = Some(value),
            META_MIGRATION => migration = Some(value),
            META_WAL_WATERMARK => marker = Some(value),
            META_WAL_IMPORT_PENDING => pending = Some(value),
            META_KEYRING => keyring = Some(value),
            META_SEAL_PASS => seal_pass = Some(value),
            other => bail!(
                "{REFUSAL} unknown metadata {:?}",
                String::from_utf8_lossy(other)
            ),
        }
    }
    match format.as_deref() {
        None => bail!("{REFUSAL} no format metadata; not a store of this binary"),
        Some(v) if v.len() == 4 && be_u32(v) == FORMAT_VERSION => {}
        Some(v) if v.len() == 4 => bail!("{REFUSAL} format {} is not {FORMAT_VERSION}", be_u32(v)),
        Some(v) => bail!("{REFUSAL} format metadata is {} bytes, not 4", v.len()),
    }
    match ack_mode.as_deref() {
        None => bail!("{REFUSAL} no ack mode metadata"),
        Some([ACK_MODE_LOCAL]) => {}
        Some(v) => bail!("{REFUSAL} ack mode {v:?} is not local"),
    }
    match next_map_id.as_deref() {
        None => bail!("{REFUSAL} no next map id metadata"),
        Some(v) if v.len() == 4 && be_u32(v) >= 1 => {}
        Some(v) => bail!("{REFUSAL} next map id {v:?} is not a u32 of at least 1"),
    }
    let imported = check_flag_mark("import", marker.as_deref())?;
    let import_pending = check_flag_mark("import-pending", pending.as_deref())?;
    if imported && import_pending {
        bail!("{REFUSAL} an import-pending mark beside the import marker");
    }
    let keyring = check_sealing_rows(keyring.as_deref(), seal_pass.as_deref())?;
    let reserved = [
        KeyRange {
            start: Some(vec![KS_MAP_BY_ID + 1]),
            end: Some(vec![KS_PRIMARY]),
        },
        KeyRange {
            start: Some(vec![KS_BACKUP + 1]),
            end: Some(vec![KS_PARTITION_META]),
        },
        KeyRange {
            start: Some(vec![KS_PARTITION_META + 1]),
            end: None,
        },
    ];
    for range in reserved {
        if let Some(key) = first_key_in(snapshot, range)? {
            bail!("{REFUSAL} a key in the reserved keyspace {:#04x}", key[0]);
        }
    }
    let applied = read_applied_rows(snapshot, imported)?;
    Ok(Some(StoreMetadata {
        migration,
        imported,
        import_pending,
        applied,
        keyring,
    }))
}

/// One operation of [`EngineMapDataStore::write_records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOp<'a> {
    /// Stores `bytes` as the record `(map, backup, key)`.
    Put {
        /// The map.
        map: &'a str,
        /// The backup keyspace when `true`.
        backup: bool,
        /// The user key.
        key: &'a str,
        /// The record's msgpack, stored as given.
        bytes: &'a [u8],
    },
    /// Removes the record `(map, backup, key)`; never issues an id or sets a flag.
    Delete {
        /// The map.
        map: &'a str,
        /// The backup keyspace when `true`.
        backup: bool,
        /// The user key.
        key: &'a str,
    },
    /// Registers the map and sets the flag of `backup` without storing a record.
    Touch {
        /// The map.
        map: &'a str,
        /// The backup keyspace when `true`.
        backup: bool,
    },
    /// `Some` sets the migration mark, `None` removes it.
    SetMigrationMark(Option<&'a [u8]>),
    /// Stores `sequence` as the applied watermark row of WAL partition
    /// `partition`.
    SetAppliedWatermark {
        /// The WAL partition.
        partition: u32,
        /// The applied watermark.
        sequence: u64,
    },
    /// Puts the import marker and removes the import-pending mark.
    MarkWatermarksImported,
}

impl RecordOp<'_> {
    /// The map the op names and the flag bits it needs on that map's row:
    /// a `Delete` needs none, so it never issues an id or sets a flag.
    fn map_need(&self) -> Option<(&str, u8)> {
        match *self {
            Self::Put { map, backup, .. } | Self::Touch { map, backup } => {
                Some((map, RecordSpace::of(backup).flag()))
            }
            Self::Delete { map, .. } => Some((map, 0)),
            // Metadata ops name no map. Listed one by one so a new variant
            // must decide whether it names a map.
            Self::SetMigrationMark(_)
            | Self::SetAppliedWatermark { .. }
            | Self::MarkWatermarksImported => None,
        }
    }
}

/// The registry side of one `write_records` call: every named map's row as
/// it will stand after the batch, the registry rows to put into the batch,
/// and the registry lock when an id is issued or a flag set.
struct RegistryPlan<'s, 'n> {
    rows: HashMap<&'n str, Option<(u32, u8)>>,
    puts: Vec<(Vec<u8>, Vec<u8>)>,
    guard: Option<MutexGuard<'s, ()>>,
}

/// A [`MapDataStore`] over a [`KvEngine`], in the key format of this module.
pub struct EngineMapDataStore<E: KvEngine> {
    engine: Arc<E>,
    /// Held by a write that issues an id or sets a flag, from its registry
    /// re-read to the return of its engine write.
    registry_lock: Mutex<()>,
    /// `name → (map_id, flags)`, filled only from rows that exist.
    cache: RwLock<HashMap<String, (u32, u8)>>,
    /// The durable `0x20` row of every WAL partition, loaded at open and
    /// updated only after the engine write that stored it returned. Held
    /// across a commit's check, write and update, so no row moves below the
    /// value it holds, whoever commits.
    applied: Mutex<[u64; WAL_PARTITIONS]>,
    /// The effective unwrapped keyring: `None` for a store that never had
    /// encryption on. Values are sealed while it has an active key.
    keyring: Option<Arc<Keyring>>,
    #[cfg(test)]
    registry_park: Mutex<Option<RegistryPark>>,
}

/// A first write of a map could not be given an id: every id is issued.
#[derive(Debug)]
struct MapIdSpaceExhausted {
    /// The maps left without an id.
    maps: Vec<String>,
}

impl std::fmt::Display for MapIdSpaceExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "map id space exhausted: no id for {:?}", self.maps)
    }
}

impl std::error::Error for MapIdSpaceExhausted {}

/// A one-shot blocking park inside `write_records`.
#[cfg(test)]
struct RegistryPark {
    reached: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl<E: KvEngine> EngineMapDataStore<E> {
    /// Opens the store over `engine`, formatting an empty engine.
    ///
    /// # Errors
    ///
    /// A refusal starting `store format: refusing to open:` when the engine
    /// holds metadata this binary does not know, a key in a reserved
    /// keyspace, an unfinished migration (`TG-ENG-006`), or a keyring, which
    /// only [`Self::open_sealed`] opens (`TG-ENC-006`); an engine error.
    pub fn open(engine: Arc<E>) -> anyhow::Result<Self> {
        let metadata = read_store_metadata(&*engine.snapshot()?)?;
        let applied = match metadata {
            Some(StoreMetadata {
                keyring: Some(_), ..
            }) => {
                bail!("{REFUSAL} the store is sealed; it opens only with its master key");
            }
            None => {
                let mut batch = WriteBatch::new();
                for (key, value) in format_rows() {
                    batch.put(key, value);
                }
                engine.write(batch, WriteOptions { sync: true })?;
                [0; WAL_PARTITIONS]
            }
            Some(StoreMetadata {
                migration: Some(_), ..
            }) => {
                bail!("{REFUSAL} a migration into this store is unfinished");
            }
            Some(StoreMetadata { applied, .. }) => applied,
        };
        Ok(Self::over(engine, &applied, None))
    }

    /// Opens the store over `engine` with the keys of `encryption`,
    /// formatting an empty engine (`TG-ENC-004`, `TG-ENC-006`,
    /// `TG-ENC-007`): the metadata checks of [`Self::open`]; every key of the
    /// keyring unwrapped, re-wrapped first when the keyring is under
    /// `encryption.previous` (a master-key rotation); then `On` with no active
    /// key adds one and starts a seal pass, `Off` with an active key makes
    /// every key decrypt-only and starts an unseal pass. Whatever changes is
    /// one durable batch. No pass runs here: [`Self::run_seal_pass`] runs it.
    ///
    /// # Errors
    ///
    /// The refusals of [`Self::open`] except the keyring's; a refusal
    /// starting `encryption: refusing to start:` when a key cannot be
    /// unwrapped, with nothing written; an engine error.
    pub fn open_sealed(engine: Arc<E>, encryption: &Encryption) -> anyhow::Result<Self> {
        let metadata = read_store_metadata(&*engine.snapshot()?)?;
        let mut batch = WriteBatch::new();
        let (applied, row) = match metadata {
            None => {
                for (key, value) in format_rows() {
                    batch.put(key, value);
                }
                ([0; WAL_PARTITIONS], None)
            }
            Some(StoreMetadata {
                migration: Some(_), ..
            }) => {
                bail!("{REFUSAL} a migration into this store is unfinished");
            }
            Some(StoreMetadata {
                applied, keyring, ..
            }) => (applied, keyring),
        };
        let (mut keyring, mut row_changed) = match row {
            Some(row) => {
                let (keyring, rotated) = unwrap_keyring(row, encryption)?;
                (Some(keyring), rotated)
            }
            None => (None, false),
        };
        let active = keyring.as_ref().and_then(Keyring::active_id);
        let pass = match (encryption.mode, active) {
            (EncryptionMode::On, None) => {
                let Some(provider) = encryption.provider.as_deref() else {
                    return Err(key_refusal(
                        "encryption is on, but no master-key provider is configured",
                    ));
                };
                let base = match keyring.take() {
                    Some(keyring) => keyring,
                    None => Keyring::empty(provider),
                };
                let enabled = base.with_new_active(provider)?;
                tracing::info!(
                    key_id = enabled.active_id(),
                    provider = provider.name(),
                    "encryption: enabled; new values are sealed and a seal pass seals the stored ones"
                );
                keyring = Some(enabled);
                Some(PassDirection::Seal)
            }
            (EncryptionMode::Off, Some(_)) => {
                keyring = keyring.map(Keyring::disabled);
                tracing::info!(
                    "encryption: disabled; new values are stored plain and an unseal pass unseals the stored ones"
                );
                Some(PassDirection::Unseal)
            }
            _ => None,
        };
        if let Some(direction) = pass {
            row_changed = true;
            batch.put(
                meta_key(META_SEAL_PASS),
                SealPass {
                    direction,
                    cursor: Vec::new(),
                }
                .encode(),
            );
        }
        if row_changed {
            if let Some(keyring) = &keyring {
                batch.put(meta_key(META_KEYRING), keyring.row().encode()?);
            }
        }
        if !batch.is_empty() {
            engine.write(batch, WriteOptions { sync: true })?;
        }
        Ok(Self::over(engine, &applied, keyring.map(Arc::new)))
    }

    /// Runs a present `seal_pass` to its end (`TG-ENC-004`): walks `0x10` then
    /// `0x11` from the cursor and rewrites every value not in the pass's
    /// target form, in batches that each carry the new cursor, then deletes
    /// the row. A store with no pass returns at once.
    ///
    /// # Errors
    ///
    /// An engine error, or a value that does not open; the durable cursor
    /// is where the next run resumes.
    pub fn run_seal_pass(&self) -> anyhow::Result<()> {
        self.seal_pass_with(SEAL_PASS_BATCH_ROWS, SEAL_PASS_BATCH_BYTES, &mut || {})
    }

    /// [`Self::run_seal_pass`] with its batch limits, calling `after_write`
    /// after every engine write of the pass.
    fn seal_pass_with(
        &self,
        max_rows: usize,
        max_bytes: usize,
        after_write: &mut dyn FnMut(),
    ) -> anyhow::Result<()> {
        let Some(pass) = self
            .engine
            .get(&meta_key(META_SEAL_PASS))?
            .map(|v| SealPass::decode(&v).map_err(|e| anyhow!("{REFUSAL} {e}")))
            .transpose()?
        else {
            return Ok(());
        };
        let word = pass.direction.word();
        let keyring = self.keyring.as_deref();
        let target = match pass.direction {
            PassDirection::Seal => Some(
                keyring
                    .and_then(Keyring::active_id)
                    .ok_or_else(|| anyhow!("a seal pass with no active key"))?,
            ),
            PassDirection::Unseal => None,
        };
        tracing::info!("encryption: {word} pass started");
        let started = std::time::Instant::now();
        let mut logged = started;
        let mut rows: u64 = 0;
        let mut cursor = pass.cursor;
        loop {
            let snapshot = self.engine.snapshot()?;
            let start = if cursor.is_empty() {
                vec![KS_PRIMARY]
            } else {
                let mut start = cursor.clone();
                start.push(0x00);
                start
            };
            let range = KeyRange {
                start: Some(start),
                end: Some(vec![KS_BACKUP + 1]),
            };
            let mut batch = WriteBatch::new();
            let (mut walked, mut bytes, mut last) = (0usize, 0usize, None);
            let mut more = false;
            for entry in snapshot.iter(range)? {
                let (key, value) = entry?;
                if walked >= max_rows || (walked > 0 && bytes + value.len() > max_bytes) {
                    more = true;
                    break;
                }
                walked += 1;
                bytes += value.len();
                let rewrite = match target {
                    Some(active) => sealed_key_id(&value) != Some(active),
                    None => value.first() == Some(&sealing::SEALED_MARK),
                };
                if rewrite {
                    let plain = open_value(keyring, &key, &value)?;
                    let stored = match keyring.filter(|_| target.is_some()) {
                        Some(keyring) => keyring.seal_value(&key, &plain)?,
                        None => plain.into_owned(),
                    };
                    batch.put(key.clone(), stored);
                    rows += 1;
                }
                last = Some(key);
            }
            drop(snapshot);
            if let Some(last) = last {
                // The cursor rides in the batch of the rows it covers, so
                // no crash leaves it ahead of them (TG-ENC-004).
                batch.put(
                    meta_key(META_SEAL_PASS),
                    SealPass {
                        direction: pass.direction,
                        cursor: last.clone(),
                    }
                    .encode(),
                );
                self.engine.write(batch, WriteOptions { sync: true })?;
                after_write();
                cursor = last;
            }
            if !more {
                break;
            }
            if logged.elapsed() >= SEAL_PASS_PROGRESS_EVERY {
                logged = std::time::Instant::now();
                tracing::info!(rows, "encryption: {word} pass progress");
            }
        }
        let mut done = WriteBatch::new();
        done.delete(meta_key(META_SEAL_PASS));
        self.engine.write(done, WriteOptions { sync: true })?;
        after_write();
        tracing::info!(
            rows,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "encryption: {word} pass complete"
        );
        Ok(())
    }

    /// The plaintext of the record value `value` stored at `engine_key`.
    fn open_record<'v>(&self, engine_key: &[u8], value: &'v [u8]) -> anyhow::Result<Cow<'v, [u8]>> {
        Ok(open_value(self.keyring.as_deref(), engine_key, value)?)
    }

    fn over(
        engine: Arc<E>,
        applied: &[u64; WAL_PARTITIONS],
        keyring: Option<Arc<Keyring>>,
    ) -> Self {
        Self {
            engine,
            registry_lock: Mutex::new(()),
            cache: RwLock::new(HashMap::new()),
            applied: Mutex::new(*applied),
            keyring,
            #[cfg(test)]
            registry_park: Mutex::new(None),
        }
    }

    /// Opens a store whose migration is unfinished, or formats an empty
    /// engine with `initial_mark` in the same batch; returns the mark.
    ///
    /// # Errors
    ///
    /// As [`EngineMapDataStore::open`], except that a migration mark is
    /// required.
    pub fn open_for_migration(
        engine: Arc<E>,
        initial_mark: &[u8],
    ) -> anyhow::Result<(Self, Vec<u8>)> {
        let metadata = read_store_metadata(&*engine.snapshot()?)?;
        let (mark, applied) = match metadata {
            None => {
                // One batch, so no crash point leaves a formatted store
                // without its mark.
                let mut batch = WriteBatch::new();
                for (key, value) in format_rows() {
                    batch.put(key, value);
                }
                batch.put(meta_key(META_MIGRATION), initial_mark.to_vec());
                engine.write(batch, WriteOptions { sync: true })?;
                (initial_mark.to_vec(), [0; WAL_PARTITIONS])
            }
            Some(StoreMetadata {
                migration: Some(mark),
                applied,
                ..
            }) => (mark, applied),
            Some(StoreMetadata {
                migration: None, ..
            }) => bail!(
                "store format: refusing to resume a migration: the store has no migration mark"
            ),
        };
        Ok((Self::over(engine, &applied, None), mark))
    }

    /// The migration mark of `engine`, read from one snapshot; writes nothing.
    ///
    /// # Errors
    ///
    /// An engine error, never read as "absent".
    pub fn read_migration_mark(engine: &E) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(engine.snapshot()?.get(&meta_key(META_MIGRATION))?)
    }

    /// The registry row of `name`: from the cache, else from the engine,
    /// caching only a row that exists.
    fn cached_row(&self, name: &str) -> anyhow::Result<Option<(u32, u8)>> {
        if let Some(row) = self.cache.read().get(name) {
            return Ok(Some(*row));
        }
        let row = self
            .engine
            .get(&map_by_name_key(name))?
            .map(|value| decode_map_by_name_value(&value))
            .transpose()?;
        if let Some(row) = row {
            self.cache.write().insert(name.to_string(), row);
        }
        Ok(row)
    }

    /// The id of `name` for a read from `snapshot`: from the cache, else
    /// from the snapshot, caching only a row that exists.
    fn read_map_id(&self, snapshot: &dyn KvSnapshot, name: &str) -> anyhow::Result<Option<u32>> {
        if let Some((map_id, _)) = self.cache.read().get(name) {
            return Ok(Some(*map_id));
        }
        let row = registry_row_in(snapshot, name)?;
        if let Some(row) = row {
            self.cache.write().insert(name.to_string(), row);
        }
        Ok(row.map(|(map_id, _)| map_id))
    }

    /// Plans the registry rows a call needs (`TG-ENG-005`). A call whose
    /// every needed flag is already known takes no lock; any other re-reads
    /// every row under `registry_lock`, which the plan holds until the
    /// caller's engine write has returned, so two first writes cannot read
    /// the same `next_map_id`.
    fn plan_registry<'n>(
        &self,
        needs: &BTreeMap<&'n str, u8>,
    ) -> anyhow::Result<RegistryPlan<'_, 'n>> {
        let mut rows = HashMap::with_capacity(needs.len());
        let mut settled = true;
        for (&name, &need) in needs {
            let row = self.cached_row(name)?;
            settled &= match row {
                Some((_, flags)) => flags & need == need,
                None => need == 0,
            };
            rows.insert(name, row);
        }
        if settled {
            return Ok(RegistryPlan {
                rows,
                puts: Vec::new(),
                guard: None,
            });
        }

        let guard = self.registry_lock.lock();
        let mut puts = Vec::new();
        let mut next_map_id: Option<u32> = None;
        let mut exhausted = Vec::new();
        for (&name, &need) in needs {
            let current = self
                .engine
                .get(&map_by_name_key(name))?
                .map(|value| decode_map_by_name_value(&value))
                .transpose()?;
            let planned = match current {
                Some((map_id, flags)) => {
                    if flags | need != flags {
                        puts.push((
                            map_by_name_key(name),
                            map_by_name_value(map_id, flags | need),
                        ));
                    }
                    Some((map_id, flags | need))
                }
                None if need == 0 => None,
                None => {
                    let map_id = match next_map_id {
                        Some(id) => id,
                        None => self.stored_next_map_id()?,
                    };
                    // `u32::MAX` is never issued, so `map_id + 1` — the end
                    // of a map's partition range — never overflows. Every
                    // map left without an id is named, so a commit can
                    // refuse exactly its writes.
                    if map_id == u32::MAX {
                        exhausted.push(name.to_string());
                        next_map_id = Some(map_id);
                        continue;
                    }
                    puts.push((map_by_name_key(name), map_by_name_value(map_id, need)));
                    puts.push((map_by_id_key(map_id), name.as_bytes().to_vec()));
                    next_map_id = Some(map_id + 1);
                    Some((map_id, need))
                }
            };
            rows.insert(name, planned);
        }
        if !exhausted.is_empty() {
            return Err(MapIdSpaceExhausted { maps: exhausted }.into());
        }
        if let Some(next) = next_map_id {
            puts.push((meta_key(META_NEXT_MAP_ID), next.to_be_bytes().to_vec()));
            self.registry_park_point();
        }
        Ok(RegistryPlan {
            rows,
            puts,
            guard: Some(guard),
        })
    }

    fn stored_next_map_id(&self) -> anyhow::Result<u32> {
        match self.engine.get(&meta_key(META_NEXT_MAP_ID))? {
            Some(value) if value.len() == 4 => Ok(be_u32(&value)),
            _ => bail!("next map id metadata is missing or malformed"),
        }
    }

    /// Applies `ops`, and every registry row they need, as one engine batch.
    ///
    /// # Errors
    ///
    /// A refused map name (nothing is written); `map id space exhausted`;
    /// an engine error, with nothing of the batch applied.
    pub fn write_records(&self, ops: &[RecordOp<'_>], options: WriteOptions) -> anyhow::Result<()> {
        let mut needs: BTreeMap<&str, u8> = BTreeMap::new();
        for op in ops {
            if let Some((map, need)) = op.map_need() {
                check_name(map)?;
                *needs.entry(map).or_insert(0) |= need;
            }
        }
        let plan = self.plan_registry(&needs)?;

        let mut batch = WriteBatch::new();
        for (key, value) in &plan.puts {
            batch.put(key.clone(), value.clone());
        }
        for op in ops {
            match *op {
                RecordOp::Put {
                    map,
                    backup,
                    key,
                    bytes,
                } => {
                    let (map_id, _) =
                        plan.rows.get(map).copied().flatten().ok_or_else(|| {
                            anyhow!("map {map:?} has no registry row after its plan")
                        })?;
                    let engine_key = RecordKey::encode(RecordSpace::of(backup), map_id, key);
                    // The one place a record value enters a batch, so the
                    // one place it is sealed (TG-ENC-002).
                    let value = match self.keyring.as_deref() {
                        Some(keyring) if keyring.active_id().is_some() => {
                            keyring.seal_value(&engine_key, bytes)?
                        }
                        _ => bytes.to_vec(),
                    };
                    batch.put(engine_key, value);
                }
                RecordOp::Delete { map, backup, key } => {
                    // A map without a registry row holds no record to delete.
                    if let Some((map_id, _)) = plan.rows.get(map).copied().flatten() {
                        batch.delete(RecordKey::encode(RecordSpace::of(backup), map_id, key));
                    }
                }
                RecordOp::Touch { .. } => {}
                RecordOp::SetMigrationMark(Some(mark)) => {
                    batch.put(meta_key(META_MIGRATION), mark.to_vec());
                }
                RecordOp::SetMigrationMark(None) => {
                    // The batch that ends a migration also records that the
                    // WAL directory's sidecars still have to be imported, so
                    // a crash before the WAL open cannot lose that.
                    batch.delete(meta_key(META_MIGRATION));
                    batch.put(meta_key(META_WAL_IMPORT_PENDING), vec![1]);
                }
                RecordOp::SetAppliedWatermark {
                    partition,
                    sequence,
                } => batch.put(applied_row_key(partition), sequence.to_be_bytes().to_vec()),
                RecordOp::MarkWatermarksImported => {
                    batch.put(meta_key(META_WAL_WATERMARK), vec![1]);
                    batch.delete(meta_key(META_WAL_IMPORT_PENDING));
                }
            }
        }
        if !batch.is_empty() {
            self.engine.write(batch, options)?;
        }
        // Only after the batch is applied: a failed write must leave no id
        // in the cache that the engine does not hold.
        if plan.guard.is_some() {
            let mut cache = self.cache.write();
            for (name, row) in &plan.rows {
                if let Some(row) = row {
                    cache.insert((*name).to_string(), *row);
                }
            }
        }
        Ok(())
    }

    /// Visits every record of `(map, backup)` in [`PARTITION_THEN_KEY`]
    /// order, from one snapshot, as `(key, raw value bytes)`.
    ///
    /// # Errors
    ///
    /// A refused map name, an engine error, or the first error of `visit`,
    /// which stops the walk.
    pub fn visit_records(
        &self,
        map: &str,
        backup: bool,
        visit: &mut dyn FnMut(&str, &[u8]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        check_name(map)?;
        let space = RecordSpace::of(backup);
        let snapshot = self.engine.snapshot()?;
        let Some(map_id) = self.read_map_id(&*snapshot, map)? else {
            return Ok(());
        };
        for partition in 0..PARTITION_COUNT {
            for entry in snapshot.iter(partition_range(space, partition, map_id, None))? {
                let (key, value) = entry?;
                let value = self.open_record(&key, &value)?;
                visit(&RecordKey::decode(&key)?.key, &value)?;
            }
        }
        Ok(())
    }

    /// Makes every returned write durable.
    ///
    /// # Errors
    ///
    /// The engine's error.
    pub fn sync(&self) -> anyhow::Result<()> {
        self.engine.sync()?;
        Ok(())
    }

    /// One batch of a value scan of `(map, backup)`, from one snapshot,
    /// resumed strictly after `cursor` when given. Partitions are walked in
    /// sequence; the redb store's budget rules apply unchanged.
    fn scan_from(
        &self,
        map: &str,
        backup: bool,
        cursor: Option<&ScanCursor>,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        check_name(map)?;
        let budget = if max_batch_cost == 0 {
            DEFAULT_SCAN_BATCH_COST
        } else {
            max_batch_cost
        };
        let space = RecordSpace::of(backup);
        let snapshot = self.engine.snapshot()?;
        let Some(map_id) = self.read_map_id(&*snapshot, map)? else {
            return Ok(ScanBatch::default());
        };
        let (first_partition, mut resume_after) = match cursor {
            Some(cursor) => (
                resume_partition(cursor, space, map_id)?,
                Some(cursor.0.as_slice()),
            ),
            None => (0, None),
        };

        let mut records: Vec<(String, RecordValue)> = Vec::new();
        let mut last_engine_key: Option<Vec<u8>> = None;
        let mut accumulated: u64 = 0;
        let mut more_rows_pending = false;
        'walk: for partition in first_partition..PARTITION_COUNT {
            let range = partition_range(space, partition, map_id, resume_after.take());
            for entry in snapshot.iter(range)? {
                let (key, bytes) = entry?;
                let row_cost = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                // Check before push: a batch never holds the row that would
                // cross the budget, except its first row, admitted whole.
                if !records.is_empty() && accumulated.saturating_add(row_cost) > budget {
                    more_rows_pending = true;
                    break 'walk;
                }
                accumulated += row_cost;
                let value: RecordValue = rmp_serde::from_slice(&self.open_record(&key, &bytes)?)?;
                records.push((RecordKey::decode(&key)?.key, value));
                last_engine_key = Some(key);
            }
        }
        let next_cursor = if more_rows_pending {
            last_engine_key.map(ScanCursor)
        } else {
            None
        };
        Ok(ScanBatch {
            records,
            next_cursor,
        })
    }

    /// Writes one op for `(map, key)` and nothing else.
    fn write_one(&self, op: RecordOp<'_>) -> anyhow::Result<()> {
        self.write_records(&[op], WriteOptions { sync: true })
    }

    #[cfg(test)]
    fn registry_park_point(&self) {
        let park = self.registry_park.lock().take();
        if let Some(park) = park {
            let _ = park.reached.send(());
            let _ = park.release.recv();
        }
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)]
    fn registry_park_point(&self) {}

    /// Arms a one-shot park in the next `write_records` call that issues a
    /// map id: between its in-lock registry re-read and its engine write it
    /// sends on the returned receiver's channel, then waits for a message on
    /// the returned sender or for the sender to be dropped.
    #[cfg(test)]
    pub(crate) fn arm_registry_park(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (reached, reached_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *self.registry_park.lock() = Some(RegistryPark {
            reached,
            release: release_rx,
        });
        (reached_rx, release)
    }

    /// The number of registry cache entries.
    #[cfg(test)]
    pub(crate) fn registry_cache_len(&self) -> usize {
        self.cache.read().len()
    }
}

/// The engine range of one partition of a map: `[ks ‖ p ‖ id, ks ‖ p ‖ (id + 1))`,
/// starting strictly after `after` when given.
fn partition_range(
    space: RecordSpace,
    partition: u32,
    map_id: u32,
    after: Option<&[u8]>,
) -> KeyRange {
    let start = match after {
        Some(after) => {
            let mut start = Vec::with_capacity(after.len() + 1);
            start.extend_from_slice(after);
            start.push(0x00);
            start
        }
        None => record_prefix(space.byte(), partition, map_id),
    };
    KeyRange {
        start: Some(start),
        end: Some(record_prefix(space.byte(), partition, map_id + 1)),
    }
}

/// The partition a scan resumes in, after checking that `cursor` is a record
/// key of this scan's keyspace and map.
fn resume_partition(cursor: &ScanCursor, space: RecordSpace, map_id: u32) -> anyhow::Result<u32> {
    let bytes = &cursor.0;
    if bytes.len() < 9 || bytes[0] != space.byte() || be_u32(&bytes[5..9]) != map_id {
        bail!("scan cursor is not a record key of this map");
    }
    let key = std::str::from_utf8(&bytes[9..])
        .map_err(|e| anyhow!("scan cursor is not valid UTF-8: {e}"))?;
    let partition = be_u32(&bytes[1..5]);
    if partition != hash_to_partition(key) {
        bail!("scan cursor is not a record key of this map");
    }
    Ok(partition)
}

/// Logs and returns a refusal of `open_sealed` for want of a key
/// (`TG-ENC-006`).
fn key_refusal(reason: &str) -> anyhow::Error {
    let line = format!("{KEY_REFUSAL} {reason}");
    tracing::error!("{line}");
    anyhow!(line)
}

/// Every key of `row` unwrapped under `encryption.provider`, or under
/// `encryption.previous` and then re-wrapped under the provider (a master-key
/// rotation, `TG-ENC-007`); `true` when it was re-wrapped. Writes nothing.
fn unwrap_keyring(row: KeyringRow, encryption: &Encryption) -> anyhow::Result<(Keyring, bool)> {
    let expected = fingerprint_hex(&row.kek_fingerprint);
    let first_id = row.keys.first().map_or(0, |k| k.id);
    let Some(provider) = encryption.provider.as_deref() else {
        return Err(key_refusal(&format!(
            "the store is sealed; set TOPGUN_MASTER_KEY_PROVIDER to the key that sealed it (fingerprint {expected})"
        )));
    };
    let name = provider.name();
    let unwrap_with = |row: KeyringRow, kek: &dyn sealing::MasterKeyProvider, which: &str| {
        Keyring::unwrap(row, kek).map_err(|e| {
            key_refusal(&format!(
                "provider {name}: key {}: the wrap does not authenticate under the {which} master key (fingerprint {expected})",
                e.key_id
            ))
        })
    };
    if row.kek_fingerprint == provider.fingerprint() {
        return Ok((unwrap_with(row, provider, "configured")?, false));
    }
    match encryption.previous.as_deref() {
        Some(previous) if row.kek_fingerprint == previous.fingerprint() => {
            let keyring = unwrap_with(row, previous, "previous")?.rewrapped(provider)?;
            tracing::info!(
                from = %expected,
                to = %fingerprint_hex(&provider.fingerprint()),
                "encryption: master key rotated; the keyring is re-wrapped under the new key"
            );
            Ok((keyring, true))
        }
        _ => Err(key_refusal(&format!(
            "provider {name}: key {first_id}: the keyring is wrapped by the master key with fingerprint {expected}, but the configured key has fingerprint {}; set TOPGUN_MASTER_KEY_FILE to that key, or set TOPGUN_MASTER_KEY_PREVIOUS_FILE to it to rotate to the configured key",
            fingerprint_hex(&provider.fingerprint())
        ))),
    }
}

#[async_trait]
impl<E: KvEngine> MapDataStore for EngineMapDataStore<E> {
    fn classify_error(&self, err: &anyhow::Error) -> StoreErrorClass {
        // By type only: the first engine error in the chain decides.
        let engine_error = err
            .downcast_ref::<KvError>()
            .or_else(|| err.chain().find_map(|e| e.downcast_ref::<KvError>()));
        match engine_error {
            Some(KvError::Broken { cause, .. }) => StoreErrorClass::StoreBroken(*cause),
            _ => StoreErrorClass::Unknown,
        }
    }

    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        // The one msgpack encoding of a stored record, so `add` and
        // `add_encoded` store the same bytes.
        let bytes = rmp_serde::to_vec_named(value)?;
        self.write_one(RecordOp::Put {
            map,
            backup: false,
            key,
            bytes: &bytes,
        })
    }

    fn accepts_encoded(&self) -> bool {
        true
    }

    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        self.write_one(RecordOp::Put {
            map,
            backup: false,
            key,
            bytes,
        })
    }

    fn scan_order(&self) -> Option<ScanOrder> {
        // One snapshot per scan call, partitions walked in sequence and a
        // resume strictly after the cursor's engine key (TG-ENG-004).
        Some(PARTITION_THEN_KEY)
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        let bytes = rmp_serde::to_vec_named(value)?;
        self.write_one(RecordOp::Put {
            map,
            backup: true,
            key,
            bytes: &bytes,
        })
    }

    async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        self.write_one(RecordOp::Delete {
            map,
            backup: false,
            key,
        })
    }

    async fn remove_backup(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        self.write_one(RecordOp::Delete {
            map,
            backup: true,
            key,
        })
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        check_name(map)?;
        let snapshot = self.engine.snapshot()?;
        let Some(map_id) = self.read_map_id(&*snapshot, map)? else {
            return Ok(None);
        };
        let engine_key = RecordKey::encode(RecordSpace::Primary, map_id, key);
        match snapshot.get(&engine_key)? {
            Some(bytes) => Ok(Some(rmp_serde::from_slice(
                &self.open_record(&engine_key, &bytes)?,
            )?)),
            None => Ok(None),
        }
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        check_name(map)?;
        let snapshot = self.engine.snapshot()?;
        let Some(map_id) = self.read_map_id(&*snapshot, map)? else {
            return Ok(Vec::new());
        };
        let mut found = Vec::with_capacity(keys.len());
        for key in keys {
            let engine_key = RecordKey::encode(RecordSpace::Primary, map_id, key);
            if let Some(bytes) = snapshot.get(&engine_key)? {
                found.push((
                    key.clone(),
                    rmp_serde::from_slice(&self.open_record(&engine_key, &bytes)?)?,
                ));
            }
        }
        Ok(found)
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        check_name(map)?;
        let space = RecordSpace::of(is_backup);
        let snapshot = self.engine.snapshot()?;
        let Some(map_id) = self.read_map_id(&*snapshot, map)? else {
            return Ok(());
        };
        // Fixed-size batches, so peak memory stays bounded whatever the map's size.
        let mut batch: Vec<MerkleLeaf> = Vec::with_capacity(LEAF_BATCH_SIZE);
        for partition in 0..PARTITION_COUNT {
            for entry in snapshot.iter(partition_range(space, partition, map_id, None))? {
                let (engine_key, bytes) = entry?;
                let value: RecordValue =
                    rmp_serde::from_slice(&self.open_record(&engine_key, &bytes)?)?;
                let key = RecordKey::decode(&engine_key)?.key;
                if let Some((kind, leaf_hash)) = merkle_leaf_hash(&key, &value) {
                    batch.push(MerkleLeaf {
                        key,
                        kind,
                        leaf_hash,
                    });
                }
                if batch.len() >= LEAF_BATCH_SIZE {
                    sink.consume(std::mem::take(&mut batch)).await?;
                    batch = Vec::with_capacity(LEAF_BATCH_SIZE);
                }
            }
        }
        if !batch.is_empty() {
            sink.consume(batch).await?;
        }
        Ok(())
    }

    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        self.scan_from(map, is_backup, None, max_batch_cost)
    }

    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        self.scan_from(map, is_backup, Some(&cursor), max_batch_cost)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        // Before the empty-keys return: a refused name is refused whatever
        // the key list holds.
        check_name(map)?;
        if keys.is_empty() {
            return Ok(());
        }
        let ops: Vec<RecordOp<'_>> = keys
            .iter()
            .map(|key| RecordOp::Delete {
                map,
                backup: false,
                key,
            })
            .collect();
        self.write_records(&ops, WriteOptions { sync: true })
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        // A map is listed from its first primary write on and never
        // dropped, as a redb primary table is (TG-ENG-005).
        let snapshot = self.engine.snapshot()?;
        let range = KeyRange {
            start: Some(vec![KS_MAP_BY_NAME]),
            end: Some(vec![KS_MAP_BY_ID]),
        };
        let mut names = Vec::new();
        for entry in snapshot.iter(range)? {
            let (key, value) = entry?;
            let row = decode_map_by_name_row(&key, &value)?;
            if row.flags & FLAG_PRIMARY != 0 {
                names.push(row.name);
            }
        }
        // Rows come in byte order of the UTF-8 names, which is `String` order.
        Ok(names)
    }

    fn is_loadable(&self, _key: &str) -> bool {
        // Write-through: every write is durable before it returns.
        true
    }

    fn pending_operation_count(&self) -> u64 {
        0
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn commit(&self, commit: &FlushCommit<'_>) -> anyhow::Result<()> {
        // Refused writes are named before anything is written, so the caller
        // can tell them from a store that refuses everything.
        let mut refused = Vec::new();
        let mut reasons = Vec::new();
        for (index, write) in commit.writes.iter().enumerate() {
            if let Err(reason) = check_name(write_map(write)) {
                refused.push(index);
                reasons.push(reason.to_string());
            }
        }
        if !refused.is_empty() {
            return Err(
                anyhow::Error::new(RecordRefused { indices: refused }).context(reasons.join("; "))
            );
        }

        // The bytes `add` stores for a value, encoded before the lock.
        let encoded = commit
            .writes
            .iter()
            .map(|write| match write {
                StoreWrite::Put {
                    value: PutValue::Value(value),
                    ..
                } => rmp_serde::to_vec_named(*value).map(Cow::Owned),
                StoreWrite::Put {
                    value: PutValue::Encoded(bytes),
                    ..
                } => Ok(Cow::Borrowed(bytes.as_slice())),
                StoreWrite::Remove { .. } => Ok(Cow::Borrowed(&[][..])),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut ops = Vec::with_capacity(commit.writes.len() + commit.applied.len());
        for (write, encoded) in commit.writes.iter().zip(&encoded) {
            ops.push(match write {
                StoreWrite::Put { map, key, .. } => RecordOp::Put {
                    map,
                    backup: false,
                    key,
                    bytes: encoded,
                },
                StoreWrite::Remove { map, key, .. } => RecordOp::Delete {
                    map,
                    backup: false,
                    key,
                },
            });
        }

        // Check, write and update under one lock and with no `.await`, so
        // no other commit can store a row between this check and this write.
        let mut held = self.applied.lock();
        let mut next = *held;
        for &(partition, sequence) in &commit.applied {
            let Some(slot) = next.get_mut(partition as usize) else {
                bail!(
                    "applied watermark of WAL partition {partition} refused: there are \
                     {PARTITION_COUNT} partitions"
                );
            };
            // Equal is a re-commit of the same frontier; lower would let a
            // later restart replay frames the store already holds.
            if sequence < *slot {
                let refusal = format!(
                    "applied watermark regression refused: WAL partition {partition} holds \
                     {slot}, the commit carries {sequence}"
                );
                tracing::error!("{refusal}");
                bail!(refusal);
            }
            *slot = sequence;
            ops.push(RecordOp::SetAppliedWatermark {
                partition,
                sequence,
            });
        }
        match self.write_records(&ops, WriteOptions { sync: true }) {
            Ok(()) => {
                *held = next;
                Ok(())
            }
            Err(err) => match err.downcast_ref::<MapIdSpaceExhausted>() {
                Some(exhausted) => {
                    let indices = commit
                        .writes
                        .iter()
                        .enumerate()
                        .filter(|(_, write)| {
                            matches!(write, StoreWrite::Put { map, .. }
                                if exhausted.maps.iter().any(|m| m == map))
                        })
                        .map(|(index, _)| index)
                        .collect();
                    let reason = err.to_string();
                    Err(anyhow::Error::new(RecordRefused { indices }).context(reason))
                }
                // Returned as it is, so `classify_error` still finds the
                // engine's error in the chain (TG-WB-010).
                None => Err(err),
            },
        }
    }

    fn keeps_applied_watermarks(&self) -> bool {
        true
    }

    fn wal_keyring(&self) -> Option<Arc<Keyring>> {
        self.keyring.clone()
    }

    fn applied_watermarks(&self) -> anyhow::Result<Option<AppliedWatermarks>> {
        // One snapshot, so the marks and the rows are of one moment.
        let held = read_store_metadata(&*self.engine.snapshot()?)?;
        Ok(Some(match held {
            Some(held) => AppliedWatermarks {
                imported: held.imported,
                import_pending: held.import_pending,
                by_partition: held.applied.to_vec(),
            },
            None => AppliedWatermarks {
                imported: false,
                import_pending: false,
                by_partition: vec![0; WAL_PARTITIONS],
            },
        }))
    }

    fn import_applied_watermarks(&self, rows: &[(u32, u64)]) -> anyhow::Result<()> {
        // Under the commit lock, so no commit can store a row between the
        // marker check and the import.
        let mut held = self.applied.lock();
        if self.engine.get(&meta_key(META_WAL_WATERMARK))?.is_some() {
            bail!("applied watermarks refused: the store has imported its WAL sidecars already");
        }
        let mut next = *held;
        let mut ops = Vec::with_capacity(rows.len() + 1);
        for &(partition, sequence) in rows {
            let Some(slot) = next.get_mut(partition as usize) else {
                bail!(
                    "applied watermark of WAL partition {partition} refused: there are \
                     {PARTITION_COUNT} partitions"
                );
            };
            // An absent row already means 0.
            if sequence > 0 {
                *slot = sequence;
                ops.push(RecordOp::SetAppliedWatermark {
                    partition,
                    sequence,
                });
            }
        }
        // The only construction of the marker op: the marker is written in
        // the batch of the rows it vouches for and nowhere else (TG-ENG-006).
        ops.push(RecordOp::MarkWatermarksImported);
        self.write_records(&ops, WriteOptions { sync: true })?;
        *held = next;
        Ok(())
    }
}

/// The map a commit write names.
fn write_map<'a>(write: &StoreWrite<'a>) -> &'a str {
    match write {
        StoreWrite::Put { map, .. } | StoreWrite::Remove { map, .. } => map,
    }
}

#[cfg(all(test, feature = "redb"))]
mod sealing_tests {
    //! Sealed values in the engine store: a stolen file, a transplanted
    //! value, a pass killed and resumed, an open without keys, and a
    //! master-key rotation that rewrites only the keyring.

    use std::collections::BTreeMap;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::super::kv_crash::{read_all, CrashMode, CrashTarget, MemTarget, RedbTarget};
    use super::super::kv_engine::MemKvEngine;
    use super::super::redb_engine::RedbKvEngine;
    use super::*;
    use crate::storage::sealing::{
        generate_key, FileKeyProvider, MasterKeyProvider, ValueOpenError, SEALED_MARK,
    };

    type Rows = BTreeMap<Vec<u8>, Vec<u8>>;

    fn kek() -> Arc<dyn MasterKeyProvider> {
        Arc::new(FileKeyProvider::from_kek(generate_key().expect("random")))
    }

    fn with(mode: EncryptionMode, provider: &Arc<dyn MasterKeyProvider>) -> Encryption {
        Encryption {
            mode,
            provider: Some(Arc::clone(provider)),
            previous: None,
        }
    }

    fn on(provider: &Arc<dyn MasterKeyProvider>) -> Encryption {
        with(EncryptionMode::On, provider)
    }

    fn off(provider: &Arc<dyn MasterKeyProvider>) -> Encryption {
        with(EncryptionMode::Off, provider)
    }

    fn lww(value: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(value.to_string()),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n".to_string(),
            },
        }
    }

    fn put(
        store: &EngineMapDataStore<impl KvEngine>,
        map: &str,
        backup: bool,
        key: &str,
        value: &str,
    ) {
        let bytes = rmp_serde::to_vec_named(&lww(value)).expect("encode");
        store
            .write_records(
                &[RecordOp::Put {
                    map,
                    backup,
                    key,
                    bytes: &bytes,
                }],
                WriteOptions { sync: true },
            )
            .expect("put");
    }

    fn rows(engine: &impl KvEngine) -> Rows {
        read_all(engine).expect("read all")
    }

    fn record_rows(engine: &impl KvEngine) -> Rows {
        rows(engine)
            .into_iter()
            .filter(|(k, _)| k[0] == KS_PRIMARY || k[0] == KS_BACKUP)
            .collect()
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    fn is_auth_failure(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<ValueOpenError>(),
            Some(ValueOpenError::AuthenticationFailed { .. })
        )
    }

    /// TG-ENC-002, TG-ENC-006: values written under `on` are not in the
    /// file; a wrong master key is refused with nothing changed; the right
    /// one reads every value back.
    #[test]
    fn a_stolen_engine_file_holds_no_marker_and_opens_only_with_its_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("engine.redb");
        let right = kek();
        let markers: Vec<String> = (0..200)
            .map(|i| format!("MARKER-PLAINTEXT-{i:04}"))
            .collect();
        {
            let engine = Arc::new(RedbKvEngine::open(&path).expect("open"));
            let store = EngineMapDataStore::open_sealed(engine, &on(&right)).expect("enable");
            store.run_seal_pass().expect("pass");
            for (i, marker) in markers.iter().enumerate() {
                put(&store, "m", i % 2 == 1, &format!("k{i}"), marker);
            }
        }
        let file = std::fs::read(&path).expect("read file");
        assert!(
            !file
                .windows(b"MARKER-PLAINTEXT".len())
                .any(|w| w == b"MARKER-PLAINTEXT"),
            "a marker is in the engine file"
        );
        let engine = Arc::new(RedbKvEngine::open(&path).expect("reopen"));
        check_wrong_key_then_right_key(&engine, &right, &markers);
    }

    /// The same over the reference engine: the refusal and the read-back.
    #[test]
    fn a_wrong_master_key_is_refused_over_the_reference_engine_too() {
        let engine = Arc::new(MemKvEngine::new());
        let right = kek();
        let markers: Vec<String> = (0..20).map(|i| format!("marker-{i}")).collect();
        {
            let store =
                EngineMapDataStore::open_sealed(Arc::clone(&engine), &on(&right)).expect("enable");
            for (i, marker) in markers.iter().enumerate() {
                put(&store, "m", i % 2 == 1, &format!("k{i}"), marker);
            }
        }
        check_wrong_key_then_right_key(&engine, &right, &markers);
    }

    fn check_wrong_key_then_right_key<E: KvEngine>(
        engine: &Arc<E>,
        right: &Arc<dyn MasterKeyProvider>,
        markers: &[String],
    ) {
        let before = rows(&**engine);
        let wrong = kek();
        for encryption in [on(&wrong), off(&wrong), Encryption::off()] {
            let error = EngineMapDataStore::open_sealed(Arc::clone(engine), &encryption)
                .err()
                .expect("a wrong key is refused");
            assert!(
                error
                    .to_string()
                    .starts_with("encryption: refusing to start:"),
                "{error}"
            );
            assert!(
                error
                    .to_string()
                    .contains(&fingerprint_hex(&right.fingerprint())),
                "the refusal names the fingerprint the keyring expects: {error}"
            );
            assert_eq!(rows(&**engine), before, "a refusal wrote something");
        }
        let store = EngineMapDataStore::open_sealed(Arc::clone(engine), &on(right)).expect("right");
        for (i, marker) in markers.iter().enumerate() {
            let backup = i % 2 == 1;
            let mut found = Vec::new();
            store
                .visit_records("m", backup, &mut |key, bytes| {
                    if key == format!("k{i}") {
                        found.push(rmp_serde::from_slice::<RecordValue>(bytes)?);
                    }
                    Ok(())
                })
                .expect("visit");
            assert_eq!(found, vec![lww(marker)]);
        }
        assert_eq!(
            rows(&**engine),
            before,
            "an open with the right key changed a row"
        );
    }

    struct Sink;

    #[async_trait]
    impl LeafSink for Sink {
        async fn consume(&mut self, _batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// TG-ENC-002: a sealed value copied to another key, the same key
    /// of another map, or the backup space fails authentication; scans fail
    /// rather than skip.
    fn transplant<E: KvEngine>(engine: &Arc<E>) {
        let provider = kek();
        let store =
            EngineMapDataStore::open_sealed(Arc::clone(engine), &on(&provider)).expect("on");
        put(&store, "a", false, "k1", "secret");
        put(&store, "b", false, "other", "x");
        let id = |map: &str| store.cached_row(map).expect("row").expect("registered").0;
        let source = RecordKey::encode(RecordSpace::Primary, id("a"), "k1");
        let sealed = engine.get(&source).expect("get").expect("row");
        assert_eq!(sealed[0], SEALED_MARK);
        let targets = [
            RecordKey::encode(RecordSpace::Primary, id("a"), "k2"),
            RecordKey::encode(RecordSpace::Primary, id("b"), "k1"),
            RecordKey::encode(RecordSpace::Backup, id("a"), "k1"),
        ];
        let mut batch = WriteBatch::new();
        for target in &targets {
            batch.put(target.clone(), sealed.clone());
        }
        engine
            .write(batch, WriteOptions { sync: true })
            .expect("plant");

        block_on(async {
            assert!(is_auth_failure(
                &store.load("a", "k2").await.expect_err("same map")
            ));
            assert!(is_auth_failure(
                &store.load("b", "k1").await.expect_err("other map")
            ));
            assert!(is_auth_failure(
                &store
                    .scan_values("a", true, 0)
                    .await
                    .expect_err("backup scan")
            ));
            assert!(is_auth_failure(
                &store
                    .load_all("b", &["k1".to_string()])
                    .await
                    .expect_err("load all")
            ));
            assert!(is_auth_failure(
                &store
                    .enumerate_leaves("a", false, &mut Sink)
                    .await
                    .expect_err("leaves")
            ));
            assert!(is_auth_failure(
                &store.scan_values("a", false, 0).await.expect_err("scan")
            ));
            // The source still opens.
            assert_eq!(
                store.load("a", "k1").await.expect("source"),
                Some(lww("secret"))
            );
        });
        for backup in [false, true] {
            let error = store
                .visit_records("a", backup, &mut |_, _| Ok(()))
                .expect_err("visit");
            assert!(is_auth_failure(&error), "{error}");
        }
    }

    #[test]
    fn a_transplanted_sealed_value_fails_authentication_over_the_reference_engine() {
        transplant(&Arc::new(MemKvEngine::new()));
    }

    #[test]
    fn a_transplanted_sealed_value_fails_authentication_over_redb() {
        let dir = tempfile::tempdir().expect("tempdir");
        transplant(&Arc::new(
            RedbKvEngine::open(&dir.path().join("engine.redb")).expect("open"),
        ));
    }

    /// TG-ENC-006: the plain open refuses a store with a keyring and
    /// writes nothing.
    #[test]
    fn the_plain_open_refuses_a_sealed_store_and_writes_nothing() {
        let engine = Arc::new(MemKvEngine::new());
        let provider = kek();
        let store =
            EngineMapDataStore::open_sealed(Arc::clone(&engine), &on(&provider)).expect("on");
        put(&store, "m", false, "k", "v");
        drop(store);
        let before = rows(&*engine);
        let error = EngineMapDataStore::open(Arc::clone(&engine))
            .err()
            .expect("refused");
        assert!(error.to_string().contains("sealed"), "{error}");
        assert_eq!(rows(&*engine), before);
    }

    /// TG-ENC-004: a keyring or seal pass row this store does not produce is
    /// refused at open with nothing written.
    #[test]
    fn a_planted_keyring_or_seal_pass_row_is_refused_at_open() {
        let provider = kek();
        let keyring = Keyring::empty(&*provider)
            .with_new_active(&*provider)
            .expect("key");
        let active_row = keyring.row().encode().expect("encode");
        let disabled_row = keyring.disabled().row().encode().expect("encode");
        let pass = |direction, cursor: &[u8]| {
            SealPass {
                direction,
                cursor: cursor.to_vec(),
            }
            .encode()
        };
        let cases: Vec<Vec<(&[u8], Vec<u8>)>> = vec![
            vec![(META_SEAL_PASS, pass(PassDirection::Seal, &[]))],
            vec![
                (META_KEYRING, active_row.clone()),
                (META_SEAL_PASS, vec![0x03]),
            ],
            vec![
                (META_KEYRING, disabled_row.clone()),
                (META_SEAL_PASS, pass(PassDirection::Seal, &[])),
            ],
            vec![
                (META_KEYRING, active_row.clone()),
                (META_SEAL_PASS, pass(PassDirection::Unseal, &[])),
            ],
            vec![
                (META_KEYRING, active_row.clone()),
                (META_SEAL_PASS, pass(PassDirection::Seal, &[0x20])),
            ],
            vec![(META_KEYRING, b"\x01".to_vec())],
        ];
        for planted in cases {
            let engine = Arc::new(MemKvEngine::new());
            drop(EngineMapDataStore::open(Arc::clone(&engine)).expect("format"));
            let mut batch = WriteBatch::new();
            for (name, value) in &planted {
                batch.put(meta_key(name), value.clone());
            }
            engine
                .write(batch, WriteOptions { sync: true })
                .expect("plant");
            let before = rows(&*engine);
            let error = EngineMapDataStore::open_sealed(Arc::clone(&engine), &on(&provider))
                .err()
                .expect("refused");
            assert!(error.to_string().starts_with(REFUSAL), "{error}");
            assert_eq!(rows(&*engine), before);
        }
    }

    /// TG-ENC-007: a rotation rewrites the keyring row and no other.
    #[test]
    fn a_master_key_rotation_rewrites_only_the_keyring_row() {
        rotate(&Arc::new(MemKvEngine::new()));
        let dir = tempfile::tempdir().expect("tempdir");
        rotate(&Arc::new(
            RedbKvEngine::open(&dir.path().join("engine.redb")).expect("open"),
        ));
    }

    fn rotate<E: KvEngine>(engine: &Arc<E>) {
        let old = kek();
        let new = kek();
        {
            let store = EngineMapDataStore::open_sealed(Arc::clone(engine), &on(&old)).expect("on");
            store.run_seal_pass().expect("pass");
            for i in 0..50 {
                put(&store, "m", i % 3 == 0, &format!("k{i}"), &format!("v{i}"));
            }
        }
        let before = rows(&**engine);
        let rotation = Encryption {
            mode: EncryptionMode::On,
            provider: Some(Arc::clone(&new)),
            previous: Some(Arc::clone(&old)),
        };
        let store = EngineMapDataStore::open_sealed(Arc::clone(engine), &rotation).expect("rotate");
        store.run_seal_pass().expect("no pass");
        drop(store);
        let after = rows(&**engine);
        let keyring_key = meta_key(META_KEYRING);
        assert_eq!(before.len(), after.len());
        for (key, value) in &before {
            if *key == keyring_key {
                assert_ne!(after.get(key), Some(value), "the keyring was not rewritten");
            } else {
                assert_eq!(
                    after.get(key),
                    Some(value),
                    "a row other than the keyring changed"
                );
            }
        }
        let row = KeyringRow::decode(&after[&keyring_key]).expect("keyring");
        assert_eq!(row.kek_fingerprint, new.fingerprint());
        // The old key alone is refused now; the new one opens every value.
        assert!(EngineMapDataStore::open_sealed(Arc::clone(engine), &on(&old)).is_err());
        let store = EngineMapDataStore::open_sealed(Arc::clone(engine), &on(&new)).expect("new");
        block_on(async {
            assert_eq!(store.load("m", "k1").await.expect("load"), Some(lww("v1")));
        });
        // With the new key in place the previous one is ignored.
        drop(store);
        let again = rows(&**engine);
        drop(EngineMapDataStore::open_sealed(Arc::clone(engine), &rotation).expect("ignored"));
        assert_eq!(rows(&**engine), again);
    }

    /// 30 001 records over two maps and both spaces: four batches of the pass.
    fn populate(store: &EngineMapDataStore<impl KvEngine>) {
        let values: Vec<(String, Vec<u8>)> = (0..30_001)
            .map(|i| {
                (
                    format!("key-{i}"),
                    rmp_serde::to_vec_named(&lww(&format!("value-{i}"))).expect("encode"),
                )
            })
            .collect();
        let ops: Vec<RecordOp<'_>> = values
            .iter()
            .enumerate()
            .map(|(i, (key, bytes))| RecordOp::Put {
                map: if i % 5 == 0 { "b" } else { "a" },
                backup: i % 7 == 0,
                key,
                bytes,
            })
            .collect();
        store
            .write_records(&ops, WriteOptions { sync: true })
            .expect("populate");
    }

    /// The pass's engine writes, as op counts relative to its start, from an
    /// uninterrupted run.
    fn pass_boundaries<T: CrashTarget>(
        provider: &Arc<dyn MasterKeyProvider>,
        direction: PassDirection,
    ) -> Vec<u64> {
        let target = start_pass::<T>(provider, direction);
        let store = open_for(&target, provider, direction);
        let base = target.ops_so_far();
        let mut boundaries = Vec::new();
        store
            .seal_pass_with(SEAL_PASS_BATCH_ROWS, SEAL_PASS_BATCH_BYTES, &mut || {
                boundaries.push(target.ops_so_far() - base);
            })
            .expect("pass");
        boundaries
    }

    fn open_for<T: CrashTarget>(
        target: &T,
        provider: &Arc<dyn MasterKeyProvider>,
        direction: PassDirection,
    ) -> EngineMapDataStore<T::Engine> {
        let encryption = match direction {
            PassDirection::Seal => on(provider),
            PassDirection::Unseal => off(provider),
        };
        EngineMapDataStore::open_sealed(target.engine(), &encryption).expect("open")
    }

    /// A store of plain values, or of values sealed by a finished pass,
    /// whose next open starts a pass in `direction`.
    fn start_pass<T: CrashTarget>(
        provider: &Arc<dyn MasterKeyProvider>,
        direction: PassDirection,
    ) -> T {
        let target = T::fresh();
        let store = EngineMapDataStore::open(target.engine()).expect("plain");
        populate(&store);
        drop(store);
        if direction == PassDirection::Unseal {
            let store =
                EngineMapDataStore::open_sealed(target.engine(), &on(provider)).expect("on");
            store.run_seal_pass().expect("seal");
        }
        target
    }

    /// TG-ENC-004: a seal and an unseal pass over 30 001 rows, killed
    /// at every batch boundary and at one point inside every batch, end
    /// after a restart with every row in the target form and its value
    /// unchanged, and no pass row.
    fn pass_survives_crashes<T: CrashTarget>(modes: &[CrashMode]) {
        let provider = kek();
        let plain_rows = {
            let target = T::fresh();
            let store = EngineMapDataStore::open(target.engine()).expect("plain");
            populate(&store);
            record_rows(&*target.engine())
        };
        for direction in [PassDirection::Seal, PassDirection::Unseal] {
            let boundaries = pass_boundaries::<T>(&provider, direction);
            assert!(
                boundaries.len() >= 4,
                "{boundaries:?}: fewer than three batches"
            );
            let mut points = vec![0];
            for pair in boundaries.windows(2) {
                points.push(pair[0]);
                points.push(u64::midpoint(pair[0], pair[1]));
            }
            points.push(boundaries[0] / 2);
            points.sort_unstable();
            points.dedup();
            for &point in &points {
                for &mode in modes {
                    let target = start_pass::<T>(&provider, direction);
                    let store = open_for(&target, &provider, direction);
                    target.arm_death(target.ops_so_far() + point);
                    let died = store.run_seal_pass().is_err();
                    drop(store);
                    let target = target.crash(mode, 0);
                    let store = open_for(&target, &provider, direction);
                    store.run_seal_pass().unwrap_or_else(|e| {
                        panic!(
                            "{} {direction:?} resume after op {point} ({died}): {e}",
                            T::NAME
                        )
                    });
                    let keyring = store.keyring.clone();
                    let active = keyring.as_deref().and_then(Keyring::active_id);
                    let engine = target.engine();
                    assert_eq!(
                        engine.get(&meta_key(META_SEAL_PASS)).expect("get"),
                        None,
                        "{} {direction:?} op {point}: the pass row is left",
                        T::NAME
                    );
                    let now = record_rows(&*engine);
                    assert_eq!(now.len(), plain_rows.len());
                    for (key, plain) in &plain_rows {
                        let stored = &now[key];
                        match direction {
                            PassDirection::Seal => {
                                assert_eq!(
                                    sealed_key_id(stored),
                                    active,
                                    "{} op {point}: a row is not sealed under the active key",
                                    T::NAME
                                );
                                assert_eq!(
                                    open_value(keyring.as_deref(), key, stored)
                                        .expect("open")
                                        .as_ref(),
                                    plain.as_slice()
                                );
                            }
                            PassDirection::Unseal => {
                                assert_eq!(
                                    stored,
                                    plain,
                                    "{} op {point}: a row is not plain",
                                    T::NAME
                                );
                            }
                        }
                    }
                    drop(store);
                }
            }
        }
    }

    #[test]
    fn the_seal_pass_resumes_to_the_same_end_after_a_crash_over_the_reference_engine() {
        pass_survives_crashes::<MemTarget>(&[CrashMode::DropUnsynced]);
    }

    /// A crash that keeps everything synced and nothing after: the pass is
    /// judged at the grain of its durable batches. Images that keep a random
    /// part of the unsynced writes test redb's own recovery, not the pass.
    #[test]
    fn the_seal_pass_resumes_to_the_same_end_after_a_crash_over_redb() {
        pass_survives_crashes::<RedbTarget>(&[CrashMode::DropUnsynced]);
    }
}
