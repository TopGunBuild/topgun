//! Query-to-DAG conversion layer.
//!
//! `QueryToDagConverter` transforms a `Query` into a serializable `DagPlanDescriptor`
//! with the appropriate processor pipeline. The descriptor is sent over the wire to
//! remote nodes, which reconstruct the runtime `Dag` via `Dag::from_descriptor()`.
//!
//! This module never instantiates runtime processors — it only builds `VertexDescriptor`
//! and `Edge` structures with `ProcessorType` identifiers and config values. The
//! `RecordStoreFactory` is not needed here; it is injected at `Dag::from_descriptor()` time.

use std::collections::HashMap;

use anyhow::Result;
use topgun_core::messages::base::{PredicateNode, PredicateOp, Query, SortDirection, SortField};

use crate::dag::types::{DagPlanDescriptor, Edge, ProcessorType, RoutingPolicy, VertexDescriptor};

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Converts a `r#where` `HashMap` (key=field, value=equality target) into a
/// `PredicateNode` compatible with `FilterProcessorSupplier`.
///
/// Each entry becomes an `Eq` leaf. Multiple entries are wrapped in an `And` node.
/// A single entry is returned directly to avoid unnecessary nesting.
fn where_to_predicate(map: &HashMap<String, rmpv::Value>) -> PredicateNode {
    let eq_nodes: Vec<PredicateNode> = map
        .iter()
        .map(|(k, v)| PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some(k.clone()),
            value: Some(v.clone()),
            ..Default::default()
        })
        .collect();

    if eq_nodes.len() == 1 {
        eq_nodes.into_iter().next().unwrap()
    } else {
        PredicateNode {
            op: PredicateOp::And,
            children: Some(eq_nodes),
            ..Default::default()
        }
    }
}

/// Serializes a `PredicateNode` into `rmpv::Value` for storage in `VertexDescriptor::config`.
fn predicate_to_config(predicate: &PredicateNode) -> Result<rmpv::Value> {
    let bytes = rmp_serde::to_vec_named(predicate)?;
    let val: rmpv::Value = rmp_serde::from_slice(&bytes)?;
    Ok(val)
}

// ---------------------------------------------------------------------------
// QueryToDagConverter
// ---------------------------------------------------------------------------

/// Converts `Query` instances to serializable `DagPlanDescriptor`s.
///
/// The resulting descriptor can be:
/// - Sent to remote nodes in a `DagExecutePayload` for distributed execution
/// - Reconstructed locally via `Dag::from_descriptor()` for single-node bypass
pub struct QueryToDagConverter;

impl QueryToDagConverter {
    /// Returns `true` if the query requires distribution across multiple nodes.
    ///
    /// Distribution is needed when `partition_assignment` maps more than one node,
    /// meaning partitions relevant to this query reside on multiple cluster members.
    #[must_use]
    pub fn needs_distribution(
        _query: &Query,
        partition_assignment: &HashMap<String, Vec<u32>>,
    ) -> bool {
        partition_assignment.len() > 1
    }

    /// Converts a `Query` to a `DagPlanDescriptor` for the given partition assignment.
    ///
    /// Pipeline layout (single-node):
    ///   scan -> [filter] -> [local-aggregate] -> [sort] -> [limit] -> collector
    /// (No combine vertex single-node: the parallelism-1 local-aggregate already produces
    /// complete per-group results, so its emitted key set is final.)
    ///
    /// Pipeline layout (multi-node) — `NetworkSender`/`NetworkReceiver` vertices are
    /// inserted at the partition boundary between per-node local processors and the
    /// coordinator-side collector:
    ///   scan -> [filter] -> [local-aggregate] -> network-sender
    ///   network-receiver -> [combine-aggregate] -> [sort] -> [limit] -> collector
    ///
    /// # Errors
    /// Returns an error if predicate serialization fails.
    ///
    /// # Panics
    /// Does not panic in practice — the internal `expect` is guarded by a prior
    /// `is_some_and(|v| !v.is_empty())` check on `query.group_by`.
    #[allow(clippy::too_many_lines)]
    pub fn convert_query(
        query: &Query,
        map_name: &str,
        partition_assignment: &HashMap<String, Vec<u32>>,
    ) -> Result<DagPlanDescriptor> {
        let multi_node = Self::needs_distribution(query, partition_assignment);

        let mut vertices: Vec<VertexDescriptor> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut edge_priority = 0u32;

        // --- Step 1: Scan vertex ---
        let scan_config = rmpv::Value::Map(vec![(
            rmpv::Value::String("mapName".into()),
            rmpv::Value::String(map_name.into()),
        )]);

        vertices.push(VertexDescriptor {
            name: "scan".to_string(),
            local_parallelism: 1,
            processor_type: ProcessorType::Scan,
            preferred_partitions: None,
            config: Some(scan_config),
        });

        let mut last_vertex = "scan".to_string();

        // --- Step 2: Filter vertex (from r#where or predicate) ---
        let filter_predicate: Option<PredicateNode> = if let Some(ref where_map) = query.r#where {
            Some(where_to_predicate(where_map))
        } else {
            query.predicate.clone()
        };

