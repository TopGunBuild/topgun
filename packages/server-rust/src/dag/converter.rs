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
use topgun_core::messages::base::{PredicateNode, PredicateOp, Query};

use crate::dag::types::{
    DagPlanDescriptor, Edge, ProcessorType, RoutingPolicy, VertexDescriptor,
};

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
            children: None,
        })
        .collect();

    if eq_nodes.len() == 1 {
        eq_nodes.into_iter().next().unwrap()
    } else {
        PredicateNode {
            op: PredicateOp::And,
            attribute: None,
            value: None,
            children: Some(eq_nodes),
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
    ///   scan -> [filter] -> [local-aggregate -> combine-aggregate] -> collector
    ///
    /// Pipeline layout (multi-node) — `NetworkSender`/`NetworkReceiver` vertices are
    /// inserted at the partition boundary between per-node local processors and the
    /// coordinator-side collector:
    ///   scan -> [filter] -> [local-aggregate] -> network-sender
    ///   network-receiver -> [combine-aggregate] -> collector
    ///
    /// Note: `query.sort` and `query.limit` are not supported in the DAG path for this
    /// spec and are intentionally ignored. Sort/limit post-processing is deferred.
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
            let group_by_fields = query.group_by.as_ref().expect("has_group_by guard ensures Some");
            let first_field = group_by_fields
                .first()
                .cloned()
                .unwrap_or_default();

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
                    rmpv::Value::String("aggField".into()),
                    rmpv::Value::String("".into()),
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
            }

            // Combine-aggregate: merges partial aggregates from all nodes.
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
        } else if multi_node {
            // No GROUP BY but multi-node: insert network boundary before collector.
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

        // --- Step 4: Collector sink ---
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
        assert!(!QueryToDagConverter::needs_distribution(&q, &single_node_assignment()));
    }

    #[test]
    fn needs_distribution_true_for_multi_node() {
        let q = Query::default();
        assert!(QueryToDagConverter::needs_distribution(&q, &multi_node_assignment()));
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
                children: None,
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
        assert!(vertex_names(&desc).contains(&"combine-aggregate"));
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
            assert!(map_name_entry.is_some(), "scan config should contain mapName");
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
}
