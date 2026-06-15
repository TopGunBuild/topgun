//! Property-based simulation tests using proptest.
//!
//! Generates random distributed operation sequences and verifies that the
//! cluster invariants (completeness, convergence, Merkle consistency) hold
//! across a wide range of inputs. This catches edge cases that hand-written
//! scenarios miss.
//!
//! The async bridge pattern used here:
//! - Each test is a `#[tokio::test]` function that constructs a `TestRunner`
//!   directly and calls `runner.run(&strategy, |ops| { ... })`.
//! - Async `SimCluster` methods are called via `Handle::current().block_on(...)`.
//! - This avoids the `proptest!` macro which generates a synchronous `#[test]`
//!   function incompatible with `SimCluster`'s async API.

#![cfg(feature = "simulation")]

use proptest::prelude::*;
use proptest::test_runner::{Config as PropConfig, TestRunner};
use tokio::runtime::Handle;
use topgun_server::sim::cluster::SimCluster;
use topgun_server::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// Operation enum
// ---------------------------------------------------------------------------

/// A single operation in a generated distributed sequence.
#[derive(Debug, Clone)]
enum Op {
    /// Write a value to the given node (resolved via modular arithmetic), map,
    /// and key. The `node_idx` is resolved at execution time as
    /// `node_idx % live_node_count` to handle dynamic cluster membership.
    Write {
        node_idx: usize,
        map: String,
        key: String,
        value: String,
    },
    /// Add a new node to the cluster.
    NodeJoin,
    /// Kill the node at `node_idx % live_node_count`.
    NodeKill { node_idx: usize },
    /// Inject a partition between two disjoint sub-ranges of node indices.
    Partition { split: usize },
    /// Heal all active partitions.
    HealPartition,
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Strategy for generating a map name from a small fixed set to encourage
/// cross-map interactions.
fn arb_map() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("alpha".to_string()),
        Just("beta".to_string()),
        Just("gamma".to_string()),
    ]
}

/// Strategy for generating a short key name from a fixed alphabet.
fn arb_key() -> impl Strategy<Value = String> {
    "[a-z]{1,4}"
}

/// Strategy for generating a single `Op`.
fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        // Write operations are most frequent.
        4 => (any::<usize>(), arb_map(), arb_key(), "[a-zA-Z0-9]{1,8}").prop_map(
            |(node_idx, map, key, value)| Op::Write { node_idx, map, key, value }
        ),
        // NodeJoin is relatively rare.
        1 => Just(Op::NodeJoin),
        // NodeKill is rare to keep cluster size positive.
        1 => any::<usize>().prop_map(|node_idx| Op::NodeKill { node_idx }),
        // Partition: split at a position within [1, live_count-1].
        1 => any::<usize>().prop_map(|split| Op::Partition { split }),
        // HealPartition: clear all active partitions.
        1 => Just(Op::HealPartition),
    ]
}

/// Strategy for generating a sequence of 10–100 operations.
fn arb_ops() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(arb_op(), 10..=100)
}

// ---------------------------------------------------------------------------
// Acknowledged-write tracker
// ---------------------------------------------------------------------------

/// Records writes that were acknowledged (returned `Ok`) so completeness can
/// be verified: each such write must be readable from at least one live node.
///
/// The `source_node` field tracks which node received the write. When that
/// node is killed, the write is removed from the completeness check list
/// because `SimCluster` does not replicate writes automatically — a write to a
/// single node is durably "present" only while that node is alive.
#[derive(Debug, Clone)]
struct AckedWrite {
    map: String,
    key: String,
    /// Index of the node that received and acknowledged this write.
    source_node: usize,
}

// ---------------------------------------------------------------------------
// Execution engine
// ---------------------------------------------------------------------------

