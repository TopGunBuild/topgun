//! A live subscribe racing writes to its map, under a partition and heal of
//! the other nodes, ends with the subscriber holding the stored rows
//! (TG-LIVE-001).

#![cfg(feature = "simulation")]

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use topgun_core::messages::base::{ChangeEventType, ClientOp, Query};
use topgun_core::messages::query::{QuerySubMessage, QuerySubPayload};
use topgun_core::messages::sync::ClientOpMessage;
use topgun_core::messages::Message;
use topgun_core::types::Value;
use topgun_core::{LWWRecord, Timestamp};
use topgun_server::network::{ConnectionConfig, ConnectionKind, OutboundMessage};
use topgun_server::query::delta_buffer::QUERY_SNAPSHOT_OVERFLOW;
use topgun_server::service::domain::CrdtService;
use topgun_server::service::operation::{
    service_names, CallerOrigin, Operation, OperationContext, OperationResponse,
};
use topgun_server::sim::cluster::{SimCluster, SimNode};
use topgun_server::storage::record::RecordValue;
use tower::Service;

const ITERATIONS: usize = 32;
const SEEDED: usize = 20;
const WRITES: usize = 60;
const QUERY_ID: &str = "live";

fn stamp(node_id: &str) -> Timestamp {
    Timestamp {
        millis: 0,
        counter: 0,
        node_id: node_id.to_string(),
    }
}

/// A trusted write of `key`: `Some(n)` puts `n`, `None` removes the key.
async fn apply(crdt: &Arc<CrdtService>, map: &str, key: &str, value: Option<i64>) {
    let mut ctx = OperationContext::new(0, service_names::CRDT, stamp(""), 5000);
    ctx.partition_id = Some(topgun_core::hash_to_partition(key));
    ctx.caller_origin = CallerOrigin::System;
    let op = ClientOp {
        map_name: map.to_string(),
        key: key.to_string(),
        record: Some(value.map(|n| LWWRecord {
            value: Some(rmpv::Value::from(n)),
            timestamp: stamp("writer"),
            ttl_ms: None,
        })),
        ..ClientOp::default()
    };
    let mut svc = Arc::clone(crdt);
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

async fn subscribe(
    node: &mut SimNode,
    connection: topgun_server::network::ConnectionId,
    map: &str,
) -> OperationResponse {
    let mut ctx = OperationContext::new(0, service_names::QUERY, stamp(&node.node_id), 5000);
    ctx.connection_id = Some(connection);
    let op = Operation::QuerySubscribe {
        ctx,
        payload: QuerySubMessage {
            payload: QuerySubPayload {
                query_id: QUERY_ID.to_string(),
                map_name: map.to_string(),
                query: Query::default(),
                fields: None,
                snapshot_only: false,
            },
        },
    };
    Service::call(&mut node.operation_router, op)
        .await
        .expect("subscribe")
}

fn int(value: &rmpv::Value) -> i64 {
    value.as_i64().expect("an integer row")
}

/// Applies the query's frames waiting on `rx` the way the client does: a
/// `QUERY_RESP` replaces the rows, a `QUERY_UPDATE` edits them. Returns
/// whether a snapshot arrived.
fn replay(rx: &mut mpsc::Receiver<OutboundMessage>, rows: &mut BTreeMap<String, i64>) -> bool {
    let mut snapshot = false;
    while let Ok(frame) = rx.try_recv() {
        let Some(bytes) = frame.payload() else { break };
        match rmp_serde::from_slice::<Message>(bytes).expect("decodes") {
            Message::QueryResp(resp) if resp.payload.query_id == QUERY_ID => {
                assert!(!snapshot, "one snapshot per subscribe");
                assert_eq!(resp.payload.code, None, "a delivered snapshot");
                snapshot = true;
                *rows = resp
                    .payload
                    .results
                    .iter()
                    .map(|e| (e.key.clone(), int(&e.value)))
                    .collect();
            }
            Message::QueryUpdate { payload } if payload.query_id == QUERY_ID => {
                assert!(
                    snapshot,
                    "an update of {} precedes the snapshot",
                    payload.key
                );
                match payload.change_type {
                    ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                        rows.insert(payload.key, int(&payload.value));
                    }
                    ChangeEventType::LEAVE => {
                        rows.remove(&payload.key);
                    }
                }
            }
            _ => {}
        }
    }
    snapshot
}

