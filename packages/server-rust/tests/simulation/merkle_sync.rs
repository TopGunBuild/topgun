use topgun_server::sim::cluster::SimCluster;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A late-joining node should receive all previously written keys after a
/// single Merkle sync pass from an established node.
#[tokio::test]
async fn late_joiner_receives_all_data() {
    let mut cluster = SimCluster::new(2, 10);
    cluster.start().expect("cluster should start");

    // Write 5 distinct keys to node 0 before the late joiner exists.
    let keys = ["key1", "key2", "key3", "key4", "key5"];
    for key in &keys {
        cluster
            .write(0, "map", key, rmpv::Value::String(key.to_string().into()))
            .await
            .unwrap_or_else(|e| panic!("write {key} should succeed: {e}"));
    }

    // Add a new node; it starts with no data.
    let node2 = cluster.add_node().expect("add_node should succeed");

    // One Merkle sync pass transfers all keys from node 0 to the late joiner.
    cluster
        .merkle_sync_pair(0, node2, "map")
        .await
        .expect("merkle sync 0→node2 should succeed");

    // All 5 keys must be readable on the late joiner.
    for key in &keys {
        let result = cluster
            .read(node2, "map", key)
            .await
            .unwrap_or_else(|e| panic!("read {key} on late joiner should succeed: {e}"));
        assert!(
            result.is_some(),
            "late joiner should have key {key} after Merkle sync"
        );
    }
}

/// After a network partition where each isolated node writes a different key,
/// healing the partition and running bidirectional Merkle sync must leave
/// both nodes holding both keys.
#[tokio::test]
async fn merkle_sync_after_partition_heal() {
    let mut cluster = SimCluster::new(2, 20);
    cluster.start().expect("cluster should start");

    // Isolate the two nodes from each other.
    cluster.inject_partition(&[0], &[1]);

    cluster
        .write(0, "map", "A", rmpv::Value::String("value-A".into()))
        .await
        .expect("write A on node 0 should succeed");

    cluster
        .write(1, "map", "B", rmpv::Value::String("value-B".into()))
        .await
        .expect("write B on node 1 should succeed");

    // Restore connectivity so cross-node Merkle sync is possible.
    cluster.heal_partition();

    // Bidirectional Merkle sync reconciles both sides: node 0 sends key A to
    // node 1, and node 1 sends key B to node 0.
    cluster
        .merkle_sync_pair(0, 1, "map")
        .await
        .expect("merkle sync 0→1 should succeed");
    cluster
        .merkle_sync_pair(1, 0, "map")
        .await
        .expect("merkle sync 1→0 should succeed");

    // Both nodes must now hold both keys.
    for node in [0usize, 1usize] {
        let a = cluster
            .read(node, "map", "A")
            .await
            .unwrap_or_else(|e| panic!("read A on node {node} should succeed: {e}"));
        assert!(
            a.is_some(),
            "node {node} should have key A after partition heal and Merkle sync"
        );

        let b = cluster
            .read(node, "map", "B")
            .await
            .unwrap_or_else(|e| panic!("read B on node {node} should succeed: {e}"));
        assert!(
            b.is_some(),
            "node {node} should have key B after partition heal and Merkle sync"
        );
    }
}