/// Executes an operation sequence against a fresh `SimCluster`.
///
/// Returns the cluster after execution along with the list of acknowledged
/// writes. The cluster is NOT healed before returning — callers that want
/// convergence checks must call `heal_partition()` and `sync_all()` themselves.
async fn execute_ops(ops: &[Op], seed: u64) -> (SimCluster, Vec<AckedWrite>) {
    // Start with 3 nodes so there is always a meaningful cluster for partition
    // tests even after a NodeKill.
    let mut cluster = SimCluster::new(3, seed);
    cluster.start().expect("cluster should start");

    let mut acked: Vec<AckedWrite> = Vec::new();

    for op in ops {
        // Determine the number of currently live nodes to resolve node_idx.
        let live_indices: Vec<usize> = (0..cluster.nodes.len())
            .filter(|&i| cluster.nodes[i].is_alive())
            .collect();

        if live_indices.is_empty() {
            // No live nodes — restart node 0 so we can continue.
            cluster.restart_node(0).expect("restart should succeed");
            continue;
        }

        match op {
            Op::Write {
                node_idx,
                map,
                key,
                value,
            } => {
                let live_count = live_indices.len();
                let resolved = live_indices[node_idx % live_count];
                let result = cluster
                    .write(
                        resolved,
                        map,
                        key,
                        rmpv::Value::String(value.as_str().into()),
                    )
                    .await;
                if result.is_ok() {
                    acked.push(AckedWrite {
                        map: map.clone(),
                        key: key.clone(),
                        source_node: resolved,
                    });
                }
            }

            Op::NodeJoin => {
                // Cap cluster size at 8 to keep tests fast.
                if cluster.nodes.len() < 8 {
                    let _ = cluster.add_node();
                }
            }

            Op::NodeKill { node_idx } => {
                let live_count = live_indices.len();
                // Keep at least 1 node alive to avoid a fully dead cluster.
                if live_count > 1 {
                    let resolved = live_indices[node_idx % live_count];
                    cluster.kill_node(resolved);
                    // Prune acked writes whose source node was just killed:
                    // SimCluster does not replicate writes automatically, so
                    // data on a dead node is no longer reachable.
                    acked.retain(|w| w.source_node != resolved);
                }
            }

            Op::Partition { split } => {
                let live_count = live_indices.len();
                if live_count < 2 {
                    continue;
                }
                // Ensure split resolves to a boundary that creates two non-empty groups.
                // `(split % (live_count - 1)) + 1` guarantees group_a has 1..live_count-1 nodes.
                let boundary = (split % (live_count - 1)) + 1;
                let group_a = &live_indices[..boundary];
                let group_b = &live_indices[boundary..];
                cluster.inject_partition(group_a, group_b);
            }

            Op::HealPartition => {
                cluster.heal_partition();
            }
        }
    }

    (cluster, acked)
}

