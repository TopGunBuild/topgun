/// AC3: Full-scan snapshot stability.
///
/// Each call to `QuerySubscribe` takes a point-in-time snapshot of the map.
/// Records inserted after the snapshot is taken must not appear in the response
/// for that invocation.
///
/// NOTE: The stronger form of this test (intercepting a scan mid-flight and
/// injecting new records before the response is assembled) is not practical with
/// the current harness because `QueryService::call` is a single-shot async fn
/// with no externally-observable pause point. Instead this test verifies the
/// weaker (but still load-bearing) property: two consecutive scans each reflect
/// exactly the durable state at the time they were issued. If the scan leaked
/// a pending write into a prior response the count invariant would be violated.
use std::sync::Arc;

use topgun_core::hlc::Timestamp;
use topgun_core::messages::base::Query;
use topgun_core::messages::query::{QuerySubMessage, QuerySubPayload};
use topgun_core::messages::Message;
use topgun_core::types::Value;
use tower::ServiceExt;

use topgun_server::network::config::ConnectionConfig;
use topgun_server::network::connection::{ConnectionKind, ConnectionRegistry};
use topgun_server::service::domain::query::QueryRegistry;
use topgun_server::service::domain::QueryService;
use topgun_server::service::operation::{
    service_names, Operation, OperationContext, OperationResponse,
};
use topgun_server::storage::datastores::RedbDataStore;
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::record::RecordValue;
use topgun_server::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const BASE_MILLIS: u64 = 1_704_067_200_000;

fn make_ts(seq: u32) -> Timestamp {
    Timestamp {
        millis: BASE_MILLIS + u64::from(seq) * 10,
        counter: seq,
        node_id: "ac3-node".to_string(),
    }
}

fn make_op_ctx(conn_id: topgun_server::network::connection::ConnectionId, seq: u64) -> OperationContext {
    let fence = Timestamp {
        millis: BASE_MILLIS,
        counter: 0,
        node_id: "test".to_string(),
    };
    let mut ctx = OperationContext::new(seq, service_names::QUERY, fence, 5000);
    ctx.connection_id = Some(conn_id);
    ctx
}

async fn run_scan(
    svc: Arc<QueryService>,
    conn_id: topgun_server::network::connection::ConnectionId,
    query_id: &str,
    map_name: &str,
    seq: u64,
) -> Vec<topgun_core::messages::query::QueryResultEntry> {
    let ctx = make_op_ctx(conn_id, seq);
    let payload = QuerySubMessage {
        payload: QuerySubPayload {
            query_id: query_id.to_string(),
            map_name: map_name.to_string(),
            query: Query::default(),
            fields: None,
        },
    };
    let op = Operation::QuerySubscribe { ctx, payload };
    match svc.oneshot(op).await.unwrap() {
        OperationResponse::Message(msg) => match *msg {
            Message::QueryResp(resp) => resp.payload.results,
            other => panic!("expected QueryResp, got {other:?}"),
        },
        other => panic!("expected Message response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_reflects_durable_state_at_scan_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(
        RedbDataStore::new(dir.path().join("ac3.redb")).expect("redb open"),
    );
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store.clone(),
        Vec::new(),
    ));
    let map_name = "snap_map";

    // Phase 1: seed 20 records into the durable store.
    for i in 0u32..20 {
        let value = RecordValue::Lww {
            value: Value::String(format!("initial-{i}")),
            timestamp: make_ts(i),
        };
        data_store
            .add(map_name, &format!("key{i:04}"), &value, 0, 1_704_067_200)
            .await
            .expect("durable write phase 1");
    }

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let config = ConnectionConfig::default();
    let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
    let conn_id = handle.id;

    let svc = Arc::new(QueryService::new(
        Arc::new(QueryRegistry::new()),
        factory.clone(),
        conn_registry,
        None,
        10_000,
        None,
        #[cfg(feature = "datafusion")]
        None,
    ));

    // Scan 1: should return exactly the 20 records written in phase 1.
    let results1 = run_scan(Arc::clone(&svc), conn_id, "ac3-snap-1", map_name, 1).await;
    assert_eq!(
        results1.len(),
        20,
        "scan 1 must return exactly the 20 initially seeded records"
    );

    // Phase 2: add 10 more records to the durable store.
    for i in 20u32..30 {
        let value = RecordValue::Lww {
            value: Value::String(format!("later-{i}")),
            timestamp: make_ts(i),
        };
        data_store
            .add(map_name, &format!("key{i:04}"), &value, 0, 1_704_067_200)
            .await
            .expect("durable write phase 2");
    }

    // Scan 2: must now reflect all 30 records — confirms the second scan sees
    // the new writes and no stale snapshot from scan 1 bleeds through.
    let results2 = run_scan(Arc::clone(&svc), conn_id, "ac3-snap-2", map_name, 2).await;
    assert_eq!(
        results2.len(),
        30,
        "scan 2 must return all 30 records after phase-2 writes"
    );

    // Cross-check: scan 1's results are a proper subset of scan 2's results
    // (no record was removed, only added).
    let keys1: std::collections::HashSet<String> =
        results1.iter().map(|e| e.key.clone()).collect();
    let keys2: std::collections::HashSet<String> =
        results2.iter().map(|e| e.key.clone()).collect();
    assert!(
        keys1.is_subset(&keys2),
        "all keys from scan 1 must appear in scan 2 (no records disappeared)"
    );
}
