//! Smoke tests for the simulation cluster harness.
//!
//! These tests validate that `SimCluster`, `SimNode`, and `SimNetwork`
//! work correctly for single-node scenarios. Cross-node propagation and
//! convergence testing are deferred to a future spec.

#![cfg(feature = "simulation")]

use std::time::Duration;

use topgun_server::sim::cluster::{SimCluster, SimNode};
use topgun_server::sim::network::SimNetwork;
use topgun_server::storage::record::RecordValue;

/// AC1: `SimCluster::new(3, 42)` creates a 3-node cluster that starts successfully.
#[tokio::test]
async fn sim_cluster_starts_with_three_nodes() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start successfully");
    assert_eq!(cluster.nodes.len(), 3);
    assert!(cluster.nodes.iter().all(SimNode::is_alive));
}

/// AC2: Write via `SimCluster::write()` is readable via `SimCluster::read()` on the same node.
#[tokio::test]
async fn sim_cluster_write_read_same_node() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start");

    // Write a value to node 0.
    let value = rmpv::Value::Map(vec![
        (
            rmpv::Value::String("name".into()),
            rmpv::Value::String("Alice".into()),
        ),
        (
            rmpv::Value::String("age".into()),
            rmpv::Value::Integer(30.into()),
        ),
    ]);
    cluster
        .write(0, "users", "alice", value)
        .await
        .expect("write should succeed");

    // Read back from the same node.
    let result = cluster
        .read(0, "users", "alice")
        .await
        .expect("read should succeed");
    assert!(result.is_some(), "written value should be readable");

    match result.unwrap() {
        RecordValue::Lww { value, .. } => {
            // Verify the value is a Map variant with 2 entries.
            match value {
                topgun_core::types::Value::Map(ref map) => {
                    assert_eq!(map.len(), 2, "map should have 2 entries");
                }
                other => panic!("expected Map value, got {other:?}"),
            }
        }
        other => panic!("expected Lww record, got {other:?}"),
    }
}

/// AC3: `inject_partition` and `heal_partition` execute without error.
#[tokio::test]
async fn sim_cluster_fault_injection_executes() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start");

    // Partition node 0 from nodes 1 and 2.
    cluster.inject_partition(&[0], &[1, 2]);

    // Verify partition state.
    assert!(cluster.network.is_partitioned("sim-node-0", "sim-node-1"));
    assert!(cluster.network.is_partitioned("sim-node-0", "sim-node-2"));
    assert!(!cluster.network.is_partitioned("sim-node-1", "sim-node-2"));

    // Heal partition.
    cluster.heal_partition();
    assert!(!cluster.network.is_partitioned("sim-node-0", "sim-node-1"));
}

/// AC4: `advance_time` moves virtual time forward.
#[tokio::test]
async fn sim_cluster_advance_time() {
    let mut cluster = SimCluster::new(1, 42);
    cluster.start().expect("cluster should start");

    // Under madsim runtime this would advance virtual time instantly.
    // Under real tokio we just verify it completes without panic.
    cluster.advance_time(Duration::from_millis(1)).await;
}

/// AC5: Running with same seed produces identical results (determinism).
#[tokio::test]
async fn sim_cluster_deterministic_with_same_seed() {
    // Run 1
    let mut cluster1 = SimCluster::new(3, 42);
    cluster1.start().expect("cluster1 should start");
    cluster1
        .write(0, "users", "alice", rmpv::Value::String("v1".into()))
        .await
        .expect("write1 should succeed");
    let result1 = cluster1
        .read(0, "users", "alice")
        .await
        .expect("read1 should succeed");

    // Run 2 with same seed
    let mut cluster2 = SimCluster::new(3, 42);
    cluster2.start().expect("cluster2 should start");
    cluster2
        .write(0, "users", "alice", rmpv::Value::String("v1".into()))
        .await
        .expect("write2 should succeed");
    let result2 = cluster2
        .read(0, "users", "alice")
        .await
        .expect("read2 should succeed");

    // Both runs produce the same result.
    assert!(result1.is_some());
    assert!(result2.is_some());
    // Both should be Lww with the same string value.
    match (&result1.unwrap(), &result2.unwrap()) {
        (RecordValue::Lww { value: v1, .. }, RecordValue::Lww { value: v2, .. }) => {
            assert_eq!(v1, v2, "same seed should produce identical results");
        }
        _ => panic!("expected Lww records from both runs"),
    }
}

/// AC6 is verified by all tests compiling and running only with `--features simulation`.
/// Test node kill and restart lifecycle.
#[tokio::test]
async fn sim_cluster_kill_and_restart_node() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start");

    // Kill node 1.
    cluster.kill_node(1);
    assert!(!cluster.nodes[1].is_alive());

    // Write to node 0 still works.
    cluster
        .write(0, "data", "key1", rmpv::Value::String("hello".into()))
        .await
        .expect("write to alive node should succeed");

    // Restart node 1.
    cluster.restart_node(1).expect("restart should succeed");
    assert!(cluster.nodes[1].is_alive());

    // Write to restarted node works (fresh state).
    cluster
        .write(1, "data", "key2", rmpv::Value::String("world".into()))
        .await
        .expect("write to restarted node should succeed");
}

/// Test `SimNetwork` delay and reorder methods.
#[tokio::test]
async fn sim_network_delay_and_reorder() {
    let network = SimNetwork::new();

    // Add delay between two nodes.
    network.delay("node-a", "node-b", Duration::from_millis(100));
    assert_eq!(
        network.get_delay("node-a", "node-b"),
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        network.get_delay("node-b", "node-a"),
        Some(Duration::from_millis(100))
    );
    assert_eq!(network.get_delay("node-a", "node-c"), None);

    // Enable reordering.
    network.reorder("node-a", "node-b");
    assert!(network.is_reordered("node-a", "node-b"));
    assert!(network.is_reordered("node-b", "node-a"));
    assert!(!network.is_reordered("node-a", "node-c"));
}