        if let Some(predicate) = filter_predicate {
            let config = predicate_to_config(&predicate)?;

            vertices.push(VertexDescriptor {
                name: "filter".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::Filter,
                preferred_partitions: None,
                config: Some(config),
            });

            edges.push(Edge {
                source_name: last_vertex.clone(),
                source_ordinal: 0,
                dest_name: "filter".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Isolated,
                priority: edge_priority,
            });
            edge_priority += 1;
            last_vertex = "filter".to_string();
        }

        // --- Step 3: GROUP BY aggregation vertices ---
        let has_group_by = query.group_by.as_ref().is_some_and(|v| !v.is_empty());

        if has_group_by {
            // SAFETY: has_group_by is true only when group_by is Some(non-empty).
            let group_by_fields = query
                .group_by
                .as_ref()
                .expect("has_group_by guard ensures Some");
            let first_field = group_by_fields.first().cloned().unwrap_or_default();

            // Serialize the requested aggregation specs (func + optional field) into the
            // vertex config. Round-tripping `Vec<Aggregation>` through named MsgPack keeps
            // the on-wire shape (camelCase func strings) identical to what the coordinator
            // deserializes back, so the converter↔coordinator config contract cannot drift.
            // Absent/empty aggregations serialize to an empty array, which the processor
            // reads as COUNT-only mode (back-compat for groupBy-only clients).
            let aggregations_value = match &query.aggregations {
                Some(aggs) if !aggs.is_empty() => rmp_serde::to_vec_named(aggs)
                    .ok()
                    .and_then(|bytes| rmp_serde::from_slice::<rmpv::Value>(&bytes).ok())
                    .unwrap_or_else(|| rmpv::Value::Array(vec![])),
                _ => rmpv::Value::Array(vec![]),
            };

            let agg_config = rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("groupBy".into()),
                    rmpv::Value::Array(
                        group_by_fields
                            .iter()
                            .map(|f| rmpv::Value::String(f.clone().into()))
                            .collect(),
                    ),
                ),
                (
                    rmpv::Value::String("aggregations".into()),
                    aggregations_value,
                ),
            ]);

            vertices.push(VertexDescriptor {
                name: "local-aggregate".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::Aggregate,
                preferred_partitions: None,
                config: Some(agg_config),
            });

            edges.push(Edge {
                source_name: last_vertex.clone(),
                source_ordinal: 0,
                dest_name: "local-aggregate".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Isolated,
                priority: edge_priority,
            });
            edge_priority += 1;
            last_vertex = "local-aggregate".to_string();

            if multi_node {
                // Insert network boundary: sender on the worker side, receiver on coordinator side.
                vertices.push(VertexDescriptor {
                    name: "network-sender".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::NetworkSender,
                    preferred_partitions: None,
                    config: None,
                });

                edges.push(Edge {
                    source_name: last_vertex.clone(),
                    source_ordinal: 0,
                    dest_name: "network-sender".to_string(),
                    dest_ordinal: 0,
                    routing_policy: RoutingPolicy::Partitioned {
                        partition_key_field: first_field,
                    },
                    priority: edge_priority,
                });
                edge_priority += 1;

                vertices.push(VertexDescriptor {
                    name: "network-receiver".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::NetworkReceiver,
                    preferred_partitions: None,
                    config: None,
                });

                edges.push(Edge {
                    source_name: "network-sender".to_string(),
                    source_ordinal: 0,
                    dest_name: "network-receiver".to_string(),
                    dest_ordinal: 0,
                    routing_policy: RoutingPolicy::Unicast,
                    priority: edge_priority,
                });
                edge_priority += 1;

                last_vertex = "network-receiver".to_string();

                // Combine-aggregate merges partial aggregates arriving from multiple nodes.
                // It belongs to the multi-node path only: the parallelism-1 local-aggregate
                // already produces complete per-group results on a single node, so its
                // emitted (requested-only) key set is the final result. Re-running combine
                // single-node would re-project to the legacy unqualified __sum/__min/__max
                // keys and re-introduce the degenerate leak this query path is fixing.
                // (Multi-node combine still reads the legacy keys and stays COUNT-correct;
                // per-function combine of field aggregations is deferred to the cluster path.)
                vertices.push(VertexDescriptor {
                    name: "combine-aggregate".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Combine,
                    preferred_partitions: None,
                    config: None,
                });

                edges.push(Edge {
                    source_name: last_vertex.clone(),
                    source_ordinal: 0,
                    dest_name: "combine-aggregate".to_string(),
                    dest_ordinal: 0,
                    routing_policy: RoutingPolicy::Unicast,
                    priority: edge_priority,
                });
                edge_priority += 1;
                last_vertex = "combine-aggregate".to_string();
            }
        } else if multi_node {
            // No GROUP BY but multi-node: cursor filtering must happen worker-side (before
            // the network boundary) so each node filters by the global keyset position before
            // sending — applying the cursor coordinator-side over already-limited per-node
            // streams would return wrong pages.  Insert the cursor vertex here, before
            // network-sender, when the query carries a keyset cursor.
            if let Some(ref cursor_str) = query.cursor {
                let predicate_hash: u64 = query.predicate.as_ref().map_or(0, |p| {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    format!("{p:?}").hash(&mut h);
                    h.finish()
                });

                let sort_hash: u64 = query.sort.as_ref().map_or(0, |s| {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    format!("{s:?}").hash(&mut h);
                    h.finish()
                });

                let cursor_config = rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("cursor".into()),
                        rmpv::Value::String(cursor_str.clone().into()),
                    ),
                    (
                        rmpv::Value::String("predicateHash".into()),
                        rmpv::Value::Integer(rmpv::Integer::from(predicate_hash)),
                    ),
                    (
                        rmpv::Value::String("sortHash".into()),
                        rmpv::Value::Integer(rmpv::Integer::from(sort_hash)),
                    ),
                ]);

                vertices.push(VertexDescriptor {
                    name: "cursor".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Cursor,
                    preferred_partitions: None,
                    config: Some(cursor_config),
                });

                edges.push(Edge {
                    source_name: last_vertex.clone(),
                    source_ordinal: 0,
                    dest_name: "cursor".to_string(),
                    dest_ordinal: 0,
                    routing_policy: RoutingPolicy::Isolated,
                    priority: edge_priority,
                });
                edge_priority += 1;
                last_vertex = "cursor".to_string();
            }

            vertices.push(VertexDescriptor {
                name: "network-sender".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::NetworkSender,
                preferred_partitions: None,
                config: None,
            });

            edges.push(Edge {
                source_name: last_vertex.clone(),
                source_ordinal: 0,
                dest_name: "network-sender".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Unicast,
                priority: edge_priority,
            });
            edge_priority += 1;

            vertices.push(VertexDescriptor {
                name: "network-receiver".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::NetworkReceiver,
                preferred_partitions: None,
                config: None,
            });

            edges.push(Edge {
                source_name: "network-sender".to_string(),
                source_ordinal: 0,
                dest_name: "network-receiver".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Unicast,
                priority: edge_priority,
            });
            edge_priority += 1;

            last_vertex = "network-receiver".to_string();
        }

        // --- Step 3b: Cursor vertex (optional, between Filter and Sort) ---
        // A Cursor vertex is only emitted when the query carries a keyset cursor.
        // Placing it before Sort means the sort stage operates on the already-filtered
        // post-cursor result set, which is the correct semantics for keyset pagination.
        // In multi-node plans the cursor is already emitted worker-side (above) so this
        // branch is restricted to single-node plans only.
        if query.cursor.is_some() && !multi_node {
            // Pass the predicate hash and sort hash alongside the cursor token so the
            // CursorProcessor can validate that the cursor was produced by the same query
            // shape. Without this check, a cursor from a different query could return
            // incorrect results silently.
            let cursor_str = query.cursor.as_ref().expect("checked above");
            let predicate_hash: u64 = query.predicate.as_ref().map_or(0, |p| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{p:?}").hash(&mut h);
                h.finish()
            });

            let sort_hash: u64 = query.sort.as_ref().map_or(0, |s| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{s:?}").hash(&mut h);
                h.finish()
            });

            let cursor_config = rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("cursor".into()),
                    rmpv::Value::String(cursor_str.clone().into()),
                ),
                (
                    rmpv::Value::String("predicateHash".into()),
                    rmpv::Value::Integer(rmpv::Integer::from(predicate_hash)),
                ),
                (
                    rmpv::Value::String("sortHash".into()),
                    rmpv::Value::Integer(rmpv::Integer::from(sort_hash)),
                ),
            ]);

            vertices.push(VertexDescriptor {
                name: "cursor".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::Cursor,
                preferred_partitions: None,
                config: Some(cursor_config),
            });

            edges.push(Edge {
                source_name: last_vertex.clone(),
                source_ordinal: 0,
                dest_name: "cursor".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Isolated,
                priority: edge_priority,
            });
            edge_priority += 1;
            last_vertex = "cursor".to_string();
        }

        // --- Step 4: Sort vertex (optional) ---
        if let Some(ref sort_fields) = query.sort {
            if !sort_fields.is_empty() {
                // Caller-specified order is preserved: the Vec<SortField> wire type
                // carries insertion order end-to-end, so no re-ordering is applied here.
                let sort_config = rmpv::Value::Array(
                    sort_fields
                        .iter()
                        .map(|SortField { field, direction }| {
                            let dir_str = match direction {
                                SortDirection::Asc => "asc",
                                SortDirection::Desc => "desc",
                            };
                            rmpv::Value::Array(vec![
                                rmpv::Value::String(field.clone().into()),
                                rmpv::Value::String(dir_str.into()),
                            ])
                        })
                        .collect(),
                );

                vertices.push(VertexDescriptor {
                    name: "sort".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Sort,
                    preferred_partitions: None,
                    config: Some(sort_config),
                });

                edges.push(Edge {
                    source_name: last_vertex.clone(),
                    source_ordinal: 0,
                    dest_name: "sort".to_string(),
                    dest_ordinal: 0,
                    routing_policy: RoutingPolicy::Isolated,
                    priority: edge_priority,
                });
                edge_priority += 1;
                last_vertex = "sort".to_string();
            }
        }

        // --- Step 5: Limit vertex (optional) ---
        if let Some(limit) = query.limit {
            let limit_config = rmpv::Value::Integer(rmpv::Integer::from(u64::from(limit)));

            vertices.push(VertexDescriptor {
                name: "limit".to_string(),
                local_parallelism: 1,
                processor_type: ProcessorType::Limit,
                preferred_partitions: None,
                config: Some(limit_config),
            });

            edges.push(Edge {
                source_name: last_vertex.clone(),
                source_ordinal: 0,
                dest_name: "limit".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Isolated,
                priority: edge_priority,
            });
            edge_priority += 1;
            last_vertex = "limit".to_string();
        }

        // --- Step 6: Collector sink ---
        vertices.push(VertexDescriptor {
            name: "collector".to_string(),
            local_parallelism: 1,
            processor_type: ProcessorType::Collector,
            preferred_partitions: None,
            config: None,
        });

        edges.push(Edge {
            source_name: last_vertex,
            source_ordinal: 0,
            dest_name: "collector".to_string(),
            dest_ordinal: 0,
            routing_policy: RoutingPolicy::Unicast,
            priority: edge_priority,
        });

        Ok(DagPlanDescriptor { vertices, edges })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::base::Query;

    fn single_node_assignment() -> HashMap<String, Vec<u32>> {
        let mut m = HashMap::new();
        m.insert("node-1".to_string(), vec![0, 1, 2]);
        m
    }

    fn multi_node_assignment() -> HashMap<String, Vec<u32>> {
        let mut m = HashMap::new();
        m.insert("node-1".to_string(), vec![0, 1]);
        m.insert("node-2".to_string(), vec![2, 3]);
        m.insert("node-3".to_string(), vec![4, 5]);
        m
    }

    fn vertex_names(desc: &DagPlanDescriptor) -> Vec<&str> {
        desc.vertices.iter().map(|v| v.name.as_str()).collect()
    }

    // --- needs_distribution ---

    #[test]
    fn needs_distribution_false_for_single_node() {
        let q = Query::default();
        assert!(!QueryToDagConverter::needs_distribution(
            &q,
            &single_node_assignment()
        ));
    }

    #[test]
    fn needs_distribution_true_for_multi_node() {
        let q = Query::default();
        assert!(QueryToDagConverter::needs_distribution(
            &q,
            &multi_node_assignment()
        ));
    }

    // --- Simple scan-only query ---

    #[test]
    fn convert_simple_query_single_node() {
        let q = Query::default();
        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"scan"));
        assert!(vertex_names(&desc).contains(&"collector"));
        assert!(!vertex_names(&desc).contains(&"filter"));
        assert!(!vertex_names(&desc).contains(&"network-sender"));
        assert_eq!(desc.edges.len(), 1);
    }

    // --- Filter from r#where ---

    #[test]
    fn convert_query_with_where_adds_filter() {
        let mut where_map = HashMap::new();
        where_map.insert("status".to_string(), rmpv::Value::String("active".into()));

        let q = Query {
            r#where: Some(where_map),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"filter"));
        // scan -> filter -> collector
        assert_eq!(desc.edges.len(), 2);
    }

    // --- Filter from predicate ---

    #[test]
    fn convert_query_with_predicate_adds_filter() {
        let q = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("name".to_string()),
                value: Some(rmpv::Value::String("Alice".into())),
                ..Default::default()
            }),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"filter"));
    }

    // --- GROUP BY single-node ---

    #[test]
    fn convert_query_with_group_by_single_node() {
        let q = Query {
            group_by: Some(vec!["category".to_string()]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "orders", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"local-aggregate"));
        // Single-node needs no combine: the parallelism-1 local-aggregate already produces
        // complete per-group results, so its emitted key set is the final result. A combine
        // pass here would only re-project to the legacy degenerate aggregate keys.
        assert!(!vertex_names(&desc).contains(&"combine-aggregate"));
        assert!(!vertex_names(&desc).contains(&"network-sender"));
    }

    // --- GROUP BY multi-node ---

    #[test]
    fn convert_query_with_group_by_multi_node_includes_network_vertices() {
        let q = Query {
            group_by: Some(vec!["category".to_string()]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "orders", &multi_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"local-aggregate"));
        assert!(vertex_names(&desc).contains(&"network-sender"));
        assert!(vertex_names(&desc).contains(&"network-receiver"));
        assert!(vertex_names(&desc).contains(&"combine-aggregate"));
        assert!(vertex_names(&desc).contains(&"collector"));
    }

    // --- Multi-node without GROUP BY ---

    #[test]
    fn convert_query_multi_node_no_group_by_includes_network_vertices() {
        let q = Query::default();
        let desc = QueryToDagConverter::convert_query(&q, "users", &multi_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"network-sender"));
        assert!(vertex_names(&desc).contains(&"network-receiver"));
        assert!(!vertex_names(&desc).contains(&"local-aggregate"));
    }

    // --- Scan config contains mapName ---

    #[test]
    fn convert_query_scan_vertex_config_has_map_name() {
        let q = Query::default();
        let desc = QueryToDagConverter::convert_query(&q, "my_map", &single_node_assignment())
            .expect("convert should succeed");

        let scan = desc.vertices.iter().find(|v| v.name == "scan").unwrap();
        let config = scan.config.as_ref().expect("scan should have config");
        if let rmpv::Value::Map(pairs) = config {
            let map_name_entry = pairs.iter().find(|(k, _)| k.as_str() == Some("mapName"));
            assert!(
                map_name_entry.is_some(),
                "scan config should contain mapName"
            );
            let map_name_val = &map_name_entry.unwrap().1;
            assert_eq!(map_name_val.as_str(), Some("my_map"));
        } else {
            panic!("scan config should be a Map");
        }
    }

    // --- where_to_predicate helper ---

    #[test]
    fn where_to_predicate_single_entry_returns_eq_node() {
        let mut map = HashMap::new();
        map.insert("key".to_string(), rmpv::Value::String("val".into()));
        let pred = where_to_predicate(&map);
        assert_eq!(pred.op, PredicateOp::Eq);
        assert_eq!(pred.attribute, Some("key".to_string()));
    }

    #[test]
    fn where_to_predicate_multiple_entries_returns_and_node() {
        let mut map = HashMap::new();
        map.insert("a".to_string(), rmpv::Value::Integer(1.into()));
        map.insert("b".to_string(), rmpv::Value::Integer(2.into()));
        let pred = where_to_predicate(&map);
        assert_eq!(pred.op, PredicateOp::And);
        assert!(pred.children.is_some());
        let children = pred.children.unwrap();
        assert_eq!(children.len(), 2);
        assert!(children.iter().all(|c| c.op == PredicateOp::Eq));
    }

    // --- Sort vertex insertion ---

    #[test]
    fn convert_query_with_sort_inserts_sort_vertex() {
        use topgun_core::messages::base::{SortDirection, SortField};

        let q = Query {
            sort: Some(vec![SortField {
                field: "age".to_string(),
                direction: SortDirection::Desc,
            }]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"sort"));
        assert!(vertex_names(&desc).contains(&"collector"));

        // Sort should be between the last processing vertex and collector
        let sort_idx = desc.vertices.iter().position(|v| v.name == "sort").unwrap();
        let collector_idx = desc
            .vertices
            .iter()
            .position(|v| v.name == "collector")
            .unwrap();
        assert!(sort_idx < collector_idx, "sort must come before collector");

        // Verify sort config contains the field
        let sort_vertex = &desc.vertices[sort_idx];
        let config = sort_vertex
            .config
            .as_ref()
            .expect("sort should have config");
        if let rmpv::Value::Array(arr) = config {
            assert_eq!(arr.len(), 1, "one sort field");
            if let rmpv::Value::Array(pair) = &arr[0] {
                assert_eq!(pair[0].as_str(), Some("age"));
                assert_eq!(pair[1].as_str(), Some("desc"));
            } else {
                panic!("sort config entry should be an array pair");
            }
        } else {
            panic!("sort config should be an array");
        }
    }

    // --- Limit vertex insertion ---

    #[test]
    fn convert_query_with_limit_inserts_limit_vertex() {
        let q = Query {
            limit: Some(10),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(vertex_names(&desc).contains(&"limit"));

        let limit_idx = desc
            .vertices
            .iter()
            .position(|v| v.name == "limit")
            .unwrap();
        let collector_idx = desc
            .vertices
            .iter()
            .position(|v| v.name == "collector")
            .unwrap();
        assert!(
            limit_idx < collector_idx,
            "limit must come before collector"
        );

        // Verify limit config
        let limit_vertex = &desc.vertices[limit_idx];
        let config = limit_vertex
            .config
            .as_ref()
            .expect("limit should have config");
        assert_eq!(config.as_u64(), Some(10));
    }

    // --- Sort + Limit vertex ordering ---

    #[test]
    fn convert_query_with_sort_and_limit_has_correct_order() {
        use topgun_core::messages::base::{SortDirection, SortField};

        let q = Query {
            sort: Some(vec![SortField {
                field: "age".to_string(),
                direction: SortDirection::Desc,
            }]),
            limit: Some(5),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        let names = vertex_names(&desc);
        assert!(names.contains(&"sort"));
        assert!(names.contains(&"limit"));
        assert!(names.contains(&"collector"));

        let sort_idx = names.iter().position(|&n| n == "sort").unwrap();
        let limit_idx = names.iter().position(|&n| n == "limit").unwrap();
        let collector_idx = names.iter().position(|&n| n == "collector").unwrap();

        assert!(sort_idx < limit_idx, "sort must come before limit");
        assert!(
            limit_idx < collector_idx,
            "limit must come before collector"
        );

        // Verify edge chain: ... -> sort -> limit -> collector
        let sort_to_limit = desc
            .edges
            .iter()
            .find(|e| e.source_name == "sort" && e.dest_name == "limit");
        assert!(
            sort_to_limit.is_some(),
            "edge from sort to limit must exist"
        );

        let limit_to_collector = desc
            .edges
            .iter()
            .find(|e| e.source_name == "limit" && e.dest_name == "collector");
        assert!(
            limit_to_collector.is_some(),
            "edge from limit to collector must exist"
        );
    }

    // --- Sort + Limit after GROUP BY ---

    #[test]
    fn convert_query_with_group_by_and_sort_limit_inserts_after_aggregate() {
        use topgun_core::messages::base::{SortDirection, SortField};

        let q = Query {
            group_by: Some(vec!["category".to_string()]),
            sort: Some(vec![SortField {
                field: "__count".to_string(),
                direction: SortDirection::Desc,
            }]),
            limit: Some(3),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "orders", &single_node_assignment())
            .expect("convert should succeed");

        let names = vertex_names(&desc);
        // Single-node has no combine vertex; the aggregate is local-aggregate and sort/limit
        // chain after it.
        assert!(!names.contains(&"combine-aggregate"));
        let aggregate_idx = names.iter().position(|&n| n == "local-aggregate").unwrap();
        let sort_idx = names.iter().position(|&n| n == "sort").unwrap();
        let limit_idx = names.iter().position(|&n| n == "limit").unwrap();
        let collector_idx = names.iter().position(|&n| n == "collector").unwrap();

        assert!(
            aggregate_idx < sort_idx,
            "sort must come after local-aggregate"
        );
        assert!(sort_idx < limit_idx, "sort must come before limit");
        assert!(
            limit_idx < collector_idx,
            "limit must come before collector"
        );
    }

    // --- Multi-field sort: caller order preserved (not alphabetical) ---

    #[test]
    fn convert_query_multi_field_sort_preserves_caller_order() {
        use topgun_core::messages::base::{SortDirection, SortField};

        // Caller specifies "a ASC, b DESC" — alphabetical order would be the same here,
        // so we use "z_field ASC, a_field DESC" to prove caller order (z before a) wins
        // over alphabetical order (a before z).
        let q = Query {
            sort: Some(vec![
                SortField {
                    field: "z_field".to_string(),
                    direction: SortDirection::Asc,
                },
                SortField {
                    field: "a_field".to_string(),
                    direction: SortDirection::Desc,
                },
            ]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        let sort_vertex = desc.vertices.iter().find(|v| v.name == "sort").unwrap();
        let config = sort_vertex.config.as_ref().unwrap();
        if let rmpv::Value::Array(arr) = config {
            assert_eq!(arr.len(), 2);
            // First field must be "z_field" (caller-specified order, not alphabetical)
            if let rmpv::Value::Array(pair) = &arr[0] {
                assert_eq!(
                    pair[0].as_str(),
                    Some("z_field"),
                    "caller order: z_field first"
                );
                assert_eq!(pair[1].as_str(), Some("asc"));
            } else {
                panic!("sort config entry should be an array pair");
            }
            // Second field must be "a_field"
            if let rmpv::Value::Array(pair) = &arr[1] {
                assert_eq!(
                    pair[0].as_str(),
                    Some("a_field"),
                    "caller order: a_field second"
                );
                assert_eq!(pair[1].as_str(), Some("desc"));
            } else {
                panic!("sort config entry should be an array pair");
            }
        } else {
            panic!("sort config should be an array");
        }
    }

    // --- Cursor vertex insertion (between Filter and Sort) ---

    #[test]
    fn convert_query_with_cursor_inserts_cursor_vertex_between_filter_and_sort() {
        use topgun_core::messages::base::{SortDirection, SortField};

        let mut where_map = HashMap::new();
        where_map.insert("status".to_string(), rmpv::Value::String("active".into()));

        let q = Query {
            r#where: Some(where_map),
            sort: Some(vec![SortField {
                field: "age".to_string(),
                direction: SortDirection::Desc,
            }]),
            cursor: Some("opaque-cursor-token".to_string()),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        let names = vertex_names(&desc);
        assert!(names.contains(&"cursor"), "cursor vertex must be emitted");

        // Cursor must sit strictly between Filter and Sort: Scan→Filter→Cursor→Sort→…
        let filter_idx = names.iter().position(|&n| n == "filter").unwrap();
        let cursor_idx = names.iter().position(|&n| n == "cursor").unwrap();
        let sort_idx = names.iter().position(|&n| n == "sort").unwrap();
        assert!(filter_idx < cursor_idx, "cursor must come after filter");
        assert!(cursor_idx < sort_idx, "cursor must come before sort");

        // The cursor vertex is typed as Cursor and carries the keyset token in its config.
        let cursor_vertex = &desc.vertices[cursor_idx];
        assert_eq!(cursor_vertex.processor_type, ProcessorType::Cursor);
        let config = cursor_vertex
            .config
            .as_ref()
            .expect("cursor vertex should have config");
        if let rmpv::Value::Map(entries) = config {
            let token = entries
                .iter()
                .find(|(k, _)| k.as_str() == Some("cursor"))
                .map(|(_, v)| v.as_str());
            assert_eq!(token, Some(Some("opaque-cursor-token")));
        } else {
            panic!("cursor config should be a map");
        }

        // Edge chain proves the wiring: filter → cursor → sort.
        assert!(
            desc.edges
                .iter()
                .any(|e| e.source_name == "filter" && e.dest_name == "cursor"),
            "edge from filter to cursor must exist"
        );
        assert!(
            desc.edges
                .iter()
                .any(|e| e.source_name == "cursor" && e.dest_name == "sort"),
            "edge from cursor to sort must exist"
        );
    }

    #[test]
    fn convert_query_without_cursor_emits_no_cursor_vertex() {
        use topgun_core::messages::base::{SortDirection, SortField};

        // The non-paginated path must add zero cursor overhead.
        let q = Query {
            sort: Some(vec![SortField {
                field: "age".to_string(),
                direction: SortDirection::Desc,
            }]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &single_node_assignment())
            .expect("convert should succeed");

        assert!(
            !vertex_names(&desc).contains(&"cursor"),
            "no cursor vertex on the non-paginated path"
        );
    }

    // --- Multi-node cursor is worker-side (before network-sender) ---

    #[test]
    fn convert_query_multi_node_with_cursor_places_cursor_before_network_sender() {
        use topgun_core::messages::base::{SortDirection, SortField};

        // In a distributed plan the cursor vertex must sit on the worker side of the
        // network boundary so each node filters by the global keyset position before
        // sending.  A coordinator-side cursor over already-limited per-node streams
        // would silently return wrong pages.
        let q = Query {
            cursor: Some("page2-cursor-token".to_string()),
            sort: Some(vec![SortField {
                field: "score".to_string(),
                direction: SortDirection::Desc,
            }]),
            ..Default::default()
        };

        let desc = QueryToDagConverter::convert_query(&q, "users", &multi_node_assignment())
            .expect("convert should succeed");

        let names = vertex_names(&desc);
        assert!(names.contains(&"cursor"), "cursor vertex must be emitted");
        assert!(
            names.contains(&"network-sender"),
            "network-sender must be emitted in multi-node plan"
        );

        // The cursor's edge ordinal/priority must be strictly before network-sender:
        // confirm by checking the edge chain.
        let cursor_to_netsender = desc
            .edges
            .iter()
            .any(|e| e.source_name == "cursor" && e.dest_name == "network-sender");
        assert!(
            cursor_to_netsender,
            "cursor must connect directly to network-sender (worker-side cursor)"
        );

        // No edge from network-receiver to cursor: the cursor is not coordinator-side.
        let receiver_to_cursor = desc
            .edges
            .iter()
            .any(|e| e.source_name == "network-receiver" && e.dest_name == "cursor");
        assert!(
            !receiver_to_cursor,
            "cursor must not be placed after network-receiver (coordinator-side would be wrong)"
        );
    }
}
