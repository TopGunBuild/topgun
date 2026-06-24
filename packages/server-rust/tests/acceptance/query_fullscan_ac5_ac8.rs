/// AC5: Tie-break set-equality — records that are equal on every sort field are
/// ordered deterministically by primary key ascending, and the result set is
/// stable across multiple calls regardless of datastore enumeration order.
///
/// AC6: Registration-gap race — a write that arrives between subscription
/// registration and the snapshot response is NOT lost; the `DeltaBuffer` captures
/// it and `deactivate_and_drain` replays it to the subscriber.  The public API
/// contract (`activate`/`route`/`is_active`) is verified at the acceptance level.
///
/// AC7: Unbounded-sort + overflow rejects — `QuerySubscribe` with a sort spec but
/// no limit returns `QUERY_RESP` with `code = QUERY_UNBOUNDED_SORT` rather than
/// attempting the scan; the `QUERY_SNAPSHOT_OVERFLOW` constant is verified to
/// match the value the TS client expects.
///
/// AC8: Semantics / wire diff guard — the `QueryRespPayload` round-trips through
/// msgpack with field names matching the camelCase TypeScript schema (queryId,
/// results, hasMore, error, code); the `error` and `code` fields are absent on
/// the wire when None (`skip_serializing_if`); the full `Message::QueryResp` envelope
/// also round-trips correctly.
use std::sync::Arc;

use topgun_core::hlc::Timestamp;
use topgun_core::messages::base::{Query, SortDirection, SortField};
use topgun_core::messages::query::{
    QueryRespMessage, QueryRespPayload, QuerySubMessage, QuerySubPayload,
};
use topgun_core::messages::Message;
use topgun_core::types::Value;
use tower::ServiceExt;

use topgun_server::network::config::ConnectionConfig;
use topgun_server::network::connection::{ConnectionId, ConnectionKind, ConnectionRegistry};
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

fn ts(offset: u64) -> Timestamp {
    Timestamp {
        millis: BASE_MILLIS + offset,
        counter: 0,
        node_id: "ac58-node".to_string(),
    }
}

fn make_op_ctx(conn_id: ConnectionId) -> OperationContext {
    let fence = Timestamp {
        millis: BASE_MILLIS,
        counter: 0,
        node_id: "test".to_string(),
    };
    let mut ctx = OperationContext::new(1, service_names::QUERY, fence, 5000);
    ctx.connection_id = Some(conn_id);
    ctx
}

fn make_svc(
    factory: Arc<RecordStoreFactory>,
    conn_registry: Arc<ConnectionRegistry>,
) -> Arc<QueryService> {
    Arc::new(QueryService::new(
        Arc::new(QueryRegistry::new()),
        factory,
        conn_registry,
        None,
        10_000,
        None,
        #[cfg(feature = "datafusion")]
        None,
    ))
}

