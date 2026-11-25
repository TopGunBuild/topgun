//! WAL replay above the applied watermark is an idempotent merge
//! (`TG-WAL-015`): a frame at or below its key's stored record version changes
//! no row, whatever its kind, and forward replay still equals the live write.
//!
//! Every test runs over a real store-mode [`EngineMapDataStore`] and judges a
//! replay by the engine's own bytes: the primary records (`0x10`) and the
//! record versions (`0x12`). The watermark is pinned by an earlier write the
//! store refuses (a poisoned value of another key of the same partition), so
//! the frames above it stay in the replay window after recovery and can be
//! replayed again.

// Each test is one scenario read top to bottom — set up, replay, replay
// again, check — and splitting one across helpers would hide the order its
// claim depends on.
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestRunner};
use topgun_core::types::Value;

use super::batched_replay_tests::{entry, or_value, remove_frame, ts, FaultNode, Refusal};
use super::store_mode_tests::{keys_in, lww, store_frame, wal_partition, Root, MAP};
use super::tests::delta_frame;
use super::{OrDelta, WalEntry, WalOp, WalRecovery, WalStorePayload};
use crate::service::domain::crdt::{apply_or_delta, normalize_to_or_map};
use crate::storage::datastores::engine_store::{KS_PRIMARY, KS_RECORD_VERSION};
use crate::storage::map_data_store::{FlushCommit, KeyVersion, MapDataStore, PutValue, StoreWrite};
use crate::storage::record::RecordValue;

/// The WAL partition every frame of these tests is appended to, and the
/// partition of every key they write, so the pin holds back the watermark the
/// engine compares their version rows with.
const P: u32 = 7;

/// The primary records and the record versions of the node's engine: what an
/// idempotent replay must leave byte for byte.
fn records_and_versions(node: &FaultNode) -> BTreeMap<Vec<u8>, Vec<u8>> {
    node.engine
        .dump()
        .into_iter()
        .filter(|(k, _)| matches!(k.first(), Some(&KS_PRIMARY | &KS_RECORD_VERSION)))
        .collect()
}

/// `[h, pin, k]`: three keys of `MAP` in partition `P`.
fn three_keys() -> (String, String, String) {
    let keys = keys_in(P, 3);
    (keys[0].clone(), keys[1].clone(), keys[2].clone())
}

/// The frame of the write the store refuses, which pins the watermark below it.
fn pin_frame(pin: &str, sequence: u64) -> WalEntry {
    store_frame(pin, &lww("poison-pin", 1), sequence)
}

fn legacy_frame(key: &str, text: &str, millis: u64, sequence: u64) -> WalEntry {
    WalEntry {
        map: MAP.to_string(),
        key: key.to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Legacy(Value::String(text.to_string())),
        },
        timestamp: Some(ts(millis)),
        sequence,
    }
}

fn add(key: &str, tag: &str, sequence: u64) -> WalEntry {
    delta_frame(
        key,
        OrDelta::Add {
            entry: entry(tag, tag),
        },
        sequence,
    )
}

fn or_remove(key: &str, tag: &str, sequence: u64) -> WalEntry {
    delta_frame(
        key,
        OrDelta::Remove {
            tag: tag.to_string(),
        },
        sequence,
    )
}

fn prune(key: &str, tag: &str, sequence: u64) -> WalEntry {
    delta_frame(
        key,
        OrDelta::Prune {
            tags: vec![tag.to_string()],
        },
        sequence,
    )
}

/// Commits `writes` of `MAP` to the store as a write-behind flush does: each
/// with the WAL sequence it resolves, with the partition's applied row.
async fn flush(node: &FaultNode, writes: &[(&str, Option<&RecordValue>, u64)], applied: u64) {
    let writes = writes
        .iter()
        .map(|(key, value, seq)| match value {
            Some(value) => StoreWrite::Put {
                map: MAP,
                key,
                value: PutValue::Value(value),
                store_time: 0,
                seq: *seq,
            },
            None => StoreWrite::Remove {
                map: MAP,
                key,
                store_time: 0,
                seq: *seq,
            },
        })
        .collect();
    node.store
        .inner
        .commit(&FlushCommit {
            writes,
            applied: vec![(P, applied)],
        })
        .await
        .expect("a flush commit");
}

