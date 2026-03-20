use topgun_server::sim::cluster::SimCluster;
use topgun_server::storage::record::{OrMapEntry, RecordValue};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Asserts that a `RecordValue` is an `OrMap` and that all `expected_tags`
/// are present in its entry set. Panics with a descriptive message otherwise.
fn assert_or_tags(value: &RecordValue, expected_tags: &[&str]) {
    match value {
        RecordValue::OrMap { records } => {
            let present: Vec<&str> = records.iter().map(|e: &OrMapEntry| e.tag.as_str()).collect();
            for tag in expected_tags {
                assert!(
                    present.contains(tag),
                    "expected tag {tag:?} to be present in OR-Map, found: {present:?}",
                );
            }
        }
        other => panic!("expected OrMap record, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Three nodes each write a different value to the same key. After pairwise
/// Merkle sync the cluster must converge to the LWW winner (highest HLC
/// timestamp determines winner; with equal millis/counter the node_id
/// tiebreaker applies: "sim-node-2" > "sim-node-1" > "sim-node-0").
#[tokio::test]
async fn concurrent_writes_converge() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start");

    cluster
        .write(0, "data", "shared-key", rmpv::Value::String("from-node-0".into()))
        .await
        .expect("write on node 0 should succeed");

    cluster
        .write(1, "data", "shared-key", rmpv::Value::String("from-node-1".into()))
        .await
        .expect("write on node 1 should succeed");

    cluster
        .write(2, "data", "shared-key", rmpv::Value::String("from-node-2".into()))
        .await
        .expect("write on node 2 should succeed");

    // Merkle sync propagates the LWW winner to every node.
    // Node 2 has the highest-timestamp value (node_id "sim-node-2" wins the
    // tiebreaker), so syncing from node 2 to others propagates that value.
    // Syncing from node 2 to nodes 0 and 1 (and a second pass for safety):
    cluster
        .merkle_sync_pair(2, 0, "data")
        .await
        .expect("merkle sync 2→0 should succeed");
    cluster
        .merkle_sync_pair(2, 1, "data")
        .await
        .expect("merkle sync 2→1 should succeed");
    // Second round catches any remaining divergence.
    cluster
        .merkle_sync_pair(2, 0, "data")
        .await
        .expect("second merkle sync 2→0 should succeed");
    cluster
        .merkle_sync_pair(2, 1, "data")
        .await
        .expect("second merkle sync 2→1 should succeed");

    let converged = cluster
        .assert_converged("data", "shared-key")
        .await
        .expect("assert_converged should not error");

    assert!(
        converged.is_some(),
        "all nodes should hold a value after gossip"
    );

    // assert_converged panics if any two alive nodes disagree on the value.
}

/// Two nodes each OR-write a different tag to the same key. After sync both
/// nodes must hold both tags (OR-Map add-wins semantics).
///
/// Uses `sync_all` after the first write to propagate tag-A to node 1, then
/// `merkle_sync_pair` for full OR-Map convergence because `sync_all` only
/// forwards the first entry per key.
#[tokio::test]
async fn ormap_concurrent_add_remove() {
    let mut cluster = SimCluster::new(2, 7);
    cluster.start().expect("cluster should start");

    let value_a = rmpv::Value::String("alpha".into());
    let value_b = rmpv::Value::String("beta".into());

    // Write tag-A on node 0, then sync so node 1 receives it.
    cluster
        .or_write(0, "ormap", "item", "tag-A", value_a)
        .await
        .expect("or_write tag-A should succeed");
    cluster
        .sync_all("ormap", "item")
        .await
        .expect("sync after tag-A should succeed");

    // Write tag-B on node 1 (which now also holds tag-A after the above sync).
    cluster
        .or_write(1, "ormap", "item", "tag-B", value_b)
        .await
        .expect("or_write tag-B should succeed");

    // Full OR-Map Merkle sync to propagate all entries across both directions.
    // merkle_sync_pair delivers every entry in the source OR-Map, not just the first.
    cluster
        .merkle_sync_pair(1, 0, "ormap")
        .await
        .expect("merkle sync 1→0 should succeed");
    cluster
        .merkle_sync_pair(0, 1, "ormap")
        .await
        .expect("merkle sync 0→1 should succeed");

    // Both tags must be present on both nodes.
    let node0_val = cluster
        .read(0, "ormap", "item")
        .await
        .expect("read from node 0 should succeed")
        .expect("node 0 should have a value");

    let node1_val = cluster
        .read(1, "ormap", "item")
        .await
        .expect("read from node 1 should succeed")
        .expect("node 1 should have a value");

    assert_or_tags(&node0_val, &["tag-A", "tag-B"]);
    assert_or_tags(&node1_val, &["tag-A", "tag-B"]);
}

/// Each partitioned node writes to the same key in isolation. After healing
/// and Merkle syncing the cluster must converge to the LWW winner.
/// With equal wall-clock timestamps the node_id tiebreaker applies:
/// "sim-node-1" > "sim-node-0", so node 1's value wins.
#[tokio::test]
async fn write_during_partition_converges() {
    let mut cluster = SimCluster::new(2, 99);
    cluster.start().expect("cluster should start");

    // Partition nodes so they cannot communicate.
    cluster.inject_partition(&[0], &[1]);

    cluster
        .write(0, "events", "counter", rmpv::Value::String("node0-isolated".into()))
        .await
        .expect("write on partitioned node 0 should succeed");

    cluster
        .write(1, "events", "counter", rmpv::Value::String("node1-isolated".into()))
        .await
        .expect("write on partitioned node 1 should succeed");

    // Heal the partition so Merkle sync can cross node boundaries.
    cluster.heal_partition();

    // Node 1's timestamp ("sim-node-1") beats node 0's ("sim-node-0") in LWW order.
    // Syncing from node 1 to node 0 delivers the winning value to node 0.
    cluster
        .merkle_sync_pair(1, 0, "events")
        .await
        .expect("merkle sync 1→0 after heal should succeed");

    let converged = cluster
        .assert_converged("events", "counter")
        .await
        .expect("assert_converged should not error");

    assert!(
        converged.is_some(),
        "both nodes should hold a value after convergence"
    );

    // assert_converged panics if the two nodes disagree on the final value.
}