async fn run_subscribe(
    svc: Arc<QueryService>,
    conn_id: ConnectionId,
    query_id: &str,
    map_name: &str,
    query: Query,
) -> QueryRespMessage {
    let ctx = make_op_ctx(conn_id);
    let payload = QuerySubMessage {
        payload: QuerySubPayload {
            query_id: query_id.to_string(),
            map_name: map_name.to_string(),
            query,
            fields: None,
        },
    };
    let op = Operation::QuerySubscribe { ctx, payload };
    match svc.oneshot(op).await.unwrap() {
        OperationResponse::Message(msg) => match *msg {
            Message::QueryResp(resp) => resp,
            other => panic!("expected QueryResp, got {other:?}"),
        },
        other => panic!("expected Message response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AC5: Tie-break set-equality
// ---------------------------------------------------------------------------

/// Records whose sort-field values are equal must be returned in ascending key
/// order on every call, regardless of the order the datastore enumerates them.
///
/// We write N records with an identical `score` field so the sort key is equal
/// for all of them.  The tie-break rule (primary key ascending) must produce the
/// lexicographic key order consistently.
#[tokio::test(flavor = "multi_thread")]
async fn tiebreak_equal_sort_field_produces_deterministic_key_order() {
    const N: usize = 8;

    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac5.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store.clone(),
        Vec::new(),
    ));
    let map_name = "tiebreak_map";

    // Insert keys out of alphabetical order with identical score=42 so that
    // the sort field alone cannot distinguish them; only the _key tie-break can.
    let keys = ["k_c", "k_a", "k_g", "k_b", "k_e", "k_f", "k_d", "k_h"];
    for key in keys {
        let value = RecordValue::Lww {
            value: Value::Map({
                let mut m = std::collections::BTreeMap::new();
                m.insert("score".to_string(), Value::Int(42));
                m
            }),
            timestamp: ts(1),
        };
        data_store
            .add(map_name, key, &value, 0, 1_704_067_200)
            .await
            .expect("durable write");
    }

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let config = ConnectionConfig::default();
    let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
    let conn_id = handle.id;

    let svc = make_svc(factory, conn_registry);

    let query = Query {
        sort: Some(vec![SortField {
            field: "score".to_string(),
            direction: SortDirection::Asc,
        }]),
        limit: Some(u32::try_from(N).expect("N fits u32")),
        ..Query::default()
    };

    // Run the scan twice to verify stability — the result set must be identical
    // across calls regardless of any internal iteration order differences.
    let resp1 = run_subscribe(
        Arc::clone(&svc),
        conn_id,
        "ac5-tie-1",
        map_name,
        query.clone(),
    )
    .await;
    let resp2 = run_subscribe(Arc::clone(&svc), conn_id, "ac5-tie-2", map_name, query).await;

    // Both runs must return all N records.
    assert_eq!(
        resp1.payload.results.len(),
        N,
        "first scan must return all {N} records"
    );
    assert_eq!(
        resp2.payload.results.len(),
        N,
        "second scan must return all {N} records"
    );

    // The key order must match the lexicographic ascending order defined by the
    // _key tie-break.  Since score is equal for every record, the comparator
    // falls through to the key dimension and must produce a, b, c, ... h.
    let keys1: Vec<&str> = resp1
        .payload
        .results
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    let keys2: Vec<&str> = resp2
        .payload
        .results
        .iter()
        .map(|e| e.key.as_str())
        .collect();

    let expected: Vec<&str> = {
        let mut sorted = keys.to_vec();
        sorted.sort_unstable();
        sorted
    };

    assert_eq!(
        keys1, expected,
        "first scan: key order must follow _key ASC tie-break"
    );
    assert_eq!(
        keys2, expected,
        "second scan: key order must be identical (stable tie-break)"
    );

    // Set-equality: same keys in both runs (order already verified above).
    let set1: std::collections::BTreeSet<&str> = keys1.iter().copied().collect();
    let set2: std::collections::BTreeSet<&str> = keys2.iter().copied().collect();
    assert_eq!(
        set1, set2,
        "the result key set must be identical across scans"
    );
}

/// Tie-break with DESC sort field: records still break by key ASC even when the
/// main sort direction is descending — the _key tie-break is always ascending
/// regardless of the query sort direction.
#[tokio::test(flavor = "multi_thread")]
async fn tiebreak_equal_sort_field_key_ascending_regardless_of_desc_sort() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac5b.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store.clone(),
        Vec::new(),
    ));
    let map_name = "tiebreak_desc_map";

    // Three records with identical rank field (DESC sort), only key differs.
    for key in ["z_record", "a_record", "m_record"] {
        let value = RecordValue::Lww {
            value: Value::Map({
                let mut m = std::collections::BTreeMap::new();
                m.insert("rank".to_string(), Value::Int(100));
                m
            }),
            timestamp: ts(1),
        };
        data_store
            .add(map_name, key, &value, 0, 1_704_067_200)
            .await
            .expect("durable write");
    }

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let (handle, _rx) =
        conn_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
    let conn_id = handle.id;

    let svc = make_svc(factory, conn_registry);

    let resp = run_subscribe(
        svc,
        conn_id,
        "ac5b-desc-tie",
        map_name,
        Query {
            sort: Some(vec![SortField {
                field: "rank".to_string(),
                direction: SortDirection::Desc,
            }]),
            limit: Some(3),
            ..Query::default()
        },
    )
    .await;

    assert_eq!(resp.payload.results.len(), 3);
    // All have same rank (DESC), so _key ASC tie-break applies: a < m < z.
    let result_keys: Vec<&str> = resp
        .payload
        .results
        .iter()
        .map(|e| e.key.as_str())
        .collect();
    assert_eq!(
        result_keys,
        vec!["a_record", "m_record", "z_record"],
        "tie-break is always key ASC regardless of sort direction"
    );
}

