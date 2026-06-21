//! Event Journal write-path coverage under a network-partition fault.
//!
//! The journal append is a per-node, local side effect of applying a mutation.
//! These tests prove (a) a write is recorded in the writing node's journal, and
//! (b) under a network partition each side independently journals its own writes
//! — the journal does not depend on cross-node delivery.

use topgun_core::messages::JournalEventType;
use topgun_server::sim::cluster::SimCluster;

/// A local write is appended to the writing node's journal with the right type.
#[tokio::test]
async fn write_is_journaled_on_local_node() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster should start");

    cluster
        .write(0, "todos", "t1", rmpv::Value::String("buy milk".into()))
        .await
        .expect("write on node 0 should succeed");

    let (events, _) = cluster.nodes[0].journal_store.read(0, 100, None);
    assert_eq!(events.len(), 1, "node 0 journal recorded the write");
    assert_eq!(events[0].event_type, JournalEventType::PUT);
    assert_eq!(events[0].map_name, "todos");
    assert_eq!(events[0].key, "t1");

    // The write was local; other nodes journal nothing.
    let (n1, _) = cluster.nodes[1].journal_store.read(0, 100, None);
    assert!(
        n1.is_empty(),
        "node 1 journal is independent of node 0's write"
    );
}

/// Under a partition between {0} and {1,2}, each side journals only its own
/// writes — the journal is a local append, unaffected by the network fault.
#[tokio::test]
async fn journal_records_each_side_under_partition() {
    let mut cluster = SimCluster::new(3, 7);
    cluster.start().expect("cluster should start");

    cluster.inject_partition(&[0], &[1, 2]);

    cluster
        .write(0, "data", "k0", rmpv::Value::String("from-0".into()))
        .await
        .expect("write on isolated node 0 should still apply locally");
    cluster
        .write(1, "data", "k1", rmpv::Value::String("from-1".into()))
        .await
        .expect("write on node 1 should apply locally");

    let (n0, _) = cluster.nodes[0].journal_store.read(0, 100, None);
    assert_eq!(
        n0.len(),
        1,
        "node 0 journaled its own write under partition"
    );
    assert_eq!(n0[0].key, "k0");

    let (n1, _) = cluster.nodes[1].journal_store.read(0, 100, None);
    assert_eq!(
        n1.len(),
        1,
        "node 1 journaled its own write under partition"
    );
    assert_eq!(n1[0].key, "k1");

    cluster.heal_partition();
}