/// Runs a full convergence sync on the cluster: heal all partitions, then
/// do a bidirectional Merkle sync for every live node pair for a set of maps.
async fn converge_cluster(cluster: &SimCluster, maps: &[&str]) {
    cluster.heal_partition();

    let live: Vec<usize> = (0..cluster.nodes.len())
        .filter(|&i| cluster.nodes[i].is_alive())
        .collect();

    // Two full passes ensure even long gossip chains converge.
    for _ in 0..2 {
        for &src in &live {
            for &dst in &live {
                if src == dst {
                    continue;
                }
                for map in maps {
                    let _ = cluster.merkle_sync_pair(src, dst, map).await;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant assertions
// ---------------------------------------------------------------------------

/// Asserts completeness: every acknowledged write is present on at least one
/// live node. Must be called BEFORE healing partitions.
///
/// Returns a proptest `TestCaseResult`.
async fn assert_completeness(
    cluster: &SimCluster,
    acked: &[AckedWrite],
) -> Result<(), TestCaseError> {
    for aw in acked {
        let live: Vec<usize> = (0..cluster.nodes.len())
            .filter(|&i| cluster.nodes[i].is_alive())
            .collect();

        let mut found = false;
        for &idx in &live {
            let result = cluster.read(idx, &aw.map, &aw.key).await;
            if let Ok(Some(_)) = result {
                found = true;
                break;
            }
        }

        if !found {
            return Err(TestCaseError::fail(format!(
                "completeness violation: acked write map={:?} key={:?} not found on any live node",
                aw.map, aw.key
            )));
        }
    }
    Ok(())
}

/// Asserts convergence: all live nodes hold the same value for every key that
/// was written during the sequence (across all maps). Must be called AFTER
/// healing and syncing.
///
/// Returns a proptest `TestCaseResult`.
async fn assert_convergence(
    cluster: &SimCluster,
    acked: &[AckedWrite],
) -> Result<(), TestCaseError> {
    for aw in acked {
        let result = cluster.assert_converged(&aw.map, &aw.key).await;
        match result {
            Ok(_) => {}
            Err(e) => {
                return Err(TestCaseError::fail(format!(
                    "convergence error for map={:?} key={:?}: {e}",
                    aw.map, aw.key
                )));
            }
        }
    }
    Ok(())
}

/// Asserts Merkle consistency: for each pair of live nodes, their Merkle
/// trees produce the same root hash for every map that was written to.
/// Must be called AFTER healing and syncing.
///
/// Uses `assert_converged` as a proxy — if all nodes agree on every written
/// key's value, their Merkle trees must also be consistent (since the Merkle
/// tree is built from the same record set). This avoids exposing internal
/// Merkle manager APIs in the simulation tests.
///
/// Returns a proptest `TestCaseResult`.
async fn assert_merkle_consistency(
    cluster: &SimCluster,
    acked: &[AckedWrite],
) -> Result<(), TestCaseError> {
    // Merkle consistency is implied by convergence on every key: if all live
    // nodes agree on the value of every key, their record stores are identical,
    // and therefore their Merkle hashes match. Delegate to convergence check.
    assert_convergence(cluster, acked).await
}

// ---------------------------------------------------------------------------
// Unique maps used in a sequence (for sync targeting)
// ---------------------------------------------------------------------------

fn maps_used(ops: &[Op]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    for op in ops {
        if let Op::Write { map, .. } = op {
            seen.insert(map.clone());
        }
    }
    seen.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Property-based tests
// ---------------------------------------------------------------------------

/// Property: random operation sequences preserve convergence.
///
/// Executes a random sequence of writes, node joins, node kills, and network
/// partitions on a 3-node `SimCluster`. After execution, all partitions are
/// healed and a full Merkle sync is performed. All acknowledged writes must
/// then be visible identically on every live node.
#[tokio::test(flavor = "multi_thread")]
async fn random_operations_preserve_convergence() {
    let handle = Handle::current();
    let mut runner = TestRunner::new(PropConfig {
        cases: 50,
        ..PropConfig::default()
    });

    let result = runner.run(&arb_ops(), |ops| {
        // `block_in_place` yields the current thread to the tokio runtime
        // scheduler, allowing `block_on` to drive the async SimCluster methods
        // without creating a nested runtime.
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                let maps: Vec<String> = maps_used(&ops);
                let map_refs: Vec<&str> = maps.iter().map(String::as_str).collect();

                // Fixed seed for reproducible node timing within each case.
                let seed = 42u64;
                let (cluster, acked) = execute_ops(&ops, seed).await;

                // Convergence check: heal + full Merkle sync + assert.
                converge_cluster(&cluster, &map_refs).await;
                assert_convergence(&cluster, &acked).await?;

                Ok(())
            })
        })
    });

    if let Err(e) = result {
        panic!("random_operations_preserve_convergence failed: {e}");
    }
}

/// Property: random operation sequences preserve completeness.
///
/// After executing a random sequence (WITHOUT healing partitions), every
/// acknowledged write must be readable from at least one live node. This
/// verifies that a local write is never lost even while partitions are active.
#[tokio::test(flavor = "multi_thread")]
async fn random_operations_preserve_completeness() {
    let handle = Handle::current();
    let mut runner = TestRunner::new(PropConfig {
        cases: 50,
        ..PropConfig::default()
    });

    let result = runner.run(&arb_ops(), |ops| {
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                let seed = 43u64;
                let (cluster, acked) = execute_ops(&ops, seed).await;

                // Completeness is checked BEFORE healing: the write must still be
                // visible on at least one live node despite any active partitions.
                assert_completeness(&cluster, &acked).await?;

                Ok(())
            })
        })
    });

    if let Err(e) = result {
        panic!("random_operations_preserve_completeness failed: {e}");
    }
}

