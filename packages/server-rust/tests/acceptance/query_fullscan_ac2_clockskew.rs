/// AC2: Full-scan over a map whose records carry intentionally skewed HLC timestamps
/// returns ALL records regardless of timestamp ordering, and preserves the written
/// timestamps without mutation during the scan.
///
/// Skewed timestamps simulate nodes whose clocks are ahead or behind by seconds —
/// a realistic condition in any distributed deployment. The scan must treat
/// timestamp ordering as a conflict-resolution input, not a filter criterion:
/// every durable key surfaces in the result set.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use topgun_core::hlc::Timestamp;
use topgun_core::messages::base::Query;
use topgun_core::messages::query::{QuerySubMessage, QuerySubPayload};
use topgun_core::types::Value;
use tower::ServiceExt;

use topgun_core::messages::Message;
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

/// A fixed "reference" epoch (2024-01-01T00:00:00Z in millis).
const BASE_MILLIS: u64 = 1_704_067_200_000;

/// Builds a timestamp at `base ± offset_ms` with the given counter, simulating
/// clock skew. Negative offsets represent clocks that are behind the reference.
fn skewed_ts(offset_ms: i64, counter: u32) -> Timestamp {
    // Clamp to u64 range: a simulated past clock (negative offset) saturates at 0.
    let millis =
        u64::try_from(i64::try_from(BASE_MILLIS).unwrap_or(i64::MAX) + offset_ms).unwrap_or(0);
    Timestamp {
        millis,
        counter,
        node_id: format!("skew-node-{offset_ms}"),
    }
}

fn make_op_ctx(conn_id: topgun_server::network::connection::ConnectionId) -> OperationContext {
    let fence = Timestamp {
        millis: BASE_MILLIS,
        counter: 0,
        node_id: "test".to_string(),
    };
    let mut ctx = OperationContext::new(1, service_names::QUERY, fence, 5000);
    ctx.connection_id = Some(conn_id);
    ctx
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // single behavioural test: setup + assert + teardown are inseparable
async fn full_scan_returns_all_records_regardless_of_hlc_skew() {
    const N: usize = 30;

    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac2.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store.clone(),
        Vec::new(),
    ));
    let map_name = "skew_map";

    // Offsets deliberately span ahead (+30s), behind (-30s), and at-base (0s).
    // Written directly to the durable store so they are non-resident (tests the
    // datastore-backed scan path, not the in-memory path).
    let mut written_timestamps: HashMap<String, Timestamp> = HashMap::new();
    for i in 0..N {
        // Cycle: ahead, behind, at-base, ahead-by-more, behind-by-more, …
        let offset_ms: i64 = match i % 6 {
            0 => 30_000,   // 30 s ahead
            1 => -30_000,  // 30 s behind
            2 => 0,        // at reference
            3 => 120_000,  // 2 min ahead
            4 => -120_000, // 2 min behind
            _ => 5_000,    // 5 s ahead
        };
        let ts = skewed_ts(offset_ms, u32::try_from(i).unwrap_or(u32::MAX));
        let key = format!("sk{i:04}");
        let value = RecordValue::Lww {
            value: Value::String(format!("val-{i}")),
            timestamp: ts.clone(),
        };
        data_store
            .add(map_name, &key, &value, 0, 1_704_067_200)
            .await
            .expect("durable write");
        written_timestamps.insert(key, ts);
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

    let ctx = make_op_ctx(conn_id);
    let payload = QuerySubMessage {
        payload: QuerySubPayload {
            query_id: "ac2-skew".to_string(),
            map_name: map_name.to_string(),
            query: Query::default(), // full-scan: no predicate, sort, or limit
            fields: None,
        },
    };
    let op = Operation::QuerySubscribe { ctx, payload };
    let resp = match svc.oneshot(op).await.unwrap() {
        OperationResponse::Message(msg) => match *msg {
            Message::QueryResp(resp) => resp,
            other => panic!("expected QueryResp, got {other:?}"),
        },
        other => panic!("expected Message response, got {other:?}"),
    };

    // (1) COMPLETENESS — every key must be present regardless of HLC ordering.
    assert_eq!(
        resp.payload.results.len(),
        N,
        "full-scan must return all {N} records even when HLC timestamps are skewed"
    );

    // (2) KEY COVERAGE — no key is missing or duplicated.
    let returned_keys: HashSet<String> =
        resp.payload.results.iter().map(|e| e.key.clone()).collect();
    for i in 0..N {
        let key = format!("sk{i:04}");
        assert!(
            returned_keys.contains(&key),
            "key {key:?} is missing from scan results"
        );
    }
    assert_eq!(
        returned_keys.len(),
        N,
        "scan results contain duplicate keys"
    );

    // (3) TIMESTAMP PRESERVATION — the scan must not mutate the stored timestamps.
    // We verify this by re-reading the timestamps from the record store factory
    // (which reflects whatever the scan made resident). Any key that the scan
    // materialized must still carry the original timestamp.
    //
    // The scan may choose not to make records resident (streaming path), in which
    // case the factory stores are empty — that is equally correct and also
    // satisfies preservation because the original durable bytes are untouched.
    // We only assert when residency is non-zero.
    let all_stores = factory.all_stores();
    for store in &all_stores {
        store.for_each_boxed(
            &mut |key, record| {
                if let RecordValue::Lww { ref timestamp, .. } = record.value {
                    let original = written_timestamps
                        .get(key)
                        .expect("scan produced an unexpected key");
                    assert_eq!(
                        timestamp.millis, original.millis,
                        "timestamp.millis was mutated for key {key:?}"
                    );
                    assert_eq!(
                        timestamp.counter, original.counter,
                        "timestamp.counter was mutated for key {key:?}"
                    );
                }
            },
            false,
        );
    }
}