async fn version(node: &FaultNode, key: &str) -> Option<KeyVersion> {
    node.store
        .inner
        .load_version(MAP, key)
        .await
        .expect("a version read")
}

/// Reopens `root` with the pin refused, as the next start of the node.
async fn restart(node: FaultNode, root: &Root) -> FaultNode {
    node.close().await;
    let node = FaultNode::open(root);
    node.store.refuse_poisoned(Some(Refusal::Commit));
    node
}

// ---------------------------------------------------------------------------
// (a) and (b): every frame kind, replayed again and alone
// ---------------------------------------------------------------------------

/// One frame kind: the frames of key `k`, and what forward replay leaves.
struct Kind {
    name: &'static str,
    frames: Vec<WalEntry>,
    value: Option<RecordValue>,
    version: KeyVersion,
}

fn kinds(k: &str) -> Vec<Kind> {
    let live = |seq| KeyVersion {
        seq,
        removed: false,
    };
    vec![
        Kind {
            name: "LWW Store",
            frames: vec![
                store_frame(k, &lww("a", 10), 3),
                store_frame(k, &lww("b", 20), 4),
            ],
            value: Some(lww("b", 20)),
            version: live(4),
        },
        Kind {
            name: "legacy Store",
            frames: vec![legacy_frame(k, "a", 10, 3), legacy_frame(k, "b", 20, 4)],
            value: Some(RecordValue::Lww {
                value: Value::String("b".to_string()),
                timestamp: ts(20),
                ttl_ms: None,
            }),
            version: live(4),
        },
        Kind {
            name: "Remove",
            frames: vec![store_frame(k, &lww("a", 10), 3), remove_frame(k, 4)],
            value: None,
            version: KeyVersion {
                seq: 4,
                removed: true,
            },
        },
        Kind {
            name: "OR snapshot",
            frames: vec![
                store_frame(k, &or_value(&[("a", "a")], &[]), 3),
                store_frame(k, &or_value(&[("a", "a"), ("b", "b")], &["c"]), 4),
            ],
            value: Some(or_value(&[("a", "a"), ("b", "b")], &["c"])),
            version: live(4),
        },
        Kind {
            name: "OrDelta Add",
            frames: vec![add(k, "a", 3), add(k, "b", 4)],
            value: Some(or_value(&[("a", "a"), ("b", "b")], &[])),
            version: live(4),
        },
        Kind {
            name: "OrDelta Remove",
            frames: vec![add(k, "a", 3), add(k, "b", 4), or_remove(k, "a", 5)],
            value: Some(or_value(&[("b", "b")], &["a"])),
            version: live(5),
        },
        Kind {
            name: "OrDelta Prune",
            frames: vec![
                add(k, "a", 3),
                add(k, "b", 4),
                or_remove(k, "a", 5),
                prune(k, "a", 6),
            ],
            value: Some(or_value(&[("b", "b")], &[])),
            version: live(6),
        },
    ]
}