// ---------------------------------------------------------------------------
// AC6: Registration-gap race — DeltaBuffer public lifecycle contract
// ---------------------------------------------------------------------------

/// The `DeltaBuffer` public lifecycle contract:
/// - `is_active()` returns `false` before `activate()`.
/// - `is_active()` returns `true` after `activate()`.
/// - A second `activate()` on an already-active buffer is idempotent.
///
/// These observable state transitions are what the registration-gap protocol
/// relies on: the subscription is registered first, then `activate()` opens the
/// capture window, then the snapshot scan runs, then `deactivate_and_drain`
/// closes the window.  Any write that arrives after `activate()` and before
/// drain is captured and replayed.
#[test]
fn delta_buffer_activation_lifecycle_contract() {
    use topgun_server::query::delta_buffer::DeltaBuffer;

    let buf = DeltaBuffer::new(64);

    // A new buffer must start inactive.
    assert!(
        !buf.is_active(),
        "new DeltaBuffer must be inactive — no writes captured before the scan window opens"
    );

    // After activate() the window is open.
    buf.activate();
    assert!(
        buf.is_active(),
        "buffer must be active after activate() — scan window is now open"
    );

    // A second activate() on an already-active buffer must be idempotent.
    buf.activate();
    assert!(
        buf.is_active(),
        "activate() on an already-active buffer must be idempotent"
    );
}

/// `route()` while INACTIVE must be silently ignored.
///
/// This is critical for the registration-gap protocol: a write that arrives
/// BEFORE `activate()` is already visible in the scan snapshot and must NOT be
/// double-counted when the drain path replays buffered entries.  The buffer
/// enforces this by ignoring routes outside the active window.
#[test]
fn delta_buffer_route_while_inactive_does_not_change_active_state() {
    use topgun_server::query::delta_buffer::DeltaBuffer;

    let buf = DeltaBuffer::new(64);
    // Route while inactive — must be a no-op that does not change is_active.
    buf.route(
        "silent-key",
        rmpv::Value::String("value".into()),
        Timestamp {
            millis: BASE_MILLIS,
            counter: 0,
            node_id: "n".to_string(),
        },
        true,
    );

    // The buffer must remain inactive; no side-effects from the route call.
    assert!(
        !buf.is_active(),
        "route() while inactive must not alter is_active state"
    );

    // Activating now opens the window cleanly.
    buf.activate();
    assert!(buf.is_active());
}

/// Overflow guard: once more than `capacity` distinct keys have been routed, the
/// buffer enters a poisoned state.  The service path that calls
/// `deactivate_and_drain` will receive `Err(())` and must send a
/// `QUERY_SNAPSHOT_OVERFLOW` response.  We verify the externally-observable
/// signal: the buffer remains active until the drain call (the service logic
/// decides whether to signal overflow based on the drain result).
#[test]
fn delta_buffer_remains_active_after_overflow_until_drain() {
    use topgun_server::query::delta_buffer::DeltaBuffer;

    // Capacity of 2 distinct keys.
    let buf = DeltaBuffer::new(2);
    buf.activate();

    let ts_val = |ms: u64| Timestamp {
        millis: ms,
        counter: 0,
        node_id: "n".to_string(),
    };

    buf.route("k1", rmpv::Value::Nil, ts_val(1), true);
    buf.route("k2", rmpv::Value::Nil, ts_val(2), true);
    // Third distinct key would exceed capacity — the buffer enters overflow state.
    buf.route("k3", rmpv::Value::Nil, ts_val(3), true);

    // The buffer is still active until drain is called.  The service invokes
    // deactivate_and_drain (pub(crate)) and receives Err(()) which triggers the
    // QUERY_SNAPSHOT_OVERFLOW response to the subscriber.
    assert!(
        buf.is_active(),
        "overflowed buffer must remain active until deactivate_and_drain is called by the service"
    );
}

