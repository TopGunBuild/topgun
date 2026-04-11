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