/// (a) after a full replay, replaying every frame above the pinned watermark
/// again changes no primary and no version row; (b) so does replaying any one
/// of them alone. Per frame kind.
#[tokio::test(flavor = "multi_thread")]
async fn every_frame_kind_replayed_again_or_alone_changes_no_row() {
    let (h, pin, k) = three_keys();
    for kind in kinds(&k) {
        let name = kind.name;
        let root = Root::new();
        let node = FaultNode::open(&root);
        node.append(P, &store_frame(&h, &lww("h", 1), 1)).await;
        node.append(P, &pin_frame(&pin, 2)).await;
        for frame in &kind.frames {
            node.append(P, frame).await;
        }
        let node = restart(node, &root).await;

        node.recover().await.expect("recovery");
        let above: Vec<u64> = kind.frames.iter().map(|f| f.sequence).collect();
        assert_eq!(
            (
                node.load(&k).await,
                version(&node, &k).await,
                node.engine.row(P),
                node.replayable(P).await,
            ),
            (
                kind.value.clone(),
                Some(kind.version),
                Some(1),
                [vec![2], above.clone()].concat(),
            ),
            "{name}: forward replay is the live write, stored with its version, and the pin holds \
             the watermark below every frame of the key"
        );
        let replayed = records_and_versions(&node);

        // (a) the whole window above the watermark again, through recovery.
        node.recover().await.expect("recovery again");
        assert_eq!(
            records_and_versions(&node),
            replayed,
            "{name}: replaying every frame above the watermark again changes no row"
        );

        // (b) each frame alone.
        let store = node.dyn_store();
        for frame in &kind.frames {
            WalRecovery::replay_entry(&store, frame, 0, true)
                .await
                .expect("a replay of one frame");
            assert_eq!(
                records_and_versions(&node),
                replayed,
                "{name}: replaying frame {} alone changes no row",
                frame.sequence
            );
        }
        assert_eq!(
            node.replayable(P).await,
            [vec![2], above].concat(),
            "{name}: the frames are still above the watermark, so every replay above was in it"
        );
        drop(store);
        node.close().await;
    }
}

// ---------------------------------------------------------------------------
// (c): the stale cases of the edge-case rulings
// ---------------------------------------------------------------------------

/// A store a flush left holding `k` ahead of the pinned watermark, and frames
/// of `k` below its version in the WAL: what the flush stored is what stays,
/// byte for byte.
struct Stale {
    /// `(value, seq)` flushed for `k`, in order, after `h@1` was flushed.
    flushed: Vec<(Option<RecordValue>, u64)>,
    /// Frames of `k`, every one at or below its stored version.
    frames: Vec<WalEntry>,
    value: Option<RecordValue>,
    version: KeyVersion,
}

/// Builds the node of `case`, replays it, and checks that the replay changed
/// no row and left the expected value and version. Returns the node, open.
async fn replay_stale(root: &Root, case: &Stale, name: &str) -> FaultNode {
    let (h, pin, k) = three_keys();
    let node = FaultNode::open(root);
    let hv = lww("h", 1);
    node.append(P, &store_frame(&h, &hv, 1)).await;
    node.append(P, &pin_frame(&pin, 2)).await;
    flush(&node, &[(h.as_str(), Some(&hv), 1)], 1).await;
    for (value, seq) in &case.flushed {
        flush(&node, &[(k.as_str(), value.as_ref(), *seq)], 1).await;
    }
    for frame in &case.frames {
        node.append(P, frame).await;
    }
    let node = restart(node, root).await;
    let flushed = records_and_versions(&node);

    node.recover().await.expect("recovery");
    assert_eq!(
        (
            records_and_versions(&node) == flushed,
            node.load(&k).await,
            version(&node, &k).await,
            node.engine.row(P),
        ),
        (true, case.value.clone(), Some(case.version), Some(1)),
        "{name}: the stale frames change no row"
    );
    node
}

/// A stale Remove over a newer value of the key is discarded
/// (edge case "Remove without HLC racing a newer put on replay").
#[tokio::test(flavor = "multi_thread")]
async fn stale_remove_over_a_newer_value_is_discarded() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let case = Stale {
        flushed: vec![(Some(lww("b", 5)), 5)],
        frames: vec![store_frame(&k, &lww("a", 9), 3), remove_frame(&k, 4)],
        value: Some(lww("b", 5)),
        version: KeyVersion {
            seq: 5,
            removed: false,
        },
    };
    replay_stale(&root, &case, "stale Remove")
        .await
        .close()
        .await;
}