// ---------------------------------------------------------------------------
// AC7: Unbounded-sort rejects + QUERY_SNAPSHOT_OVERFLOW code constant
// ---------------------------------------------------------------------------

/// A `QuerySubscribe` with a non-empty sort spec but no limit is ACCEPTED, not
/// rejected: it is an in-memory sort over the matched set (O(result), the same
/// memory profile as a no-LIMIT full scan, which is allowed) and is soft-capped by
/// `max_query_records`. The earlier blanket `QUERY_UNBOUNDED_SORT` reject was an
/// asymmetric overreach (it rejected sort-without-limit while permitting the
/// equally-O(N) scan-without-limit) and broke the documented server-side sort API.
///
/// The genuine OOM exposure — sorting over a non-resident / larger-than-RAM match
/// set — is the streaming-source problem (TODO-532, the TODO-530 family) and the
/// protection belongs there as a SIZE/RESIDENCY-gated reject. The
/// `QUERY_UNBOUNDED_SORT` code constant is retained for that future gated reject.
#[tokio::test(flavor = "multi_thread")]
async fn unbounded_sort_query_is_accepted_not_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac7a.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store,
        Vec::new(),
    ));

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let (handle, _rx) =
        conn_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
    let conn_id = handle.id;

    let svc = make_svc(factory, conn_registry);

    // Sort by "score" ASC with NO limit — must NOT be rejected.
    let resp = run_subscribe(
        svc,
        conn_id,
        "ac7-unbounded",
        "any_map",
        Query {
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Asc,
            }]),
            limit: None,
            ..Query::default()
        },
    )
    .await;

    assert!(
        resp.payload.error.is_none(),
        "unbounded sort must NOT carry an error — it is an accepted, O(result) sort"
    );
    assert!(
        resp.payload.code.is_none(),
        "unbounded sort must NOT carry the QUERY_UNBOUNDED_SORT reject code"
    );
}

/// A sort-with-limit query must NOT be rejected — the limit makes the sort
/// safe to execute over the full-scan pager.
#[tokio::test(flavor = "multi_thread")]
async fn sort_with_limit_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_store = Arc::new(RedbDataStore::new(dir.path().join("ac7b.redb")).expect("redb open"));
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        data_store,
        Vec::new(),
    ));

    let conn_registry = Arc::new(ConnectionRegistry::new());
    let (handle, _rx) =
        conn_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
    let conn_id = handle.id;

    let svc = make_svc(factory, conn_registry);

    let resp = run_subscribe(
        svc,
        conn_id,
        "ac7-accepted",
        "any_map",
        Query {
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Asc,
            }]),
            limit: Some(50), // limit present — must be accepted
            ..Query::default()
        },
    )
    .await;

    // No rejection signal.
    assert!(
        resp.payload.error.is_none(),
        "sort-with-limit must not be rejected"
    );
    assert!(
        resp.payload.code.is_none(),
        "sort-with-limit must not set a code"
    );
}

/// The `QUERY_SNAPSHOT_OVERFLOW` string constant exported from `delta_buffer`
/// matches the hard-coded value the TS client matches on.  This guards against
/// accidental rename of the constant on the Rust side.
#[test]
fn query_snapshot_overflow_constant_value() {
    use topgun_server::query::delta_buffer::QUERY_SNAPSHOT_OVERFLOW;
    assert_eq!(
        QUERY_SNAPSHOT_OVERFLOW, "QUERY_SNAPSHOT_OVERFLOW",
        "QUERY_SNAPSHOT_OVERFLOW constant must match the TS client's expected string"
    );
}

/// The `QUERY_UNBOUNDED_SORT` string constant exported from `delta_buffer`
/// matches the hard-coded value the TS client matches on.
#[test]
fn query_unbounded_sort_constant_value() {
    use topgun_server::query::delta_buffer::QUERY_UNBOUNDED_SORT;
    assert_eq!(
        QUERY_UNBOUNDED_SORT, "QUERY_UNBOUNDED_SORT",
        "QUERY_UNBOUNDED_SORT constant must match the TS client's expected string"
    );
}

