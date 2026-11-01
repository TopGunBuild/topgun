//! Record expiry (TG-TTL-001): when a record or an OR entry with a time to
//! live stops being visible.
//!
//! A TTL is anchored at the stored, server-stamped HLC timestamp of the record
//! (LWW) or of the entry (OR): it expires at `timestamp.millis + ttl_ms`. A
//! reader decides against its own `now`, read from the clock of the
//! [`RecordStoreFactory`](super::RecordStoreFactory) that owns the record, so
//! every function here is pure: no I/O and no clock read.
//!
//! `ttl_ms` of `None` or `Some(0)` means no expiry, matching the client's
//! truthiness test on `ttlMs`; a record is still live at the exact millisecond
//! it expires and gone one millisecond later, as on the client.

use std::sync::atomic::{AtomicU64, Ordering};

use topgun_core::hlc::{ClockSource, Timestamp};

use crate::storage::record::{OrMapEntry, RecordValue};

/// Millisecond at which a value written at `timestamp_millis` with `ttl_ms`
/// expires, or `None` when it never does. Saturates instead of overflowing, so
/// a huge TTL means "never" in practice rather than a wrapped past instant.
#[must_use]
pub fn expires_at(timestamp_millis: u64, ttl_ms: Option<u64>) -> Option<u64> {
    ttl_ms
        .filter(|&ttl| ttl != 0)
        .map(|ttl| timestamp_millis.saturating_add(ttl))
}

/// Whether an LWW value stamped `timestamp` with `ttl_ms` is still live at `now`.
#[must_use]
pub fn lww_is_live(timestamp: &Timestamp, ttl_ms: Option<u64>, now: u64) -> bool {
    expires_at(timestamp.millis, ttl_ms).is_none_or(|at| now <= at)
}

/// Whether an OR entry is still live at `now`, judged on its own timestamp and TTL.
#[must_use]
pub fn entry_is_live(entry: &OrMapEntry, now: u64) -> bool {
    lww_is_live(&entry.timestamp, entry.ttl_ms, now)
}

/// Whether a stored value has anything a reader may see at `now`: a live LWW
/// value, or an OR slot with at least one live entry. A tombstone-only slot is
/// never visible.
#[must_use]
pub fn is_visible(value: &RecordValue, now: u64) -> bool {
    match value {
        RecordValue::Lww {
            timestamp, ttl_ms, ..
        } => lww_is_live(timestamp, *ttl_ms, now),
        RecordValue::OrMap { records, .. } => records.iter().any(|e| entry_is_live(e, now)),
        RecordValue::OrTombstones { .. } => false,
    }
}

/// The part of a stored value a reader may see at `now`: `None` for an expired
/// LWW value; an OR slot without its expired entries, its `tombstones` kept
/// (they are removals, not entries, and must keep suppressing older adds); any
/// other value unchanged.
#[must_use]
pub fn retain_live(value: RecordValue, now: u64) -> Option<RecordValue> {
    match value {
        RecordValue::Lww {
            ref timestamp,
            ttl_ms,
            ..
        } if !lww_is_live(timestamp, ttl_ms, now) => None,
        RecordValue::OrMap {
            mut records,
            tombstones,
        } => {
            records.retain(|e| entry_is_live(e, now));
            Some(RecordValue::OrMap {
                records,
                tombstones,
            })
        }
        other => Some(other),
    }
}