/// A removed key's version row is its tombstone: a stale Store below it is
/// discarded instead of resurrecting the key, and a later put's frame above it
/// applies and turns the row back to a live one (edge cases "LWW tombstone vs
/// later put" and "TTL-expired record", whose sweep removes through the same
/// write path).
#[tokio::test(flavor = "multi_thread")]
async fn stale_store_over_a_removed_key_is_discarded_and_a_later_put_applies() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let expiring = RecordValue::Lww {
        value: Value::String("a".to_string()),
        timestamp: ts(9),
        ttl_ms: Some(1),
    };
    let case = Stale {
        flushed: vec![(Some(lww("a", 9)), 3), (None, 5)],
        frames: vec![store_frame(&k, &expiring, 4)],
        value: None,
        version: KeyVersion {
            seq: 5,
            removed: true,
        },
    };
    let node = replay_stale(&root, &case, "stale Store over a tombstone").await;

    node.append(P, &store_frame(&k, &lww("c", 1), 6)).await;
    let node = restart(node, &root).await;
    node.recover().await.expect("recovery");
    assert_eq!(
        (node.load(&k).await, version(&node, &k).await),
        (
            Some(lww("c", 1)),
            Some(KeyVersion {
                seq: 6,
                removed: false
            })
        ),
        "a put above the tombstone applies and its row says the key holds a record"
    );
    node.close().await;
}

/// A stale OR add is discarded, so the slot keeps its order and its bytes
/// (edge case "OR Add re-applied").
#[tokio::test(flavor = "multi_thread")]
async fn stale_or_add_is_discarded_and_keeps_the_slot_order() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let slot = or_value(&[("a", "a"), ("b", "b")], &[]);
    let case = Stale {
        flushed: vec![(Some(slot.clone()), 5)],
        frames: vec![add(&k, "a", 3)],
        value: Some(slot),
        version: KeyVersion {
            seq: 5,
            removed: false,
        },
    };
    replay_stale(&root, &case, "stale OR Add")
        .await
        .close()
        .await;
}

/// A stale OR remove of a tag whose tombstone a later prune dropped is
/// discarded instead of re-adding the tombstone (edge case "OR Remove
/// re-applied after prune").
#[tokio::test(flavor = "multi_thread")]
async fn stale_or_remove_after_a_prune_is_discarded() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let slot = or_value(&[("b", "b")], &[]);
    let case = Stale {
        flushed: vec![(Some(slot.clone()), 6)],
        frames: vec![add(&k, "a", 3), or_remove(&k, "a", 4)],
        value: Some(slot),
        version: KeyVersion {
            seq: 6,
            removed: false,
        },
    };
    replay_stale(&root, &case, "stale OR Remove after a prune")
        .await
        .close()
        .await;
}

/// A stale OR add of a tag a later remove and prune took out is discarded
/// instead of bringing the tag back (edge case "Removed tag re-added on
/// replay").
#[tokio::test(flavor = "multi_thread")]
async fn stale_or_add_of_a_removed_and_pruned_tag_is_discarded() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let slot = or_value(&[("b", "b")], &[]);
    let case = Stale {
        flushed: vec![(Some(slot.clone()), 6)],
        frames: vec![add(&k, "a", 3)],
        value: Some(slot),
        version: KeyVersion {
            seq: 6,
            removed: false,
        },
    };
    replay_stale(&root, &case, "stale OR Add of a pruned tag")
        .await
        .close()
        .await;
}

/// A stale OR snapshot is discarded instead of setting the slot back (edge
/// case "OR snapshot below store state").
#[tokio::test(flavor = "multi_thread")]
async fn stale_or_snapshot_is_discarded() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let slot = or_value(&[("a", "a"), ("b", "b")], &["c"]);
    let case = Stale {
        flushed: vec![(Some(slot.clone()), 5)],
        frames: vec![store_frame(
            &k,
            &or_value(&[("a", "a"), ("c", "c")], &[]),
            3,
        )],
        value: Some(slot),
        version: KeyVersion {
            seq: 5,
            removed: false,
        },
    };
    replay_stale(&root, &case, "stale OR snapshot")
        .await
        .close()
        .await;
}