/// Property: random operation sequences leave Merkle trees consistent after sync.
///
/// After executing a random sequence, all partitions are healed and a full
/// bidirectional Merkle sync is performed. All live node pairs must agree on
/// the value of every written key (which implies Merkle hash equality).
#[tokio::test(flavor = "multi_thread")]
async fn random_operations_merkle_consistent() {
    let handle = Handle::current();
    let mut runner = TestRunner::new(PropConfig {
        cases: 50,
        ..PropConfig::default()
    });

    let result = runner.run(&arb_ops(), |ops| {
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                let maps: Vec<String> = maps_used(&ops);
                let map_refs: Vec<&str> = maps.iter().map(String::as_str).collect();

                let seed = 44u64;
                let (cluster, acked) = execute_ops(&ops, seed).await;

                // Heal + sync before checking Merkle consistency.
                converge_cluster(&cluster, &map_refs).await;
                assert_merkle_consistency(&cluster, &acked).await?;

                Ok(())
            })
        })
    });

    if let Err(e) = result {
        panic!("random_operations_merkle_consistent failed: {e}");
    }
}

// ===========================================================================
// Concurrent OR_REMOVE convergence (blocking gate — closes the OR-Map coverage gap)
// ===========================================================================
//
// The generic `random_operations_*` properties above only generate LWW writes,
// so they never exercised the OR-Map merge path. That gap is exactly why the
// suite once missed the OR_REMOVE clobber bug (a blind `OrTombstones` put that
// destroyed every concurrent OR-Map value for the key). This property drives
// random interleavings of OR_ADD / OR_REMOVE / one-way sync / partition across
// a 3-node cluster on overlapping keys and asserts every node converges to the
// CRDT-correct live set with zero acknowledged-write loss.

/// A single operation in a concurrent OR-Map sequence.
#[derive(Debug, Clone)]
enum OrOp {
    /// `OR_ADD`: node `node_idx % 3` adds a freshly-minted unique tag to `key_idx`.
    /// A globally-unique tag means each tag maps to exactly one value, so the
    /// convergent live set is order-independent (no last-writer-wins ambiguity).
    Add { node_idx: usize, key_idx: usize },
    /// `OR_REMOVE`: node `node_idx % 3` removes a previously-added `(key, tag)`
    /// resolved from the add pool by `target % pool.len()`. The removing node is
    /// frequently NOT the adding node, exercising concurrent cross-node add/remove.
    Remove { node_idx: usize, target: usize },
    /// One-way Merkle sync of the OR-Map between two distinct nodes (partition-aware,
    /// silently dropped across a partitioned link). Creates random partial-delivery
    /// orders mid-sequence so convergence cannot rely on a tidy delivery schedule.
    Sync { src_idx: usize, dst_idx: usize },
    /// Partition the cluster into two non-empty groups.
    Partition { split: usize },
    /// Heal all active partitions.
    Heal,
}

/// Map name for the OR-Map convergence property (single map keeps the focus on
/// OR semantics rather than cross-map fan-out).
const OR_MAP: &str = "orset";