/// Tags of the entries of an OR slot that have expired at `now`; empty for any
/// other value.
#[must_use]
pub fn expired_or_tags(value: &RecordValue, now: u64) -> Vec<String> {
    match value {
        RecordValue::OrMap { records, .. } => records
            .iter()
            .filter(|e| !entry_is_live(e, now))
            .map(|e| e.tag.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// A clock that only moves when told to, for tests that must decide expiry at
/// an exact millisecond. Public (not test-only) so integration and simulation
/// tests can inject it through
/// [`RecordStoreFactory::with_clock`](super::RecordStoreFactory::with_clock).
#[derive(Debug, Default)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    /// A clock reading `ms`.
    #[must_use]
    pub fn new(ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(ms),
        }
    }

    /// Sets the reading to `ms`.
    pub fn set(&self, ms: u64) {
        self.now_ms.store(ms, Ordering::SeqCst);
    }

    /// Moves the reading forward by `ms`.
    pub fn advance(&self, ms: u64) {
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl ClockSource for ManualClock {
    fn now(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// What an expiry sweep saw when it found a record expired, so that the
/// removal can re-check it under the key writer and leave alone a record that
/// was rewritten in between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpiryExpectation {
    /// The sweep saw this LWW record expired; the handler removes the key only if the stored record
    /// still has this timestamp and is still expired.
    Lww { timestamp: Timestamp },
    /// The sweep saw expired entries in this OR slot; the handler removes every entry expired at its own `now`.
    Or,
}

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::types::Value;

    fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        }
    }

    fn lww(millis: u64, ttl_ms: Option<u64>) -> RecordValue {
        RecordValue::Lww {
            value: Value::Int(1),
            timestamp: ts(millis),
            ttl_ms,
        }
    }

    fn entry(tag: &str, millis: u64, ttl_ms: Option<u64>) -> OrMapEntry {
        OrMapEntry {
            value: Value::Int(1),
            tag: tag.to_string(),
            timestamp: ts(millis),
            ttl_ms,
        }
    }

    #[test]
    fn expires_at_none_zero_normal_and_saturation() {
        assert_eq!(expires_at(1_000, None), None);
        assert_eq!(expires_at(1_000, Some(0)), None);
        assert_eq!(expires_at(1_000, Some(500)), Some(1_500));
        assert_eq!(expires_at(u64::MAX - 1, Some(10)), Some(u64::MAX));
        assert_eq!(expires_at(1, Some(u64::MAX)), Some(u64::MAX));
    }

    #[test]
    fn lww_is_live_boundaries() {
        let t = ts(1_000);
        assert!(
            lww_is_live(&t, Some(500), 1_500),
            "live at now == expires_at"
        );
        assert!(
            !lww_is_live(&t, Some(500), 1_501),
            "expired one millisecond later"
        );
        assert!(lww_is_live(&t, Some(500), 0), "live before the write");
        assert!(lww_is_live(&t, None, u64::MAX), "no TTL never expires");
        assert!(lww_is_live(&t, Some(0), u64::MAX), "ttl 0 never expires");
        assert!(
            lww_is_live(&ts(u64::MAX - 1), Some(10), u64::MAX),
            "saturated expiry is still live at u64::MAX"
        );
    }

    #[test]
    fn entry_is_live_boundaries() {
        assert!(entry_is_live(&entry("a", 1_000, Some(500)), 1_500));
        assert!(!entry_is_live(&entry("a", 1_000, Some(500)), 1_501));
        assert!(entry_is_live(&entry("a", 1_000, None), u64::MAX));
        assert!(entry_is_live(&entry("a", 1_000, Some(0)), u64::MAX));
    }

    #[test]
    fn is_visible_per_variant() {
        assert!(is_visible(&lww(1_000, Some(500)), 1_500));
        assert!(!is_visible(&lww(1_000, Some(500)), 1_501));

        let mixed = RecordValue::OrMap {
            records: vec![
                entry("old", 1_000, Some(10)),
                entry("new", 1_000, Some(900)),
            ],
            tombstones: vec![],
        };
        assert!(is_visible(&mixed, 1_500), "one live entry among expired");

        let all_expired = RecordValue::OrMap {
            records: vec![entry("a", 1_000, Some(10)), entry("b", 1_000, Some(20))],
            tombstones: vec!["gone".to_string()],
        };
        assert!(!is_visible(&all_expired, 1_500), "every entry expired");

        let empty = RecordValue::OrMap {
            records: vec![],
            tombstones: vec!["gone".to_string()],
        };
        assert!(!is_visible(&empty, 0), "no entries");

        let tombstones = RecordValue::OrTombstones {
            tags: vec!["gone".to_string()],
        };
        assert!(!is_visible(&tombstones, 0), "tombstone-only slot");
    }

    #[test]
    fn retain_live_per_variant() {
        assert_eq!(retain_live(lww(1_000, Some(500)), 1_501), None);
        assert_eq!(
            retain_live(lww(1_000, Some(500)), 1_500),
            Some(lww(1_000, Some(500)))
        );
        assert_eq!(
            retain_live(lww(1_000, None), u64::MAX),
            Some(lww(1_000, None))
        );

        let slot = RecordValue::OrMap {
            records: vec![
                entry("expired", 1_000, Some(10)),
                entry("live", 1_000, Some(900)),
                entry("forever", 1_000, None),
            ],
            tombstones: vec!["t1".to_string(), "t2".to_string()],
        };
        assert_eq!(
            retain_live(slot, 1_500),
            Some(RecordValue::OrMap {
                records: vec![
                    entry("live", 1_000, Some(900)),
                    entry("forever", 1_000, None)
                ],
                tombstones: vec!["t1".to_string(), "t2".to_string()],
            })
        );

        let tombstones = RecordValue::OrTombstones {
            tags: vec!["gone".to_string()],
        };
        assert_eq!(retain_live(tombstones.clone(), u64::MAX), Some(tombstones));
    }

    #[test]
    fn expired_or_tags_per_variant() {
        let slot = RecordValue::OrMap {
            records: vec![
                entry("e1", 1_000, Some(10)),
                entry("live", 1_000, Some(900)),
                entry("e2", 1_000, Some(499)),
                entry("forever", 1_000, None),
            ],
            tombstones: vec!["t".to_string()],
        };
        assert_eq!(
            expired_or_tags(&slot, 1_500),
            vec!["e1".to_string(), "e2".to_string()]
        );
        assert!(expired_or_tags(&lww(1_000, Some(1)), u64::MAX).is_empty());
        assert!(expired_or_tags(
            &RecordValue::OrTombstones {
                tags: vec!["gone".to_string()]
            },
            u64::MAX
        )
        .is_empty());
    }

    #[test]
    fn manual_clock_reads_what_it_is_told() {
        let clock = ManualClock::new(10);
        assert_eq!(clock.now(), 10);
        clock.advance(5);
        assert_eq!(clock.now(), 15);
        clock.set(3);
        assert_eq!(clock.now(), 3);
    }
}

/// The TTL is part of the stored value, so it survives everything the value
/// survives (TG-TTL-001): eviction and reload from the durable store, and WAL
/// replay of a snapshot frame and of an OR delta frame after a restart. Each
/// proof reopens the store from disk and compares the encoded bytes.
#[cfg(all(test, feature = "redb"))]
mod durability {
    use std::path::Path;
    use std::sync::Arc;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use crate::storage::datastores::RedbDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::key_writer::KeyWriteToken;
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::record::{OrMapEntry, RecordValue};
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
    use crate::storage::wal::{
        Wal, WalEntry, WalFsyncPolicy, WalOp, WalRecovery, WalStorePayload, WalWriter,
    };

    const MAP: &str = "ttl";

    fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 3,
            node_id: "node-a".to_string(),
        }
    }

    fn lww_with_ttl() -> RecordValue {
        RecordValue::Lww {
            value: Value::String("expiring".to_string()),
            timestamp: ts(1_700_000_000_000),
            ttl_ms: Some(60_000),
        }
    }

    fn entry_with_ttl(tag: &str, ttl_ms: Option<u64>) -> OrMapEntry {
        OrMapEntry {
            value: Value::Int(7),
            tag: tag.to_string(),
            timestamp: ts(1_700_000_000_000),
            ttl_ms,
        }
    }

    fn bytes(value: &RecordValue) -> Vec<u8> {
        rmp_serde::to_vec_named(value).expect("encode")
    }

    async fn load_from_disk(data_dir: &Path, key: &str) -> RecordValue {
        let store = RedbDataStore::new(data_dir).expect("reopen store");
        store
            .load(MAP, key)
            .await
            .expect("load")
            .expect("the record is stored")
    }

    #[tokio::test]
    async fn ttl_survives_eviction_and_reload_from_the_durable_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let value = lww_with_ttl();
        {
            let data_store = Arc::new(RedbDataStore::new(&data_dir).expect("store"));
            let factory = RecordStoreFactory::new(StorageConfig::default(), data_store, Vec::new());
            let store = factory.get_or_create(MAP, 0);
            store
                .put(
                    &KeyWriteToken::for_test(MAP, "k"),
                    "k",
                    value.clone(),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put");
            assert!(store.evict("k", false).is_some(), "the record was resident");
            assert!(!store.exists_in_memory("k"));

            let reloaded = store
                .get("k", false)
                .await
                .expect("get")
                .expect("reloaded from the store");
            assert_eq!(bytes(&reloaded.value), bytes(&value));
        }
        let from_disk = load_from_disk(&data_dir, "k").await;
        assert_eq!(bytes(&from_disk), bytes(&value));
        assert_eq!(from_disk, value);
    }

    /// Appends `entries` to a store-mode WAL over a fresh store, closes both,
    /// reopens them and runs recovery, as a restart does.
    async fn replay(dir: &Path, entries: Vec<WalEntry>) {
        let data_dir = dir.join("data");
        let wal_dir = dir.join("wal");
        {
            let store: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(&data_dir).expect("store"));
            let wal = WalWriter::open_over_store(wal_dir.clone(), WalFsyncPolicy::PerOp, store)
                .expect("WAL");
            for entry in &entries {
                wal.append(0, entry).await.expect("append");
            }
        }
        let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&data_dir).expect("store"));
        let wal = WalWriter::open_over_store(wal_dir, WalFsyncPolicy::PerOp, Arc::clone(&store))
            .expect("WAL");
        WalRecovery::new(wal, vec![0])
            .run(store)
            .await
            .expect("recovery");
    }

    #[tokio::test]
    async fn ttl_survives_wal_replay_of_a_store_frame() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lww = lww_with_ttl();
        let slot = RecordValue::OrMap {
            records: vec![entry_with_ttl("a", Some(1_000)), entry_with_ttl("b", None)],
            tombstones: vec!["gone".to_string()],
        };
        replay(
            dir.path(),
            vec![
                WalEntry {
                    map: MAP.to_string(),
                    key: "lww".to_string(),
                    op: WalOp::Store {
                        value: WalStorePayload::Record(lww.clone()),
                        expiration_time: None,
                    },
                    timestamp: Some(ts(1_700_000_000_000)),
                    sequence: 1,
                },
                WalEntry {
                    map: MAP.to_string(),
                    key: "or".to_string(),
                    op: WalOp::Store {
                        value: WalStorePayload::Record(slot.clone()),
                        expiration_time: None,
                    },
                    timestamp: None,
                    sequence: 2,
                },
            ],
        )
        .await;
        let data_dir = dir.path().join("data");
        let replayed_lww = load_from_disk(&data_dir, "lww").await;
        assert_eq!(bytes(&replayed_lww), bytes(&lww));
        let replayed_slot = load_from_disk(&data_dir, "or").await;
        assert_eq!(bytes(&replayed_slot), bytes(&slot));
    }

    /// The OR delta frame is written by the write-behind store, the one
    /// sanctioned emitter (TG-OR-003), from the witness of an OR add; the store
    /// is then abandoned without a flush, as by `kill -9` (its runtime is
    /// dropped), and the restart folds the frame onto the empty store. The same
    /// frame also decodes with the frozen shapes of the binary before `ttl_ms`.
    #[test]
    fn ttl_survives_wal_replay_of_an_or_delta_add_frame() {
        use crate::storage::datastores::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
        use crate::storage::map_data_store::ttl_bytes::{
            BaseOrDelta, BaseOrMapEntry, BaseWalEntry, EarlierFrameOp,
        };
        use crate::storage::map_data_store::WriteSource;
        use crate::storage::wal::OrDelta;

        let runtime = || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("runtime")
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let wal_dir = dir.path().join("wal");
        let added = entry_with_ttl("t1", Some(30_000));
        let slot = RecordValue::OrMap {
            records: vec![added.clone()],
            tombstones: Vec::new(),
        };

        let frames = runtime().block_on(async {
            let store: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(&data_dir).expect("store"));
            let wal = WalWriter::open_over_store(
                wal_dir.clone(),
                WalFsyncPolicy::PerOp,
                Arc::clone(&store),
            )
            .expect("WAL");
            let write_behind = WriteBehindDataStore::new_with_wal(
                store,
                WriteBehindConfig {
                    write_delay_ms: 3_600_000,
                    flush_interval_ms: 3_600_000,
                    or_delta_wal: true,
                    ..WriteBehindConfig::default()
                },
                Some(WalBootstrap {
                    wal: Arc::clone(&wal) as Arc<dyn Wal>,
                    sequence_start: 1,
                }),
            );
            write_behind
                .add_with_witness(
                    MAP,
                    "or",
                    WriteSource::Value(&slot),
                    0,
                    1_700_000_000_000,
                    Some(&OrDelta::Add {
                        entry: added.clone(),
                    }),
                )
                .await
                .expect("add");
            let mut frames = Vec::new();
            for partition in 0..topgun_core::PARTITION_COUNT {
                frames.extend(wal.unapplied(partition).await.expect("unapplied"));
            }
            frames
        });
        assert_eq!(frames.len(), 1, "one frame, not flushed");
        assert!(
            !frames[0].op.subsumes_on_coalesce(),
            "the frame is a delta, not a snapshot: {:?}",
            frames[0].op
        );
        let earlier: BaseWalEntry =
            rmp_serde::from_slice(&rmp_serde::to_vec_named(&frames[0]).expect("encode"))
                .expect("the earlier shapes decode the delta frame");
        assert_eq!(
            earlier.op,
            EarlierFrameOp::OrDelta {
                delta: BaseOrDelta::Add {
                    entry: BaseOrMapEntry {
                        value: added.value.clone(),
                        tag: added.tag.clone(),
                        timestamp: added.timestamp.clone(),
                    },
                },
            }
        );

        runtime().block_on(async {
            let store: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(&data_dir).expect("store"));
            let wal =
                WalWriter::open_over_store(wal_dir, WalFsyncPolicy::PerOp, Arc::clone(&store))
                    .expect("WAL");
            WalRecovery::new(wal, Vec::new())
                .run(store)
                .await
                .expect("recovery");
        });
        let replayed = runtime().block_on(load_from_disk(&data_dir, "or"));
        assert_eq!(bytes(&replayed), bytes(&slot));
    }
}
