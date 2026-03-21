//! Shape domain message types for partial replication subscriptions.
//!
//! Shapes allow clients to subscribe to filtered subsets of map data.
//! All structs use `#[serde(rename_all = "camelCase")]` to produce
//! wire-compatible `MsgPack` output via `rmp_serde::to_vec_named()`.

use serde::{Deserialize, Serialize};

use crate::messages::base::ChangeEventType;
use crate::schema::SyncShape;

// ---------------------------------------------------------------------------
// Helper struct
// ---------------------------------------------------------------------------

/// A single key-value record transferred in a shape response.
///
/// The record key is always included regardless of the `fields` projection
/// list on the shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeRecord {
    /// The key of the record.
    pub key: String,
    /// The record value.
    pub value: rmpv::Value,
}

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Payload for a shape subscription request.
///
/// Embeds `SyncShape` directly to avoid field drift between the schema type
/// and the wire message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeSubscribePayload {
    /// The shape definition to subscribe to.
    pub shape: SyncShape,
}

/// Payload for a shape unsubscription request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeUnsubscribePayload {
    /// Identifier of the shape subscription to cancel.
    pub shape_id: String,
}

/// Payload for a shape response from server to client.
///
/// `merkle_root_hash` of 0 means "empty tree" (no Merkle tree built yet),
/// consistent with `SyncRespRootPayload.root_hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeRespPayload {
    /// Identifier of the shape this response is for.
    pub shape_id: String,
    /// The records matching the shape filter.
    pub records: Vec<ShapeRecord>,
    /// Merkle root hash for the shape's data set. 0 means empty tree.
    pub merkle_root_hash: u32,
    /// Whether more records are available beyond this batch.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub has_more: Option<bool>,
}

/// Payload for a shape update pushed from server to client.
///
/// Sent when a record in the shape's filter set changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeUpdatePayload {
    /// Identifier of the shape this update belongs to.
    pub shape_id: String,
    /// The key of the record that changed.
    pub key: String,
    /// The new value, or `None` if the record was removed from the shape.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,
    /// The type of change event.
    pub change_type: ChangeEventType,
}

/// Payload for a shape-specific Merkle delta sync initiation.
///
/// Client sends its current shape Merkle root hash to initiate delta sync.
/// The server uses the existing `SyncRespRoot`/`SyncRespBuckets`/`SyncRespLeaf`
/// protocol, with shape paths prefixed by `<shape_id>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeSyncInitPayload {
    /// Identifier of the shape to sync.
    pub shape_id: String,
    /// Client's current shape Merkle root hash.
    pub root_hash: u32,
}

// ---------------------------------------------------------------------------
// Message wrapper structs
// ---------------------------------------------------------------------------

/// Shape subscription request message.
///
/// Follows the `QuerySubMessage` / `pub payload: XxxPayload` wrapper pattern.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeSubscribeMessage {
    /// The shape subscription payload.
    pub payload: ShapeSubscribePayload,
}

/// Shape unsubscription request message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeUnsubscribeMessage {
    /// The shape unsubscription payload.
    pub payload: ShapeUnsubscribePayload,
}

/// Shape response message from server to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeRespMessage {
    /// The shape response payload.
    pub payload: ShapeRespPayload,
}

/// Shape update message pushed from server to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeUpdateMessage {
    /// The shape update payload.
    pub payload: ShapeUpdatePayload,
}