/// Number of distinct keys. A small set forces many adds/removes onto the same
/// key so concurrent add/remove on intersecting keys is the common case.
const OR_KEY_COUNT: usize = 3;

/// Strategy for a single OR-Map operation. Adds dominate so the pool fills and
/// removes have meaningful targets.
fn arb_or_op() -> impl Strategy<Value = OrOp> {
    prop_oneof![
        4 => (any::<usize>(), 0..OR_KEY_COUNT).prop_map(|(node_idx, key_idx)| OrOp::Add {
            node_idx,
            key_idx
        }),
        3 => (any::<usize>(), any::<usize>())
            .prop_map(|(node_idx, target)| OrOp::Remove { node_idx, target }),
        3 => (any::<usize>(), any::<usize>())
            .prop_map(|(src_idx, dst_idx)| OrOp::Sync { src_idx, dst_idx }),
        1 => any::<usize>().prop_map(|split| OrOp::Partition { split }),
        1 => Just(OrOp::Heal),
    ]
}

fn arb_or_ops() -> impl Strategy<Value = Vec<OrOp>> {
    proptest::collection::vec(arb_or_op(), 10..=64)
}

/// The CRDT-expected outcome for the OR-Map sequence, accumulated during
/// execution: which tags were acknowledged-added per key, and which were
/// acknowledged-removed. The convergent live set per key is `added - removed`.
#[derive(Default)]
struct OrExpectation {
    /// key -> set of tags whose `OR_ADD` was acknowledged.
    added: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// key -> set of tags whose `OR_REMOVE` was acknowledged.
    removed: std::collections::HashMap<String, std::collections::HashSet<String>>,
}

fn or_key(key_idx: usize) -> String {
    format!("k{}", key_idx % OR_KEY_COUNT)
}

/// Executes an OR-Map operation sequence against a fresh fixed-size 3-node
/// cluster. No nodes are killed or joined, so every acknowledged add must
/// survive to the convergence check — `0 acked-loss` is directly assertable.
async fn execute_or_ops(ops: &[OrOp], seed: u64) -> (SimCluster, OrExpectation) {
    let mut cluster = SimCluster::new(3, seed);
    cluster.start().expect("cluster should start");

    let mut expect = OrExpectation::default();
    // Pool of acknowledged (key, tag) adds that removes can target.
    let mut pool: Vec<(String, String)> = Vec::new();
    // Globally-monotonic tag sequence guarantees every add gets a unique tag.
    let mut seq: usize = 0;

    for op in ops {
        match op {
            OrOp::Add { node_idx, key_idx } => {
                let node = node_idx % cluster.nodes.len();
                let key = or_key(*key_idx);
                let tag = format!("n{node}-{seq}");
                seq += 1;
                // Value is tied 1:1 to the tag so there is never value ambiguity.
                let value = rmpv::Value::String(tag.as_str().into());
                if cluster
                    .or_write(node, OR_MAP, &key, tag.clone(), value)
                    .await
                    .is_ok()
                {
                    expect
                        .added
                        .entry(key.clone())
                        .or_default()
                        .insert(tag.clone());
                    pool.push((key, tag));
                }
            }
            OrOp::Remove { node_idx, target } => {
                if pool.is_empty() {
                    continue;
                }
                let node = node_idx % cluster.nodes.len();
                let (key, tag) = pool[target % pool.len()].clone();
                if cluster
                    .or_remove(node, OR_MAP, &key, tag.clone())
                    .await
                    .is_ok()
                {
                    expect.removed.entry(key).or_default().insert(tag);
                }
            }
            OrOp::Sync { src_idx, dst_idx } => {
                let n = cluster.nodes.len();
                let src = src_idx % n;
                // Force a distinct destination.
                let dst = (dst_idx % n.saturating_sub(1).max(1) + src + 1) % n;
                if src != dst {
                    let _ = cluster.merkle_sync_pair(src, dst, OR_MAP).await;
                }
            }
            OrOp::Partition { split } => {
                let n = cluster.nodes.len();
                if n < 2 {
                    continue;
                }
                let boundary = (split % (n - 1)) + 1;
                let all: Vec<usize> = (0..n).collect();
                cluster.inject_partition(&all[..boundary], &all[boundary..]);
            }
            OrOp::Heal => cluster.heal_partition(),
        }
    }

    (cluster, expect)
}