/// Verbatim LWW writes arrive out of stamp order; the live store keeps the
/// last arrival, and so does replay, which orders by WAL sequence, not by
/// stamp (edge case "Verbatim LWW writes out of stamp order").
#[tokio::test(flavor = "multi_thread")]
async fn verbatim_lww_out_of_stamp_order_replays_to_the_last_arrival() {
    let (h, pin, k) = three_keys();
    let root = Root::new();
    let node = FaultNode::open(&root);
    node.append(P, &store_frame(&h, &lww("h", 1), 1)).await;
    node.append(P, &pin_frame(&pin, 2)).await;
    node.append(P, &store_frame(&k, &lww("stamped-late", 50), 3))
        .await;
    node.append(P, &store_frame(&k, &lww("stamped-early", 10), 4))
        .await;
    let node = restart(node, &root).await;
    node.recover().await.expect("recovery");
    let replayed = records_and_versions(&node);
    node.recover().await.expect("recovery again");
    assert_eq!(
        (
            node.load(&k).await,
            version(&node, &k).await,
            records_and_versions(&node) == replayed
        ),
        (
            Some(lww("stamped-early", 10)),
            Some(KeyVersion {
                seq: 4,
                removed: false
            }),
            true
        ),
        "replay keeps the last arrival, as the live store did, and a second replay keeps it"
    );
    node.close().await;
}

/// A version row equal to an OR add's sequence discards the add: the frame IS
/// the stored version. Applied again (a `<` gate) it would move its tag behind
/// the later one and change the slot's bytes.
#[tokio::test(flavor = "multi_thread")]
async fn equal_version_add_does_not_reorder() {
    let (_, _, k) = three_keys();
    let root = Root::new();
    let slot = or_value(&[("a", "a"), ("b", "b")], &[]);
    let case = Stale {
        flushed: vec![(Some(slot.clone()), 3)],
        frames: vec![add(&k, "a", 3)],
        value: Some(slot),
        version: KeyVersion {
            seq: 3,
            removed: false,
        },
    };
    let node = replay_stale(&root, &case, "an add at its own version").await;
    let order = match node.load(&k).await {
        Some(RecordValue::OrMap { records, .. }) => {
            records.into_iter().map(|e| e.tag).collect::<Vec<_>>()
        }
        other => panic!("an OR slot, got {other:?}"),
    };
    assert_eq!(
        order,
        ["a", "b"],
        "the later tag stays behind the earlier one"
    );
    node.close().await;
}

// ---------------------------------------------------------------------------
// The property: recovery equals the crash-free live result, and any replay
// again at or below a version changes nothing
// ---------------------------------------------------------------------------

/// The maps of the property, and its keys: key 0 takes LWW writes, keys 1 and
/// 2 take OR writes; any key can be removed whole.
const GEN_MAPS: [&str; 2] = ["pa", "pb"];

/// `(map, key)` of generated key `i`, in partition `P`.
fn gen_key(i: usize) -> (&'static str, String) {
    let map = GEN_MAPS[usize::from(i == 2)];
    let key = (0u64..10_000)
        .map(|n| format!("g{i}-{n}"))
        .find(|key| wal_partition(map, key) == P)
        .expect("a key of the partition");
    (map, key)
}

#[derive(Debug, Clone)]
enum GenOp {
    Put { millis: u64, text: u8 },
    Remove { key: usize },
    OrAdd { key: usize, tag: u8 },
    OrRemove { key: usize, tag: u8 },
    OrPrune { key: usize, tag: u8 },
    OrSnapshot { key: usize, tag: u8 },
}

impl GenOp {
    fn key(&self) -> usize {
        match self {
            Self::Put { .. } => 0,
            Self::Remove { key }
            | Self::OrAdd { key, .. }
            | Self::OrRemove { key, .. }
            | Self::OrPrune { key, .. }
            | Self::OrSnapshot { key, .. } => *key,
        }
    }
}

fn op_strategy() -> impl Strategy<Value = GenOp> {
    let or_key = 1usize..3;
    let tag = 0u8..4;
    prop_oneof![
        3 => (1u64..50, 0u8..4).prop_map(|(millis, text)| GenOp::Put { millis, text }),
        1 => (0usize..3).prop_map(|key| GenOp::Remove { key }),
        3 => (or_key.clone(), tag.clone()).prop_map(|(key, tag)| GenOp::OrAdd { key, tag }),
        2 => (or_key.clone(), tag.clone()).prop_map(|(key, tag)| GenOp::OrRemove { key, tag }),
        1 => (or_key.clone(), tag.clone()).prop_map(|(key, tag)| GenOp::OrPrune { key, tag }),
        1 => (or_key, tag).prop_map(|(key, tag)| GenOp::OrSnapshot { key, tag }),
    ]
}