// ---------------------------------------------------------------------------
// AC8: Semantics / wire diff guard
// ---------------------------------------------------------------------------

/// `QueryRespPayload` round-trips through msgpack with the exact camelCase field
/// names the TypeScript client expects.  A successful response must NOT include
/// `error` or `code` fields in the serialised output (they are
/// `skip_serializing_if = "Option::is_none"`).
#[test]
fn query_resp_payload_wire_format_success_no_error_fields() {
    let payload = QueryRespPayload {
        query_id: "q-wire-1".to_string(),
        results: vec![topgun_core::messages::query::QueryResultEntry {
            key: "rec-1".to_string(),
            value: rmpv::Value::Map(vec![(
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Alice".into()),
            )]),
        }],
        has_more: Some(false),
        ..QueryRespPayload::default()
    };

    let bytes = rmp_serde::to_vec_named(&payload).expect("serialise");
    let decoded: rmpv::Value = rmpv::decode::read_value(&mut bytes.as_slice()).expect("decode");

    let map = match &decoded {
        rmpv::Value::Map(m) => m,
        other => panic!("expected msgpack map, got {other:?}"),
    };

    // Collect field names present in the wire output.
    let field_names: std::collections::BTreeSet<&str> =
        map.iter().filter_map(|(k, _)| k.as_str()).collect();

    // Required camelCase fields.
    assert!(field_names.contains("queryId"), "queryId must be present");
    assert!(field_names.contains("results"), "results must be present");
    assert!(field_names.contains("hasMore"), "hasMore must be present");

    // Optional error fields must be ABSENT when None (skip_serializing_if).
    assert!(
        !field_names.contains("error"),
        "error must not appear on the wire when None"
    );
    assert!(
        !field_names.contains("code"),
        "code must not appear on the wire when None"
    );
    assert!(
        !field_names.contains("nextCursor"),
        "nextCursor must not appear on the wire when None"
    );
    assert!(
        !field_names.contains("merkleRootHash"),
        "merkleRootHash must not appear on the wire when None"
    );
}

/// An error response (e.g. `QUERY_UNBOUNDED_SORT`) must include the `error` and
/// `code` fields in the wire output, and the `results` array must be empty.
#[test]
fn query_resp_payload_wire_format_error_includes_code_and_error() {
    let payload = QueryRespPayload {
        query_id: "q-wire-err".to_string(),
        results: vec![],
        error: Some("Sort without LIMIT is not supported.".to_string()),
        code: Some("QUERY_UNBOUNDED_SORT".to_string()),
        ..QueryRespPayload::default()
    };

    let bytes = rmp_serde::to_vec_named(&payload).expect("serialise");
    let decoded: rmpv::Value = rmpv::decode::read_value(&mut bytes.as_slice()).expect("decode");

    let map = match &decoded {
        rmpv::Value::Map(m) => m,
        other => panic!("expected msgpack map, got {other:?}"),
    };

    let field_map: std::collections::HashMap<&str, &rmpv::Value> = map
        .iter()
        .filter_map(|(k, v)| k.as_str().map(|s| (s, v)))
        .collect();

    // Error fields must be present.
    assert!(
        field_map.contains_key("error"),
        "error field must be present in error response"
    );
    assert!(
        field_map.contains_key("code"),
        "code field must be present in error response"
    );

    assert_eq!(
        field_map["code"].as_str(),
        Some("QUERY_UNBOUNDED_SORT"),
        "code value must round-trip exactly"
    );

    // Results must be an empty array.
    match field_map.get("results") {
        Some(rmpv::Value::Array(arr)) => {
            assert!(arr.is_empty(), "results must be empty in error response");
        }
        other => panic!("expected empty results array, got {other:?}"),
    }
}