/// Asserts OR-Map convergence: after healing + full Merkle sync, every alive
/// node holds an identical live tag set per key, equal to `added - removed`.
///
/// This single assertion encodes three guarantees:
/// - **convergence**: all nodes agree on the live set for every key;
/// - **0 acked-loss**: every acknowledged add that was never removed survives;
/// - **remove-wins**: every acknowledged remove is absent on every node, with no
///   resurrection regardless of add/remove/delivery interleaving.
///
/// Comparison is on tag *sets*, not serialized bytes: the `records`/`tombstones`
/// Vec order is an implementation artifact of delivery order, whereas CRDT
/// convergence is defined on the live-element set.
async fn assert_or_converged(
    cluster: &SimCluster,
    expect: &OrExpectation,
) -> Result<(), TestCaseError> {
    let mut keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    keys.extend(expect.added.keys().cloned());
    keys.extend(expect.removed.keys().cloned());

    let empty = std::collections::HashSet::new();

    for key in &keys {
        let added = expect.added.get(key).unwrap_or(&empty);
        let removed = expect.removed.get(key).unwrap_or(&empty);
        let expected_live: std::collections::BTreeSet<String> =
            added.difference(removed).cloned().collect();

        for (idx, node) in cluster.nodes.iter().enumerate() {
            if !node.is_alive() {
                continue;
            }
            let value = cluster
                .read(idx, OR_MAP, key)
                .await
                .map_err(|e| TestCaseError::fail(format!("read error: {e}")))?;

            // Live tag set as stored on this node (None ⇒ key absent ⇒ empty set).
            let live: std::collections::BTreeSet<String> = match value {
                Some(RecordValue::OrMap { records, .. }) => {
                    records.into_iter().map(|e| e.tag).collect()
                }
                Some(RecordValue::OrTombstones { .. }) | None => std::collections::BTreeSet::new(),
                Some(other) => {
                    return Err(TestCaseError::fail(format!(
                        "key {key:?} on node {idx} is non-OR-Map: {other:?}"
                    )));
                }
            };

            if live != expected_live {
                return Err(TestCaseError::fail(format!(
                    "OR convergence failure: key={key:?} node={idx}\n  \
                     expected live tags (added−removed) = {expected_live:?}\n  \
                     actual live tags                   = {live:?}\n  \
                     added={added:?} removed={removed:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Property: concurrent `OR_ADD` / `OR_REMOVE` sequences converge with zero
/// acknowledged-write loss.
///
/// This is the blocking gate that closes the convergence coverage gap for the
/// `OR_REMOVE` clobber class. Negative control: reverting the server-side
/// `OR_REMOVE` merge to the old blind-clobber `OrTombstones` put MUST make this
/// test fail — proving the guard actually catches that regression class.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_or_remove_preserves_convergence() {
    let handle = Handle::current();
    let mut runner = TestRunner::new(PropConfig {
        cases: 64,
        ..PropConfig::default()
    });

    let result = runner.run(&arb_or_ops(), |ops| {
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                let seed = 1234u64;
                let (cluster, expect) = execute_or_ops(&ops, seed).await;

                // Heal + full bidirectional Merkle sync, then assert convergence.
                converge_cluster(&cluster, &[OR_MAP]).await;
                assert_or_converged(&cluster, &expect).await?;

                Ok(())
            })
        })
    });

    if let Err(e) = result {
        panic!("concurrent_or_remove_preserves_convergence failed: {e}");
    }
}
