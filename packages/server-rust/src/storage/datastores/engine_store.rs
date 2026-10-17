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

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use topgun_core::partition::hash_to_partition;

use super::kv_engine::{KvEngine, KvSnapshot, WriteOptions};
use crate::storage::map_data_store::{
    LeafSink, MapDataStore, ScanBatch, ScanCursor, ScanOrder, StoreErrorClass,
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

/// The engine key of the metadata row `name`: `KS_META ‖ name`.
#[must_use]
pub fn meta_key(name: &[u8]) -> Vec<u8> {
    let _ = name;
    unimplemented!()
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
        unimplemented!()
    }
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
        let _ = (space, map_id, key);
        unimplemented!()
    }

    /// The inverse of [`RecordKey::encode`].
    ///
    /// # Errors
    ///
    /// When the bytes are shorter than 9, the first byte is not a record
    /// keyspace, the user key is not UTF-8, or the partition is not
    /// `hash_to_partition` of the user key — no key this store writes.
    pub fn decode(bytes: &[u8]) -> anyhow::Result<RecordKey> {
        let _ = bytes;
        unimplemented!()
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
    let _ = name;
    unimplemented!()
}

/// Decodes a `0x01` row.
///
/// # Errors
///
/// When the key is not `KS_MAP_BY_NAME ‖ UTF-8`, the value is not 5 bytes,
/// or the id is 0.
pub fn decode_map_by_name_row(key: &[u8], value: &[u8]) -> anyhow::Result<RegistryEntry> {
    let _ = (key, value);
    unimplemented!()
}

/// The engine key of the `0x02` row of `map_id`.
#[must_use]
pub fn map_by_id_key(map_id: u32) -> Vec<u8> {
    let _ = map_id;
    unimplemented!()
}

/// Decodes a `0x02` row into `(map_id, name)`.
///
/// # Errors
///
/// When the key is not 5 bytes starting `KS_MAP_BY_ID`, or the value is not
/// UTF-8.
pub fn decode_map_by_id_row(key: &[u8], value: &[u8]) -> anyhow::Result<(u32, String)> {
    let _ = (key, value);
    unimplemented!()
}

/// The id the `0x01` row of `name` holds in `snapshot`, `None` only when the
/// row is absent. No name check and no cache.
///
/// # Errors
///
/// An engine error is returned, never read as "absent"; a malformed row is
/// an error.
pub fn map_id_in(snapshot: &dyn KvSnapshot, name: &str) -> anyhow::Result<Option<u32>> {
    let _ = (snapshot, name);
    unimplemented!()
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
    let _ = (key, hash_to_partition);
    unimplemented!()
}

fn partition_then_key_cursor_position(cursor: &ScanCursor) -> anyhow::Result<Vec<u8>> {
    let _ = cursor;
    unimplemented!()
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
        let _ = engine;
        unimplemented!()
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
        let _ = (engine, initial_mark);
        unimplemented!()
    }

    /// The migration mark of `engine`, read from one snapshot; writes nothing.
    ///
    /// # Errors
    ///
    /// An engine error, never read as "absent".
    pub fn read_migration_mark(engine: &E) -> anyhow::Result<Option<Vec<u8>>> {
        let _ = engine;
        unimplemented!()
    }

    /// Applies `ops`, and every registry row they need, as one engine batch.
    ///
    /// # Errors
    ///
    /// A refused map name (nothing is written); `map id space exhausted`;
    /// an engine error, with nothing of the batch applied.
    pub fn write_records(&self, ops: &[RecordOp<'_>], opts: WriteOptions) -> anyhow::Result<()> {
        let _ = (ops, opts);
        unimplemented!()
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
        let _ = (map, backup, visit);
        unimplemented!()
    }

    /// Makes every returned write durable.
    ///
    /// # Errors
    ///
    /// The engine's error.
    pub fn sync(&self) -> anyhow::Result<()> {
        unimplemented!()
    }

    /// Arms a one-shot park in the next `write_records` call that issues a
    /// map id: between its in-lock registry re-read and its engine write it
    /// sends on the returned receiver's channel, then waits for a message on
    /// the returned sender or for the sender to be dropped.
    #[cfg(test)]
    pub(crate) fn arm_registry_park(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let _ = &self.registry_park;
        unimplemented!()
    }

    /// The number of registry cache entries.
    #[cfg(test)]
    pub(crate) fn registry_cache_len(&self) -> usize {
        let _ = (&self.cache, &self.registry_lock);
        unimplemented!()
    }
}

#[async_trait]
impl<E: KvEngine> MapDataStore for EngineMapDataStore<E> {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        let _ = (map, key, value);
        unimplemented!()
    }

    fn accepts_encoded(&self) -> bool {
        unimplemented!()
    }

    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        let _ = (map, key, bytes);
        unimplemented!()
    }

    fn scan_order(&self) -> Option<ScanOrder> {
        unimplemented!()
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        let _ = (map, key, value);
        unimplemented!()
    }

    async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        let _ = (map, key);
        unimplemented!()
    }

    async fn remove_backup(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        let _ = (map, key);
        unimplemented!()
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        let _ = (map, key);
        unimplemented!()
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        let _ = (map, keys);
        unimplemented!()
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        let _ = (map, is_backup, sink);
        unimplemented!()
    }

    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        let _ = (map, is_backup, max_batch_cost);
        unimplemented!()
    }

    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        let _ = (map, is_backup, cursor, max_batch_cost);
        unimplemented!()
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        let _ = (map, keys);
        unimplemented!()
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        unimplemented!()
    }

    fn is_loadable(&self, _key: &str) -> bool {
        unimplemented!()
    }

    fn pending_operation_count(&self) -> u64 {
        unimplemented!()
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        unimplemented!()
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        unimplemented!()
    }

    fn classify_error(&self, err: &anyhow::Error) -> StoreErrorClass {
        let _ = err;
        unimplemented!()
    }
}