/// One generated history: the ops written before the crash, after which of
/// them a flush ran, and where among them the pinning write was made.
#[derive(Debug, Clone)]
struct History {
    ops: Vec<GenOp>,
    flush_after: Vec<bool>,
    pin_at: usize,
    /// The first frame above the watermark the suffix replay starts at, as a
    /// fraction of the frames above it.
    suffix: u8,
}

fn history_strategy() -> impl Strategy<Value = History> {
    proptest::collection::vec((op_strategy(), proptest::bool::weighted(0.3)), 1..24)
        .prop_flat_map(|ops| {
            let n = ops.len();
            (Just(ops), 0..=n, any::<u8>())
        })
        .prop_map(|(ops, pin_at, suffix)| {
            let (ops, flush_after) = ops.into_iter().unzip();
            History {
                ops,
                flush_after,
                pin_at,
                suffix,
            }
        })
}

/// The OR value the live write path leaves after `delta` over `current`.
fn live_or(current: Option<&RecordValue>, delta: &OrDelta) -> RecordValue {
    let mut value = current.cloned().unwrap_or(RecordValue::OrMap {
        records: Vec::new(),
        tombstones: Vec::new(),
    });
    normalize_to_or_map(&mut value);
    apply_or_delta(delta.clone(), &mut value);
    value
}

/// An OR slot as sets of live `(tag, value)` and of tombstones; an absent
/// slot is the empty one.
type OrView = (BTreeSet<(String, String)>, BTreeSet<String>);

fn or_view(value: Option<&RecordValue>) -> Option<OrView> {
    match value {
        None => Some((BTreeSet::new(), BTreeSet::new())),
        Some(RecordValue::OrMap {
            records,
            tombstones,
        }) => Some((
            records
                .iter()
                .map(|e| (e.tag.clone(), format!("{:?}", e.value)))
                .collect(),
            tombstones.iter().cloned().collect(),
        )),
        Some(_) => None,
    }
}

/// One flush: `(key, value, seq)` writes and the applied watermark it carries.
type Flush = (Vec<(usize, Option<RecordValue>, u64)>, u64);

/// The frames of `history` with their sequences, the live value of every
/// generated key after them, and the flushes the write-behind store would have
/// committed.
struct Lived {
    frames: Vec<(usize, WalEntry)>,
    pin_seq: u64,
    live: Vec<Option<RecordValue>>,
    flushes: Vec<Flush>,
}