/// Full `Message::QueryResp` envelope round-trip: the payload survives
/// serialisation to msgpack and deserialisation back to the original struct,
/// preserving all field values.
#[test]
fn query_resp_message_envelope_round_trip() {
    let original = Message::QueryResp(QueryRespMessage {
        payload: QueryRespPayload {
            query_id: "q-rt-1".to_string(),
            results: vec![topgun_core::messages::query::QueryResultEntry {
                key: "k1".to_string(),
                value: rmpv::Value::Integer(99.into()),
            }],
            has_more: None,
            error: None,
            code: None,
            ..QueryRespPayload::default()
        },
    });

    let bytes = rmp_serde::to_vec_named(&original).expect("serialise");
    let decoded: Message = rmp_serde::from_slice(&bytes).expect("deserialise");

    match decoded {
        Message::QueryResp(resp) => {
            assert_eq!(resp.payload.query_id, "q-rt-1");
            assert_eq!(resp.payload.results.len(), 1);
            assert_eq!(resp.payload.results[0].key, "k1");
            assert!(resp.payload.error.is_none());
            assert!(resp.payload.code.is_none());
        }
        other => panic!("expected QueryResp, got {other:?}"),
    }
}

/// When `has_more` is `Some(true)` the wire output must include it; when
/// `has_more` is `None` it must be absent.  This guards the optional field
/// contract that the TS `QueryRespPayloadSchema` uses (`z.boolean().optional()`).
#[test]
fn query_resp_has_more_absent_when_none_present_when_some() {
    let with_more = QueryRespPayload {
        query_id: "q-hm".to_string(),
        has_more: Some(true),
        ..QueryRespPayload::default()
    };
    let without_more = QueryRespPayload {
        query_id: "q-hm2".to_string(),
        has_more: None,
        ..QueryRespPayload::default()
    };

    let bytes_with = rmp_serde::to_vec_named(&with_more).expect("serialise");
    let bytes_without = rmp_serde::to_vec_named(&without_more).expect("serialise");

    let val_with: rmpv::Value =
        rmpv::decode::read_value(&mut bytes_with.as_slice()).expect("decode");
    let val_without: rmpv::Value =
        rmpv::decode::read_value(&mut bytes_without.as_slice()).expect("decode");

    let has_field = |v: &rmpv::Value, field: &str| match v {
        rmpv::Value::Map(m) => m.iter().any(|(k, _)| k.as_str() == Some(field)),
        _ => false,
    };

    assert!(
        has_field(&val_with, "hasMore"),
        "hasMore must be present when Some(true)"
    );
    assert!(
        !has_field(&val_without, "hasMore"),
        "hasMore must be absent when None (skip_serializing_if)"
    );
}

// ---------------------------------------------------------------------------
// GROUP-BY + non-resident TODO placeholder
// ---------------------------------------------------------------------------

/// TODO-GROUPBY-NON-RESIDENT: GROUP-BY over a non-resident (durable-only) record
/// set is NOT yet implemented.  The current DAG executor collects records from
/// the in-memory engine (`record_store_factory.get_all_for_map`) before passing
/// them to the DAG pipeline.  Records that were evicted from the in-memory engine
/// or written directly to the durable store (bypassing the engine) are INVISIBLE
/// to GROUP-BY queries even though the same records would appear in a plain
/// full-scan (which uses `scan_via_datastore`).
///
/// Impact: GROUP-BY aggregate results (count, sum, etc.) undercount the true map
/// cardinality when any records are non-resident.  This is a silent correctness
/// gap — the aggregate returns a plausible-looking number without indicating that
/// the result is partial.
///
/// Correct fix: thread the `MapDataStore` scan path (already used by plain
/// full-scan via `scan_via_datastore`) into the DAG input stage so GROUP-BY
/// also sees non-resident records.
///
/// This test is left as a documentation anchor and an `#[ignore]`d regression
/// guard.  When the feature is implemented, un-ignore and assert the correct
/// aggregate count.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "GROUP-BY over non-resident records is not yet implemented (TODO-GROUPBY-NON-RESIDENT)"]
async fn group_by_over_non_resident_records_not_yet_correct() {
    // Intentionally left empty — the ignore annotation marks the gap.
    // When implemented: seed N records directly to the durable store (non-resident),
    // run a GROUP-BY COUNT query, assert the count equals N not 0.
}
