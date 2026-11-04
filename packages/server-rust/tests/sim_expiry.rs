//! Expiry under a network partition (TG-TTL-001): every node sweeps its own
//! stores, and a key rewritten without a TTL on one side of the partition
//! survives the sweeps and the heal.
//!
//! The sweep here sends its removals straight to each node's CRDT service, so
//! this is no coverage of the dispatcher path.

#![cfg(feature = "simulation")]

use std::sync::Arc;

use topgun_core::messages::base::ClientOp;
use topgun_core::messages::sync::ClientOpMessage;
use topgun_core::{ClockSource, LWWRecord, ORMapRecord, SystemClock, Timestamp};
use topgun_server::service::operation::{service_names, CallerOrigin, Operation, OperationContext};
use topgun_server::sim::cluster::SimCluster;
use topgun_server::storage::expiry::ManualClock;
use topgun_server::storage::record::RecordValue;
use tower::Service;

const TTL: u64 = 30_000;
const LWW_MAP: &str = "expiry-lww";
const OR_MAP: &str = "expiry-or";

/// Applies `op` on `node_idx` as a trusted write, so its stamp and TTL are
/// stored as given.
async fn apply(cluster: &SimCluster, node_idx: usize, op: ClientOp) {
    let mut ctx = OperationContext::new(
        0,
        service_names::CRDT,
        Timestamp {
            millis: 0,
            counter: 0,
            node_id: String::new(),
        },
        5000,
    );
    ctx.partition_id = Some(topgun_core::hash_to_partition(&op.key));
    ctx.caller_origin = CallerOrigin::System;
    let mut svc = Arc::clone(&cluster.nodes[node_idx].crdt_service);
    Service::call(
        &mut svc,
        Operation::ClientOp {
            ctx,
            payload: ClientOpMessage { payload: op },
        },
    )
    .await
    .expect("write");
}

fn lww_put(key: &str, value: i64, timestamp: Timestamp, ttl_ms: Option<u64>) -> ClientOp {
    ClientOp {
        id: Some(format!("{LWW_MAP}/{key}")),
        map_name: LWW_MAP.to_string(),
        key: key.to_string(),
        op_type: None,
        record: Some(Some(LWWRecord {
            value: Some(rmpv::Value::from(value)),
            timestamp,
            ttl_ms,
        })),
        or_record: None,
        or_tag: None,
        write_concern: None,
        timeout: None,
    }
}

fn or_add(key: &str, tag: &str, timestamp: Timestamp, ttl_ms: Option<u64>) -> ClientOp {
    ClientOp {
        id: Some(format!("{OR_MAP}/{key}/{tag}")),
        map_name: OR_MAP.to_string(),
        key: key.to_string(),
        op_type: None,
        record: None,
        or_record: Some(Some(ORMapRecord {
            value: rmpv::Value::from(1),
            timestamp,
            tag: tag.to_string(),
            ttl_ms,
        })),
        or_tag: Some(Some(tag.to_string())),
        write_concern: None,
        timeout: None,
    }
}

fn stamp(millis: u64, node: usize) -> Timestamp {
    Timestamp {
        millis,
        counter: 0,
        node_id: format!("sim-node-{node}"),
    }
}

/// Three nodes share one expiry clock, seeded with the wall time once at the
/// start, because the nodes' HLCs stamp wall time. An LWW key and an OR entry
/// with a TTL, and an LWW key with a TTL that node 1 rewrites without one while
/// partitioned from node 0, reach every node. After the clock passes expiry and
/// every node sweeps, then the partition heals and every key propagates: the
/// expired key and entry are gone everywhere, and the rewritten key is present
/// everywhere with the newer value.
#[tokio::test]
async fn sim_expiry_partition_rewrite_survives() {
    let wall = SystemClock.now();
    let clock = Arc::new(ManualClock::new(wall));
    let mut cluster = SimCluster::new(3, 42).with_clock(Arc::clone(&clock));
    cluster.start().expect("cluster start");

    apply(&cluster, 0, lww_put("gone", 1, stamp(wall, 0), Some(TTL))).await;
    apply(
        &cluster,
        0,
        lww_put("rewritten", 1, stamp(wall, 0), Some(TTL)),
    )
    .await;
    apply(
        &cluster,
        0,
        or_add("gone-or", "t1", stamp(wall, 0), Some(TTL)),
    )
    .await;
    for (map, key) in [
        (LWW_MAP, "gone"),
        (LWW_MAP, "rewritten"),
        (OR_MAP, "gone-or"),
    ] {
        cluster.sync_all(map, key).await.expect("sync");
        for node in 0..3 {
            assert!(
                cluster.read(node, map, key).await.expect("read").is_some(),
                "{key} reached node {node}"
            );
        }
    }

    cluster.inject_partition(&[0], &[1, 2]);
    apply(
        &cluster,
        1,
        lww_put("rewritten", 2, stamp(wall + 1, 1), None),
    )
    .await;

    // The margin covers any wall time the test spent before this point.
    clock.advance(TTL + 60_000);
    for (node, expected_candidates) in [(0, 3), (1, 2), (2, 3)] {
        let report = cluster.nodes[node].sweep_tick().await;
        assert!(report.cycle_started && report.cycle_completed, "{report:?}");
        assert_eq!(report.candidates, expected_candidates, "node {node}");
        assert_eq!(report.dispatched, expected_candidates, "node {node}");
    }

    cluster.heal_partition();
    for (map, key) in [
        (LWW_MAP, "gone"),
        (LWW_MAP, "rewritten"),
        (OR_MAP, "gone-or"),
    ] {
        cluster.sync_all(map, key).await.expect("sync");
    }

    for node in 0..3 {
        assert_eq!(
            cluster.read(node, LWW_MAP, "gone").await.expect("read"),
            None,
            "the expired LWW key is gone on node {node}"
        );
        match cluster.read(node, OR_MAP, "gone-or").await.expect("read") {
            None => {}
            Some(RecordValue::OrMap { records, .. }) => assert!(
                records.is_empty(),
                "the expired OR entry is gone on node {node}: {records:?}"
            ),
            Some(other) => panic!("node {node}: unexpected value {other:?}"),
        }
        match cluster
            .read(node, LWW_MAP, "rewritten")
            .await
            .expect("read")
        {
            Some(RecordValue::Lww {
                value,
                timestamp,
                ttl_ms,
            }) => {
                assert_eq!(value, topgun_core::types::Value::Int(2), "node {node}");
                assert_eq!(timestamp, stamp(wall + 1, 1), "node {node}");
                assert_eq!(ttl_ms, None, "node {node}");
            }
            other => panic!("node {node}: the rewritten key must survive, got {other:?}"),
        }
    }
}