fn live_run(history: &History) -> Lived {
    let mut live: Vec<Option<RecordValue>> = vec![None; 3];
    let mut frames = Vec::new();
    let mut flushes = Vec::new();
    let mut last_seq = [0u64; 3];
    let mut dirty = BTreeSet::new();
    // Every sequence a flush has resolved; the pin is never resolved.
    let mut resolved = BTreeSet::new();
    let mut frames_of: [Vec<u64>; 3] = Default::default();
    let mut watermark = 0u64;
    let mut seq = 0u64;
    let mut pin_seq = 0u64;
    for (i, op) in history.ops.iter().enumerate() {
        if i == history.pin_at {
            seq += 1;
            pin_seq = seq;
        }
        seq += 1;
        let k = op.key();
        let (map, key) = gen_key(k);
        // Delta frames come from the WAL module's own `delta_frame`: the delta
        // variant is constructed only in its sanctioned homes (`TG-OR-003`).
        let (op_frame, value) = match op {
            GenOp::Put { millis, text } => {
                let value = lww(&format!("v{text}"), *millis);
                (
                    WalOp::Store {
                        value: WalStorePayload::Record(value.clone()),
                    },
                    Some(value),
                )
            }
            GenOp::Remove { .. } => (WalOp::Remove, None),
            GenOp::OrAdd { tag, .. } => {
                let delta = OrDelta::Add {
                    entry: entry(&format!("t{tag}"), &format!("v{tag}")),
                };
                let value = live_or(live[k].as_ref(), &delta);
                (delta_frame(&key, delta, seq).op, Some(value))
            }
            GenOp::OrRemove { tag, .. } => {
                let delta = OrDelta::Remove {
                    tag: format!("t{tag}"),
                };
                let value = live_or(live[k].as_ref(), &delta);
                (delta_frame(&key, delta, seq).op, Some(value))
            }
            GenOp::OrPrune { tag, .. } => {
                let delta = OrDelta::Prune {
                    tags: vec![format!("t{tag}")],
                };
                let value = live_or(live[k].as_ref(), &delta);
                (delta_frame(&key, delta, seq).op, Some(value))
            }
            GenOp::OrSnapshot { tag, .. } => {
                let add = OrDelta::Add {
                    entry: entry(&format!("t{tag}"), &format!("s{seq}")),
                };
                let value = live_or(live[k].as_ref(), &add);
                (
                    WalOp::Store {
                        value: WalStorePayload::Record(value.clone()),
                    },
                    Some(value),
                )
            }
        };
        let timestamp = match &value {
            Some(RecordValue::Lww { timestamp, .. }) if matches!(op, GenOp::Put { .. }) => {
                Some(timestamp.clone())
            }
            _ => None,
        };
        frames.push((
            k,
            WalEntry {
                map: map.to_string(),
                key,
                op: op_frame,
                timestamp,
                sequence: seq,
            },
        ));
        live[k] = value;
        last_seq[k] = seq;
        frames_of[k].push(seq);
        dirty.insert(k);

        if history.flush_after[i] {
            let mut writes = Vec::new();
            for k in std::mem::take(&mut dirty) {
                writes.push((k, live[k].clone(), last_seq[k]));
                resolved.extend(frames_of[k].iter().copied());
            }
            let next = (1..=seq)
                .take_while(|s| resolved.contains(s))
                .last()
                .unwrap_or(0);
            watermark = watermark.max(next);
            flushes.push((writes, watermark));
        }
    }
    if history.pin_at == history.ops.len() {
        seq += 1;
        pin_seq = seq;
    }
    Lived {
        frames,
        pin_seq,
        live,
        flushes,
    }
}

async fn gen_load(node: &FaultNode, k: usize) -> Option<RecordValue> {
    let (map, key) = gen_key(k);
    node.store.inner.load(map, &key).await.expect("load")
}

async fn gen_version(node: &FaultNode, k: usize) -> Option<KeyVersion> {
    let (map, key) = gen_key(k);
    node.store
        .inner
        .load_version(map, &key)
        .await
        .expect("a version read")
}

