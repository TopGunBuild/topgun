//! [`EngineMapDataStore`]: a [`MapDataStore`] over any [`KvEngine`].
//!
//! # Key format (`TG-ENG-004`)
//!
//! One engine holds every map. The first byte of a key names its keyspace:
//!
//! | First byte | Keyspace | Key after the byte | Value |
//! |---|---|---|---|
//! | `0x00` | store metadata | `format`, `next_map_id`, `ack_mode`, `migration` | u32 BE `1`; u32 BE; u8 `0` (local); opaque bytes |
//! | `0x01` | map by name | the map name, UTF-8 | map id u32 BE ‖ flags u8 |
//! | `0x02` | map by id | map id u32 BE | the map name, UTF-8 |
//! | `0x10` | primary records | partition u32 BE ‖ map id u32 BE ‖ user key | the record's msgpack |
//! | `0x11` | backup records | as `0x10` | as `0x10` |
//! | `0x20` | partition metadata, reserved | — | — |
//!
//! Every other first byte is reserved. The partition is
//! [`hash_to_partition`] of the user key, the function the cluster routes a
//! key by, so a partition's records are one contiguous range per map. The
//! user key is the last field, so no separator or escaping is needed.
//!
//! A map's scan walks the 271 partitions in sequence, so its rows come in
//! `(partition, user key)` order — the byte order of their engine keys — and
//! a cursor is the engine key of the last row ([`PARTITION_THEN_KEY`]).
//!
//! # Registry (`TG-ENG-005`) and metadata (`TG-ENG-006`)
//!
//! A map id is issued in the same engine batch as the map's first record, so
//! a failed first write leaves neither a registry row nor a record. A store
//! opens only at a format, ack mode and keyspace set this binary knows, and
//! with no unfinished migration; anything else is refused at open with
//! nothing written.
//!
//! # Durability
//!
//! Every [`MapDataStore`] method writes with `sync = true`: the store is
//! write-through, durable on return, as the redb store is.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use async_trait::async_trait;
use parking_lot::{Mutex, MutexGuard, RwLock};
use topgun_core::partition::{hash_to_partition, PARTITION_COUNT};

use super::kv_engine::{KeyRange, KvEngine, KvError, KvSnapshot, WriteBatch, WriteOptions};
use crate::storage::map_data_store::{
    check_map_name, merkle_leaf_hash, LeafSink, MapDataStore, MerkleLeaf, ScanBatch, ScanCursor,
    ScanOrder, StoreErrorClass,
};
use crate::storage::record::RecordValue;

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
/// Partition metadata; reserved, no key is written there yet.
pub const KS_PARTITION_META: u8 = 0x20;

/// Metadata name of the format version (u32 BE).
pub const META_FORMAT: &[u8] = b"format";
/// Metadata name of the next map id to issue (u32 BE).
pub const META_NEXT_MAP_ID: &[u8] = b"next_map_id";
/// Metadata name of the ack mode (u8, `0` = local).
pub const META_ACK_MODE: &[u8] = b"ack_mode";
/// Metadata name of an unfinished migration's mark (opaque bytes).
pub const META_MIGRATION: &[u8] = b"migration";
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

/// Number of leaves accumulated before invoking the sink once, as the redb store does.
const LEAF_BATCH_SIZE: usize = 1024;

/// Per-batch byte budget of a value scan called with `max_batch_cost == 0`,
/// as the redb store's.
const DEFAULT_SCAN_BATCH_COST: u64 = 8 * 1024 * 1024;

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
    let reserved = [
        KeyRange {
            start: Some(vec![KS_MAP_BY_ID + 1]),
            end: Some(vec![KS_PRIMARY]),
        },
        KeyRange {
            start: Some(vec![KS_BACKUP + 1]),
            end: None,
        },
    ];
    for range in reserved {
        if let Some(key) = first_key_in(snapshot, range)? {
            bail!("{REFUSAL} a key in the reserved keyspace {:#04x}", key[0]);
        }
    }
    Ok(Some(StoreMetadata { migration }))
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
            Self::SetMigrationMark(_) => None,
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
    #[cfg(test)]
    registry_park: Mutex<Option<RegistryPark>>,
}

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
    /// keyspace, or an unfinished migration (`TG-ENG-006`); an engine error.
    pub fn open(engine: Arc<E>) -> anyhow::Result<Self> {
        let metadata = read_store_metadata(&*engine.snapshot()?)?;
        match metadata {
            None => {
                let mut batch = WriteBatch::new();
                for (key, value) in format_rows() {
                    batch.put(key, value);
                }
                engine.write(batch, WriteOptions { sync: true })?;
            }
            Some(StoreMetadata {
                migration: Some(_), ..
            }) => {
                bail!("{REFUSAL} a migration into this store is unfinished");
            }
            Some(_) => {}
        }
        Ok(Self {
            engine,
            registry_lock: Mutex::new(()),
            cache: RwLock::new(HashMap::new()),
            #[cfg(test)]
            registry_park: Mutex::new(None),
        })
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
        let mark = match metadata {
            None => {
                // One batch, so no crash point leaves a formatted store
                // without its mark.
                let mut batch = WriteBatch::new();
                for (key, value) in format_rows() {
                    batch.put(key, value);
                }
                batch.put(meta_key(META_MIGRATION), initial_mark.to_vec());
                engine.write(batch, WriteOptions { sync: true })?;
                initial_mark.to_vec()
            }
            Some(StoreMetadata {
                migration: Some(mark),
            }) => mark,
            Some(StoreMetadata { migration: None }) => bail!(
                "store format: refusing to resume a migration: the store has no migration mark"
            ),
        };
        let store = Self {
            engine,
            registry_lock: Mutex::new(()),
            cache: RwLock::new(HashMap::new()),
            #[cfg(test)]
            registry_park: Mutex::new(None),
        };
        Ok((store, mark))
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
                    // of a map's partition range — never overflows.
                    if map_id == u32::MAX {
                        bail!("map id space exhausted");
                    }
                    puts.push((map_by_name_key(name), map_by_name_value(map_id, need)));
                    puts.push((map_by_id_key(map_id), name.as_bytes().to_vec()));
                    next_map_id = Some(map_id + 1);
                    Some((map_id, need))
                }
            };
            rows.insert(name, planned);
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
                    batch.put(
                        RecordKey::encode(RecordSpace::of(backup), map_id, key),
                        bytes.to_vec(),
                    );
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
                RecordOp::SetMigrationMark(None) => batch.delete(meta_key(META_MIGRATION)),
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
                let value: RecordValue = rmp_serde::from_slice(&bytes)?;
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
        match snapshot.get(&RecordKey::encode(RecordSpace::Primary, map_id, key))? {
            Some(bytes) => Ok(Some(rmp_serde::from_slice(&bytes)?)),
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
            if let Some(bytes) =
                snapshot.get(&RecordKey::encode(RecordSpace::Primary, map_id, key))?
            {
                found.push((key.clone(), rmp_serde::from_slice(&bytes)?));
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
                let value: RecordValue = rmp_serde::from_slice(&bytes)?;
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
}