/// Shape Merkle sync initiation message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeSyncInitMessage {
    /// The shape sync init payload.
    pub payload: ShapeSyncInitPayload,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::base::{ChangeEventType, PredicateNode, PredicateOp};
    use crate::schema::SyncShape;

    /// Helper: round-trip a value through named MsgPack serialization.
    fn roundtrip_named<T>(val: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let bytes = rmp_serde::to_vec_named(val).expect("serialize");
        rmp_serde::from_slice(&bytes).expect("deserialize")
    }

    // ---- ShapeSubscribeMessage roundtrip ----

    #[test]
    fn shape_subscribe_message_minimal_roundtrip() {
        let msg = ShapeSubscribeMessage {
            payload: ShapeSubscribePayload {
                shape: SyncShape {
                    shape_id: "s-1".to_string(),
                    map_name: "users".to_string(),
                    filter: None,
                    fields: None,
                    limit: None,
                },
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn shape_subscribe_message_full_roundtrip() {
        let msg = ShapeSubscribeMessage {
            payload: ShapeSubscribePayload {
                shape: SyncShape {
                    shape_id: "s-2".to_string(),
                    map_name: "products".to_string(),
                    filter: Some(PredicateNode {
                        op: PredicateOp::Gt,
                        attribute: Some("price".to_string()),
                        value: Some(rmpv::Value::Integer(100.into())),
                        children: None,
                    }),
                    fields: Some(vec!["name".to_string(), "price".to_string()]),
                    limit: Some(500),
                },
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- ShapeUnsubscribeMessage roundtrip ----

    #[test]
    fn shape_unsubscribe_message_roundtrip() {
        let msg = ShapeUnsubscribeMessage {
            payload: ShapeUnsubscribePayload {
                shape_id: "s-1".to_string(),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- ShapeRespMessage roundtrip ----

    #[test]
    fn shape_resp_message_empty_roundtrip() {
        let msg = ShapeRespMessage {
            payload: ShapeRespPayload {
                shape_id: "s-1".to_string(),
                records: vec![],
                merkle_root_hash: 0,
                has_more: None,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn shape_resp_message_with_records_roundtrip() {
        let msg = ShapeRespMessage {
            payload: ShapeRespPayload {
                shape_id: "s-1".to_string(),
                records: vec![
                    ShapeRecord {
                        key: "user-1".to_string(),
                        value: rmpv::Value::Map(vec![(
                            rmpv::Value::String("name".into()),
                            rmpv::Value::String("Alice".into()),
                        )]),
                    },
                    ShapeRecord {
                        key: "user-2".to_string(),
                        value: rmpv::Value::Map(vec![(
                            rmpv::Value::String("name".into()),
                            rmpv::Value::String("Bob".into()),
                        )]),
                    },
                ],
                merkle_root_hash: 12345,
                has_more: Some(true),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- ShapeUpdateMessage roundtrip ----

    #[test]
    fn shape_update_message_enter_roundtrip() {
        let msg = ShapeUpdateMessage {
            payload: ShapeUpdatePayload {
                shape_id: "s-1".to_string(),
                key: "user-3".to_string(),
                value: Some(rmpv::Value::Map(vec![(
                    rmpv::Value::String("name".into()),
                    rmpv::Value::String("Carol".into()),
                )])),
                change_type: ChangeEventType::ENTER,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn shape_update_message_leave_roundtrip() {
        let msg = ShapeUpdateMessage {
            payload: ShapeUpdatePayload {
                shape_id: "s-1".to_string(),
                key: "user-3".to_string(),
                value: None,
                change_type: ChangeEventType::LEAVE,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn shape_update_message_update_roundtrip() {
        let msg = ShapeUpdateMessage {
            payload: ShapeUpdatePayload {
                shape_id: "s-2".to_string(),
                key: "product-5".to_string(),
                value: Some(rmpv::Value::Integer(200.into())),
                change_type: ChangeEventType::UPDATE,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- ShapeSyncInitMessage roundtrip ----

    #[test]
    fn shape_sync_init_message_roundtrip() {
        let msg = ShapeSyncInitMessage {
            payload: ShapeSyncInitPayload {
                shape_id: "s-1".to_string(),
                root_hash: 98765,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn shape_sync_init_message_zero_hash_roundtrip() {
        let msg = ShapeSyncInitMessage {
            payload: ShapeSyncInitPayload {
                shape_id: "s-new".to_string(),
                root_hash: 0,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- camelCase field name verification ----

    #[test]
    fn shape_subscribe_camel_case_field_names() {
        let msg = ShapeSubscribeMessage::default();
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");
        let keys: Vec<&str> = map.iter().filter_map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"payload"), "expected 'payload' key");
    }

    #[test]
    fn shape_resp_camel_case_field_names() {
        let msg = ShapeRespMessage {
            payload: ShapeRespPayload {
                shape_id: "s-1".to_string(),
                records: vec![],
                merkle_root_hash: 0,
                has_more: Some(false),
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let outer_map = val.as_map().expect("should be a map");
        let payload = outer_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");
        let payload_keys: Vec<&str> =
            payload_map.iter().filter_map(|(k, _)| k.as_str()).collect();
        assert!(payload_keys.contains(&"shapeId"), "expected camelCase 'shapeId'");
        assert!(
            payload_keys.contains(&"merkleRootHash"),
            "expected camelCase 'merkleRootHash'"
        );
        assert!(payload_keys.contains(&"hasMore"), "expected camelCase 'hasMore'");
    }

    #[test]
    fn shape_update_camel_case_field_names() {
        let msg = ShapeUpdateMessage {
            payload: ShapeUpdatePayload {
                shape_id: "s-1".to_string(),
                key: "k".to_string(),
                value: None,
                change_type: ChangeEventType::LEAVE,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let outer_map = val.as_map().expect("should be a map");
        let payload = outer_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");
        let payload_keys: Vec<&str> =
            payload_map.iter().filter_map(|(k, _)| k.as_str()).collect();
        assert!(payload_keys.contains(&"shapeId"), "expected camelCase 'shapeId'");
        assert!(payload_keys.contains(&"changeType"), "expected camelCase 'changeType'");
    }

    #[test]
    fn shape_sync_init_camel_case_field_names() {
        let msg = ShapeSyncInitMessage {
            payload: ShapeSyncInitPayload {
                shape_id: "s-1".to_string(),
                root_hash: 0,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let outer_map = val.as_map().expect("should be a map");
        let payload = outer_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");
        let payload_keys: Vec<&str> =
            payload_map.iter().filter_map(|(k, _)| k.as_str()).collect();
        assert!(payload_keys.contains(&"shapeId"), "expected camelCase 'shapeId'");
        assert!(payload_keys.contains(&"rootHash"), "expected camelCase 'rootHash'");
    }

    // ---- Optional field omission ----

    #[test]
    fn shape_resp_optional_has_more_omitted_when_none() {
        let msg = ShapeRespMessage {
            payload: ShapeRespPayload {
                shape_id: "s-1".to_string(),
                records: vec![],
                merkle_root_hash: 0,
                has_more: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let outer_map = val.as_map().expect("should be a map");
        let payload = outer_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");
        let has_more_key = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("hasMore"));
        assert!(!has_more_key, "hasMore should be omitted when None");
    }

    #[test]
    fn shape_update_optional_value_omitted_when_none() {
        let msg = ShapeUpdateMessage {
            payload: ShapeUpdatePayload {
                shape_id: "s-1".to_string(),
                key: "k".to_string(),
                value: None,
                change_type: ChangeEventType::LEAVE,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let outer_map = val.as_map().expect("should be a map");
        let payload = outer_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");
        let has_value_key = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("value"));
        assert!(!has_value_key, "value should be omitted when None");
    }

    // ---- Default derive ----

    #[test]
    fn shape_subscribe_payload_default_constructs() {
        let p = ShapeSubscribePayload::default();
        assert_eq!(p.shape.shape_id, "");
        assert_eq!(p.shape.map_name, "");
        assert_eq!(p.shape.filter, None);
        assert_eq!(p.shape.fields, None);
        assert_eq!(p.shape.limit, None);
    }
}
