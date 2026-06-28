/// AC4: HLC-monotone `LiveWindow` guard — writes with strictly increasing HLC
/// timestamps are all accepted into the result set.
///
/// The `LiveWindow` tracks a per-key last-seen HLC pair `(millis, counter)`.
/// When a datastore scan page arrives, records whose HLC is not strictly greater
/// than the last-seen value for that key are silently dropped (stale-page guard).
///
/// This test verifies the positive case: 10 records written in sequence with
/// monotonically increasing HLC timestamps are ALL surfaced in the full-scan
/// result. No record should be silently dropped when timestamps are correctly
/// increasing.
use std::collections::HashSet;
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

/// Epoch anchor for this test suite.
const BASE_MILLIS: u64 = 1_704_067_200_000;

/// Returns a timestamp that is strictly greater than all previous timestamps
/// produced by `seq < i`. Monotone by construction: millis increases by 1 per
/// record, counter resets to 0. Simulates a well-behaved single-node write
/// sequence where each write happens 1 ms later.
fn monotone_ts(seq: u32) -> Timestamp {
    Timestamp {
        millis: BASE_MILLIS + u64::from(seq),
        counter: 0,
        node_id: "ac4-node".to_string(),
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
async fn full_scan_accepts_all_records_with_monotone_hlc() {
    const N: usize = 10;

    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac4.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store.clone(),
        Vec::new(),
    ));
    let map_name = "mono_map";

    // Write N records with strictly increasing HLC timestamps directly to the
    // durable store (non-resident). The datastore scan path invokes
    // `apply_mutation_with_hlc` on the LiveWindow; with monotone per-key
    // timestamps every record should pass the guard.
    for i in 0u32..u32::try_from(N).expect("N fits in u32") {
        let ts = monotone_ts(i);
        let value = RecordValue::Lww {
            value: Value::String(format!("record-{i}")),
            timestamp: ts,
        };
        data_store
            .add(map_name, &format!("mono{i:04}"), &value, 0, 1_704_067_200)
            .await
            .expect("durable write");
    }

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let config = ConnectionConfig::default();
    let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
    let conn_id = handle.id;

    let svc = Arc::new(QueryService::new(
        Arc::new(QueryRegistry::new()),
        factory,
        conn_registry,
        10_000,
        None,
        #[cfg(feature = "datafusion")]
        None,
    ));

    let ctx = make_op_ctx(conn_id);
    let payload = QuerySubMessage {
        payload: QuerySubPayload {
            query_id: "ac4-mono".to_string(),
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

    // (1) COMPLETENESS — all N records with monotone HLC must be returned.
    // The LiveWindow HLC guard only drops records when a stale page arrives for
    // a key that already has a NEWER resident value. Since every key is unique
    // and the timestamps are monotone, no record should be dropped.
    assert_eq!(
        resp.payload.results.len(),
        N,
        "full-scan must return all {N} records with monotone HLC timestamps"
    );

    // (2) KEY COVERAGE — every expected key is present, no duplicates.
    let returned_keys: HashSet<String> =
        resp.payload.results.iter().map(|e| e.key.clone()).collect();
    for i in 0..N {
        let key = format!("mono{i:04}");
        assert!(
            returned_keys.contains(&key),
            "key {key:?} missing from scan results — monotone HLC guard dropped it unexpectedly"
        );
    }
    assert_eq!(
        returned_keys.len(),
        N,
        "scan result set contains duplicate keys"
    );
}

/// Regression guard for the stale-page case: a record written with a lower HLC
/// than an already-resident value for the same key must NOT clobber the fresher
/// value. This simulates a stale datastore page racing with a live CRDT write.
///
/// We exercise this via the `LiveWindow` directly (not through `QueryService`) to
/// keep the test deterministic without a real concurrent write race.
#[test]
fn live_window_stale_hlc_page_does_not_overwrite_fresher_resident() {
    use topgun_server::query::window::LiveWindow;

    let window = LiveWindow::new(Vec::new(), None);

    // Simulate a live CRDT write arriving at millis=200 for key "k".
    // apply_mutation_with_hlc records (200, 0) as the last-seen HLC.
    let deltas_fresh = window.apply_mutation_with_hlc(
        "k",
        Some(&rmpv::Value::String("fresh-value".into())),
        true,
        200, // millis
        0,   // counter
    );
    assert_eq!(
        deltas_fresh.len(),
        1,
        "fresh write should produce an ENTER delta"
    );

    // Simulate a stale datastore page arriving with millis=100 (before the
    // live write). The monotone guard must drop it: no delta emitted.
    let deltas_stale = window.apply_mutation_with_hlc(
        "k",
        Some(&rmpv::Value::String("stale-value".into())),
        true,
        100, // millis — older than 200
        0,
    );
    assert!(
        deltas_stale.is_empty(),
        "stale HLC page must be silently dropped by the monotone guard"
    );
}