async fn stored_rows(cluster: &SimCluster, map: &str) -> BTreeMap<String, i64> {
    let mut rows = BTreeMap::new();
    for i in 0..SEEDED + WRITES {
        let key = format!("k{i}");
        if let Some(RecordValue::Lww {
            value: Value::Int(n),
            ..
        }) = cluster.read(0, map, &key).await.expect("read")
        {
            rows.insert(key, n);
        }
    }
    rows
}

/// Fault-scenario smoke, not a proof of the capture: `SimNode::build` ignores
/// its seed and nothing here makes a write land during the scan, so it cannot
/// show that the snapshot buffer captured anything (the in-process tests of
/// the query service do). It checks the end state under a fault: a live
/// subscribe on node 0 races new keys, updates and removes of its map while
/// nodes 1 and 2 are partitioned; replayed as the client applies them, its
/// frames hold the stored rows, or, after an overflow that followed the
/// snapshot, a resubscribe's rows do.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sim_live_subscribe_converges_under_concurrent_writes() {
    let mut cluster = SimCluster::new(3, 42);
    cluster.start().expect("cluster starts");
    let crdt = Arc::clone(&cluster.nodes[0].crdt_service);

    for iteration in 0..ITERATIONS {
        let map = format!("live-{iteration}");
        for i in 0..SEEDED {
            apply(
                &crdt,
                &map,
                &format!("k{i}"),
                Some(i64::try_from(i).expect("small")),
            )
            .await;
        }
        let (handle, mut rx) = cluster.nodes[0]
            .connection_registry
            .register(ConnectionKind::Client, &ConnectionConfig::default());

        cluster.inject_partition(&[1], &[2]);
        let writer = {
            let crdt = Arc::clone(&crdt);
            let map = map.clone();
            tokio::spawn(async move {
                for w in 0..WRITES {
                    let n = i64::try_from(iteration * WRITES + w).expect("small");
                    match w % 3 {
                        0 => apply(&crdt, &map, &format!("k{}", SEEDED + w), Some(n)).await,
                        1 => apply(&crdt, &map, &format!("k{}", w % SEEDED), Some(n)).await,
                        _ => apply(&crdt, &map, &format!("k{}", w % SEEDED), None).await,
                    }
                    tokio::task::yield_now().await;
                }
            })
        };
        let response = subscribe(&mut cluster.nodes[0], handle.id, &map).await;
        writer.await.expect("writer");
        cluster.heal_partition();

        let mut rows = BTreeMap::new();
        match response {
            OperationResponse::Empty => {
                assert!(replay(&mut rx, &mut rows), "the snapshot is on the channel");
            }
            OperationResponse::Messages(frames) => {
                assert!(
                    matches!(&frames[..], [Message::QueryResp(s), Message::QueryResp(o)]
                        if s.payload.code.is_none()
                            && o.payload.code.as_deref() == Some(QUERY_SNAPSHOT_OVERFLOW)),
                    "an overflow follows the snapshot: {frames:?}"
                );
                assert!(!replay(&mut rx, &mut rows), "nothing was enqueued");
                let again = subscribe(&mut cluster.nodes[0], handle.id, &map).await;
                assert!(matches!(again, OperationResponse::Empty));
                assert!(replay(&mut rx, &mut rows), "the resubscribe's snapshot");
            }
            other => panic!("unexpected subscribe response: {other:?}"),
        }
        assert_eq!(
            rows,
            stored_rows(&cluster, &map).await,
            "iteration {iteration}: the replayed rows are the stored rows"
        );
        let _ = cluster.nodes[0].connection_registry.remove(handle.id);
    }
}
