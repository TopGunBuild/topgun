use topgun_server::sim::cluster::SimCluster;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When a new node joins an existing cluster and receives a Merkle sync from
/// an existing node, it should have all keys that were already propagated to
/// that source node.
#[tokio::test]
async fn add_node_receives_synced_data() {
    let mut cluster = SimCluster::new(2, 42);
    cluster.start().expect("cluster should start");

    // Write 3 keys to node 0.
    cluster
        .write(0, "map", "k1", rmpv::Value::String("v1".into()))
        .await
        .expect("write k1 should succeed");
    cluster
        .write(0, "map", "k2", rmpv::Value::String("v2".into()))
        .await
        .expect("write k2 should succeed");
    cluster
        .write(0, "map", "k3", rmpv::Value::String("v3".into()))
        .await
        .expect("write k3 should succeed");

    // Propagate each key to all existing nodes (nodes 0 and 1).
    cluster.sync_all("map", "k1").await.expect("sync k1");
    cluster.sync_all("map", "k2").await.expect("sync k2");
    cluster.sync_all("map", "k3").await.expect("sync k3");

    // Add a third node and get its index.
    let new_idx = cluster.add_node().expect("add_node should succeed");

    // A single Merkle sync from node 0 delivers all keys to the new node.
    cluster
        .merkle_sync_pair(0, new_idx, "map")
        .await
        .expect("merkle sync to new node should succeed");

    // The new node must have all 3 keys.
    let r1 = cluster
        .read(new_idx, "map", "k1")
        .await
        .expect("read k1 from new node");
    let r2 = cluster
        .read(new_idx, "map", "k2")
        .await
        .expect("read k2 from new node");
    let r3 = cluster
        .read(new_idx, "map", "k3")
        .await
        .expect("read k3 from new node");

    assert!(r1.is_some(), "new node should have k1 after Merkle sync");
    assert!(r2.is_some(), "new node should have k2 after Merkle sync");
    assert!(r3.is_some(), "new node should have k3 after Merkle sync");
}

/// Killing a node must not affect the data held by the remaining nodes.
#[tokio::test]
async fn kill_node_data_remains_on_survivors() {
    let mut cluster = SimCluster::new(3, 7);
    cluster.start().expect("cluster should start");

    cluster
        .write(0, "map", "resilient-key", rmpv::Value::String("value".into()))
        .await
        .expect("write should succeed");

    // Propagate to all nodes before killing any.
    cluster
        .sync_all("map", "resilient-key")
        .await
        .expect("sync should succeed");

    // Kill the middle node.
    cluster.kill_node(1);

    // The two survivors must still return the correct value.
    let from_node0 = cluster
        .read(0, "map", "resilient-key")
        .await
        .expect("read from node 0 should succeed");
    let from_node2 = cluster
        .read(2, "map", "resilient-key")
        .await
        .expect("read from node 2 should succeed");

    assert!(
        from_node0.is_some(),
        "node 0 should retain data after node 1 is killed"
    );
    assert!(
        from_node2.is_some(),
        "node 2 should retain data after node 1 is killed"
    );
}

/// Adding a node and then killing another node must not cause data loss on
/// the remaining nodes (the newly joined node included).
#[tokio::test]
async fn add_remove_cycle_no_data_loss() {
    let mut cluster = SimCluster::new(2, 99);
    cluster.start().expect("cluster should start");

    // Write 3 keys to node 0.
    cluster
        .write(0, "map", "a", rmpv::Value::String("va".into()))
        .await
        .expect("write a should succeed");
    cluster
        .write(0, "map", "b", rmpv::Value::String("vb".into()))
        .await
        .expect("write b should succeed");
    cluster
        .write(0, "map", "c", rmpv::Value::String("vc".into()))
        .await
        .expect("write c should succeed");

    // Propagate each key to node 1.
    cluster.sync_all("map", "a").await.expect("sync a");
    cluster.sync_all("map", "b").await.expect("sync b");
    cluster.sync_all("map", "c").await.expect("sync c");

    // Add node 2 and sync all keys to it from node 0.
    let new_idx = cluster.add_node().expect("add_node should succeed");
    assert_eq!(new_idx, 2, "new node should be at index 2");

    cluster
        .merkle_sync_pair(0, 2, "map")
        .await
        .expect("merkle sync 0→2 should succeed");

    // Kill node 1 — data should survive on nodes 0 and 2.
    cluster.kill_node(1);

    // Node 0 must still have all 3 keys.
    for key in &["a", "b", "c"] {
        let result = cluster
            .read(0, "map", key)
            .await
            .unwrap_or_else(|_| panic!("read {key} from node 0 should succeed"));
        assert!(
            result.is_some(),
            "node 0 should have key {key} after node 1 is killed"
        );
    }

    // Node 2 (the late-joiner) must also have all 3 keys.
    for key in &["a", "b", "c"] {
        let result = cluster
            .read(2, "map", key)
            .await
            .unwrap_or_else(|_| panic!("read {key} from node 2 should succeed"));
        assert!(
            result.is_some(),
            "node 2 should have key {key} after node 1 is killed"
        );
    }
}