/// Writes `history` as the live node would have before its crash, recovers,
/// and checks the property; `Err` names what failed.
async fn check_history(history: &History) -> Result<(), String> {
    let lived = live_run(history);
    let (_, pin, _) = three_keys();
    let root = Root::new();
    let node = FaultNode::open(&root);
    let mut flushes = lived.flushes.iter();
    let mut flush_points = history.flush_after.iter();
    for (_, frame) in &lived.frames {
        if frame.sequence == lived.pin_seq + 1 {
            node.append(P, &pin_frame(&pin, lived.pin_seq)).await;
        }
        node.append(P, frame).await;
        if *flush_points.next().expect("one flag per op") {
            let (writes, applied) = flushes.next().expect("a flush per flag");
            let gen_keys: Vec<_> = writes.iter().map(|(k, _, _)| gen_key(*k)).collect();
            let writes = writes
                .iter()
                .zip(&gen_keys)
                .map(|((_, value, seq), (map, key))| match value {
                    Some(value) => StoreWrite::Put {
                        map,
                        key,
                        value: PutValue::Value(value),
                        store_time: 0,
                        seq: *seq,
                    },
                    None => StoreWrite::Remove {
                        map,
                        key,
                        store_time: 0,
                        seq: *seq,
                    },
                })
                .collect();
            node.store
                .inner
                .commit(&FlushCommit {
                    writes,
                    applied: if *applied > 0 {
                        vec![(P, *applied)]
                    } else {
                        Vec::new()
                    },
                })
                .await
                .map_err(|e| format!("a flush commit failed: {e:#}"))?;
        }
    }
    if lived.pin_seq > lived.frames.last().map_or(0, |(_, f)| f.sequence) {
        node.append(P, &pin_frame(&pin, lived.pin_seq)).await;
    }
    let node = restart(node, &root).await;
    node.recover()
        .await
        .map_err(|e| format!("recovery failed: {e:#}"))?;

    // Recovery equals the live result: LWW byte for byte, OR by its sets.
    for k in 0..3 {
        let recovered = gen_load(&node, k).await;
        let expected = &lived.live[k];
        let equal = match (expected, &recovered) {
            (Some(RecordValue::OrMap { .. }), _) | (None, Some(RecordValue::OrMap { .. })) => {
                or_view(expected.as_ref()) == or_view(recovered.as_ref())
            }
            _ => {
                expected
                    .as_ref()
                    .map(rmp_serde::to_vec_named)
                    .transpose()
                    .ok()
                    == recovered
                        .as_ref()
                        .map(rmp_serde::to_vec_named)
                        .transpose()
                        .ok()
            }
        };
        if !equal {
            return Err(format!(
                "key {k}: recovered {recovered:?}, the live write left {expected:?}"
            ));
        }
    }

    let replayed = records_and_versions(&node);
    let store = node.dyn_store();
    let above: Vec<&WalEntry> = {
        let replayable = node.replayable(P).await;
        lived
            .frames
            .iter()
            .map(|(_, f)| f)
            .filter(|f| replayable.contains(&f.sequence))
            .collect()
    };

    // The whole window again, through recovery.
    node.recover()
        .await
        .map_err(|e| format!("recovery again failed: {e:#}"))?;
    if records_and_versions(&node) != replayed {
        return Err("a second recovery changed a row".to_string());
    }

    // A suffix of the frames above the watermark, frame by frame.
    let start = above.len() * usize::from(history.suffix) / 256;
    for frame in &above[start..] {
        WalRecovery::replay_entry(&store, frame, 0, true)
            .await
            .map_err(|e| format!("a replay of frame {}: {e:#}", frame.sequence))?;
        if records_and_versions(&node) != replayed {
            return Err(format!(
                "replaying the suffix from frame {} changed a row at frame {}",
                above[start].sequence, frame.sequence
            ));
        }
    }

    // Every frame at or below its key's version, alone.
    for k in 0..3 {
        let Some(version) = gen_version(&node, k).await else {
            continue;
        };
        for (_, frame) in lived
            .frames
            .iter()
            .filter(|(fk, f)| *fk == k && f.sequence <= version.seq)
        {
            WalRecovery::replay_entry(&store, frame, 0, true)
                .await
                .map_err(|e| format!("a replay of frame {}: {e:#}", frame.sequence))?;
            if records_and_versions(&node) != replayed {
                return Err(format!(
                    "frame {} at or below its key's version {} changed a row",
                    frame.sequence, version.seq
                ));
            }
        }
    }
    drop(store);
    node.close().await;
    Ok(())
}

/// Random histories over 3 keys of 2 maps — LWW puts and removes, OR adds,
/// removes, prunes and snapshots — with random flush points, a random pin
/// and a crash after the last op: recovery equals the crash-free live result,
/// and replaying any suffix of the frames above the watermark, or any frame
/// at or below its key's version, afterwards changes no row.
#[test]
fn recovery_equals_live_and_replaying_again_changes_no_row() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mut runner = TestRunner::new(ProptestConfig {
        cases: 96,
        failure_persistence: None,
        max_shrink_iters: 128,
        ..ProptestConfig::default()
    });
    runner
        .run(&history_strategy(), |history| {
            let result = runtime.block_on(check_history(&history));
            prop_assert!(result.is_ok(), "{}", result.unwrap_err());
            Ok(())
        })
        .expect("recovery equals the live result and replaying again changes no row");
}
